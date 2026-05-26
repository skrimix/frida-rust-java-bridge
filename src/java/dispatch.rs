use super::*;

pub(super) struct RawObject(pub(super) jni::jobject);

pub(super) fn call_instance_return(
    env: &Env<'_>,
    object: &(impl AsJObject + ?Sized),
    method: &MethodId,
    args: &[JavaValue],
) -> Result<JavaReturn> {
    // SAFETY: `raw::Class` resolves `method` from the selected class immediately before dispatch.
    // High-level selected handles validate receivers before reaching this helper; callers using
    // `raw::Class` are intentionally at the low-level descriptor/value boundary.
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            unsafe { env.call_instance_void_method(object, method, args)? };
            JavaReturn::Void
        }
        JavaType::Boolean => {
            JavaReturn::Boolean(unsafe { env.call_instance_boolean_method(object, method, args)? })
        }
        JavaType::Byte => {
            JavaReturn::Byte(unsafe { env.call_instance_byte_method(object, method, args)? })
        }
        JavaType::Char => {
            JavaReturn::Char(unsafe { env.call_instance_char_method(object, method, args)? })
        }
        JavaType::Short => {
            JavaReturn::Short(unsafe { env.call_instance_short_method(object, method, args)? })
        }
        JavaType::Int => {
            JavaReturn::Int(unsafe { env.call_instance_int_method(object, method, args)? })
        }
        JavaType::Long => {
            JavaReturn::Long(unsafe { env.call_instance_long_method(object, method, args)? })
        }
        JavaType::Float => {
            JavaReturn::Float(unsafe { env.call_instance_float_method(object, method, args)? })
        }
        JavaType::Double => {
            JavaReturn::Double(unsafe { env.call_instance_double_method(object, method, args)? })
        }
        JavaType::Object(_) => JavaReturn::Object(
            unsafe { env.call_instance_object_method(object, method, args)? }
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            unsafe { env.call_instance_object_method(object, method, args)? }
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
    // SAFETY: `raw::Class` resolves `method` from `class` immediately before dispatch.
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            unsafe { env.call_static_void_method(class, method, args)? };
            JavaReturn::Void
        }
        JavaType::Boolean => {
            JavaReturn::Boolean(unsafe { env.call_static_boolean_method(class, method, args)? })
        }
        JavaType::Byte => {
            JavaReturn::Byte(unsafe { env.call_static_byte_method(class, method, args)? })
        }
        JavaType::Char => {
            JavaReturn::Char(unsafe { env.call_static_char_method(class, method, args)? })
        }
        JavaType::Short => {
            JavaReturn::Short(unsafe { env.call_static_short_method(class, method, args)? })
        }
        JavaType::Int => {
            JavaReturn::Int(unsafe { env.call_static_int_method(class, method, args)? })
        }
        JavaType::Long => {
            JavaReturn::Long(unsafe { env.call_static_long_method(class, method, args)? })
        }
        JavaType::Float => {
            JavaReturn::Float(unsafe { env.call_static_float_method(class, method, args)? })
        }
        JavaType::Double => {
            JavaReturn::Double(unsafe { env.call_static_double_method(class, method, args)? })
        }
        JavaType::Object(_) => JavaReturn::Object(
            unsafe { env.call_static_object_method(class, method, args)? }
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            unsafe { env.call_static_object_method(class, method, args)? }
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
    // SAFETY: `raw::Class` resolves `field` from the selected class immediately before dispatch.
    // High-level selected handles validate receivers before reaching this helper.
    Ok(match field.ty() {
        JavaType::Boolean => {
            JavaReturn::Boolean(unsafe { env.get_instance_boolean_field(object, field)? })
        }
        JavaType::Byte => JavaReturn::Byte(unsafe { env.get_instance_byte_field(object, field)? }),
        JavaType::Char => JavaReturn::Char(unsafe { env.get_instance_char_field(object, field)? }),
        JavaType::Short => {
            JavaReturn::Short(unsafe { env.get_instance_short_field(object, field)? })
        }
        JavaType::Int => JavaReturn::Int(unsafe { env.get_instance_int_field(object, field)? }),
        JavaType::Long => JavaReturn::Long(unsafe { env.get_instance_long_field(object, field)? }),
        JavaType::Float => {
            JavaReturn::Float(unsafe { env.get_instance_float_field(object, field)? })
        }
        JavaType::Double => {
            JavaReturn::Double(unsafe { env.get_instance_double_field(object, field)? })
        }
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "java::raw::Class::get_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(_) => JavaReturn::Object(
            unsafe { env.get_instance_object_field(object, field)? }
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            unsafe { env.get_instance_object_field(object, field)? }
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
    // SAFETY: `raw::Class` resolves `field` from the selected class immediately before dispatch.
    // High-level selected handles validate receivers before reaching this helper.
    match value {
        JavaValue::Boolean(value) => unsafe {
            env.set_instance_boolean_field(object, field, value)
        },
        JavaValue::Byte(value) => unsafe { env.set_instance_byte_field(object, field, value) },
        JavaValue::Char(value) => unsafe { env.set_instance_char_field(object, field, value) },
        JavaValue::Short(value) => unsafe { env.set_instance_short_field(object, field, value) },
        JavaValue::Int(value) => unsafe { env.set_instance_int_field(object, field, value) },
        JavaValue::Long(value) => unsafe { env.set_instance_long_field(object, field, value) },
        JavaValue::Float(value) => unsafe { env.set_instance_float_field(object, field, value) },
        JavaValue::Double(value) => unsafe { env.set_instance_double_field(object, field, value) },
        JavaValue::Object(value) if !value.is_null() => {
            let value = RawObject(value.as_jobject());
            unsafe { env.set_instance_object_field(object, field, Some(&value)) }
        }
        JavaValue::Object(_) | JavaValue::Null => unsafe {
            env.set_instance_object_field(object, field, None)
        },
    }
}

pub(super) fn get_static_field(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    field: &FieldId,
) -> Result<JavaReturn> {
    // SAFETY: `raw::Class` resolves `field` from `class` immediately before dispatch.
    Ok(match field.ty() {
        JavaType::Boolean => {
            JavaReturn::Boolean(unsafe { env.get_static_boolean_field(class, field)? })
        }
        JavaType::Byte => JavaReturn::Byte(unsafe { env.get_static_byte_field(class, field)? }),
        JavaType::Char => JavaReturn::Char(unsafe { env.get_static_char_field(class, field)? }),
        JavaType::Short => JavaReturn::Short(unsafe { env.get_static_short_field(class, field)? }),
        JavaType::Int => JavaReturn::Int(unsafe { env.get_static_int_field(class, field)? }),
        JavaType::Long => JavaReturn::Long(unsafe { env.get_static_long_field(class, field)? }),
        JavaType::Float => JavaReturn::Float(unsafe { env.get_static_float_field(class, field)? }),
        JavaType::Double => {
            JavaReturn::Double(unsafe { env.get_static_double_field(class, field)? })
        }
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "java::raw::Class::get_static_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(_) => JavaReturn::Object(
            unsafe { env.get_static_object_field(class, field)? }
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Array(
            unsafe { env.get_static_object_field(class, field)? }
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
    // SAFETY: `raw::Class` resolves `field` from `class` immediately before dispatch.
    match value {
        JavaValue::Boolean(value) => unsafe { env.set_static_boolean_field(class, field, value) },
        JavaValue::Byte(value) => unsafe { env.set_static_byte_field(class, field, value) },
        JavaValue::Char(value) => unsafe { env.set_static_char_field(class, field, value) },
        JavaValue::Short(value) => unsafe { env.set_static_short_field(class, field, value) },
        JavaValue::Int(value) => unsafe { env.set_static_int_field(class, field, value) },
        JavaValue::Long(value) => unsafe { env.set_static_long_field(class, field, value) },
        JavaValue::Float(value) => unsafe { env.set_static_float_field(class, field, value) },
        JavaValue::Double(value) => unsafe { env.set_static_double_field(class, field, value) },
        JavaValue::Object(value) if !value.is_null() => {
            let value = RawObject(value.as_jobject());
            unsafe { env.set_static_object_field(class, field, Some(&value)) }
        }
        JavaValue::Object(_) | JavaValue::Null => unsafe {
            env.set_static_object_field(class, field, None)
        },
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
