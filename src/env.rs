use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
};

use crate::{
    error::{Error, Result},
    jni,
    refs::{
        AsJClass, AsJObject, ClassRef, GlobalRef, LocalRef, ObjectRef, StringRef, ThrowableRef,
    },
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodKind {
    Constructor,
    Instance,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodRef {
    raw: jni::jmethodID,
    kind: MethodKind,
    signature: MethodSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    Instance,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldRef {
    raw: jni::jfieldID,
    kind: FieldKind,
    ty: JavaType,
}

// JNI method and field IDs are VM-stable identifiers tied to their defining class.
unsafe impl Send for MethodRef {}
unsafe impl Sync for MethodRef {}
unsafe impl Send for FieldRef {}
unsafe impl Sync for FieldRef {}

struct InstancePrimitiveCall<'a> {
    object: &'a dyn AsJObject,
    method: &'a MethodRef,
    args: &'a [JavaValue],
    expected_return: JavaType,
    operation: &'static str,
    slot: usize,
}

struct InstancePrimitiveField<'a> {
    object: &'a dyn AsJObject,
    field: &'a FieldRef,
    expected_type: JavaType,
    operation: &'static str,
    slot: usize,
}

struct StaticPrimitiveCall<'a> {
    class: &'a dyn AsJClass,
    method: &'a MethodRef,
    args: &'a [JavaValue],
    expected_return: JavaType,
    operation: &'static str,
    slot: usize,
}

struct StaticPrimitiveField<'a> {
    class: &'a dyn AsJClass,
    field: &'a FieldRef,
    expected_type: JavaType,
    operation: &'static str,
    slot: usize,
}

#[derive(Clone, Copy)]
pub struct Env<'vm> {
    handle: NonNull<jni::JNIEnv>,
    vm: &'vm Vm,
    _thread_affine: PhantomData<Rc<()>>,
}

pub struct AttachedEnv<'vm> {
    env: Env<'vm>,
    vm: &'vm Vm,
    detach_on_drop: bool,
}

impl<'vm> Env<'vm> {
    pub(crate) fn from_raw(handle: NonNull<jni::JNIEnv>, vm: &'vm Vm) -> Self {
        Self {
            handle,
            vm,
            _thread_affine: PhantomData,
        }
    }

    pub fn handle(&self) -> NonNull<jni::JNIEnv> {
        self.handle
    }

