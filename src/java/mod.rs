//! Safe, high-level API for interacting with the Java Virtual Machine.
//!
//! This module provides the primary ways to locate Java classes, construct objects, call methods,
//! read or write fields, and execute tasks on the main Android thread.
//!
//! ### Choosing How to Run Your Code
//!
//! Depending on when and how your agent needs to execute, you have three primary entry points:
//!
//! 1. **[`Java::perform`]:** The most robust entry point for typical hooks. If the application is still starting up,
//!    `perform` will safely wait until the main Android class loader is available before running your closure. It automatically
//!    attaches the current thread to the VM and sets up the app class loader scope.
//! 2. **[`Java::perform_now`]:** Runs your closure synchronously and immediately on the current thread, assuming the VM is
//!    ready and a class loader has already been published.
//! 3. **[`Java::attach`]:** Enters a manual synchronous scope (returning a [`JavaScope`]) which keeps the current thread
//!    attached to the VM. This is useful when you want to perform multiple step-by-step JNI operations and control the lifecycle yourself.
//!
//! ### Working with Java Types
//!
//! - **Classes:** Use [`Java::use_class`] to resolve a class name (like `"java.lang.String"`) and get a [`JavaClass`] wrapper.
//! - **Objects:** Use [`JavaObject`] to interact with Java instances.
//! - **Arrays:** Use [`JavaArray`] to read or write elements of Java arrays.
//! - **Advanced/Raw JNI:** If high-level wrappers are too restrictive, the [`raw`] sub-module provides safe access to the
//!   underlying raw class definitions and direct [`JavaValue`] arguments.

use std::{
    collections::HashMap,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
};

use crate::{
    env::{AttachedEnv, Env},
    error::Result,
    jni,
    signature::JavaType,
    value::JavaValue,
    vm::Vm,
};

#[macro_use]
mod macros;

mod args;
mod array;
mod class;
mod conversion;
mod dispatch;
mod handle;
mod loader;
mod lookup;
mod main_thread;
mod members;
mod object;
mod perform;
pub mod raw;
pub mod replacement;
mod returns;

mod sealed {
    pub trait IntoJavaFieldValueSealed {}
}

use self::{
    array::{
        array_from_ref_with_class, array_from_ref_with_declared, object_from_ref_with_declared,
    },
    loader::app_class_loader_from_activity_thread,
    lookup::{find_class_with_loader, normalize_class_lookup_name},
    main_thread::MainThreadState,
    perform::{
        AppPerformState, PendingPerform, app_perform_state, complete_perform,
        default_app_loader_global, default_java_global, perform_callback_with_result,
    },
};

pub(crate) use self::{
    main_thread::main_thread_scheduling_support, perform::app_loader_deferral_support,
    returns::display_java_char,
};
pub use self::{
    main_thread::{MainThreadTaskHandle, MainThreadTaskStatus},
    perform::{PerformHandle, PerformResult, PerformStatus},
};
pub use crate::loader::{ClassLoaderKind, ClassLoaderRef};

static APP_PERFORM_STATE: OnceLock<AppPerformState> = OnceLock::new();
static MAIN_THREAD_STATE: OnceLock<MainThreadState> = OnceLock::new();

/// The main coordinator and entry point for all Java operations in the process.
///
/// Use this struct to obtain a VM handle, schedule callbacks, query system capabilities,
/// and load Java classes.
///
/// ### Example
///
/// ```no_run
/// use frida_rust_java_bridge::Java;
///
/// let java = Java::obtain().unwrap();
/// java.perform(|java| {
///     let activity = java.use_class("android.app.Activity").unwrap();
///     // Your Java work here...
///     Ok(())
/// }).unwrap();
/// ```
///
/// ### Class Loader Resolution
///
/// By default, a bare `Java` handle performs low-level bootstrap lookups (looking up core Java classes like
/// `java.lang.String`).
///
/// - When you use [`Java::perform`], it automatically configures the handle to prefer the application's class loader,
///   allowing you to load custom classes defined by the Android app.
/// - If you need to search in a specific custom loader, you can use [`Java::with_loader`] to scope
///   all class lookups to that specific class loader instance.
#[derive(Clone)]
pub struct Java {
    vm: Vm,
    loader: Option<ClassLoaderRef>,
    classes: Arc<Mutex<HashMap<String, raw::Class>>>,
}

