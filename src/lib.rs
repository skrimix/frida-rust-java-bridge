//! A Rust-native Java bridge designed for Frida agents running inside Android ART processes.
//!
//! If you want to interact with Java classes, call methods, or replace implementations in a running Android app,
//! this crate provides safe, idiomatic Rust wrappers around Android's internal ART runtime and the raw JNI.
//!
//! ### Getting Started
//!
//! The main entry point is [`Java`], which represents your handle to the Java VM. Most of your work will
//! begin by calling [`Java::obtain`] to get a handle, and then using [`Java::perform`] to execute code inside
//! the application context:
//!
//! ```no_run
//! use frida_rust_java_bridge::{Java, Result};
//!
//! fn install() -> Result<()> {
//!     let java = Java::obtain()?;
//!     // perform() attaches the thread to the VM and schedules our closure to run
//!     // once the application's class loader is fully initialized.
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
//! ### Key Abstractions
//!
//! - **High-Level API:** For everyday tasks, use [`JavaClass`] (to work with classes), [`JavaObject`] (to interact
//!   with Java objects), and [`JavaArray`] (to manipulate arrays). You can call methods, get/set fields, and cast values
//!   with safe Rust types.
//! - **Method Hooking / Replacement:** You can intercept and replace Java methods or constructors using [`JavaHookGuard`].
//!   This lets you run custom Rust code whenever a Java method is called, inspect arguments, call the original implementation,
//!   and return custom results.
//! - **Low-Level & Raw JNI:** If you need fine-grained control or direct JNI access, the [`mod@env`], [`refs`], [`jni`], and
//!   [`JavaValue`] modules provide a thread-safe, safe-Rust surface over raw JNI handles.
//!
//! ### Platform & Compatibility
//!
//! This library targets Android ART exclusively. Under the hood, it dynamically probes ART internals to safely hook
//! and mutate execution states. If a feature (like class enumeration or method replacement) is unsupported on the current
//! Android version or CPU architecture, it will return a clean, structured `UnsupportedFeature` error instead of panicking
//! or crashing.
//!
//! For a detailed look at what is supported and what is coming next, check out `CURRENT_BEHAVIOR.md` and `FEATURE_PROGRESS.md`.

#![cfg_attr(not(target_os = "android"), allow(unused))]
#![allow(private_bounds)]
#![allow(private_interfaces)]

#[cfg(target_os = "android")]
pub(crate) mod android;
#[cfg(all(target_os = "android", feature = "apk-perform-test"))]
mod apk_perform_test;
#[cfg(all(target_os = "android", feature = "app-process-test"))]
mod app_process_test;
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
    FromJavaHookReturn, FromJavaValue, IntoJavaHookReturn, JavaConstructorHookContext,
    JavaConstructorInitialized, JavaHookArgument, JavaHookArguments, JavaHookContext,
    JavaHookError, JavaHookGuard, JavaHookReturn, JavaHookReturnObject, JavaHookSet,
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

/// Builds an explicit Java argument list for calls with many arguments.
///
/// Most method calls can pass `()`, a single value, a tuple, an array, or a slice directly. Use this
/// macro when an argument list is longer than the supported tuple arities or when building a list
/// incrementally would be less clear.
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
