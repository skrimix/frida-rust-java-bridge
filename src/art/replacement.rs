use std::{
    collections::{HashMap, HashSet},
    ffi::{c_int, c_void},
    mem::ManuallyDrop,
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use frida_gum::{
    Module, NativePointer,
    instruction_writer::{
        Aarch64BranchCondition, Aarch64InstructionWriter, Aarch64Register, InstructionWriter,
    },
    interceptor::{Interceptor, InvocationContext, InvocationListener, Listener},
};

use super::{
    ArtVmAccess, ArtVmHandle,
    backend::{ArtBackend, GetOatQuickMethodHeader},
    features::*,
    layout::*,
    memory::{MemoryRange, MemoryRanges, mmap, mprotect, munmap},
    resolution::{
        find_gc_synchronization_entries, find_interpreter_do_call_entries, resolve_pointer_any,
    },
    runtime_layout::{
        android_api_level, detect_runtime_layout_for_method_replacement, unsupported_feature,
        unsupported_support,
    },
    symbols::*,
    threads::SuspendedAllThreads,
};
use crate::{
    capabilities::FeatureSupport,
    env::{Env, MethodKind},
    error::{Error, Result},
    jni,
};

static ART_REPLACEMENT_CONTROLLER: OnceLock<Arc<ArtReplacementController>> = OnceLock::new();
static ORIGINAL_GET_OAT_QUICK_METHOD_HEADER: AtomicUsize = AtomicUsize::new(0);
pub(super) static ORIGINAL_CALL_BYPASS_METHOD: AtomicUsize = AtomicUsize::new(0);
pub(super) static ORIGINAL_CALL_BYPASS_THREAD: AtomicUsize = AtomicUsize::new(0);
pub(super) static ORIGINAL_CALL_BYPASS_OWNER_THREAD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CALL_BYPASS_LOCK: Mutex<()> = Mutex::new(());

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

pub(super) struct ArtReplacementController {
    pub(super) do_call_entries: Vec<usize>,
    get_oat_quick_method_header: Option<*const c_void>,
    gc_synchronization_entries: Vec<GcSynchronizationEntry>,
    mappings: Mutex<ArtReplacementMappings>,
    quick_entrypoint_hooks: Mutex<ArtQuickEntrypointHooks>,
    hook_install: Mutex<()>,
    hooks: OnceLock<ArtReplacementHooks>,
}

#[derive(Debug, Default)]
struct ArtReplacementMappings {
    methods: HashMap<usize, ArtReplacementRecord>,
    jni_ids: HashMap<usize, usize>,
    replacements: HashMap<usize, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtReplacementRecord {
    replacement: usize,
    dispatch_thunk_start: usize,
    dispatch_thunk_end: usize,
    synchronization: ArtReplacementSynchronization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ArtReplacementSynchronization {
    pub(super) quick_code_offset: usize,
    pub(super) thread_managed_stack_offset: usize,
    pub(super) nterp_entrypoint: Option<usize>,
    pub(super) quick_to_interpreter_bridge: usize,
}

#[derive(Default)]
struct ArtQuickEntrypointHooks {
    addresses: HashSet<usize>,
    hooks: Vec<HookedQuickEntrypoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GcSynchronizationEntry {
    pub(super) address: usize,
    pub(super) timing: GcSynchronizationTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GcSynchronizationTiming {
    OnEnter,
    OnLeave,
}

struct ArtReplacementHooks {
    _interceptor: Interceptor,
    _listeners: Vec<HookedInterpreterDoCall>,
    _gc_listeners: Vec<HookedGcSynchronization>,
    _get_oat_quick_method_header: Option<ReplacedGetOatQuickMethodHeader>,
}

struct HookedInterpreterDoCall {
    _listener: Box<ArtMethodTranslationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct HookedGcSynchronization {
    _listener: Box<ArtReplacementSynchronizationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct HookedQuickEntrypoint {
    _interceptor: Interceptor,
    _listener: Box<ArtMethodTranslationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct ReplacedGetOatQuickMethodHeader {
    _function: NativePointer,
    _original: NativePointer,
}

struct ArtMethodTranslationListener {
    controller: Arc<ArtReplacementController>,
    source: ArtMethodTranslationSource,
}

struct ArtReplacementSynchronizationListener {
    controller: Arc<ArtReplacementController>,
    timing: GcSynchronizationTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtMethodTranslationSource {
    InterpreterDoCall,
    QuickEntrypoint,
}

pub(super) struct ArtMethodClone {
    method: NonNull<c_void>,
    length: usize,
}

pub(super) struct ArtMethodDispatchThunk {
    pointer: NonNull<c_void>,
    length: usize,
}

pub(crate) struct OriginalMethodCallBypass {
    _lock: Option<MutexGuard<'static, ()>>,
    previous: usize,
    previous_thread: usize,
}

pub(crate) struct ArtMethodReplacementGuard {
    pub(super) backend: ArtBackend,
    pub(super) vm: ArtVmHandle,
    pub(super) method: *mut c_void,
    pub(super) cloned_method: ArtMethodClone,
    pub(super) dispatch_thunk: ArtMethodDispatchThunk,
    pub(super) layout: ArtMethodReplacementLayout,
    pub(super) original: ArtMethodSnapshot,
    // Retained for backend diagnostic summaries covered by host ART tests.
    #[allow(dead_code)]
    pub(super) original_patched: ArtMethodSnapshot,
    // Retained for backend diagnostic summaries covered by host ART tests.
    #[allow(dead_code)]
    pub(super) clone_patched: ArtMethodSnapshot,
    pub(super) reverted: bool,
}

impl ArtReplacementController {
    pub(super) fn new(module: &Module) -> Self {
        Self {
            do_call_entries: find_interpreter_do_call_entries(module),
            get_oat_quick_method_header: resolve_pointer_any(
                module,
                &[
                    GET_OAT_QUICK_METHOD_HEADER_USIZE,
                    GET_OAT_QUICK_METHOD_HEADER_U32,
                ],
            ),
            gc_synchronization_entries: find_gc_synchronization_entries(module),
            mappings: Mutex::new(ArtReplacementMappings::default()),
            quick_entrypoint_hooks: Mutex::new(ArtQuickEntrypointHooks::default()),
            hook_install: Mutex::new(()),
            hooks: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn empty_for_tests() -> Self {
        Self {
            do_call_entries: Vec::new(),
            get_oat_quick_method_header: None,
            gc_synchronization_entries: Vec::new(),
            mappings: Mutex::new(ArtReplacementMappings::default()),
            quick_entrypoint_hooks: Mutex::new(ArtQuickEntrypointHooks::default()),
            hook_install: Mutex::new(()),
            hooks: OnceLock::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn with_dispatch_for_tests() -> Self {
        let mut controller = Self::empty_for_tests();
        controller.do_call_entries.push(0x1000);
        controller
    }

    pub(super) fn ensure_dispatch_supported(&self) -> Result<()> {
        if self.do_call_entries.is_empty() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART interpreter DoCall entrypoint is unavailable for cloned replacement dispatch",
            );
        }
        Ok(())
    }

    pub(super) fn ensure_hooks(self: &Arc<Self>) -> Result<()> {
        self.ensure_dispatch_supported()?;
        if self.hooks.get().is_some() {
            return Ok(());
        }

        let _install = self
            .hook_install
            .lock()
            .expect("ART replacement hook install mutex poisoned");
        if self.hooks.get().is_some() {
            return Ok(());
        }

        let _ = ART_REPLACEMENT_CONTROLLER.set(self.clone());
        let hooks = ArtReplacementHooks::install(self.clone())?;
        let _ = self.hooks.set(hooks);
        Ok(())
    }

    pub(super) fn ensure_quick_entrypoint_hooks(
        self: &Arc<Self>,
        trampolines: &ArtClassLinkerTrampolines,
    ) -> Result<()> {
        let mut quick_hooks = self
            .quick_entrypoint_hooks
            .lock()
            .expect("ART replacement quick hooks mutex poisoned");
        for entrypoint in [
            trampolines.quick_generic_jni_trampoline,
            trampolines.quick_resolution_trampoline,
            trampolines.quick_to_interpreter_bridge_trampoline,
        ] {
            let address = entrypoint as usize;
            if address == 0 || !quick_hooks.addresses.insert(address) {
                continue;
            }

            let mut interceptor = Interceptor::obtain(crate::native::process_gum());
            let mut listener = Box::new(ArtMethodTranslationListener {
                controller: self.clone(),
                source: ArtMethodTranslationSource::QuickEntrypoint,
            });
            let handle = interceptor
                .attach(NativePointer(entrypoint), listener.as_mut())
                .map_err(|error| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: format!("unable to hook ART quick entrypoint: {error:?}"),
                })?;
            quick_hooks.hooks.push(HookedQuickEntrypoint {
                _interceptor: interceptor,
                _listener: listener,
                _handle: ManuallyDrop::new(handle),
            });
        }
        Ok(())
    }

    pub(super) fn register(
        &self,
        original: *mut c_void,
        replacement: *mut c_void,
        dispatch_thunk: *mut c_void,
        dispatch_thunk_len: usize,
        synchronization: ArtReplacementSynchronization,
    ) -> Result<()> {
        if original.is_null() || replacement.is_null() || dispatch_thunk.is_null() {
            return Err(Error::NullReturn {
                operation: "ART replacement mapping",
            });
        }
        let Some(dispatch_thunk_end) = (dispatch_thunk as usize).checked_add(dispatch_thunk_len)
        else {
            return Err(Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "dispatch thunk address range overflowed".to_owned(),
            });
        };
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        if mappings.methods.contains_key(&(original as usize)) {
            return Err(Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "target ArtMethod already has an active replacement".to_owned(),
            });
        }
        if mappings.replacements.contains_key(&(replacement as usize)) {
            return Err(Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "replacement ArtMethod is already registered".to_owned(),
            });
        }
        mappings.methods.insert(
            original as usize,
            ArtReplacementRecord {
                replacement: replacement as usize,
                dispatch_thunk_start: dispatch_thunk as usize,
                dispatch_thunk_end,
                synchronization,
            },
        );
        mappings
            .replacements
            .insert(replacement as usize, original as usize);
        Ok(())
    }

    pub(super) fn register_jni_id(&self, jni_id: jni::jmethodID, original: *mut c_void) {
        if jni_id.is_null() || original.is_null() {
            return;
        }
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.jni_ids.insert(jni_id as usize, original as usize);
    }

    pub(super) fn unregister(&self, original: *mut c_void) {
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        if let Some(record) = mappings.methods.remove(&(original as usize)) {
            mappings.replacements.remove(&record.replacement);
            mappings
                .jni_ids
                .retain(|_, registered_original| *registered_original != original as usize);
        }
    }

    pub(super) fn replacement_for(&self, original: *mut c_void) -> Option<*mut c_void> {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings
            .methods
            .get(&(original as usize))
            .map(|record| record.replacement as *mut c_void)
    }

    pub(super) fn is_replacement_method(&self, method: *mut c_void) -> bool {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.replacements.contains_key(&(method as usize))
    }

    pub(super) fn has_dispatch_thunk_pc(&self, method: *mut c_void, pc: usize) -> bool {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings
            .methods
            .get(&(method as usize))
            .is_some_and(|record| {
                pc >= record.dispatch_thunk_start && pc < record.dispatch_thunk_end
            })
    }

    pub(super) fn art_method_for_jni_id(&self, method: usize) -> usize {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.jni_ids.get(&method).copied().unwrap_or(method)
    }

    #[cfg(test)]
    pub(super) fn translate_method_argument(&self, method: usize) -> usize {
        self.translate_method_argument_for_thread(method, 0)
    }

    pub(super) fn translate_method_argument_for_thread(
        &self,
        method: usize,
        thread: usize,
    ) -> usize {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        let Some(record) = mappings.methods.get(&method) else {
            return method;
        };
        if replacement_frame_is_active(
            record.replacement,
            thread,
            record.synchronization.thread_managed_stack_offset,
        ) {
            method
        } else {
            record.replacement
        }
    }

    pub(super) fn synchronize_replacement_methods(&self) {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        for (original, record) in &mappings.methods {
            unsafe {
                let original_declaring_class = *original as *const u32;
                let replacement_declaring_class = record.replacement as *mut u32;
                let declaring_class = ptr::read_unaligned(original_declaring_class);
                ptr::write_unaligned(replacement_declaring_class, declaring_class);

                if let Some(nterp_entrypoint) = record.synchronization.nterp_entrypoint {
                    let original_quick_code =
                        (*original + record.synchronization.quick_code_offset) as *mut usize;
                    if ptr::read_unaligned(original_quick_code) == nterp_entrypoint {
                        ptr::write_unaligned(
                            original_quick_code,
                            record.synchronization.quick_to_interpreter_bridge,
                        );
                    }
                }
            }
        }
    }
}

impl ArtReplacementHooks {
    pub(super) fn install(controller: Arc<ArtReplacementController>) -> Result<Self> {
        let mut interceptor = Interceptor::obtain(crate::native::process_gum());
        let mut listeners = Vec::new();
        let mut gc_listeners = Vec::new();

        for address in &controller.do_call_entries {
            let mut listener = Box::new(ArtMethodTranslationListener {
                controller: controller.clone(),
                source: ArtMethodTranslationSource::InterpreterDoCall,
            });
            let handle = interceptor
                .attach(NativePointer(*address as *mut c_void), listener.as_mut())
                .map_err(|error| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: format!("unable to hook ART interpreter DoCall: {error:?}"),
                })?;
            listeners.push(HookedInterpreterDoCall {
                _listener: listener,
                _handle: ManuallyDrop::new(handle),
            });
        }

        for entry in &controller.gc_synchronization_entries {
            let mut listener = Box::new(ArtReplacementSynchronizationListener {
                controller: controller.clone(),
                timing: entry.timing,
            });
            let handle = interceptor
                .attach(
                    NativePointer(entry.address as *mut c_void),
                    listener.as_mut(),
                )
                .map_err(|error| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: format!("unable to hook ART replacement GC synchronization: {error:?}"),
                })?;
            gc_listeners.push(HookedGcSynchronization {
                _listener: listener,
                _handle: ManuallyDrop::new(handle),
            });
        }

        let get_oat_quick_method_header =
            if let Some(function) = controller.get_oat_quick_method_header {
                match interceptor.replace(
                    NativePointer(function as *mut c_void),
                    NativePointer(on_art_method_get_oat_quick_method_header as *mut c_void),
                    NativePointer(ptr::null_mut()),
                ) {
                    Ok(original) => {
                        ORIGINAL_GET_OAT_QUICK_METHOD_HEADER
                            .store(original.0 as usize, Ordering::SeqCst);
                        Some(ReplacedGetOatQuickMethodHeader {
                            _function: NativePointer(function as *mut c_void),
                            _original: original,
                        })
                    }
                    Err(error) => {
                        return Err(Error::UnsupportedFeature {
                            feature: FEATURE_METHOD_REPLACEMENT,
                            reason: format!(
                                "unable to hook ArtMethod::GetOatQuickMethodHeader: {error:?}"
                            ),
                        });
                    }
                }
            } else {
                None
            };

        Ok(Self {
            _interceptor: interceptor,
            _listeners: listeners,
            _gc_listeners: gc_listeners,
            _get_oat_quick_method_header: get_oat_quick_method_header,
        })
    }
}

impl InvocationListener for ArtMethodTranslationListener {
    fn on_enter(&mut self, context: InvocationContext<'_>) {
        let method = context.arg(0);
        let thread = self.art_thread(&context);
        let translated = self
            .controller
            .translate_method_argument_for_thread(method, thread);
        if translated != method {
            context.set_arg(0, translated);
        }
    }

    fn on_leave(&mut self, _context: InvocationContext<'_>) {}
}

impl ArtMethodTranslationListener {
    fn art_thread(&self, context: &InvocationContext<'_>) -> usize {
        match self.source {
            ArtMethodTranslationSource::InterpreterDoCall => context.arg(1),
            ArtMethodTranslationSource::QuickEntrypoint => {
                #[cfg(target_arch = "aarch64")]
                {
                    context.cpu_context().reg(19) as usize
                }

                #[cfg(not(target_arch = "aarch64"))]
                {
                    0
                }
            }
        }
    }
}

impl InvocationListener for ArtReplacementSynchronizationListener {
    fn on_enter(&mut self, _context: InvocationContext<'_>) {
        if self.timing == GcSynchronizationTiming::OnEnter {
            self.controller.synchronize_replacement_methods();
        }
    }

    fn on_leave(&mut self, _context: InvocationContext<'_>) {
        if self.timing == GcSynchronizationTiming::OnLeave {
            self.controller.synchronize_replacement_methods();
        }
    }
}

pub(super) unsafe extern "C" fn on_art_method_get_oat_quick_method_header(
    method: *mut c_void,
    pc: usize,
) -> *mut c_void {
    if ART_REPLACEMENT_CONTROLLER.get().is_some_and(|controller| {
        controller.is_replacement_method(method) || controller.has_dispatch_thunk_pc(method, pc)
    }) {
        return ptr::null_mut();
    }

    let original = ORIGINAL_GET_OAT_QUICK_METHOD_HEADER.load(Ordering::SeqCst);
    if original == 0 {
        return ptr::null_mut();
    }

    let original: GetOatQuickMethodHeader = unsafe { std::mem::transmute(original) };
    unsafe { original(method, pc) }
}

// Gum's interceptor objects are process-global and protected internally. The controller only
// mutates its map through a mutex, and hooks are installed once for the lifetime of the backend.
unsafe impl Send for ArtReplacementController {}
unsafe impl Sync for ArtReplacementController {}
unsafe impl Send for ArtReplacementHooks {}
unsafe impl Sync for ArtReplacementHooks {}

// Replacement guards own process-runtime ART patch state. Revert may run from any attached thread, and
// the backend/controller mutate shared process state behind their own synchronization.
unsafe impl Send for ArtMethodReplacementGuard {}

impl ArtMethodReplacementGuard {
    pub(crate) fn revert(&mut self) -> Result<()> {
        if self.reverted {
            return Ok(());
        }
        self.backend
            .restore_method(&self.vm, self.method, &self.layout, self.original)?;
        self.backend.replacement_controller.unregister(self.method);
        self.reverted = true;
        Ok(())
    }

    // Retained as an internal backend diagnostic for host tests; it is no longer public facade API.
    #[allow(dead_code)]
    pub(crate) fn debug_summary(&self) -> String {
        format!(
            "backend=clone-active, method={:?}, cloned_method={:?}, dispatch_thunk={:?}, api_level={}, jni_ids_indirection={:?}, uses_indirect_jni_ids={}, method_size={}, access_flags_offset={}, jni_code_offset={}, quick_code_offset={}, interpreter_code_offset={:?}, thread_managed_stack_offset={}, quick_generic_jni_trampoline={:?}, quick_to_interpreter_bridge_trampoline={:?}, do_call_hooks={}, quick_entrypoint_hooks={}, get_oat_quick_method_header_hook={}, gc_synchronization_hooks={}, original={{access_flags=0x{:08x}, jni_code={:?}, quick_code={:?}, interpreter_code={:?}}}, original_patched={{access_flags=0x{:08x}, jni_code={:?}, quick_code={:?}, interpreter_code={:?}}}, clone_patched={{access_flags=0x{:08x}, jni_code={:?}, quick_code={:?}, interpreter_code={:?}}}",
            self.method,
            self.cloned_method.as_ptr(),
            self.dispatch_thunk.as_ptr(),
            self.layout.api_level,
            self.layout.runtime.jni_ids_indirection,
            self.layout.runtime.uses_indirect_jni_ids(),
            self.layout.method.method_size,
            self.layout.method.access_flags_offset,
            self.layout.method.jni_code_offset,
            self.layout.method.quick_code_offset,
            self.layout.method.interpreter_code_offset,
            self.layout.thread_managed_stack_offset,
            self.layout.trampolines.quick_generic_jni_trampoline,
            self.layout
                .trampolines
                .quick_to_interpreter_bridge_trampoline,
            self.backend.replacement_controller.do_call_entries.len(),
            self.backend
                .replacement_controller
                .quick_entrypoint_hooks
                .lock()
                .expect("ART replacement quick hooks mutex poisoned")
                .hooks
                .len(),
            self.backend
                .replacement_controller
                .get_oat_quick_method_header
                .is_some(),
            self.backend
                .replacement_controller
                .gc_synchronization_entries
                .len(),
            self.original.access_flags,
            self.original.jni_code,
            self.original.quick_code,
            self.original.interpreter_code,
            self.original_patched.access_flags,
            self.original_patched.jni_code,
            self.original_patched.quick_code,
            self.original_patched.interpreter_code,
            self.clone_patched.access_flags,
            self.clone_patched.jni_code,
            self.clone_patched.quick_code,
            self.clone_patched.interpreter_code,
        )
    }
}

impl Drop for ArtMethodReplacementGuard {
    fn drop(&mut self) {
        if !self.reverted && self.revert().is_err() {
            // Keep cloned method and dispatch thunk memory mapped if ART may still branch to them.
            self.cloned_method.leak();
            self.dispatch_thunk.leak();
            self.reverted = true;
        }
    }
}

impl ArtMethodClone {
    pub(super) fn copy_from(
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

    pub(super) fn as_ptr(&self) -> *mut c_void {
        self.method.as_ptr()
    }

    pub(super) fn memory_ranges(&self) -> MemoryRanges {
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

pub(crate) fn original_method_call_bypass(
    method: usize,
    thread: usize,
) -> OriginalMethodCallBypass {
    let method = ART_REPLACEMENT_CONTROLLER
        .get()
        .map_or(method, |controller| {
            controller.art_method_for_jni_id(method)
        });
    let lock = if thread != 0 && ORIGINAL_CALL_BYPASS_OWNER_THREAD.load(Ordering::SeqCst) == thread
    {
        None
    } else {
        let lock = ORIGINAL_CALL_BYPASS_LOCK
            .lock()
            .expect("ART original-call bypass mutex poisoned");
        ORIGINAL_CALL_BYPASS_OWNER_THREAD.store(thread, Ordering::SeqCst);
        Some(lock)
    };
    let previous = ORIGINAL_CALL_BYPASS_METHOD.swap(method, Ordering::SeqCst);
    let previous_thread = ORIGINAL_CALL_BYPASS_THREAD.swap(thread, Ordering::SeqCst);
    OriginalMethodCallBypass {
        _lock: lock,
        previous,
        previous_thread,
    }
}

impl Drop for OriginalMethodCallBypass {
    fn drop(&mut self) {
        ORIGINAL_CALL_BYPASS_METHOD.store(self.previous, Ordering::SeqCst);
        ORIGINAL_CALL_BYPASS_THREAD.store(self.previous_thread, Ordering::SeqCst);
        if self._lock.is_some() {
            ORIGINAL_CALL_BYPASS_OWNER_THREAD.store(0, Ordering::SeqCst);
        }
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

    write_original_call_bypass_check(&writer, ORIGINAL)?;

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

pub(super) fn write_original_call_bypass_check(
    writer: &Aarch64InstructionWriter,
    original_label: u64,
) -> Result<()> {
    const NOT_ORIGINAL: u64 = 4;

    ensure_writer(
        writer.put_ldr_reg_u64(
            Aarch64Register::X16,
            (&ORIGINAL_CALL_BYPASS_METHOD as *const AtomicUsize) as u64,
        ),
        "emit original-call bypass cell load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_reg_offset(Aarch64Register::X16, Aarch64Register::X16, 0),
        "emit original-call bypass method load",
    )?;
    ensure_writer(
        writer.put_cmp_reg_reg(Aarch64Register::X0, Aarch64Register::X16),
        "emit original-call bypass comparison",
    )?;
    writer.put_bcond_label(Aarch64BranchCondition::Ne, NOT_ORIGINAL);
    ensure_writer(
        writer.put_ldr_reg_u64(
            Aarch64Register::X16,
            (&ORIGINAL_CALL_BYPASS_THREAD as *const AtomicUsize) as u64,
        ),
        "emit original-call bypass thread cell load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_reg_offset(Aarch64Register::X16, Aarch64Register::X16, 0),
        "emit original-call bypass thread load",
    )?;
    ensure_writer(
        writer.put_cmp_reg_reg(Aarch64Register::X19, Aarch64Register::X16),
        "emit original-call bypass thread comparison",
    )?;
    writer.put_bcond_label(Aarch64BranchCondition::Eq, original_label);
    writer.put_label(NOT_ORIGINAL);
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
    pub(super) fn new(
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

    pub(super) fn len(&self) -> usize {
        self.length
    }

    fn leak(&mut self) {
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
