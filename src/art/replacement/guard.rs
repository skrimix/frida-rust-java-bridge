use std::ffi::c_void;

use super::super::{
    ArtVmHandle,
    backend::ArtBackend,
    layout::{ArtMethodReplacementLayout, ArtMethodSnapshot},
};
use super::{ArtMethodClone, ArtMethodDispatchThunk};
use crate::error::Result;

pub(crate) struct ArtMethodReplacementGuard {
    pub(in crate::art) backend: ArtBackend,
    pub(in crate::art) vm: ArtVmHandle,
    pub(in crate::art) method: *mut c_void,
    pub(in crate::art) cloned_method: ArtMethodClone,
    pub(in crate::art) dispatch_thunk: ArtMethodDispatchThunk,
    pub(in crate::art) layout: ArtMethodReplacementLayout,
    pub(in crate::art) original: ArtMethodSnapshot,
    #[cfg(test)]
    pub(in crate::art) original_patched: ArtMethodSnapshot,
    #[cfg(test)]
    pub(in crate::art) clone_patched: ArtMethodSnapshot,
    pub(in crate::art) reverted: bool,
}

// Replacement guards own process-runtime ART patch state. Revert may run from any attached thread,
// and the backend/controller mutate shared process state behind their own synchronization.
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

    #[cfg(test)]
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
                .quick_entrypoint_hook_count(),
            self.backend
                .replacement_controller
                .has_get_oat_quick_method_header_hook(),
            self.backend
                .replacement_controller
                .gc_synchronization_hook_count(),
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
