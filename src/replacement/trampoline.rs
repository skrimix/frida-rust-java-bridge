use std::{
    ffi::{c_int, c_void},
    mem::{align_of, offset_of, size_of},
    ptr::{self, NonNull},
};

use frida_gum::instruction_writer::{Aarch64InstructionWriter, Aarch64Register, InstructionWriter};

use crate::{Error, Result};

use super::{
    FEATURE_CLOSURE_REPLACEMENT,
    closure::{
        ClosureArgumentLocation, ClosureInvocationFrame, ClosureReplacementLayout,
        ClosureReplacementState, ClosureValueLayout, dispatch_closure_invocation,
    },
};

const THUNK_CODE_LENGTH: usize = 4096;
const MAX_INVOCATION_FRAME_SIZE: usize = 4096;

unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: isize,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, length: usize, prot: c_int) -> c_int;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
}

pub(super) struct ClosureReplacementThunk {
    pointer: NonNull<c_void>,
    length: usize,
}

// The thunk owns one executable mapping and only frees it on drop. The pointer is not tied to a
// thread, and replacement guards may be stored in process-global state for startup hooks.
unsafe impl Send for ClosureReplacementThunk {}

impl ClosureReplacementThunk {
    pub(super) fn new(
        layout: &ClosureReplacementLayout,
        state: *mut ClosureReplacementState,
    ) -> Result<Self> {
        let _gum = crate::runtime::process_gum();
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;

        if !cfg!(target_arch = "aarch64") {
            return Err(Error::UnsupportedFeature {
                feature: FEATURE_CLOSURE_REPLACEMENT,
                reason: "closure replacement trampolines are currently arm64-only".to_owned(),
            });
        }
        if state.is_null() {
            return Err(Error::NullReturn {
                operation: "closure replacement state",
            });
        }
        validate_closure_trampoline_layout(layout, "replacement::replace_closure_method")?;

        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                THUNK_CODE_LENGTH,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return Err(Error::UnsupportedFeature {
                feature: FEATURE_CLOSURE_REPLACEMENT,
                reason: "unable to allocate closure replacement trampoline".to_owned(),
            });
        }

        if let Err(error) = write_closure_trampoline(pointer, state, layout) {
            unsafe { munmap(pointer, THUNK_CODE_LENGTH) };
            return Err(error);
        }
        unsafe {
            frida_gum_sys::gum_clear_cache(pointer, THUNK_CODE_LENGTH as u64);
            if mprotect(pointer, THUNK_CODE_LENGTH, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, THUNK_CODE_LENGTH);
                return Err(Error::UnsupportedFeature {
                    feature: FEATURE_CLOSURE_REPLACEMENT,
                    reason: "unable to protect closure replacement trampoline".to_owned(),
                });
            }
        }

        let Some(pointer) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, THUNK_CODE_LENGTH) };
            return Err(Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            pointer,
            length: THUNK_CODE_LENGTH,
        })
    }

    pub(super) fn as_ptr(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }

    pub(super) fn leak(&mut self) {
        self.length = 0;
    }
}

impl Drop for ClosureReplacementThunk {
    fn drop(&mut self) {
        if self.length != 0 {
            unsafe {
                munmap(self.pointer.as_ptr(), self.length);
            }
        }
    }
}

pub(super) fn validate_closure_trampoline_layout(
    layout: &ClosureReplacementLayout,
    operation: &'static str,
) -> Result<()> {
    let frame_size = closure_invocation_frame_size(layout).ok_or_else(|| {
        Error::InvalidReplacementImplementation {
            operation,
            expected: format!(
                "closure replacement invocation frame up to {MAX_INVOCATION_FRAME_SIZE} bytes"
            ),
            actual: "descriptor overflows closure invocation frame sizing",
        }
    })?;
    if frame_size > MAX_INVOCATION_FRAME_SIZE {
        return Err(Error::InvalidReplacementImplementation {
            operation,
            expected: format!(
                "closure replacement invocation frame up to {MAX_INVOCATION_FRAME_SIZE} bytes"
            ),
            actual: "descriptor is too large",
        });
    }
    Ok(())
}

