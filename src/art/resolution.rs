use std::{collections::HashSet, ffi::c_void, sync::Arc};

use frida_gum::Module;

use super::{
    backend::{DecodeGlobalKind, GetInstancesKind, PrettyMethod, SuspendAll, VisitClassesKind},
    enumeration::PrettyMethodFunction,
    memory::ExecutableMemory,
    replacement::{GcSynchronizationEntry, GcSynchronizationTiming},
    symbols::*,
};
use crate::runtime::native_pointer_to_fn;

pub(super) fn resolve<T: Copy>(module: &Module, symbol: &'static str) -> Option<T> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .and_then(|pointer| native_pointer_to_fn(pointer).ok())
}

pub(super) fn resolve_get_instances(module: &Module) -> Option<GetInstancesKind> {
    resolve(module, GET_INSTANCES)
        .map(GetInstancesKind::Exact)
        .or_else(|| resolve(module, GET_INSTANCES_ASSIGNABLE).map(GetInstancesKind::WithAssignable))
}

pub(super) fn resolve_decode_global(module: &Module) -> Option<DecodeGlobalKind> {
    resolve(module, DECODE_GLOBAL_NO_THREAD)
        .map(DecodeGlobalKind::NoThread)
        .or_else(|| resolve(module, DECODE_GLOBAL_WITH_THREAD).map(DecodeGlobalKind::WithThread))
        .or_else(|| resolve(module, THREAD_DECODE_GLOBAL_JOBJECT).map(DecodeGlobalKind::Thread))
}

pub(super) fn resolve_pointer(module: &Module, symbol: &'static str) -> Option<*const c_void> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .map(|pointer| pointer.0 as *const c_void)
}

pub(super) fn resolve_pointer_any(
    module: &Module,
    symbols: &[&'static str],
) -> Option<*const c_void> {
    symbols
        .iter()
        .find_map(|symbol| resolve_pointer(module, symbol))
}

pub(super) fn resolve_any<T: Copy>(module: &Module, symbols: &[&'static str]) -> Option<T> {
    symbols.iter().find_map(|symbol| resolve(module, symbol))
}

pub(super) fn resolve_pretty_method(module: &Module) -> Option<PrettyMethodFunction> {
    let pointer = resolve_pointer_any(module, &[PRETTY_METHOD, PRETTY_METHOD_NULL_SAFE])?;
    #[cfg(target_arch = "aarch64")]
    {
        let thunk = Arc::new(ExecutableMemory::aarch64_pretty_method_thunk(pointer).ok()?);
        let function = unsafe {
            std::mem::transmute_copy::<*mut c_void, PrettyMethod>(&thunk.pointer.as_ptr())
        };
        Some(PrettyMethodFunction {
            function,
            _thunk: Some(thunk),
        })
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let function = native_pointer_to_fn(frida_gum::NativePointer(pointer as usize)).ok()?;
        Some(PrettyMethodFunction {
            function,
            _thunk: None,
        })
    }
}

pub(super) fn resolve_suspend_all(module: &Module) -> Option<SuspendAll> {
    resolve(module, SUSPEND_ALL_WITH_CAUSE)
        .map(SuspendAll::WithCause)
        .or_else(|| resolve(module, SUSPEND_ALL_LEGACY).map(SuspendAll::Legacy))
}

pub(super) fn resolve_visit_classes(module: &Module) -> Option<VisitClassesKind> {
    resolve(module, VISIT_CLASSES_VISITOR)
        .map(VisitClassesKind::Visitor)
        .or_else(|| resolve(module, VISIT_CLASSES_CALLBACK).map(VisitClassesKind::Callback))
}

pub(super) fn find_interpreter_do_call_entries(module: &Module) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for export in module.enumerate_exports() {
        if is_interpreter_do_call_symbol(&export.name) && seen.insert(export.address) {
            entries.push(export.address);
        }
    }
    for symbol in module.enumerate_symbols() {
        if is_interpreter_do_call_symbol(&symbol.name) && seen.insert(symbol.address) {
            entries.push(symbol.address);
        }
    }

    entries
}

pub(super) fn find_gc_synchronization_entries(module: &Module) -> Vec<GcSynchronizationEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    if let Some(address) = resolve_pointer(module, GC_COLLECT_GARBAGE_INTERNAL) {
        push_gc_synchronization_entry(
            &mut entries,
            &mut seen,
            address,
            GcSynchronizationTiming::OnLeave,
        );
    }
    if let Some(address) = resolve_pointer_any(
        module,
        &[
            CONCURRENT_COPYING_COPYING_PHASE,
            CONCURRENT_COPYING_MARKING_PHASE,
        ],
    ) {
        push_gc_synchronization_entry(
            &mut entries,
            &mut seen,
            address,
            GcSynchronizationTiming::OnLeave,
        );
    }
    if let Some(address) = resolve_pointer_any(
        module,
        &[THREAD_RUN_FLIP_FUNCTION, THREAD_RUN_FLIP_FUNCTION_WITH_FLAG],
    ) {
        push_gc_synchronization_entry(
            &mut entries,
            &mut seen,
            address,
            GcSynchronizationTiming::OnEnter,
        );
    }

    entries
}

pub(super) fn push_gc_synchronization_entry(
    entries: &mut Vec<GcSynchronizationEntry>,
    seen: &mut HashSet<usize>,
    address: *const c_void,
    timing: GcSynchronizationTiming,
) {
    let address = address as usize;
    if seen.insert(address) {
        entries.push(GcSynchronizationEntry { address, timing });
    }
}

pub(super) fn is_interpreter_do_call_symbol(name: &str) -> bool {
    name.starts_with("_ZN3art11interpreter6DoCall")
        && name.contains("ArtMethod")
        && name.contains("ShadowFrame")
        && name.contains("JValue")
}
