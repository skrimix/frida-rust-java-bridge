use super::*;
use crate::java::conversion::{
    prepare_field_reference, prepare_field_rust_string, prepare_field_value,
};

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
        prepare_field_value(self.into(), expected, operation).map(PreparedJavaFieldValue::from)
    }
}

impl IntoJavaFieldValue for &str {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_rust_string(self, env, expected, operation).map(PreparedJavaFieldValue::from)
    }
}

impl IntoJavaFieldValue for String {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_rust_string(&self, env, expected, operation).map(PreparedJavaFieldValue::from)
    }
}

impl IntoJavaFieldValue for &String {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_rust_string(self, env, expected, operation).map(PreparedJavaFieldValue::from)
    }
}

impl<'local> IntoJavaFieldValue for &JavaLocalObject<'local> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_reference(self.as_jobject(), expected, operation)
            .map(PreparedJavaFieldValue::from)
    }
}

impl<'local> IntoJavaFieldValue for Option<&JavaLocalObject<'local>> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_reference(
            self.map_or(std::ptr::null_mut(), |value| value.as_jobject()),
            expected,
            operation,
        )
        .map(PreparedJavaFieldValue::from)
    }
}

impl<'local> IntoJavaFieldValue for &JavaLocalArray<'local> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_reference(self.as_jobject(), expected, operation)
            .map(PreparedJavaFieldValue::from)
    }
}

impl<'local> IntoJavaFieldValue for Option<&JavaLocalArray<'local>> {
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_field_reference(
            self.map_or(std::ptr::null_mut(), |value| value.as_jobject()),
            expected,
            operation,
        )
        .map(PreparedJavaFieldValue::from)
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
