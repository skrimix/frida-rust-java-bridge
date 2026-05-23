use std::{fmt, ptr};

use crate::{
    Error, Result,
    env::{Env, MethodKind},
    java::{
        IntoJavaArgs, JavaArray, JavaConstructor, JavaLocalArray, JavaLocalObject, JavaMethod,
        JavaObject, JavaRawReturn, RawJavaClass, display_java_char,
    },
    jni, metadata,
    refs::{AsJClass, JavaObjectRef},
    signature::{JavaType, MethodSignature},
    value::{JavaValue, RawJavaObject},
};

use super::{
    closure::{
        ClosureMethodReplacement, ReplacementInvocation, replace_closure_method,
        replace_constructor_closure, validate_closure_replacement_signature,
    },
    original::RawJavaReturn,
};

const METHOD_HOOK_OPERATION: &str = "JavaMethod::replace";
const CONSTRUCTOR_HOOK_OPERATION: &str = "JavaConstructor::replace";

pub struct JavaHookGuard {
    inner: ClosureMethodReplacement,
}

/// Error or panic reported by an installed Java replacement callback.
///
/// Replacement callbacks run later, when Java calls the hooked method. Callback failures still
/// cause Java callers to receive the JNI default value for the method return type, except for Java
/// exceptions from original-call helpers or Java wrapper calls, which are rethrown to the Java
/// caller when the callback returns the Java-backed error.
/// Callers can attach an [`JavaHookGuard::on_error`] reporter to observe failures as they happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaHookError {
    kind: MethodKind,
    name: String,
    signature: MethodSignature,
    message: String,
}

/// Invocation details passed to installed Rust method hooks.
///
/// This is a thin ergonomic wrapper over the raw closure-backed replacement callback. It is only
/// valid while the current thread is executing the replacement callback. The full argument list is
/// intentionally exposed for exploratory hooks; typed helpers are conveniences on top.
pub struct JavaHookContext<'state> {
    pub(crate) inner: ReplacementInvocation<'state>,
}

/// Untyped callback-argument inspection view.
///
/// Prefer [`JavaHookContext::arg`], [`JavaHookContext::arg_object`],
/// and [`JavaHookContext::arg_array`] in hooks that know the expected argument shape.
pub struct JavaHookArguments<'context, 'state> {
    context: &'context JavaHookContext<'state>,
}

/// Iterator over untyped callback-argument inspection values.
pub struct JavaHookArgumentsIter<'context, 'state> {
    context: &'context JavaHookContext<'state>,
    index: usize,
}

/// One safely inspectable replacement argument.
#[derive(Debug)]
pub enum JavaHookArgument<'state> {
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(Option<JavaLocalObject<'state>>),
    Array(Option<JavaLocalArray<'state>>),
}

/// Return value accepted by installed Rust method hooks.
///
/// This is the raw-reference specialization of [`crate::JavaReturn`]. Object and array helpers
/// borrow crate-owned JNI-backed wrappers. Explicit raw returns are only available through unsafe
/// constructors on this alias.
pub type JavaHookReturn = JavaRawReturn;

mod sealed {
    pub trait FromJavaValueSealed {}
}

/// Converts Rust values into hook return values.
///
/// This keeps the backend's explicit [`JavaHookReturn`] shape available while allowing
/// callbacks to return ordinary Rust primitives and borrowed Java objects for supported lanes.
/// Numeric values are adapted to the selected Java method's return descriptor at the hook
/// boundary, so Rust's default literal types do not accidentally select the wrong JNI return lane.
pub trait IntoJavaHookReturn {
    fn into_hook_return(self) -> JavaHookReturn;

    #[doc(hidden)]
    fn into_hook_return_for(
        self,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn>
    where
        Self: Sized,
    {
        self.into_hook_return()
            .coerce_for_return_type(return_type, operation)
    }
}

/// Converts one raw replacement argument into a typed Rust value.
///
/// This is intentionally limited to values that can be extracted without taking ownership of JNI
/// references. Object-like arguments are exposed as raw nullable JNI references for now.
pub trait FromJavaValue: sealed::FromJavaValueSealed + Sized {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self>;
}

/// Converts one replacement argument into a typed Rust value with access to the hook context.
///
/// This powers [`JavaHookContext::arg`]. Primitive conversions are provided through
/// [`FromJavaValue`], while context-aware conversions such as `String` can read JNI-backed
/// references safely during the callback.
pub trait FromJavaHookArgument<'state>: Sized {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self>;
}

/// Extracts a typed value from an [`JavaHookReturn`].
///
/// This is primarily useful with [`JavaHookContext::call_original`].
pub trait FromJavaHookReturn<'state>: Sized {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self>;
}

pub trait JavaHookTarget {
    /// Replaces this hook target with a guarded Rust closure.
    fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn;
}

#[derive(Default)]
pub struct JavaHookSet {
    guards: Vec<JavaHookGuard>,
}

impl JavaHookGuard {
    /// Restores the original method now.
    ///
    /// This is safe to call more than once; after a successful restore, later calls are no-ops. If
    /// restore reports an error, the replacement stays active. Dropping a guard that has not been
    /// successfully restored also attempts a restore.
    pub fn revert(&mut self) -> Result<()> {
        self.inner.revert()
    }

    /// Returns a backend debug summary for diagnostics when the internal ART backend provides one.
    pub fn debug_summary(&self) -> Option<String> {
        self.inner.debug_summary()
    }

    /// Installs an error reporter and returns this guard for call-site chaining.
    ///
    /// The reporter is called on the Java thread that encountered the callback failure, after the
    /// same error has been recorded for [`JavaHookGuard::last_error`].
    pub fn on_error<F>(self, handler: F) -> Self
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        self.set_error_handler(handler);
        self
    }

    /// Installs or replaces the error reporter for this guard.
    ///
    /// Use this when a guard has already been stored somewhere. For builder-style installation at
    /// the replacement call site, prefer [`JavaHookGuard::on_error`].
    pub fn set_error_handler<F>(&self, handler: F)
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        self.inner.set_error_handler(handler);
    }

    /// Clears the installed error reporter, if any.
    pub fn clear_error_handler(&self) {
        self.inner.clear_error_handler();
    }

    /// Returns the most recent callback error or panic recorded by the replacement.
    ///
    /// Callback failures cause Java callers to receive the JNI default value for the method's
    /// return type unless the callback failure preserved a Java exception for rethrow, and the
    /// error is kept here for explicit inspection.
    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error()
    }

    /// Returns and clears the most recent callback error or panic recorded by the replacement.
    pub fn take_last_error(&self) -> Option<String> {
        self.inner.take_last_error()
    }
}

