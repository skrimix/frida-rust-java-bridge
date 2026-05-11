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
}
