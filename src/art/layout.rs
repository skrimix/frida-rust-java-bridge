use std::{ffi::c_void, ptr};

use frida_gum::Module;

use super::{features::*, memory::MemoryRanges};
use crate::error::{Error, Result};

pub(super) const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
pub(super) const STD_STRING_SIZE: usize = 3 * POINTER_SIZE;
pub(super) const K_POINTER_JNI_ID_TYPE: i32 = 0;
pub(super) const K_ACC_PUBLIC: u32 = 0x0001;
pub(super) const K_ACC_STATIC: u32 = 0x0008;
pub(super) const K_ACC_FINAL: u32 = 0x0010;
pub(super) const K_ACC_NATIVE: u32 = 0x0100;
pub(super) const K_ACC_FAST_NATIVE: u32 = 0x00080000;
pub(super) const K_ACC_CRITICAL_NATIVE: u32 = 0x00200000;
pub(super) const K_ACC_JAVA_FLAGS_MASK: u32 = 0xffff;
pub(super) const K_ACC_CONSTRUCTOR: u32 = 0x00010000;
pub(super) const K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE: u32 = 0x40000000;
pub(super) const K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG: u32 = 0x00100000;
pub(super) const K_ACC_NTERP_INVOKE_FAST_PATH_FLAG: u32 = 0x00200000;
pub(super) const K_ACC_PUBLIC_API: u32 = 0x10000000;
pub(super) const K_ACC_SKIP_ACCESS_CHECKS: u32 = 0x00080000;
pub(super) const K_ACC_SINGLE_IMPLEMENTATION: u32 = 0x08000000;
pub(super) const CLASS_LAYOUT_SCAN_LIMIT: usize = 0x100;
pub(super) const METHOD_LAYOUT_SCAN_LIMIT: usize = 64;
pub(super) const ART_METHOD_MIN_SIZE: usize = 16;
pub(super) const ART_METHOD_MAX_SIZE: usize = 256;
pub(super) const ART_METHOD_ARRAY_MAX_PROBE: usize = 100;

pub(super) type ArtQuickEntrypointPredicate =
    unsafe extern "C" fn(*mut c_void, *const c_void) -> bool;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtModuleRange {
    pub(super) start: usize,
    pub(super) end: usize,
}

impl ArtModuleRange {
    pub(crate) fn from_module(module: &Module) -> Self {
        let range = module.range();
        let start = range.base_address().0 as usize;
        let end = start.saturating_add(range.size());
        Self { start, end }
    }

