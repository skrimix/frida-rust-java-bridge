use std::ptr;

use crate::{
    Error, Result,
    env::{Env, MethodKind},
    java::{
        IntoJavaArgs, JavaArray, JavaLocalArray, JavaLocalObject, JavaMethodOverload, JavaObject,
    },
    jni,
    refs::AsJObject,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
};

use super::{
    closure::{
        ClosureMethodReplacement, ReplacementInvocation, replace_closure_method,
        validate_closure_replacement_signature,
    },
    original::RawJavaReturn,
};

const IMPLEMENTATION_OPERATION: &str = "JavaMethodOverload::install_implementation";

pub struct ImplementationGuard {
    inner: ClosureMethodReplacement,
}

/// Invocation details passed to installed Rust method implementations.
///
/// This is a thin ergonomic wrapper over the raw closure-backed replacement callback. It is only
/// valid while the current thread is executing the replacement callback. The full argument list is
/// intentionally exposed for exploratory hooks; typed helpers are conveniences on top.
pub struct ImplementationInvocation<'state> {
    pub(crate) inner: ReplacementInvocation<'state>,
}

/// Return value accepted by installed Rust method implementations.
///
/// Object and array helpers borrow an existing JNI-backed wrapper and return its raw reference to
/// Java. The borrowed object or array must remain valid until the callback returns.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImplementationReturn {
    Void,
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(Option<jni::jobject>),
    Array(Option<jni::jobject>),
}

/// Converts Rust values into implementation return values.
///
/// This keeps the backend's explicit [`ImplementationReturn`] shape available while allowing
/// callbacks to return ordinary Rust primitives and borrowed Java objects for supported lanes.
pub trait IntoImplementationReturn {
    fn into_implementation_return(self) -> ImplementationReturn;
}

/// Converts one raw replacement argument into a typed Rust value.
///
/// This is intentionally limited to values that can be extracted without taking ownership of JNI
/// references. Object-like arguments are exposed as raw nullable JNI references for now.
pub trait FromJavaValue: Sized {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self>;
}

/// Extracts a typed value from an [`ImplementationReturn`].
///
/// This is primarily useful with [`ImplementationInvocation::call_original_as`].
pub trait FromImplementationReturn: Sized {
    fn from_implementation_return(
        value: ImplementationReturn,
        operation: &'static str,
    ) -> Result<Self>;
}

impl ImplementationGuard {
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

impl<'state> ImplementationInvocation<'state> {
    pub fn env_raw(&self) -> *mut jni::JNIEnv {
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

    pub fn class(&self) -> Option<jni::jclass> {
        self.inner.class()
    }

    pub fn receiver(&self) -> Option<jni::jobject> {
        self.inner.receiver()
    }

    pub fn receiver_object(&self) -> Result<Option<JavaLocalObject<'state>>> {
        self.inner
            .receiver()
            .map(|receiver| {
                self.local_object(receiver, "ImplementationInvocation::receiver_object")
            })
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
                .local_object(value, "ImplementationInvocation::arg_object")
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
                .local_array(value, element_type, "ImplementationInvocation::arg_array")
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
    /// This is safe at this API layer because `ImplementationInvocation` is only constructed while
    /// the current thread is inside the active replacement callback.
    pub fn call_original<A: IntoJavaArgs>(&self, args: A) -> Result<ImplementationReturn> {
        let original = unsafe { self.inner.call_original(args)? };
        Ok(ImplementationReturn::from_raw_for_type(
            original,
            self.signature().return_type(),
        ))
    }

    pub fn call_original_as<T, A>(&self, args: A) -> Result<T>
    where
        T: FromImplementationReturn,
        A: IntoJavaArgs,
    {
        T::from_implementation_return(
            self.call_original(args)?,
            "ImplementationInvocation::call_original_as",
        )
    }

    pub fn call_original_object<A: IntoJavaArgs>(
        &self,
        args: A,
    ) -> Result<Option<JavaLocalObject<'state>>> {
        match self.call_original(args)? {
            ImplementationReturn::Object(value) => value
                .map(|object| {
                    self.local_object(object, "ImplementationInvocation::call_original_object")
                })
                .transpose(),
            other => Err(invalid_implementation_return(
                "ImplementationInvocation::call_original_object",
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
                    operation: "ImplementationInvocation::call_original_array",
                    expected: "array",
                    actual: actual.to_string(),
                });
            }
        };

