use super::*;
use crate::coercion::{JavaValueCoercionError, coerce_java_value};

pub(super) fn accepts_rust_string(expected: &JavaType) -> bool {
    matches!(
        expected,
        JavaType::Object(class) if class == "java/lang/String" || class == "java/lang/Object"
            || class == "java/lang/CharSequence"
    )
}

pub(super) fn coerce_java_call_value(
    value: JavaValue,
    expected: &JavaType,
    index: usize,
) -> Result<JavaValue> {
    coerce_java_value(value, expected).map_err(|error| match error {
        JavaValueCoercionError::Type { actual } => Error::InvalidArgumentType {
            index,
            expected: expected.to_string(),
            actual,
        },
        JavaValueCoercionError::Value { actual } => Error::InvalidArgumentValue {
            index,
            expected: expected.to_string(),
            actual,
        },
    })
}

pub(super) fn coerce_java_field_value(
    value: JavaValue,
    expected: &JavaType,
    operation: &'static str,
) -> Result<JavaValue> {
    coerce_java_value(value, expected).map_err(|error| match error {
        JavaValueCoercionError::Type { actual } => Error::InvalidFieldValueType {
            operation,
            expected: expected.to_string(),
            actual,
        },
        JavaValueCoercionError::Value { actual } => Error::InvalidFieldValue {
            operation,
            expected: expected.to_string(),
            actual,
        },
    })
}

pub(crate) fn can_coerce_java_value(value: JavaValue, expected: &JavaType) -> bool {
    crate::coercion::can_coerce_java_value(value, expected)
}
