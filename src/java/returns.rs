use super::*;

impl<O, A> JavaReturn<O, A> {
    pub const VOID: Self = Self::Void;

    pub fn void() -> Self {
        Self::Void
    }

    pub fn boolean(value: bool) -> Self {
        Self::Boolean(value)
    }

    pub fn byte(value: jni::jbyte) -> Self {
        Self::Byte(value)
    }

    pub fn char(value: jni::jchar) -> Self {
        Self::Char(value)
    }

    pub fn short(value: jni::jshort) -> Self {
        Self::Short(value)
    }

    pub fn int(value: jni::jint) -> Self {
        Self::Int(value)
    }

    pub fn long(value: jni::jlong) -> Self {
        Self::Long(value)
    }

    pub fn float(value: jni::jfloat) -> Self {
        Self::Float(value)
    }

    pub fn double(value: jni::jdouble) -> Self {
        Self::Double(value)
    }

    pub fn kind_name(&self) -> &'static str {
        return_type_name(self)
    }

    pub fn into_void(self, operation: &'static str) -> Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(invalid_return(operation, "void", other)),
        }
    }

    java_return_extractors! {
        into_boolean, Boolean, bool, "boolean";
        into_byte, Byte, jni::jbyte, "byte";
        into_char, Char, jni::jchar, "char";
        into_short, Short, jni::jshort, "short";
        into_int, Int, jni::jint, "int";
        into_long, Long, jni::jlong, "long";
        into_float, Float, jni::jfloat, "float";
        into_double, Double, jni::jdouble, "double";
        into_object, Object, Option<O>, "object";
        into_array, Array, Option<A>, "array";
    }
}

impl FromJavaReturn for JavaReturn<JavaObject, JavaArray> {
    fn from_java_return(value: JavaReturn, _operation: &'static str) -> Result<Self> {
        Ok(value)
    }
}

impl JavaReturn<JavaObject, JavaArray> {
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
            Self::Object(Some(value)) => value.java_display()?,
            Self::Object(None) | Self::Array(None) => "null".to_owned(),
            Self::Array(Some(value)) => value.java_display()?,
        })
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

fn invalid_return<O, A>(
    operation: &'static str,
    expected: &'static str,
    actual: JavaReturn<O, A>,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: return_type_name(&actual).to_owned(),
    }
}

fn return_type_name<O, A>(value: &JavaReturn<O, A>) -> &'static str {
    match value {
        JavaReturn::Void => "void",
        JavaReturn::Boolean(_) => "boolean",
        JavaReturn::Byte(_) => "byte",
        JavaReturn::Char(_) => "char",
        JavaReturn::Short(_) => "short",
        JavaReturn::Int(_) => "int",
        JavaReturn::Long(_) => "long",
        JavaReturn::Float(_) => "float",
        JavaReturn::Double(_) => "double",
        JavaReturn::Object(_) => "object",
        JavaReturn::Array(_) => "array",
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

    type OwnedReturn = JavaReturn<JavaObject, JavaArray>;

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
        assert_eq!(
            OwnedReturn::Array(None).java_display(),
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
            OwnedReturn::Array(None)
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
                actual: "object".to_owned(),
            }
        );

        let error = OwnedReturn::Array(None).into_object("TestSubject.staticIntArrayEcho");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.staticIntArrayEcho",
                expected: "object",
                actual: "array".to_owned(),
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
        assert_eq!(OwnedReturn::Object(None).kind_name(), "object");
        assert_eq!(OwnedReturn::Array(None).kind_name(), "array");
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
            JavaArray::from_java_return(OwnedReturn::Array(None), "required array").unwrap_err(),
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
