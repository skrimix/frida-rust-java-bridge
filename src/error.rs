use std::{char::DecodeUtf16Error, ffi::NulError, str::Utf8Error};

use thiserror::Error as ThisError;

pub type Result<T> = std::result::Result<T, Error>;

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
    #[error("{operation} raised a Java exception")]
    JavaException { operation: &'static str },
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
    #[error("{operation} expected {expected}, got {actual}")]
    InvalidObjectType {
        operation: &'static str,
        expected: &'static str,
        actual: String,
    },
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
    #[error("class {class} has no {kind} overload {name}{arguments}")]
    OverloadNotFound {
        class: String,
        kind: &'static str,
        name: String,
        arguments: String,
    },
    #[error("class {class} has ambiguous {kind} overload {name}{arguments}: {matches} matches")]
    AmbiguousOverload {
        class: String,
        kind: &'static str,
        name: String,
        arguments: String,
        matches: usize,
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
    #[error("string contains an interior NUL: {value:?}")]
    InteriorNul { value: String },
    #[error("JNI string is not valid UTF-8")]
    InvalidUtf8,
    #[error("JNI string is not valid UTF-16")]
    InvalidUtf16,
}

impl Error {
    pub(crate) fn jni_result(operation: &'static str, code: i32) -> Result<()> {
        if code == crate::jni::JNI_OK {
            Ok(())
        } else {
            Err(Self::JniCallFailed { operation, code })
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
        assert_eq!(Error::jni_result("test", crate::jni::JNI_OK), Ok(()));
    }

    #[test]
    fn maps_jni_error_to_structured_error() {
        assert_eq!(
            Error::jni_result("test", -2),
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
}
