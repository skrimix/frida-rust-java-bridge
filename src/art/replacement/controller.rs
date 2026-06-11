use std::{
    collections::HashMap,
    ffi::c_void,
    ptr,
    sync::{Arc, Mutex, OnceLock},
};

use frida_gum::Module;

use super::{
    dispatch::replacement_frame_is_active,
    hooks::{ArtQuickEntrypointHooks, ArtReplacementHooks},
};
use crate::{error::Result, jni};

use super::super::{
    features::*,
    resolution::{
        find_gc_synchronization_entries, find_interpreter_do_call_entries, resolve_pointer_any,
    },
    symbols::{GET_OAT_QUICK_METHOD_HEADER_U32, GET_OAT_QUICK_METHOD_HEADER_USIZE},
};

static ART_REPLACEMENT_CONTROLLER: OnceLock<Arc<ArtReplacementController>> = OnceLock::new();

pub(in crate::art) struct ArtReplacementController {
    pub(super) do_call_entries: Vec<usize>,
    pub(super) get_oat_quick_method_header: Option<*const c_void>,
    pub(super) gc_synchronization_entries: Vec<GcSynchronizationEntry>,
    mappings: Mutex<ArtReplacementMappings>,
    pub(super) quick_entrypoint_hooks: Mutex<ArtQuickEntrypointHooks>,
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
pub(in crate::art) struct ArtReplacementSynchronization {
    pub(in crate::art) quick_code_offset: usize,
    pub(in crate::art) thread_managed_stack_offset: usize,
    pub(in crate::art) nterp_entrypoint: Option<usize>,
    pub(in crate::art) quick_to_interpreter_bridge: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::art) struct GcSynchronizationEntry {
    pub(in crate::art) address: usize,
    pub(in crate::art) timing: GcSynchronizationTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::art) enum GcSynchronizationTiming {
    OnEnter,
    OnLeave,
}

impl ArtReplacementController {
    pub(in crate::art) fn new(module: &Module) -> Self {
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
    pub(in crate::art) fn empty_for_tests() -> Self {
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
    pub(in crate::art) fn with_dispatch_for_tests() -> Self {
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

    pub(in crate::art) fn register(
        &self,
        original: *mut c_void,
        replacement: *mut c_void,
        dispatch_thunk: *mut c_void,
        dispatch_thunk_len: usize,
        synchronization: ArtReplacementSynchronization,
    ) -> Result<()> {
        if original.is_null() || replacement.is_null() || dispatch_thunk.is_null() {
            return Err(crate::error::Error::NullReturn {
                operation: "ART replacement mapping",
            });
        }
        let Some(dispatch_thunk_end) = (dispatch_thunk as usize).checked_add(dispatch_thunk_len)
        else {
            return Err(crate::error::Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "dispatch thunk address range overflowed".to_owned(),
            });
        };
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        if mappings.methods.contains_key(&(original as usize)) {
            return Err(crate::error::Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "target ArtMethod already has an active replacement".to_owned(),
            });
        }
        if mappings.replacements.contains_key(&(replacement as usize)) {
            return Err(crate::error::Error::InvalidReplacementState {
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

    pub(in crate::art) fn register_jni_id(&self, jni_id: jni::jmethodID, original: *mut c_void) {
        if jni_id.is_null() || original.is_null() {
            return;
        }
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.jni_ids.insert(jni_id as usize, original as usize);
    }

    pub(in crate::art) fn unregister(&self, original: *mut c_void) {
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

    pub(in crate::art) fn replacement_for(&self, original: *mut c_void) -> Option<*mut c_void> {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings
            .methods
            .get(&(original as usize))
            .map(|record| record.replacement as *mut c_void)
    }

    pub(in crate::art) fn is_replacement_method(&self, method: *mut c_void) -> bool {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.replacements.contains_key(&(method as usize))
    }

    pub(in crate::art) fn has_dispatch_thunk_pc(&self, method: *mut c_void, pc: usize) -> bool {
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

    pub(in crate::art) fn art_method_for_jni_id(&self, method: usize) -> usize {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.jni_ids.get(&method).copied().unwrap_or(method)
    }

    #[cfg(test)]
    pub(in crate::art) fn translate_method_argument(&self, method: usize) -> usize {
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

    pub(in crate::art) fn synchronize_replacement_methods(&self) {
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

    #[cfg(test)]
    pub(super) fn quick_entrypoint_hook_count(&self) -> usize {
        self.quick_entrypoint_hooks
            .lock()
            .expect("ART replacement quick hooks mutex poisoned")
            .hooks
            .len()
    }

    #[cfg(test)]
    pub(super) fn has_get_oat_quick_method_header_hook(&self) -> bool {
        self.get_oat_quick_method_header.is_some()
    }

    #[cfg(test)]
    pub(super) fn gc_synchronization_hook_count(&self) -> usize {
        self.gc_synchronization_entries.len()
    }
}

pub(super) fn global_replacement_controller() -> Option<&'static Arc<ArtReplacementController>> {
    ART_REPLACEMENT_CONTROLLER.get()
}

// Gum's interceptor objects are process-global and protected internally. The controller only
// mutates its map through a mutex, and hooks are installed once for the lifetime of the backend.
unsafe impl Send for ArtReplacementController {}
unsafe impl Sync for ArtReplacementController {}
