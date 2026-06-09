//! Java values passed through dynamic or raw call paths.
//!
//! Most high-level calls accept ordinary Rust values directly. Use [`JavaValue`] when you need to
//! build a dynamic argument list, inspect hook arguments, or cross an explicitly raw JNI boundary.

use crate::{jni, signature::JavaType};

/// Raw JNI object reference carried through an explicit raw value lane.
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
    /// `raw` must be null or a valid JNI local/global reference and must
    /// remain valid until the operation consuming it has completed.
    pub unsafe fn from_raw_jobject(raw: jni::jobject) -> Self {
        Self { raw }
    }

    pub(crate) fn as_jobject(self) -> jni::jobject {
        self.raw
    }

    /// Returns `true` when the wrapped raw JNI object handle is null.
    pub fn is_null(self) -> bool {
        self.raw.is_null()
    }

    /// Returns the wrapped raw JNI object reference.
    ///
    /// # Safety
    ///
    /// The caller must ensure the handle is only used on an attached
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

/// Java primitive, `void`, `null`, or nullable reference value.
///
/// The reference payload changes by context. Raw values use [`RawJavaObject`], high-level returns
/// use crate-owned wrappers, and hook arguments can use callback-local views.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum JavaValue<R = RawJavaObject> {
    Void,
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    /// Nullable Java reference value.
    ///
    /// With the default [`RawJavaObject`] payload this is the explicit raw JNI reference lane.
    /// Normal callers should prefer crate-owned wrappers, [`JavaValue::null`], or the explicit
    /// unsafe [`JavaValue::object_raw`] constructor.
    Object(Option<R>),
}

impl<R> JavaValue<R> {
    /// Constant form of [`JavaValue::Void`].
    pub const VOID: Self = Self::Void;
    /// Constant form of a Java null object reference.
    pub const NULL: Self = Self::Object(None);

    pub fn void() -> Self {
        Self::Void
    }

    pub fn null() -> Self {
        Self::NULL
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
        self.type_name()
    }

    /// Converts this value into `()` when it is Java `void`.
    ///
    /// Returns [`crate::Error::InvalidReturnType`] when the value is not `void`.
    pub fn into_void(self, operation: &'static str) -> crate::Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(crate::Error::InvalidReturnType {
                operation,
                expected: "void",
                actual: other.type_name().to_owned(),
            }),
        }
    }

    /// Converts this value into its nullable reference payload.
    ///
    /// Returns [`crate::Error::InvalidReturnType`] when the value is a primitive or `void`.
    pub fn into_reference(self, operation: &'static str) -> crate::Result<Option<R>> {
        match self {
            Self::Object(value) => Ok(value),
            other => Err(crate::Error::InvalidReturnType {
                operation,
                expected: "object",
                actual: other.type_name().to_owned(),
            }),
        }
    }

    pub(crate) fn matches_type(&self, expected: &JavaType) -> bool {
        match (self, expected) {
            (Self::Boolean(_), JavaType::Boolean)
            | (Self::Byte(_), JavaType::Byte)
            | (Self::Char(_), JavaType::Char)
            | (Self::Short(_), JavaType::Short)
            | (Self::Int(_), JavaType::Int)
            | (Self::Long(_), JavaType::Long)
            | (Self::Float(_), JavaType::Float)
            | (Self::Double(_), JavaType::Double) => true,
            (Self::Object(_), expected) => expected.is_reference(),
            (Self::Void, JavaType::Void) => true,
            _ => false,
        }
    }

    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Self::Void => "void",
            Self::Boolean(_) => "boolean",
            Self::Byte(_) => "byte",
            Self::Char(_) => "char",
            Self::Short(_) => "short",
            Self::Int(_) => "int",
            Self::Long(_) => "long",
            Self::Float(_) => "float",
            Self::Double(_) => "double",
            Self::Object(Some(_)) => "object",
            Self::Object(None) => "null",
        }
    }
}

impl JavaValue {
    pub(crate) fn object_ref(object: jni::jobject) -> Self {
        if object.is_null() {
            Self::NULL
        } else {
            Self::Object(Some(RawJavaObject::from_raw(object)))
        }
    }

