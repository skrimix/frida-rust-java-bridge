use super::*;

impl JavaReturn {
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
        into_object, Object, Option<JavaObject>, "object";
        into_array, Array, Option<JavaArray>, "array";
    }
}

fn invalid_return(operation: &'static str, expected: &'static str, actual: JavaReturn) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: return_type_name(&actual).to_owned(),
    }
}

fn return_type_name(value: &JavaReturn) -> &'static str {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_java_return_values() {
        JavaReturn::Void.into_void("void").unwrap();
        assert!(JavaReturn::Boolean(true).into_boolean("boolean").unwrap());
        assert_eq!(JavaReturn::Byte(-7).into_byte("byte").unwrap(), -7);
        assert_eq!(JavaReturn::Char(65).into_char("char").unwrap(), 65);
        assert_eq!(JavaReturn::Short(-300).into_short("short").unwrap(), -300);
        assert_eq!(JavaReturn::Int(42).into_int("int").unwrap(), 42);
        assert_eq!(JavaReturn::Long(9001).into_long("long").unwrap(), 9001);
        assert_eq!(JavaReturn::Float(1.5).into_float("float").unwrap(), 1.5);
        assert_eq!(JavaReturn::Double(2.5).into_double("double").unwrap(), 2.5);
        assert!(
            JavaReturn::Object(None)
                .into_object("object")
                .unwrap()
                .is_none()
        );
        assert!(
            JavaReturn::Array(None)
                .into_array("array")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn reports_java_return_type_mismatches() {
        let error = JavaReturn::Int(7).into_object("TestSubject.message");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.message",
                expected: "object",
                actual: "int".to_owned(),
            }
        );

        let error = JavaReturn::Object(None).into_int("TestSubject.answer");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.answer",
                expected: "int",
                actual: "object".to_owned(),
            }
        );

        let error = JavaReturn::Array(None).into_object("TestSubject.staticIntArrayEcho");
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
        assert_eq!(JavaReturn::Void.kind_name(), "void");
        assert_eq!(JavaReturn::Boolean(true).kind_name(), "boolean");
        assert_eq!(JavaReturn::Byte(-7).kind_name(), "byte");
        assert_eq!(JavaReturn::Char(65).kind_name(), "char");
        assert_eq!(JavaReturn::Short(-300).kind_name(), "short");
        assert_eq!(JavaReturn::Int(42).kind_name(), "int");
        assert_eq!(JavaReturn::Long(9001).kind_name(), "long");
        assert_eq!(JavaReturn::Float(1.5).kind_name(), "float");
        assert_eq!(JavaReturn::Double(2.5).kind_name(), "double");
        assert_eq!(JavaReturn::Object(None).kind_name(), "object");
        assert_eq!(JavaReturn::Array(None).kind_name(), "array");
    }
}