    pub fn vm(&self) -> &'vm Vm {
        self.vm
    }

    pub fn version(&self) -> jni::jint {
        let get_version = self.function::<jni::GetVersion>(jni::ENV_GET_VERSION);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { get_version(self.handle.as_ptr()) }
    }

    pub fn find_class(&self, name: &str) -> Result<ClassRef<'_>> {
        let class = self.find_class_raw(name)?;
        unsafe { LocalRef::from_raw(self, class) }
    }

    pub fn find_class_raw(&self, name: &str) -> Result<jni::jclass> {
        let name = CString::new(name)?;
        let find_class = self.function::<jni::FindClass>(jni::ENV_FIND_CLASS);

        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        let class = unsafe { find_class(self.handle.as_ptr(), name.as_ptr()) };
        self.check_pending_exception("JNIEnv::FindClass")?;

        if class.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::FindClass",
            })
        } else {
            Ok(class)
        }
    }

    pub fn new_string_utf(&self, text: &str) -> Result<StringRef<'_>> {
        let string = self.new_string_utf_raw(text)?;
        unsafe { LocalRef::from_raw(self, string) }
    }

    pub fn new_string_utf_raw(&self, text: &str) -> Result<jni::jstring> {
        let text = CString::new(text)?;
        let new_string_utf = self.function::<jni::NewStringUtf>(jni::ENV_NEW_STRING_UTF);

        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        let string = unsafe { new_string_utf(self.handle.as_ptr(), text.as_ptr()) };
        self.check_pending_exception("JNIEnv::NewStringUTF")?;

        if string.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::NewStringUTF",
            })
        } else {
            Ok(string)
        }
    }

    pub fn get_string(&self, string: &StringRef<'_>) -> Result<String> {
        unsafe { self.get_string_raw(string.as_jstring()) }
    }

    pub fn get_string_utf(&self, string: &StringRef<'_>) -> Result<String> {
        unsafe { self.get_string_utf_raw(string.as_jstring()) }
    }

    /// Copies a Java string into a Rust `String` through JNI's UTF-16 string accessors.
    ///
    /// # Safety
    ///
    /// `string` must be a valid `jstring` local or global reference for this VM.
    pub unsafe fn get_string_raw(&self, string: jni::jstring) -> Result<String> {
        let get_string_length = self.function::<jni::GetStringLength>(jni::ENV_GET_STRING_LENGTH);
        let get_string_chars = self.function::<jni::GetStringChars>(jni::ENV_GET_STRING_CHARS);
        let release_string_chars =
            self.function::<jni::ReleaseStringChars>(jni::ENV_RELEASE_STRING_CHARS);
        let mut is_copy = jni::JNI_FALSE;

        let length = unsafe { get_string_length(self.handle.as_ptr(), string) };
        let chars = unsafe { get_string_chars(self.handle.as_ptr(), string, &mut is_copy) };
        if chars.is_null() {
            self.check_pending_exception("JNIEnv::GetStringChars")?;
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetStringChars",
            });
        }

        let chars = unsafe { std::slice::from_raw_parts(chars, length as usize) };
        let result =
            char::decode_utf16(chars.iter().copied()).collect::<std::result::Result<String, _>>();

        unsafe { release_string_chars(self.handle.as_ptr(), string, chars.as_ptr()) };

        result.map_err(Error::from)
    }

    /// Copies a Java string into a Rust `String`.
    ///
    /// # Safety
    ///
    /// `string` must be a valid `jstring` local or global reference for this VM.
    pub unsafe fn get_string_utf_raw(&self, string: jni::jstring) -> Result<String> {
        let get_string_utf_chars =
            self.function::<jni::GetStringUtfChars>(jni::ENV_GET_STRING_UTF_CHARS);
        let release_string_utf_chars =
            self.function::<jni::ReleaseStringUtfChars>(jni::ENV_RELEASE_STRING_UTF_CHARS);
        let mut is_copy = jni::JNI_FALSE;

        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and `string`
        // is expected to be a valid jstring owned by the caller.
        let chars = unsafe { get_string_utf_chars(self.handle.as_ptr(), string, &mut is_copy) };
        if chars.is_null() {
            self.check_pending_exception("JNIEnv::GetStringUTFChars")?;
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetStringUTFChars",
            });
        }

        // SAFETY: JNI returned a non-null, NUL-terminated modified UTF-8 buffer.
        let result = unsafe { CStr::from_ptr(chars) }
            .to_str()
            .map(str::to_owned)
            .map_err(Error::from);

        // SAFETY: The buffer came from GetStringUTFChars for the same jstring/env pair.
        unsafe { release_string_utf_chars(self.handle.as_ptr(), string, chars) };

        result
    }

    pub fn exception_check(&self) -> bool {
        let exception_check = self.function::<jni::ExceptionCheck>(jni::ENV_EXCEPTION_CHECK);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_check(self.handle.as_ptr()) == jni::JNI_TRUE }
    }

    pub fn exception_clear(&self) {
        let exception_clear = self.function::<jni::ExceptionClear>(jni::ENV_EXCEPTION_CLEAR);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_clear(self.handle.as_ptr()) };
    }

    pub fn exception_occurred(&self) -> Option<ThrowableRef<'_>> {
        let throwable = unsafe { self.exception_occurred_raw() };
        unsafe { LocalRef::from_nullable(self, throwable) }
    }

    /// Returns the pending exception local reference, if any.
    ///
    /// # Safety
    ///
    /// The returned local reference follows JNI local reference rules and must only be used on
    /// the current attached thread.
    pub unsafe fn exception_occurred_raw(&self) -> jni::jthrowable {
        let exception_occurred =
            self.function::<jni::ExceptionOccurred>(jni::ENV_EXCEPTION_OCCURRED);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_occurred(self.handle.as_ptr()) }
    }

    pub fn new_global_ref<K>(&self, object: &LocalRef<'_, K>) -> Result<GlobalRef<K>> {
        let reference = unsafe { self.new_global_ref_raw(object.as_jobject())? };
        unsafe { GlobalRef::from_raw(self.vm.clone(), reference) }
    }

    /// Creates a global reference for a JNI object.
    ///
    /// # Safety
    ///
    /// `object` must be a valid JNI local or global reference for this VM.
    pub unsafe fn new_global_ref_raw(&self, object: jni::jobject) -> Result<jni::jobject> {
        let new_global_ref = self.function::<jni::NewGlobalRef>(jni::ENV_NEW_GLOBAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is a valid JNI reference.
        let reference = unsafe { new_global_ref(self.handle.as_ptr(), object) };
        self.check_pending_exception("JNIEnv::NewGlobalRef")?;

        if object.is_null() || !reference.is_null() {
            Ok(reference)
        } else {
            Err(Error::NullReturn {
                operation: "JNIEnv::NewGlobalRef",
            })
        }
    }

    /// Deletes a global JNI reference.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid global reference for this VM.
    pub unsafe fn delete_global_ref_raw(&self, object: jni::jobject) {
        let delete_global_ref = self.function::<jni::DeleteGlobalRef>(jni::ENV_DELETE_GLOBAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is null or a valid global reference.
        unsafe { delete_global_ref(self.handle.as_ptr(), object) };
    }

    /// Deletes a local JNI reference.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid local reference on the current JNI frame.
    pub unsafe fn delete_local_ref_raw(&self, object: jni::jobject) {
        let delete_local_ref = self.function::<jni::DeleteLocalRef>(jni::ENV_DELETE_LOCAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is null or a valid local reference.
        unsafe { delete_local_ref(self.handle.as_ptr(), object) };
    }

    pub fn get_method(
        &self,
        class: &impl AsJClass,
        name: &str,
        signature: &str,
    ) -> Result<MethodRef> {
        let signature = MethodSignature::parse(signature)?;
        let raw = self.get_method_id_raw(class.as_jclass(), name, &signature)?;
        Ok(MethodRef {
            raw,
            kind: MethodKind::Instance,
            signature,
        })
    }

    pub fn get_constructor(&self, class: &impl AsJClass, signature: &str) -> Result<MethodRef> {
        let signature = MethodSignature::parse(signature)?;
        if signature.return_type() != &JavaType::Void {
            return Err(Error::InvalidReturnType {
                operation: "JNIEnv::GetMethodID(<init>)",
                expected: "void",
                actual: signature.return_type().to_string(),
            });
        }

        let raw = self.get_method_id_raw(class.as_jclass(), "<init>", &signature)?;
        Ok(MethodRef {
            raw,
            kind: MethodKind::Constructor,
            signature,
        })
    }

    pub fn get_static_method(
        &self,
        class: &impl AsJClass,
        name: &str,
        signature: &str,
    ) -> Result<MethodRef> {
        let signature = MethodSignature::parse(signature)?;
        let raw = self.get_static_method_id_raw(class.as_jclass(), name, &signature)?;
        Ok(MethodRef {
            raw,
            kind: MethodKind::Static,
            signature,
        })
    }

    pub fn get_field(&self, class: &impl AsJClass, name: &str, ty: &str) -> Result<FieldRef> {
        let ty = JavaType::parse(ty)?;
        let raw = self.get_field_id_raw(class.as_jclass(), name, &ty)?;
        Ok(FieldRef {
            raw,
            kind: FieldKind::Instance,
            ty,
        })
    }

    pub fn get_static_field(
        &self,
        class: &impl AsJClass,
        name: &str,
        ty: &str,
    ) -> Result<FieldRef> {
        let ty = JavaType::parse(ty)?;
        let raw = self.get_static_field_id_raw(class.as_jclass(), name, &ty)?;
        Ok(FieldRef {
            raw,
            kind: FieldKind::Static,
            ty,
        })
    }

    pub fn new_object(
        &self,
        class: &impl AsJClass,
        constructor: &MethodRef,
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

    pub fn call_object_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
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

    pub fn call_boolean_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<bool> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Boolean,
                operation: "JNIEnv::CallBooleanMethodA",
                slot: jni::ENV_CALL_BOOLEAN_METHOD_A,
            },
            |call: jni::CallBooleanMethodA, env, object, method, args| unsafe {
                call(env, object, method, args) == jni::JNI_TRUE
            },
        )
    }

    pub fn call_byte_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jbyte> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Byte,
                operation: "JNIEnv::CallByteMethodA",
                slot: jni::ENV_CALL_BYTE_METHOD_A,
            },
            |call: jni::CallByteMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_char_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jchar> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Char,
                operation: "JNIEnv::CallCharMethodA",
                slot: jni::ENV_CALL_CHAR_METHOD_A,
            },
            |call: jni::CallCharMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_short_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jshort> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Short,
                operation: "JNIEnv::CallShortMethodA",
                slot: jni::ENV_CALL_SHORT_METHOD_A,
            },
            |call: jni::CallShortMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_int_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jint> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Int,
                operation: "JNIEnv::CallIntMethodA",
                slot: jni::ENV_CALL_INT_METHOD_A,
            },
            |call: jni::CallIntMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_long_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jlong> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Long,
                operation: "JNIEnv::CallLongMethodA",
                slot: jni::ENV_CALL_LONG_METHOD_A,
            },
            |call: jni::CallLongMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_float_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jfloat> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Float,
                operation: "JNIEnv::CallFloatMethodA",
                slot: jni::ENV_CALL_FLOAT_METHOD_A,
            },
            |call: jni::CallFloatMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_double_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jdouble> {
        self.call_instance_primitive(
            InstancePrimitiveCall {
                object,
                method,
                args,
                expected_return: JavaType::Double,
                operation: "JNIEnv::CallDoubleMethodA",
                slot: jni::ENV_CALL_DOUBLE_METHOD_A,
            },
            |call: jni::CallDoubleMethodA, env, object, method, args| unsafe {
                call(env, object, method, args)
            },
        )
    }

    pub fn call_void_method(
        &self,
        object: &impl AsJObject,
        method: &MethodRef,
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
        method: &MethodRef,
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

    pub fn call_static_boolean_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<bool> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Boolean,
                operation: "JNIEnv::CallStaticBooleanMethodA",
                slot: jni::ENV_CALL_STATIC_BOOLEAN_METHOD_A,
            },
            |call: jni::CallStaticBooleanMethodA, env, class, method, args| unsafe {
                call(env, class, method, args) == jni::JNI_TRUE
            },
        )
    }

    pub fn call_static_byte_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jbyte> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Byte,
                operation: "JNIEnv::CallStaticByteMethodA",
                slot: jni::ENV_CALL_STATIC_BYTE_METHOD_A,
            },
            |call: jni::CallStaticByteMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_char_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jchar> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Char,
                operation: "JNIEnv::CallStaticCharMethodA",
                slot: jni::ENV_CALL_STATIC_CHAR_METHOD_A,
            },
            |call: jni::CallStaticCharMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_short_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jshort> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Short,
                operation: "JNIEnv::CallStaticShortMethodA",
                slot: jni::ENV_CALL_STATIC_SHORT_METHOD_A,
            },
            |call: jni::CallStaticShortMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_int_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jint> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Int,
                operation: "JNIEnv::CallStaticIntMethodA",
                slot: jni::ENV_CALL_STATIC_INT_METHOD_A,
            },
            |call: jni::CallStaticIntMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_long_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jlong> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Long,
                operation: "JNIEnv::CallStaticLongMethodA",
                slot: jni::ENV_CALL_STATIC_LONG_METHOD_A,
            },
            |call: jni::CallStaticLongMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_float_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jfloat> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Float,
                operation: "JNIEnv::CallStaticFloatMethodA",
                slot: jni::ENV_CALL_STATIC_FLOAT_METHOD_A,
            },
            |call: jni::CallStaticFloatMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_double_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
        args: &[JavaValue],
    ) -> Result<jni::jdouble> {
        self.call_static_primitive(
            StaticPrimitiveCall {
                class,
                method,
                args,
                expected_return: JavaType::Double,
                operation: "JNIEnv::CallStaticDoubleMethodA",
                slot: jni::ENV_CALL_STATIC_DOUBLE_METHOD_A,
            },
            |call: jni::CallStaticDoubleMethodA, env, class, method, args| unsafe {
                call(env, class, method, args)
            },
        )
    }

    pub fn call_static_void_method(
        &self,
        class: &impl AsJClass,
        method: &MethodRef,
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

    pub fn get_object_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
    ) -> Result<Option<ObjectRef<'_>>> {
        field.ensure_instance_type(JavaType::Object(String::new()), "JNIEnv::GetObjectField")?;
        let get = self.function::<jni::GetObjectField>(jni::ENV_GET_OBJECT_FIELD);
        let value = unsafe { get(self.handle.as_ptr(), object.as_jobject(), field.raw) };
        self.check_pending_exception("JNIEnv::GetObjectField")?;
        Ok(unsafe { LocalRef::from_nullable(self, value) })
    }

    pub fn set_object_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: Option<&dyn AsJObject>,
    ) -> Result<()> {
        field.ensure_instance_type(JavaType::Object(String::new()), "JNIEnv::SetObjectField")?;
        let set = self.function::<jni::SetObjectField>(jni::ENV_SET_OBJECT_FIELD);
        let value = value.map_or(std::ptr::null_mut(), AsJObject::as_jobject);
        unsafe { set(self.handle.as_ptr(), object.as_jobject(), field.raw, value) };
        self.check_pending_exception("JNIEnv::SetObjectField")
    }

    pub fn get_boolean_field(&self, object: &impl AsJObject, field: &FieldRef) -> Result<bool> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Boolean,
                operation: "JNIEnv::GetBooleanField",
                slot: jni::ENV_GET_BOOLEAN_FIELD,
            },
            |get: jni::GetBooleanField, env, object, field| unsafe {
                get(env, object, field) == jni::JNI_TRUE
            },
        )
    }

    pub fn set_boolean_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: bool,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Boolean,
                operation: "JNIEnv::SetBooleanField",
                slot: jni::ENV_SET_BOOLEAN_FIELD,
            },
            |set: jni::SetBooleanField, env, object, field| unsafe {
                set(
                    env,
                    object,
                    field,
                    if value { jni::JNI_TRUE } else { jni::JNI_FALSE },
                )
            },
        )
    }

    pub fn get_byte_field(&self, object: &impl AsJObject, field: &FieldRef) -> Result<jni::jbyte> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Byte,
                operation: "JNIEnv::GetByteField",
                slot: jni::ENV_GET_BYTE_FIELD,
            },
            |get: jni::GetByteField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_byte_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jbyte,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Byte,
                operation: "JNIEnv::SetByteField",
                slot: jni::ENV_SET_BYTE_FIELD,
            },
            |set: jni::SetByteField, env, object, field| unsafe { set(env, object, field, value) },
        )
    }

    pub fn get_char_field(&self, object: &impl AsJObject, field: &FieldRef) -> Result<jni::jchar> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Char,
                operation: "JNIEnv::GetCharField",
                slot: jni::ENV_GET_CHAR_FIELD,
            },
            |get: jni::GetCharField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_char_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jchar,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Char,
                operation: "JNIEnv::SetCharField",
                slot: jni::ENV_SET_CHAR_FIELD,
            },
            |set: jni::SetCharField, env, object, field| unsafe { set(env, object, field, value) },
        )
    }

    pub fn get_short_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
    ) -> Result<jni::jshort> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Short,
                operation: "JNIEnv::GetShortField",
                slot: jni::ENV_GET_SHORT_FIELD,
            },
            |get: jni::GetShortField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_short_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jshort,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Short,
                operation: "JNIEnv::SetShortField",
                slot: jni::ENV_SET_SHORT_FIELD,
            },
            |set: jni::SetShortField, env, object, field| unsafe { set(env, object, field, value) },
        )
    }

    pub fn get_int_field(&self, object: &impl AsJObject, field: &FieldRef) -> Result<jni::jint> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Int,
                operation: "JNIEnv::GetIntField",
                slot: jni::ENV_GET_INT_FIELD,
            },
            |get: jni::GetIntField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_int_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jint,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Int,
                operation: "JNIEnv::SetIntField",
                slot: jni::ENV_SET_INT_FIELD,
            },
            |set: jni::SetIntField, env, object, field| unsafe { set(env, object, field, value) },
        )
    }

    pub fn get_long_field(&self, object: &impl AsJObject, field: &FieldRef) -> Result<jni::jlong> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Long,
                operation: "JNIEnv::GetLongField",
                slot: jni::ENV_GET_LONG_FIELD,
            },
            |get: jni::GetLongField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_long_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jlong,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Long,
                operation: "JNIEnv::SetLongField",
                slot: jni::ENV_SET_LONG_FIELD,
            },
            |set: jni::SetLongField, env, object, field| unsafe { set(env, object, field, value) },
        )
    }

    pub fn get_float_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
    ) -> Result<jni::jfloat> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Float,
                operation: "JNIEnv::GetFloatField",
                slot: jni::ENV_GET_FLOAT_FIELD,
            },
            |get: jni::GetFloatField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_float_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jfloat,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Float,
                operation: "JNIEnv::SetFloatField",
                slot: jni::ENV_SET_FLOAT_FIELD,
            },
            |set: jni::SetFloatField, env, object, field| unsafe { set(env, object, field, value) },
        )
    }

    pub fn get_double_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
    ) -> Result<jni::jdouble> {
        self.get_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Double,
                operation: "JNIEnv::GetDoubleField",
                slot: jni::ENV_GET_DOUBLE_FIELD,
            },
            |get: jni::GetDoubleField, env, object, field| unsafe { get(env, object, field) },
        )
    }

    pub fn set_double_field(
        &self,
        object: &impl AsJObject,
        field: &FieldRef,
        value: jni::jdouble,
    ) -> Result<()> {
        self.set_instance_primitive_field(
            InstancePrimitiveField {
                object,
                field,
                expected_type: JavaType::Double,
                operation: "JNIEnv::SetDoubleField",
                slot: jni::ENV_SET_DOUBLE_FIELD,
            },
            |set: jni::SetDoubleField, env, object, field| unsafe {
                set(env, object, field, value)
            },
        )
    }

    pub fn get_static_object_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
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
        field: &FieldRef,
        value: Option<&dyn AsJObject>,
    ) -> Result<()> {
        field.ensure_static_type(
            JavaType::Object(String::new()),
            "JNIEnv::SetStaticObjectField",
        )?;
        let set = self.function::<jni::SetStaticObjectField>(jni::ENV_SET_STATIC_OBJECT_FIELD);
        let value = value.map_or(std::ptr::null_mut(), AsJObject::as_jobject);
        unsafe { set(self.handle.as_ptr(), class.as_jclass(), field.raw, value) };
        self.check_pending_exception("JNIEnv::SetStaticObjectField")
    }

    pub fn get_static_boolean_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<bool> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Boolean,
                operation: "JNIEnv::GetStaticBooleanField",
                slot: jni::ENV_GET_STATIC_BOOLEAN_FIELD,
            },
            |get: jni::GetStaticBooleanField, env, class, field| unsafe {
                get(env, class, field) == jni::JNI_TRUE
            },
        )
    }

    pub fn set_static_boolean_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: bool,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Boolean,
                operation: "JNIEnv::SetStaticBooleanField",
                slot: jni::ENV_SET_STATIC_BOOLEAN_FIELD,
            },
            |set: jni::SetStaticBooleanField, env, class, field| unsafe {
                set(
                    env,
                    class,
                    field,
                    if value { jni::JNI_TRUE } else { jni::JNI_FALSE },
                )
            },
        )
    }

    pub fn get_static_int_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jint> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Int,
                operation: "JNIEnv::GetStaticIntField",
                slot: jni::ENV_GET_STATIC_INT_FIELD,
            },
            |get: jni::GetStaticIntField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_int_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jint,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Int,
                operation: "JNIEnv::SetStaticIntField",
                slot: jni::ENV_SET_STATIC_INT_FIELD,
            },
            |set: jni::SetStaticIntField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    pub fn get_static_byte_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jbyte> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Byte,
                operation: "JNIEnv::GetStaticByteField",
                slot: jni::ENV_GET_STATIC_BYTE_FIELD,
            },
            |get: jni::GetStaticByteField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_byte_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jbyte,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Byte,
                operation: "JNIEnv::SetStaticByteField",
                slot: jni::ENV_SET_STATIC_BYTE_FIELD,
            },
            |set: jni::SetStaticByteField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    pub fn get_static_char_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jchar> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Char,
                operation: "JNIEnv::GetStaticCharField",
                slot: jni::ENV_GET_STATIC_CHAR_FIELD,
            },
            |get: jni::GetStaticCharField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_char_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jchar,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Char,
                operation: "JNIEnv::SetStaticCharField",
                slot: jni::ENV_SET_STATIC_CHAR_FIELD,
            },
            |set: jni::SetStaticCharField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    pub fn get_static_short_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jshort> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Short,
                operation: "JNIEnv::GetStaticShortField",
                slot: jni::ENV_GET_STATIC_SHORT_FIELD,
            },
            |get: jni::GetStaticShortField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_short_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jshort,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Short,
                operation: "JNIEnv::SetStaticShortField",
                slot: jni::ENV_SET_STATIC_SHORT_FIELD,
            },
            |set: jni::SetStaticShortField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    pub fn get_static_long_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jlong> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Long,
                operation: "JNIEnv::GetStaticLongField",
                slot: jni::ENV_GET_STATIC_LONG_FIELD,
            },
            |get: jni::GetStaticLongField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_long_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jlong,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Long,
                operation: "JNIEnv::SetStaticLongField",
                slot: jni::ENV_SET_STATIC_LONG_FIELD,
            },
            |set: jni::SetStaticLongField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    pub fn get_static_float_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jfloat> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Float,
                operation: "JNIEnv::GetStaticFloatField",
                slot: jni::ENV_GET_STATIC_FLOAT_FIELD,
            },
            |get: jni::GetStaticFloatField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_float_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jfloat,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Float,
                operation: "JNIEnv::SetStaticFloatField",
                slot: jni::ENV_SET_STATIC_FLOAT_FIELD,
            },
            |set: jni::SetStaticFloatField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    pub fn get_static_double_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
    ) -> Result<jni::jdouble> {
        self.get_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Double,
                operation: "JNIEnv::GetStaticDoubleField",
                slot: jni::ENV_GET_STATIC_DOUBLE_FIELD,
            },
            |get: jni::GetStaticDoubleField, env, class, field| unsafe { get(env, class, field) },
        )
    }

    pub fn set_static_double_field(
        &self,
        class: &impl AsJClass,
        field: &FieldRef,
        value: jni::jdouble,
    ) -> Result<()> {
        self.set_static_primitive_field(
            StaticPrimitiveField {
                class,
                field,
                expected_type: JavaType::Double,
                operation: "JNIEnv::SetStaticDoubleField",
                slot: jni::ENV_SET_STATIC_DOUBLE_FIELD,
            },
            |set: jni::SetStaticDoubleField, env, class, field| unsafe {
                set(env, class, field, value)
            },
        )
    }

    fn get_method_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        signature: &MethodSignature,
    ) -> Result<jni::jmethodID> {
        let name = CString::new(name)?;
        let signature = CString::new(signature.to_string())?;
        let get_method_id = self.function::<jni::GetMethodId>(jni::ENV_GET_METHOD_ID);
        let method = unsafe {
            get_method_id(
                self.handle.as_ptr(),
                class,
                name.as_ptr(),
                signature.as_ptr(),
            )
        };
        self.check_pending_exception("JNIEnv::GetMethodID")?;
        if method.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetMethodID",
            })
        } else {
            Ok(method)
        }
    }

    fn get_static_method_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        signature: &MethodSignature,
    ) -> Result<jni::jmethodID> {
        let name = CString::new(name)?;
        let signature = CString::new(signature.to_string())?;
        let get_static_method_id =
            self.function::<jni::GetStaticMethodId>(jni::ENV_GET_STATIC_METHOD_ID);
        let method = unsafe {
            get_static_method_id(
                self.handle.as_ptr(),
                class,
                name.as_ptr(),
                signature.as_ptr(),
            )
        };
        self.check_pending_exception("JNIEnv::GetStaticMethodID")?;
        if method.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetStaticMethodID",
            })
        } else {
            Ok(method)
        }
    }

    fn get_field_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        ty: &JavaType,
    ) -> Result<jni::jfieldID> {
        let name = CString::new(name)?;
        let ty = CString::new(ty.to_string())?;
        let get_field_id = self.function::<jni::GetFieldId>(jni::ENV_GET_FIELD_ID);
        let field =
            unsafe { get_field_id(self.handle.as_ptr(), class, name.as_ptr(), ty.as_ptr()) };
        self.check_pending_exception("JNIEnv::GetFieldID")?;
        if field.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetFieldID",
            })
        } else {
            Ok(field)
        }
    }

    fn get_static_field_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        ty: &JavaType,
    ) -> Result<jni::jfieldID> {
        let name = CString::new(name)?;
        let ty = CString::new(ty.to_string())?;
        let get_static_field_id =
            self.function::<jni::GetStaticFieldId>(jni::ENV_GET_STATIC_FIELD_ID);
        let field =
            unsafe { get_static_field_id(self.handle.as_ptr(), class, name.as_ptr(), ty.as_ptr()) };
        self.check_pending_exception("JNIEnv::GetStaticFieldID")?;
        if field.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetStaticFieldID",
            })
        } else {
            Ok(field)
        }
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
            request.object.as_jobject(),
            request.method.raw,
            jni_args_ptr(&args),
        );
        self.check_pending_exception(request.operation)?;
        Ok(value)
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
            request.object.as_jobject(),
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
            request.object.as_jobject(),
            request.field.raw,
        );
        self.check_pending_exception(request.operation)
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

    fn check_pending_exception(&self, operation: &'static str) -> Result<()> {
        if self.exception_check() {
            self.exception_clear();
            Err(Error::JavaException { operation })
        } else {
            Ok(())
        }
    }

    fn function<T: Copy>(&self, slot: usize) -> T {
        unsafe { jni::env_function(self.handle, slot) }
    }
}

