use std::{
    collections::{HashMap, VecDeque},
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
    env::{AttachedEnv, Env, FieldId, FieldKind, MethodId, MethodKind},
    error::{Error, Result},
    jni,
    metadata::{
        self, JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryGroup,
    },
    refs::{
        ArrayKind, AsJClass, AsJObject, BorrowedLocalRef, ClassKind, ClassRef, GlobalRef,
        JavaObjectRef, LocalRef, ObjectKind,
    },
    replacement,
    runtime::{FeatureSupport, JavaCapabilities},
    signature::{JavaType, MethodSignature},
    value::{JavaValue, RawJavaObject},
    vm::Vm,
};

#[macro_use]
mod macros;

mod args;
mod array;
mod class;
mod dispatch;
mod display;
mod handle;
mod loader;
mod lookup;
mod main_thread;
mod object;
mod perform;
mod returns;
mod wrapper;

use self::{
    array::{array_from_ref, object_from_ref, object_ref_from_ref},
    class::JavaClassInner,
    dispatch::{
        RawObject, call_instance_return, call_static_return, get_instance_field, get_static_field,
        set_instance_field, set_static_field,
    },
    loader::app_class_loader_from_activity_thread,
    lookup::{find_class_with_loader, normalize_class_lookup_name},
    main_thread::MainThreadState,
    object::object_to_string,
    perform::{
        AppPerformState, PendingPerform, class_loader_from_get_class_loader, complete_perform,
        perform_callback_with_result,
    },
};

/// Low-level Java handles used by explicit JNI-style operations.
///
/// Most callers should use [`JavaClass`] from [`Java::use_class`] for reflection-backed member
/// lookup and typed wrapper calls. Values in this module are still safe crate-owned handles, but
/// expose a lower-level descriptor-and-`JavaValue` API.
pub mod raw {
    use super::*;

    /// An owned global reference to a Java class plus cached method and field IDs.
    ///
    /// The cached JNI IDs are tied to this class' defining identity. Instances from a different
    /// loader should be resolved through that loader's [`Java`] value instead of reusing this class
    /// handle. `name()` returns a Java binary name such as `java.lang.String`, matching the
    /// upstream `frida-java-bridge` user-facing class-name convention. Descriptors and [`JavaType`]
    /// values still use JNI slash-style names such as `Ljava/lang/String;`.
    #[derive(Clone)]
    pub struct Class {
        pub(crate) inner: Arc<JavaClassInner>,
    }
}

pub(crate) use self::display::display_java_char;
pub(crate) use self::raw::Class as RawJavaClass;
pub(crate) use self::{
    main_thread::main_thread_scheduling_support, perform::app_loader_deferral_support,
};

static APP_PERFORM_STATE: OnceLock<AppPerformState> = OnceLock::new();
static MAIN_THREAD_STATE: OnceLock<MainThreadState> = OnceLock::new();

/// A convenience handle for Java operations in one VM and one optional class-loader scope.
///
/// The JS-like path is:
///
/// ```ignore
/// let java = Java::obtain()?;
/// java.perform(|java| {
///     let activity = java.use_class("android.app.Activity")?;
///     Ok(())
/// })?;
/// ```
///
/// `perform()` waits for the app class loader when needed and runs the callback inside a
/// [`JavaScope`]. Most high-level app code should live there and use `use_class()`.
///
/// A plain `Java` value still performs bootstrap-style `find_class()` lookups for low-level
/// callers. `with_loader()` creates a new `Java` value that resolves names through the supplied
/// `ClassLoaderRef`. Class lookup caches are intentionally per-`Java` instance so bootstrap and
/// loader-backed lookups cannot share class identity by accident.
#[derive(Clone)]
pub struct Java {
    vm: Vm,
    loader: Option<ClassLoaderRef>,
    classes: Arc<Mutex<HashMap<String, RawJavaClass>>>,
}