    pub(super) fn contains(&self, address: usize) -> bool {
        let address = normalize_address(address);
        let start = normalize_address(self.start);
        let end = normalize_address(self.end);
        address >= start && address < end
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtRuntimeLayout {
    pub(super) runtime: *mut c_void,
    pub(super) heap: *mut c_void,
    pub(super) thread_list: *mut c_void,
    pub(super) class_linker: *mut c_void,
    pub(super) intern_table: *mut c_void,
    pub(super) jni_id_manager: *mut c_void,
    pub(super) jni_ids_indirection: Option<i32>,
}

impl ArtRuntimeLayout {
    pub(super) fn uses_indirect_jni_ids(&self) -> bool {
        !self.jni_id_manager.is_null()
            && self
                .jni_ids_indirection
                .is_some_and(|indirection| indirection != K_POINTER_JNI_ID_TYPE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtMethodQueryLayout {
    pub(super) class_methods_offset: usize,
    pub(super) class_copied_methods_offset: usize,
    pub(super) method_size: usize,
    pub(super) method_access_flags_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtArray {
    pub(super) data: *mut c_void,
    pub(super) length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtMethodRuntimeLayout {
    pub(super) method_size: usize,
    pub(super) access_flags_offset: usize,
    pub(super) jni_code_offset: usize,
    pub(super) quick_code_offset: usize,
    pub(super) interpreter_code_offset: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtClassLinkerTrampolines {
    pub(super) quick_resolution_trampoline: *mut c_void,
    pub(super) quick_imt_conflict_trampoline: *mut c_void,
    pub(super) quick_generic_jni_trampoline: *mut c_void,
    pub(super) quick_to_interpreter_bridge_trampoline: *mut c_void,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ArtClassLinkerEntrypointPredicates {
    pub(super) is_quick_resolution_stub: ArtQuickEntrypointPredicate,
    pub(super) is_quick_to_interpreter_bridge: ArtQuickEntrypointPredicate,
    pub(super) is_quick_generic_jni_stub: ArtQuickEntrypointPredicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtMethodReplacementLayout {
    pub(super) api_level: i32,
    pub(super) runtime: ArtRuntimeLayout,
    pub(super) method: ArtMethodRuntimeLayout,
    pub(super) trampolines: ArtClassLinkerTrampolines,
    pub(super) thread_managed_stack_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtDeoptimizationLayout {
    pub(super) api_level: i32,
    pub(super) runtime: ArtRuntimeLayout,
    pub(super) instrumentation: Option<*mut c_void>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtMethodSnapshot {
    pub(super) access_flags: u32,
    pub(super) jni_code: *mut c_void,
    pub(super) quick_code: *mut c_void,
    pub(super) interpreter_code: Option<*mut c_void>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ClassLinkerTrampolineOffsets {
    pub(super) quick_resolution: usize,
    pub(super) quick_imt_conflict: usize,
    pub(super) quick_generic_jni: usize,
    pub(super) quick_to_interpreter_bridge: usize,
}

pub(super) fn detect_thread_class_method_layout(
    thread_class: *mut c_void,
    method_candidates: &[Vec<*mut c_void>],
    method_size: usize,
    memory: &MemoryRanges,
) -> Result<ArtMethodQueryLayout> {
    for methods_offset in (0..CLASS_LAYOUT_SCAN_LIMIT).step_by(4) {
        let Some(array) = read_art_array(thread_class, methods_offset, POINTER_SIZE, memory) else {
            continue;
        };
        if array.length == 0 || array.length > ART_METHOD_ARRAY_MAX_PROBE {
            continue;
        }

        let Some(array_bytes) = array.length.checked_mul(method_size) else {
            continue;
        };
        if !memory.contains(array.data as usize, array_bytes) {
            continue;
        }
        if !art_method_array_contains_all(array, method_size, method_candidates) {
            continue;
        }

        let copied_methods_offset =
            detect_copied_methods_offset(thread_class, methods_offset, array.length, memory)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_QUERY,
                    reason: "unable to determine mirror::Class copied-method count offset"
                        .to_owned(),
                })?;
        return Ok(ArtMethodQueryLayout {
            class_methods_offset: methods_offset,
            class_copied_methods_offset: copied_methods_offset,
            method_size,
            method_access_flags_offset: 0,
        });
    }

    unsupported_method_query("unable to determine mirror::Class methods layout")
}

pub(super) fn detect_class_linker_trampolines(
    layout: &ArtRuntimeLayout,
    api_level: i32,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
) -> Result<ArtClassLinkerTrampolines> {
    if layout.intern_table.is_null() {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "ART Runtime intern table pointer is null",
        );
    }

    let start_offset = if POINTER_SIZE == 4 { 100 } else { 200 };
    let end_offset = start_offset + (100 * POINTER_SIZE);
    for offset in (start_offset..end_offset).step_by(POINTER_SIZE) {
        let Some(value) = read_usize(
            (layout.class_linker as usize + offset) as *const c_void,
            memory,
        ) else {
            continue;
        };
        if value != layout.intern_table as usize {
            continue;
        }

        let offsets = class_linker_trampoline_offsets_from_anchor(api_level, offset);
        return read_class_linker_trampolines(layout.class_linker, offsets, memory);
    }

    detect_class_linker_trampolines_by_predicate(layout, predicates, memory)
}

pub(super) fn detect_class_linker_trampolines_by_predicate(
    layout: &ArtRuntimeLayout,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
) -> Result<ArtClassLinkerTrampolines> {
    let Some(predicates) = predicates else {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "unable to determine ClassLinker trampoline offsets: intern table anchor was not found and ClassLinker quick-entrypoint predicate symbols are unavailable",
        );
    };

    let start_offset = if POINTER_SIZE == 4 { 100 } else { 200 };
    let end_offset = start_offset + (512 * POINTER_SIZE);
    let mut candidate = None;
    for quick_resolution_offset in
        (start_offset..end_offset - (3 * POINTER_SIZE)).step_by(POINTER_SIZE)
    {
        let Some(quick_resolution) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset) as *const c_void,
            memory,
        ) else {
            continue;
        };
        let Some(_quick_imt_conflict) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset + POINTER_SIZE)
                as *const c_void,
            memory,
        ) else {
            continue;
        };
        let Some(quick_generic_jni) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset + (2 * POINTER_SIZE))
                as *const c_void,
            memory,
        ) else {
            continue;
        };
        let Some(quick_to_interpreter) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset + (3 * POINTER_SIZE))
                as *const c_void,
            memory,
        ) else {
            continue;
        };

        let class_linker = normalize_address(layout.class_linker as usize) as *mut c_void;
        let is_match = unsafe {
            (predicates.is_quick_resolution_stub)(class_linker, quick_resolution as *const c_void)
                && (predicates.is_quick_generic_jni_stub)(
                    class_linker,
                    quick_generic_jni as *const c_void,
                )
                && (predicates.is_quick_to_interpreter_bridge)(
                    class_linker,
                    quick_to_interpreter as *const c_void,
                )
        };
        if !is_match {
            continue;
        }

        let offsets =
            class_linker_trampoline_offsets_from_quick_resolution(quick_resolution_offset);
        let trampolines = read_class_linker_trampolines(layout.class_linker, offsets, memory)?;
        if candidate.replace(trampolines).is_some() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to determine ClassLinker trampoline offsets: predicate scan found multiple candidates",
            );
        }
    }

    if let Some(trampolines) = candidate {
        return Ok(trampolines);
    }
    unsupported_feature(
        FEATURE_METHOD_REPLACEMENT,
        "unable to determine ClassLinker trampoline offsets: intern table anchor was not found and predicate scan found no quick trampoline sequence",
    )
}

