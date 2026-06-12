use std::{
    ffi::{c_int, c_void},
    ptr::{self, NonNull},
};

use frida_gum::instruction_writer::{
    Aarch64BranchCondition, Aarch64InstructionWriter, Aarch64Register, InstructionWriter,
};

use super::super::{
    features::{FEATURE_METHOD_REPLACEMENT, unsupported_feature},
    layout::POINTER_SIZE,
    memory::{mmap, mprotect, munmap},
};
use crate::error::Result;

pub(in crate::art) struct ArtMethodDispatchThunk {
    pointer: NonNull<c_void>,
    length: usize,
}

pub(in crate::art) fn replacement_frame_is_active(
    replacement: usize,
    thread: usize,
    thread_managed_stack_offset: usize,
) -> bool {
    if replacement == 0 || thread == 0 {
        return false;
    }

    unsafe {
        let managed_stack = (thread + thread_managed_stack_offset) as *const usize;
        let top_quick_frame = ptr::read_unaligned(managed_stack) & !0x3usize;
        if top_quick_frame != 0 {
            return false;
        }

        let link = ptr::read_unaligned(managed_stack.byte_add(POINTER_SIZE));
        if link == 0 {
            return false;
        }

        let link_top_quick_frame = ptr::read_unaligned(link as *const usize) & !0x3usize;
        if link_top_quick_frame == 0 {
            return false;
        }

        ptr::read_unaligned(link_top_quick_frame as *const usize) == replacement
    }
}

pub(super) fn write_art_method_dispatch_thunk(
    code: *mut c_void,
    cloned_method: *mut c_void,
    original_dispatch_code: *mut c_void,
    quick_code_offset: usize,
    thread_managed_stack_offset: usize,
) -> Result<()> {
    const CHECK_LINK: u64 = 1;
    const ORIGINAL: u64 = 2;
    const REPLACEMENT: u64 = 3;

    let writer = Aarch64InstructionWriter::new(code as u64);

    put_cbz_label(&writer, Aarch64Register::X19, REPLACEMENT);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(
            Aarch64Register::X16,
            Aarch64Register::X19,
            thread_managed_stack_offset as u64,
        ),
        "emit managed-stack load",
    )?;
    put_cbz_label(&writer, Aarch64Register::X16, CHECK_LINK);
    writer.put_b_label(REPLACEMENT);

    writer.put_label(CHECK_LINK);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(
            Aarch64Register::X16,
            Aarch64Register::X19,
            (thread_managed_stack_offset + POINTER_SIZE) as u64,
        ),
        "emit managed-stack link load",
    )?;
    put_cbz_label(&writer, Aarch64Register::X16, REPLACEMENT);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(Aarch64Register::X16, Aarch64Register::X16, 0),
        "emit linked managed-stack quick-frame load",
    )?;
    write_replacement_frame_check(&writer, ORIGINAL, REPLACEMENT, cloned_method)?;

    writer.put_label(ORIGINAL);
    ensure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X16, original_dispatch_code as u64),
        "emit original dispatch load",
    )?;
    ensure_writer(
        writer.put_br_reg(Aarch64Register::X16),
        "emit original dispatch branch",
    )?;

    writer.put_label(REPLACEMENT);
    ensure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X0, cloned_method as u64),
        "emit cloned ArtMethod load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_reg_offset(
            Aarch64Register::X16,
            Aarch64Register::X0,
            quick_code_offset as u64,
        ),
        "emit cloned quick-entrypoint load",
    )?;
    ensure_writer(
        writer.put_br_reg(Aarch64Register::X16),
        "emit replacement dispatch branch",
    )?;
    writer.put_nop();

    ensure_writer(writer.flush(), "flush ART method dispatch thunk")
}

