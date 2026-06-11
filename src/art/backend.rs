use std::{
    ffi::{c_char, c_void},
    sync::{Arc, OnceLock},
};

use frida_gum::Module;

use super::{
    deoptimization::{DeoptimizationArtSymbols, resolve_deoptimization_symbols},
    enumeration::{
        EnumerationArtSymbols, HeapArtSymbols, resolve_enumeration_symbols, resolve_heap_symbols,
    },
    layout::{ArtModuleRange, ArtRuntimeLayout},
    replacement::ArtReplacementController,
    resolution::{resolve, resolve_any, resolve_pointer},
    runnable_thread,
    symbols::*,
};
use crate::{error::Result, jni};

pub(super) type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
pub(super) type DecodeMethodId = unsafe extern "C" fn(*mut c_void, jni::jmethodID) -> *mut c_void;
pub(super) type IsQuickEntrypoint = unsafe extern "C" fn(*mut c_void, *const c_void) -> bool;
pub(super) type SuspendAllWithCause = unsafe extern "C" fn(*mut c_void, *const c_char, bool);
pub(super) type SuspendAllLegacy = unsafe extern "C" fn(*mut c_void);
pub(super) type ResumeAll = unsafe extern "C" fn(*mut c_void);

/// Resolved ART interface for the current process.
///
/// `ArtBackend` records the ART symbols, module ranges, inferred layout support, and guarded
/// operations that were proven for this process. It is not the ART runtime object itself. Runtime
/// pointers stay inside layout structs returned by probing, and missing or mismatched pieces should
/// become `UnsupportedFeature` reasons at the public boundary.
#[derive(Clone)]
pub(crate) struct ArtBackend {
    pub(super) android_runtime: Option<ArtModuleRange>,
    pub(super) common: CommonArtSymbols,
    pub(super) enumeration: EnumerationArtSymbols,
    pub(super) heap: HeapArtSymbols,
    pub(super) deoptimization: DeoptimizationArtSymbols,
    pub(super) runnable_thread: Arc<OnceLock<runnable_thread::RunnableThreadTransition>>,
    pub(super) replacement_controller: Arc<ArtReplacementController>,
}

#[derive(Clone)]
pub(super) struct CommonArtSymbols {
    pub(super) add_global_ref: Option<AddGlobalRef>,
    pub(super) suspend_all: Option<SuspendAll>,
    pub(super) resume_all: Option<ResumeAll>,
    pub(super) decode_method_id: Option<DecodeMethodId>,
    pub(super) set_jni_id_type: Option<*const c_void>,
    pub(super) is_quick_resolution_stub: Option<IsQuickEntrypoint>,
    pub(super) is_quick_to_interpreter_bridge: Option<IsQuickEntrypoint>,
    pub(super) is_quick_generic_jni_stub: Option<IsQuickEntrypoint>,
    pub(super) exception_clear: Option<*const c_void>,
    pub(super) fatal_error: Option<*const c_void>,
}

#[derive(Clone, Copy)]
pub(super) enum SuspendAll {
    WithCause(SuspendAllWithCause),
    Legacy(SuspendAllLegacy),
}

impl ArtBackend {
    pub(crate) fn from_module(module: &Module, android_runtime: Option<ArtModuleRange>) -> Self {
        Self {
            android_runtime,
            common: CommonArtSymbols {
                add_global_ref: resolve_any(
                    module,
                    &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER],
                ),
                suspend_all: resolve_suspend_all(module),
                resume_all: resolve(module, RESUME_ALL),
                decode_method_id: resolve(module, DECODE_METHOD_ID),
                set_jni_id_type: resolve_pointer(module, SET_JNI_ID_TYPE),
                is_quick_resolution_stub: resolve(module, IS_QUICK_RESOLUTION_STUB),
                is_quick_to_interpreter_bridge: resolve(module, IS_QUICK_TO_INTERPRETER_BRIDGE),
                is_quick_generic_jni_stub: resolve(module, IS_QUICK_GENERIC_JNI_STUB),
                exception_clear: resolve_pointer(module, JNI_EXCEPTION_CLEAR),
                fatal_error: resolve_pointer(module, JNI_FATAL_ERROR),
            },
            enumeration: resolve_enumeration_symbols(module),
            heap: resolve_heap_symbols(module),
            deoptimization: resolve_deoptimization_symbols(module),
            runnable_thread: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::new(module)),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            android_runtime: None,
            common: CommonArtSymbols {
                add_global_ref: None,
                suspend_all: None,
                resume_all: None,
                decode_method_id: None,
                set_jni_id_type: None,
                is_quick_resolution_stub: None,
                is_quick_to_interpreter_bridge: None,
                is_quick_generic_jni_stub: None,
                exception_clear: None,
                fatal_error: None,
            },
            enumeration: EnumerationArtSymbols {
                visit_class_loaders: None,
                visit_classes: None,
                get_class_descriptor: None,
                pretty_method: None,
            },
            heap: HeapArtSymbols {
                visit_objects: None,
                get_instances: None,
                decode_global: None,
            },
            deoptimization: DeoptimizationArtSymbols {
                dbg_set_jdwp_allowed: None,
                dbg_configure_jdwp: None,
                internal_start_debugger: None,
                dbg_start_jdwp: None,
                dbg_go_active: None,
                dbg_request_deoptimization: None,
                dbg_manage_deoptimization: None,
                dbg_registry: None,
                dbg_debugger_active: None,
                instrumentation_enable_deoptimization: None,
                instrumentation_deoptimize_everything: None,
                instrumentation_deoptimize: None,
                runtime_deoptimize_boot_image: None,
                jdwp_adb_state_accept: None,
                jdwp_adb_state_receive_client_fd: None,
            },
            runnable_thread: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::empty_for_tests()),
        }
    }

    pub(super) fn art_method_from_jni_id(
        &self,
        layout: &ArtRuntimeLayout,
        method_id: jni::jmethodID,
    ) -> Vec<*mut c_void> {
        if layout.uses_indirect_jni_ids() {
            return self
                .common
                .decode_method_id
                .and_then(|decode_method_id| {
                    let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
                    (!decoded.is_null()).then_some(decoded)
                })
                .into_iter()
                .collect();
        }

        let mut candidates = vec![method_id.cast::<c_void>()];
        if layout.jni_ids_indirection.is_none()
            && let Some(decode_method_id) = self.common.decode_method_id
            && !layout.jni_id_manager.is_null()
        {
            let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
            if !decoded.is_null() && !candidates.contains(&decoded) {
                candidates.push(decoded);
            }
        }

        candidates
    }

    pub(super) fn with_runnable_art_thread(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let transition = self.runnable_thread(env, feature)?;
        transition.run(feature, env, f)
    }

    fn runnable_thread(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
    ) -> Result<&runnable_thread::RunnableThreadTransition> {
        if let Some(transition) = self.runnable_thread.get() {
            return Ok(transition);
        }

        let transition = runnable_thread::build(
            feature,
            env,
            self.common.exception_clear,
            self.common.fatal_error,
        )?;
        let _ = self.runnable_thread.set(transition);
        Ok(self
            .runnable_thread
            .get()
            .expect("runnable thread transition was just initialized"))
    }
}

fn resolve_suspend_all(module: &Module) -> Option<SuspendAll> {
    resolve(module, SUSPEND_ALL_WITH_CAUSE)
        .map(SuspendAll::WithCause)
        .or_else(|| resolve(module, SUSPEND_ALL_LEGACY).map(SuspendAll::Legacy))
}
