use std::{collections::HashSet, ffi::c_void};

use frida_gum::Module;

use super::{
    replacement::{GcSynchronizationEntry, GcSynchronizationTiming},
    symbols::*,
};
use crate::native::native_pointer_to_fn;

pub(super) fn resolve<T: Copy>(module: &Module, symbol: &'static str) -> Option<T> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .map(native_pointer_to_fn)
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
