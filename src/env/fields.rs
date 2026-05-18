use super::*;

struct InstancePrimitiveField<'a> {
    object: jni::jobject,
    field: &'a FieldId,
    expected_type: JavaType,
    operation: &'static str,
    slot: usize,
}

struct StaticPrimitiveField<'a> {
    class: &'a dyn AsJClass,
    field: &'a FieldId,
    expected_type: JavaType,
    operation: &'static str,
    slot: usize,
}

impl Env<'_> {
    pub fn get_instance_object_field(
        &self,
        object: &(impl AsJObject + ?Sized),
        field: &FieldId,
    ) -> Result<Option<ObjectRef<'_>>> {
        field.ensure_instance_type(JavaType::Object(String::new()), "JNIEnv::GetObjectField")?;
        let get = self.function::<jni::GetObjectField>(jni::ENV_GET_OBJECT_FIELD);
        let value = unsafe { get(self.handle.as_ptr(), object.as_jobject(), field.raw) };
        self.check_pending_exception("JNIEnv::GetObjectField")?;
        Ok(unsafe { LocalRef::from_nullable(self, value) })
    }

    pub fn set_instance_object_field(
        &self,
        object: &(impl AsJObject + ?Sized),
        field: &FieldId,
        value: Option<&dyn AsJObject>,
    ) -> Result<()> {
        field.ensure_instance_type(JavaType::Object(String::new()), "JNIEnv::SetObjectField")?;
        let set = self.function::<jni::SetObjectField>(jni::ENV_SET_OBJECT_FIELD);
        let value = value.map_or(ptr::null_mut(), AsJObject::as_jobject);
        unsafe { set(self.handle.as_ptr(), object.as_jobject(), field.raw, value) };
        self.check_pending_exception("JNIEnv::SetObjectField")
    }

    primitive_instance_fields! {
        get_instance_boolean_field, set_instance_boolean_field, bool, jni::jboolean, JavaType::Boolean,
        "JNIEnv::GetBooleanField", jni::ENV_GET_BOOLEAN_FIELD, jni::GetBooleanField, |value| value == jni::JNI_TRUE,
        "JNIEnv::SetBooleanField", jni::ENV_SET_BOOLEAN_FIELD, jni::SetBooleanField,
        |value| if value { jni::JNI_TRUE } else { jni::JNI_FALSE };

        get_instance_byte_field, set_instance_byte_field, jni::jbyte, jni::jbyte, JavaType::Byte,
        "JNIEnv::GetByteField", jni::ENV_GET_BYTE_FIELD, jni::GetByteField, |value| value,
        "JNIEnv::SetByteField", jni::ENV_SET_BYTE_FIELD, jni::SetByteField, |value| value;

        get_instance_char_field, set_instance_char_field, jni::jchar, jni::jchar, JavaType::Char,
        "JNIEnv::GetCharField", jni::ENV_GET_CHAR_FIELD, jni::GetCharField, |value| value,
        "JNIEnv::SetCharField", jni::ENV_SET_CHAR_FIELD, jni::SetCharField, |value| value;

        get_instance_short_field, set_instance_short_field, jni::jshort, jni::jshort, JavaType::Short,
        "JNIEnv::GetShortField", jni::ENV_GET_SHORT_FIELD, jni::GetShortField, |value| value,
        "JNIEnv::SetShortField", jni::ENV_SET_SHORT_FIELD, jni::SetShortField, |value| value;

        get_instance_int_field, set_instance_int_field, jni::jint, jni::jint, JavaType::Int,
        "JNIEnv::GetIntField", jni::ENV_GET_INT_FIELD, jni::GetIntField, |value| value,
        "JNIEnv::SetIntField", jni::ENV_SET_INT_FIELD, jni::SetIntField, |value| value;

        get_instance_long_field, set_instance_long_field, jni::jlong, jni::jlong, JavaType::Long,
        "JNIEnv::GetLongField", jni::ENV_GET_LONG_FIELD, jni::GetLongField, |value| value,
        "JNIEnv::SetLongField", jni::ENV_SET_LONG_FIELD, jni::SetLongField, |value| value;

        get_instance_float_field, set_instance_float_field, jni::jfloat, jni::jfloat, JavaType::Float,
        "JNIEnv::GetFloatField", jni::ENV_GET_FLOAT_FIELD, jni::GetFloatField, |value| value,
        "JNIEnv::SetFloatField", jni::ENV_SET_FLOAT_FIELD, jni::SetFloatField, |value| value;

        get_instance_double_field, set_instance_double_field, jni::jdouble, jni::jdouble, JavaType::Double,
        "JNIEnv::GetDoubleField", jni::ENV_GET_DOUBLE_FIELD, jni::GetDoubleField, |value| value,
        "JNIEnv::SetDoubleField", jni::ENV_SET_DOUBLE_FIELD, jni::SetDoubleField, |value| value;
    }

    pub fn get_static_object_field(
        &self,
        class: &impl AsJClass,
        field: &FieldId,
    ) -> Result<Option<ObjectRef<'_>>> {
        field.ensure_static_type(
            JavaType::Object(String::new()),
            "JNIEnv::GetStaticObjectField",
        )?;
        let get = self.function::<jni::GetStaticObjectField>(jni::ENV_GET_STATIC_OBJECT_FIELD);
        let value = unsafe { get(self.handle.as_ptr(), class.as_jclass(), field.raw) };
        self.check_pending_exception("JNIEnv::GetStaticObjectField")?;
        Ok(unsafe { LocalRef::from_nullable(self, value) })
    }

    pub fn set_static_object_field(
        &self,
        class: &impl AsJClass,
        field: &FieldId,
        value: Option<&dyn AsJObject>,
    ) -> Result<()> {
        field.ensure_static_type(
            JavaType::Object(String::new()),
            "JNIEnv::SetStaticObjectField",
        )?;
        let set = self.function::<jni::SetStaticObjectField>(jni::ENV_SET_STATIC_OBJECT_FIELD);
        let value = value.map_or(ptr::null_mut(), AsJObject::as_jobject);
        unsafe { set(self.handle.as_ptr(), class.as_jclass(), field.raw, value) };
        self.check_pending_exception("JNIEnv::SetStaticObjectField")
    }

    primitive_static_fields! {
        get_static_boolean_field, set_static_boolean_field, bool, jni::jboolean, JavaType::Boolean,
        "JNIEnv::GetStaticBooleanField", jni::ENV_GET_STATIC_BOOLEAN_FIELD, jni::GetStaticBooleanField,
        |value| value == jni::JNI_TRUE,
        "JNIEnv::SetStaticBooleanField", jni::ENV_SET_STATIC_BOOLEAN_FIELD, jni::SetStaticBooleanField,
        |value| if value { jni::JNI_TRUE } else { jni::JNI_FALSE };

        get_static_byte_field, set_static_byte_field, jni::jbyte, jni::jbyte, JavaType::Byte,
        "JNIEnv::GetStaticByteField", jni::ENV_GET_STATIC_BYTE_FIELD, jni::GetStaticByteField, |value| value,
        "JNIEnv::SetStaticByteField", jni::ENV_SET_STATIC_BYTE_FIELD, jni::SetStaticByteField, |value| value;

        get_static_char_field, set_static_char_field, jni::jchar, jni::jchar, JavaType::Char,
        "JNIEnv::GetStaticCharField", jni::ENV_GET_STATIC_CHAR_FIELD, jni::GetStaticCharField, |value| value,
        "JNIEnv::SetStaticCharField", jni::ENV_SET_STATIC_CHAR_FIELD, jni::SetStaticCharField, |value| value;

        get_static_short_field, set_static_short_field, jni::jshort, jni::jshort, JavaType::Short,
        "JNIEnv::GetStaticShortField", jni::ENV_GET_STATIC_SHORT_FIELD, jni::GetStaticShortField, |value| value,
        "JNIEnv::SetStaticShortField", jni::ENV_SET_STATIC_SHORT_FIELD, jni::SetStaticShortField, |value| value;

        get_static_int_field, set_static_int_field, jni::jint, jni::jint, JavaType::Int,
        "JNIEnv::GetStaticIntField", jni::ENV_GET_STATIC_INT_FIELD, jni::GetStaticIntField, |value| value,
        "JNIEnv::SetStaticIntField", jni::ENV_SET_STATIC_INT_FIELD, jni::SetStaticIntField, |value| value;

        get_static_long_field, set_static_long_field, jni::jlong, jni::jlong, JavaType::Long,
        "JNIEnv::GetStaticLongField", jni::ENV_GET_STATIC_LONG_FIELD, jni::GetStaticLongField, |value| value,
        "JNIEnv::SetStaticLongField", jni::ENV_SET_STATIC_LONG_FIELD, jni::SetStaticLongField, |value| value;

        get_static_float_field, set_static_float_field, jni::jfloat, jni::jfloat, JavaType::Float,
        "JNIEnv::GetStaticFloatField", jni::ENV_GET_STATIC_FLOAT_FIELD, jni::GetStaticFloatField, |value| value,
        "JNIEnv::SetStaticFloatField", jni::ENV_SET_STATIC_FLOAT_FIELD, jni::SetStaticFloatField, |value| value;

        get_static_double_field, set_static_double_field, jni::jdouble, jni::jdouble, JavaType::Double,
        "JNIEnv::GetStaticDoubleField", jni::ENV_GET_STATIC_DOUBLE_FIELD, jni::GetStaticDoubleField, |value| value,
        "JNIEnv::SetStaticDoubleField", jni::ENV_SET_STATIC_DOUBLE_FIELD, jni::SetStaticDoubleField, |value| value;
    }

    fn get_instance_primitive_field<T, F, C>(
        &self,
        request: InstancePrimitiveField<'_>,
        get: C,
    ) -> Result<T>
    where
        F: Copy,
        C: FnOnce(F, *mut jni::JNIEnv, jni::jobject, jni::jfieldID) -> T,
    {
        request
            .field
            .ensure_instance_type(request.expected_type, request.operation)?;
        let function = self.function::<F>(request.slot);
        let value = get(
            function,
            self.handle.as_ptr(),
            request.object,
            request.field.raw,
        );
        self.check_pending_exception(request.operation)?;
        Ok(value)
    }

    fn set_instance_primitive_field<F, C>(
        &self,
        request: InstancePrimitiveField<'_>,
        set: C,
    ) -> Result<()>
    where
        F: Copy,
        C: FnOnce(F, *mut jni::JNIEnv, jni::jobject, jni::jfieldID),
    {
        request
            .field
            .ensure_instance_type(request.expected_type, request.operation)?;
        let function = self.function::<F>(request.slot);
        set(
            function,
            self.handle.as_ptr(),
            request.object,
            request.field.raw,
        );
        self.check_pending_exception(request.operation)
    }

    fn get_static_primitive_field<T, F, C>(
        &self,
        request: StaticPrimitiveField<'_>,
        get: C,
    ) -> Result<T>
    where
        F: Copy,
        C: FnOnce(F, *mut jni::JNIEnv, jni::jclass, jni::jfieldID) -> T,
    {
        request
            .field
            .ensure_static_type(request.expected_type, request.operation)?;
        let function = self.function::<F>(request.slot);
        let value = get(
            function,
            self.handle.as_ptr(),
            request.class.as_jclass(),
            request.field.raw,
        );
        self.check_pending_exception(request.operation)?;
        Ok(value)
    }

    fn set_static_primitive_field<F, C>(
        &self,
        request: StaticPrimitiveField<'_>,
        set: C,
    ) -> Result<()>
    where
        F: Copy,
        C: FnOnce(F, *mut jni::JNIEnv, jni::jclass, jni::jfieldID),
    {
        request
            .field
            .ensure_static_type(request.expected_type, request.operation)?;
        let function = self.function::<F>(request.slot);
        set(
            function,
            self.handle.as_ptr(),
            request.class.as_jclass(),
            request.field.raw,
        );
        self.check_pending_exception(request.operation)
    }
}