    /// Builds a raw JNI reference argument.
    ///
    /// Passing a null raw handle produces [`JavaValue::null`]. Prefer that constructor when
    /// constructing Java null arguments outside raw JNI plumbing.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid local/global reference for the attached VM and must remain
    /// valid until the JNI call consuming the returned value has completed.
    pub unsafe fn object_raw(object: jni::jobject) -> Self {
        Self::object_ref(object)
    }

    /// Converts this value into the raw JNI union used for JNI method calls.
    ///
    /// For object values this preserves the raw reference lane. Java null is represented as a null
    /// `jobject`.
    pub fn to_jvalue(self) -> jni::jvalue {
        match self {
            Self::Void => jni::jvalue {
                l: std::ptr::null_mut(),
            },
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
            Self::Object(Some(value)) => jni::jvalue {
                l: value.as_jobject(),
            },
            Self::Object(None) => jni::jvalue {
                l: std::ptr::null_mut(),
            },
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
        assert!(JavaValue::<RawJavaObject>::Int(1).matches_type(&JavaType::Int));
        assert!(!JavaValue::<RawJavaObject>::Int(1).matches_type(&JavaType::Long));
        assert!(
            JavaValue::<RawJavaObject>::NULL
                .matches_type(&JavaType::Object("java/lang/String".to_owned()))
        );
        assert!(
            unsafe { JavaValue::object_raw(std::ptr::null_mut()) }
                .matches_type(&JavaType::Array(Box::new(JavaType::Int)))
        );
        assert_eq!(
            unsafe { JavaValue::object_raw(std::ptr::null_mut()) },
            JavaValue::<RawJavaObject>::NULL
        );
    }

    #[test]
    fn rejects_reference_values_for_primitive_types() {
        assert!(!JavaValue::<RawJavaObject>::NULL.matches_type(&JavaType::Int));
        assert!(
            !unsafe { JavaValue::object_raw(std::ptr::null_mut()) }
                .matches_type(&JavaType::Boolean)
        );
    }

    #[test]
    fn reports_value_type_names() {
        assert_eq!(
            JavaValue::<RawJavaObject>::Boolean(true).type_name(),
            "boolean"
        );
        assert_eq!(JavaValue::<RawJavaObject>::Byte(-1).type_name(), "byte");
        assert_eq!(JavaValue::<RawJavaObject>::Char(65).type_name(), "char");
        assert_eq!(JavaValue::<RawJavaObject>::Short(-2).type_name(), "short");
        assert_eq!(JavaValue::<RawJavaObject>::Int(3).type_name(), "int");
        assert_eq!(JavaValue::<RawJavaObject>::Long(4).type_name(), "long");
        assert_eq!(JavaValue::<RawJavaObject>::Float(1.5).type_name(), "float");
        assert_eq!(
            JavaValue::<RawJavaObject>::Double(2.5).type_name(),
            "double"
        );
        assert_eq!(
            unsafe { JavaValue::object_raw(std::ptr::null_mut()) }.type_name(),
            "null"
        );
        assert_eq!(JavaValue::<RawJavaObject>::NULL.type_name(), "null");
    }

    #[test]
    fn marshals_values_to_jni_union_slots() {
        let object = std::ptr::dangling_mut();

        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Boolean(true).to_jvalue().z },
            jni::JNI_TRUE
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Boolean(false).to_jvalue().z },
            jni::JNI_FALSE
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Byte(-7).to_jvalue().b },
            -7
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Char(65).to_jvalue().c },
            65
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Short(-9).to_jvalue().s },
            -9
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Int(11).to_jvalue().i },
            11
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Long(13).to_jvalue().j },
            13
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Float(1.25).to_jvalue().f },
            1.25
        );
        assert_eq!(
            unsafe { JavaValue::<RawJavaObject>::Double(2.5).to_jvalue().d },
            2.5
        );
        assert_eq!(
            unsafe { JavaValue::object_raw(object).to_jvalue().l },
            object
        );
        assert!(unsafe { JavaValue::<RawJavaObject>::NULL.to_jvalue().l }.is_null());
    }
}
