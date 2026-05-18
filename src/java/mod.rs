use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    fmt, ptr,
    ptr::NonNull,
    rc::Rc,
    sync::{Arc, Mutex, OnceLock},
};

use crate::{
    env::{AttachedEnv, Env, FieldId, FieldKind, MethodId, MethodKind},
    error::{Error, Result},
    jni,
    metadata::{
        self, JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryGroup,
    },
    refs::{ArrayKind, AsJClass, AsJObject, ClassKind, ClassRef, GlobalRef, LocalRef, ObjectKind},
    replacement,
    runtime::{FeatureSupport, JavaCapabilities},
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
mod returns;
mod wrapper;

use self::{
    array::{array_from_ref, object_from_ref},
    dispatch::{
        call_instance_return, call_static_return, get_instance_field, get_static_field,
        set_instance_field, set_static_field,
    },
    loader::{
        app_class_loader_from_activity_thread, app_perform_state, default_app_java,
        default_app_loader,
    },
    lookup::{find_class_with_loader, normalize_class_lookup_name},
    perform::{class_loader_from_get_class_loader, complete_perform},
};

pub(crate) use self::{
    main_thread::main_thread_scheduling_support, perform::app_loader_deferral_support,
};

static APP_PERFORM_STATE: OnceLock<AppPerformState> = OnceLock::new();
static MAIN_THREAD_STATE: OnceLock<MainThreadState> = OnceLock::new();

/// A convenience handle for Java operations in one VM and one optional class-loader scope.
///
/// A plain `Java` value performs bootstrap-style `FindClass` lookups. `with_loader()` creates a
/// new `Java` value that resolves names through the supplied `ClassLoaderRef`. Class lookup caches
/// are intentionally per-`Java` instance so bootstrap and loader-backed lookups cannot share class
/// identity by accident.
#[derive(Clone)]
pub struct Java {
    vm: Vm,
    loader: Option<ClassLoaderRef>,
    classes: Arc<Mutex<HashMap<String, JavaClass>>>,
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

/// An owned global reference to a Java class plus cached method and field IDs.
///
/// The cached JNI IDs are tied to this class' defining identity. Instances from a different loader
/// should be resolved through that loader's `Java` value instead of reusing this class wrapper.
/// `name()` returns a Java binary name such as `java.lang.String`, matching the upstream
/// `frida-java-bridge` user-facing class-name convention. Descriptors and `JavaType` values still
/// use JNI slash-style names such as `Ljava/lang/String;`.
#[derive(Clone)]
pub struct JavaClass {
    inner: Arc<JavaClassInner>,
}

/// A GumJS-inspired class wrapper backed by the crate's explicit Rust-native API.
///
/// `JavaClassWrapper` is intentionally not a drop-in clone of JavaScript `Java.use()`. It provides
/// a permanent wrapper surface for class/member metadata and explicit overload calls, while method
/// replacement, automatic overload dispatch, and JavaScript object semantics remain separate
/// milestones.
#[derive(Clone)]
pub struct JavaClassWrapper {
    class: JavaClass,
    methods: Rc<RefCell<Option<Vec<JavaMethodMetadata>>>>,
    fields: Rc<RefCell<Option<Vec<JavaFieldMetadata>>>>,
}

/// A selected constructor overload on a `JavaClassWrapper`.
#[derive(Clone)]
pub struct JavaConstructorOverload {
    class: JavaClass,
    metadata: JavaMethodMetadata,
}

/// A selected method overload on a `JavaClassWrapper`.
#[derive(Clone)]
pub struct JavaMethodOverload {
    class: JavaClass,
    metadata: JavaMethodMetadata,
}

/// A selected field on a `JavaClassWrapper`.
#[derive(Clone)]
pub struct JavaFieldHandle {
    class: JavaClass,
    metadata: JavaFieldMetadata,
}

/// Controls whether heap instance enumeration should keep delivering matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaChooseControl {
    Continue,
    Stop,
}

/// An owned global reference to a Java object.
///
/// Object wrappers retain the VM and JNI reference ownership only. They do not currently record the
/// defining class loader of the object's class; callers should keep using the relevant `JavaClass`
/// or loader-backed `Java` value for follow-up class and member lookup.
pub struct JavaObject {
    vm: Vm,
    object: GlobalRef<ObjectKind>,
}

/// An owned global reference to a Java array.
///
/// Array wrappers keep the JNI reference plus the expected element type. Primitive arrays expose
/// copy-in/copy-out helpers; object arrays expose nullable element access.
pub struct JavaArray {
    vm: Vm,
    array: GlobalRef<ArrayKind>,
    element_type: JavaType,
}

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

#[derive(Debug)]
pub enum JavaReturn {
    Void,
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(Option<JavaObject>),
    Array(Option<JavaArray>),
}

/// Converts common Rust argument containers into explicit JNI argument values.
///
/// This keeps low-level JNI marshaling visible through `JavaValue`, while letting wrapper and
/// overload call sites pass tuples, arrays, slices, or vectors without hand-building temporary
/// slices every time.
pub trait IntoJavaArgs {
    fn into_java_args(self) -> Vec<JavaValue>;
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

pub struct PreparedJavaArgValues {
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
}

pub struct PreparedJavaArgs<'vm> {
    env: AttachedEnv<'vm>,
    values: Vec<JavaValue>,
    local_refs: Vec<jni::jobject>,
}

struct JavaClassInner {
    vm: Vm,
    name: String,
    class: GlobalRef<ClassKind>,
    methods: Mutex<HashMap<MethodKey, MethodId>>,
    fields: Mutex<HashMap<FieldKey, FieldId>>,
}

type PerformCallback = Box<dyn FnOnce(Java) -> Result<()> + Send + 'static>;
type MainThreadCallback = Box<dyn FnOnce(Java) -> Result<()> + Send + 'static>;

struct PendingPerform {
    callback: PerformCallback,
    state: Arc<Mutex<PerformStatus>>,
}

struct AppPerformState {
    vm: Vm,
    inner: Mutex<AppPerformInner>,
}

struct AppPerformInner {
    default: Option<DefaultAppLoader>,
    pending: VecDeque<PendingPerform>,
    hooks: Option<AppPerformHooks>,
}

#[derive(Clone)]
struct DefaultAppLoader {
    loader: ClassLoaderRef,
    classes: Arc<Mutex<HashMap<String, JavaClass>>>,
}

struct AppPerformHooks {
    _make_application: Option<replacement::MethodReplacement>,
    _get_package_info: Option<replacement::MethodReplacement>,
}

struct PendingMainThreadTask {
    java: Java,
    callback: MainThreadCallback,
    state: Arc<Mutex<MainThreadTaskStatus>>,
}

struct MainThreadState {
    vm: Vm,
    main_thread_id: u32,
    inner: Mutex<MainThreadInner>,
}

struct MainThreadInner {
    pending: VecDeque<PendingMainThreadTask>,
    hooks: Option<MainThreadHooks>,
}

struct MainThreadHooks {
    _interceptor: frida_gum::interceptor::Interceptor,
    _listener_handle: frida_gum::interceptor::Listener,
    _listener: Box<MainThreadPollListener>,
}

struct MainThreadPollListener;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MethodKey {
    kind: MethodKind,
    name: String,
    signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FieldKey {
    kind: FieldKind,
    name: String,
    ty: String,
}

struct RawObject(jni::jobject);
