//! A Rust Java bridge for Frida agents running inside Android Runtime (ART) processes.
//!
//! Use this crate when a Rust agent needs to work with Java classes, call methods, inspect
//! objects, or replace implementations in a running Android app.
//!
//! ### Getting Started
//!
//! Start with [`Java::obtain()`]. To work with application classes, initialize the app class
//! loader using one of these methods:
//!
//! - [`Java::perform()`] queues a callback that runs when the loader is ready (non-blocking).
//! - [`Java::wait_for_app_loader()`] returns immediately when the loader is already known or
//!   currently available, otherwise it blocks until the timeout expires.
//!
//! After initialization, high-level Java APIs can be called directly. They attach the current
//! thread as needed; use [`Java::attach()`] when you want several synchronous operations to reuse
//! one attached scope or when code needs direct JNI-style access.
//!
//! ```no_run
//! #[cfg(target_os = "android")]
//! use frida_rust_java_bridge::{Java, Result};
//!
//! #[cfg(target_os = "android")]
//! fn install() -> Result<()> {
//!     let java = Java::obtain()?;
//!     java.perform(|java| {
//!         let activity = java.use_class("android.app.Activity")?;
//!         let name: String = activity.call("getName", ())?;
//!         let _ = name;
//!         Ok(())
//!     })?;
//!     Ok(())
//! }
//! ```
//!
//! ### Main Pieces
//!
//! - [`JavaClass`] lets you construct objects, call static methods, read fields, and install
//!   replacements.
//! - [`JavaObject`] and [`JavaArray`] keep Java references alive while you work with instances
//!   and arrays from Rust.
//! - [`JavaHookGuard`] owns installed method and constructor replacements. Keep the guard alive
//!   while the replacement should stay active.
//! - [`mod@env`], [`refs`], [`jni`], and [`JavaValue`] are for code that deliberately crosses
//!   into JNI-shaped APIs.
//!
//! ### Platform
//!
//! This library targets Android Runtime (ART) only. ART details vary across devices, so features such as
//! class enumeration, heap enumeration, deoptimization, and method replacement are probed at
//! runtime. Unsupported features return [`Error::UnsupportedFeature`] with a reason.
//!
#![cfg_attr(not(target_os = "android"), allow(unused))]
#![allow(private_bounds)]
#![allow(private_interfaces)]

#[cfg(target_os = "android")]
pub(crate) mod android;
#[cfg(target_os = "android")]
pub(crate) mod art;
#[cfg(target_os = "android")]
mod capabilities;
pub(crate) mod coercion;
#[cfg(target_os = "android")]
pub mod env;
/// Shared error and result types returned by bridge operations.
pub mod error;
#[cfg(target_os = "android")]
pub mod java;
#[cfg(target_os = "android")]
mod loader;
#[cfg(target_os = "android")]
mod method_query;
#[cfg(target_os = "android")]
mod native;
// Raw JNI type aliases are platform-unconditional so host builds can still name Java values,
// signatures, and raw handles without linking Android runtime support. Operations that
// touch a real VM stay behind the Android-gated modules above.
#[cfg(all(target_os = "android", feature = "__art-selftest"))]
#[doc(hidden)]
pub mod art_selftest;
pub mod jni;
#[cfg(target_os = "android")]
/// Reflection-style class, method, and field metadata returned by Java facade queries.
pub mod metadata;
#[cfg(target_os = "android")]
pub mod refs;
#[cfg(target_os = "android")]
mod runtime;
pub mod signature;
pub mod value;
#[cfg(target_os = "android")]
/// Java VM discovery, attachment, and thread guard APIs.
pub mod vm;

#[cfg(target_os = "android")]
pub use android::AndroidVersion;
#[cfg(target_os = "android")]
pub use capabilities::{FeatureSupport, JavaCapabilities};
pub use error::{Error, Result};
#[cfg(target_os = "android")]
pub use java::replacement::{
    FromJavaHookReturn, FromJavaValue, IntoJavaHookReturn, JavaHookArgument, JavaHookArguments,
    JavaHookContext, JavaHookError, JavaHookGuard, JavaHookReturn, JavaHookReturnObject,
    JavaHookSet,
};
#[cfg(target_os = "android")]
pub use java::{
    FromJavaReturn, IntoJavaFieldValue, Java, JavaArgs, JavaArray, JavaChooseControl, JavaClass,
    JavaConstructor, JavaField, JavaLocalArray, JavaLocalObject, JavaLocalReturn,
    JavaLocalReturnRef, JavaMethod, JavaObject, JavaReturn, JavaReturnRef, JavaScope,
    MainThreadTaskHandle, MainThreadTaskStatus, PerformHandle, PerformResult, PerformStatus,
};
#[cfg(target_os = "android")]
pub use loader::{ClassLoaderKind, ClassLoaderRef};
#[cfg(target_os = "android")]
pub use metadata::modifiers;
#[cfg(target_os = "android")]
pub use metadata::{
    JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryClass,
    JavaMethodQueryGroup,
};
pub use signature::{JavaType, MethodSignature};
pub use value::JavaValue;

/// Builds an explicit Java argument list.
///
/// Most calls can pass `()`, one value, a tuple, an array, or a slice directly. Use this macro
/// when the argument count is dynamic or longer than the supported tuple arities.
#[cfg(target_os = "android")]
#[macro_export]
macro_rules! java_args {
    ($($value:expr),* $(,)?) => {{
        let mut args = $crate::JavaArgs::with_capacity(0usize $(+ {
            let _ = stringify!($value);
            1usize
        })*);
        $(
            args.push($value);
        )*
        args
    }};
}
