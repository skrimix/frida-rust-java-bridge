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
    pub fn new_object(
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
        unsafe { LocalRef::from_raw(self, object) }
    }

    pub fn call_instance_object_method(
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
        Ok(unsafe { LocalRef::from_nullable(self, value) })
    }

    primitive_instance_method_calls! {
        call_instance_boolean_method, bool, JavaType::Boolean, "JNIEnv::CallBooleanMethodA",
        jni::ENV_CALL_BOOLEAN_METHOD_A, jni::CallBooleanMethodA, |value| value == jni::JNI_TRUE;

        call_instance_byte_method, jni::jbyte, JavaType::Byte, "JNIEnv::CallByteMethodA",
        jni::ENV_CALL_BYTE_METHOD_A, jni::CallByteMethodA, |value| value;

        call_instance_char_method, jni::jchar, JavaType::Char, "JNIEnv::CallCharMethodA",
        jni::ENV_CALL_CHAR_METHOD_A, jni::CallCharMethodA, |value| value;

        call_instance_short_method, jni::jshort, JavaType::Short, "JNIEnv::CallShortMethodA",
        jni::ENV_CALL_SHORT_METHOD_A, jni::CallShortMethodA, |value| value;

        call_instance_int_method, jni::jint, JavaType::Int, "JNIEnv::CallIntMethodA",
        jni::ENV_CALL_INT_METHOD_A, jni::CallIntMethodA, |value| value;

        call_instance_long_method, jni::jlong, JavaType::Long, "JNIEnv::CallLongMethodA",
        jni::ENV_CALL_LONG_METHOD_A, jni::CallLongMethodA, |value| value;

        call_instance_float_method, jni::jfloat, JavaType::Float, "JNIEnv::CallFloatMethodA",
        jni::ENV_CALL_FLOAT_METHOD_A, jni::CallFloatMethodA, |value| value;

        call_instance_double_method, jni::jdouble, JavaType::Double, "JNIEnv::CallDoubleMethodA",
        jni::ENV_CALL_DOUBLE_METHOD_A, jni::CallDoubleMethodA, |value| value;
    }

    pub fn call_instance_void_method(
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

    pub fn call_static_object_method(
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
        Ok(unsafe { LocalRef::from_nullable(self, value) })
    }

    primitive_static_method_calls! {
        call_static_boolean_method, bool, JavaType::Boolean, "JNIEnv::CallStaticBooleanMethodA",
        jni::ENV_CALL_STATIC_BOOLEAN_METHOD_A, jni::CallStaticBooleanMethodA, |value| value == jni::JNI_TRUE;

        call_static_byte_method, jni::jbyte, JavaType::Byte, "JNIEnv::CallStaticByteMethodA",
        jni::ENV_CALL_STATIC_BYTE_METHOD_A, jni::CallStaticByteMethodA, |value| value;

        call_static_char_method, jni::jchar, JavaType::Char, "JNIEnv::CallStaticCharMethodA",
        jni::ENV_CALL_STATIC_CHAR_METHOD_A, jni::CallStaticCharMethodA, |value| value;

        call_static_short_method, jni::jshort, JavaType::Short, "JNIEnv::CallStaticShortMethodA",
        jni::ENV_CALL_STATIC_SHORT_METHOD_A, jni::CallStaticShortMethodA, |value| value;

        call_static_int_method, jni::jint, JavaType::Int, "JNIEnv::CallStaticIntMethodA",
        jni::ENV_CALL_STATIC_INT_METHOD_A, jni::CallStaticIntMethodA, |value| value;

        call_static_long_method, jni::jlong, JavaType::Long, "JNIEnv::CallStaticLongMethodA",
        jni::ENV_CALL_STATIC_LONG_METHOD_A, jni::CallStaticLongMethodA, |value| value;

        call_static_float_method, jni::jfloat, JavaType::Float, "JNIEnv::CallStaticFloatMethodA",
        jni::ENV_CALL_STATIC_FLOAT_METHOD_A, jni::CallStaticFloatMethodA, |value| value;

        call_static_double_method, jni::jdouble, JavaType::Double, "JNIEnv::CallStaticDoubleMethodA",
        jni::ENV_CALL_STATIC_DOUBLE_METHOD_A, jni::CallStaticDoubleMethodA, |value| value;
    }

    pub fn call_static_void_method(
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
