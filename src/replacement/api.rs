use std::ptr;

use crate::{
    Error, Result,
    env::{Env, MethodKind},
    java::{
        IntoJavaArgs, JavaArray, JavaConstructor, JavaLocalArray, JavaLocalObject, JavaMethod,
        JavaObject,
    },
    jni,
    refs::JavaObjectRef,
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

/// Invocation details passed to installed Rust method hooks.
///
/// This is a thin ergonomic wrapper over the raw closure-backed replacement callback. It is only
/// valid while the current thread is executing the replacement callback. The full argument list is
/// intentionally exposed for exploratory hooks; typed helpers are conveniences on top.
pub struct JavaHookContext<'state> {
    pub(crate) inner: ReplacementInvocation<'state>,
}

/// Return value accepted by installed Rust method hooks.
///
/// Object and array helpers borrow crate-owned JNI-backed wrappers. Explicit raw returns are only
/// available through [`RawJavaObject`], whose arbitrary-raw constructor is unsafe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JavaHookReturn {
    Void,
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(Option<RawJavaObject>),
    Array(Option<RawJavaObject>),
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
pub trait FromJavaValue: Sized {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self>;
}

/// Extracts a typed value from an [`JavaHookReturn`].
///
/// This is primarily useful with [`JavaHookContext::call_original`].
pub trait FromJavaHookReturn: Sized {
    fn from_hook_return(value: JavaHookReturn, operation: &'static str) -> Result<Self>;
}

pub trait JavaHookTarget {
    /// Replaces this hook target with a guarded Rust closure.
    ///
    /// # Safety
    ///
    /// This is backed by the experimental ART method-replacement prototype. The caller must keep
    /// the returned guard alive while the replacement should remain active and ensure callback
    /// return values are valid for the selected Java method signature.
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
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

    /// Returns a backend debug summary for diagnostics when the hidden ART backend provides one.
    pub fn debug_summary(&self) -> Option<String> {
        self.inner.debug_summary()
    }

    /// Returns the most recent callback error or panic recorded by the replacement.
    ///
    /// Callback failures cause Java callers to receive the JNI default value for the method's
    /// return type, and the error is kept here for explicit inspection.
    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error()
    }

    /// Returns and clears the most recent callback error or panic recorded by the replacement.
    pub fn take_last_error(&self) -> Option<String> {
        self.inner.take_last_error()
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
    ///
    /// # Safety
    ///
    /// This has the same ART method-replacement safety requirements as [`JavaHookTarget::replace`].
    /// Keep the set alive for as long as its replacements should remain installed.
    pub unsafe fn replace<T, F, R>(&mut self, target: T, callback: F) -> Result<&mut JavaHookGuard>
    where
        T: JavaHookTarget,
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        let guard = unsafe { target.replace(callback)? };
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
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        unsafe { install_method_hook(self, callback) }
    }
}

impl JavaHookTarget for &JavaMethod {
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        unsafe { (*self).replace(callback) }
    }
}

impl JavaHookTarget for JavaConstructor {
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        unsafe { install_constructor_hook(self, callback) }
    }
}

impl JavaHookTarget for &JavaConstructor {
    unsafe fn replace<F, R>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: IntoJavaHookReturn,
    {
        unsafe { (*self).replace(callback) }
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

    pub fn this_object(&self) -> Result<Option<JavaLocalObject<'state>>> {
        self.inner
            .receiver()
            .map(|receiver| self.local_object(receiver, "JavaHookContext::this_object"))
            .transpose()
    }

    pub fn arguments(&self) -> &[JavaValue] {
        self.inner.arguments()
    }

    pub fn args(&self) -> &[JavaValue] {
        self.arguments()
    }

    pub fn argument_count(&self) -> usize {
        self.arguments().len()
    }

    pub fn arg<T: FromJavaValue>(&self, index: usize) -> Result<T> {
        let value = self
            .arguments()
            .get(index)
            .copied()
            .ok_or(Error::InvalidArguments {
                expected: index + 1,
                actual: self.arguments().len(),
            })?;
        T::from_java_value(value, index)
    }

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
                    actual: self.arguments().len(),
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

    pub fn arg_string(&self, index: usize) -> Result<Option<String>> {
        self.arg_object(index)?
            .map(|object| object.get_string())
            .transpose()
    }