impl MethodRef {
    pub fn raw(&self) -> jni::jmethodID {
        self.raw
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.signature
    }

    fn ensure_kind(&self, expected: MethodKind, operation: &'static str) -> Result<()> {
        if self.kind == expected {
            Ok(())
        } else {
            Err(Error::WrongMethodKind { operation })
        }
    }

    fn ensure_instance_return(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        self.ensure_kind(MethodKind::Instance, operation)?;
        self.ensure_return(expected, operation)
    }

    fn ensure_static_return(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        self.ensure_kind(MethodKind::Static, operation)?;
        self.ensure_return(expected, operation)
    }

    fn ensure_return(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        let actual = self.signature.return_type();
        let matches = if expected.is_reference() {
            actual.is_reference()
        } else {
            actual == &expected
        };

        if matches {
            Ok(())
        } else {
            Err(Error::InvalidReturnType {
                operation,
                expected: expected.jni_return_name(),
                actual: actual.to_string(),
            })
        }
    }
}

impl FieldRef {
    pub fn raw(&self) -> jni::jfieldID {
        self.raw
    }

    pub fn kind(&self) -> FieldKind {
        self.kind
    }

    pub fn ty(&self) -> &JavaType {
        &self.ty
    }

    fn ensure_kind(&self, expected: FieldKind, operation: &'static str) -> Result<()> {
        if self.kind == expected {
            Ok(())
        } else {
            Err(Error::WrongFieldKind { operation })
        }
    }