impl JavaHookError {
    pub(crate) fn new(
        kind: MethodKind,
        name: String,
        signature: MethodSignature,
        message: String,
    ) -> Self {
        Self {
            kind,
            name,
            signature,
            message,
        }
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.signature
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for JavaHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}{}: {}",
            hook_kind_name(self.kind),
            self.name,
            self.signature,
            self.message
        )
    }
}

impl JavaHookSet {
    pub fn new() -> Self {
        Self { guards: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.guards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.guards.is_empty()
    }

    pub fn push(&mut self, guard: JavaHookGuard) {
        self.guards.push(guard);
    }

    /// Replaces `target` and stores the returned guard in this set.
    pub fn replace<T, F, R>(&mut self, target: T, callback: F) -> Result<&mut JavaHookGuard>
    where
        T: JavaHookTarget,
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        let guard = target.replace(callback)?;
        self.guards.push(guard);
        Ok(self
            .guards
            .last_mut()
            .expect("guard was just pushed into JavaHookSet"))
    }

    pub fn revert_all(&mut self) -> Result<()> {
        for guard in self.guards.iter_mut().rev() {
            guard.revert()?;
        }
        Ok(())
    }

    pub fn last_errors(&self) -> Vec<String> {
        self.guards
            .iter()
            .filter_map(JavaHookGuard::last_error)
            .collect()
    }
}

impl JavaHookTarget for JavaMethod {
    fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        JavaMethod::replace(self, callback)
    }
}

impl JavaHookTarget for &JavaMethod {
    fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        JavaMethod::replace(self, callback)
    }
}

pub trait UnsafeJavaHookTarget {
    /// Replaces this constructor-like hook target with a guarded Rust closure.
    ///
    /// # Safety
    ///
    /// Constructor callbacks must initialize the receiver consistently enough for Java code that
    /// observes the object, and must return void.
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn;
}

impl UnsafeJavaHookTarget for JavaConstructor {
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        unsafe { JavaConstructor::replace(self, callback) }
    }
}

impl UnsafeJavaHookTarget for &JavaConstructor {
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        unsafe { JavaConstructor::replace(self, callback) }
    }
}

impl<'state> JavaHookContext<'state> {
    /// Returns the raw callback `JNIEnv`.
    ///
    /// # Safety
    ///
    /// The returned pointer is valid only while this replacement callback is executing.
    pub unsafe fn raw_env(&self) -> *mut jni::JNIEnv {
        self.inner.env_raw()
    }

