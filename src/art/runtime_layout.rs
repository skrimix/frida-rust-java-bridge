use std::{ffi::c_void, ptr::NonNull};

use super::{layout::*, memory::MemoryRanges, runnable_thread};
use crate::{
    error::{Error, Result},
    jni,
    runtime::FeatureSupport,
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
    detect_runtime_layout_from_runtime(api_level, runtime, vm.as_ptr() as usize, feature)
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

pub(super) fn detect_runtime_layout_from_runtime(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    match scan_runtime_layout_candidates(api_level, runtime, vm_value, feature, |layout| {
        Ok(Some(layout))
    })? {
        RuntimeLayoutScan::Accepted(layout) => Ok(layout),
        RuntimeLayoutScan::Exhausted { .. } => {
            unsupported_feature(feature, "unable to determine ART Runtime field offsets")
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
    let scan =
        scan_runtime_layout_candidates(api_level, runtime, vm_value, feature, |mut layout| {
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
        })?;

    match scan {
        RuntimeLayoutScan::Accepted(layout_and_trampolines) => Ok(layout_and_trampolines),
        RuntimeLayoutScan::Exhausted { found_vm } => {
            if let Some(reason) = candidate_failure {
                return unsupported_feature(feature, reason);
            }
            if found_vm {
                return unsupported_feature(
                    feature,
                    "unable to determine ART Runtime field offsets: no non-null ClassLinker candidates",
                );
            }

            unsupported_feature(feature, "unable to determine ART Runtime field offsets")
        }
    }
}

enum RuntimeLayoutScan<T> {
    Accepted(T),
    Exhausted { found_vm: bool },
}

fn scan_runtime_layout_candidates<T>(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
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
        return unsupported_feature(feature, "ART Runtime pointer is null");
    }

    let runtime = runtime.cast::<usize>();
    let mut found_vm = false;
    for offset in (384..(384 + (100 * POINTER_SIZE))).step_by(POINTER_SIZE) {
        let value = unsafe { runtime.byte_add(offset).read() };
        if value != vm_value {
            continue;
        }
        found_vm = true;

        for class_linker_offset in class_linker_offsets_for_api(api_level, offset) {
            if class_linker_offset < (2 * POINTER_SIZE) {
                continue;
            }

            let intern_table_offset = class_linker_offset - POINTER_SIZE;
            let thread_list_offset = intern_table_offset - POINTER_SIZE;
            let heap_offset = heap_offset_for_api(api_level, thread_list_offset);
            if heap_offset >= thread_list_offset {
                continue;
            }
            let heap = unsafe { runtime.byte_add(heap_offset).read() as *mut c_void };
            let thread_list = unsafe { runtime.byte_add(thread_list_offset).read() as *mut c_void };
            let class_linker =
                unsafe { runtime.byte_add(class_linker_offset).read() as *mut c_void };
            let intern_table =
                unsafe { runtime.byte_add(intern_table_offset).read() as *mut c_void };
            let jni_id_manager = if api_level >= 30 {
                unsafe { runtime.byte_add(offset - POINTER_SIZE).read() as *mut c_void }
            } else {
                std::ptr::null_mut()
            };

            if heap.is_null()
                || thread_list.is_null()
                || class_linker.is_null()
                || intern_table.is_null()
            {
                continue;
            }

            let layout = ArtRuntimeLayout {
                runtime: runtime.cast(),
                heap,
                thread_list,
                class_linker,
                intern_table,
                jni_id_manager,
                jni_ids_indirection: None,
            };

            if let Some(accepted) = accept(layout)? {
                return Ok(RuntimeLayoutScan::Accepted(accepted));
            }
        }
    }

    Ok(RuntimeLayoutScan::Exhausted { found_vm })
}

pub(super) fn class_linker_offsets_for_api(api_level: i32, vm_offset: usize) -> Vec<usize> {
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

pub(super) fn heap_offset_for_api(api_level: i32, thread_list_offset: usize) -> usize {
    if api_level >= 34 {
        thread_list_offset.saturating_sub(9 * POINTER_SIZE)
    } else if api_level >= 24 {
        thread_list_offset.saturating_sub(8 * POINTER_SIZE)
    } else if api_level >= 23 {
        thread_list_offset.saturating_sub(7 * POINTER_SIZE)
    } else {
        thread_list_offset.saturating_sub(4 * POINTER_SIZE)
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

pub(super) fn unsupported_support(reason: impl Into<String>) -> FeatureSupport {
    FeatureSupport::Unsupported {
        reason: reason.into(),
    }
}

pub(super) fn unsupported_feature<T>(
    feature: &'static str,
    reason: impl Into<String>,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}

pub(super) fn art_runtime_from_vm(vm: NonNull<jni::JavaVM>) -> *mut c_void {
    unsafe { vm.as_ptr().cast::<*mut c_void>().add(1).read() }
}