    fn ensure_instance_type(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        self.ensure_kind(FieldKind::Instance, operation)?;
        self.ensure_type(expected, operation)
    }

    fn ensure_static_type(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        self.ensure_kind(FieldKind::Static, operation)?;
        self.ensure_type(expected, operation)
    }

    fn ensure_type(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        let matches = if expected.is_reference() {
            self.ty.is_reference()
        } else {
            self.ty == expected
        };

        if matches {
            Ok(())
        } else {
            Err(Error::InvalidFieldType {
                operation,
                expected: expected.jni_return_name(),
                actual: self.ty.to_string(),
            })
        }
    }
}

fn jni_args(args: &[JavaValue]) -> Vec<jni::jvalue> {
    args.iter().copied().map(JavaValue::to_jvalue).collect()
}

fn jni_args_ptr(args: &[jni::jvalue]) -> *const jni::jvalue {
    if args.is_empty() {
        std::ptr::null()
    } else {
        args.as_ptr()
    }
}

impl<'vm> AttachedEnv<'vm> {
    pub(crate) fn new(vm: &'vm Vm, env: Env<'vm>, detach_on_drop: bool) -> Self {
        Self {
            env,
            vm,
            detach_on_drop,
        }
    }

    pub fn env(&self) -> Env<'vm> {
        self.env
    }

    pub fn detach_on_drop(&self) -> bool {
        self.detach_on_drop
    }
}