/// A synchronous Java operation scope whose current thread is attached to the VM.
///
/// This is the handle passed to `Java::perform()` and `Java::perform_now()` callbacks. It keeps a
/// JNI attachment guard alive for the lexical callback scope and exposes the same loader scope as
/// the underlying [`Java`] handle. It dereferences to `Java`, so helpers that accept `&Java` can
/// also accept `&JavaScope`.
///
/// `Java::attach()` returns this guard directly when a caller wants to enter the scope manually.
/// `Java::perform_now()` is the closure-shaped spelling for entering the same kind of scope
/// immediately. `Java::perform()` also enters a scope, but may defer until the app class loader is
/// available first.
///
/// Use `env()` when code truly needs raw JNI access. Ordinary bridge code should prefer
/// `use_class()`, object wrappers, arrays, and other safe helpers on the scope itself.
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
    assert_impl_all!(JavaRef: Send, Sync);
    assert_impl_all!(JavaArray: Send, Sync);
    assert_impl_all!(RawJavaClass: Send, Sync);
    assert_impl_all!(ClassLoaderRef: Send, Sync);
    assert_not_impl_any!(JavaLocalObject<'static>: Send, Sync);
    assert_not_impl_any!(JavaLocalRef<'static>: Send, Sync);
    assert_not_impl_any!(JavaLocalArray<'static>: Send, Sync);
}

/// Describes how a `ClassLoaderRef` entered this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassLoaderKind {
    /// The process system class loader returned by `ClassLoader.getSystemClassLoader()`.
    System,
    /// The app class loader selected from `ActivityThread.currentApplication()`.
    App,
    /// A loader explicitly wrapped from a Java object.
    Object,
    /// A loader discovered by ART class-loader enumeration.
    Enumerated,
}

/// An owned global reference to a `java.lang.ClassLoader`.
///
/// Loader references are VM-scoped and may be cloned cheaply. They are validated as
/// `java.lang.ClassLoader` instances when constructed.
#[derive(Clone)]
pub struct ClassLoaderRef {
    vm: Vm,
    object: Arc<GlobalRef<ObjectKind>>,
    kind: ClassLoaderKind,
}

/// A GumJS-inspired class wrapper backed by the crate's explicit Rust-native API.
///
/// `JavaClass` exposes Frida-like method groups where overload selection is shared by calls and
/// replacement. Method replacement, automatic overload dispatch, and JavaScript object semantics
/// remain separate milestones.
#[derive(Clone)]
pub struct JavaClass {
    class: RawJavaClass,
    methods: Arc<Mutex<Option<Vec<JavaMethodMetadata>>>>,
    instance_methods: Arc<Mutex<Option<Vec<JavaMethodMetadata>>>>,
    fields: Arc<Mutex<Option<Vec<JavaFieldMetadata>>>>,
    instance_fields: Arc<Mutex<Option<Vec<JavaFieldMetadata>>>>,
}

/// A named Java method group containing the currently visible non-constructor overloads.
#[derive(Clone)]
pub struct JavaMethodGroup {
    class: RawJavaClass,
    name: String,
    overloads: Vec<JavaMethodMetadata>,
}

/// A selected constructor overload on a `JavaClass`.
#[derive(Clone)]
pub struct JavaConstructor {
    class: RawJavaClass,
    metadata: JavaMethodMetadata,
}

/// A selected method on a `JavaClass`.
#[derive(Clone)]
pub struct JavaMethod {
    class: RawJavaClass,
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
    class: RawJavaClass,
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

/// An unbound Java object reference over a specific JNI reference storage kind.
///
/// The default `JavaRef` spelling is an owned global JNI reference. Other storage kinds are used
/// for callback-local borrowed views while sharing the same low-level reference API.
pub struct JavaRef<R = GlobalRef<ObjectKind>> {
    vm: Vm,
    reference: R,
}

/// A Java object viewed through a selected class wrapper.
///
/// The default `JavaObject` spelling is an owned global JNI reference plus the wrapper class used
/// for high-level instance member lookup. Other storage kinds are used for callback-local borrowed
/// views while sharing the same wrapper-bound object API.
pub struct JavaObject<R = GlobalRef<ObjectKind>> {
    class: JavaClass,
    reference: JavaRef<R>,
}

/// A borrowed unbound Java object reference valid only for the callback or JNI frame that produced
/// it.
pub type JavaLocalRef<'local> = JavaRef<BorrowedLocalRef<'local, ObjectKind>>;

/// A borrowed Java object view valid only for the callback or JNI frame that produced it.
///
/// Local object views do not own the JNI reference and never delete it on drop. They are intended
/// for replacement callbacks where ART/JNI passes `this`, arguments, or original-return locals that
/// are valid only while the callback is executing. Call `retain()` to keep the object afterwards.
pub type JavaLocalObject<'local> = JavaObject<BorrowedLocalRef<'local, ObjectKind>>;

/// A Java array wrapper over a specific JNI reference storage kind.
///
/// The default `JavaArray` spelling is an owned global JNI reference. Array wrappers keep the JNI
/// reference plus the expected element type. Primitive arrays expose copy-in/copy-out helpers;
/// object arrays expose nullable element access.
pub struct JavaArray<R = GlobalRef<ArrayKind>> {
    vm: Vm,
    array: R,
    element_type: JavaType,
}

/// A borrowed Java array reference valid only for the callback or JNI frame that produced it.
///
/// Local array views mirror [`JavaArray`] copy-in/copy-out helpers while borrowing the JNI array
/// handle. They do not delete the JNI reference on drop; call `retain()` to keep the array beyond
/// the current callback.
pub type JavaLocalArray<'local> = JavaArray<BorrowedLocalRef<'local, ArrayKind>>;

/// Current state of a deferred app-loader operation registered through `Java::perform`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerformStatus {
    Pending,
    Completed,
    Failed(Error),
}

/// A handle to a `Java::perform` callback.
#[derive(Clone)]
pub struct PerformHandle {
    state: Arc<Mutex<PerformStatus>>,
}

/// A handle to a `Java::perform` callback and its eventual value.
#[derive(Clone)]
pub struct PerformResult<T> {
    handle: PerformHandle,
    value: Arc<Mutex<Option<Result<T>>>>,
}

/// Current state of a callback scheduled through `Java::schedule_on_main_thread`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MainThreadTaskStatus {
    Pending,
    Completed,
    Failed(Error),
}

