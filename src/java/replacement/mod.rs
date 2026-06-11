//! Guarded Java method and constructor replacement.
//!
//! Use [`JavaMethod::replace`](crate::JavaMethod::replace) to replace a selected method and keep
//! the returned [`JavaHookGuard`] alive while the replacement should remain active. Method
//! callbacks receive [`JavaHookContext`], can call the original implementation, and return values
//! through [`JavaHookContext::ret`].
//!
//! Use [`JavaConstructor::replace`](crate::JavaConstructor::replace) for constructor replacement.
//! Constructor callbacks receive [`JavaHookContext`] and return void.
mod api;
mod arguments;
mod closure;
mod context;
mod original;
mod returns;
mod targets;
mod trampoline;

const FEATURE_CLOSURE_REPLACEMENT: &str = "closure-backed method replacement";

pub use api::{JavaHookError, JavaHookGuard, JavaHookSet};
pub use arguments::{FromJavaValue, JavaHookArgument, JavaHookArguments};
pub use context::JavaHookContext;
pub use returns::{FromJavaHookReturn, IntoJavaHookReturn, JavaHookReturn, JavaHookReturnObject};

#[cfg(test)]
mod tests;
