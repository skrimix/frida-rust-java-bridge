use super::{conversion::*, *};

impl<T> IntoJavaFieldValue for T
where
    T: Into<JavaValue>,
{
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        let value = self.into();
        let value = coerce_java_field_value(value, expected, operation)?;
        Ok(PreparedJavaFieldValue::new(value, None))
    }
}

impl IntoJavaFieldValue for &str {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_rust_string_field_value(self, env, expected, operation)
    }
}

impl IntoJavaFieldValue for String {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_rust_string_field_value(&self, env, expected, operation)
    }
}

impl IntoJavaFieldValue for &String {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_rust_string_field_value(self, env, expected, operation)
    }
}

impl<'local> IntoJavaFieldValue for &JavaLocalObject<'local> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_reference_field_value(self.as_jobject(), expected, operation)
    }
}

impl<'local> IntoJavaFieldValue for Option<&JavaLocalObject<'local>> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_reference_field_value(
            self.map_or(std::ptr::null_mut(), |value| value.as_jobject()),
            expected,
            operation,
        )
    }
}

impl<'local> IntoJavaFieldValue for &JavaLocalArray<'local> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_reference_field_value(self.as_jobject(), expected, operation)
    }
}

impl<'local> IntoJavaFieldValue for Option<&JavaLocalArray<'local>> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_reference_field_value(
            self.map_or(std::ptr::null_mut(), |value| value.as_jobject()),
            expected,
            operation,
        )
    }
}

impl<T> sealed::IntoJavaFieldValueSealed for T where T: Into<JavaValue> {}
impl sealed::IntoJavaFieldValueSealed for &str {}
impl sealed::IntoJavaFieldValueSealed for String {}
impl sealed::IntoJavaFieldValueSealed for &String {}
impl sealed::IntoJavaFieldValueSealed for &JavaLocalObject<'_> {}
impl sealed::IntoJavaFieldValueSealed for Option<&JavaLocalObject<'_>> {}
impl sealed::IntoJavaFieldValueSealed for &JavaLocalArray<'_> {}
impl sealed::IntoJavaFieldValueSealed for Option<&JavaLocalArray<'_>> {}

fn prepare_rust_string_field_value(
    value: &str,
    env: &Env<'_>,
    expected: &JavaType,
    operation: &'static str,
) -> Result<PreparedJavaFieldValue> {
    if !accepts_rust_string(expected) {
        return Err(Error::InvalidFieldValueType {
            operation,
            expected: expected.to_string(),
            actual: "string",
        });
    }

    // SAFETY: The local string reference is stored in `PreparedJavaFieldValue` and deleted after
    // the JNI field operation consuming it completes.
    let local_ref = unsafe { env.new_string_utf_raw(value)? };
    Ok(PreparedJavaFieldValue::new(
        JavaValue::object_ref(local_ref),
        Some(local_ref),
    ))
}

fn prepare_reference_field_value(
    object: jni::jobject,
    expected: &JavaType,
    operation: &'static str,
) -> Result<PreparedJavaFieldValue> {
    let value = if object.is_null() {
        JavaValue::NULL
    } else {
        JavaValue::object_ref(object)
    };
    let value = coerce_java_field_value(value, expected, operation)?;
    Ok(PreparedJavaFieldValue::new(value, None))
}