    pub fn env(&self) -> Result<Env<'state>> {
        self.inner.env()
    }

    pub fn kind(&self) -> MethodKind {
        self.inner.kind()
    }

    pub fn name(&self) -> &str {
        self.inner.name()
    }

    pub fn signature(&self) -> &MethodSignature {
        self.inner.signature()
    }

    /// Returns the raw class argument for a static-method hook.
    ///
    /// # Safety
    ///
    /// The returned local reference is valid only while this replacement callback is executing.
    pub unsafe fn raw_class(&self) -> Option<jni::jclass> {
        self.inner.class()
    }

    /// Returns the raw receiver argument for an instance-method or constructor hook.
    ///
    /// # Safety
    ///
    /// The returned local reference is valid only while this replacement callback is executing.
    pub unsafe fn raw_receiver(&self) -> Option<jni::jobject> {
        self.inner.receiver()
    }

    /// Returns the current Java `this` object for an instance-method or constructor hook.
    ///
    /// Static method hooks do not have a `this` object; use
    /// [`JavaHookContext::maybe_this_object`] when handling static and instance hooks through the
    /// same callback path.
    pub fn this_object(&self) -> Result<JavaLocalObject<'state>> {
        self.receiver_object("JavaHookContext::this_object")?
            .ok_or(Error::WrongMethodKind {
                operation: "JavaHookContext::this_object",
            })
    }

    /// Returns the current Java `this` object when this hook has one.
    ///
    /// This returns `Ok(None)` for static-method hooks.
    pub fn maybe_this_object(&self) -> Result<Option<JavaLocalObject<'state>>> {
        self.receiver_object("JavaHookContext::maybe_this_object")
    }

    fn receiver_object(&self, operation: &'static str) -> Result<Option<JavaLocalObject<'state>>> {
        self.inner
            .receiver()
            .map(|receiver| self.local_object(receiver, operation))
            .transpose()
    }

    /// Returns an untyped inspection view over the callback arguments.
    ///
    /// Typed hooks should usually prefer [`JavaHookContext::arg`],
    /// [`JavaHookContext::arg_object`], or [`JavaHookContext::arg_array`].
    pub fn args(&self) -> JavaHookArguments<'_, 'state> {
        JavaHookArguments { context: self }
    }

    /// Returns one untyped inspection value.
    ///
    /// Prefer typed helpers when the callback knows the expected argument type.
    pub fn arg_value(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        self.hook_argument(index)
    }

    /// Returns one argument formatted for diagnostic logging.
    ///
    /// Primitive values are formatted directly, null reference lanes are rendered as `null`,
    /// `java.lang.String` values are extracted as Rust strings, and other references use Java's
    /// `toString()` implementation.
    ///
    /// This is a convenience shorthand for `invocation.arg_value(index)?.java_display()`.
    pub fn arg_display(&self, index: usize) -> Result<String> {
        self.arg_value(index)?.java_display()
    }

    /// Returns the raw callback arguments.
    ///
    /// # Safety
    ///
    /// Object references in the returned values are valid only while this replacement callback is
    /// executing. Use [`JavaHookContext::args`] for safe iterable argument views.
    pub unsafe fn raw_arguments(&self) -> &[JavaValue] {
        self.inner.arguments()
    }

    /// Returns a raw object-like argument.
    ///
    /// # Safety
    ///
    /// The returned raw reference is valid only while this replacement callback is executing.
    pub unsafe fn raw_arg_object(&self, index: usize) -> Result<Option<RawJavaObject>> {
        match self.argument_value(index)? {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => Ok(Some(value)),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }

    /// Extracts one argument through a typed conversion.
    pub fn arg<T: FromJavaHookArgument<'state>>(&self, index: usize) -> Result<T> {
        let value = self
            .inner
            .arguments()
            .get(index)
            .copied()
            .ok_or(Error::InvalidArguments {
                expected: index + 1,
                actual: self.inner.arguments().len(),
            })?;
        T::from_hook_argument(self, value, index)
    }

    /// Returns one object-like argument as a callback-local object view.
    pub fn arg_object(&self, index: usize) -> Result<Option<JavaLocalObject<'state>>> {
        match self.argument_value(index)? {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => self
                .local_object(value.as_jobject(), "JavaHookContext::arg_object")
                .map(Some),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }

    /// Returns one array argument as a callback-local array view.
    pub fn arg_array(&self, index: usize) -> Result<Option<JavaLocalArray<'state>>> {
        let element_type = match self.signature().arguments().get(index) {
            Some(JavaType::Array(element)) => (**element).clone(),
            Some(_actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "array".to_owned(),
                    actual: "non-array",
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: self.inner.arguments().len(),
                });
            }
        };

        match self.argument_value(index)? {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => self
                .local_array(
                    value.as_jobject(),
                    element_type,
                    "JavaHookContext::arg_array",
                )
                .map(Some),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "array", other)),
        }
    }

    /// Calls the replaced method's original implementation and returns the raw hook return lane.
    ///
    /// # Safety
    ///
    /// Object references in the returned value are valid only while this replacement callback is
    /// executing. Prefer the typed original-call helpers for safe object and array views.
    pub unsafe fn call_original_raw<A: IntoJavaArgs>(&self, args: A) -> Result<JavaHookReturn> {
        let original = unsafe { self.inner.call_original(args)? };
        Ok(JavaHookReturn::from_raw_for_type(
            original,
            self.signature().return_type(),
        ))
    }

    /// Calls the replaced method's original implementation with the callback's current arguments.
    pub fn call_original_current(&self) -> Result<JavaHookReturn> {
        unsafe { self.call_original_raw(self.inner.arguments()) }
    }

    pub fn call_original_return<A: IntoJavaArgs>(&self, args: A) -> Result<JavaHookReturn> {
        unsafe { self.call_original_raw(args) }
    }

    pub fn call_original<T>(&self, args: impl IntoJavaArgs) -> Result<T>
    where
        T: FromJavaHookReturn<'state>,
    {
        T::from_hook_return(
            unsafe { self.call_original_raw(args)? },
            self,
            "JavaHookContext::call_original",
        )
    }

    pub fn call_original_void<A: IntoJavaArgs>(&self, args: A) -> Result<()> {
        unsafe { self.call_original_raw(args)? }.into_void("JavaHookContext::call_original_void")
    }

    pub fn call_original_object<A: IntoJavaArgs>(
        &self,
        args: A,
    ) -> Result<Option<JavaLocalObject<'state>>> {
        match unsafe { self.call_original_raw(args)? } {
            JavaHookReturn::Object(value) => value
                .map(|object| {
                    self.local_object(object.as_jobject(), "JavaHookContext::call_original_object")
                })
                .transpose(),
            other => Err(invalid_hook_return(
                "JavaHookContext::call_original_object",
                "object",
                other,
            )),
        }
    }

    pub fn call_original_array<A: IntoJavaArgs>(
        &self,
        args: A,
    ) -> Result<Option<JavaLocalArray<'state>>> {
        let element_type = match self.signature().return_type() {
            JavaType::Array(element) => (**element).clone(),
            actual => {
                return Err(Error::InvalidReturnType {
                    operation: "JavaHookContext::call_original_array",
                    expected: "array",
                    actual: actual.to_string(),
                });
            }
        };

        match unsafe { self.call_original_raw(args)? } {
            JavaHookReturn::Array(value) => value
                .map(|array| {
                    self.local_array(
                        array.as_jobject(),
                        element_type,
                        "JavaHookContext::call_original_array",
                    )
                })
                .transpose(),
            other => Err(invalid_hook_return(
                "JavaHookContext::call_original_array",
                "array",
                other,
            )),
        }
    }

    fn argument_value(&self, index: usize) -> Result<JavaValue> {
        self.inner
            .arguments()
            .get(index)
            .copied()
            .ok_or(Error::InvalidArguments {
                expected: index + 1,
                actual: self.inner.arguments().len(),
            })
    }

    fn hook_argument(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        let value = self.argument_value(index)?;
        Ok(match value {
            JavaValue::Boolean(value) => JavaHookArgument::Boolean(value),
            JavaValue::Byte(value) => JavaHookArgument::Byte(value),
            JavaValue::Char(value) => JavaHookArgument::Char(value),
            JavaValue::Short(value) => JavaHookArgument::Short(value),
            JavaValue::Int(value) => JavaHookArgument::Int(value),
            JavaValue::Long(value) => JavaHookArgument::Long(value),
            JavaValue::Float(value) => JavaHookArgument::Float(value),
            JavaValue::Double(value) => JavaHookArgument::Double(value),
            JavaValue::Null => self.null_reference_argument(index)?,
            JavaValue::Object(value) if value.is_null() => self.null_reference_argument(index)?,
            JavaValue::Object(value) => match self.signature().arguments().get(index) {
                Some(JavaType::Array(element)) => JavaHookArgument::Array(Some(self.local_array(
                    value.as_jobject(),
                    (**element).clone(),
                    "JavaHookContext::arg_value",
                )?)),
                Some(JavaType::Object(_)) => JavaHookArgument::Object(Some(
                    self.local_object(value.as_jobject(), "JavaHookContext::arg_value")?,
                )),
                Some(other) => {
                    return Err(Error::InvalidArgumentType {
                        index,
                        expected: other.to_string(),
                        actual: "object",
                    });
                }
                None => {
                    return Err(Error::InvalidArguments {
                        expected: index + 1,
                        actual: self.inner.arguments().len(),
                    });
                }
            },
        })
    }

    fn null_reference_argument(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        match self.signature().arguments().get(index) {
            Some(JavaType::Array(_)) => Ok(JavaHookArgument::Array(None)),
            Some(JavaType::Object(_)) => Ok(JavaHookArgument::Object(None)),
            Some(other) => Err(Error::InvalidArgumentType {
                index,
                expected: other.to_string(),
                actual: "null",
            }),
            None => Err(Error::InvalidArguments {
                expected: index + 1,
                actual: self.inner.arguments().len(),
            }),
        }
    }

    fn local_object(
        &self,
        value: jni::jobject,
        operation: &'static str,
    ) -> Result<JavaLocalObject<'state>> {
        if value.is_null() {
            return Err(Error::NullReturn { operation });
        }
        let env = self.env()?;
        unsafe { JavaLocalObject::from_raw(env.vm().clone(), value) }
    }

    fn local_array(
        &self,
        value: jni::jobject,
        element_type: JavaType,
        operation: &'static str,
    ) -> Result<JavaLocalArray<'state>> {
        if value.is_null() {
            return Err(Error::NullReturn { operation });
        }
        let env = self.env()?;
        unsafe { JavaLocalArray::from_raw(env.vm().clone(), value, element_type) }
    }
}

