use std::{
    ffi::{c_int, c_void},
    ptr::{self, NonNull},
};

use super::super::{
    features::{FEATURE_METHOD_REPLACEMENT, unsupported_feature},
    layout::*,
    memory::{MemoryRange, MemoryRanges, mmap, munmap},
};
use crate::{
    env::MethodKind,
    error::{Error, Result},
};

pub(in crate::art) struct ArtMethodClone {
    method: NonNull<c_void>,
    length: usize,
}

impl ArtMethodClone {
    pub(in crate::art) fn copy_from(
        method: *mut c_void,
        layout: &ArtMethodRuntimeLayout,
        memory: &MemoryRanges,
    ) -> Result<Self> {
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;

        if method.is_null() || !memory.contains(method as usize, layout.method_size) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "target ArtMethod is not readable for cloning",
            );
        }
        if layout.method_size == 0 {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "target ArtMethod clone size is zero",
            );
        }

        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                layout.method_size,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to allocate cloned ArtMethod",
            );
        }

        unsafe {
            ptr::copy_nonoverlapping(
                method.cast::<u8>(),
                pointer.cast::<u8>(),
                layout.method_size,
            );
        }
        let Some(method) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, layout.method_size) };
            return Err(Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            method,
            length: layout.method_size,
        })
    }

    pub(in crate::art) fn as_ptr(&self) -> *mut c_void {
        self.method.as_ptr()
    }

    pub(in crate::art) fn memory_ranges(&self) -> MemoryRanges {
        MemoryRanges {
            ranges: vec![MemoryRange {
                start: self.as_ptr() as usize,
                end: self.as_ptr() as usize + self.length,
                writable: false,
                executable: false,
            }],
        }
    }

    pub(super) fn leak(&mut self) {
        self.length = 0;
    }
}

impl Drop for ArtMethodClone {
    fn drop(&mut self) {
        if self.length != 0 {
            unsafe {
                munmap(self.as_ptr(), self.length);
            }
        }
    }
}

pub(in crate::art) fn snapshot_art_method(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    memory: &MemoryRanges,
) -> Result<ArtMethodSnapshot> {
    if method.is_null() || !memory.contains(method as usize, layout.method_size) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "target ArtMethod is not readable",
        );
    }

    let access_flags = read_u32(
        unsafe { method.byte_add(layout.access_flags_offset) },
        memory,
    )
    .ok_or_else(|| Error::UnsupportedFeature {
        feature: FEATURE_METHOD_REPLACEMENT,
        reason: "target ArtMethod access flags are not readable".to_owned(),
    })?;
    let jni_code = read_usize(unsafe { method.byte_add(layout.jni_code_offset) }, memory)
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "target ArtMethod JNI entrypoint is not readable".to_owned(),
        })? as *mut c_void;
    let quick_code = read_usize(unsafe { method.byte_add(layout.quick_code_offset) }, memory)
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "target ArtMethod quick entrypoint is not readable".to_owned(),
        })? as *mut c_void;
    let interpreter_code = layout
        .interpreter_code_offset
        .map(|offset| {
            read_usize(unsafe { method.byte_add(offset) }, memory)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: "target ArtMethod interpreter entrypoint is not readable".to_owned(),
                })
                .map(|value| value as *mut c_void)
        })
        .transpose()?;

    Ok(ArtMethodSnapshot {
        access_flags,
        jni_code,
        quick_code,
        interpreter_code,
    })
}

pub(in crate::art) fn validate_replacement_function(
    replacement: *mut c_void,
    memory: &MemoryRanges,
) -> Result<()> {
    if replacement.is_null() {
        return Err(Error::NullReturn {
            operation: "ART replacement function",
        });
    }
    if !memory.contains_executable(replacement as usize, 1) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "replacement function is not executable",
        );
    }
    Ok(())
}

pub(in crate::art) fn validate_replacement_trampoline(
    trampolines: &ArtClassLinkerTrampolines,
    memory: &MemoryRanges,
) -> Result<()> {
    let trampoline = trampolines.quick_generic_jni_trampoline;
    if trampoline.is_null() || !memory.contains_executable(trampoline as usize, 1) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "ClassLinker quick generic JNI trampoline is unavailable or not executable",
        );
    }
    Ok(())
}

pub(in crate::art) fn art_method_kind_matches(
    snapshot: ArtMethodSnapshot,
    kind: MethodKind,
) -> bool {
    match kind {
        MethodKind::Static => snapshot.access_flags & K_ACC_STATIC != 0,
        MethodKind::Instance => snapshot.access_flags & (K_ACC_STATIC | K_ACC_CONSTRUCTOR) == 0,
        MethodKind::Constructor => snapshot.access_flags & K_ACC_CONSTRUCTOR != 0,
    }
}

