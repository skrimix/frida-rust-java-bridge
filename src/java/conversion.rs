use crate::{
    Error, Result,
    coercion::{JavaValueCoercionError, coerce_java_value},
    env::Env,
    jni,
    signature::JavaType,
    value::JavaValue,
};

pub(crate) struct PreparedJavaValue {
    value: JavaValue,
    local_ref: Option<jni::jobject>,
}

impl PreparedJavaValue {
    fn new(value: JavaValue, local_ref: Option<jni::jobject>) -> Self {
        Self { value, local_ref }
    }

    pub(crate) fn into_parts(self) -> (JavaValue, Option<jni::jobject>) {
        (self.value, self.local_ref)
    }
}

pub(crate) fn accepts_rust_string(expected: &JavaType) -> bool {
    matches!(
        expected,
        JavaType::Object(class) if class == "java/lang/String" || class == "java/lang/Object"
            || class == "java/lang/CharSequence"
    )
}

pub(crate) fn prepare_call_value(
    value: JavaValue,
    expected: &JavaType,
    index: usize,
) -> Result<PreparedJavaValue> {
    let value = coerce_java_call_value(value, expected, index)?;
    Ok(PreparedJavaValue::new(value, None))
}

pub(crate) fn prepare_call_reference(
    object: jni::jobject,
    expected: &JavaType,
    index: usize,
) -> Result<PreparedJavaValue> {
    prepare_call_value(java_value_from_raw_object(object), expected, index)
}

pub(crate) fn prepare_call_rust_string(
    value: &str,
    env: &Env<'_>,
    expected: &JavaType,
    index: usize,
) -> Result<PreparedJavaValue> {
    if !accepts_rust_string(expected) {
        return Err(Error::InvalidArgumentType {
            index,
            expected: expected.to_string(),
            actual: "string",
        });
    }

    // SAFETY: The prepared value owns the local ref until the JNI call consumes it.
    let local_ref = unsafe { env.new_string_utf_raw(value)? };
    Ok(PreparedJavaValue::new(
        JavaValue::object_ref(local_ref),
        Some(local_ref),
    ))
}

pub(crate) fn prepare_field_value(
    value: JavaValue,
    expected: &JavaType,
    operation: &'static str,
) -> Result<PreparedJavaValue> {
    let value = coerce_java_field_value(value, expected, operation)?;
    Ok(PreparedJavaValue::new(value, None))
}

pub(crate) fn prepare_field_reference(
    object: jni::jobject,
    expected: &JavaType,
    operation: &'static str,
) -> Result<PreparedJavaValue> {
    prepare_field_value(java_value_from_raw_object(object), expected, operation)
}

pub(crate) fn prepare_field_rust_string(
    value: &str,
    env: &Env<'_>,
    expected: &JavaType,
    operation: &'static str,
) -> Result<PreparedJavaValue> {
    if !accepts_rust_string(expected) {
        return Err(Error::InvalidFieldValueType {
            operation,
            expected: expected.to_string(),
            actual: "string",
        });
    }

    // SAFETY: The prepared value owns the local ref until the JNI field operation consumes it.
    let local_ref = unsafe { env.new_string_utf_raw(value)? };
    Ok(PreparedJavaValue::new(
        JavaValue::object_ref(local_ref),
        Some(local_ref),
    ))
}

pub(crate) fn coerce_java_call_value(
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

pub(crate) fn coerce_java_field_value(
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

pub(crate) fn coerce_java_return_value<R>(
    value: JavaValue<R>,
    expected: &JavaType,
    operation: &'static str,
) -> Result<JavaValue<R>> {
    coerce_java_value(value, expected).map_err(|error| match error {
        JavaValueCoercionError::Type { actual } => Error::InvalidReturnType {
            operation,
            expected: expected.jni_return_name(),
            actual: actual.to_owned(),
        },
        JavaValueCoercionError::Value { actual } => Error::InvalidReturnType {
            operation,
            expected: expected.jni_return_name(),
            actual,
        },
    })
}

pub(crate) fn can_coerce_java_value(value: JavaValue, expected: &JavaType) -> bool {
    crate::coercion::can_coerce_java_value(value, expected)
}

pub(crate) fn java_value_from_raw_object(object: jni::jobject) -> JavaValue {
    JavaValue::object_ref(object)
}