fn write_closure_trampoline(
    code: *mut c_void,
    state: *mut ClosureReplacementState,
    layout: &ClosureReplacementLayout,
) -> Result<()> {
    let _gum = crate::runtime::process_gum();
    let writer = Aarch64InstructionWriter::new(code as u64);

    let argument_count = layout.arguments.len();
    let arguments_offset = closure_arguments_offset();
    let frame_size = closure_invocation_frame_size(layout)
        .expect("closure trampoline layout checked before code generation");

    ensure_closure_writer(
        writer.put_push_reg_reg(Aarch64Register::Fp, Aarch64Register::Lr),
        "save closure trampoline frame",
    )?;
    ensure_closure_writer(
        writer.put_sub_reg_reg_imm(Aarch64Register::Sp, Aarch64Register::Sp, frame_size as u64),
        "allocate closure invocation frame",
    )?;

    store_immediate(&writer, state as u64, frame_offset("state"))?;
    store_register(
        &writer,
        Aarch64Register::X0,
        frame_offset("env"),
        "store closure JNI env",
    )?;
    store_register(
        &writer,
        Aarch64Register::X1,
        frame_offset("target"),
        "store closure JNI target",
    )?;
    ensure_closure_writer(
        writer.put_add_reg_reg_imm(
            Aarch64Register::X16,
            Aarch64Register::Sp,
            arguments_offset as u64,
        ),
        "compute closure argument buffer",
    )?;
    store_register(
        &writer,
        Aarch64Register::X16,
        frame_offset("arguments"),
        "store closure argument buffer",
    )?;
    store_immediate(
        &writer,
        argument_count as u64,
        frame_offset("argument_count"),
    )?;

    for (index, argument) in layout.arguments.iter().enumerate() {
        let slot = arguments_offset + index * size_of::<jni::jvalue>();
        match argument.location {
            ClosureArgumentLocation::GeneralRegister(register) => match argument.value {
                ClosureValueLayout::General32 => store_register(
                    &writer,
                    general_32_register(register),
                    slot,
                    "capture general 32-bit closure argument",
                )?,
                ClosureValueLayout::General64 | ClosureValueLayout::Reference => store_register(
                    &writer,
                    general_64_register(register),
                    slot,
                    "capture general 64-bit closure argument",
                )?,
                ClosureValueLayout::Void
                | ClosureValueLayout::Float32
                | ClosureValueLayout::Float64 => unreachable!("invalid general argument layout"),
            },
            ClosureArgumentLocation::FloatRegister(register) => match argument.value {
                ClosureValueLayout::Float32 => store_register(
                    &writer,
                    float_32_register(register),
                    slot,
                    "capture float closure argument",
                )?,
                ClosureValueLayout::Float64 => store_register(
                    &writer,
                    float_64_register(register),
                    slot,
                    "capture double closure argument",
                )?,
                ClosureValueLayout::Void
                | ClosureValueLayout::General32
                | ClosureValueLayout::General64
                | ClosureValueLayout::Reference => unreachable!("invalid float argument layout"),
            },
            ClosureArgumentLocation::Stack { offset } => {
                let entry_stack_offset = frame_size + 16 + offset;
                ensure_closure_writer(
                    writer.put_ldr_reg_reg_offset(
                        Aarch64Register::X16,
                        Aarch64Register::Sp,
                        entry_stack_offset as u64,
                    ),
                    "load stack-passed closure argument",
                )?;
                store_register(
                    &writer,
                    Aarch64Register::X16,
                    slot,
                    "capture stack argument",
                )?;
            }
        }
    }

    ensure_closure_writer(
        writer.put_mov_reg_reg(Aarch64Register::X0, Aarch64Register::Sp),
        "pass closure invocation frame",
    )?;
    ensure_closure_writer(
        writer.put_ldr_reg_u64(
            Aarch64Register::X16,
            dispatch_closure_invocation as *const () as u64,
        ),
        "load closure replacement dispatcher",
    )?;
    ensure_closure_writer(
        put_blr_reg(&writer, Aarch64Register::X16),
        "call closure dispatcher",
    )?;

    load_return_value(&writer, layout.return_value, frame_offset("return_value"))?;
    ensure_closure_writer(
        writer.put_add_reg_reg_imm(Aarch64Register::Sp, Aarch64Register::Sp, frame_size as u64),
        "release closure invocation frame",
    )?;
    ensure_closure_writer(
        writer.put_pop_reg_reg(Aarch64Register::Fp, Aarch64Register::Lr),
        "restore closure trampoline frame",
    )?;
    put_ret(&writer);
    ensure_closure_writer(writer.flush(), "flush closure replacement trampoline")
}

fn store_immediate(writer: &Aarch64InstructionWriter, value: u64, offset: usize) -> Result<()> {
    ensure_closure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X16, value),
        "load closure frame immediate",
    )?;
    store_register(
        writer,
        Aarch64Register::X16,
        offset,
        "store closure frame immediate",
    )
}

fn store_register(
    writer: &Aarch64InstructionWriter,
    register: Aarch64Register,
    offset: usize,
    operation: &'static str,
) -> Result<()> {
    ensure_closure_writer(
        writer.put_str_reg_reg_offset(register, Aarch64Register::Sp, offset as u64),
        operation,
    )
}