impl<'context, 'state> JavaHookArguments<'context, 'state> {
    pub fn len(&self) -> usize {
        self.context.inner.arguments().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        self.context.arg_value(index)
    }

    pub fn iter(&self) -> JavaHookArgumentsIter<'context, 'state> {
        JavaHookArgumentsIter {
            context: self.context,
            index: 0,
        }
    }
}

impl<'context, 'state> IntoIterator for JavaHookArguments<'context, 'state> {
    type Item = Result<JavaHookArgument<'state>>;
    type IntoIter = JavaHookArgumentsIter<'context, 'state>;

    fn into_iter(self) -> Self::IntoIter {
        JavaHookArgumentsIter {
            context: self.context,
            index: 0,
        }
    }
}

impl<'context, 'state> Iterator for JavaHookArgumentsIter<'context, 'state> {
    type Item = Result<JavaHookArgument<'state>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.context.inner.arguments().len() {
            return None;
        }
        let index = self.index;
        self.index += 1;
        Some(self.context.arg_value(index))
    }
}

impl JavaHookArgument<'_> {
    pub fn java_display(&self) -> Result<String> {
        Ok(match self {
            Self::Boolean(value) => value.to_string(),
            Self::Byte(value) => value.to_string(),
            Self::Char(value) => display_java_char(*value),
            Self::Short(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Long(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Double(value) => value.to_string(),
            Self::Object(Some(value)) => value.java_display()?,
            Self::Object(None) | Self::Array(None) => "null".to_owned(),
            Self::Array(Some(value)) => value.java_display()?,
        })
    }
}

impl JavaHookReturn {
    /// Extracts a raw JNI object reference from an object return.
    ///
    /// # Safety
    ///
    /// The returned reference has the lifetime and VM identity of the hook/original-call context
    /// that produced it. The caller must only use it while that context remains valid.
    pub unsafe fn into_raw_object(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Object(value) => Ok(value.map_or(ptr::null_mut(), RawJavaObject::as_jobject)),
            other => Err(invalid_hook_return(operation, "object", other)),
        }
    }

    /// Extracts a raw JNI array/object reference from an array return.
    ///
    /// # Safety
    ///
    /// The returned reference has the lifetime and VM identity of the hook/original-call context
    /// that produced it. The caller must only use it while that context remains valid.
    pub unsafe fn into_raw_array(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Array(value) => Ok(value.map_or(ptr::null_mut(), RawJavaObject::as_jobject)),
            other => Err(invalid_hook_return(operation, "array", other)),
        }
    }

    pub fn object<T: JavaObjectRef + ?Sized>(value: Option<&T>) -> Self {
        Self::Object(value.map(|value| {
            RawJavaObject::from_raw(crate::refs::sealed::JavaObjectRefSealed::as_jobject(value))
        }))
    }

    pub fn array<T: JavaObjectRef + ?Sized>(value: Option<&T>) -> Self {
        Self::Array(value.map(|value| {
            RawJavaObject::from_raw(crate::refs::sealed::JavaObjectRefSealed::as_jobject(value))
        }))
    }

    pub fn null_object() -> Self {
        Self::Object(None)
    }

    pub fn null_array() -> Self {
        Self::Array(None)
    }

    /// Builds an object return from a raw JNI reference.
    ///
    /// # Safety
    ///
    /// `value` must be null or a valid local/global reference for this VM and must remain valid
    /// until the replacement callback returns to ART.
    pub unsafe fn raw_object(value: jni::jobject) -> Self {
        if value.is_null() {
            Self::null_object()
        } else {
            Self::Object(Some(RawJavaObject::from_raw(value)))
        }
    }

    /// Builds an array return from a raw JNI reference.
    ///
    /// # Safety
    ///
    /// `value` must be null or a valid array local/global reference for this VM and must remain
    /// valid until the replacement callback returns to ART.
    pub unsafe fn raw_array(value: jni::jobject) -> Self {
        if value.is_null() {
            Self::null_array()
        } else {
            Self::Array(Some(RawJavaObject::from_raw(value)))
        }
    }

    fn from_raw(value: RawJavaReturn) -> Self {
        match value {
            RawJavaReturn::Void => Self::Void,
            RawJavaReturn::Boolean(value) => Self::Boolean(value != jni::JNI_FALSE),
            RawJavaReturn::Byte(value) => Self::Byte(value),
            RawJavaReturn::Char(value) => Self::Char(value),
            RawJavaReturn::Short(value) => Self::Short(value),
            RawJavaReturn::Int(value) => Self::Int(value),
            RawJavaReturn::Long(value) => Self::Long(value),
            RawJavaReturn::Float(value) => Self::Float(value),
            RawJavaReturn::Double(value) => Self::Double(value),
            RawJavaReturn::Object(value) => {
                if value.is_null() {
                    Self::Object(None)
                } else {
                    Self::Object(Some(RawJavaObject::from_raw(value)))
                }
            }
        }
    }

    pub(crate) fn from_raw_for_type(value: RawJavaReturn, return_type: &JavaType) -> Self {
        match (value, return_type) {
            (RawJavaReturn::Object(value), JavaType::Array(_)) => {
                if value.is_null() {
                    Self::null_array()
                } else {
                    Self::Array(Some(RawJavaObject::from_raw(value)))
                }
            }
            (value, _) => Self::from_raw(value),
        }
    }

    pub(crate) fn into_raw(self) -> RawJavaReturn {
        match self {
            Self::Void => RawJavaReturn::Void,
            Self::Boolean(value) => {
                RawJavaReturn::Boolean(if value { jni::JNI_TRUE } else { jni::JNI_FALSE })
            }
            Self::Byte(value) => RawJavaReturn::Byte(value),
            Self::Char(value) => RawJavaReturn::Char(value),
            Self::Short(value) => RawJavaReturn::Short(value),
            Self::Int(value) => RawJavaReturn::Int(value),
            Self::Long(value) => RawJavaReturn::Long(value),
            Self::Float(value) => RawJavaReturn::Float(value),
            Self::Double(value) => RawJavaReturn::Double(value),
            Self::Object(value) | Self::Array(value) => {
                RawJavaReturn::Object(value.map_or(ptr::null_mut(), RawJavaObject::as_jobject))
            }
        }
    }

    fn coerce_for_return_type(
        self,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<Self> {
        let value = match (return_type, self) {
            (JavaType::Void, Self::Void) => Self::Void,
            (JavaType::Boolean, Self::Boolean(value)) => Self::Boolean(value),
            (JavaType::Byte, Self::Byte(value)) => Self::Byte(value),
            (JavaType::Byte, Self::Int(value)) => {
                narrow_int_return(value, i8::MIN as i32, i8::MAX as i32, "byte", operation)
                    .map(|value| Self::Byte(value as jni::jbyte))?
            }
            (JavaType::Char, Self::Char(value)) => Self::Char(value),
            (JavaType::Char, Self::Int(value)) => {
                narrow_int_return(value, 0, u16::MAX as i32, "char", operation)
                    .map(|value| Self::Char(value as jni::jchar))?
            }
            (JavaType::Short, Self::Short(value)) => Self::Short(value),
            (JavaType::Short, Self::Int(value)) => {
                narrow_int_return(value, i16::MIN as i32, i16::MAX as i32, "short", operation)
                    .map(|value| Self::Short(value as jni::jshort))?
            }
            (JavaType::Int, Self::Int(value)) => Self::Int(value),
            (JavaType::Long, Self::Long(value)) => Self::Long(value),
            (JavaType::Long, Self::Int(value)) => Self::Long(value as jni::jlong),
            (JavaType::Float, Self::Float(value)) => Self::Float(value),
            (JavaType::Float, Self::Double(value)) => {
                Self::Float(double_to_float_return(value, operation)?)
            }
            (JavaType::Double, Self::Double(value)) => Self::Double(value),
            (JavaType::Double, Self::Float(value)) => Self::Double(value as jni::jdouble),
            (JavaType::Object(_), Self::Object(value) | Self::Array(value)) => Self::Object(value),
            (JavaType::Array(_), Self::Array(value) | Self::Object(value)) => Self::Array(value),
            (return_type, actual) => Err(invalid_hook_return(
                operation,
                return_type.jni_return_name(),
                actual,
            ))?,
        };
        Ok(value)
    }

    fn validate_for_return_type(
        self,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<Self> {
        let value = match (return_type, self) {
            (JavaType::Void, Self::Void) => Self::Void,
            (JavaType::Boolean, Self::Boolean(value)) => Self::Boolean(value),
            (JavaType::Byte, Self::Byte(value)) => Self::Byte(value),
            (JavaType::Char, Self::Char(value)) => Self::Char(value),
            (JavaType::Short, Self::Short(value)) => Self::Short(value),
            (JavaType::Int, Self::Int(value)) => Self::Int(value),
            (JavaType::Long, Self::Long(value)) => Self::Long(value),
            (JavaType::Float, Self::Float(value)) => Self::Float(value),
            (JavaType::Double, Self::Double(value)) => Self::Double(value),
            (JavaType::Object(_), Self::Object(value) | Self::Array(value)) => Self::Object(value),
            (JavaType::Array(_), Self::Array(value) | Self::Object(value)) => Self::Array(value),
            (return_type, actual) => Err(invalid_hook_return(
                operation,
                return_type.jni_return_name(),
                actual,
            ))?,
        };
        Ok(value)
    }
}

impl IntoJavaHookReturn for JavaHookReturn {
    fn into_hook_return(self) -> JavaHookReturn {
        self
    }

    fn into_hook_return_for(
        self,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn> {
        self.validate_for_return_type(return_type, operation)
    }
}

impl IntoJavaHookReturn for () {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::void()
    }
}

impl IntoJavaHookReturn for bool {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::boolean(self)
    }
}

