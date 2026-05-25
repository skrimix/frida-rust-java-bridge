use std::{char::DecodeUtf16Error, ffi::NulError, str::Utf8Error};

#[cfg(target_os = "android")]
use std::sync::Arc;

use thiserror::Error as ThisError;

use crate::jni;

#[cfg(target_os = "android")]
use crate::vm::Vm;

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(target_os = "android")]
#[derive(Clone)]
pub struct JavaThrowable {
    inner: Arc<JavaThrowableInner>,
}

#[cfg(not(target_os = "android"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaThrowable {
    _private: (),
}

#[cfg(target_os = "android")]
struct JavaThrowableInner {
    vm: Vm,
    throwable: jni::jthrowable,
}

#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
pub enum Error {
    #[error("Android ART runtime module was not found")]
    ArtRuntimeNotFound,
    #[error("symbol {symbol} was not found in {module}")]
    SymbolNotFound {
        module: String,
        symbol: &'static str,
    },
    #[error("{feature} is not supported: {reason}")]
    UnsupportedFeature {
        feature: &'static str,
        reason: String,
    },
    #[error("default app class loader is not available: {reason}")]
    AppClassLoaderUnavailable { reason: String },
    #[error("no created Java VM was found")]
    NoCreatedJavaVm,
    #[error("{operation} failed with JNI result {code}")]
    JniCallFailed { operation: &'static str, code: i32 },
    #[error("{operation} raised a Java exception: {exception}")]
    JavaException {
        operation: &'static str,
        exception: String,
        throwable: Option<JavaThrowable>,
    },
    #[error("{operation} returned null")]
    NullReturn { operation: &'static str },
    #[error("invalid Java signature {signature:?} at offset {offset}: {message}")]
    InvalidSignature {
        signature: String,
        offset: usize,
        message: &'static str,
    },
    #[error("expected {expected} Java arguments, got {actual}")]
    InvalidArguments { expected: usize, actual: usize },
    #[error("Java argument {index} has type {actual}, expected {expected}")]
    InvalidArgumentType {
        index: usize,
        expected: String,
        actual: &'static str,
    },
    #[error("Java argument {index} has value {actual}, expected {expected}")]
    InvalidArgumentValue {
        index: usize,
        expected: String,
        actual: String,
    },
    #[error("{operation} requires {expected} return, got {actual}")]
    InvalidReturnType {
        operation: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("{operation} requires {expected} field, got {actual}")]
    InvalidFieldType {
        operation: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("{operation} field value has type {actual}, expected {expected}")]
    InvalidFieldValueType {
        operation: &'static str,
        expected: String,
        actual: &'static str,
    },
    #[error("{operation} field value {actual}, expected {expected}")]
    InvalidFieldValue {
        operation: &'static str,
        expected: String,
        actual: String,
    },
    #[error("{operation} expected {expected}, got {actual}")]
    InvalidObjectType {
        operation: &'static str,
        expected: &'static str,
        actual: String,
    },
    #[error("class loader returned {actual} for requested class {requested}")]
    ClassLookupMismatch { requested: String, actual: String },
    #[error("invalid query {query:?}: {message}")]
    InvalidQuery {
        query: String,
        message: &'static str,
    },
    #[error("class {class} has no {kind} method {name}{signature}")]
    MethodNotFound {
        class: String,
        kind: &'static str,
        name: String,
        signature: String,
    },
    #[error("class {class} has no {kind} method {name}")]
    MethodNameNotFound {
        class: String,
        kind: &'static str,
        name: String,
    },
    #[error("class {class} has no {kind} overload {name}{arguments}")]
    OverloadNotFound {
        class: String,
        kind: &'static str,
        name: String,
        arguments: String,
    },
    #[error(
        "class {class} has no compatible {kind} overload {name} for arguments {arguments}; candidates: {candidates:?}"
    )]
    NoCompatibleOverload {
        class: String,
        kind: &'static str,
        name: String,
        arguments: String,
        candidates: Vec<String>,
    },
    #[error("class {class} has ambiguous {kind} overload {name}{arguments}: {matches} matches")]
    AmbiguousOverload {
        class: String,
        kind: &'static str,
        name: String,
        arguments: String,
        matches: usize,
    },
    #[error(
        "class {class} has ambiguous {kind} method {name}; use overload(...) to choose one of: {candidates:?}"
    )]
    AmbiguousMethod {
        class: String,
        kind: &'static str,
        name: String,
        candidates: Vec<String>,
    },
    #[error("class {class} has no {kind} field {name}: {ty}")]
    FieldNotFound {
        class: String,
        kind: &'static str,
        name: String,
        ty: String,
    },
    #[error("class {class} has no {kind} field {name}")]
    FieldNameNotFound {
        class: String,
        kind: &'static str,
        name: String,
    },
    #[error("class {class} has ambiguous {kind} field {name}; choose one of: {candidates:?}")]
    AmbiguousField {
        class: String,
        kind: &'static str,
        name: String,
        candidates: Vec<String>,
    },
    #[error("{operation} was called with the wrong method kind")]
    WrongMethodKind { operation: &'static str },
    #[error("{operation} was called with the wrong field kind")]
    WrongFieldKind { operation: &'static str },
    #[error("{operation} expected {expected} replacement implementation, got {actual}")]
    InvalidReplacementImplementation {
        operation: &'static str,
        expected: String,
        actual: &'static str,
    },
    #[error("{operation} does not support replacement implementation for {method}: {reason}")]
    UnsupportedReplacementImplementation {
        operation: &'static str,
        method: String,
        reason: &'static str,
    },
    #[error("{operation} invalid replacement state: {reason}")]
    InvalidReplacementState {
        operation: &'static str,
        reason: String,
    },
    #[error("string contains an interior NUL: {value:?}")]
    InteriorNul { value: String },
    #[error("JNI string is not valid UTF-8")]
    InvalidUtf8,
    #[error("JNI string is not valid UTF-16")]
    InvalidUtf16,
}

impl Error {
    pub(crate) fn check_jni_result(operation: &'static str, code: i32) -> Result<()> {
        if code == crate::jni::JNI_OK {
            Ok(())
        } else {
            Err(Self::JniCallFailed { operation, code })
        }
    }

    pub(crate) fn java_throwable(&self) -> Option<&JavaThrowable> {
        match self {
            Self::JavaException { throwable, .. } => throwable.as_ref(),
            _ => None,
        }
    }
}

#[cfg(target_os = "android")]
impl JavaThrowable {
    pub(crate) unsafe fn from_global_raw(vm: Vm, throwable: jni::jthrowable) -> Self {
        Self {
            inner: Arc::new(JavaThrowableInner { vm, throwable }),
        }
    }

    pub(crate) unsafe fn throw(&self, env: std::ptr::NonNull<jni::JNIEnv>) -> Result<()> {
        let throw = unsafe { jni::env_function::<jni::Throw>(env, jni::ENV_THROW) };
        let result = unsafe { throw(env.as_ptr(), self.inner.throwable) };
        Error::check_jni_result("JNIEnv::Throw", result)
    }
}

#[cfg(target_os = "android")]
impl std::fmt::Debug for JavaThrowable {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaThrowable")
            .field("throwable", &self.inner.throwable)
            .finish_non_exhaustive()
    }
}

