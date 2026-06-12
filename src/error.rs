use std::{char::DecodeUtf16Error, ffi::NulError, str::Utf8Error, time::Duration};

#[cfg(target_os = "android")]
use std::sync::Arc;

use thiserror::Error as ThisError;

use crate::jni;

#[cfg(target_os = "android")]
use crate::vm::Vm;

#[cfg(target_os = "android")]
/// Java exception object captured during a JNI operation.
///
/// Most callers only need the formatted text in [`Error::JavaException`]. The throwable is kept as
/// a global reference so internal code can rethrow it on an attached thread when needed.
#[derive(Clone)]
pub struct JavaThrowable {
    inner: Arc<JavaThrowableInner>,
}

#[cfg(not(target_os = "android"))]
/// Placeholder exception handle used on non-Android builds.
///
/// Non-Android builds can name error types for host-testable code, but they cannot capture a live
/// Java exception object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaThrowable {
    _private: (),
}

#[cfg(target_os = "android")]
struct JavaThrowableInner {
    vm: Vm,
    throwable: jni::jthrowable,
}

/// Result type returned by bridge APIs.
///
/// Errors are structured so callers can distinguish unsupported runtime features, Java
/// exceptions, lookup failures, type mismatches, and raw JNI failures without parsing display text.
pub type Result<T> = std::result::Result<T, Error>;

/// Error returned while working with Java, JNI, or ART.
///
/// The display text is meant for diagnostics. Match on variants such as
/// [`Error::UnsupportedFeature`] or [`Error::JavaException`] when code needs to handle a specific
/// failure.
#[derive(Debug, Clone, PartialEq, Eq, ThisError)]
pub enum Error {
    // Runtime discovery and feature support.
    #[error("ART runtime module was not found")]
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
    #[cfg(feature = "art-selftest")]
    #[error("{harness} test failed: {reason}")]
    TestFailure {
        harness: &'static str,
        reason: String,
    },
    /// The Android app class loader is not available to the current operation.
    ///
    /// This usually means app startup has not captured an `Application` yet, or the current process
    /// shape cannot support deferred app-loader discovery.
    #[error("default app class loader is not available: {reason}")]
    AppClassLoaderUnavailable { reason: String },
    /// Waiting for the Android app class loader exceeded the specified timeout.
    #[error("timed out after {timeout:?} waiting for default app class loader: {reason}")]
    AppClassLoaderWaitTimedOut { timeout: Duration, reason: String },
    /// Waiting for an Android main-thread task exceeded the specified timeout.
    #[error("timed out after {timeout:?} waiting for main-thread task: {reason}")]
    MainThreadTaskWaitTimedOut { timeout: Duration, reason: String },
    #[error("no created Java VM was found")]
    NoCreatedJavaVm,

    // Raw JNI call state.
    #[error("{operation} failed with JNI result {code}")]
    JniCallFailed { operation: &'static str, code: i32 },
    /// A Java exception was pending after a JNI operation.
    ///
    /// Safe JNI helpers clear the pending exception before returning this error. The optional
    /// throwable is retained when the call site needs to rethrow the same Java exception later.
    #[error("{operation} raised a Java exception: {exception}")]
    JavaException {
        operation: &'static str,
        exception: String,
        throwable: Option<JavaThrowable>,
    },
    #[error("{operation} returned null")]
    NullReturn { operation: &'static str },

    // Descriptor parsing and argument/return conversion.
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

    // Class-loader and query validation.
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

    // Method, constructor, overload, and field selection.
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

    // Method and constructor replacement.
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

    // String conversion.
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

// JNI global references are process-runtime handles and may be used from any attached thread.
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

        self.vm.delete_global_ref_best_effort(self.throwable);
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
    fn formats_app_class_loader_wait_timeout() {
        let error = Error::AppClassLoaderWaitTimedOut {
            timeout: Duration::from_millis(25),
            reason: "startup hooks did not publish a loader".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "timed out after 25ms waiting for default app class loader: startup hooks did not publish a loader"
        );
    }

    #[test]
    fn formats_main_thread_task_wait_timeout() {
        let error = Error::MainThreadTaskWaitTimedOut {
            timeout: Duration::from_millis(25),
            reason: "main-thread task was still pending".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "timed out after 25ms waiting for main-thread task: main-thread task was still pending"
        );
    }

    #[test]
    fn formats_class_lookup_mismatch() {
        let error = Error::ClassLookupMismatch {
            requested: "frida.rust.java.bridge.test.TestSubject".to_owned(),
            actual: "java.lang.String".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "class loader returned java.lang.String for requested class frida.rust.java.bridge.test.TestSubject"
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

    #[cfg(feature = "art-selftest")]
    #[test]
    fn formats_test_failure_distinctly_from_unsupported_feature() {
        let error = Error::TestFailure {
            harness: "app_process",
            reason: "answer mismatch".to_owned(),
        };

        assert_eq!(
            error.to_string(),
            "app_process test failed: answer mismatch"
        );
    }
}
