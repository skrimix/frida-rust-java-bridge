mod bypass;
mod controller;
mod dispatch;
mod guard;
mod hooks;
mod method;

use std::ffi::c_void;

pub(crate) use bypass::original_method_call_bypass;
#[cfg(test)]
pub(super) use bypass::{
    ORIGINAL_CALL_BYPASS_METHOD, ORIGINAL_CALL_BYPASS_OWNER_THREAD, ORIGINAL_CALL_BYPASS_THREAD,
};
pub(super) use controller::{
    ArtReplacementController, ArtReplacementSynchronization, GcSynchronizationEntry,
    GcSynchronizationTiming,
};
pub(super) use dispatch::ArtMethodDispatchThunk;
#[cfg(test)]
pub(super) use dispatch::replacement_frame_is_active;
pub(crate) use guard::ArtMethodReplacementGuard;
#[cfg(test)]
pub(super) use method::patch_art_method;
pub(super) use method::{
    ArtMethodClone, art_method_kind_matches, clone_replacement_art_method,
    compile_dont_bother_flag, patch_art_method_verified,
    patched_original_method_for_clone_dispatch, patched_replacement_method,
    restore_art_method_verified, snapshot_art_method, validate_replacement_function,
    validate_replacement_trampoline,
};

use super::{
    ArtVmAccess, ArtVmHandle,
    backend::ArtBackend,
    features::*,
    layout::*,
    memory::MemoryRanges,
    runtime_layout::{android_api_level, detect_runtime_layout_for_method_replacement},
    threads::SuspendedAllThreads,
};
use crate::{
    capabilities::FeatureSupport,
    env::{Env, MethodKind},
    error::{Error, Result},
    jni,
};

impl ArtBackend {
    pub(crate) fn method_replacement_support(&self, vm: &impl ArtVmAccess) -> FeatureSupport {
        match self.detect_method_replacement_prerequisites(vm) {
            Ok(_) => FeatureSupport::Supported,
            Err(Error::UnsupportedFeature { reason, .. }) => unsupported_support(reason),
            Err(error) => unsupported_support(error.to_string()),
        }
    }

