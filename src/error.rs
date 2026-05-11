use std::{char::DecodeUtf16Error, ffi::NulError, fmt, str::Utf8Error};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    ArtRuntimeNotFound,
    SymbolNotFound {
        module: String,
        symbol: &'static str,
    },
    NoCreatedJavaVm,
    JniCallFailed {
        operation: &'static str,
        code: i32,
    },
    JavaException {
        operation: &'static str,
    },
    NullReturn {
        operation: &'static str,
    },
    InvalidSignature {
        signature: String,
        offset: usize,
        message: &'static str,
    },
    InvalidArguments {
        expected: usize,
        actual: usize,
    },
    InvalidArgumentType {
        index: usize,
        expected: String,
        actual: &'static str,
    },
    InvalidReturnType {
        operation: &'static str,
        expected: &'static str,
        actual: String,
    },
    WrongMethodKind {
        operation: &'static str,
    },
    InteriorNul {
        value: String,
    },
    InvalidUtf8,
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

impl fmt::Display for Error {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArtRuntimeNotFound => write!(fmt, "Android ART runtime module was not found"),
            Self::SymbolNotFound { module, symbol } => {
                write!(fmt, "symbol {symbol} was not found in {module}")
            }
            Self::NoCreatedJavaVm => write!(fmt, "no created Java VM was found"),
            Self::JniCallFailed { operation, code } => {
                write!(fmt, "{operation} failed with JNI result {code}")
            }
            Self::JavaException { operation } => {
                write!(fmt, "{operation} raised a Java exception")
            }
            Self::NullReturn { operation } => write!(fmt, "{operation} returned null"),
            Self::InvalidSignature {
                signature,
                offset,
                message,
            } => {
                write!(
                    fmt,
                    "invalid Java signature {signature:?} at offset {offset}: {message}"
                )
            }
            Self::InvalidArguments { expected, actual } => {
                write!(fmt, "expected {expected} Java arguments, got {actual}")
            }
            Self::InvalidArgumentType {
                index,
                expected,
                actual,
            } => {
                write!(
                    fmt,
                    "Java argument {index} has type {actual}, expected {expected}"
                )
            }
            Self::InvalidReturnType {
                operation,
                expected,
                actual,
            } => {
                write!(fmt, "{operation} requires {expected} return, got {actual}")
            }
            Self::WrongMethodKind { operation } => {
                write!(fmt, "{operation} was called with the wrong method kind")
            }
            Self::InteriorNul { value } => {
                write!(fmt, "string contains an interior NUL: {value:?}")
            }
            Self::InvalidUtf8 => write!(fmt, "JNI string is not valid UTF-8"),
            Self::InvalidUtf16 => write!(fmt, "JNI string is not valid UTF-16"),
        }
    }
}

impl std::error::Error for Error {}

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
}