pub(super) fn class_linker_trampoline_offsets_from_anchor(
    api_level: i32,
    intern_table_anchor_offset: usize,
) -> ClassLinkerTrampolineOffsets {
    let delta = if api_level >= 30 {
        6
    } else if api_level >= 29 {
        4
    } else {
        3
    };
    let quick_generic_jni = intern_table_anchor_offset + (delta * POINTER_SIZE);
    let quick_resolution = if api_level >= 23 {
        quick_generic_jni - (2 * POINTER_SIZE)
    } else {
        quick_generic_jni - (3 * POINTER_SIZE)
    };

    ClassLinkerTrampolineOffsets {
        quick_resolution,
        quick_imt_conflict: quick_generic_jni - POINTER_SIZE,
        quick_generic_jni,
        quick_to_interpreter_bridge: quick_generic_jni + POINTER_SIZE,
    }
}

pub(super) fn class_linker_trampoline_offsets_from_quick_resolution(
    quick_resolution: usize,
) -> ClassLinkerTrampolineOffsets {
    ClassLinkerTrampolineOffsets {
        quick_resolution,
        quick_imt_conflict: quick_resolution + POINTER_SIZE,
        quick_generic_jni: quick_resolution + (2 * POINTER_SIZE),
        quick_to_interpreter_bridge: quick_resolution + (3 * POINTER_SIZE),
    }
}

pub(super) fn read_class_linker_trampolines(
    class_linker: *mut c_void,
    offsets: ClassLinkerTrampolineOffsets,
    memory: &MemoryRanges,
) -> Result<ArtClassLinkerTrampolines> {
    Ok(ArtClassLinkerTrampolines {
        quick_resolution_trampoline: read_trampoline(
            class_linker,
            offsets.quick_resolution,
            memory,
            "quick resolution trampoline",
        )?,
        quick_imt_conflict_trampoline: read_trampoline(
            class_linker,
            offsets.quick_imt_conflict,
            memory,
            "quick IMT conflict trampoline",
        )?,
        quick_generic_jni_trampoline: read_trampoline(
            class_linker,
            offsets.quick_generic_jni,
            memory,
            "quick generic JNI trampoline",
        )?,
        quick_to_interpreter_bridge_trampoline: read_trampoline(
            class_linker,
            offsets.quick_to_interpreter_bridge,
            memory,
            "quick-to-interpreter bridge trampoline",
        )?,
    })
}