    pub(crate) fn replace_method(
        &self,
        vm: ArtVmHandle,
        kind: MethodKind,
        method_id: jni::jmethodID,
        replacement: *mut c_void,
    ) -> Result<ArtMethodReplacementGuard> {
        if replacement.is_null() {
            return Err(Error::NullReturn {
                operation: "ART replacement function",
            });
        }

        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        validate_replacement_function(replacement, &memory)?;
        let api_level = android_api_level(FEATURE_METHOD_REPLACEMENT)?;
        let layout = self.detect_method_replacement_prerequisites(&vm)?;
        self.replacement_controller.ensure_hooks()?;
        self.replacement_controller
            .ensure_quick_entrypoint_hooks(&layout.trampolines)?;
        let mut guard = None;

        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |_thread| {
            let candidates = self.art_method_from_jni_id(&layout.runtime, method_id);
            let compile_dont_bother = compile_dont_bother_flag(api_level);
            let mut saw_readable_candidate = false;
            let mut saw_wrong_kind_candidate = false;
            for method in candidates {
                let Ok(original) = snapshot_art_method(method, &layout.method, &memory) else {
                    continue;
                };
                saw_readable_candidate = true;
                if !art_method_kind_matches(original, kind) {
                    saw_wrong_kind_candidate = true;
                    continue;
                }
                let clone_patched = patched_replacement_method(
                    original,
                    replacement,
                    layout.trampolines.quick_generic_jni_trampoline,
                    compile_dont_bother,
                );
                let cloned_method = clone_replacement_art_method(
                    method,
                    &layout.method,
                    original,
                    clone_patched,
                    &memory,
                )?;
                let dispatch_thunk = ArtMethodDispatchThunk::new(
                    cloned_method.as_ptr(),
                    layout.trampolines.quick_to_interpreter_bridge_trampoline,
                    layout.method.quick_code_offset,
                    layout.thread_managed_stack_offset,
                )?;
                let original_patched = patched_original_method_for_clone_dispatch(
                    original,
                    dispatch_thunk.as_ptr(),
                    compile_dont_bother,
                );
                let _suspended = self.suspend_all_threads_for_replacement(&layout.runtime)?;
                self.replacement_controller.register(
                    method,
                    cloned_method.as_ptr(),
                    dispatch_thunk.as_ptr(),
                    dispatch_thunk.len(),
                    ArtReplacementSynchronization {
                        quick_code_offset: layout.method.quick_code_offset,
                        thread_managed_stack_offset: layout.thread_managed_stack_offset,
                        nterp_entrypoint: None,
                        quick_to_interpreter_bridge: layout
                            .trampolines
                            .quick_to_interpreter_bridge_trampoline
                            as usize,
                    },
                )?;
                self.replacement_controller
                    .register_jni_id(method_id, method);
                if let Err(error) = patch_art_method_verified(
                    method,
                    &layout.method,
                    original,
                    original_patched,
                    &memory,
                ) {
                    self.replacement_controller.unregister(method);
                    return Err(error);
                }
                self.replacement_controller
                    .synchronize_replacement_methods();
                guard = Some(ArtMethodReplacementGuard {
                    backend: self.clone(),
                    vm: vm.clone(),
                    method,
                    cloned_method,
                    dispatch_thunk,
                    layout,
                    original,
                    original_patched,
                    clone_patched,
                    reverted: false,
                });
                return Ok(());
            }

            if saw_wrong_kind_candidate {
                let reason = match kind {
                    MethodKind::Static => "resolved target ArtMethod is not static",
                    MethodKind::Instance => "resolved target ArtMethod is static",
                    MethodKind::Constructor => "resolved target ArtMethod is a constructor",
                };
                return unsupported_feature(FEATURE_METHOD_REPLACEMENT, reason);
            }
            if saw_readable_candidate {
                let reason = match kind {
                    MethodKind::Static => {
                        "unable to resolve a static target ArtMethod from JNI method ID"
                    }
                    MethodKind::Instance => {
                        "unable to resolve an instance target ArtMethod from JNI method ID"
                    }
                    MethodKind::Constructor => {
                        "unable to resolve a constructor target ArtMethod from JNI method ID"
                    }
                };
                return unsupported_feature(FEATURE_METHOD_REPLACEMENT, reason);
            }
            unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to resolve target ArtMethod from JNI method ID: no readable candidates",
            )
        })?;

        guard.ok_or(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "method replacement did not produce a guard".to_owned(),
        })
    }

    pub(super) fn restore_method(
        &self,
        vm: &impl ArtVmAccess,
        method: *mut c_void,
        layout: &ArtMethodReplacementLayout,
        original: ArtMethodSnapshot,
    ) -> Result<()> {
        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |_thread| {
            let _suspended = self.suspend_all_threads_for_replacement(&layout.runtime)?;
            restore_art_method_verified(method, &layout.method, original, &memory)
        })
    }

    pub(super) fn detect_method_replacement_prerequisites(
        &self,
        vm: &impl ArtVmAccess,
    ) -> Result<ArtMethodReplacementLayout> {
        self.replacement_controller.ensure_dispatch_supported()?;
        if self.enumeration.pretty_method.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod::PrettyMethod is unavailable",
            );
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "only arm64-v8a is supported in this milestone",
            );
        }
        if self.common.suspend_all.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ThreadList::SuspendAll is unavailable for safe method patching",
            );
        }
        if self.common.resume_all.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ThreadList::ResumeAll is unavailable for safe method patching",
            );
        }
        let android_runtime = self
            .android_runtime
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "libandroid_runtime.so is unavailable".to_owned(),
            })?;

        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        let api_level = android_api_level(FEATURE_METHOD_REPLACEMENT)?;
        let (runtime_layout, trampolines) = detect_runtime_layout_for_method_replacement(
            // SAFETY: Replacement layout probing operates on the live process JavaVM.
            unsafe { vm.handle() },
            api_level,
            self.common.set_jni_id_type,
            self.class_linker_entrypoint_predicates(),
            &memory,
            FEATURE_METHOD_REPLACEMENT,
        )?;
        validate_replacement_trampoline(&trampolines, &memory)?;
        if runtime_layout.uses_indirect_jni_ids() && self.common.decode_method_id.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "JniIdManager::DecodeMethodId is unavailable for indirect JNI method IDs",
            );
        }
        let layout_method = find_method_replacement_layout_probe(&env)?;

        let mut layout = None;
        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |thread| {
            let process_method = self.art_method_from_jni_id(&runtime_layout, layout_method);
            let method_layout = detect_art_method_replacement_layout(
                &process_method,
                android_runtime,
                api_level,
                &memory,
                true,
                FEATURE_METHOD_REPLACEMENT,
            )?;
            layout = Some(ArtMethodReplacementLayout {
                api_level,
                runtime: runtime_layout,
                method: method_layout,
                trampolines,
                thread_managed_stack_offset: detect_art_thread_managed_stack_offset(
                    FEATURE_METHOD_REPLACEMENT,
                    thread,
                    // SAFETY: `env` is borrowed on the current attached thread for this probe.
                    unsafe { env.handle() }.as_ptr().cast(),
                )?,
            });
            Ok(())
        })?;

        layout.ok_or(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "method replacement prerequisites were not probed".to_owned(),
        })
    }

    fn class_linker_entrypoint_predicates(&self) -> Option<ArtClassLinkerEntrypointPredicates> {
        Some(ArtClassLinkerEntrypointPredicates {
            is_quick_resolution_stub: self.common.is_quick_resolution_stub?,
            is_quick_to_interpreter_bridge: self.common.is_quick_to_interpreter_bridge?,
            is_quick_generic_jni_stub: self.common.is_quick_generic_jni_stub?,
        })
    }

    fn suspend_all_threads_for_replacement(
        &self,
        layout: &ArtRuntimeLayout,
    ) -> Result<SuspendedAllThreads> {
        let suspend_all = self
            .common
            .suspend_all
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ThreadList::SuspendAll is unavailable for safe method patching".to_owned(),
            })?;
        let resume_all = self
            .common
            .resume_all
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ThreadList::ResumeAll is unavailable for safe method patching".to_owned(),
            })?;
        Ok(SuspendedAllThreads::new(
            suspend_all,
            resume_all,
            layout.thread_list,
        ))
    }
}

fn find_method_replacement_layout_probe(env: &Env<'_>) -> Result<jni::jmethodID> {
    let method = env
        .find_class("android/os/Process")
        .and_then(|class| env.lookup_static_method(&class, "getElapsedCpuTime", "()J"))
        .or_else(|_| {
            let system = env.find_class("java/lang/System")?;
            env.lookup_static_method(&system, "currentTimeMillis", "()J")
        })?;
    Ok(unsafe { method.raw() })
}
