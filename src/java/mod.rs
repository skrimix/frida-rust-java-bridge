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
    fmt,
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
};

#[cfg(test)]
use std::ptr;

use crate::{
    capabilities::{FeatureSupport, JavaCapabilities},
    env::{AttachedEnv, Env, FieldId, FieldKind, MethodId, MethodKind},
    error::{Error, Result},
    jni,
    metadata::{
        self, JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryClass,
        JavaMethodQueryGroup,
    },
    refs::{
        ArrayKind, AsJClass, AsJObject, BorrowedLocalRef, ClassKind, ClassRef, GlobalRef,
        JavaObjectRef, LocalRef, ObjectKind, StringKind,
    },
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

#[macro_use]
mod macros;

mod args;
mod array;
mod class;
mod dispatch;
mod handle;
mod loader;
mod lookup;
mod main_thread;
mod object;
mod perform;
pub mod raw;
pub mod replacement;
mod returns;
mod wrapper;

mod sealed {
    pub trait IntoJavaFieldValueSealed {}
}

use self::{
    array::{array_from_ref, object_from_ref},
    dispatch::{
        RawObject, call_instance_return, call_static_return, get_instance_field, get_static_field,
        set_instance_field, set_static_field,
    },
    loader::app_class_loader_from_activity_thread,
    lookup::{find_class_with_loader, normalize_class_lookup_name},
    main_thread::MainThreadState,
    object::runtime_class,
    perform::{
        AppPerformState, PendingPerform, class_loader_from_get_class_loader, complete_perform,
        perform_callback_with_result,
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

/// A high-level, reflection-backed wrapper for a Java class.
///
/// A `JavaClass` lets you perform class-level operations in Rust, such as:
/// - Creating new instances (constructors).
/// - Calling static methods.
/// - Reading and writing static fields.
/// - Casting or checking if objects are instances of this class.
/// - Finding existing instances of this class currently alive on the heap.
/// - Installing method or constructor replacements (hooks).
///
/// ### Overload Selection
///
/// When calling methods or constructors, this wrapper will automatically resolve and choose the best
/// overload based on the arguments you pass. If you need to target a highly specific overload or resolve
/// ambiguity, you can use the explicit `*_with` methods to select a signature manually.
#[derive(Clone)]
pub struct JavaClass {
    class: raw::Class,
    methods: Arc<Mutex<Option<Vec<JavaMethodMetadata>>>>,
    visible_methods: Arc<Mutex<Option<Vec<JavaMethodMetadata>>>>,
    fields: Arc<Mutex<Option<Vec<JavaFieldMetadata>>>>,
    visible_fields: Arc<Mutex<Option<Vec<JavaFieldMetadata>>>>,
}

/// A named Java method group containing the currently visible non-constructor overloads.
#[derive(Clone)]
pub struct JavaMethodGroup {
    class: raw::Class,
    name: String,
    overloads: Vec<JavaMethodMetadata>,
}

/// A selected constructor overload on a `JavaClass`.
#[derive(Clone)]
pub struct JavaConstructor {
    class: raw::Class,
    metadata: JavaMethodMetadata,
}

/// A selected method on a `JavaClass`.
#[derive(Clone)]
pub struct JavaMethod {
    class: raw::Class,
    metadata: JavaMethodMetadata,
}

/// A named Java method group bound to one borrowed Java receiver.
pub struct JavaBoundMethodGroup<'object> {
    object: &'object (dyn JavaObjectRef + 'object),
    group: JavaMethodGroup,
}

/// A selected field on a `JavaClass`.
#[derive(Clone)]
pub struct JavaField {
    class: raw::Class,
    metadata: JavaFieldMetadata,
}

/// A borrowed Java object bound to an explicit class wrapper for ergonomic instance calls.
///
/// This borrows the object reference and keeps the caller-selected class/loader context visible.
pub struct JavaBoundObject<'object> {
    class: JavaClass,
    object: &'object (dyn JavaObjectRef + 'object),
}

