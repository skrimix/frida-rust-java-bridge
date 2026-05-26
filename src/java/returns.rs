use super::*;

impl std::fmt::Debug for JavaReturnRef {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Object(value) => fmt.debug_tuple("Object").field(value).finish(),
            Self::Array(value) => fmt.debug_tuple("Array").field(value).finish(),
        }
    }
}

impl PartialEq for JavaReturnRef {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Object(a), Self::Object(b)) => unsafe { a.raw_jobject() == b.raw_jobject() },
            (Self::Array(a), Self::Array(b)) => unsafe { a.raw_jobject() == b.raw_jobject() },
            _ => false,
        }
    }
}

impl std::fmt::Debug for JavaLocalReturnRef<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Object(value) => fmt.debug_tuple("Object").field(value).finish(),
            Self::Array(value) => fmt.debug_tuple("Array").field(value).finish(),
        }
    }
}

impl PartialEq for JavaLocalReturnRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Object(a), Self::Object(b)) => unsafe { a.raw_jobject() == b.raw_jobject() },
            (Self::Array(a), Self::Array(b)) => unsafe { a.raw_jobject() == b.raw_jobject() },
            _ => false,
        }
    }
}

macro_rules! java_value_extractors {
    ($($method:ident, $variant:ident, $ty:ty, $name:literal;)+) => {
        impl<R> JavaValue<R> {
            $(
                pub fn $method(self, operation: &'static str) -> Result<$ty> {
                    match self {
                        Self::$variant(value) => Ok(value),
                        other => Err(invalid_value_return(operation, $name, other)),
                    }
                }
            )+
        }
    };
}

java_value_extractors! {
    into_boolean, Boolean, bool, "boolean";
    into_byte, Byte, jni::jbyte, "byte";
    into_char, Char, jni::jchar, "char";
    into_short, Short, jni::jshort, "short";
    into_int, Int, jni::jint, "int";
    into_long, Long, jni::jlong, "long";
    into_float, Float, jni::jfloat, "float";
    into_double, Double, jni::jdouble, "double";
}

impl FromJavaReturn for JavaReturn {
    fn from_java_return(value: JavaReturn, _operation: &'static str) -> Result<Self> {
        Ok(value)
    }
}

