use std::{
    ffi::{c_char, c_void},
    sync::{Arc, OnceLock},
};

use frida_gum::Module;

use super::{
    enumeration::*,
    layout::{ArtModuleRange, ArtRuntimeLayout},
    replacement::ArtReplacementController,
    resolution::*,
    runnable_thread,
    strings::ArtStdString,
    symbols::*,
};
use crate::{error::Result, jni};

pub(super) type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
pub(super) type GetClassDescriptor =
    unsafe extern "C" fn(*mut c_void, *mut ArtStdString) -> *const c_char;
pub(super) type PrettyMethod = unsafe extern "C" fn(*mut ArtStdString, *mut c_void, bool);
pub(super) type DecodeMethodId = unsafe extern "C" fn(*mut c_void, jni::jmethodID) -> *mut c_void;
pub(super) type IsQuickEntrypoint = unsafe extern "C" fn(*mut c_void, *const c_void) -> bool;
pub(super) type SuspendAllWithCause = unsafe extern "C" fn(*mut c_void, *const c_char, bool);
pub(super) type SuspendAllLegacy = unsafe extern "C" fn(*mut c_void);
pub(super) type ResumeAll = unsafe extern "C" fn(*mut c_void);
pub(super) type VisitClassLoaders = unsafe extern "C" fn(*mut c_void, *mut ArtClassLoaderVisitor);
pub(super) type VisitClasses = unsafe extern "C" fn(*mut c_void, *mut ArtClassVisitor);
pub(super) type VisitClassesCallback =
    unsafe extern "C" fn(*mut c_void, ArtClassCallback, *mut c_void);
pub(super) type ArtClassCallback = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
pub(super) type VisitObjects = unsafe extern "C" fn(*mut c_void, HeapObjectCallback, *mut c_void);
pub(super) type HeapObjectCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub(super) type GetInstances =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i32, *mut c_void);
pub(super) type GetInstancesAssignable =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, bool, i32, *mut c_void);
pub(super) type DecodeGlobalNoThread =
    unsafe extern "C" fn(*mut jni::JavaVM, jni::jobject) -> usize;
pub(super) type DecodeGlobalWithThread =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, jni::jobject) -> usize;
pub(super) type ThreadDecodeGlobalJObject =
    unsafe extern "C" fn(*mut c_void, jni::jobject) -> usize;
pub(super) type GetOatQuickMethodHeader = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
pub(super) type DbgSetJdwpAllowed = unsafe extern "C" fn(bool);
pub(super) type DbgConfigureJdwp = unsafe extern "C" fn(*const c_void);
pub(super) type StartDebugger = unsafe extern "C" fn(*mut c_void);
pub(super) type DbgStartJdwp = unsafe extern "C" fn();
pub(super) type DbgGoActive = unsafe extern "C" fn();
pub(super) type DbgRequestDeoptimization = unsafe extern "C" fn(*const c_void);
pub(super) type DbgManageDeoptimization = unsafe extern "C" fn();
pub(super) type InstrumentationEnableDeoptimization = unsafe extern "C" fn(*mut c_void);
pub(super) type InstrumentationDeoptimizeEverything =
    unsafe extern "C" fn(*mut c_void, *const c_char);
pub(super) type InstrumentationDeoptimize = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub(super) type RuntimeDeoptimizeBootImage = unsafe extern "C" fn(*mut c_void);

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

#[derive(Clone)]
pub(super) struct EnumerationArtSymbols {
    pub(super) visit_class_loaders: Option<VisitClassLoaders>,
    pub(super) visit_classes: Option<VisitClassesKind>,
    pub(super) get_class_descriptor: Option<GetClassDescriptor>,
    pub(super) pretty_method: Option<PrettyMethodFunction>,
}

#[derive(Clone)]
pub(super) struct HeapArtSymbols {
    pub(super) visit_objects: Option<VisitObjects>,
    pub(super) get_instances: Option<GetInstancesKind>,
    pub(super) decode_global: Option<DecodeGlobalKind>,
}

