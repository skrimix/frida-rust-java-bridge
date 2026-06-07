use std::{fmt, marker::PhantomData, ptr, ptr::NonNull};

use crate::{
    Error, Result,
    env::Env,
    java::{
        Java, JavaArray, JavaClass, JavaLocalArray, JavaLocalObject, JavaObject,
        conversion::{accepts_rust_string, coerce_java_return_value},
        raw,
    },
    jni, metadata,
    refs::{AsJClass, JavaObjectRef},
    signature::JavaType,
    value::{JavaValue, RawJavaObject},
    vm::Vm,
};

use super::{context::JavaHookContext, original::RawJavaReturn};

/// Reference payload used by hook returns.
///
/// The lifetime ties callback-local object and array returns to the active hook invocation until
/// the replacement layer promotes them for Java.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct JavaHookReturnObject<'state> {
    raw: RawJavaObject,
    _state: PhantomData<&'state ()>,
}

impl<'state> JavaHookReturnObject<'state> {
    fn from_raw(raw: jni::jobject) -> Self {
        Self {
            raw: RawJavaObject::from_raw(raw),
            _state: PhantomData,
        }
    }

    pub(super) fn as_jobject(self) -> jni::jobject {
        self.raw.as_jobject()
    }
}

impl fmt::Debug for JavaHookReturnObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("JavaHookReturnObject")
            .field(&self.raw)
            .finish()
    }
}

/// Explicit return value accepted by installed Rust method hooks.
///
/// This token is bound to the active callback lifetime so local JNI references cannot be
/// represented as a storable return value outside the callback that produced them.
pub type JavaHookReturn<'state> = JavaValue<JavaHookReturnObject<'state>>;

/// Converts Rust values into hook return values.
///
/// This powers [`JavaHookContext::ret`], allowing callbacks to pass ordinary Rust primitives,
/// strings, owned Java wrappers, or callback-local Java views into one lifetime-bound return token.
/// Numeric values are adapted to the selected Java method's return descriptor at the hook boundary,
/// so Rust's default literal types do not accidentally select the wrong JNI return lane.
pub trait IntoJavaHookReturn<'state> {
    #[doc(hidden)]
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>>;
}

/// Extracts a typed original-call return value.
///
/// This is used by [`JavaHookContext::call_original`].
pub trait FromJavaHookReturn<'state>: Sized {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self>;
}

impl<'state> JavaHookReturn<'state> {
    /// Extracts a raw JNI object reference from an object return.
    ///
    /// # Safety
    ///
    /// The returned reference has the lifetime and VM identity of the hook/original-call context
    /// that produced it. The caller must only use it while that context remains valid.
    pub unsafe fn into_raw_object(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Object(value) => {
                Ok(value.map_or(ptr::null_mut(), JavaHookReturnObject::as_jobject))
            }
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
            Self::Object(value) => {
                Ok(value.map_or(ptr::null_mut(), JavaHookReturnObject::as_jobject))
            }
            other => Err(invalid_hook_return(operation, "array", other)),
        }
    }

    /// Builds an object return from a borrowed crate-owned Java reference.
    ///
    /// # Safety
    ///
    /// The borrowed reference must be valid for the VM running the replacement callback and must
    /// remain valid until the callback returns to ART. Callback-local references must be returned
    /// immediately from the active callback and must not be stored.
    pub unsafe fn object<T: JavaObjectRef + ?Sized>(value: Option<&T>) -> Self {
        Self::Object(value.map(|value| {
            JavaHookReturnObject::from_raw(crate::refs::sealed::JavaObjectRefSealed::as_jobject(
                value,
            ))
        }))
    }

    /// Builds an array return from a borrowed crate-owned Java array reference.
    ///
    /// # Safety
    ///
    /// The borrowed reference must be a valid array reference for the VM running the replacement
    /// callback and must remain valid until the callback returns to ART. Callback-local references
    /// must be returned immediately from the active callback and must not be stored.
    pub unsafe fn array<T: JavaObjectRef + ?Sized>(value: Option<&T>) -> Self {
        Self::Object(value.map(|value| {
            JavaHookReturnObject::from_raw(crate::refs::sealed::JavaObjectRefSealed::as_jobject(
                value,
            ))
        }))
    }

    pub fn null_object() -> Self {
        Self::Object(None)
    }

    pub fn null_array() -> Self {
        Self::Object(None)
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
            Self::Object(Some(JavaHookReturnObject::from_raw(value)))
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
            Self::Object(Some(JavaHookReturnObject::from_raw(value)))
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
                    Self::Object(Some(JavaHookReturnObject::from_raw(value)))
                }
            }
        }
    }

    pub(super) fn from_raw_for_type(value: RawJavaReturn, return_type: &JavaType) -> Self {
        let _ = return_type;
        Self::from_raw(value)
    }

    pub(super) fn into_raw(self) -> RawJavaReturn {
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
            Self::Object(value) => RawJavaReturn::Object(
                value.map_or(ptr::null_mut(), JavaHookReturnObject::as_jobject),
            ),
        }
    }

    pub(super) fn coerce_for_return_type(
        self,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<Self> {
        coerce_java_return_value(self, return_type, operation)
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
            (JavaType::Object(_) | JavaType::Array(_), Self::Object(value)) => Self::Object(value),
            (return_type, actual) => Err(invalid_hook_return(
                operation,
                return_type.jni_return_name(),
                actual,
            ))?,
        };
        Ok(value)
    }
}

