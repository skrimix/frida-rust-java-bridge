use super::*;

pub(super) struct RawObject(pub(super) jni::jobject);

pub(super) fn call_instance_return(
    env: &Env<'_>,
    object: &(impl AsJObject + ?Sized),
    method: &MethodId,
    args: &[JavaValue],
) -> Result<JavaReturn> {
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            env.call_instance_void_method(object, method, args)?;
            JavaReturn::Void
        }
        JavaType::Boolean => {
            JavaReturn::Boolean(env.call_instance_boolean_method(object, method, args)?)
        }
        JavaType::Byte => JavaReturn::Byte(env.call_instance_byte_method(object, method, args)?),
        JavaType::Char => JavaReturn::Char(env.call_instance_char_method(object, method, args)?),
        JavaType::Short => JavaReturn::Short(env.call_instance_short_method(object, method, args)?),
        JavaType::Int => JavaReturn::Int(env.call_instance_int_method(object, method, args)?),
        JavaType::Long => JavaReturn::Long(env.call_instance_long_method(object, method, args)?),
        JavaType::Float => JavaReturn::Float(env.call_instance_float_method(object, method, args)?),
        JavaType::Double => {
            JavaReturn::Double(env.call_instance_double_method(object, method, args)?)
        }
        JavaType::Object(_) => JavaReturn::Object(
            env.call_instance_object_method(object, method, args)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            env.call_instance_object_method(object, method, args)?
                .map(|object| array_from_ref(env, env.vm(), &object, (**element).clone()))
                .transpose()?,
        ),
    })
}

pub(super) fn call_static_return(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    method: &MethodId,
    args: &[JavaValue],
) -> Result<JavaReturn> {
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            env.call_static_void_method(class, method, args)?;
            JavaReturn::Void
        }
        JavaType::Boolean => {
            JavaReturn::Boolean(env.call_static_boolean_method(class, method, args)?)
        }
        JavaType::Byte => JavaReturn::Byte(env.call_static_byte_method(class, method, args)?),
        JavaType::Char => JavaReturn::Char(env.call_static_char_method(class, method, args)?),
        JavaType::Short => JavaReturn::Short(env.call_static_short_method(class, method, args)?),
        JavaType::Int => JavaReturn::Int(env.call_static_int_method(class, method, args)?),
        JavaType::Long => JavaReturn::Long(env.call_static_long_method(class, method, args)?),
        JavaType::Float => JavaReturn::Float(env.call_static_float_method(class, method, args)?),
        JavaType::Double => JavaReturn::Double(env.call_static_double_method(class, method, args)?),
        JavaType::Object(_) => JavaReturn::Object(
            env.call_static_object_method(class, method, args)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            env.call_static_object_method(class, method, args)?
                .map(|object| array_from_ref(env, env.vm(), &object, (**element).clone()))
                .transpose()?,
        ),
    })
}

pub(super) fn get_instance_field(
    env: &Env<'_>,
    object: &(impl AsJObject + ?Sized),
    field: &FieldId,
) -> Result<JavaReturn> {
    Ok(match field.ty() {
        JavaType::Boolean => JavaReturn::Boolean(env.get_instance_boolean_field(object, field)?),
        JavaType::Byte => JavaReturn::Byte(env.get_instance_byte_field(object, field)?),
        JavaType::Char => JavaReturn::Char(env.get_instance_char_field(object, field)?),
        JavaType::Short => JavaReturn::Short(env.get_instance_short_field(object, field)?),
        JavaType::Int => JavaReturn::Int(env.get_instance_int_field(object, field)?),
        JavaType::Long => JavaReturn::Long(env.get_instance_long_field(object, field)?),
        JavaType::Float => JavaReturn::Float(env.get_instance_float_field(object, field)?),
        JavaType::Double => JavaReturn::Double(env.get_instance_double_field(object, field)?),
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "RawJavaClass::get_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(_) => JavaReturn::Object(
            env.get_instance_object_field(object, field)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            env.get_instance_object_field(object, field)?
                .map(|object| array_from_ref(env, env.vm(), &object, (**element).clone()))
                .transpose()?,
        ),
    })
}