pub(in crate::art) fn patched_replacement_method(
    original: ArtMethodSnapshot,
    replacement: *mut c_void,
    quick_generic_jni_trampoline: *mut c_void,
    compile_dont_bother: u32,
) -> ArtMethodSnapshot {
    let removed_flags = K_ACC_CRITICAL_NATIVE
        | K_ACC_FAST_NATIVE
        | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
        | K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_SINGLE_IMPLEMENTATION
        | K_ACC_SKIP_ACCESS_CHECKS;
    ArtMethodSnapshot {
        access_flags: (original.access_flags & !removed_flags) | K_ACC_NATIVE | compile_dont_bother,
        jni_code: replacement,
        quick_code: quick_generic_jni_trampoline,
        interpreter_code: original.interpreter_code,
    }
}

pub(in crate::art) fn patched_original_method_for_clone_dispatch(
    original: ArtMethodSnapshot,
    quick_to_interpreter_bridge_trampoline: *mut c_void,
    compile_dont_bother: u32,
) -> ArtMethodSnapshot {
    let removed_flags = K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
        | K_ACC_SINGLE_IMPLEMENTATION
        | K_ACC_SKIP_ACCESS_CHECKS;
    ArtMethodSnapshot {
        access_flags: (original.access_flags & !removed_flags) | compile_dont_bother,
        jni_code: original.jni_code,
        quick_code: quick_to_interpreter_bridge_trampoline,
        interpreter_code: original.interpreter_code,
    }
}

pub(in crate::art) fn patch_art_method_verified(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    original: ArtMethodSnapshot,
    patched: ArtMethodSnapshot,
    memory: &MemoryRanges,
) -> Result<()> {
    patch_art_method(method, layout, patched);
    match snapshot_art_method(method, layout, memory) {
        Ok(snapshot) if snapshot == patched => Ok(()),
        Ok(snapshot) => {
            patch_art_method(method, layout, original);
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: format!(
                    "target ArtMethod patch verification failed: expected {patched:?}, got {snapshot:?}"
                ),
            })
        }
        Err(error) => {
            patch_art_method(method, layout, original);
            Err(error)
        }
    }
}

pub(in crate::art) fn clone_replacement_art_method(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    original: ArtMethodSnapshot,
    patched: ArtMethodSnapshot,
    memory: &MemoryRanges,
) -> Result<ArtMethodClone> {
    let cloned_method = ArtMethodClone::copy_from(method, layout, memory)?;
    let clone_memory = cloned_method.memory_ranges();
    match snapshot_art_method(cloned_method.as_ptr(), layout, &clone_memory) {
        Ok(snapshot) if snapshot == original => {}
        Ok(snapshot) => {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                format!(
                    "cloned ArtMethod snapshot mismatch: expected {original:?}, got {snapshot:?}"
                ),
            );
        }
        Err(error) => return Err(error),
    }
    patch_art_method_verified(
        cloned_method.as_ptr(),
        layout,
        original,
        patched,
        &clone_memory,
    )?;
    Ok(cloned_method)
}

pub(in crate::art) fn restore_art_method_verified(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    original: ArtMethodSnapshot,
    memory: &MemoryRanges,
) -> Result<()> {
    patch_art_method(method, layout, original);
    match snapshot_art_method(method, layout, memory) {
        Ok(snapshot) if snapshot == original => Ok(()),
        Ok(snapshot) => Err(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: format!(
                "target ArtMethod restore verification failed: expected {original:?}, got {snapshot:?}"
            ),
        }),
        Err(error) => Err(error),
    }
}

pub(in crate::art) fn patch_art_method(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    snapshot: ArtMethodSnapshot,
) {
    write_u32(
        unsafe { method.byte_add(layout.access_flags_offset) },
        snapshot.access_flags,
    );
    write_usize(
        unsafe { method.byte_add(layout.jni_code_offset) },
        snapshot.jni_code as usize,
    );
    write_usize(
        unsafe { method.byte_add(layout.quick_code_offset) },
        snapshot.quick_code as usize,
    );
    if let (Some(offset), Some(interpreter_code)) =
        (layout.interpreter_code_offset, snapshot.interpreter_code)
    {
        write_usize(
            unsafe { method.byte_add(offset) },
            interpreter_code as usize,
        );
    }
}

pub(in crate::art) fn compile_dont_bother_flag(api_level: i32) -> u32 {
    if api_level >= 27 {
        0x02000000
    } else if api_level >= 24 {
        0x01000000
    } else {
        0
    }
}