        match self.call_original(args)? {
            ImplementationReturn::Array(value) => value
                .map(|array| {
                    self.local_array(
                        array,
                        element_type,
                        "ImplementationInvocation::call_original_array",
                    )
                })
                .transpose(),
            other => Err(invalid_implementation_return(
                "ImplementationInvocation::call_original_array",
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

impl ImplementationReturn {
    pub fn into_void(self, operation: &'static str) -> Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(invalid_implementation_return(operation, "void", other)),
        }
    }

    pub fn into_boolean(self, operation: &'static str) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "boolean", other)),
        }
    }

    pub fn into_byte(self, operation: &'static str) -> Result<jni::jbyte> {
        match self {
            Self::Byte(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "byte", other)),
        }
    }

    pub fn into_char(self, operation: &'static str) -> Result<jni::jchar> {
        match self {
            Self::Char(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "char", other)),
        }
    }

    pub fn into_short(self, operation: &'static str) -> Result<jni::jshort> {
        match self {
            Self::Short(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "short", other)),
        }
    }

    pub fn into_int(self, operation: &'static str) -> Result<jni::jint> {
        match self {
            Self::Int(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "int", other)),
        }
    }

    pub fn into_long(self, operation: &'static str) -> Result<jni::jlong> {
        match self {
            Self::Long(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "long", other)),
        }
    }

    pub fn into_float(self, operation: &'static str) -> Result<jni::jfloat> {
        match self {
            Self::Float(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "float", other)),
        }
    }

    pub fn into_double(self, operation: &'static str) -> Result<jni::jdouble> {
        match self {
            Self::Double(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "double", other)),
        }
    }

    pub fn into_object(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Object(value) => Ok(value.unwrap_or(ptr::null_mut())),
            other => Err(invalid_implementation_return(operation, "object", other)),
        }
    }

    pub fn into_array(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Array(value) => Ok(value.unwrap_or(ptr::null_mut())),
            other => Err(invalid_implementation_return(operation, "array", other)),
        }
    }

    pub fn object<T: AsJObject + ?Sized>(value: Option<&T>) -> Self {
        Self::Object(value.map(AsJObject::as_jobject))
    }

    pub fn array<T: AsJObject + ?Sized>(value: Option<&T>) -> Self {
        Self::Array(value.map(AsJObject::as_jobject))
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
            Self::Object(Some(value))
        }
    }

    pub(crate) fn raw_array(value: jni::jobject) -> Self {
        if value.is_null() {
            Self::Array(None)
        } else {
            Self::Array(Some(value))
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
                RawJavaReturn::Object(value.unwrap_or(ptr::null_mut()))
            }
        }
    }
}

impl IntoImplementationReturn for ImplementationReturn {
    fn into_implementation_return(self) -> ImplementationReturn {
        self
    }
}

impl IntoImplementationReturn for () {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::Void
    }
}

impl IntoImplementationReturn for bool {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::Boolean(self)
    }
}

macro_rules! impl_implementation_primitive_conversion {
    ($type:ty, $return_variant:ident, $value_variant:ident, $extractor:ident, $name:literal) => {
        impl IntoImplementationReturn for $type {
            fn into_implementation_return(self) -> ImplementationReturn {
                ImplementationReturn::$return_variant(self)
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

        impl FromImplementationReturn for $type {
            fn from_implementation_return(
                value: ImplementationReturn,
                operation: &'static str,
            ) -> Result<Self> {
                value.$extractor(operation)
            }
        }
    };
}

impl_implementation_primitive_conversion!(jni::jbyte, Byte, Byte, into_byte, "byte");
impl_implementation_primitive_conversion!(jni::jchar, Char, Char, into_char, "char");
impl_implementation_primitive_conversion!(jni::jshort, Short, Short, into_short, "short");
impl_implementation_primitive_conversion!(jni::jint, Int, Int, into_int, "int");
impl_implementation_primitive_conversion!(jni::jlong, Long, Long, into_long, "long");
impl_implementation_primitive_conversion!(jni::jfloat, Float, Float, into_float, "float");
impl_implementation_primitive_conversion!(jni::jdouble, Double, Double, into_double, "double");

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

impl FromJavaValue for jni::jobject {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Object(value) => Ok(value),
            JavaValue::Null => Ok(ptr::null_mut()),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }
}

impl FromJavaValue for Option<jni::jobject> {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Object(value) if value.is_null() => Ok(None),
            JavaValue::Object(value) => Ok(Some(value)),
            JavaValue::Null => Ok(None),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }
}