impl<'vm> std::ops::Deref for AttachedEnv<'vm> {
    type Target = Env<'vm>;

    fn deref(&self) -> &Self::Target {
        &self.env
    }
}

impl Drop for AttachedEnv<'_> {
    fn drop(&mut self) {
        if self.detach_on_drop {
            let _ = self.vm.detach_current_thread();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(kind: MethodKind, return_type: JavaType) -> MethodRef {
        MethodRef {
            raw: std::ptr::dangling_mut(),
            kind,
            signature: MethodSignature::new(Vec::new(), return_type),
        }
    }

    fn field(kind: FieldKind, ty: JavaType) -> FieldRef {
        FieldRef {
            raw: std::ptr::dangling_mut(),
            kind,
            ty,
        }
    }

    #[test]
    fn method_return_guards_accept_matching_kinds_and_reference_returns() {
        let instance_object = method(
            MethodKind::Instance,
            JavaType::Object("java/lang/String".to_owned()),
        );
        assert_eq!(
            instance_object.ensure_instance_return(JavaType::Object(String::new()), "test"),
            Ok(())
        );

        let static_array = method(MethodKind::Static, JavaType::Array(Box::new(JavaType::Int)));
        assert_eq!(
            static_array.ensure_static_return(JavaType::Object(String::new()), "test"),
            Ok(())
        );
    }

    #[test]
    fn method_return_guards_report_kind_and_type_mismatches() {
        let static_int = method(MethodKind::Static, JavaType::Int);
        assert_eq!(
            static_int.ensure_instance_return(JavaType::Int, "test"),
            Err(Error::WrongMethodKind { operation: "test" })
        );

        let instance_long = method(MethodKind::Instance, JavaType::Long);
        assert_eq!(
            instance_long.ensure_instance_return(JavaType::Int, "test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "int",
                actual: "J".to_owned(),
            })
        );
    }

    #[test]
    fn field_type_guards_accept_matching_kinds_and_reference_fields() {
        let instance_object = field(
            FieldKind::Instance,
            JavaType::Object("java/lang/String".to_owned()),
        );
        assert_eq!(
            instance_object.ensure_instance_type(JavaType::Object(String::new()), "test"),
            Ok(())
        );

        let static_array = field(FieldKind::Static, JavaType::Array(Box::new(JavaType::Int)));
        assert_eq!(
            static_array.ensure_static_type(JavaType::Object(String::new()), "test"),
            Ok(())
        );
    }

    #[test]
    fn field_type_guards_report_kind_and_type_mismatches() {
        let static_int = field(FieldKind::Static, JavaType::Int);
        assert_eq!(
            static_int.ensure_instance_type(JavaType::Int, "test"),
            Err(Error::WrongFieldKind { operation: "test" })
        );

        let instance_long = field(FieldKind::Instance, JavaType::Long);
        assert_eq!(
            instance_long.ensure_instance_type(JavaType::Int, "test"),
            Err(Error::InvalidFieldType {
                operation: "test",
                expected: "int",
                actual: "J".to_owned(),
            })
        );
    }

    #[test]
    fn jni_argument_buffers_use_null_for_empty_slices() {
        let empty = jni_args(&[]);
        assert!(jni_args_ptr(&empty).is_null());

        let args = jni_args(&[JavaValue::Int(42), JavaValue::Null]);
        assert_eq!(args.len(), 2);
        assert!(!jni_args_ptr(&args).is_null());
        assert_eq!(unsafe { args[0].i }, 42);
        assert!(unsafe { args[1].l }.is_null());
    }
}
