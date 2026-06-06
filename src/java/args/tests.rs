use super::call::JavaCallArg;
use super::conversion::{accepts_rust_string, coerce_java_call_value, coerce_java_field_value};
use super::*;
use static_assertions::{assert_impl_all, assert_not_impl_any};

assert_not_impl_any!(&'static JavaLocalObject<'static>: Into<JavaValue>);
assert_not_impl_any!(Option<&'static JavaLocalObject<'static>>: Into<JavaValue>);
assert_not_impl_any!(&'static JavaLocalArray<'static>: Into<JavaValue>);
assert_not_impl_any!(Option<&'static JavaLocalArray<'static>>: Into<JavaValue>);
assert_impl_all!(&'static JavaLocalObject<'static>: JavaCallArg);
assert_impl_all!(Option<&'static JavaLocalObject<'static>>: JavaCallArg);
assert_impl_all!(&'static JavaLocalArray<'static>: JavaCallArg);
assert_impl_all!(Option<&'static JavaLocalArray<'static>>: JavaCallArg);

#[test]
fn converts_common_java_argument_containers() {
    assert_eq!(().into_java_args(), Vec::<JavaValue>::new());

    let values = [JavaValue::Int(7), JavaValue::NULL];
    assert_eq!(
        values.into_java_args(),
        vec![JavaValue::Int(7), JavaValue::NULL]
    );
    assert_eq!(
        (&values).into_java_args(),
        vec![JavaValue::Int(7), JavaValue::NULL]
    );

    let slice: &[JavaValue] = &values;
    assert_eq!(
        slice.into_java_args(),
        vec![JavaValue::Int(7), JavaValue::NULL]
    );

    assert_eq!(
        vec![JavaValue::Boolean(true)].into_java_args(),
        vec![JavaValue::Boolean(true)]
    );
}

#[test]
fn converts_explicit_java_args_container() {
    let mut args = JavaArgs::with_capacity(2);
    args.push(7 as jni::jint);
    args.push(JavaValue::NULL);

    assert_eq!(args.len(), 2);
    assert!(!args.is_empty());
    assert_eq!(args.as_slice(), &[JavaValue::Int(7), JavaValue::NULL]);
    assert_eq!(
        (&args).into_java_args(),
        vec![JavaValue::Int(7), JavaValue::NULL]
    );
    assert_eq!(
        args.into_java_args(),
        vec![JavaValue::Int(7), JavaValue::NULL]
    );
}

#[test]
fn java_args_macro_builds_long_mixed_lists() {
    let args = crate::java_args![
        1 as jni::jint,
        2 as jni::jint,
        3 as jni::jint,
        4 as jni::jint,
        5 as jni::jint,
        6 as jni::jint,
        7 as jni::jint,
        8 as jni::jint,
        9 as jni::jint,
        true,
        JavaValue::NULL,
    ];

    assert_eq!(args.len(), 11);
    assert_eq!(
        args.into_java_args(),
        vec![
            JavaValue::Int(1),
            JavaValue::Int(2),
            JavaValue::Int(3),
            JavaValue::Int(4),
            JavaValue::Int(5),
            JavaValue::Int(6),
            JavaValue::Int(7),
            JavaValue::Int(8),
            JavaValue::Int(9),
            JavaValue::Boolean(true),
            JavaValue::NULL,
        ]
    );
}

#[test]
fn converts_tuple_java_arguments() {
    assert_eq!(
        (7 as jni::jint, true, JavaValue::NULL).into_java_args(),
        vec![JavaValue::Int(7), JavaValue::Boolean(true), JavaValue::NULL]
    );
}

#[test]
fn converts_bare_single_java_argument() {
    assert_eq!((7 as jni::jint).into_java_args(), vec![JavaValue::Int(7)]);
    assert_eq!(JavaValue::NULL.into_java_args(), vec![JavaValue::NULL]);
}

