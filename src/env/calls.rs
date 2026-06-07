use super::*;

struct InstancePrimitiveCall<'a> {
    object: jni::jobject,
    method: &'a MethodId,
    args: &'a [JavaValue],
    expected_return: JavaType,
    operation: &'static str,
    slot: usize,
}

struct StaticPrimitiveCall<'a> {
    class: &'a dyn AsJClass,
    method: &'a MethodId,
    args: &'a [JavaValue],
    expected_return: JavaType,
    operation: &'static str,
    slot: usize,
}

impl Env<'_> {
    /// Allocates a Java object with a detached constructor ID.
    ///
    /// # Safety
    ///
    /// `constructor` must have been resolved from `class` in this VM, and every object reference
    /// in `args` must be valid for this attached thread until the JNI call completes.
    pub unsafe fn new_object(
        &self,
        class: &impl AsJClass,
        constructor: &MethodId,
        args: &[JavaValue],
    ) -> Result<ObjectRef<'_>> {
        constructor.ensure_kind(MethodKind::Constructor, "JNIEnv::NewObjectA")?;
        constructor.signature.validate_arguments(args)?;
        let args = jni_args(args);
        let new_object = self.function::<jni::NewObjectA>(jni::ENV_NEW_OBJECT_A);
        let object = unsafe {
            new_object(
                self.handle.as_ptr(),
                class.as_jclass(),
                constructor.raw,
                jni_args_ptr(&args),
            )
        };
        self.check_pending_exception("JNIEnv::NewObjectA")?;
        unsafe { LocalRef::from_raw(self.local_ref_scope(), object) }
    }

    /// Calls an instance method with a detached method ID and returns an object local reference.
    ///
    /// # Safety
    ///
    /// `method` must have been resolved from `object`'s class or one of its supertypes in this VM,
    /// and every object reference in `args` must be valid for this attached thread until the JNI
    /// call completes.
    pub unsafe fn call_instance_object_method(
        &self,
        object: &(impl AsJObject + ?Sized),
        method: &MethodId,
        args: &[JavaValue],
    ) -> Result<Option<ObjectRef<'_>>> {
        method
            .ensure_instance_return(JavaType::Object(String::new()), "JNIEnv::CallObjectMethodA")?;
        method.signature.validate_arguments(args)?;
        let args = jni_args(args);
        let call = self.function::<jni::CallObjectMethodA>(jni::ENV_CALL_OBJECT_METHOD_A);
        let value = unsafe {
            call(
                self.handle.as_ptr(),
                object.as_jobject(),
                method.raw,
                jni_args_ptr(&args),
            )
        };
        self.check_pending_exception("JNIEnv::CallObjectMethodA")?;
        Ok(unsafe { LocalRef::from_nullable(self.local_ref_scope(), value) })
    }

    primitive_instance_method_calls!();

    /// Calls an instance void method with a detached method ID.
    ///
    /// # Safety
    ///
    /// `method` must have been resolved from `object`'s class or one of its supertypes in this VM,
    /// and every object reference in `args` must be valid for this attached thread until the JNI
    /// call completes.
    pub unsafe fn call_instance_void_method(
        &self,
        object: &(impl AsJObject + ?Sized),
        method: &MethodId,
        args: &[JavaValue],
    ) -> Result<()> {
        method.ensure_instance_return(JavaType::Void, "JNIEnv::CallVoidMethodA")?;
        method.signature.validate_arguments(args)?;
        let args = jni_args(args);
        let call = self.function::<jni::CallVoidMethodA>(jni::ENV_CALL_VOID_METHOD_A);
        unsafe {
            call(
                self.handle.as_ptr(),
                object.as_jobject(),
                method.raw,
                jni_args_ptr(&args),
            )
        };
        self.check_pending_exception("JNIEnv::CallVoidMethodA")
    }

    /// Calls a static method with a detached method ID and returns an object local reference.
    ///
    /// # Safety
    ///
    /// `method` must have been resolved from `class` in this VM, and every object reference in
    /// `args` must be valid for this attached thread until the JNI call completes.
    pub unsafe fn call_static_object_method(
        &self,
        class: &impl AsJClass,
        method: &MethodId,
        args: &[JavaValue],
    ) -> Result<Option<ObjectRef<'_>>> {
        method.ensure_static_return(
            JavaType::Object(String::new()),
            "JNIEnv::CallStaticObjectMethodA",
        )?;
        method.signature.validate_arguments(args)?;
        let args = jni_args(args);
        let call =
            self.function::<jni::CallStaticObjectMethodA>(jni::ENV_CALL_STATIC_OBJECT_METHOD_A);
        let value = unsafe {
            call(
                self.handle.as_ptr(),
                class.as_jclass(),
                method.raw,
                jni_args_ptr(&args),
            )
        };
        self.check_pending_exception("JNIEnv::CallStaticObjectMethodA")?;
        Ok(unsafe { LocalRef::from_nullable(self.local_ref_scope(), value) })
    }

    primitive_static_method_calls!();

    /// Calls a static void method with a detached method ID.
    ///
    /// # Safety
    ///
    /// `method` must have been resolved from `class` in this VM, and every object reference in
    /// `args` must be valid for this attached thread until the JNI call completes.
    pub unsafe fn call_static_void_method(
        &self,
        class: &impl AsJClass,
        method: &MethodId,
        args: &[JavaValue],
    ) -> Result<()> {
        method.ensure_static_return(JavaType::Void, "JNIEnv::CallStaticVoidMethodA")?;
        method.signature.validate_arguments(args)?;
        let args = jni_args(args);
        let call = self.function::<jni::CallStaticVoidMethodA>(jni::ENV_CALL_STATIC_VOID_METHOD_A);
        unsafe {
            call(
                self.handle.as_ptr(),
                class.as_jclass(),
                method.raw,
                jni_args_ptr(&args),
            )
        };
        self.check_pending_exception("JNIEnv::CallStaticVoidMethodA")
    }

    fn call_instance_primitive<T, F, C>(
        &self,
        request: InstancePrimitiveCall<'_>,
        call: C,
    ) -> Result<T>
    where
        F: Copy,
        C: FnOnce(F, *mut jni::JNIEnv, jni::jobject, jni::jmethodID, *const jni::jvalue) -> T,
    {
        request
            .method
            .ensure_instance_return(request.expected_return, request.operation)?;
        request.method.signature.validate_arguments(request.args)?;
        let args = jni_args(request.args);
        let function = self.function::<F>(request.slot);
        let value = call(
            function,
            self.handle.as_ptr(),
            request.object,
            request.method.raw,
            jni_args_ptr(&args),
        );
        self.check_pending_exception(request.operation)?;
        Ok(value)
    }

    fn call_static_primitive<T, F, C>(&self, request: StaticPrimitiveCall<'_>, call: C) -> Result<T>
    where
        F: Copy,
        C: FnOnce(F, *mut jni::JNIEnv, jni::jclass, jni::jmethodID, *const jni::jvalue) -> T,
    {
        request
            .method
            .ensure_static_return(request.expected_return, request.operation)?;
        request.method.signature.validate_arguments(request.args)?;
        let args = jni_args(request.args);
        let function = self.function::<F>(request.slot);
        let value = call(
            function,
            self.handle.as_ptr(),
            request.class.as_jclass(),
            request.method.raw,
            jni_args_ptr(&args),
        );
        self.check_pending_exception(request.operation)?;
        Ok(value)
    }
}