pub(super) fn set_instance_field(
    env: &Env<'_>,
    object: &(impl AsJObject + ?Sized),
    field: &FieldId,
    value: JavaValue,
) -> Result<()> {
    validate_field_value(field, value)?;
    match value {
        JavaValue::Boolean(value) => env.set_instance_boolean_field(object, field, value),
        JavaValue::Byte(value) => env.set_instance_byte_field(object, field, value),
        JavaValue::Char(value) => env.set_instance_char_field(object, field, value),
        JavaValue::Short(value) => env.set_instance_short_field(object, field, value),
        JavaValue::Int(value) => env.set_instance_int_field(object, field, value),
        JavaValue::Long(value) => env.set_instance_long_field(object, field, value),
        JavaValue::Float(value) => env.set_instance_float_field(object, field, value),
        JavaValue::Double(value) => env.set_instance_double_field(object, field, value),
        JavaValue::Object(value) if !value.is_null() => {
            let value = RawObject(value);
            env.set_instance_object_field(object, field, Some(&value))
        }
        JavaValue::Object(_) | JavaValue::Null => {
            env.set_instance_object_field(object, field, None)
        }
    }
}

pub(super) fn get_static_field(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    field: &FieldId,
) -> Result<JavaReturn> {
    Ok(match field.ty() {
        JavaType::Boolean => JavaReturn::Boolean(env.get_static_boolean_field(class, field)?),
        JavaType::Byte => JavaReturn::Byte(env.get_static_byte_field(class, field)?),
        JavaType::Char => JavaReturn::Char(env.get_static_char_field(class, field)?),
        JavaType::Short => JavaReturn::Short(env.get_static_short_field(class, field)?),
        JavaType::Int => JavaReturn::Int(env.get_static_int_field(class, field)?),
        JavaType::Long => JavaReturn::Long(env.get_static_long_field(class, field)?),
        JavaType::Float => JavaReturn::Float(env.get_static_float_field(class, field)?),
        JavaType::Double => JavaReturn::Double(env.get_static_double_field(class, field)?),
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "RawJavaClass::get_static_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(_) => JavaReturn::Object(
            env.get_static_object_field(class, field)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            env.get_static_object_field(class, field)?
                .map(|object| array_from_ref(env, env.vm(), &object, (**element).clone()))
                .transpose()?,
        ),
    })
}

pub(super) fn set_static_field(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    field: &FieldId,
    value: JavaValue,
) -> Result<()> {
    validate_field_value(field, value)?;
    match value {
        JavaValue::Boolean(value) => env.set_static_boolean_field(class, field, value),
        JavaValue::Byte(value) => env.set_static_byte_field(class, field, value),
        JavaValue::Char(value) => env.set_static_char_field(class, field, value),
        JavaValue::Short(value) => env.set_static_short_field(class, field, value),
        JavaValue::Int(value) => env.set_static_int_field(class, field, value),
        JavaValue::Long(value) => env.set_static_long_field(class, field, value),
        JavaValue::Float(value) => env.set_static_float_field(class, field, value),
        JavaValue::Double(value) => env.set_static_double_field(class, field, value),
        JavaValue::Object(value) if !value.is_null() => {
            let value = RawObject(value);
            env.set_static_object_field(class, field, Some(&value))
        }
        JavaValue::Object(_) | JavaValue::Null => env.set_static_object_field(class, field, None),
    }
}

fn validate_field_value(field: &FieldId, value: JavaValue) -> Result<()> {
    if value.matches_type(field.ty()) {
        Ok(())
    } else {
        Err(Error::InvalidArgumentType {
            index: 0,
            expected: field.ty().to_string(),
            actual: value.type_name(),
        })
    }
}

impl AsJObject for RawObject {
    fn as_jobject(&self) -> jni::jobject {
        self.0
    }
}
