use std::{
    ffi::{c_int, c_void},
    ptr::{self, NonNull},
};

use frida_gum::instruction_writer::{Aarch64InstructionWriter, Aarch64Register, InstructionWriter};

use crate::{Error, Result};

use super::{
    FEATURE_CLOSURE_REPLACEMENT,
    closure::{
        ClosureReplacementAbi, ClosureReplacementState, closure_i32_i32_to_i32,
        closure_no_args_boolean, closure_no_args_byte, closure_no_args_char,
        closure_no_args_double, closure_no_args_float, closure_no_args_int, closure_no_args_long,
        closure_no_args_object, closure_no_args_short, closure_no_args_void,
        closure_one_reference_to_reference, closure_one_reference_to_void,
    },
};

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

impl ClosureReplacementThunk {
    pub(super) fn new(
        abi: ClosureReplacementAbi,
        state: *mut ClosureReplacementState,
    ) -> Result<Self> {
        let _gum = crate::runtime::process_gum();
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;
        const LENGTH: usize = 4096;

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

        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                LENGTH,
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

        if let Err(error) = write_closure_trampoline(pointer, state, abi) {
            unsafe { munmap(pointer, LENGTH) };
            return Err(error);
        }
        unsafe {
            frida_gum_sys::gum_clear_cache(pointer, LENGTH as u64);
            if mprotect(pointer, LENGTH, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, LENGTH);
                return Err(Error::UnsupportedFeature {
                    feature: FEATURE_CLOSURE_REPLACEMENT,
                    reason: "unable to protect closure replacement trampoline".to_owned(),
                });
            }
        }

        let Some(pointer) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, LENGTH) };
            return Err(Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            pointer,
            length: LENGTH,
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

fn write_closure_trampoline(
    code: *mut c_void,
    state: *mut ClosureReplacementState,
    abi: ClosureReplacementAbi,
) -> Result<()> {
    let _gum = crate::runtime::process_gum();
    let writer = Aarch64InstructionWriter::new(code as u64);
    match closure_trampoline_extra_arg_count(abi) {
        0 => {
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X2, Aarch64Register::X1),
                "move JNI target",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X1, Aarch64Register::X0),
                "move JNI env",
            )?;
        }
        1 => {
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X3, Aarch64Register::X2),
                "move first JNI argument",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X2, Aarch64Register::X1),
                "move JNI target",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X1, Aarch64Register::X0),
                "move JNI env",
            )?;
        }
        2 => {
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X4, Aarch64Register::X3),
                "move second JNI argument",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X3, Aarch64Register::X2),
                "move first JNI argument",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X2, Aarch64Register::X1),
                "move JNI target",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X1, Aarch64Register::X0),
                "move JNI env",
            )?;
        }
        _ => unreachable!("closure replacement supports at most two Java arguments"),
    }
    ensure_closure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X0, state as u64),
        "load closure replacement state",
    )?;
    ensure_closure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X16, closure_handler_for_abi(abi) as u64),
        "load closure replacement handler",
    )?;
    ensure_closure_writer(
        writer.put_br_reg(Aarch64Register::X16),
        "branch to closure replacement handler",
    )?;
    ensure_closure_writer(writer.flush(), "flush closure replacement trampoline")
}

fn closure_trampoline_extra_arg_count(abi: ClosureReplacementAbi) -> usize {
    match abi {
        ClosureReplacementAbi::NoArgsVoid
        | ClosureReplacementAbi::NoArgsBoolean
        | ClosureReplacementAbi::NoArgsByte
        | ClosureReplacementAbi::NoArgsChar
        | ClosureReplacementAbi::NoArgsShort
        | ClosureReplacementAbi::NoArgsInt
        | ClosureReplacementAbi::NoArgsLong
        | ClosureReplacementAbi::NoArgsFloat
        | ClosureReplacementAbi::NoArgsDouble
        | ClosureReplacementAbi::NoArgsObject => 0,
        ClosureReplacementAbi::OneReferenceToReference
        | ClosureReplacementAbi::OneReferenceToVoid => 1,
        ClosureReplacementAbi::I32I32ToI32 => 2,
    }
}

fn closure_handler_for_abi(abi: ClosureReplacementAbi) -> *const c_void {
    match abi {
        ClosureReplacementAbi::NoArgsVoid => closure_no_args_void as *const c_void,
        ClosureReplacementAbi::NoArgsBoolean => closure_no_args_boolean as *const c_void,
        ClosureReplacementAbi::NoArgsByte => closure_no_args_byte as *const c_void,
        ClosureReplacementAbi::NoArgsChar => closure_no_args_char as *const c_void,
        ClosureReplacementAbi::NoArgsShort => closure_no_args_short as *const c_void,
        ClosureReplacementAbi::NoArgsInt => closure_no_args_int as *const c_void,
        ClosureReplacementAbi::NoArgsLong => closure_no_args_long as *const c_void,
        ClosureReplacementAbi::NoArgsFloat => closure_no_args_float as *const c_void,
        ClosureReplacementAbi::NoArgsDouble => closure_no_args_double as *const c_void,
        ClosureReplacementAbi::NoArgsObject => closure_no_args_object as *const c_void,
        ClosureReplacementAbi::OneReferenceToReference => {
            closure_one_reference_to_reference as *const c_void
        }
        ClosureReplacementAbi::OneReferenceToVoid => closure_one_reference_to_void as *const c_void,
        ClosureReplacementAbi::I32I32ToI32 => closure_i32_i32_to_i32 as *const c_void,
    }
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