macro_rules! impl_hook_primitive_conversion {
    ($type:ty, $return_constructor:ident, $value_variant:ident, $extractor:ident, $name:literal) => {
        impl IntoJavaHookReturn for $type {
            fn into_hook_return(self) -> JavaHookReturn {
                JavaHookReturn::$return_constructor(self)
            }
        }

        impl sealed::FromJavaValueSealed for $type {}

        impl FromJavaValue for $type {
            fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
                match value {
                    JavaValue::$value_variant(value) => Ok(value),
                    other => Err(invalid_java_value(index, $name, other)),
                }
            }
        }

        impl<'state> FromJavaHookReturn<'state> for $type {
            fn from_hook_return(
                value: JavaHookReturn,
                _context: &JavaHookContext<'state>,
                operation: &'static str,
            ) -> Result<Self> {
                value.$extractor(operation)
            }
        }
    };
}

impl_hook_primitive_conversion!(jni::jbyte, byte, Byte, into_byte, "byte");
impl_hook_primitive_conversion!(jni::jchar, char, Char, into_char, "char");
impl_hook_primitive_conversion!(jni::jshort, short, Short, into_short, "short");
impl_hook_primitive_conversion!(jni::jint, int, Int, into_int, "int");
impl_hook_primitive_conversion!(jni::jlong, long, Long, into_long, "long");
impl_hook_primitive_conversion!(jni::jfloat, float, Float, into_float, "float");
impl_hook_primitive_conversion!(jni::jdouble, double, Double, into_double, "double");

impl sealed::FromJavaValueSealed for bool {}

impl FromJavaValue for bool {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Boolean(value) => Ok(value),
            other => Err(invalid_java_value(index, "boolean", other)),
        }
    }
}

impl<'state, T> FromJavaHookArgument<'state> for T
where
    T: FromJavaValue,
{
    fn from_hook_argument(
        _context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        T::from_java_value(value, index)
    }
}

