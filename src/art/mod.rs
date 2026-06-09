//! Internal Android ART integration.
//!
//! This module is internal. It discovers ART symbols, probes runtime layouts, manages
//! runnable-thread state, enumerates runtime data, and installs guarded method replacements.
//!
//! ### Safety Design
//!
//! ART layout changes between Android versions and devices. Public APIs should only expose features
//! that this backend has proved available for the current process; otherwise they return
//! `UnsupportedFeature` with a reason.

mod backend;
mod deoptimization;
mod enumeration;
mod features;
mod layout;
mod memory;
mod replacement;
mod resolution;
mod runnable_thread;
mod runtime_layout;
mod strings;
mod symbols;
mod threads;
mod vm_access;

#[cfg(test)]
mod tests;

pub(crate) use backend::{ArtBackend, ArtModuleRange};
pub(crate) use enumeration::{
    ArtClassLoaderHandle, ArtHeapInstanceHandle, ArtLoadedClassHandle, ArtMethodQueryGroup,
};
pub(crate) use replacement::{ArtMethodReplacementGuard, original_method_call_bypass};
pub(crate) use vm_access::{ArtVmAccess, ArtVmHandle};