#[cfg(target_os = "android")]
impl PartialEq for JavaThrowable {
    fn eq(&self, other: &Self) -> bool {
        self.inner.throwable == other.inner.throwable
    }
}

#[cfg(target_os = "android")]
impl Eq for JavaThrowable {}

// JNI global references are VM-scoped handles and may be used from any attached thread.
#[cfg(target_os = "android")]
unsafe impl Send for JavaThrowableInner {}
#[cfg(target_os = "android")]
unsafe impl Sync for JavaThrowableInner {}

#[cfg(target_os = "android")]
impl Drop for JavaThrowableInner {
    fn drop(&mut self) {
        if self.throwable.is_null() {
            return;
        }

        if let Ok(env) = self.vm.attach_current_thread() {
            unsafe { env.delete_global_ref_raw(self.throwable) };
        }
    }
}

impl From<NulError> for Error {
    fn from(error: NulError) -> Self {
        let bytes = error.into_vec();
        Self::InteriorNul {
            value: String::from_utf8_lossy(&bytes).into_owned(),
        }
    }
}

impl From<Utf8Error> for Error {
    fn from(_: Utf8Error) -> Self {
        Self::InvalidUtf8
    }
}

impl From<DecodeUtf16Error> for Error {
    fn from(_: DecodeUtf16Error) -> Self {
        Self::InvalidUtf16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_jni_ok_to_success() {
        assert_eq!(Error::check_jni_result("test", crate::jni::JNI_OK), Ok(()));
    }

    #[test]
    fn maps_jni_error_to_structured_error() {
        assert_eq!(
            Error::check_jni_result("test", -2),
            Err(Error::JniCallFailed {
                operation: "test",
                code: -2,
            })
        );
    }

    #[test]
    fn formats_app_class_loader_unavailable() {
        let error = Error::AppClassLoaderUnavailable {
            reason: "ActivityThread.currentApplication() returned null".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "default app class loader is not available: ActivityThread.currentApplication() returned null"
        );
    }

    #[test]
    fn formats_class_lookup_mismatch() {
        let error = Error::ClassLookupMismatch {
            requested: "frida.java.bridge.rs.test.TestSubject".to_owned(),
            actual: "java.lang.String".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "class loader returned java.lang.String for requested class frida.java.bridge.rs.test.TestSubject"
        );
    }

    #[test]
    fn formats_java_exception_details() {
        let error = Error::JavaException {
            operation: "JNIEnv::CallStaticObjectMethodA",
            exception: "java.lang.IllegalStateException: boom".to_owned(),
            throwable: None,
        };

        assert_eq!(
            error.to_string(),
            "JNIEnv::CallStaticObjectMethodA raised a Java exception: java.lang.IllegalStateException: boom"
        );
    }
}