fn load_return_value(
    writer: &Aarch64InstructionWriter,
    value: ClosureValueLayout,
    offset: usize,
) -> Result<()> {
    match value {
        ClosureValueLayout::Void => Ok(()),
        ClosureValueLayout::General32 => ensure_closure_writer(
            writer.put_ldr_reg_reg_offset(Aarch64Register::W0, Aarch64Register::Sp, offset as u64),
            "load 32-bit closure return",
        ),
        ClosureValueLayout::General64 | ClosureValueLayout::Reference => ensure_closure_writer(
            writer.put_ldr_reg_reg_offset(Aarch64Register::X0, Aarch64Register::Sp, offset as u64),
            "load 64-bit closure return",
        ),
        ClosureValueLayout::Float32 => ensure_closure_writer(
            writer.put_ldr_reg_reg_offset(Aarch64Register::S0, Aarch64Register::Sp, offset as u64),
            "load float closure return",
        ),
        ClosureValueLayout::Float64 => ensure_closure_writer(
            writer.put_ldr_reg_reg_offset(Aarch64Register::D0, Aarch64Register::Sp, offset as u64),
            "load double closure return",
        ),
    }
}

fn frame_offset(field: &str) -> usize {
    match field {
        "state" => offset_of!(ClosureInvocationFrame, state),
        "env" => offset_of!(ClosureInvocationFrame, env),
        "target" => offset_of!(ClosureInvocationFrame, target),
        "arguments" => offset_of!(ClosureInvocationFrame, arguments),
        "argument_count" => offset_of!(ClosureInvocationFrame, argument_count),
        "return_value" => offset_of!(ClosureInvocationFrame, return_value),
        _ => unreachable!("unknown closure invocation frame field"),
    }
}

fn general_64_register(register: u8) -> Aarch64Register {
    match register {
        0 => Aarch64Register::X0,
        1 => Aarch64Register::X1,
        2 => Aarch64Register::X2,
        3 => Aarch64Register::X3,
        4 => Aarch64Register::X4,
        5 => Aarch64Register::X5,
        6 => Aarch64Register::X6,
        7 => Aarch64Register::X7,
        _ => unreachable!("unsupported closure general register"),
    }
}

fn general_32_register(register: u8) -> Aarch64Register {
    match register {
        0 => Aarch64Register::W0,
        1 => Aarch64Register::W1,
        2 => Aarch64Register::W2,
        3 => Aarch64Register::W3,
        4 => Aarch64Register::W4,
        5 => Aarch64Register::W5,
        6 => Aarch64Register::W6,
        7 => Aarch64Register::W7,
        _ => unreachable!("unsupported closure general register"),
    }
}

fn float_32_register(register: u8) -> Aarch64Register {
    match register {
        0 => Aarch64Register::S0,
        1 => Aarch64Register::S1,
        2 => Aarch64Register::S2,
        3 => Aarch64Register::S3,
        4 => Aarch64Register::S4,
        5 => Aarch64Register::S5,
        6 => Aarch64Register::S6,
        7 => Aarch64Register::S7,
        _ => unreachable!("unsupported closure float register"),
    }
}

fn float_64_register(register: u8) -> Aarch64Register {
    match register {
        0 => Aarch64Register::D0,
        1 => Aarch64Register::D1,
        2 => Aarch64Register::D2,
        3 => Aarch64Register::D3,
        4 => Aarch64Register::D4,
        5 => Aarch64Register::D5,
        6 => Aarch64Register::D6,
        7 => Aarch64Register::D7,
        _ => unreachable!("unsupported closure float register"),
    }
}

fn put_blr_reg(writer: &Aarch64InstructionWriter, register: Aarch64Register) -> bool {
    unsafe { frida_gum_sys::gum_arm64_writer_put_blr_reg(writer.raw_writer(), register as u32) };
    true
}

fn put_ret(writer: &Aarch64InstructionWriter) {
    writer.put_bytes(&0xd65f_03c0_u32.to_le_bytes());
}

fn align_up(value: usize, align: usize) -> usize {
    debug_assert!(align.is_power_of_two());
    (value + align - 1) & !(align - 1)
}

fn closure_arguments_offset() -> usize {
    align_up(
        size_of::<ClosureInvocationFrame>(),
        align_of::<jni::jvalue>(),
    )
}

fn closure_invocation_frame_size(layout: &ClosureReplacementLayout) -> Option<usize> {
    let arguments_size = layout
        .arguments
        .len()
        .checked_mul(size_of::<jni::jvalue>())?;
    let unaligned = closure_arguments_offset().checked_add(arguments_size)?;
    let with_padding = unaligned.checked_add(15)?;
    Some(with_padding & !15)
}

mod jni {
    pub(super) use crate::jni::jvalue;
}

fn ensure_closure_writer(ok: bool, operation: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::UnsupportedFeature {
            feature: FEATURE_CLOSURE_REPLACEMENT,
            reason: format!("{operation} failed while generating closure trampoline"),
        })
    }
}