    /// Calls the replaced method's original implementation from this callback.
    ///
    /// This is safe at this API layer because `JavaHookContext` is only constructed while
    /// the current thread is inside the active replacement callback.
    pub fn call_original_raw<A: IntoJavaArgs>(&self, args: A) -> Result<JavaHookReturn> {
        let original = unsafe { self.inner.call_original(args)? };
        Ok(JavaHookReturn::from_raw_for_type(
            original,
            self.signature().return_type(),
        ))
    }

    pub fn call_original<T>(&self, args: impl IntoJavaArgs) -> Result<T>
    where
        T: FromJavaHookReturn,
    {
        T::from_hook_return(
            self.call_original_raw(args)?,
            "JavaHookContext::call_original",
        )
    }

    pub fn call_original_void<A: IntoJavaArgs>(&self, args: A) -> Result<()> {
        self.call_original_raw(args)?
            .into_void("JavaHookContext::call_original_void")
    }

    pub fn call_original_object<A: IntoJavaArgs>(
        &self,
        args: A,
    ) -> Result<Option<JavaLocalObject<'state>>> {
        match self.call_original_raw(args)? {
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

        match self.call_original_raw(args)? {
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

    pub fn call_original_string<A: IntoJavaArgs>(&self, args: A) -> Result<Option<String>> {
        self.call_original_object(args)?
            .map(|object| object.get_string())
            .transpose()
    }

    fn argument_value(&self, index: usize) -> Result<JavaValue> {
        self.arguments()
            .get(index)
            .copied()
            .ok_or(Error::InvalidArguments {
                expected: index + 1,
                actual: self.arguments().len(),
            })
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

impl JavaHookReturn {
    pub fn into_void(self, operation: &'static str) -> Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(invalid_hook_return(operation, "void", other)),
        }
    }

    pub fn into_boolean(self, operation: &'static str) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "boolean", other)),
        }
    }

    pub fn into_byte(self, operation: &'static str) -> Result<jni::jbyte> {
        match self {
            Self::Byte(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "byte", other)),
        }
    }

    pub fn into_char(self, operation: &'static str) -> Result<jni::jchar> {
        match self {
            Self::Char(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "char", other)),
        }
    }