impl<'state> FromJavaHookArgument<'state> for Option<JavaLocalObject<'state>> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        match context.signature().arguments().get(index) {
            Some(JavaType::Object(_)) => {}
            Some(JavaType::Array(_)) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "object".to_owned(),
                    actual: "array",
                });
            }
            Some(actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "object".to_owned(),
                    actual: actual.jni_return_name(),
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: context.inner.arguments().len(),
                });
            }
        }

        match value {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => context
                .local_object(value.as_jobject(), "JavaHookContext::arg")
                .map(Some),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "object", other)),
        }
    }
}

impl<'state> FromJavaHookArgument<'state> for JavaLocalObject<'state> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        Option::<JavaLocalObject<'state>>::from_hook_argument(context, value, index)?.ok_or(
            Error::NullReturn {
                operation: "JavaHookContext::arg",
            },
        )
    }
}

impl<'state> FromJavaHookArgument<'state> for Option<JavaLocalArray<'state>> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        let element_type = match context.signature().arguments().get(index) {
            Some(JavaType::Array(element)) => (**element).clone(),
            Some(JavaType::Object(_)) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "array".to_owned(),
                    actual: "object",
                });
            }
            Some(actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "array".to_owned(),
                    actual: actual.jni_return_name(),
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: context.inner.arguments().len(),
                });
            }
        };

        match value {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => context
                .local_array(value.as_jobject(), element_type, "JavaHookContext::arg")
                .map(Some),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "array", other)),
        }
    }
}

impl<'state> FromJavaHookArgument<'state> for JavaLocalArray<'state> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        Option::<JavaLocalArray<'state>>::from_hook_argument(context, value, index)?.ok_or(
            Error::NullReturn {
                operation: "JavaHookContext::arg",
            },
        )
    }
}

impl<'state> FromJavaHookArgument<'state> for Option<String> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        match value {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => context
                .local_object(value.as_jobject(), "JavaHookContext::arg")?
                .get_string()
                .map(Some),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "java.lang.String", other)),
        }
    }
}

impl<'state> FromJavaHookArgument<'state> for String {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        Option::<String>::from_hook_argument(context, value, index)?.ok_or(Error::NullReturn {
            operation: "JavaHookContext::arg",
        })
    }
}

impl<'state> FromJavaHookReturn<'state> for () {
    fn from_hook_return(
        value: JavaHookReturn,
        _context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        value.into_void(operation)
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaHookReturn {
    fn from_hook_return(
        value: JavaHookReturn,
        _context: &JavaHookContext<'state>,
        _operation: &'static str,
    ) -> Result<Self> {
        Ok(value)
    }
}

impl<'state> FromJavaHookReturn<'state> for bool {
    fn from_hook_return(
        value: JavaHookReturn,
        _context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        value.into_boolean(operation)
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<String> {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            JavaHookReturn::Object(value) => value
                .map(|object| {
                    context
                        .local_object(object.as_jobject(), operation)?
                        .get_string()
                })
                .transpose(),
            other => Err(invalid_hook_return(operation, "java.lang.String", other)),
        }
    }
}

impl<'state> FromJavaHookReturn<'state> for String {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<String>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<JavaLocalObject<'state>> {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            JavaHookReturn::Object(value) => value
                .map(|object| context.local_object(object.as_jobject(), operation))
                .transpose(),
            other => Err(invalid_hook_return(operation, "object", other)),
        }
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaLocalObject<'state> {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaLocalObject<'state>>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<JavaLocalArray<'state>> {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            JavaHookReturn::Array(value) => {
                let element_type = match context.signature().return_type() {
                    JavaType::Array(element) => (**element).clone(),
                    actual => {
                        return Err(Error::InvalidReturnType {
                            operation,
                            expected: "array",
                            actual: actual.to_string(),
                        });
                    }
                };
                value
                    .map(|array| context.local_array(array.as_jobject(), element_type, operation))
                    .transpose()
            }
            other => Err(invalid_hook_return(operation, "array", other)),
        }
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaLocalArray<'state> {
    fn from_hook_return(
        value: JavaHookReturn,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaLocalArray<'state>>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<R> IntoJavaHookReturn for &JavaObject<R>
where
    R: JavaObjectRef,
{
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(Some(self))
    }
}

impl<R> IntoJavaHookReturn for Option<&JavaObject<R>>
where
    R: JavaObjectRef,
{
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(self)
    }
}

impl IntoJavaHookReturn for JavaLocalObject<'_> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(Some(&self))
    }
}

impl IntoJavaHookReturn for Option<JavaLocalObject<'_>> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(self.as_ref())
    }
}

impl From<JavaLocalObject<'_>> for JavaHookReturn {
    fn from(value: JavaLocalObject<'_>) -> Self {
        Self::object(Some(&value))
    }
}

impl From<Option<JavaLocalObject<'_>>> for JavaHookReturn {
    fn from(value: Option<JavaLocalObject<'_>>) -> Self {
        Self::object(value.as_ref())
    }
}

impl<R> IntoJavaHookReturn for &JavaArray<R>
where
    R: JavaObjectRef,
{
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(Some(self))
    }
}

impl<R> IntoJavaHookReturn for Option<&JavaArray<R>>
where
    R: JavaObjectRef,
{
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(self)
    }
}

impl IntoJavaHookReturn for JavaLocalArray<'_> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(Some(&self))
    }
}

impl IntoJavaHookReturn for Option<JavaLocalArray<'_>> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(self.as_ref())
    }
}

impl From<JavaLocalArray<'_>> for JavaHookReturn {
    fn from(value: JavaLocalArray<'_>) -> Self {
        Self::array(Some(&value))
    }
}

impl From<Option<JavaLocalArray<'_>>> for JavaHookReturn {
    fn from(value: Option<JavaLocalArray<'_>>) -> Self {
        Self::array(value.as_ref())
    }
}

fn invalid_java_value(index: usize, expected: &'static str, actual: JavaValue) -> Error {
    Error::InvalidArgumentType {
        index,
        expected: expected.to_owned(),
        actual: actual.type_name(),
    }
}

fn invalid_hook_return(
    operation: &'static str,
    expected: &'static str,
    actual: JavaHookReturn,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: hook_return_type_name(actual).to_owned(),
    }
}

fn narrow_int_return(
    value: jni::jint,
    min: jni::jint,
    max: jni::jint,
    expected: &'static str,
    operation: &'static str,
) -> Result<jni::jint> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(Error::InvalidReturnType {
            operation,
            expected,
            actual: format!("int {value} outside {expected} range"),
        })
    }
}

