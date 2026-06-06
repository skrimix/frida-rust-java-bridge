//! Under-the-hood Android ART (Android Runtime) interaction layer.
//!
//! This module contains internal mechanics for parsing and interacting with Android's ART engine.
//! It is primarily maintainer-facing and handles tasks like:
//! - Dynamic discovery of ART runtime symbols.
//! - Probing memory layouts of classes and methods across different Android versions.
//! - Managing runnable-thread states during hooks.
//! - Safely deoptimizing and mutating method entry points at runtime.
//!
//! ### Safety Design
//!
//! Direct ART memory layout modification is highly delicate. To keep this library stable and safe, all
//! direct runtime mutations are kept behind strict unsafe boundaries. When a feature is not supported
//! on the current Android version, runtime layout, or CPU architecture, the backend will report a
//! structured `UnsupportedFeature` error instead of guessing or causing process instability.

mod backend;
mod capabilities;
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

#[cfg(test)]
mod tests;

pub(crate) use backend::{ArtBackend, ArtModuleRange};
pub(crate) use enumeration::{
    ArtClassLoaderHandle, ArtHeapInstanceHandle, ArtLoadedClassHandle, ArtMethodQueryGroup,
};
pub(crate) use replacement::{ArtMethodReplacementGuard, original_method_call_bypass};