/// An active execution scope representing a thread attached to the Java VM.
///
/// A `JavaScope` is passed directly into your callbacks when using [`Java::perform`] or [`Java::perform_now`],
/// and is returned as a lifetime guard by [`Java::attach`]. It guarantees that the current thread remains
/// attached to the Java VM for the duration of the scope, and automatically handles cleaning up references when dropped.
///
/// ### Usage
///
/// `JavaScope` dereferences to [`Java`], meaning you can directly call any high-level method (like [`JavaScope::use_class`],
/// [`JavaScope::new_string_utf`], or [`JavaScope::new_boolean_array`]) directly on it.
///
/// If you need to drop down to the low-level raw JNI layer for advanced operations, call [`.env()`](JavaScope::env) to get
/// access to the raw JNI environment.
pub struct JavaScope<'java> {
    java: &'java Java,
    env: AttachedEnv<'java>,
    _thread_affine: PhantomData<Rc<()>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    assert_impl_all!(Java: Send, Sync);
    assert_impl_all!(JavaClass: Send, Sync);
    assert_not_impl_any!(JavaScope<'static>: Send, Sync);
    assert_impl_all!(JavaObject: Send, Sync);
    assert_impl_all!(JavaArray: Send, Sync);
    assert_impl_all!(raw::Class: Send, Sync);
    assert_impl_all!(ClassLoaderRef: Send, Sync);
    assert_not_impl_any!(JavaLocalObject<'static>: Send, Sync);
    assert_not_impl_any!(JavaLocalArray<'static>: Send, Sync);
}

pub use self::{
    array::{JavaArray, JavaLocalArray},
    class::{JavaChooseControl, JavaClass},
    members::{
        JavaConstructor, JavaField, JavaFieldReceiver, JavaMethod, JavaMethodGroup,
        JavaMethodReceiver,
    },
    object::{
        JavaBoundFieldHandle, JavaBoundMethodGroup, JavaBoundMethodOverload, JavaLocalObject,
        JavaObject,
    },
    returns::{JavaLocalReturn, JavaLocalReturnRef, JavaReturn, JavaReturnRef},
};

/// A helper trait to convert Rust values and collections into JNI call arguments.
///
/// You do not typically need to call this trait's methods directly. It is implemented for a wide
/// range of Rust types so that you can pass arguments to Java methods naturally:
/// - `()` for zero-argument calls.
/// - A single value (like `5` or `true`) for single-argument calls.
/// - A tuple (like `(5, true, "hello")`) for multiple arguments.
/// - Arrays, slices, and vectors of compatible types.
/// - [`JavaArgs`] when building argument lists dynamically or passing a very large number of arguments.
pub trait IntoJavaArgs {
    fn into_java_args(self) -> Vec<JavaValue>;
}

/// An explicit list of Java arguments, useful when standard tuple syntax isn't enough.
///
/// While you can usually pass arguments as standard Rust tuples or values, `JavaArgs` is perfect when:
/// - You have a dynamic number of arguments that you need to build at runtime.
/// - You have more arguments than the maximum arity supported by tuple helper traits.
///
/// Use the [`java_args!`](crate::java_args) macro to easily construct this list at your call sites.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JavaArgs {
    values: Vec<JavaValue>,
}

/// Converts high-level wrapper call arguments into JNI argument values.
///
/// Unlike [`IntoJavaArgs`], this conversion sees the selected overload's parameter types and the
/// current JNI environment. This lets wrapper calls accept Rust strings for `java.lang.String` and
/// `java.lang.Object` parameters while keeping the temporary `jstring` local references alive until
/// the JNI call returns.
///
/// This trait is intentionally sealed through its private overload-argument supertrait. Use the supported
/// argument shapes instead of implementing it yourself: `()`, one supported argument, tuples,
/// arrays, slices, vectors, [`JavaArgs`], and [`JavaValue`] lists.
pub trait IntoJavaCallArgs: IntoJavaOverloadArgs {
    fn into_java_call_args<'env, 'scope>(
        self,
        env: &'env Env<'scope>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'scope>>;
}

pub(crate) trait IntoJavaOverloadArgs {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg>;
}

/// Extracts a typed Rust value from a wrapper method or field return.
pub trait FromJavaReturn: Sized {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self>;
}

/// Converts high-level field assignment values into JNI field values.
///
/// This trait is sealed. Field assignment supports Rust values convertible into [`JavaValue`] plus
/// Rust string values for Java string-compatible fields.
pub trait IntoJavaFieldValue: sealed::IntoJavaFieldValueSealed {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue>;
}

#[doc(hidden)]
/// Internal prepared call-argument storage.
///
/// This type is public only because it appears in the sealed [`IntoJavaCallArgs`] trait method. It
/// is not a supported extension point for external implementations.
pub struct PreparedJavaCallArgs<'env, 'scope> {
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
    cleanup_env: &'env Env<'scope>,
}

#[doc(hidden)]
/// Internal overload-selection argument storage.
///
/// This type is public only because it appears in the sealed call-argument plumbing.
pub enum JavaOverloadArg {
    Value(JavaValue),
    RustString(String),
}

#[doc(hidden)]
/// Internal prepared field-value storage.
///
/// This type is public only because it appears in the sealed [`IntoJavaFieldValue`] trait method.
/// It is not a supported extension point for external implementations.
pub struct PreparedJavaFieldValue {
    value: JavaValue,
    local_ref: Option<jni::jobject>,
}

pub(crate) struct AttachedJavaCallArgs<'scope> {
    env: AttachedEnv<'scope>,
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
}