pub(super) fn read_trampoline(
    class_linker: *mut c_void,
    offset: usize,
    memory: &MemoryRanges,
    name: &'static str,
) -> Result<*mut c_void> {
    let Some(value) = read_usize((class_linker as usize + offset) as *const c_void, memory) else {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("unable to read ClassLinker {name} at offset {offset:#x}"),
        );
    };
    if value == 0 {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("ClassLinker {name} at offset {offset:#x} is null"),
        );
    }
    if !memory.contains_executable(value, 1) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("ClassLinker {name} at offset {offset:#x} is not executable"),
        );
    }
    Ok(value as *mut c_void)
}

pub(super) fn art_method_array_contains_all(
    array: ArtArray,
    method_size: usize,
    method_candidates: &[Vec<*mut c_void>],
) -> bool {
    method_candidates.iter().all(|candidates| {
        (0..array.length).any(|index| {
            let method = unsafe { array.data.byte_add(index * method_size) };
            candidates.contains(&method)
        })
    })
}

pub(super) fn detect_copied_methods_offset(
    class: *mut c_void,
    methods_offset: usize,
    method_count: usize,
    memory: &MemoryRanges,
) -> Option<usize> {
    if method_count > u16::MAX as usize {
        return None;
    }
    for offset in (methods_offset..CLASS_LAYOUT_SCAN_LIMIT).step_by(4) {
        let value = read_u16((class as usize + offset) as *const c_void, memory)?;
        if value as usize == method_count {
            return Some(offset);
        }
    }
    None
}

