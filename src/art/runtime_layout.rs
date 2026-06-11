use std::{ffi::c_void, ptr::NonNull};

use super::{features::unsupported_feature, layout::*, memory::MemoryRanges, runnable_thread};
use crate::{
    capabilities::FeatureSupport,
    error::{Error, Result},
    jni,
};

pub(super) fn detect_runtime_layout(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let api_level = android_api_level(feature)?;
    detect_runtime_layout_for_api(vm, api_level, feature)
}

pub(super) fn detect_runtime_layout_for_api(
    vm: NonNull<jni::JavaVM>,
    api_level: i32,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let runtime = art_runtime_from_vm(vm);
    let memory = MemoryRanges::current_for_feature(feature)?;
    detect_runtime_layout_from_runtime_with_memory(
        api_level,
        runtime,
        vm.as_ptr() as usize,
        &memory,
        feature,
    )
}

pub(super) fn detect_runtime_layout_for_method_replacement(
    vm: NonNull<jni::JavaVM>,
    api_level: i32,
    set_jni_id_type: Option<*const c_void>,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<(ArtRuntimeLayout, ArtClassLinkerTrampolines)> {
    let runtime = art_runtime_from_vm(vm);
    detect_runtime_layout_and_trampolines_from_runtime(
        api_level,
        runtime,
        vm.as_ptr() as usize,
        set_jni_id_type,
        predicates,
        memory,
        feature,
    )
}

#[cfg(test)]
pub(super) fn detect_runtime_layout_from_runtime(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    match scan_runtime_layout_candidates(api_level, runtime, vm_value, None, feature, |layout| {
        Ok(Some(layout))
    })? {
        RuntimeLayoutScan::Accepted(layout) => Ok(layout),
        RuntimeLayoutScan::Exhausted { .. } => {
            unsupported_feature(feature, "unable to determine ART runtime field offsets")
        }
    }
}

pub(super) fn detect_runtime_layout_from_runtime_with_memory(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    match scan_runtime_layout_candidates(
        api_level,
        runtime,
        vm_value,
        Some(memory),
        feature,
        |layout| Ok(Some(layout)),
    )? {
        RuntimeLayoutScan::Accepted(layout) => Ok(layout),
        RuntimeLayoutScan::Exhausted { .. } => {
            unsupported_feature(feature, "unable to determine ART runtime field offsets")
        }
    }
}

pub(super) fn detect_runtime_layout_and_trampolines_from_runtime(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    set_jni_id_type: Option<*const c_void>,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<(ArtRuntimeLayout, ArtClassLinkerTrampolines)> {
    let mut candidate_failure = None;
    let scan = scan_runtime_layout_candidates(
        api_level,
        runtime,
        vm_value,
        Some(memory),
        feature,
        |mut layout| {
            let jni_ids_indirection =
                detect_jni_ids_indirection(layout.runtime, set_jni_id_type, memory, feature)?;
            layout.jni_ids_indirection = jni_ids_indirection;

            match detect_class_linker_trampolines(&layout, api_level, predicates, memory) {
                Ok(trampolines) => Ok(Some((layout, trampolines))),
                Err(Error::UnsupportedFeature { reason, .. }) => {
                    candidate_failure.get_or_insert(reason);
                    Ok(None)
                }
                Err(error) => {
                    candidate_failure.get_or_insert(error.to_string());
                    Ok(None)
                }
            }
        },
    )?;

    match scan {
        RuntimeLayoutScan::Accepted(layout_and_trampolines) => Ok(layout_and_trampolines),
        RuntimeLayoutScan::Exhausted { found_vm } => {
            if let Some(reason) = candidate_failure {
                return unsupported_feature(feature, reason);
            }
            if found_vm {
                return unsupported_feature(
                    feature,
                    "unable to determine ART runtime field offsets: no non-null ClassLinker candidates",
                );
            }

            unsupported_feature(feature, "unable to determine ART runtime field offsets")
        }
    }
}

enum RuntimeLayoutScan<T> {
    Accepted(T),
    Exhausted { found_vm: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct RuntimeLayoutOffsets {
    pub(super) vm: usize,
    pub(super) heap: usize,
    pub(super) thread_list: usize,
    pub(super) intern_table: usize,
    pub(super) class_linker: usize,
    pub(super) jni_id_manager: Option<usize>,
}

impl RuntimeLayoutOffsets {
    fn from_class_linker(api_level: i32, vm: usize, class_linker: usize) -> Option<Self> {
        let intern_table = class_linker.checked_sub(POINTER_SIZE)?;
        let thread_list = intern_table.checked_sub(POINTER_SIZE)?;
        let heap = runtime_heap_offset_for_api(api_level, thread_list)?;
        let jni_id_manager = if api_level >= 30 {
            Some(vm.checked_sub(POINTER_SIZE)?)
        } else {
            None
        };

        Some(Self {
            vm,
            heap,
            thread_list,
            intern_table,
            class_linker,
            jni_id_manager,
        })
    }
}

fn scan_runtime_layout_candidates<T>(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    memory: Option<&MemoryRanges>,
    feature: &'static str,
    mut accept: impl FnMut(ArtRuntimeLayout) -> Result<Option<T>>,
) -> Result<RuntimeLayoutScan<T>> {
    if api_level < 26 {
        return unsupported_feature(
            feature,
            format!("Android API level {api_level} is below the API 26+ arm64 milestone"),
        );
    }
    if runtime.is_null() {
        return unsupported_feature(feature, "ART runtime pointer is null");
    }

    let runtime = runtime.cast::<usize>();
    let mut found_vm = false;
    for vm_offset in (384..(384 + (100 * POINTER_SIZE))).step_by(POINTER_SIZE) {
        let Some(value) = read_runtime_word(runtime, vm_offset, memory) else {
            continue;
        };
        if value != vm_value {
            continue;
        }
        found_vm = true;

        for offsets in runtime_layout_offset_candidates_for_api(api_level, vm_offset) {
            let Some(layout) = read_runtime_layout_candidate(runtime, offsets, memory) else {
                continue;
            };

            if layout.heap.is_null()
                || layout.thread_list.is_null()
                || layout.class_linker.is_null()
                || layout.intern_table.is_null()
            {
                continue;
            }
            if !runtime_layout_pointers_are_readable(&layout, memory) {
                continue;
            }

            if let Some(accepted) = accept(layout)? {
                return Ok(RuntimeLayoutScan::Accepted(accepted));
            }
        }
    }

    Ok(RuntimeLayoutScan::Exhausted { found_vm })
}

fn read_runtime_layout_candidate(
    runtime: *const usize,
    offsets: RuntimeLayoutOffsets,
    memory: Option<&MemoryRanges>,
) -> Option<ArtRuntimeLayout> {
    let heap = read_runtime_pointer(runtime, offsets.heap, memory)?;
    let thread_list = read_runtime_pointer(runtime, offsets.thread_list, memory)?;
    let intern_table = read_runtime_pointer(runtime, offsets.intern_table, memory)?;
    let class_linker = read_runtime_pointer(runtime, offsets.class_linker, memory)?;
    let jni_id_manager = match offsets.jni_id_manager {
        Some(offset) => read_runtime_pointer(runtime, offset, memory)?,
        None => std::ptr::null_mut(),
    };

    Some(ArtRuntimeLayout {
        runtime: runtime.cast_mut().cast(),
        heap,
        thread_list,
        class_linker,
        intern_table,
        jni_id_manager,
        jni_ids_indirection: None,
    })
}

fn read_runtime_word(
    runtime: *const usize,
    offset: usize,
    memory: Option<&MemoryRanges>,
) -> Option<usize> {
    let address = runtime as usize + offset;
    if memory.is_some_and(|memory| !memory.contains(address, POINTER_SIZE)) {
        return None;
    }
    Some(unsafe { runtime.byte_add(offset).read() })
}

fn read_runtime_pointer(
    runtime: *const usize,
    offset: usize,
    memory: Option<&MemoryRanges>,
) -> Option<*mut c_void> {
    read_runtime_word(runtime, offset, memory).map(|value| value as *mut c_void)
}

fn runtime_layout_pointers_are_readable(
    layout: &ArtRuntimeLayout,
    memory: Option<&MemoryRanges>,
) -> bool {
    let Some(memory) = memory else {
        return true;
    };

    [
        layout.heap,
        layout.thread_list,
        layout.class_linker,
        layout.intern_table,
    ]
    .into_iter()
    .all(|pointer| memory.contains(pointer as usize, POINTER_SIZE))
        && (layout.jni_id_manager.is_null()
            || memory.contains(layout.jni_id_manager as usize, POINTER_SIZE))
}

pub(super) fn runtime_layout_offset_candidates_for_api(
    api_level: i32,
    vm_offset: usize,
) -> Vec<RuntimeLayoutOffsets> {
    class_linker_offsets_for_api(api_level, vm_offset)
        .into_iter()
        .filter_map(|class_linker| {
            RuntimeLayoutOffsets::from_class_linker(api_level, vm_offset, class_linker)
        })
        .collect()
}

fn class_linker_offsets_for_api(api_level: i32, vm_offset: usize) -> Vec<usize> {
    if api_level >= 33 {
        vec![vm_offset - (4 * POINTER_SIZE)]
    } else if api_level >= 30 {
        vec![
            vm_offset - (3 * POINTER_SIZE),
            vm_offset - (4 * POINTER_SIZE),
        ]
    } else if api_level >= 29 {
        vec![vm_offset - (2 * POINTER_SIZE)]
    } else if api_level >= 27 {
        vec![vm_offset - STD_STRING_SIZE - (3 * POINTER_SIZE)]
    } else {
        vec![vm_offset - STD_STRING_SIZE - (2 * POINTER_SIZE)]
    }
}

fn runtime_heap_offset_for_api(api_level: i32, thread_list_offset: usize) -> Option<usize> {
    if api_level >= 34 {
        thread_list_offset.checked_sub(9 * POINTER_SIZE)
    } else if api_level >= 24 {
        thread_list_offset.checked_sub(8 * POINTER_SIZE)
    } else if api_level >= 23 {
        thread_list_offset.checked_sub(7 * POINTER_SIZE)
    } else {
        thread_list_offset.checked_sub(4 * POINTER_SIZE)
    }
}

pub(super) fn detect_jni_ids_indirection(
    runtime: *mut c_void,
    set_jni_id_type: Option<*const c_void>,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<Option<i32>> {
    let Some(set_jni_id_type) = set_jni_id_type else {
        return Ok(None);
    };
    let Some(offset) = detect_jni_ids_indirection_offset(feature, set_jni_id_type)? else {
        return Ok(None);
    };
    Ok(read_u32((runtime as usize + offset) as *const c_void, memory).map(|value| value as i32))
}

#[cfg(target_arch = "aarch64")]
pub(super) fn detect_jni_ids_indirection_offset(
    feature: &'static str,
    set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    runnable_thread::detect_jni_ids_indirection_offset(feature, set_jni_id_type)
}

#[cfg(not(target_arch = "aarch64"))]
pub(super) fn detect_jni_ids_indirection_offset(
    _feature: &'static str,
    _set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    Ok(None)
}

pub(super) fn android_api_level(feature: &'static str) -> Result<i32> {
    crate::android::android_api_level_for_feature(feature)
}

pub(super) fn runtime_layout_support(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> FeatureSupport {
    match detect_runtime_layout(vm, feature) {
        Ok(_) => FeatureSupport::Supported,
        Err(Error::UnsupportedFeature { reason, .. }) => FeatureSupport::Unsupported { reason },
        Err(error) => FeatureSupport::Unsupported {
            reason: error.to_string(),
        },
    }
}

pub(super) fn ensure_feature_supported(
    feature: &'static str,
    support: FeatureSupport,
) -> Result<()> {
    match support {
        FeatureSupport::Supported => Ok(()),
        FeatureSupport::Unsupported { reason } => unsupported_feature(feature, reason),
    }
}

pub(super) fn art_runtime_from_vm(vm: NonNull<jni::JavaVM>) -> *mut c_void {
    unsafe { vm.as_ptr().cast::<*mut c_void>().add(1).read() }
}
