#![allow(dead_code)]

mod backend;
mod deoptimization;
mod enumeration;
mod features;
mod layout;
mod replacement;
mod runnable_thread;
mod support;
mod symbols;

#[cfg(test)]
mod tests;

pub(crate) use backend::{ArtBackend, ArtModuleRange};
pub(crate) use replacement::{ArtMethodReplacementGuard, original_method_call_bypass};