impl FromImplementationReturn for ImplementationReturn {
    fn from_implementation_return(
        value: ImplementationReturn,
        _operation: &'static str,
    ) -> Result<Self> {
        Ok(value)
    }
}

impl FromImplementationReturn for () {
    fn from_implementation_return(
        value: ImplementationReturn,
        operation: &'static str,
    ) -> Result<Self> {
        value.into_void(operation)
    }
}

impl FromImplementationReturn for bool {
    fn from_implementation_return(
        value: ImplementationReturn,
        operation: &'static str,
    ) -> Result<Self> {
        value.into_boolean(operation)
    }
}

impl FromImplementationReturn for jni::jobject {
    fn from_implementation_return(
        value: ImplementationReturn,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            ImplementationReturn::Object(value) | ImplementationReturn::Array(value) => {
                Ok(value.unwrap_or(ptr::null_mut()))
            }
            other => Err(invalid_implementation_return(operation, "reference", other)),
        }
    }
}

impl FromImplementationReturn for Option<jni::jobject> {
    fn from_implementation_return(
        value: ImplementationReturn,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            ImplementationReturn::Object(value) | ImplementationReturn::Array(value) => Ok(value),
            other => Err(invalid_implementation_return(operation, "reference", other)),
        }
    }
}

impl IntoImplementationReturn for &JavaObject {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::object(Some(self))
    }
}

impl IntoImplementationReturn for Option<&JavaObject> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::object(self)
    }
}

impl IntoImplementationReturn for &JavaLocalObject<'_> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::object(Some(self))
    }
}

impl IntoImplementationReturn for Option<&JavaLocalObject<'_>> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::object(self)
    }
}

impl IntoImplementationReturn for &JavaArray {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::array(Some(self))
    }
}

impl IntoImplementationReturn for Option<&JavaArray> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::array(self)
    }
}

impl IntoImplementationReturn for &JavaLocalArray<'_> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::array(Some(self))
    }
}

impl IntoImplementationReturn for Option<&JavaLocalArray<'_>> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::array(self)
    }
}

impl IntoImplementationReturn for jni::jobject {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::raw_object(self)
    }
}

impl IntoImplementationReturn for Option<jni::jobject> {
    fn into_implementation_return(self) -> ImplementationReturn {
        ImplementationReturn::Object(self)
    }
}

fn invalid_java_value(index: usize, expected: &'static str, actual: JavaValue) -> Error {
    Error::InvalidArgumentType {
        index,
        expected: expected.to_owned(),
        actual: actual.type_name(),
    }
}

fn invalid_implementation_return(
    operation: &'static str,
    expected: &'static str,
    actual: ImplementationReturn,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: implementation_return_type_name(actual).to_owned(),
    }
}

fn implementation_return_type_name(value: ImplementationReturn) -> &'static str {
    match value {
        ImplementationReturn::Void => "void",
        ImplementationReturn::Boolean(_) => "boolean",
        ImplementationReturn::Byte(_) => "byte",
        ImplementationReturn::Char(_) => "char",
        ImplementationReturn::Short(_) => "short",
        ImplementationReturn::Int(_) => "int",
        ImplementationReturn::Long(_) => "long",
        ImplementationReturn::Float(_) => "float",
        ImplementationReturn::Double(_) => "double",
        ImplementationReturn::Object(_) => "object",
        ImplementationReturn::Array(_) => "array",
    }
}

