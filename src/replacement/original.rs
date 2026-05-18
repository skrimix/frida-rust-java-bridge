use crate::{
    Error, Result,
    env::MethodKind,
    java::{IntoJavaArgs, JavaClass, JavaConstructorOverload, JavaMethodOverload},
    jni,
    signature::MethodSignature,
};

use super::native::{
    call_original_constructor_method, call_original_instance_method, call_original_static_method,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RawJavaReturn {
    Void,
    Boolean(jni::jboolean),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(jni::jobject),
}

/// Captures the metadata needed to call a replaced method's original implementation.
#[derive(Clone)]
pub(crate) struct OriginalMethod {
    kind: MethodKind,
    name: String,
    signature: String,
    declaring_class: Option<JavaClass>,
}

impl OriginalMethod {
    pub(crate) fn new(overload: &JavaMethodOverload) -> Result<Self> {
        Self::from_parts(
            overload.kind(),
            overload.name(),
            &overload.signature().to_string(),
        )
    }

    pub(crate) fn new_constructor(overload: &JavaConstructorOverload) -> Result<Self> {
        Ok(Self {
            kind: MethodKind::Constructor,
            name: "<init>".to_owned(),
            signature: MethodSignature::parse(&overload.signature().to_string())?.to_string(),
            declaring_class: Some(overload.class().clone()),
        })
    }

    #[cfg(test)]
    pub(crate) fn kind(&self) -> MethodKind {
        self.kind
    }

    #[cfg(test)]
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    #[cfg(test)]
    pub(crate) fn signature(&self) -> &str {
        &self.signature
    }

    /// Calls this static method's original implementation from a replacement callback.
    ///
    /// # Safety
    ///
    /// `env` and `class` must be the valid JNI environment and declaring class received by the
    /// active replacement callback, and this must only be called while the current thread is inside
    /// a replacement for this method.
    pub(crate) unsafe fn call_static<A: IntoJavaArgs>(
        &self,
        env: *mut jni::JNIEnv,
        class: jni::jclass,
        args: A,
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_static",
            });
        }
        unsafe { call_original_static_method(env, class, &self.name, &self.signature, args) }
    }

    /// Calls this instance method's original implementation from a replacement callback.
    ///
    /// # Safety
    ///
    /// `env` and `receiver` must be the valid JNI environment and receiver received by the active
    /// replacement callback, and this must only be called while the current thread is inside a
    /// replacement for this method.
    pub(crate) unsafe fn call_instance<A: IntoJavaArgs>(
        &self,
        env: *mut jni::JNIEnv,
        receiver: jni::jobject,
        args: A,
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Instance {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_instance",
            });
        }
        unsafe { call_original_instance_method(env, receiver, &self.name, &self.signature, args) }
    }

    /// Calls this constructor's original implementation from a replacement callback.
    ///
    /// # Safety
    ///
    /// `env` and `receiver` must be the valid JNI environment and receiver received by the active
    /// constructor replacement callback, and this must only be called while the current thread is
    /// inside a replacement for this constructor.
    pub(crate) unsafe fn call_constructor<A: IntoJavaArgs>(
        &self,
        env: *mut jni::JNIEnv,
        receiver: jni::jobject,
        args: A,
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Constructor {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_constructor",
            });
        }
        let declaring_class = self
            .declaring_class
            .as_ref()
            .ok_or(Error::WrongMethodKind {
                operation: "OriginalMethod::call_constructor",
            })?;
        unsafe {
            call_original_constructor_method(env, receiver, declaring_class, &self.signature, args)
        }
    }

    pub(crate) fn from_parts(kind: MethodKind, name: &str, signature: &str) -> Result<Self> {
        if kind == MethodKind::Constructor {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::new",
            });
        }
        Ok(Self {
            kind,
            name: name.to_owned(),
            signature: MethodSignature::parse(signature)?.to_string(),
            declaring_class: None,
        })
    }
}

impl std::fmt::Debug for OriginalMethod {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OriginalMethod")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("signature", &self.signature)
            .field(
                "declaring_class",
                &self.declaring_class.as_ref().map(JavaClass::name),
            )
            .finish()
    }
}

impl PartialEq for OriginalMethod {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
            && self.name == other.name
            && self.signature == other.signature
            && self.declaring_class.as_ref().map(JavaClass::name)
                == other.declaring_class.as_ref().map(JavaClass::name)
    }
}

impl Eq for OriginalMethod {}

#[allow(dead_code)]
impl RawJavaReturn {
    pub(crate) fn into_void(self, operation: &'static str) -> Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(invalid_raw_return(operation, "void", other)),
        }
    }

    pub(crate) fn into_boolean(self, operation: &'static str) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(value == jni::JNI_TRUE),
            other => Err(invalid_raw_return(operation, "boolean", other)),
        }
    }

    pub(crate) fn into_byte(self, operation: &'static str) -> Result<jni::jbyte> {
        match self {
            Self::Byte(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "byte", other)),
        }
    }

    pub(crate) fn into_char(self, operation: &'static str) -> Result<jni::jchar> {
        match self {
            Self::Char(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "char", other)),
        }
    }

    pub(crate) fn into_short(self, operation: &'static str) -> Result<jni::jshort> {
        match self {
            Self::Short(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "short", other)),
        }
    }

    pub(crate) fn into_int(self, operation: &'static str) -> Result<jni::jint> {
        match self {
            Self::Int(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "int", other)),
        }
    }

    pub(crate) fn into_long(self, operation: &'static str) -> Result<jni::jlong> {
        match self {
            Self::Long(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "long", other)),
        }
    }

    pub(crate) fn into_float(self, operation: &'static str) -> Result<jni::jfloat> {
        match self {
            Self::Float(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "float", other)),
        }
    }

    pub(crate) fn into_double(self, operation: &'static str) -> Result<jni::jdouble> {
        match self {
            Self::Double(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "double", other)),
        }
    }

    pub(crate) fn into_object(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Object(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "object", other)),
        }
    }
}

pub(super) fn invalid_raw_return(
    operation: &'static str,
    expected: &'static str,
    actual: RawJavaReturn,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: raw_return_type_name(actual).to_owned(),
    }
}

fn raw_return_type_name(value: RawJavaReturn) -> &'static str {
    match value {
        RawJavaReturn::Void => "void",
        RawJavaReturn::Boolean(_) => "boolean",
        RawJavaReturn::Byte(_) => "byte",
        RawJavaReturn::Char(_) => "char",
        RawJavaReturn::Short(_) => "short",
        RawJavaReturn::Int(_) => "int",
        RawJavaReturn::Long(_) => "long",
        RawJavaReturn::Float(_) => "float",
        RawJavaReturn::Double(_) => "double",
        RawJavaReturn::Object(_) => "object",
    }
}