/// A selected method bound to one borrowed Java receiver.
pub struct JavaBoundMethodOverload<'object> {
    object: &'object (dyn JavaObjectRef + 'object),
    overload: JavaMethod,
}

/// A selected field bound to one borrowed Java receiver.
pub struct JavaBoundFieldHandle<'object> {
    object: &'object (dyn JavaObjectRef + 'object),
    field: JavaField,
}

/// Controls whether heap instance enumeration should keep delivering matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaChooseControl {
    Continue,
    Stop,
}

/// A safe wrapper representing a Java object instance.
///
/// By default, a `JavaObject` owns an underlying global JNI reference. This means the object is kept alive
/// in Java as long as the Rust wrapper is held, and it can be safely sent and moved across different Rust threads
/// (as long as those threads attach to the Java VM when performing operations).
///
/// It also stores a reference to its wrapper [`JavaClass`] to enable convenient instance method calls
/// and field access.
///
/// ### Callback-Local Views
///
/// In replacement hooks, Java objects are often passed as callback-local views (such as `JavaLocalObject`).
/// These views borrow the underlying JNI reference and are valid *only* for the duration of the callback.
/// If you need to keep a callback-local object alive after the hook returns, call `.retain()` to promote
/// it to an owned global `JavaObject`.
pub struct JavaObject<R = GlobalRef<ObjectKind>> {
    class: JavaClass,
    vm: Vm,
    reference: R,
}

/// A callback-local borrowed view of a Java object.
///
/// Unlike a standard [`JavaObject`], a local object view only borrows the underlying JNI reference and
/// does not clean it up on drop. These are typically passed as `this` or as argument values inside replacement
/// hook callbacks, and are valid only while that callback is running. Call `.retain()` to promote this local
/// view into an owned global reference.
pub type JavaLocalObject<'local> = JavaObject<BorrowedLocalRef<'local, ObjectKind>>;

/// A safe wrapper representing a Java array.
///
/// Like [`JavaObject`], the default `JavaArray` owns a global JNI reference, keeping the array alive in Java
/// and letting you send it across threads.
///
/// ### Working with Arrays
///
/// - **Primitive Arrays:** Provides efficient copy-in and copy-out helpers (e.g., to convert between Rust slices and Java arrays).
/// - **Object Arrays:** Allows reading and writing individual elements, supporting nullable object references.
///
/// Callback-local array views (like `JavaLocalArray` in hooks) only borrow the array for the duration of the callback.
/// Use `.retain()` if the array needs to outlive the callback scope.
pub struct JavaArray<R = GlobalRef<ArrayKind>> {
    object: JavaObject<R>,
    element_type: JavaType,
}

/// A callback-local borrowed view of a Java array.
///
/// Local array views mirror all standard [`JavaArray`] operations but borrow the underlying JNI reference.
/// They are valid only for the duration of the replacement callback where they were provided.
/// Call `.retain()` to promote this local view into an owned global array.
pub type JavaLocalArray<'local> = JavaArray<BorrowedLocalRef<'local, ArrayKind>>;

/// Reference payload used by normal high-level Java returns.
pub enum JavaReturnRef {
    Object(JavaObject),
    Array(JavaArray),
}

/// A normal high-level Java return value.
pub type JavaReturn = JavaValue<JavaReturnRef>;

/// Reference payload used by Java returns that borrow from a callback or JNI frame.
pub enum JavaLocalReturnRef<'local> {
    Object(JavaLocalObject<'local>),
    Array(JavaLocalArray<'local>),
}

/// A Java return value whose references borrow from a callback or JNI frame.
pub type JavaLocalReturn<'local> = JavaValue<JavaLocalReturnRef<'local>>;

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
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>>;
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
pub struct PreparedJavaCallArgs<'env, 'vm> {
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
    cleanup_env: &'env Env<'vm>,
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

pub(crate) struct AttachedJavaCallArgs<'vm> {
    env: AttachedEnv<'vm>,
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
}
