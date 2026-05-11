use crate::{jni, signature::JavaType};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JavaValue {
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(jni::jobject),
    Null,
}

impl JavaValue {
    pub fn to_jvalue(self) -> jni::jvalue {
        match self {
            Self::Boolean(value) => jni::jvalue {
                z: if value { jni::JNI_TRUE } else { jni::JNI_FALSE },
            },
            Self::Byte(value) => jni::jvalue { b: value },
            Self::Char(value) => jni::jvalue { c: value },
            Self::Short(value) => jni::jvalue { s: value },
            Self::Int(value) => jni::jvalue { i: value },
            Self::Long(value) => jni::jvalue { j: value },
            Self::Float(value) => jni::jvalue { f: value },
            Self::Double(value) => jni::jvalue { d: value },
            Self::Object(value) => jni::jvalue { l: value },
            Self::Null => jni::jvalue {
                l: std::ptr::null_mut(),
            },
        }
    }

    pub(crate) fn matches_type(self, expected: &JavaType) -> bool {
        match (self, expected) {
            (Self::Boolean(_), JavaType::Boolean)
            | (Self::Byte(_), JavaType::Byte)
            | (Self::Char(_), JavaType::Char)
            | (Self::Short(_), JavaType::Short)
            | (Self::Int(_), JavaType::Int)
            | (Self::Long(_), JavaType::Long)
            | (Self::Float(_), JavaType::Float)
            | (Self::Double(_), JavaType::Double) => true,
            (Self::Object(_), expected) | (Self::Null, expected) => expected.is_reference(),
            _ => false,
        }
    }

    pub(crate) fn type_name(self) -> &'static str {
        match self {
            Self::Boolean(_) => "boolean",
            Self::Byte(_) => "byte",
            Self::Char(_) => "char",
            Self::Short(_) => "short",
            Self::Int(_) => "int",
            Self::Long(_) => "long",
            Self::Float(_) => "float",
            Self::Double(_) => "double",
            Self::Object(_) => "object",
            Self::Null => "null",
        }
    }
}

impl From<bool> for JavaValue {
    fn from(value: bool) -> Self {
        Self::Boolean(value)
    }
}

impl From<jni::jbyte> for JavaValue {
    fn from(value: jni::jbyte) -> Self {
        Self::Byte(value)
    }
}

impl From<jni::jchar> for JavaValue {
    fn from(value: jni::jchar) -> Self {
        Self::Char(value)
    }
}

impl From<jni::jshort> for JavaValue {
    fn from(value: jni::jshort) -> Self {
        Self::Short(value)
    }
}

impl From<jni::jint> for JavaValue {
    fn from(value: jni::jint) -> Self {
        Self::Int(value)
    }
}

impl From<jni::jlong> for JavaValue {
    fn from(value: jni::jlong) -> Self {
        Self::Long(value)
    }
}

impl From<jni::jfloat> for JavaValue {
    fn from(value: jni::jfloat) -> Self {
        Self::Float(value)
    }
}

impl From<jni::jdouble> for JavaValue {
    fn from(value: jni::jdouble) -> Self {
        Self::Double(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_argument_kinds() {
        assert!(JavaValue::Int(1).matches_type(&JavaType::Int));
        assert!(!JavaValue::Int(1).matches_type(&JavaType::Long));
        assert!(JavaValue::Null.matches_type(&JavaType::Object("java/lang/String".to_owned())));
        assert!(
            JavaValue::Object(std::ptr::null_mut())
                .matches_type(&JavaType::Array(Box::new(JavaType::Int)))
        );
    }

    #[test]
    fn rejects_reference_values_for_primitive_types() {
        assert!(!JavaValue::Null.matches_type(&JavaType::Int));
        assert!(!JavaValue::Object(std::ptr::null_mut()).matches_type(&JavaType::Boolean));
    }

    #[test]
    fn reports_value_type_names() {
        assert_eq!(JavaValue::Boolean(true).type_name(), "boolean");
        assert_eq!(JavaValue::Byte(-1).type_name(), "byte");
        assert_eq!(JavaValue::Char(65).type_name(), "char");
        assert_eq!(JavaValue::Short(-2).type_name(), "short");
        assert_eq!(JavaValue::Int(3).type_name(), "int");
        assert_eq!(JavaValue::Long(4).type_name(), "long");
        assert_eq!(JavaValue::Float(1.5).type_name(), "float");
        assert_eq!(JavaValue::Double(2.5).type_name(), "double");
        assert_eq!(
            JavaValue::Object(std::ptr::null_mut()).type_name(),
            "object"
        );
        assert_eq!(JavaValue::Null.type_name(), "null");
    }

    #[test]
    fn marshals_values_to_jni_union_slots() {
        let object = std::ptr::dangling_mut();

        assert_eq!(
            unsafe { JavaValue::Boolean(true).to_jvalue().z },
            jni::JNI_TRUE
        );
        assert_eq!(
            unsafe { JavaValue::Boolean(false).to_jvalue().z },
            jni::JNI_FALSE
        );
        assert_eq!(unsafe { JavaValue::Byte(-7).to_jvalue().b }, -7);
        assert_eq!(unsafe { JavaValue::Char(65).to_jvalue().c }, 65);
        assert_eq!(unsafe { JavaValue::Short(-9).to_jvalue().s }, -9);
        assert_eq!(unsafe { JavaValue::Int(11).to_jvalue().i }, 11);
        assert_eq!(unsafe { JavaValue::Long(13).to_jvalue().j }, 13);
        assert_eq!(unsafe { JavaValue::Float(1.25).to_jvalue().f }, 1.25);
        assert_eq!(unsafe { JavaValue::Double(2.5).to_jvalue().d }, 2.5);
        assert_eq!(unsafe { JavaValue::Object(object).to_jvalue().l }, object);
        assert!(unsafe { JavaValue::Null.to_jvalue().l }.is_null());
    }
}
