use super::*;

pub(super) struct RawObject(pub(super) jni::jobject);

pub(super) fn call_instance_return(
    env: &Env<'_>,
    holder: &raw::Class,
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
        JavaType::Object(name) => JavaReturn::Object(
            unsafe { env.call_instance_object_method(object, method, args)? }
                .map(|object| {
                    object_from_ref_with_declared(env, holder, &object, name, "Java method return")
                        .map(JavaReturnRef::Object)
                })
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Object(
            unsafe { env.call_instance_object_method(object, method, args)? }
                .map(|object| array_from_ref(env, holder.vm(), &object, (**element).clone()))
                .transpose()?
                .map(JavaReturnRef::Array),
        ),
    })
}

pub(super) fn call_static_return(
    env: &Env<'_>,
    holder: &raw::Class,
    method: &MethodId,
    args: &[JavaValue],
) -> Result<JavaReturn> {
    // SAFETY: `raw::Class` resolves `method` from `class` immediately before dispatch.
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            unsafe { env.call_static_void_method(&holder.inner.class, method, args)? };
            JavaReturn::Void
        }
        JavaType::Boolean => JavaReturn::Boolean(unsafe {
            env.call_static_boolean_method(&holder.inner.class, method, args)?
        }),
        JavaType::Byte => JavaReturn::Byte(unsafe {
            env.call_static_byte_method(&holder.inner.class, method, args)?
        }),
        JavaType::Char => JavaReturn::Char(unsafe {
            env.call_static_char_method(&holder.inner.class, method, args)?
        }),
        JavaType::Short => JavaReturn::Short(unsafe {
            env.call_static_short_method(&holder.inner.class, method, args)?
        }),
        JavaType::Int => JavaReturn::Int(unsafe {
            env.call_static_int_method(&holder.inner.class, method, args)?
        }),
        JavaType::Long => JavaReturn::Long(unsafe {
            env.call_static_long_method(&holder.inner.class, method, args)?
        }),
        JavaType::Float => JavaReturn::Float(unsafe {
            env.call_static_float_method(&holder.inner.class, method, args)?
        }),
        JavaType::Double => JavaReturn::Double(unsafe {
            env.call_static_double_method(&holder.inner.class, method, args)?
        }),
        JavaType::Object(name) => JavaReturn::Object(
            unsafe { env.call_static_object_method(&holder.inner.class, method, args)? }
                .map(|object| {
                    object_from_ref_with_declared(
                        env,
                        holder,
                        &object,
                        name,
                        "Java static method return",
                    )
                    .map(JavaReturnRef::Object)
                })
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Object(
            unsafe { env.call_static_object_method(&holder.inner.class, method, args)? }
                .map(|object| array_from_ref(env, holder.vm(), &object, (**element).clone()))
                .transpose()?
                .map(JavaReturnRef::Array),
        ),
    })
}

pub(super) fn get_instance_field(
    env: &Env<'_>,
    holder: &raw::Class,
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
        JavaType::Object(name) => JavaReturn::Object(
            unsafe { env.get_instance_object_field(object, field)? }
                .map(|object| {
                    object_from_ref_with_declared(env, holder, &object, name, "Java field value")
                        .map(JavaReturnRef::Object)
                })
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Object(
            unsafe { env.get_instance_object_field(object, field)? }
                .map(|object| array_from_ref(env, holder.vm(), &object, (**element).clone()))
                .transpose()?
                .map(JavaReturnRef::Array),
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
        JavaValue::Object(Some(value)) => {
            let value = RawObject(value.as_jobject());
            unsafe { env.set_instance_object_field(object, field, Some(&value)) }
        }
        JavaValue::Object(None) => unsafe { env.set_instance_object_field(object, field, None) },
        JavaValue::Void => unreachable!("field value was validated before dispatch"),
    }
}

pub(super) fn get_static_field(
    env: &Env<'_>,
    holder: &raw::Class,
    field: &FieldId,
) -> Result<JavaReturn> {
    // SAFETY: `raw::Class` resolves `field` from `class` immediately before dispatch.
    Ok(match field.ty() {
        JavaType::Boolean => JavaReturn::Boolean(unsafe {
            env.get_static_boolean_field(&holder.inner.class, field)?
        }),
        JavaType::Byte => {
            JavaReturn::Byte(unsafe { env.get_static_byte_field(&holder.inner.class, field)? })
        }
        JavaType::Char => {
            JavaReturn::Char(unsafe { env.get_static_char_field(&holder.inner.class, field)? })
        }
        JavaType::Short => {
            JavaReturn::Short(unsafe { env.get_static_short_field(&holder.inner.class, field)? })
        }
        JavaType::Int => {
            JavaReturn::Int(unsafe { env.get_static_int_field(&holder.inner.class, field)? })
        }
        JavaType::Long => {
            JavaReturn::Long(unsafe { env.get_static_long_field(&holder.inner.class, field)? })
        }
        JavaType::Float => {
            JavaReturn::Float(unsafe { env.get_static_float_field(&holder.inner.class, field)? })
        }
        JavaType::Double => {
            JavaReturn::Double(unsafe { env.get_static_double_field(&holder.inner.class, field)? })
        }
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "java::raw::Class::get_static_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(name) => JavaReturn::Object(
            unsafe { env.get_static_object_field(&holder.inner.class, field)? }
                .map(|object| {
                    object_from_ref_with_declared(
                        env,
                        holder,
                        &object,
                        name,
                        "Java static field value",
                    )
                    .map(JavaReturnRef::Object)
                })
                .transpose()?,
        ),
        JavaType::Array(element) => JavaReturn::Object(
            unsafe { env.get_static_object_field(&holder.inner.class, field)? }
                .map(|object| array_from_ref(env, holder.vm(), &object, (**element).clone()))
                .transpose()?
                .map(JavaReturnRef::Array),
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
        JavaValue::Object(Some(value)) => {
            let value = RawObject(value.as_jobject());
            unsafe { env.set_static_object_field(class, field, Some(&value)) }
        }
        JavaValue::Object(None) => unsafe { env.set_static_object_field(class, field, None) },
        JavaValue::Void => unreachable!("field value was validated before dispatch"),
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

fn object_from_ref_with_declared(
    env: &Env<'_>,
    holder: &raw::Class,
    object: &(impl JavaObjectRef + ?Sized),
    name: &str,
    operation: &'static str,
) -> Result<JavaObject> {
    let java = Java::new(holder.vm().clone());
    let scoped_java = match metadata::class_loader(env, holder.vm(), holder)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    let class = JavaClass::from_raw(scoped_java.find_class(&name.replace('/', "."))?);
    if class.is_instance(object)? {
        let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
        let reference = unsafe { GlobalRef::from_raw(holder.vm().clone(), reference)? };
        Ok(JavaObject::from_global_ref(class, reference))
    } else {
        let actual = env.get_object_class(object)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "declared return type",
            actual: format!("{:p} is not {}", actual.as_jclass(), name.replace('/', ".")),
        })
    }
}
