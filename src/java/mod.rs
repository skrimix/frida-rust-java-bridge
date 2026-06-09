//! High-level Java API.
//!
//! This is the normal place to start when Rust code needs to work with Java classes, objects,
//! arrays, fields, methods, hooks, or Android main-thread callbacks.
//!
//! ### Choosing How to Run Your Code
//!
//! There are three common ways to enter Java:
//!
//! 1. [`Java::perform`] runs app-class work once the application's class loader is known.
//! 2. [`Java::perform_now`] runs immediately in this handle's current loader scope.
//! 3. [`Java::attach`] returns a guard that keeps the current thread attached while you keep it.
//!
//! ### Working with Java Types
//!
//! Use [`Java::use_class`] to get a [`JavaClass`], then use [`JavaObject`] and [`JavaArray`] for
//! instances and arrays. The [`raw`] module and [`JavaValue`] are available when code really needs
//! JNI-shaped values.

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

/// Main handle for Java work in this process.
///
/// A `Java` handle knows which VM it belongs to and, optionally, which class loader should be
/// used for class lookups.
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
/// A bare `Java` handle performs low-level bootstrap lookups, which is useful for core classes
/// such as `java.lang.String`.
///
/// - [`Java::perform`] and [`Java::wait_for_app_loader`] provide a handle scoped to the
///   application's class loader.
/// - [`Java::with_loader`] creates a handle scoped to a specific loader.
#[derive(Clone)]
pub struct Java {
    vm: Vm,
    loader: Option<ClassLoaderRef>,
    classes: Arc<Mutex<HashMap<String, raw::Class>>>,
}

/// Guard for a thread attached to the Java VM.
///
/// `JavaScope` is passed into [`Java::perform`] and [`Java::perform_now`] callbacks, and returned
/// by [`Java::attach`]. The current thread stays attached while the scope is alive.
///
/// ### Usage
///
/// `JavaScope` dereferences to [`Java`], so methods such as [`JavaScope::use_class`],
/// [`JavaScope::new_string_utf`], and [`JavaScope::new_boolean_array`] can be called directly.
///
/// Call [`.env()`](JavaScope::env) when code needs direct JNI-style operations.
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

/// Converts Rust values into Java call arguments.
///
/// You usually don't call this trait directly. It lets wrapper calls accept familiar Rust shapes:
/// - `()` for zero-argument calls.
/// - A single value, such as `5` or `true`.
/// - A tuple, such as `(5, true, "hello")`.
/// - Arrays, slices, and vectors of compatible types.
/// - [`JavaArgs`] for dynamic or very long argument lists.
pub trait IntoJavaArgs {
    fn into_java_args(self) -> Vec<JavaValue>;
}

/// Explicit list of Java arguments.
///
/// Use `JavaArgs` when the argument count is dynamic or larger than the supported tuple arities.
///
/// The [`java_args!`](crate::java_args) macro is the usual way to build one at a call site.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct JavaArgs {
    values: Vec<JavaValue>,
}

/// Converts wrapper call arguments into JNI argument values.
///
/// Unlike [`IntoJavaArgs`], this conversion sees the selected overload's parameter types and the
/// current JNI environment. This lets wrapper calls accept Rust strings for Java string-compatible
/// parameters while keeping temporary `jstring` references alive until the JNI call returns.
///
/// This trait is sealed. Use the supported argument shapes instead of implementing it yourself:
/// `()`, one supported argument, tuples, arrays, slices, vectors, [`JavaArgs`], and [`JavaValue`]
/// lists.
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

/// Converts a Java wrapper return into a Rust value.
pub trait FromJavaReturn: Sized {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self>;
}

/// Converts field assignment values into JNI field values.
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

/// Internal overload-selection argument storage.
pub(crate) enum JavaOverloadArg {
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