impl<'state> IntoJavaHookReturn<'state> for JavaHookReturn<'state> {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = env;
        let _ = vm;
        self.validate_for_return_type(return_type, operation)
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaHookReturn<'state> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        _context: &JavaHookContext<'state>,
        _operation: &'static str,
    ) -> Result<Self> {
        Ok(value)
    }
}

impl<'state> IntoJavaHookReturn<'state> for () {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = env;
        let _ = vm;
        JavaHookReturn::void().coerce_for_return_type(return_type, operation)
    }
}

impl<'state> IntoJavaHookReturn<'state> for bool {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = env;
        let _ = vm;
        JavaHookReturn::boolean(self).coerce_for_return_type(return_type, operation)
    }
}

macro_rules! impl_hook_primitive_return {
    ($type:ty, $return_constructor:ident, $extractor:ident) => {
        impl<'state> IntoJavaHookReturn<'state> for $type {
            fn into_hook_return_for(
                self,
                env: *mut jni::JNIEnv,
                vm: &Vm,
                return_type: &JavaType,
                operation: &'static str,
            ) -> Result<JavaHookReturn<'state>> {
                let _ = env;
                let _ = vm;
                JavaHookReturn::$return_constructor(self)
                    .coerce_for_return_type(return_type, operation)
            }
        }

        impl<'state> FromJavaHookReturn<'state> for $type {
            fn from_hook_return(
                value: JavaHookReturn<'state>,
                _context: &JavaHookContext<'state>,
                operation: &'static str,
            ) -> Result<Self> {
                value.$extractor(operation)
            }
        }
    };
}

impl_hook_primitive_return!(jni::jbyte, byte, into_byte);
impl_hook_primitive_return!(jni::jchar, char, into_char);
impl_hook_primitive_return!(jni::jshort, short, into_short);
impl_hook_primitive_return!(jni::jint, int, into_int);
impl_hook_primitive_return!(jni::jlong, long, into_long);
impl_hook_primitive_return!(jni::jfloat, float, into_float);
impl_hook_primitive_return!(jni::jdouble, double, into_double);

impl<'state> FromJavaHookReturn<'state> for () {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        _context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        value.into_void(operation)
    }
}

impl<'state> FromJavaHookReturn<'state> for bool {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        _context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        value.into_boolean(operation)
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<String> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            JavaHookReturn::Object(value) => value
                .map(|object| {
                    context
                        .local_object_for_return(object.as_jobject(), operation)?
                        .get_string()
                })
                .transpose(),
            other => Err(invalid_hook_return(operation, "java.lang.String", other)),
        }
    }
}

