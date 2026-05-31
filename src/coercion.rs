use crate::{jni, signature::JavaType, value::JavaValue};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum JavaValueCoercionError {
    Type { actual: &'static str },
    Value { actual: String },
}

pub(crate) fn coerce_java_value<R>(
    value: JavaValue<R>,
    expected: &JavaType,
) -> std::result::Result<JavaValue<R>, JavaValueCoercionError> {
    if value.matches_type(expected) {
        return Ok(value);
    }

    match (value, expected) {
        (JavaValue::Int(value), JavaType::Byte) => {
            narrow_int_value(value, i8::MIN as i32, i8::MAX as i32, "byte")
                .map(|value| JavaValue::Byte(value as jni::jbyte))
        }
        (JavaValue::Int(value), JavaType::Char) => {
            narrow_int_value(value, 0, u16::MAX as i32, "char")
                .map(|value| JavaValue::Char(value as jni::jchar))
        }
        (JavaValue::Int(value), JavaType::Short) => {
            narrow_int_value(value, i16::MIN as i32, i16::MAX as i32, "short")
                .map(|value| JavaValue::Short(value as jni::jshort))
        }
        (JavaValue::Int(value), JavaType::Long) => Ok(JavaValue::Long(value as jni::jlong)),
        (JavaValue::Float(value), JavaType::Double) => Ok(JavaValue::Double(value as jni::jdouble)),
        (JavaValue::Double(value), JavaType::Float) => {
            double_to_float_value(value).map(JavaValue::Float)
        }
        (value, _) => Err(JavaValueCoercionError::Type {
            actual: value.type_name(),
        }),
    }
}

pub(crate) fn can_coerce_java_value<R>(value: JavaValue<R>, expected: &JavaType) -> bool {
    coerce_java_value(value, expected).is_ok()
}

fn narrow_int_value(
    value: jni::jint,
    min: jni::jint,
    max: jni::jint,
    expected: &'static str,
) -> std::result::Result<jni::jint, JavaValueCoercionError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(JavaValueCoercionError::Value {
            actual: format!("int {value} outside {expected} range"),
        })
    }
}

fn double_to_float_value(
    value: jni::jdouble,
) -> std::result::Result<jni::jfloat, JavaValueCoercionError> {
    if value.is_finite() && value.abs() <= f32::MAX as f64 {
        Ok(value as jni::jfloat)
    } else {
        Err(JavaValueCoercionError::Value {
            actual: format!("double {value} is not finite or outside float range"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::RawJavaObject;

    type RawValue = JavaValue<RawJavaObject>;

    #[test]
    fn preserves_exact_descriptor_matches() {
        assert_eq!(
            coerce_java_value(RawValue::Boolean(true), &JavaType::Boolean).unwrap(),
            RawValue::Boolean(true)
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(7), &JavaType::Int).unwrap(),
            RawValue::Int(7)
        );
        assert_eq!(
            coerce_java_value(
                RawValue::NULL,
                &JavaType::Object("java/lang/Object".to_owned())
            )
            .unwrap(),
            RawValue::NULL
        );
    }

    #[test]
    fn coerces_descriptor_selected_numeric_values() {
        assert_eq!(
            coerce_java_value(RawValue::Int(7), &JavaType::Byte).unwrap(),
            RawValue::Byte(7)
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(65), &JavaType::Char).unwrap(),
            RawValue::Char(65)
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(-300), &JavaType::Short).unwrap(),
            RawValue::Short(-300)
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(7), &JavaType::Long).unwrap(),
            RawValue::Long(7)
        );
        assert_eq!(
            coerce_java_value(RawValue::Float(1.5), &JavaType::Double).unwrap(),
            RawValue::Double(1.5)
        );
        assert_eq!(
            coerce_java_value(RawValue::Double(2.5), &JavaType::Float).unwrap(),
            RawValue::Float(2.5)
        );
    }

    #[test]
    fn rejects_out_of_range_int_narrowing() {
        assert_eq!(
            coerce_java_value(RawValue::Int(128), &JavaType::Byte).unwrap_err(),
            JavaValueCoercionError::Value {
                actual: "int 128 outside byte range".to_owned(),
            }
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(-1), &JavaType::Char).unwrap_err(),
            JavaValueCoercionError::Value {
                actual: "int -1 outside char range".to_owned(),
            }
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(32768), &JavaType::Short).unwrap_err(),
            JavaValueCoercionError::Value {
                actual: "int 32768 outside short range".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_non_finite_and_out_of_range_double_to_float() {
        for value in [f64::MAX, f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(
                coerce_java_value(RawValue::Double(value), &JavaType::Float).unwrap_err(),
                JavaValueCoercionError::Value {
                    actual: format!("double {value} is not finite or outside float range"),
                }
            );
        }
    }

    #[test]
    fn reports_unsupported_coercions_as_type_errors() {
        assert_eq!(
            coerce_java_value(RawValue::Long(7), &JavaType::Int).unwrap_err(),
            JavaValueCoercionError::Type { actual: "long" }
        );
        assert_eq!(
            coerce_java_value(RawValue::Int(7), &JavaType::Float).unwrap_err(),
            JavaValueCoercionError::Type { actual: "int" }
        );
    }
}