    pub fn into_short(self, operation: &'static str) -> Result<jni::jshort> {
        match self {
            Self::Short(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "short", other)),
        }
    }

    pub fn into_int(self, operation: &'static str) -> Result<jni::jint> {
        match self {
            Self::Int(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "int", other)),
        }
    }

    pub fn into_long(self, operation: &'static str) -> Result<jni::jlong> {
        match self {
            Self::Long(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "long", other)),
        }
    }

    pub fn into_float(self, operation: &'static str) -> Result<jni::jfloat> {
        match self {
            Self::Float(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "float", other)),
        }
    }

    pub fn into_double(self, operation: &'static str) -> Result<jni::jdouble> {
        match self {
            Self::Double(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "double", other)),
        }
    }

    /// Extracts a raw JNI object reference from an object return.
    ///
    /// # Safety
    ///
    /// The returned reference has the lifetime and VM identity of the hook/original-call context
    /// that produced it. The caller must only use it while that context remains valid.
    pub unsafe fn into_object(self, operation: &'static str) -> Result<jni::jobject> {
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
    pub unsafe fn into_array(self, operation: &'static str) -> Result<jni::jobject> {
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

    pub(crate) fn raw_object(value: jni::jobject) -> Self {
        if value.is_null() {
            Self::Object(None)
        } else {
            Self::Object(Some(RawJavaObject::from_raw(value)))
        }
    }

    pub(crate) fn raw_array(value: jni::jobject) -> Self {
        if value.is_null() {
            Self::Array(None)
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
            RawJavaReturn::Object(value) => Self::raw_object(value),
        }
    }

    pub(crate) fn from_raw_for_type(value: RawJavaReturn, return_type: &JavaType) -> Self {
        match (value, return_type) {
            (RawJavaReturn::Object(value), JavaType::Array(_)) => Self::raw_array(value),
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
        match (return_type, self) {
            (JavaType::Void, Self::Void) => Ok(Self::Void),
            (JavaType::Boolean, Self::Boolean(value)) => Ok(Self::Boolean(value)),
            (JavaType::Byte, Self::Byte(value)) => Ok(Self::Byte(value)),
            (JavaType::Byte, Self::Int(value)) => {
                narrow_int_return(value, i8::MIN as i32, i8::MAX as i32, "byte", operation)
                    .map(|value| Self::Byte(value as jni::jbyte))
            }
            (JavaType::Char, Self::Char(value)) => Ok(Self::Char(value)),
            (JavaType::Char, Self::Int(value)) => {
                narrow_int_return(value, 0, u16::MAX as i32, "char", operation)
                    .map(|value| Self::Char(value as jni::jchar))
            }
            (JavaType::Short, Self::Short(value)) => Ok(Self::Short(value)),
            (JavaType::Short, Self::Int(value)) => {
                narrow_int_return(value, i16::MIN as i32, i16::MAX as i32, "short", operation)
                    .map(|value| Self::Short(value as jni::jshort))
            }
            (JavaType::Int, Self::Int(value)) => Ok(Self::Int(value)),
            (JavaType::Long, Self::Long(value)) => Ok(Self::Long(value)),
            (JavaType::Long, Self::Int(value)) => Ok(Self::Long(value as jni::jlong)),
            (JavaType::Float, Self::Float(value)) => Ok(Self::Float(value)),
            (JavaType::Float, Self::Double(value)) => {
                double_to_float_return(value, operation).map(Self::Float)
            }
            (JavaType::Double, Self::Double(value)) => Ok(Self::Double(value)),
            (JavaType::Double, Self::Float(value)) => Ok(Self::Double(value as jni::jdouble)),
            (JavaType::Object(_), Self::Object(value) | Self::Array(value)) => {
                Ok(Self::Object(value))
            }
            (JavaType::Array(_), Self::Array(value) | Self::Object(value)) => {
                Ok(Self::Array(value))
            }
            (return_type, actual) => Err(invalid_hook_return(
                operation,
                return_type.jni_return_name(),
                actual,
            )),
        }
    }

    fn validate_for_return_type(
        self,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<Self> {
        match (return_type, self) {
            (JavaType::Void, Self::Void) => Ok(Self::Void),
            (JavaType::Boolean, Self::Boolean(value)) => Ok(Self::Boolean(value)),
            (JavaType::Byte, Self::Byte(value)) => Ok(Self::Byte(value)),
            (JavaType::Char, Self::Char(value)) => Ok(Self::Char(value)),
            (JavaType::Short, Self::Short(value)) => Ok(Self::Short(value)),
            (JavaType::Int, Self::Int(value)) => Ok(Self::Int(value)),
            (JavaType::Long, Self::Long(value)) => Ok(Self::Long(value)),
            (JavaType::Float, Self::Float(value)) => Ok(Self::Float(value)),
            (JavaType::Double, Self::Double(value)) => Ok(Self::Double(value)),
            (JavaType::Object(_), Self::Object(value) | Self::Array(value)) => {
                Ok(Self::Object(value))
            }
            (JavaType::Array(_), Self::Array(value) | Self::Object(value)) => {
                Ok(Self::Array(value))
            }
            (return_type, actual) => Err(invalid_hook_return(
                operation,
                return_type.jni_return_name(),
                actual,
            )),
        }
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
        JavaHookReturn::Void
    }
}

impl IntoJavaHookReturn for bool {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::Boolean(self)
    }
}

macro_rules! impl_hook_primitive_conversion {
    ($type:ty, $return_variant:ident, $value_variant:ident, $extractor:ident, $name:literal) => {
        impl IntoJavaHookReturn for $type {
            fn into_hook_return(self) -> JavaHookReturn {
                JavaHookReturn::$return_variant(self)
            }
        }

        impl FromJavaValue for $type {
            fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
                match value {
                    JavaValue::$value_variant(value) => Ok(value),
                    other => Err(invalid_java_value(index, $name, other)),
                }
            }
        }

        impl FromJavaHookReturn for $type {
            fn from_hook_return(value: JavaHookReturn, operation: &'static str) -> Result<Self> {
                value.$extractor(operation)
            }
        }
    };
}

impl_hook_primitive_conversion!(jni::jbyte, Byte, Byte, into_byte, "byte");
impl_hook_primitive_conversion!(jni::jchar, Char, Char, into_char, "char");
impl_hook_primitive_conversion!(jni::jshort, Short, Short, into_short, "short");
impl_hook_primitive_conversion!(jni::jint, Int, Int, into_int, "int");
impl_hook_primitive_conversion!(jni::jlong, Long, Long, into_long, "long");
impl_hook_primitive_conversion!(jni::jfloat, Float, Float, into_float, "float");
impl_hook_primitive_conversion!(jni::jdouble, Double, Double, into_double, "double");

impl FromJavaValue for JavaValue {
    fn from_java_value(value: JavaValue, _index: usize) -> Result<Self> {
        Ok(value)
    }
}

impl FromJavaValue for bool {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Boolean(value) => Ok(value),
            other => Err(invalid_java_value(index, "boolean", other)),
        }
    }
}