pub(super) fn write_replacement_frame_check(
    writer: &Aarch64InstructionWriter,
    original_label: u64,
    replacement_label: u64,
    cloned_method: *mut c_void,
) -> Result<()> {
    ensure_writer(
        put_and_reg_reg_imm(writer, Aarch64Register::X16, Aarch64Register::X16, !0x3u64),
        "emit managed-stack frame tag mask",
    )?;
    put_cbz_label(writer, Aarch64Register::X16, replacement_label);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(Aarch64Register::X16, Aarch64Register::X16, 0),
        "emit top quick-frame ArtMethod load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X17, cloned_method as u64),
        "emit cloned ArtMethod comparison load",
    )?;
    ensure_writer(
        writer.put_cmp_reg_reg(Aarch64Register::X16, Aarch64Register::X17),
        "emit cloned ArtMethod comparison",
    )?;
    writer.put_bcond_label(Aarch64BranchCondition::Eq, original_label);
    writer.put_b_label(replacement_label);
    Ok(())
}

pub(super) fn put_cbz_label(writer: &Aarch64InstructionWriter, reg: Aarch64Register, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_cbz_reg_label(
            writer.raw_writer(),
            reg as u32,
            label as *const c_void,
        );
    }
}

pub(super) fn put_and_reg_reg_imm(
    writer: &Aarch64InstructionWriter,
    dst: Aarch64Register,
    left: Aarch64Register,
    right: u64,
) -> bool {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_and_reg_reg_imm(
            writer.raw_writer(),
            dst as u32,
            left as u32,
            right,
        ) != 0
    }
}

pub(super) fn ensure_writer(ok: bool, operation: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("{operation} failed while generating dispatch thunk"),
        )
    }
}

impl ArtMethodDispatchThunk {
    pub(in crate::art) fn new(
        cloned_method: *mut c_void,
        original_dispatch_code: *mut c_void,
        quick_code_offset: usize,
        thread_managed_stack_offset: usize,
    ) -> Result<Self> {
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;
        const LENGTH: usize = 4096;

        if cloned_method.is_null() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "cloned ArtMethod is null for dispatch thunk",
            );
        }
        if original_dispatch_code.is_null() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "original ArtMethod dispatch entrypoint is null for dispatch thunk",
            );
        }
        if !quick_code_offset.is_multiple_of(POINTER_SIZE) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod quick entrypoint offset is not pointer-aligned",
            );
        }
        if !thread_managed_stack_offset.is_multiple_of(POINTER_SIZE) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART Thread managed stack offset is not pointer-aligned",
            );
        }
        if quick_code_offset / POINTER_SIZE > 0x0fff {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod quick entrypoint offset is too large for dispatch thunk",
            );
        }
        if thread_managed_stack_offset / POINTER_SIZE > 0x0fff {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART Thread managed stack offset is too large for dispatch thunk",
            );
        }
        if (thread_managed_stack_offset + POINTER_SIZE) / POINTER_SIZE > 0x0fff {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART Thread managed stack link offset is too large for dispatch thunk",
            );
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
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to allocate ArtMethod dispatch thunk",
            );
        }

        if let Err(error) = write_art_method_dispatch_thunk(
            pointer,
            cloned_method,
            original_dispatch_code,
            quick_code_offset,
            thread_managed_stack_offset,
        ) {
            unsafe { munmap(pointer, LENGTH) };
            return Err(error);
        }
        unsafe {
            frida_gum_sys::gum_clear_cache(pointer, LENGTH as u64);
            if mprotect(pointer, LENGTH, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, LENGTH);
                return unsupported_feature(
                    FEATURE_METHOD_REPLACEMENT,
                    "unable to protect ArtMethod dispatch thunk",
                );
            }
        }

        let Some(pointer) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, LENGTH) };
            return Err(crate::error::Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            pointer,
            length: LENGTH,
        })
    }

    pub(in crate::art) fn as_ptr(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }

    pub(in crate::art) fn len(&self) -> usize {
        self.length
    }

    #[cfg(test)]
    pub(in crate::art) fn from_pointer_for_tests(pointer: *mut c_void) -> Self {
        Self {
            pointer: NonNull::new(pointer).unwrap_or(NonNull::dangling()),
            length: 0,
        }
    }

    pub(super) fn leak(&mut self) {
        self.length = 0;
    }
}

impl Drop for ArtMethodDispatchThunk {
    fn drop(&mut self) {
        if self.length != 0 {
            unsafe {
                munmap(self.as_ptr(), self.length);
            }
        }
    }
}