impl<'state> FromJavaHookReturn<'state> for String {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<String>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<JavaLocalObject<'state>> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            JavaHookReturn::Object(value) => value
                .map(|object| context.local_object_for_return(object.as_jobject(), operation))
                .transpose(),
            other => Err(invalid_hook_return(operation, "object", other)),
        }
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaLocalObject<'state> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaLocalObject<'state>>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<JavaObject> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaLocalObject<'state>>::from_hook_return(value, context, operation)?
            .map(|object| object.retain())
            .transpose()
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaObject {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaObject>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<JavaLocalArray<'state>> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        match value {
            JavaHookReturn::Object(value) => {
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
                    .map(|array| {
                        context.local_array_for_type(
                            array.as_jobject(),
                            &JavaType::Array(Box::new(element_type)),
                            operation,
                        )
                    })
                    .transpose()
            }
            other => Err(invalid_hook_return(operation, "array", other)),
        }
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaLocalArray<'state> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaLocalArray<'state>>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> FromJavaHookReturn<'state> for Option<JavaArray> {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaLocalArray<'state>>::from_hook_return(value, context, operation)?
            .map(|array| array.retain())
            .transpose()
    }
}

impl<'state> FromJavaHookReturn<'state> for JavaArray {
    fn from_hook_return(
        value: JavaHookReturn<'state>,
        context: &JavaHookContext<'state>,
        operation: &'static str,
    ) -> Result<Self> {
        Option::<JavaArray>::from_hook_return(value, context, operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl<'state> IntoJavaHookReturn<'state> for String {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        string_hook_return(env, vm, Some(&self), return_type, operation)
    }
}

impl<'state> IntoJavaHookReturn<'state> for &str {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        string_hook_return(env, vm, Some(self), return_type, operation)
    }
}

impl<'state> IntoJavaHookReturn<'state> for Option<String> {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        match self {
            Some(value) => string_hook_return(env, vm, Some(&value), return_type, operation),
            None => JavaHookReturn::null_object().coerce_for_return_type(return_type, operation),
        }
    }
}

impl<'state> IntoJavaHookReturn<'state> for Option<&str> {
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        match self {
            Some(value) => string_hook_return(env, vm, Some(value), return_type, operation),
            None => JavaHookReturn::null_object().coerce_for_return_type(return_type, operation),
        }
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for JavaObject<R>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        wrapper_hook_return(env, self.vm(), Some(&self), return_type, operation)
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for Option<JavaObject<R>>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        match self {
            Some(value) => {
                wrapper_hook_return(env, value.vm(), Some(&value), return_type, operation)
            }
            None => JavaHookReturn::null_object().coerce_for_return_type(return_type, operation),
        }
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for &JavaObject<R>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        wrapper_hook_return(env, self.vm(), Some(self), return_type, operation)
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for Option<&JavaObject<R>>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        match self {
            Some(value) => {
                wrapper_hook_return(env, value.vm(), Some(value), return_type, operation)
            }
            None => JavaHookReturn::null_object().coerce_for_return_type(return_type, operation),
        }
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for JavaArray<R>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        wrapper_hook_return(env, self.vm_ref(), Some(&self), return_type, operation)
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for Option<JavaArray<R>>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        match self {
            Some(value) => {
                wrapper_hook_return(env, value.vm_ref(), Some(&value), return_type, operation)
            }
            None => JavaHookReturn::null_array().coerce_for_return_type(return_type, operation),
        }
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for &JavaArray<R>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        wrapper_hook_return(env, self.vm_ref(), Some(self), return_type, operation)
    }
}

impl<'state, R> IntoJavaHookReturn<'state> for Option<&JavaArray<R>>
where
    R: JavaObjectRef,
{
    fn into_hook_return_for(
        self,
        env: *mut jni::JNIEnv,
        vm: &Vm,
        return_type: &JavaType,
        operation: &'static str,
    ) -> Result<JavaHookReturn<'state>> {
        let _ = vm;
        match self {
            Some(value) => {
                wrapper_hook_return(env, value.vm_ref(), Some(value), return_type, operation)
            }
            None => JavaHookReturn::null_array().coerce_for_return_type(return_type, operation),
        }
    }
}