fn double_to_float_return(value: jni::jdouble, operation: &'static str) -> Result<jni::jfloat> {
    if value.is_finite() && value.abs() > f32::MAX as f64 {
        Err(Error::InvalidReturnType {
            operation,
            expected: "float",
            actual: format!("double {value} outside float range"),
        })
    } else {
        Ok(value as jni::jfloat)
    }
}

fn hook_return_type_name(value: JavaHookReturn) -> &'static str {
    match value {
        JavaHookReturn::Void => "void",
        JavaHookReturn::Boolean(_) => "boolean",
        JavaHookReturn::Byte(_) => "byte",
        JavaHookReturn::Char(_) => "char",
        JavaHookReturn::Short(_) => "short",
        JavaHookReturn::Int(_) => "int",
        JavaHookReturn::Long(_) => "long",
        JavaHookReturn::Float(_) => "float",
        JavaHookReturn::Double(_) => "double",
        JavaHookReturn::Object(_) => "object",
        JavaHookReturn::Array(_) => "array",
    }
}

pub(crate) unsafe fn install_method_hook<F, R>(
    overload: &JavaMethod,
    callback: F,
) -> Result<JavaHookGuard>
where
    F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
    R: IntoJavaHookReturn,
{
    validate_hook_abi(overload.kind(), overload.name(), overload.signature())?;
    let return_type = overload.signature().return_type().clone();
    let return_class = resolve_reference_return_class(overload.class(), &return_type)?;
    let inner = unsafe {
        replace_closure_method(overload, move |invocation| {
            let env = invocation.env_raw();
            callback(JavaHookContext { inner: invocation }).and_then(|value| {
                let hook_return = value
                    .into_hook_return_for(&return_type, "closure replacement return")
                    .and_then(|value| {
                        validate_reference_return(env, &return_class, &return_type, value)
                    })?;
                Ok(hook_return.into_raw())
            })
        })
    }?;
    Ok(JavaHookGuard { inner })
}

pub(crate) unsafe fn install_constructor_hook<F, R>(
    overload: &JavaConstructor,
    callback: F,
) -> Result<JavaHookGuard>
where
    F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
    R: IntoJavaHookReturn,
{
    validate_constructor_hook_abi(overload.signature())?;
    let return_type = overload.signature().return_type().clone();
    let inner = unsafe {
        replace_constructor_closure(overload, move |invocation| {
            callback(JavaHookContext { inner: invocation }).and_then(|value| {
                value
                    .into_hook_return_for(&return_type, "closure replacement return")
                    .map(JavaHookReturn::into_raw)
            })
        })
    }?;
    Ok(JavaHookGuard { inner })
}

fn resolve_reference_return_class(
    class: &RawJavaClass,
    return_type: &JavaType,
) -> Result<Option<RawJavaClass>> {
    if !return_type.is_reference() {
        return Ok(None);
    }

    let env = class.vm().attach_current_thread()?;
    let java = class.vm().java();
    let scoped_java = match metadata::class_loader(&env, &java, class)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    scoped_java.find_class(&return_type.to_string()).map(Some)
}

fn validate_reference_return(
    env: *mut jni::JNIEnv,
    return_class: &Option<RawJavaClass>,
    return_type: &JavaType,
    value: JavaHookReturn,
) -> Result<JavaHookReturn> {
    let Some(return_class) = return_class else {
        return Ok(value);
    };
    let raw = match value {
        JavaHookReturn::Object(None) | JavaHookReturn::Array(None) => return Ok(value),
        JavaHookReturn::Object(Some(object)) | JavaHookReturn::Array(Some(object)) => {
            object.as_jobject()
        }
        _ => return Ok(value),
    };

    let env = ptr::NonNull::new(env).ok_or(Error::NullReturn {
        operation: "closure replacement JNIEnv",
    })?;
    let env = Env::from_raw(env, return_class.vm());
    let object = unsafe { JavaLocalObject::from_raw(return_class.vm().clone(), raw)? };
    if env.is_instance_of(&object, return_class)? {
        Ok(value)
    } else {
        let actual = env.get_object_class(&object)?;
        Err(Error::InvalidObjectType {
            operation: "closure replacement return",
            expected: return_type.jni_return_name(),
            actual: format!("{:p} is not {}", actual.as_jclass(), return_type),
        })
    }
}

pub(crate) fn validate_hook_abi(
    kind: MethodKind,
    name: &str,
    signature: &MethodSignature,
) -> Result<()> {
    if kind == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: METHOD_HOOK_OPERATION,
        });
    }
    hook_signature_supported(kind, signature, METHOD_HOOK_OPERATION).map_err(|error| match error {
        Error::WrongMethodKind { .. } => Error::WrongMethodKind {
            operation: METHOD_HOOK_OPERATION,
        },
        Error::InvalidReplacementImplementation { actual, .. } => {
            Error::UnsupportedReplacementImplementation {
                operation: METHOD_HOOK_OPERATION,
                method: format!("{} {name}", hook_kind_name(kind)),
                reason: hook_unsupported_reason(actual),
            }
        }
        other => other,
    })
}

pub(crate) fn validate_constructor_hook_abi(signature: &MethodSignature) -> Result<()> {
    hook_signature_supported(
        MethodKind::Constructor,
        signature,
        CONSTRUCTOR_HOOK_OPERATION,
    )
    .map_err(|error| match error {
        Error::WrongMethodKind { .. } => Error::WrongMethodKind {
            operation: CONSTRUCTOR_HOOK_OPERATION,
        },
        Error::InvalidReplacementImplementation { actual, .. } => {
            Error::UnsupportedReplacementImplementation {
                operation: CONSTRUCTOR_HOOK_OPERATION,
                method: "constructor <init>".to_owned(),
                reason: hook_unsupported_reason(actual),
            }
        }
        other => other,
    })
}

fn hook_signature_supported(
    kind: MethodKind,
    signature: &MethodSignature,
    operation: &'static str,
) -> Result<()> {
    if kind == MethodKind::Constructor {
        return validate_closure_replacement_signature(kind, signature, operation);
    }
    validate_closure_replacement_signature(kind, signature, operation)
}

fn hook_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Static => "static method",
        MethodKind::Instance => "instance method",
        MethodKind::Constructor => "constructor",
    }
}

