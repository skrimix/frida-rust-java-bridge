use std::ptr;

use crate::{
    env::{Env, FieldId},
    error::Result,
    jni,
    refs::{AsJClass, AsJObject, LocalRef, ObjectRef},
    signature::JavaType,
};

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
    /// Gets an instance object field with a detached field ID.
    ///
    /// # Safety
    ///
    /// `field` must have been resolved from `object`'s class or one of its supertypes in this VM.
    pub unsafe fn get_instance_object_field(
        &self,
        object: &(impl AsJObject + ?Sized),
        field: &FieldId,
    ) -> Result<Option<ObjectRef<'_>>> {
        field.ensure_instance_type(JavaType::Object(String::new()), "JNIEnv::GetObjectField")?;
        let get = self.function::<jni::GetObjectField>(jni::ENV_GET_OBJECT_FIELD);
        let value = unsafe { get(self.handle.as_ptr(), object.as_jobject(), field.raw) };
        self.check_pending_exception("JNIEnv::GetObjectField")?;
        Ok(unsafe { LocalRef::from_nullable(self.local_ref_scope(), value) })
    }

    /// Sets an instance object field with a detached field ID.
    ///
    /// # Safety
    ///
    /// `field` must have been resolved from `object`'s class or one of its supertypes in this VM,
    /// and `value`, when present, must be valid for this attached thread until the JNI call
    /// completes.
    pub unsafe fn set_instance_object_field(
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

    primitive_instance_fields!();

    /// Gets a static object field with a detached field ID.
    ///
    /// # Safety
    ///
    /// `field` must have been resolved from `class` in this VM.
    pub unsafe fn get_static_object_field(
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
        Ok(unsafe { LocalRef::from_nullable(self.local_ref_scope(), value) })
    }

    /// Sets a static object field with a detached field ID.
    ///
    /// # Safety
    ///
    /// `field` must have been resolved from `class` in this VM, and `value`, when present, must be
    /// valid for this attached thread until the JNI call completes.
    pub unsafe fn set_static_object_field(
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

    primitive_static_fields!();

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
