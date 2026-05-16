use super::*;

pub(super) fn expect_int(value: JavaReturn, expected: i32, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Int(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("int {expected}"), other),
    }
}

pub(super) fn expect_bool(
    value: JavaReturn,
    expected: bool,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Boolean(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("boolean {expected}"), other),
    }
}

pub(super) fn expect_byte(
    value: JavaReturn,
    expected: jni::jbyte,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Byte(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("byte {expected}"), other),
    }
}

pub(super) fn expect_char(
    value: JavaReturn,
    expected: jni::jchar,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Char(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("char {expected}"), other),
    }
}

pub(super) fn expect_short(
    value: JavaReturn,
    expected: jni::jshort,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Short(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("short {expected}"), other),
    }
}

pub(super) fn expect_long(
    value: JavaReturn,
    expected: jni::jlong,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Long(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("long {expected}"), other),
    }
}

pub(super) fn expect_float(
    value: JavaReturn,
    expected: jni::jfloat,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Float(value) if (value - expected).abs() < 0.0001 => Ok(()),
        other => replacement_mismatch(operation, format!("float {expected}"), other),
    }
}

pub(super) fn expect_double(
    value: JavaReturn,
    expected: jni::jdouble,
    operation: &'static str,
) -> Result<()> {
    match value {
        JavaReturn::Double(value) if (value - expected).abs() < 0.0001 => Ok(()),
        other => replacement_mismatch(operation, format!("double {expected}"), other),
    }
}

pub(super) fn expect_string(
    value: JavaReturn,
    expected: Option<&str>,
    operation: &'static str,
) -> Result<()> {
    match (value, expected) {
        (JavaReturn::Object(None), None) => Ok(()),
        (JavaReturn::Object(Some(value)), Some(expected)) if value.get_string()? == expected => {
            Ok(())
        }
        (other, expected) => replacement_mismatch(operation, format!("string {expected:?}"), other),
    }
}

pub(super) fn expect_object_same(
    env: &Env<'_>,
    value: JavaReturn,
    expected: Option<jni::jobject>,
    operation: &'static str,
) -> Result<()> {
    match (value, expected) {
        (JavaReturn::Object(None), None) => Ok(()),
        (JavaReturn::Array(None), None) => Ok(()),
        (JavaReturn::Object(Some(value)), Some(expected)) => {
            let expected = RawObject(expected);
            if env.is_same_object(&value, &expected)? {
                Ok(())
            } else {
                replacement_mismatch(
                    operation,
                    "same object".to_owned(),
                    JavaReturn::Object(Some(value)),
                )
            }
        }
        (JavaReturn::Array(Some(value)), Some(expected)) => {
            let expected = RawObject(expected);
            if env.is_same_object(&value, &expected)? {
                Ok(())
            } else {
                replacement_mismatch(
                    operation,
                    "same object".to_owned(),
                    JavaReturn::Array(Some(value)),
                )
            }
        }
        (other, None) => replacement_mismatch(operation, "null object".to_owned(), other),
        (other, Some(_)) => replacement_mismatch(operation, "object".to_owned(), other),
    }
}

pub(super) fn read_int(value: JavaReturn, operation: &'static str) -> Result<i32> {
    match value {
        JavaReturn::Int(value) => Ok(value),
        other => test_error(format!("{operation} returned unexpected value {other:?}")),
    }
}

pub(super) fn read_object(
    value: JavaReturn,
    operation: &'static str,
) -> Result<Option<JavaObject>> {
    match value {
        JavaReturn::Object(value) => Ok(value),
        other => test_error(format!("{operation} returned unexpected value {other:?}")),
    }
}

pub(super) fn require_method<'a>(
    methods: &'a [JavaMethodMetadata],
    name: &str,
    kind: MethodKind,
    signature: &str,
    operation: &'static str,
) -> Result<&'a JavaMethodMetadata> {
    methods
        .iter()
        .find(|method| {
            method.name == name && method.kind == kind && method.signature.to_string() == signature
        })
        .ok_or_else(|| test_failure(format!("{operation} metadata was not found")))
}

pub(super) fn require_field<'a>(
    fields: &'a [JavaFieldMetadata],
    name: &str,
    kind: FieldKind,
    ty: &JavaType,
    operation: &'static str,
) -> Result<&'a JavaFieldMetadata> {
    fields
        .iter()
        .find(|field| field.name == name && field.kind == kind && &field.ty == ty)
        .ok_or_else(|| test_failure(format!("{operation} metadata was not found")))
}

pub(super) fn test_error<T>(reason: impl Into<String>) -> Result<T> {
    Err(test_failure(reason))
}

pub(super) fn test_failure(reason: impl Into<String>) -> Error {
    Error::UnsupportedFeature {
        feature: "app_process test",
        reason: reason.into(),
    }
}

pub(super) fn replacement_mismatch<T>(
    operation: &'static str,
    expected: String,
    actual: JavaReturn,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature: "ART method replacement",
        reason: format!("{operation} mismatch: expected {expected}, got {actual:?}"),
    })
}

pub(super) fn replacement_counter_mismatch<T>(
    operation: &'static str,
    expected: i32,
    actual: i32,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature: "ART method replacement",
        reason: format!("{operation} mismatch: expected counter {expected}, got {actual}"),
    })
}

pub(super) fn expect_clone_backend_summary(summary: &str) -> Result<()> {
    if summary.contains("backend=clone-active")
        && summary.contains("original_patched=")
        && summary.contains("clone_patched=")
    {
        return Ok(());
    }
    Err(Error::UnsupportedFeature {
        feature: "ART method replacement",
        reason: format!("replacement did not use cloned-method backend: {summary}"),
    })
}

pub(super) fn expect_replacement_clone_backend(
    replacement: &experimental::MethodReplacement,
    operation: &'static str,
) -> Result<()> {
    let Some(summary) = replacement.debug_summary() else {
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: format!("{operation} debug summary was unavailable"),
        });
    };
    expect_clone_backend_summary(&summary)
}

pub(super) fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

pub(super) fn new_raw_string(env: *mut jni::JNIEnv, text: &str) -> jni::jstring {
    let Some(env) = NonNull::new(env) else {
        return ptr::null_mut();
    };
    let Ok(runtime) = Runtime::obtain() else {
        return ptr::null_mut();
    };
    let vm = runtime.vm();
    let env = Env::from_raw(env, &vm);
    env.new_string_utf_raw(text).unwrap_or(ptr::null_mut())
}