#[derive(Clone)]
pub(super) struct DeoptimizationArtSymbols {
    pub(super) dbg_set_jdwp_allowed: Option<DbgSetJdwpAllowed>,
    pub(super) dbg_configure_jdwp: Option<DbgConfigureJdwp>,
    pub(super) internal_start_debugger: Option<StartDebugger>,
    pub(super) dbg_start_jdwp: Option<DbgStartJdwp>,
    pub(super) dbg_go_active: Option<DbgGoActive>,
    pub(super) dbg_request_deoptimization: Option<DbgRequestDeoptimization>,
    pub(super) dbg_manage_deoptimization: Option<DbgManageDeoptimization>,
    pub(super) dbg_registry: Option<*const c_void>,
    pub(super) dbg_debugger_active: Option<*const c_void>,
    pub(super) instrumentation_enable_deoptimization: Option<InstrumentationEnableDeoptimization>,
    pub(super) instrumentation_deoptimize_everything: Option<InstrumentationDeoptimizeEverything>,
    pub(super) instrumentation_deoptimize: Option<InstrumentationDeoptimize>,
    pub(super) runtime_deoptimize_boot_image: Option<RuntimeDeoptimizeBootImage>,
    pub(super) jdwp_adb_state_accept: Option<*const c_void>,
    pub(super) jdwp_adb_state_receive_client_fd: Option<*const c_void>,
}

#[derive(Clone, Copy)]
pub(super) enum SuspendAll {
    WithCause(SuspendAllWithCause),
    Legacy(SuspendAllLegacy),
}

#[derive(Clone, Copy)]
pub(super) enum VisitClassesKind {
    Visitor(VisitClasses),
    Callback(VisitClassesCallback),
}

#[derive(Clone, Copy)]
pub(super) enum GetInstancesKind {
    Exact(GetInstances),
    WithAssignable(GetInstancesAssignable),
}

#[derive(Clone, Copy)]
pub(super) enum DecodeGlobalKind {
    NoThread(DecodeGlobalNoThread),
    WithThread(DecodeGlobalWithThread),
    Thread(ThreadDecodeGlobalJObject),
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
            enumeration: EnumerationArtSymbols {
                visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
                visit_classes: resolve_visit_classes(module),
                get_class_descriptor: resolve(module, GET_CLASS_DESCRIPTOR),
                pretty_method: resolve_pretty_method(module),
            },
            heap: HeapArtSymbols {
                visit_objects: resolve(module, VISIT_OBJECTS),
                get_instances: resolve_get_instances(module),
                decode_global: resolve_decode_global(module),
            },
            deoptimization: DeoptimizationArtSymbols {
                dbg_set_jdwp_allowed: resolve(module, DBG_SET_JDWP_ALLOWED),
                dbg_configure_jdwp: resolve(module, DBG_CONFIGURE_JDWP),
                internal_start_debugger: resolve(module, INTERNAL_DEBUGGER_CONTROL_START_DEBUGGER),
                dbg_start_jdwp: resolve(module, DBG_START_JDWP),
                dbg_go_active: resolve(module, DBG_GO_ACTIVE),
                dbg_request_deoptimization: resolve(module, DBG_REQUEST_DEOPTIMIZATION),
                dbg_manage_deoptimization: resolve(module, DBG_MANAGE_DEOPTIMIZATION),
                dbg_registry: resolve_pointer(module, DBG_REGISTRY),
                dbg_debugger_active: resolve_pointer(module, DBG_DEBUGGER_ACTIVE),
                instrumentation_enable_deoptimization: resolve(
                    module,
                    INSTRUMENTATION_ENABLE_DEOPTIMIZATION,
                ),
                instrumentation_deoptimize_everything: resolve(
                    module,
                    INSTRUMENTATION_DEOPTIMIZE_EVERYTHING,
                ),
                instrumentation_deoptimize: resolve(module, INSTRUMENTATION_DEOPTIMIZE),
                runtime_deoptimize_boot_image: resolve(module, RUNTIME_DEOPTIMIZE_BOOT_IMAGE),
                jdwp_adb_state_accept: resolve_pointer(module, JDWP_ADB_STATE_ACCEPT),
                jdwp_adb_state_receive_client_fd: resolve_pointer(
                    module,
                    JDWP_ADB_STATE_RECEIVE_CLIENT_FD,
                ),
            },
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
