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

#[cfg(test)]
mod tests;

pub(crate) use backend::{ArtBackend, ArtModuleRange};
pub(crate) use replacement::{ArtMethodReplacementGuard, original_method_call_bypass};