pub(super) fn detect_art_method_runtime_layout(
    method_candidates: &[*mut c_void],
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<ArtMethodRuntimeLayout> {
    let expected_native = 0x0001 | K_ACC_STATIC | K_ACC_NATIVE;
    let expected_final_native = expected_native | K_ACC_FINAL;
    let mask = !(K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_PUBLIC_API
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG);
    for &method in method_candidates {
        if method.is_null() || !memory.contains(method as usize, METHOD_LAYOUT_SCAN_LIMIT) {
            continue;
        }
        let mut access_flags_offset = None;
        for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(4) {
            let Some(flags) = read_u32((method as usize + offset) as *const c_void, memory) else {
                continue;
            };
            let relevant_flags = flags & mask;
            if relevant_flags == expected_native || relevant_flags == expected_final_native {
                access_flags_offset = Some(offset);
                break;
            }
        }

        let Some(access_flags_offset) = access_flags_offset else {
            continue;
        };
        let Some(entrypoints) = detect_art_method_entrypoints(method, memory) else {
            continue;
        };
        return Ok(ArtMethodRuntimeLayout {
            method_size: entrypoints.method_size,
            access_flags_offset,
            jni_code_offset: entrypoints.jni_code_offset,
            quick_code_offset: entrypoints.quick_code_offset,
            interpreter_code_offset: entrypoints.interpreter_code_offset,
        });
    }

    unsupported_feature(feature, "unable to determine ArtMethod runtime layout")
}

pub(super) fn detect_art_method_replacement_layout(
    method_candidates: &[*mut c_void],
    native_runtime: ArtModuleRange,
    api_level: i32,
    memory: &MemoryRanges,
    allow_executable_entrypoint_fallback: bool,
    feature: &'static str,
) -> Result<ArtMethodRuntimeLayout> {
    let expected_native = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
    let expected_non_final_native = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE;
    let entrypoint_field_size = if api_level <= 21 { 8 } else { POINTER_SIZE };
    let mut evidence = ReplacementMethodLayoutEvidence::default();

    for &method in method_candidates {
        if method.is_null() || !memory.contains(method as usize, METHOD_LAYOUT_SCAN_LIMIT) {
            continue;
        }
        evidence.saw_readable_candidate();

        let mut jni_code_offset = None;
        let mut executable_jni_code_offset = None;
        let mut access_flags_offset = None;
        for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(4) {
            if jni_code_offset.is_none()
                && let Some(address) =
                    read_usize((method as usize + offset) as *const c_void, memory)
            {
                if native_runtime.contains(address) {
                    jni_code_offset = Some(offset);
                    evidence.saw_native_runtime_entrypoint();
                } else if executable_jni_code_offset.is_none()
                    && allow_executable_entrypoint_fallback
                    && memory.contains_executable(address, 1)
                {
                    executable_jni_code_offset = Some(offset);
                    evidence.saw_executable_entrypoint();
                }
            }

            if access_flags_offset.is_none()
                && let Some(flags) = read_u32((method as usize + offset) as *const c_void, memory)
                && matches!(
                    flags & K_ACC_JAVA_FLAGS_MASK,
                    value if value == expected_native || value == expected_non_final_native
                )
            {
                access_flags_offset = Some(offset);
                evidence.saw_access_flags();
            }

            if jni_code_offset.is_some() && access_flags_offset.is_some() {
                break;
            }
        }

        let jni_code_offset = jni_code_offset.or(executable_jni_code_offset);
        let (Some(jni_code_offset), Some(access_flags_offset)) =
            (jni_code_offset, access_flags_offset)
        else {
            continue;
        };

        let Some(quick_code_offset) = jni_code_offset.checked_add(entrypoint_field_size) else {
            continue;
        };
        let Some(method_size) =
            quick_code_offset.checked_add(if api_level <= 21 { 32 } else { POINTER_SIZE })
        else {
            continue;
        };
        if !(ART_METHOD_MIN_SIZE..=ART_METHOD_MAX_SIZE).contains(&method_size)
            || !memory.contains(method as usize, method_size)
        {
            continue;
        }

        return Ok(ArtMethodRuntimeLayout {
            method_size,
            access_flags_offset,
            jni_code_offset,
            quick_code_offset,
            interpreter_code_offset: None,
        });
    }

    unsupported_feature(
        feature,
        evidence.unsupported_reason(allow_executable_entrypoint_fallback),
    )
}

#[derive(Default)]
struct ReplacementMethodLayoutEvidence {
    has_readable_candidate: bool,
    has_native_runtime_entrypoint: bool,
    has_executable_entrypoint: bool,
    has_access_flags: bool,
}

impl ReplacementMethodLayoutEvidence {
    fn saw_readable_candidate(&mut self) {
        self.has_readable_candidate = true;
    }

    fn saw_native_runtime_entrypoint(&mut self) {
        self.has_native_runtime_entrypoint = true;
        self.has_executable_entrypoint = true;
    }

    fn saw_executable_entrypoint(&mut self) {
        self.has_executable_entrypoint = true;
    }

    fn saw_access_flags(&mut self) {
        self.has_access_flags = true;
    }

    fn unsupported_reason(&self, allow_executable_entrypoint_fallback: bool) -> &'static str {
        if !self.has_readable_candidate {
            "unable to determine ArtMethod runtime layout: no readable method candidates"
        } else if !self.has_executable_entrypoint {
            "unable to determine ArtMethod runtime layout: native entrypoint is not executable"
        } else if !self.has_native_runtime_entrypoint && !allow_executable_entrypoint_fallback {
            "unable to determine ArtMethod runtime layout: native entrypoint is outside libandroid_runtime.so"
        } else if !self.has_access_flags {
            "unable to determine ArtMethod runtime layout: native access flags were not found"
        } else {
            "unable to determine ArtMethod runtime layout: derived layout is not readable"
        }
    }
}