fn hook_unsupported_reason(actual: &'static str) -> &'static str {
    match actual {
        "descriptor is too large" | "descriptor overflows closure invocation frame sizing" => {
            "descriptor has too many arguments"
        }
        _ => "descriptor is unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(value: &str) -> MethodSignature {
        MethodSignature::parse(value).unwrap()
    }

    #[test]
    fn hook_return_conversions_report_expected_types() {
        assert_eq!(JavaHookReturn::int(7).into_int("test int").unwrap(), 7);
        assert_eq!(
            unsafe {
                JavaHookReturn::null_object()
                    .into_raw_object("test object")
                    .unwrap()
            },
            ptr::null_mut()
        );
        assert_eq!(
            unsafe {
                JavaHookReturn::null_array()
                    .into_raw_array("test array")
                    .unwrap()
            },
            ptr::null_mut()
        );

        assert_eq!(
            JavaHookReturn::null_object()
                .into_int("test wrong return")
                .unwrap_err(),
            Error::InvalidReturnType {
                operation: "test wrong return",
                expected: "int",
                actual: "object".to_owned(),
            }
        );
    }

    #[test]
    fn raw_object_and_array_returns_preserve_nullability() {
        assert_eq!(
            unsafe { JavaHookReturn::raw_object(ptr::null_mut()) },
            JavaHookReturn::null_object()
        );
        assert_eq!(
            unsafe { JavaHookReturn::raw_array(ptr::null_mut()) },
            JavaHookReturn::null_array()
        );

        let object = 0x1234usize as jni::jobject;
        let array = 0x5678usize as jni::jobject;
        assert_eq!(
            unsafe { JavaHookReturn::raw_object(object) }.into_raw(),
            RawJavaReturn::Object(object)
        );
        assert_eq!(
            unsafe { JavaHookReturn::raw_array(array) }.into_raw(),
            RawJavaReturn::Object(array)
        );
    }

    #[test]
    fn hook_admission_accepts_current_facade_lanes() {
        for (kind, name, descriptor) in [
            (MethodKind::Static, "staticAnswer", "()I"),
            (MethodKind::Static, "staticString", "()Ljava/lang/String;"),
            (MethodKind::Static, "staticArray", "()[Ljava/lang/Object;"),
            (MethodKind::Static, "staticIdentity", "(I)I"),
            (
                MethodKind::Static,
                "staticObjectEcho",
                "(Ljava/lang/Object;)Ljava/lang/Object;",
            ),
            (MethodKind::Instance, "objectSink", "(Ljava/lang/Object;)V"),
            (
                MethodKind::Static,
                "staticObjectIntVoid",
                "(Ljava/lang/Object;I)V",
            ),
            (MethodKind::Instance, "instanceAdd", "(II)I"),
            (
                MethodKind::Static,
                "staticObjectPairEcho",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            ),
            (
                MethodKind::Instance,
                "instanceObjectPairEcho",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            ),
            (MethodKind::Static, "staticPrimitiveMix", "(ZBCS)I"),
            (MethodKind::Instance, "instancePrimitiveMix", "(ZBCS)I"),
            (MethodKind::Static, "staticWide", "(JD)J"),
            (MethodKind::Instance, "instanceWide", "(JD)J"),
            (MethodKind::Static, "staticFloatMix", "(FD)D"),
            (MethodKind::Instance, "instanceFloatMix", "(FD)D"),
            (
                MethodKind::Static,
                "staticStackSpill",
                "(IIIIIIIIDDDDDDDDD)D",
            ),
            (
                MethodKind::Instance,
                "instanceStackSpill",
                "(IIIIIIIIDDDDDDDDD)D",
            ),
            (
                MethodKind::Static,
                "staticMixedReferences",
                "(Ljava/lang/Object;I[Ljava/lang/Object;Z)Ljava/lang/Object;",
            ),
            (MethodKind::Instance, "sumIntArray", "([I)I"),
        ] {
            validate_hook_abi(kind, name, &signature(descriptor)).unwrap();
        }
    }

    #[test]
    fn hook_admission_error_names_facade_and_reason() {
        let many_int_args = format!("({})I", "I".repeat(600));
        let error = validate_hook_abi(MethodKind::Static, "tooLarge", &signature(&many_int_args))
            .unwrap_err();

        let Error::UnsupportedReplacementImplementation {
            operation,
            method,
            reason,
        } = error
        else {
            panic!("unexpected admission error: {error:?}");
        };

        assert_eq!(operation, METHOD_HOOK_OPERATION);
        assert!(method.starts_with("static method tooLarge"));
        assert_eq!(reason, "descriptor has too many arguments");
    }

    #[test]
    fn hook_admission_rejects_constructors_as_facade_operation() {
        assert_eq!(
            validate_hook_abi(MethodKind::Constructor, "$init", &signature("()V")).unwrap_err(),
            Error::WrongMethodKind {
                operation: METHOD_HOOK_OPERATION,
            }
        );
    }

    #[test]
    fn constructor_hook_admission_accepts_void_constructor_lanes() {
        for descriptor in ["()V", "(I)V", "(Ljava/lang/Object;IZ[Ljava/lang/Object;)V"] {
            validate_constructor_hook_abi(&signature(descriptor))
                .unwrap_or_else(|_| panic!("constructor facade should accept {descriptor}"));
        }
    }

    #[test]
    fn constructor_hook_admission_error_names_facade_and_reason() {
        let many_int_args = format!("({})V", "I".repeat(600));
        let error = validate_constructor_hook_abi(&signature(&many_int_args)).unwrap_err();

        let Error::UnsupportedReplacementImplementation {
            operation,
            method,
            reason,
        } = error
        else {
            panic!("unexpected admission error: {error:?}");
        };

        assert_eq!(operation, CONSTRUCTOR_HOOK_OPERATION);
        assert_eq!(method, "constructor <init>");
        assert_eq!(reason, "descriptor has too many arguments");
    }

    #[test]
    fn constructor_hook_admission_rejects_non_void_descriptors() {
        assert_eq!(
            validate_constructor_hook_abi(&signature("()I")).unwrap_err(),
            Error::UnsupportedReplacementImplementation {
                operation: CONSTRUCTOR_HOOK_OPERATION,
                method: "constructor <init>".to_owned(),
                reason: "descriptor is unsupported",
            }
        );
    }
}