impl FromJavaValue for RawJavaObject {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Object(value) if !value.is_null() => Ok(value),
            JavaValue::Object(_) | JavaValue::Null => Err(Error::NullReturn {
                operation: "JavaHookContext::arg raw object",
            }),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }
}

impl FromJavaValue for Option<RawJavaObject> {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => Ok(Some(value)),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }
}

impl FromJavaHookReturn for JavaHookReturn {
    fn from_hook_return(value: JavaHookReturn, _operation: &'static str) -> Result<Self> {
        Ok(value)
    }
}

impl FromJavaHookReturn for () {
    fn from_hook_return(value: JavaHookReturn, operation: &'static str) -> Result<Self> {
        value.into_void(operation)
    }
}

impl FromJavaHookReturn for RawJavaObject {
    fn from_hook_return(value: JavaHookReturn, operation: &'static str) -> Result<Self> {
        match value {
            JavaHookReturn::Object(Some(value)) | JavaHookReturn::Array(Some(value)) => Ok(value),
            JavaHookReturn::Object(None) | JavaHookReturn::Array(None) => {
                Err(Error::NullReturn { operation })
            }
            other => Err(invalid_hook_return(operation, "reference", other)),
        }
    }
}

impl FromJavaHookReturn for Option<RawJavaObject> {
    fn from_hook_return(value: JavaHookReturn, operation: &'static str) -> Result<Self> {
        match value {
            JavaHookReturn::Object(value) | JavaHookReturn::Array(value) => Ok(value),
            other => Err(invalid_hook_return(operation, "reference", other)),
        }
    }
}

impl FromJavaHookReturn for bool {
    fn from_hook_return(value: JavaHookReturn, operation: &'static str) -> Result<Self> {
        value.into_boolean(operation)
    }
}

impl IntoJavaHookReturn for &JavaObject {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(Some(self))
    }
}

impl IntoJavaHookReturn for Option<&JavaObject> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(self)
    }
}

impl IntoJavaHookReturn for &JavaLocalObject<'_> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(Some(self))
    }
}

impl IntoJavaHookReturn for Option<&JavaLocalObject<'_>> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::object(self)
    }
}

impl IntoJavaHookReturn for &JavaArray {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(Some(self))
    }
}

impl IntoJavaHookReturn for Option<&JavaArray> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(self)
    }
}

impl IntoJavaHookReturn for &JavaLocalArray<'_> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(Some(self))
    }
}

impl IntoJavaHookReturn for Option<&JavaLocalArray<'_>> {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::array(self)
    }
}

impl IntoJavaHookReturn for RawJavaObject {
    fn into_hook_return(self) -> JavaHookReturn {
        JavaHookReturn::raw_object(self.as_jobject())
    }
}

impl IntoJavaHookReturn for Option<RawJavaObject> {
    fn into_hook_return(self) -> JavaHookReturn {
        match self {
            Some(value) => JavaHookReturn::raw_object(value.as_jobject()),
            None => JavaHookReturn::null_object(),
        }
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
    let inner = unsafe {
        replace_closure_method(overload, move |invocation| {
            callback(JavaHookContext { inner: invocation }).and_then(|value| {
                value
                    .into_hook_return_for(&return_type, "closure replacement return")
                    .map(JavaHookReturn::into_raw)
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
        assert_eq!(JavaHookReturn::Int(7).into_int("test int").unwrap(), 7);
        assert_eq!(
            unsafe {
                JavaHookReturn::Object(None)
                    .into_object("test object")
                    .unwrap()
            },
            ptr::null_mut()
        );
        assert_eq!(
            unsafe {
                JavaHookReturn::Array(None)
                    .into_array("test array")
                    .unwrap()
            },
            ptr::null_mut()
        );

        assert_eq!(
            JavaHookReturn::Object(None)
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
            JavaHookReturn::raw_object(ptr::null_mut()),
            JavaHookReturn::Object(None)
        );
        assert_eq!(
            JavaHookReturn::raw_array(ptr::null_mut()),
            JavaHookReturn::Array(None)
        );

        let object = 0x1234usize as jni::jobject;
        let array = 0x5678usize as jni::jobject;
        assert_eq!(
            JavaHookReturn::raw_object(object),
            JavaHookReturn::Object(Some(RawJavaObject::from_raw(object)))
        );
        assert_eq!(
            JavaHookReturn::raw_array(array),
            JavaHookReturn::Array(Some(RawJavaObject::from_raw(array)))
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