pub(crate) unsafe fn install_implementation_method<F, R>(
    overload: &JavaMethodOverload,
    callback: F,
) -> Result<ImplementationGuard>
where
    F: for<'a> Fn(ImplementationInvocation<'a>) -> Result<R> + Send + Sync + 'static,
    R: IntoImplementationReturn,
{
    validate_implementation_abi(overload.kind(), overload.name(), overload.signature())?;
    let inner = unsafe {
        replace_closure_method(overload, move |invocation| {
            callback(ImplementationInvocation { inner: invocation })
                .map(|value| value.into_implementation_return().into_raw())
        })
    }?;
    Ok(ImplementationGuard { inner })
}

pub(crate) fn validate_implementation_abi(
    kind: MethodKind,
    name: &str,
    signature: &MethodSignature,
) -> Result<()> {
    implementation_signature_supported(kind, signature).map_err(|error| match error {
        Error::WrongMethodKind { .. } => Error::WrongMethodKind {
            operation: IMPLEMENTATION_OPERATION,
        },
        Error::InvalidReplacementImplementation { actual, .. } => {
            Error::UnsupportedReplacementImplementation {
                operation: IMPLEMENTATION_OPERATION,
                method: format!("{} {name}", implementation_kind_name(kind)),
                reason: implementation_unsupported_reason(actual),
            }
        }
        other => other,
    })
}

fn implementation_signature_supported(kind: MethodKind, signature: &MethodSignature) -> Result<()> {
    validate_closure_replacement_signature(kind, signature, IMPLEMENTATION_OPERATION)
}

fn implementation_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Static => "static method",
        MethodKind::Instance => "instance method",
        MethodKind::Constructor => "constructor",
    }
}

fn implementation_unsupported_reason(actual: &'static str) -> &'static str {
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
    fn implementation_return_conversions_report_expected_types() {
        assert_eq!(
            ImplementationReturn::Int(7).into_int("test int").unwrap(),
            7
        );
        assert_eq!(
            ImplementationReturn::Object(None)
                .into_object("test object")
                .unwrap(),
            ptr::null_mut()
        );
        assert_eq!(
            ImplementationReturn::Array(None)
                .into_array("test array")
                .unwrap(),
            ptr::null_mut()
        );

        assert_eq!(
            ImplementationReturn::Object(None)
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
            ImplementationReturn::raw_object(ptr::null_mut()),
            ImplementationReturn::Object(None)
        );
        assert_eq!(
            ImplementationReturn::raw_array(ptr::null_mut()),
            ImplementationReturn::Array(None)
        );

        let object = 0x1234usize as jni::jobject;
        let array = 0x5678usize as jni::jobject;
        assert_eq!(
            ImplementationReturn::raw_object(object),
            ImplementationReturn::Object(Some(object))
        );
        assert_eq!(
            ImplementationReturn::raw_array(array),
            ImplementationReturn::Array(Some(array))
        );
    }

    #[test]
    fn implementation_admission_accepts_current_facade_lanes() {
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
            validate_implementation_abi(kind, name, &signature(descriptor)).unwrap();
        }
    }

    #[test]
    fn implementation_admission_error_names_facade_and_reason() {
        let many_int_args = format!("({})I", "I".repeat(600));
        let error =
            validate_implementation_abi(MethodKind::Static, "tooLarge", &signature(&many_int_args))
                .unwrap_err();

        let Error::UnsupportedReplacementImplementation {
            operation,
            method,
            reason,
        } = error
        else {
            panic!("unexpected admission error: {error:?}");
        };

        assert_eq!(operation, IMPLEMENTATION_OPERATION);
        assert!(method.starts_with("static method tooLarge"));
        assert_eq!(reason, "descriptor has too many arguments");
    }

    #[test]
    fn implementation_admission_rejects_constructors_as_facade_operation() {
        assert_eq!(
            validate_implementation_abi(MethodKind::Constructor, "$init", &signature("()V"))
                .unwrap_err(),
            Error::WrongMethodKind {
                operation: IMPLEMENTATION_OPERATION,
            }
        );
    }
}