pub(super) fn detect_art_thread_managed_stack_offset(
    feature: &'static str,
    thread: *mut c_void,
    env: *mut c_void,
) -> Result<usize> {
    if thread.is_null() {
        return unsupported_feature(feature, "ART Thread pointer is null");
    }

    let thread = thread.cast::<usize>();
    let env_value = env as usize;
    for offset in (144..256).step_by(POINTER_SIZE) {
        let value = unsafe { thread.byte_add(offset).read() };
        if value == env_value {
            return offset
                .checked_sub(4 * POINTER_SIZE)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature,
                    reason: "ART Thread managed stack offset underflowed".to_owned(),
                });
        }
    }

    unsupported_feature(
        feature,
        "unable to determine ART Thread managed stack offset",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtMethodEntrypoints {
    method_size: usize,
    jni_code_offset: usize,
    quick_code_offset: usize,
    interpreter_code_offset: Option<usize>,
}

pub(super) fn detect_art_method_entrypoints(
    method: *mut c_void,
    memory: &MemoryRanges,
) -> Option<ArtMethodEntrypoints> {
    let mut previous_executable_pointer_offset: Option<usize> = None;
    for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(POINTER_SIZE) {
        let value = read_usize((method as usize + offset) as *const c_void, memory)?;
        if !memory.contains_executable(value, 1) {
            continue;
        }

        if let Some(previous) = previous_executable_pointer_offset
            && offset == previous + POINTER_SIZE
        {
            let size = offset + POINTER_SIZE;
            if (ART_METHOD_MIN_SIZE..=ART_METHOD_MAX_SIZE).contains(&size) {
                let interpreter_code_offset =
                    previous.checked_sub(POINTER_SIZE).filter(|&offset| {
                        let pointer =
                            read_usize((method as usize + offset) as *const c_void, memory);
                        pointer.is_some_and(|pointer| memory.contains_executable(pointer, 1))
                    });
                return Some(ArtMethodEntrypoints {
                    method_size: size,
                    jni_code_offset: previous,
                    quick_code_offset: offset,
                    interpreter_code_offset,
                });
            }
        }
        previous_executable_pointer_offset = Some(offset);
    }

    None
}

pub(super) fn read_art_array(
    object: *mut c_void,
    offset: usize,
    length_size: usize,
    memory: &MemoryRanges,
) -> Option<ArtArray> {
    let header = read_usize((object as usize + offset) as *const c_void, memory)? as *mut c_void;
    if header.is_null() || !memory.contains(header as usize, length_size) {
        return None;
    }

    let length = if length_size == 4 {
        read_u32(header.cast(), memory)? as usize
    } else {
        read_usize(header.cast(), memory)?
    };
    if length == 0 {
        return None;
    }

    let data = unsafe { header.byte_add(length_size) };
    Some(ArtArray { data, length })
}

pub(super) fn read_usize(pointer: *const c_void, memory: &MemoryRanges) -> Option<usize> {
    let address = normalize_address(pointer as usize);
    if !memory.contains(address, POINTER_SIZE) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(address as *const usize) })
}

pub(super) fn read_u32(pointer: *const c_void, memory: &MemoryRanges) -> Option<u32> {
    let address = normalize_address(pointer as usize);
    if !memory.contains(address, std::mem::size_of::<u32>()) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(address as *const u32) })
}

pub(super) fn read_u16(pointer: *const c_void, memory: &MemoryRanges) -> Option<u16> {
    let address = normalize_address(pointer as usize);
    if !memory.contains(address, std::mem::size_of::<u16>()) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(address as *const u16) })
}

pub(super) fn write_usize(pointer: *mut c_void, value: usize) {
    let address = normalize_address(pointer as usize);
    unsafe { ptr::write_unaligned(address as *mut usize, value) };
}

pub(super) fn write_u32(pointer: *mut c_void, value: u32) {
    let address = normalize_address(pointer as usize);
    unsafe { ptr::write_unaligned(address as *mut u32, value) };
}

pub(super) fn normalize_address(address: usize) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        if POINTER_SIZE == 8 {
            return address & 0x00ff_ffff_ffff_ffff;
        }
    }
    address
}

pub(super) fn class_loader_key(class: *mut c_void) -> u32 {
    unsafe { ptr::read_unaligned((class as usize + (2 * 4)) as *const u32) }
}

pub(super) fn unsupported_method_query<T>(reason: impl Into<String>) -> Result<T> {
    unsupported_feature(FEATURE_METHOD_QUERY, reason)
}
