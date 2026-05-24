use crate::{jni, signature::JavaType};

/// A raw JNI object reference carried through an explicitly raw Java value lane.
///
/// This wrapper is intentionally not directly constructible from safe code. Use crate-owned object
/// wrappers for normal calls, or [`JavaValue::object_raw`] when crossing a low-level JNI boundary.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct RawJavaObject {
    raw: jni::jobject,
}

impl RawJavaObject {
    pub(crate) fn from_raw(raw: jni::jobject) -> Self {
        Self { raw }
    }

    /// Wraps a raw JNI object reference for explicit raw value plumbing.
    ///
    /// # Safety
    ///
    /// `raw` must be null or a valid JNI local/global reference for the intended VM and must
    /// remain valid until the operation consuming it has completed.
    pub unsafe fn from_raw_jobject(raw: jni::jobject) -> Self {
        Self { raw }
    }

    pub(crate) fn as_jobject(self) -> jni::jobject {
        self.raw
    }

    pub fn is_null(self) -> bool {
        self.raw.is_null()
    }

    /// Returns the wrapped raw JNI object reference.
    ///
    /// # Safety
    ///
    /// The caller must ensure the handle is only used with the VM it came from, on an attached
    /// thread, and within the lifetime of the underlying local/global reference.
    pub unsafe fn raw_jobject(self) -> jni::jobject {
        self.raw
    }
}

impl std::fmt::Debug for RawJavaObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("RawJavaObject").field(&self.raw).finish()
    }
}

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
    /// Raw JNI reference argument.
    ///
    /// Prefer passing crate-owned Java wrappers to high-level APIs. Constructing this variant
    /// directly is a low-level escape hatch: the type system cannot prove VM identity, reference
    /// lifetime, or object kind. The variant stays structurally public so crate-internal raw
    /// plumbing can share the same enum representation across module boundaries; normal callers
    /// should use safe wrappers, [`JavaValue::Null`], or [`JavaValue::object_raw`].
    #[doc(hidden)]
    Object(RawJavaObject),
    Null,
}

impl JavaValue {
    pub(crate) fn object_ref(object: jni::jobject) -> Self {
        Self::Object(RawJavaObject::from_raw(object))
    }

    /// Builds a raw JNI reference argument.
    ///
    /// Passing a null raw handle produces [`JavaValue::Null`]. Prefer that variant directly when
    /// constructing Java null arguments outside raw JNI plumbing.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid local/global reference for the attached VM and must remain
    /// valid until the JNI call consuming the returned value has completed.
    pub unsafe fn object_raw(object: jni::jobject) -> Self {
        if object.is_null() {
            Self::Null
        } else {
            Self::Object(RawJavaObject::from_raw(object))
        }
    }

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
            Self::Object(value) => jni::jvalue {
                l: value.as_jobject(),
            },
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
            unsafe { JavaValue::object_raw(std::ptr::null_mut()) }
                .matches_type(&JavaType::Array(Box::new(JavaType::Int)))
        );
        assert_eq!(
            unsafe { JavaValue::object_raw(std::ptr::null_mut()) },
            JavaValue::Null
        );
    }

    #[test]
    fn rejects_reference_values_for_primitive_types() {
        assert!(!JavaValue::Null.matches_type(&JavaType::Int));
        assert!(
            !unsafe { JavaValue::object_raw(std::ptr::null_mut()) }
                .matches_type(&JavaType::Boolean)
        );
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
            unsafe { JavaValue::object_raw(std::ptr::null_mut()) }.type_name(),
            "null"
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
        assert_eq!(
            unsafe { JavaValue::object_raw(object).to_jvalue().l },
            object
        );
        assert!(unsafe { JavaValue::Null.to_jvalue().l }.is_null());
    }
}