impl JavaReturn {
    pub fn java_display(&self) -> Result<String> {
        Ok(match self {
            Self::Void => "void".to_owned(),
            Self::Boolean(value) => value.to_string(),
            Self::Byte(value) => value.to_string(),
            Self::Char(value) => display_java_char(*value),
            Self::Short(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Long(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Double(value) => value.to_string(),
            Self::Object(Some(JavaReturnRef::Object(value))) => value.java_display()?,
            Self::Object(Some(JavaReturnRef::Array(value))) => value.java_display()?,
            Self::Object(None) => "null".to_owned(),
        })
    }

    pub fn into_object(self, operation: &'static str) -> Result<Option<JavaObject>> {
        match self {
            Self::Object(Some(JavaReturnRef::Object(value))) => Ok(Some(value)),
            Self::Object(None) => Ok(None),
            other => Err(invalid_value_return(operation, "object", other)),
        }
    }

    pub fn into_array(self, operation: &'static str) -> Result<Option<JavaArray>> {
        match self {
            Self::Object(Some(JavaReturnRef::Array(value))) => Ok(Some(value)),
            Self::Object(None) => Ok(None),
            other => Err(invalid_value_return(operation, "array", other)),
        }
    }
}

impl FromJavaReturn for () {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self> {
        value.into_void(operation)
    }
}

macro_rules! impl_from_java_return {
    ($($ty:ty, $extractor:ident;)+) => {
        $(
            impl FromJavaReturn for $ty {
                fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self> {
                    value.$extractor(operation)
                }
            }
        )+
    };
}

impl_from_java_return! {
    bool, into_boolean;
    jni::jbyte, into_byte;
    jni::jchar, into_char;
    jni::jshort, into_short;
    jni::jint, into_int;
    jni::jlong, into_long;
    jni::jfloat, into_float;
    jni::jdouble, into_double;
    Option<JavaObject>, into_object;
    Option<JavaArray>, into_array;
}

impl FromJavaReturn for JavaObject {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self> {
        value
            .into_object(operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl FromJavaReturn for JavaArray {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self> {
        value
            .into_array(operation)?
            .ok_or(Error::NullReturn { operation })
    }
}

impl FromJavaReturn for Option<String> {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self> {
        value
            .into_object(operation)?
            .map(|object| object.get_string())
            .transpose()
    }
}

impl FromJavaReturn for String {
    fn from_java_return(value: JavaReturn, operation: &'static str) -> Result<Self> {
        Option::<String>::from_java_return(value, operation)?.ok_or(Error::NullReturn { operation })
    }
}

fn invalid_value_return<R>(
    operation: &'static str,
    expected: &'static str,
    actual: JavaValue<R>,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: actual.type_name().to_owned(),
    }
}

pub(crate) fn display_java_char(value: jni::jchar) -> String {
    char::from_u32(value as u32)
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("\\u{value:04X}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    type OwnedReturn = JavaReturn;

    #[test]
    fn displays_java_chars() {
        assert_eq!(display_java_char('A' as jni::jchar), "A");
        assert_eq!(display_java_char(0xD800), "\\uD800");
    }

    #[test]
    fn displays_primitive_and_null_returns() {
        assert_eq!(OwnedReturn::Void.java_display(), Ok("void".to_owned()));
        assert_eq!(
            OwnedReturn::Boolean(true).java_display(),
            Ok("true".to_owned())
        );
        assert_eq!(OwnedReturn::Byte(-7).java_display(), Ok("-7".to_owned()));
        assert_eq!(
            OwnedReturn::Char('A' as jni::jchar).java_display(),
            Ok("A".to_owned())
        );
        assert_eq!(
            OwnedReturn::Short(-300).java_display(),
            Ok("-300".to_owned())
        );
        assert_eq!(OwnedReturn::Int(42).java_display(), Ok("42".to_owned()));
        assert_eq!(
            OwnedReturn::Long(9001).java_display(),
            Ok("9001".to_owned())
        );
        assert_eq!(OwnedReturn::Float(1.5).java_display(), Ok("1.5".to_owned()));
        assert_eq!(
            OwnedReturn::Double(2.5).java_display(),
            Ok("2.5".to_owned())
        );
        assert_eq!(
            OwnedReturn::Object(None).java_display(),
            Ok("null".to_owned())
        );
    }

    #[test]
    fn extracts_java_return_values() {
        OwnedReturn::Void.into_void("void").unwrap();
        assert!(OwnedReturn::Boolean(true).into_boolean("boolean").unwrap());
        assert_eq!(OwnedReturn::Byte(-7).into_byte("byte").unwrap(), -7);
        assert_eq!(OwnedReturn::Char(65).into_char("char").unwrap(), 65);
        assert_eq!(OwnedReturn::Short(-300).into_short("short").unwrap(), -300);
        assert_eq!(OwnedReturn::Int(42).into_int("int").unwrap(), 42);
        assert_eq!(OwnedReturn::Long(9001).into_long("long").unwrap(), 9001);
        assert_eq!(OwnedReturn::Float(1.5).into_float("float").unwrap(), 1.5);
        assert_eq!(OwnedReturn::Double(2.5).into_double("double").unwrap(), 2.5);
        assert!(
            OwnedReturn::Object(None)
                .into_object("object")
                .unwrap()
                .is_none()
        );
        assert!(
            OwnedReturn::Object(None)
                .into_array("array")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reports_java_return_type_mismatches() {
        let error = OwnedReturn::Int(7).into_object("TestSubject.message");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.message",
                expected: "object",
                actual: "int".to_owned(),
            }
        );

        let error = OwnedReturn::Object(None).into_int("TestSubject.answer");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.answer",
                expected: "int",
                actual: "null".to_owned(),
            }
        );
    }

    #[test]
    fn reports_java_return_kind_names() {
        assert_eq!(OwnedReturn::Void.kind_name(), "void");
        assert_eq!(OwnedReturn::Boolean(true).kind_name(), "boolean");
        assert_eq!(OwnedReturn::Byte(-7).kind_name(), "byte");
        assert_eq!(OwnedReturn::Char(65).kind_name(), "char");
        assert_eq!(OwnedReturn::Short(-300).kind_name(), "short");
        assert_eq!(OwnedReturn::Int(42).kind_name(), "int");
        assert_eq!(OwnedReturn::Long(9001).kind_name(), "long");
        assert_eq!(OwnedReturn::Float(1.5).kind_name(), "float");
        assert_eq!(OwnedReturn::Double(2.5).kind_name(), "double");
        assert_eq!(OwnedReturn::Object(None).kind_name(), "null");
    }

    #[test]
    fn extracts_typed_java_returns() {
        let value: jni::jint =
            FromJavaReturn::from_java_return(OwnedReturn::Int(42), "typed int").unwrap();
        assert_eq!(value, 42);

        let value: Option<JavaObject> =
            FromJavaReturn::from_java_return(OwnedReturn::Object(None), "typed object").unwrap();
        assert!(value.is_none());

        let value: Option<String> =
            FromJavaReturn::from_java_return(OwnedReturn::Object(None), "typed string").unwrap();
        assert!(value.is_none());
    }

    #[test]
    fn rejects_null_for_required_typed_references() {
        assert_eq!(
            JavaObject::from_java_return(OwnedReturn::Object(None), "required object").unwrap_err(),
            Error::NullReturn {
                operation: "required object",
            }
        );
        assert_eq!(
            JavaArray::from_java_return(OwnedReturn::Object(None), "required array").unwrap_err(),
            Error::NullReturn {
                operation: "required array",
            }
        );
        assert_eq!(
            String::from_java_return(OwnedReturn::Object(None), "required string").unwrap_err(),
            Error::NullReturn {
                operation: "required string",
            }
        );
    }

    #[test]
    fn reports_typed_java_return_mismatches() {
        let error = bool::from_java_return(OwnedReturn::Int(7), "typed boolean");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "typed boolean",
                expected: "boolean",
                actual: "int".to_owned(),
            }
        );
    }
}