#[test]
fn converts_optional_java_object_arguments() {
    assert_eq!(JavaValue::from(None::<&JavaObject>), JavaValue::NULL);
    assert_eq!(
        (None::<&JavaObject>,).into_java_args(),
        vec![JavaValue::NULL]
    );
}

#[test]
fn recognizes_rust_string_argument_targets() {
    assert!(accepts_rust_string(&JavaType::Object(
        "java/lang/String".to_owned()
    )));
    assert!(accepts_rust_string(&JavaType::Object(
        "java/lang/Object".to_owned()
    )));
    assert!(accepts_rust_string(&JavaType::Object(
        "java/lang/CharSequence".to_owned()
    )));
    assert!(!accepts_rust_string(&JavaType::Object(
        "java/lang/StringBuilder".to_owned()
    )));
    assert!(!accepts_rust_string(&JavaType::Int));
    assert!(!accepts_rust_string(&JavaType::Array(Box::new(
        JavaType::Object("java/lang/String".to_owned())
    ))));
}

#[test]
fn coerces_descriptor_selected_numeric_arguments_conservatively() {
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(7), &JavaType::Byte, 0).unwrap(),
        JavaValue::Byte(7)
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(65), &JavaType::Char, 0).unwrap(),
        JavaValue::Char(65)
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(-300), &JavaType::Short, 0).unwrap(),
        JavaValue::Short(-300)
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(7), &JavaType::Long, 0).unwrap(),
        JavaValue::Long(7)
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Float(1.5), &JavaType::Double, 0).unwrap(),
        JavaValue::Double(1.5)
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Double(2.5), &JavaType::Float, 0).unwrap(),
        JavaValue::Float(2.5)
    );
}

#[test]
fn rejects_out_of_range_numeric_argument_coercions() {
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(128), &JavaType::Byte, 1).unwrap_err(),
        Error::InvalidArgumentValue {
            index: 1,
            expected: "B".to_owned(),
            actual: "int 128 outside byte range".to_owned(),
        }
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(-1), &JavaType::Char, 2).unwrap_err(),
        Error::InvalidArgumentValue {
            index: 2,
            expected: "C".to_owned(),
            actual: "int -1 outside char range".to_owned(),
        }
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Double(f64::MAX), &JavaType::Float, 3).unwrap_err(),
        Error::InvalidArgumentValue {
            index: 3,
            expected: "F".to_owned(),
            actual: format!("double {} is not finite or outside float range", f64::MAX),
        }
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Double(f64::INFINITY), &JavaType::Float, 4).unwrap_err(),
        Error::InvalidArgumentValue {
            index: 4,
            expected: "F".to_owned(),
            actual: "double inf is not finite or outside float range".to_owned(),
        }
    );
}

#[test]
fn preserves_exact_type_rejection_for_unsupported_coercions() {
    assert_eq!(
        coerce_java_call_value(JavaValue::Long(7), &JavaType::Int, 0).unwrap_err(),
        Error::InvalidArgumentType {
            index: 0,
            expected: "I".to_owned(),
            actual: "long",
        }
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Boolean(true), &JavaType::Int, 0).unwrap_err(),
        Error::InvalidArgumentType {
            index: 0,
            expected: "I".to_owned(),
            actual: "boolean",
        }
    );
    assert_eq!(
        coerce_java_call_value(JavaValue::Int(7), &JavaType::Float, 0).unwrap_err(),
        Error::InvalidArgumentType {
            index: 0,
            expected: "F".to_owned(),
            actual: "int",
        }
    );
}

#[test]
fn reports_out_of_range_numeric_field_values() {
    assert_eq!(
        coerce_java_field_value(JavaValue::Int(32768), &JavaType::Short, "field").unwrap_err(),
        Error::InvalidFieldValue {
            operation: "field",
            expected: "S".to_owned(),
            actual: "int 32768 outside short range".to_owned(),
        }
    );
}