fn wrapper_hook_return<'state, T: JavaObjectRef + ?Sized>(
    env: *mut jni::JNIEnv,
    vm: &crate::vm::Vm,
    value: Option<&T>,
    return_type: &JavaType,
    operation: &'static str,
) -> Result<JavaHookReturn<'state>> {
    let Some(value) = value else {
        return JavaHookReturn::null_object().coerce_for_return_type(return_type, operation);
    };
    let env = NonNull::new(env).ok_or(Error::NullReturn {
        operation: "closure replacement JNIEnv",
    })?;
    let env = Env::from_raw(env, vm.clone());
    let local = unsafe {
        env.new_local_ref_raw(crate::refs::sealed::JavaObjectRefSealed::as_jobject(value))?
    };
    unsafe { JavaHookReturn::raw_object(local) }.coerce_for_return_type(return_type, operation)
}

fn string_hook_return<'state>(
    env: *mut jni::JNIEnv,
    vm: &Vm,
    value: Option<&str>,
    return_type: &JavaType,
    operation: &'static str,
) -> Result<JavaHookReturn<'state>> {
    let Some(value) = value else {
        return JavaHookReturn::null_object().coerce_for_return_type(return_type, operation);
    };
    if !accepts_rust_string(return_type) {
        return Err(Error::InvalidReturnType {
            operation,
            expected: return_type.jni_return_name(),
            actual: "string".to_owned(),
        });
    }
    let env = NonNull::new(env).ok_or(Error::NullReturn {
        operation: "closure replacement JNIEnv",
    })?;
    let env = Env::from_raw(env, vm.clone());
    let local = unsafe { env.new_string_utf_raw(value)? };
    unsafe { JavaHookReturn::raw_object(local) }.coerce_for_return_type(return_type, operation)
}

pub(super) fn invalid_hook_return(
    operation: &'static str,
    expected: &'static str,
    actual: JavaHookReturn<'_>,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: hook_return_type_name(actual).to_owned(),
    }
}

fn hook_return_type_name(value: JavaHookReturn<'_>) -> &'static str {
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
    }
}

pub(super) fn resolve_reference_return_class(
    class: &raw::Class,
    return_type: &JavaType,
) -> Result<Option<raw::Class>> {
    if !return_type.is_reference() {
        return Ok(None);
    }

    let env = class.vm().attach_current_thread()?;
    let java = Java::new(class.vm().clone());
    let scoped_java = match metadata::class_loader(&env, class.vm(), class)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    scoped_java.find_class(&return_type.to_string()).map(Some)
}

pub(super) fn validate_reference_return<'state>(
    env: *mut jni::JNIEnv,
    return_class: &Option<raw::Class>,
    return_type: &JavaType,
    value: JavaHookReturn<'state>,
) -> Result<JavaHookReturn<'state>> {
    let Some(return_class) = return_class else {
        return Ok(value);
    };
    let raw = match value {
        JavaHookReturn::Object(None) => return Ok(value),
        JavaHookReturn::Object(Some(object)) => object.as_jobject(),
        _ => return Ok(value),
    };

    let env = ptr::NonNull::new(env).ok_or(Error::NullReturn {
        operation: "closure replacement JNIEnv",
    })?;
    let env = Env::from_raw(env, return_class.vm().clone());
    let object = unsafe {
        JavaLocalObject::from_raw_with_class(JavaClass::from_raw(return_class.clone()), raw)?
    };
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

#[cfg(test)]
mod tests {
    use super::*;

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
                actual: "null".to_owned(),
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
    fn hook_return_double_to_float_rejects_non_finite_values() {
        for value in [f64::INFINITY, f64::NAN] {
            assert_eq!(
                JavaHookReturn::double(value)
                    .coerce_for_return_type(&JavaType::Float, "test float return")
                    .unwrap_err(),
                Error::InvalidReturnType {
                    operation: "test float return",
                    expected: "float",
                    actual: format!("double {value} is not finite or outside float range"),
                }
            );
        }
    }
}