/// A handle to a callback scheduled on Android's main thread.
#[derive(Clone)]
pub struct MainThreadTaskHandle {
    state: Arc<Mutex<MainThreadTaskStatus>>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JavaReturn<O = JavaObject, A = JavaArray> {
    Void,
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(Option<O>),
    Array(Option<A>),
}

/// A Java return value whose references borrow from a callback or JNI frame.
pub type JavaLocalReturn<'local> = JavaReturn<JavaLocalObject<'local>, JavaLocalArray<'local>>;

/// A Java return value carrying explicit raw JNI object references.
pub type JavaRawReturn = JavaReturn<RawJavaObject, RawJavaObject>;

/// Converts common Rust argument containers into explicit JNI argument values.
///
/// This keeps low-level JNI marshaling visible through `JavaValue`, while letting wrapper and
/// overload call sites pass tuples, arrays, slices, or vectors without hand-building temporary
/// slices every time.
pub trait IntoJavaArgs {
    fn into_java_args(self) -> Vec<JavaValue>;
}

/// Explicit JNI argument list for calls that need more room than tuple call syntax.
///
/// Most call sites can pass `()`, a single argument, a tuple, an array, or a slice directly. Use
/// [`java_args!`](crate::java_args) when a hook needs a long original-call list without capturing
/// raw JNI reference values in the replacement closure.
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
pub trait IntoJavaCallArgs {
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaArgValues>;
}

/// Extracts a typed Rust value from a wrapper method or field return.
pub trait FromJavaReturn: Sized {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self>;
}

/// Converts high-level field assignment values into JNI field values.
pub trait IntoJavaFieldValue {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue>;
}

#[doc(hidden)]
pub struct PreparedJavaArgValues {
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
}

#[doc(hidden)]
pub struct PreparedJavaFieldValue {
    value: JavaValue,
    local_ref: Option<jni::jobject>,
}

pub(crate) struct PreparedJavaArgs<'vm> {
    env: AttachedEnv<'vm>,
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
}
