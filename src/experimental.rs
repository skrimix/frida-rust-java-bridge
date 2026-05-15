use std::{
    ffi::{CString, c_void},
    ptr::{self, NonNull},
};

use crate::{
    Error, Result,
    art::{ArtMethodReplacementGuard, original_method_call_bypass},
    env::MethodKind,
    java::{JavaClass, JavaMethodOverload},
    jni,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
};

pub type StaticVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass);
pub type StaticStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jstring;
pub type StaticBooleanReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jboolean;
pub type StaticByteReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jbyte;
pub type StaticCharReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jchar;
pub type StaticShortReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jshort;
pub type StaticI32ReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jint;
pub type StaticI64ReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jlong;
pub type StaticF32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jfloat;
pub type StaticF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jdouble;
pub type StaticStringToStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jstring) -> jni::jstring;
pub type StaticReferenceToReferenceReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jobject) -> jni::jobject;
pub type StaticI32I32ToI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jint, jni::jint) -> jni::jint;
pub type StaticZBCSToI32ReplacementFn = unsafe extern "C" fn(
    *mut jni::JNIEnv,
    jni::jclass,
    jni::jboolean,
    jni::jbyte,
    jni::jchar,
    jni::jshort,
) -> jni::jint;
pub type StaticI64F64ToI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jlong, jni::jdouble) -> jni::jlong;
pub type StaticF32F64ToF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jfloat, jni::jdouble) -> jni::jdouble;
pub type InstanceVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject);
pub type InstanceBooleanReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jboolean;
pub type InstanceByteReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jbyte;
pub type InstanceCharReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jchar;
pub type InstanceShortReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jshort;
pub type InstanceI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jint;
pub type InstanceI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jlong;
pub type InstanceF32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jfloat;
pub type InstanceF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jdouble;
pub type InstanceStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jstring;
pub type InstanceStringToStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jstring) -> jni::jstring;
pub type InstanceReferenceToReferenceReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jobject) -> jni::jobject;
pub type InstanceI32I32ToI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jint, jni::jint) -> jni::jint;
pub type InstanceZBCSToI32ReplacementFn = unsafe extern "C" fn(
    *mut jni::JNIEnv,
    jni::jobject,
    jni::jboolean,
    jni::jbyte,
    jni::jchar,
    jni::jshort,
) -> jni::jint;
pub type InstanceI64F64ToI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jlong, jni::jdouble) -> jni::jlong;
pub type InstanceF32F64ToF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jfloat, jni::jdouble) -> jni::jdouble;

/// A JNI-native implementation supported by the current experimental overload facade.
///
/// Each variant names the exact method kind and ABI shape accepted by the hidden ART backend. This
/// intentionally keeps unsupported signatures visible instead of weakening type checks.
#[doc(hidden)]
#[derive(Clone, Copy)]
pub enum MethodImplementation {
    StaticVoid(StaticVoidReplacementFn),
    StaticString(StaticStringReplacementFn),
    StaticBoolean(StaticBooleanReplacementFn),
    StaticByte(StaticByteReplacementFn),
    StaticChar(StaticCharReplacementFn),
    StaticShort(StaticShortReplacementFn),
    StaticI32(StaticI32ReplacementFn),
    StaticI64(StaticI64ReplacementFn),
    StaticF32(StaticF32ReplacementFn),
    StaticF64(StaticF64ReplacementFn),
    StaticStringToString(StaticStringToStringReplacementFn),
    StaticReferenceToReference(StaticReferenceToReferenceReplacementFn),
    StaticI32I32ToI32(StaticI32I32ToI32ReplacementFn),
    StaticZBCSToI32(StaticZBCSToI32ReplacementFn),
    StaticI64F64ToI64(StaticI64F64ToI64ReplacementFn),
    StaticF32F64ToF64(StaticF32F64ToF64ReplacementFn),
    InstanceVoid(InstanceVoidReplacementFn),
    InstanceBoolean(InstanceBooleanReplacementFn),
    InstanceByte(InstanceByteReplacementFn),
    InstanceChar(InstanceCharReplacementFn),
    InstanceShort(InstanceShortReplacementFn),
    InstanceI32(InstanceI32ReplacementFn),
    InstanceI64(InstanceI64ReplacementFn),
    InstanceF32(InstanceF32ReplacementFn),
    InstanceF64(InstanceF64ReplacementFn),
    InstanceString(InstanceStringReplacementFn),
    InstanceStringToString(InstanceStringToStringReplacementFn),
    InstanceReferenceToReference(InstanceReferenceToReferenceReplacementFn),
    InstanceI32I32ToI32(InstanceI32I32ToI32ReplacementFn),
    InstanceZBCSToI32(InstanceZBCSToI32ReplacementFn),
    InstanceI64F64ToI64(InstanceI64F64ToI64ReplacementFn),
    InstanceF32F64ToF64(InstanceF32F64ToF64ReplacementFn),
}

macro_rules! static_replacement {
    (
        $(#[$meta:meta])*
        $function:ident,
        $replacement_type:ty,
        $signature:literal,
        $guard_type:ty
    ) => {
        $(#[$meta])*
        #[doc(hidden)]
        pub unsafe fn $function(
            class: &JavaClass,
            name: &str,
            replacement: $replacement_type,
        ) -> Result<$guard_type> {
            replace_static_method(class, name, $signature, replacement as *const () as *mut c_void)
        }
    };
}

macro_rules! instance_replacement {
    (
        $(#[$meta:meta])*
        $function:ident,
        $replacement_type:ty,
        $signature:literal,
        $guard_type:ty
    ) => {
        $(#[$meta])*
        #[doc(hidden)]
        pub unsafe fn $function(
            class: &JavaClass,
            name: &str,
            replacement: $replacement_type,
        ) -> Result<$guard_type> {
            replace_instance_method(class, name, $signature, replacement as *const () as *mut c_void)
        }
    };
}

#[doc(hidden)]
pub struct MethodReplacement {
    inner: Option<ArtMethodReplacementGuard>,
}

impl MethodReplacement {
    pub fn revert(mut self) -> Result<()> {
        if let Some(mut inner) = self.inner.take() {
            inner.revert()?;
        }
        Ok(())
    }

    pub fn debug_summary(&self) -> Option<String> {
        self.inner.as_ref().map(|inner| inner.debug_summary())
    }
}

impl Drop for MethodReplacement {
    fn drop(&mut self) {
        if let Some(inner) = &mut self.inner {
            let _ = inner.revert();
        }
    }
}

#[doc(hidden)]
pub type StaticMethodReplacement = MethodReplacement;
#[doc(hidden)]
pub type StaticNoArgReplacement = MethodReplacement;
#[doc(hidden)]
pub type StaticI32Replacement = MethodReplacement;
#[doc(hidden)]
pub type InstanceMethodReplacement = MethodReplacement;
#[doc(hidden)]
pub type InstanceI32Replacement = MethodReplacement;

#[derive(Debug, Clone, Copy, PartialEq)]
#[doc(hidden)]
pub enum RawJavaReturn {
    Void,
    Boolean(jni::jboolean),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(jni::jobject),
}

/// Captures the metadata needed to call a replaced method's original implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[doc(hidden)]
pub struct OriginalMethod {
    kind: MethodKind,
    name: String,
    signature: String,
}

impl OriginalMethod {
    pub fn new(overload: &JavaMethodOverload) -> Result<Self> {
        Self::from_parts(
            overload.kind(),
            overload.name(),
            &overload.signature().to_string(),
        )
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn signature(&self) -> &str {
        &self.signature
    }

    pub unsafe fn call_static(
        &self,
        env: *mut jni::JNIEnv,
        class: jni::jclass,
        args: &[JavaValue],
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_static",
            });
        }
        unsafe { call_original_static_method(env, class, &self.name, &self.signature, args) }
    }

    pub unsafe fn call_instance(
        &self,
        env: *mut jni::JNIEnv,
        receiver: jni::jobject,
        args: &[JavaValue],
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Instance {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_instance",
            });
        }
        unsafe { call_original_instance_method(env, receiver, &self.name, &self.signature, args) }
    }

    fn from_parts(kind: MethodKind, name: &str, signature: &str) -> Result<Self> {
        if kind == MethodKind::Constructor {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::new",
            });
        }
        Ok(Self {
            kind,
            name: name.to_owned(),
            signature: MethodSignature::parse(signature)?.to_string(),
        })
    }
}

/// Replaces a selected overload using the current experimental ART backend.
///
/// This is an overload-first facade over the lower-level signature-specific helpers. It keeps the
/// replacement callback ABI explicit while letting callers reuse `JavaClassWrapper` overload
/// selection.
///
/// # Safety
///
/// The selected `implementation` function must be a valid JNI native function for `overload` and
/// must remain valid until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_method(
    overload: &JavaMethodOverload,
    implementation: MethodImplementation,
) -> Result<MethodReplacement> {
    if overload.kind() == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "experimental::replace_method",
        });
    }

    let signature = overload.signature().to_string();
    let replacement = replacement_pointer_for(overload.kind(), &signature, implementation)?;
    match overload.kind() {
        MethodKind::Static => {
            replace_static_method(overload.class(), overload.name(), &signature, replacement)
        }
        MethodKind::Instance => {
            replace_instance_method(overload.class(), overload.name(), &signature, replacement)
        }
        MethodKind::Constructor => Err(Error::WrongMethodKind {
            operation: "experimental::replace_method",
        }),
    }
}

#[doc(hidden)]
pub unsafe fn call_original_static_i32_method(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
) -> Result<jni::jint> {
    match unsafe { call_original_static_method(env, class, name, "()I", &[])? } {
        RawJavaReturn::Int(value) => Ok(value),
        other => Err(invalid_raw_return(
            "call_original_static_i32_method",
            "int",
            other,
        )),
    }
}

#[doc(hidden)]
pub unsafe fn call_original_instance_i32_method(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
) -> Result<jni::jint> {
    match unsafe { call_original_instance_method(env, receiver, name, "()I", &[])? } {
        RawJavaReturn::Int(value) => Ok(value),
        other => Err(invalid_raw_return(
            "call_original_instance_i32_method",
            "int",
            other,
        )),
    }
}

#[doc(hidden)]
pub unsafe fn call_original_static_method(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
    signature: &str,
    args: &[JavaValue],
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement class",
        });
    }

    let parsed = MethodSignature::parse(signature)?;
    parsed.validate_arguments(args)?;
    let name = CString::new(name)?;
    let signature = CString::new(signature)?;
    let get_static_method =
        unsafe { jni::env_function::<jni::GetStaticMethodId>(env, jni::ENV_GET_STATIC_METHOD_ID) };
    let method =
        unsafe { get_static_method(env.as_ptr(), class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check_pending_exception(env, "JNIEnv::GetStaticMethodID")? };
    if method.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetStaticMethodID",
        });
    }

    unsafe { call_original_static_by_return(env, class, method, parsed.return_type(), args) }
}

#[doc(hidden)]
pub unsafe fn call_original_instance_method(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
    signature: &str,
    args: &[JavaValue],
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    if receiver.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement receiver",
        });
    }

    let get_object_class =
        unsafe { jni::env_function::<jni::GetObjectClass>(env, jni::ENV_GET_OBJECT_CLASS) };
    let class = unsafe { get_object_class(env.as_ptr(), receiver) };
    unsafe { check_pending_exception(env, "JNIEnv::GetObjectClass")? };
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetObjectClass",
        });
    }

    let result = unsafe {
        let parsed = MethodSignature::parse(signature)?;
        parsed.validate_arguments(args)?;
        let name = CString::new(name)?;
        let signature = CString::new(signature)?;
        let get_method = jni::env_function::<jni::GetMethodId>(env, jni::ENV_GET_METHOD_ID);
        let method = get_method(env.as_ptr(), class, name.as_ptr(), signature.as_ptr());
        check_pending_exception(env, "JNIEnv::GetMethodID")?;
        if method.is_null() {
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetMethodID",
            });
        }

        call_original_instance_by_return(env, receiver, method, parsed.return_type(), args)
    };

    let delete_local_ref =
        unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
    unsafe { delete_local_ref(env.as_ptr(), class) };
    result
}

static_replacement!(
    /// Replaces a static Java method with signature `()V` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_void_method,
    StaticVoidReplacementFn,
    "()V",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_static_string_method,
    StaticStringReplacementFn,
    "()Ljava/lang/String;",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()Z` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_boolean_method,
    StaticBooleanReplacementFn,
    "()Z",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()B` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_byte_method,
    StaticByteReplacementFn,
    "()B",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()C` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_char_method,
    StaticCharReplacementFn,
    "()C",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()S` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_short_method,
    StaticShortReplacementFn,
    "()S",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i32_method,
    StaticI32ReplacementFn,
    "()I",
    StaticI32Replacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i64_method,
    StaticI64ReplacementFn,
    "()J",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()F` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_f32_method,
    StaticF32ReplacementFn,
    "()F",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_f64_method,
    StaticF64ReplacementFn,
    "()D",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(Ljava/lang/String;)Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_static_string_to_string_method,
    StaticStringToStringReplacementFn,
    "(Ljava/lang/String;)Ljava/lang/String;",
    StaticMethodReplacement
);

/// Replaces a static Java method with one reference argument and a reference return value.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped. Any returned object must be valid in the
/// calling JNI environment, for example a local reference created in the callback or a global
/// reference retained for the callback lifetime.
#[doc(hidden)]
pub unsafe fn replace_static_reference_to_reference_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: StaticReferenceToReferenceReplacementFn,
) -> Result<StaticMethodReplacement> {
    validate_reference_to_reference_signature(
        signature,
        "replace_static_reference_to_reference_method",
    )?;
    replace_static_method(
        class,
        name,
        signature,
        replacement as *const () as *mut c_void,
    )
}

static_replacement!(
    /// Replaces a static Java method with signature `(II)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i32_i32_to_i32_method,
    StaticI32I32ToI32ReplacementFn,
    "(II)I",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(ZBCS)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_z_b_c_s_to_i32_method,
    StaticZBCSToI32ReplacementFn,
    "(ZBCS)I",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(JD)J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i64_f64_to_i64_method,
    StaticI64F64ToI64ReplacementFn,
    "(JD)J",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(FD)D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_f32_f64_to_f64_method,
    StaticF32F64ToF64ReplacementFn,
    "(FD)D",
    StaticMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()V` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_void_method,
    InstanceVoidReplacementFn,
    "()V",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()Z` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_boolean_method,
    InstanceBooleanReplacementFn,
    "()Z",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()B` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_byte_method,
    InstanceByteReplacementFn,
    "()B",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()C` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_char_method,
    InstanceCharReplacementFn,
    "()C",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()S` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_short_method,
    InstanceShortReplacementFn,
    "()S",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i32_method,
    InstanceI32ReplacementFn,
    "()I",
    InstanceI32Replacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i64_method,
    InstanceI64ReplacementFn,
    "()J",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()F` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_f32_method,
    InstanceF32ReplacementFn,
    "()F",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_f64_method,
    InstanceF64ReplacementFn,
    "()D",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_instance_string_method,
    InstanceStringReplacementFn,
    "()Ljava/lang/String;",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(II)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i32_i32_to_i32_method,
    InstanceI32I32ToI32ReplacementFn,
    "(II)I",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(ZBCS)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_z_b_c_s_to_i32_method,
    InstanceZBCSToI32ReplacementFn,
    "(ZBCS)I",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(JD)J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i64_f64_to_i64_method,
    InstanceI64F64ToI64ReplacementFn,
    "(JD)J",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(FD)D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_f32_f64_to_f64_method,
    InstanceF32F64ToF64ReplacementFn,
    "(FD)D",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(Ljava/lang/String;)Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_instance_string_to_string_method,
    InstanceStringToStringReplacementFn,
    "(Ljava/lang/String;)Ljava/lang/String;",
    InstanceMethodReplacement
);

/// Replaces an instance Java method with one reference argument and a reference return value.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped. Any returned object must be valid in the
/// calling JNI environment, for example a local reference created in the callback or a global
/// reference retained for the callback lifetime.
#[doc(hidden)]
pub unsafe fn replace_instance_reference_to_reference_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: InstanceReferenceToReferenceReplacementFn,
) -> Result<InstanceMethodReplacement> {
    validate_reference_to_reference_signature(
        signature,
        "replace_instance_reference_to_reference_method",
    )?;
    replace_instance_method(
        class,
        name,
        signature,
        replacement as *const () as *mut c_void,
    )
}

fn replace_static_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<StaticMethodReplacement> {
    let method = class.resolve_static_method(name, signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}

fn replace_instance_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<InstanceMethodReplacement> {
    let method = class.resolve_instance_method(name, signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}

fn replacement_pointer_for(
    kind: MethodKind,
    signature: &str,
    implementation: MethodImplementation,
) -> Result<*mut c_void> {
    let signature = MethodSignature::parse(signature)?.to_string();
    match implementation {
        MethodImplementation::StaticVoid(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()V", "StaticVoid")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticString(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "()Ljava/lang/String;",
                "StaticString",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticBoolean(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()Z", "StaticBoolean")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticByte(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()B", "StaticByte")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticChar(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()C", "StaticChar")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticShort(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()S", "StaticShort")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticI32(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()I", "StaticI32")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticI64(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()J", "StaticI64")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticF32(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()F", "StaticF32")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticF64(function) => {
            expect_replacement(kind, &signature, MethodKind::Static, "()D", "StaticF64")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticStringToString(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "(Ljava/lang/String;)Ljava/lang/String;",
                "StaticStringToString",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticReferenceToReference(function) => {
            expect_reference_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "StaticReferenceToReference",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticI32I32ToI32(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "(II)I",
                "StaticI32I32ToI32",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticZBCSToI32(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "(ZBCS)I",
                "StaticZBCSToI32",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticI64F64ToI64(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "(JD)J",
                "StaticI64F64ToI64",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::StaticF32F64ToF64(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Static,
                "(FD)D",
                "StaticF32F64ToF64",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceVoid(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "()V",
                "InstanceVoid",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceBoolean(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "()Z",
                "InstanceBoolean",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceByte(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "()B",
                "InstanceByte",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceChar(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "()C",
                "InstanceChar",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceShort(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "()S",
                "InstanceShort",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceI32(function) => {
            expect_replacement(kind, &signature, MethodKind::Instance, "()I", "InstanceI32")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceI64(function) => {
            expect_replacement(kind, &signature, MethodKind::Instance, "()J", "InstanceI64")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceF32(function) => {
            expect_replacement(kind, &signature, MethodKind::Instance, "()F", "InstanceF32")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceF64(function) => {
            expect_replacement(kind, &signature, MethodKind::Instance, "()D", "InstanceF64")?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceString(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "()Ljava/lang/String;",
                "InstanceString",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceStringToString(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "(Ljava/lang/String;)Ljava/lang/String;",
                "InstanceStringToString",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceReferenceToReference(function) => {
            expect_reference_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "InstanceReferenceToReference",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceI32I32ToI32(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "(II)I",
                "InstanceI32I32ToI32",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceZBCSToI32(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "(ZBCS)I",
                "InstanceZBCSToI32",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceI64F64ToI64(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "(JD)J",
                "InstanceI64F64ToI64",
            )?;
            Ok(function as *const () as *mut c_void)
        }
        MethodImplementation::InstanceF32F64ToF64(function) => {
            expect_replacement(
                kind,
                &signature,
                MethodKind::Instance,
                "(FD)D",
                "InstanceF32F64ToF64",
            )?;
            Ok(function as *const () as *mut c_void)
        }
    }
}

fn expect_reference_replacement(
    actual_kind: MethodKind,
    actual_signature: &str,
    expected_kind: MethodKind,
    actual_implementation: &'static str,
) -> Result<()> {
    if actual_kind != expected_kind {
        return Err(replacement_mismatch(
            expected_kind,
            "one-reference-argument/reference-return",
            actual_implementation,
        ));
    }
    validate_reference_to_reference_signature(actual_signature, "experimental::replace_method")
}

fn expect_replacement(
    actual_kind: MethodKind,
    actual_signature: &str,
    expected_kind: MethodKind,
    expected_signature: &'static str,
    actual_implementation: &'static str,
) -> Result<()> {
    if actual_kind == expected_kind && actual_signature == expected_signature {
        Ok(())
    } else {
        Err(replacement_mismatch(
            expected_kind,
            expected_signature,
            actual_implementation,
        ))
    }
}

fn replacement_mismatch(
    expected_kind: MethodKind,
    expected_signature: &str,
    actual_implementation: &'static str,
) -> Error {
    Error::InvalidReplacementImplementation {
        operation: "experimental::replace_method",
        expected: format!(
            "{} method {expected_signature}",
            replacement_kind_name(expected_kind)
        ),
        actual: actual_implementation,
    }
}

fn replacement_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Constructor => "constructor",
        MethodKind::Instance => "instance",
        MethodKind::Static => "static",
    }
}

fn validate_reference_to_reference_signature(
    signature: &str,
    operation: &'static str,
) -> Result<()> {
    let parsed = MethodSignature::parse(signature)?;
    if parsed.arguments().len() != 1 {
        return Err(Error::InvalidArguments {
            expected: 1,
            actual: parsed.arguments().len(),
        });
    }
    if !parsed.arguments()[0].is_reference() {
        return Err(Error::InvalidArgumentType {
            index: 0,
            expected: "reference".to_owned(),
            actual: parsed.arguments()[0].jni_return_name(),
        });
    }
    if !parsed.return_type().is_reference() {
        return Err(Error::InvalidReturnType {
            operation,
            expected: "reference",
            actual: parsed.return_type().to_string(),
        });
    }
    Ok(())
}

fn non_null_env(env: *mut jni::JNIEnv) -> Result<NonNull<jni::JNIEnv>> {
    NonNull::new(env).ok_or(crate::Error::NullReturn {
        operation: "replacement JNIEnv",
    })
}

fn jni_args(args: &[JavaValue]) -> Vec<jni::jvalue> {
    args.iter().map(|value| value.to_jvalue()).collect()
}

fn jni_args_ptr(args: &[jni::jvalue]) -> *const jni::jvalue {
    if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    }
}

unsafe fn art_thread_from_env(env: NonNull<jni::JNIEnv>) -> usize {
    unsafe { env.as_ptr().cast::<*mut c_void>().add(1).read() as usize }
}

unsafe fn call_original_static_by_return(
    env: NonNull<jni::JNIEnv>,
    class: jni::jclass,
    method: jni::jmethodID,
    return_type: &JavaType,
    args: &[JavaValue],
) -> Result<RawJavaReturn> {
    let args = jni_args(args);
    let args = jni_args_ptr(&args);
    let thread = unsafe { art_thread_from_env(env) };
    let _bypass = original_method_call_bypass(method as usize, thread);
    let result = match return_type {
        JavaType::Void => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticVoidMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_VOID_METHOD_A,
                )
            };
            unsafe { call(env.as_ptr(), class, method, args) };
            RawJavaReturn::Void
        }
        JavaType::Boolean => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticBooleanMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_BOOLEAN_METHOD_A,
                )
            };
            RawJavaReturn::Boolean(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Byte => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticByteMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_BYTE_METHOD_A,
                )
            };
            RawJavaReturn::Byte(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Char => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticCharMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_CHAR_METHOD_A,
                )
            };
            RawJavaReturn::Char(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Short => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticShortMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_SHORT_METHOD_A,
                )
            };
            RawJavaReturn::Short(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Int => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticIntMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_INT_METHOD_A,
                )
            };
            RawJavaReturn::Int(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Long => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticLongMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_LONG_METHOD_A,
                )
            };
            RawJavaReturn::Long(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Float => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticFloatMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_FLOAT_METHOD_A,
                )
            };
            RawJavaReturn::Float(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Double => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticDoubleMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_DOUBLE_METHOD_A,
                )
            };
            RawJavaReturn::Double(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Object(_) | JavaType::Array(_) => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticObjectMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_OBJECT_METHOD_A,
                )
            };
            RawJavaReturn::Object(unsafe { call(env.as_ptr(), class, method, args) })
        }
    };
    unsafe { check_pending_exception(env, "JNIEnv::CallStaticMethodA")? };
    Ok(result)
}

unsafe fn call_original_instance_by_return(
    env: NonNull<jni::JNIEnv>,
    receiver: jni::jobject,
    method: jni::jmethodID,
    return_type: &JavaType,
    args: &[JavaValue],
) -> Result<RawJavaReturn> {
    let args = jni_args(args);
    let args = jni_args_ptr(&args);
    let thread = unsafe { art_thread_from_env(env) };
    let _bypass = original_method_call_bypass(method as usize, thread);
    let result = match return_type {
        JavaType::Void => {
            let call = unsafe {
                jni::env_function::<jni::CallVoidMethodA>(env, jni::ENV_CALL_VOID_METHOD_A)
            };
            unsafe { call(env.as_ptr(), receiver, method, args) };
            RawJavaReturn::Void
        }
        JavaType::Boolean => {
            let call = unsafe {
                jni::env_function::<jni::CallBooleanMethodA>(env, jni::ENV_CALL_BOOLEAN_METHOD_A)
            };
            RawJavaReturn::Boolean(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Byte => {
            let call = unsafe {
                jni::env_function::<jni::CallByteMethodA>(env, jni::ENV_CALL_BYTE_METHOD_A)
            };
            RawJavaReturn::Byte(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Char => {
            let call = unsafe {
                jni::env_function::<jni::CallCharMethodA>(env, jni::ENV_CALL_CHAR_METHOD_A)
            };
            RawJavaReturn::Char(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Short => {
            let call = unsafe {
                jni::env_function::<jni::CallShortMethodA>(env, jni::ENV_CALL_SHORT_METHOD_A)
            };
            RawJavaReturn::Short(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Int => {
            let call = unsafe {
                jni::env_function::<jni::CallIntMethodA>(env, jni::ENV_CALL_INT_METHOD_A)
            };
            RawJavaReturn::Int(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Long => {
            let call = unsafe {
                jni::env_function::<jni::CallLongMethodA>(env, jni::ENV_CALL_LONG_METHOD_A)
            };
            RawJavaReturn::Long(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Float => {
            let call = unsafe {
                jni::env_function::<jni::CallFloatMethodA>(env, jni::ENV_CALL_FLOAT_METHOD_A)
            };
            RawJavaReturn::Float(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Double => {
            let call = unsafe {
                jni::env_function::<jni::CallDoubleMethodA>(env, jni::ENV_CALL_DOUBLE_METHOD_A)
            };
            RawJavaReturn::Double(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Object(_) | JavaType::Array(_) => {
            let call = unsafe {
                jni::env_function::<jni::CallObjectMethodA>(env, jni::ENV_CALL_OBJECT_METHOD_A)
            };
            RawJavaReturn::Object(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
    };
    unsafe { check_pending_exception(env, "JNIEnv::CallMethodA")? };
    Ok(result)
}

fn invalid_raw_return(
    operation: &'static str,
    expected: &'static str,
    actual: RawJavaReturn,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: raw_return_type_name(actual).to_owned(),
    }
}

fn raw_return_type_name(value: RawJavaReturn) -> &'static str {
    match value {
        RawJavaReturn::Void => "void",
        RawJavaReturn::Boolean(_) => "boolean",
        RawJavaReturn::Byte(_) => "byte",
        RawJavaReturn::Char(_) => "char",
        RawJavaReturn::Short(_) => "short",
        RawJavaReturn::Int(_) => "int",
        RawJavaReturn::Long(_) => "long",
        RawJavaReturn::Float(_) => "float",
        RawJavaReturn::Double(_) => "double",
        RawJavaReturn::Object(_) => "object",
    }
}

unsafe fn check_pending_exception(
    env: NonNull<jni::JNIEnv>,
    operation: &'static str,
) -> Result<()> {
    let exception_check =
        unsafe { jni::env_function::<jni::ExceptionCheck>(env, jni::ENV_EXCEPTION_CHECK) };
    if unsafe { exception_check(env.as_ptr()) } == jni::JNI_TRUE {
        let exception_clear =
            unsafe { jni::env_function::<jni::ExceptionClear>(env, jni::ENV_EXCEPTION_CLEAR) };
        unsafe { exception_clear(env.as_ptr()) };
        Err(Error::JavaException { operation })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn static_i32(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jint {
        1
    }

    unsafe extern "C" fn static_object_echo(
        _env: *mut jni::JNIEnv,
        _class: jni::jclass,
        value: jni::jobject,
    ) -> jni::jobject {
        value
    }

    unsafe extern "C" fn instance_i32(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
    ) -> jni::jint {
        1
    }

    unsafe extern "C" fn instance_string_to_string(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        value: jni::jstring,
    ) -> jni::jstring {
        value
    }

    #[test]
    fn accepts_matching_replacement_implementations() {
        replacement_pointer_for(
            MethodKind::Static,
            "()I",
            MethodImplementation::StaticI32(static_i32),
        )
        .expect("static int implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "(Ljava/lang/String;)Ljava/lang/String;",
            MethodImplementation::InstanceStringToString(instance_string_to_string),
        )
        .expect("instance string implementation should match");

        replacement_pointer_for(
            MethodKind::Static,
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            MethodImplementation::StaticReferenceToReference(static_object_echo),
        )
        .expect("static reference implementation should match");
    }

    #[test]
    fn rejects_mismatched_replacement_implementations() {
        assert_eq!(
            replacement_pointer_for(
                MethodKind::Instance,
                "()I",
                MethodImplementation::StaticI32(static_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "StaticI32",
            })
        );

        assert_eq!(
            replacement_pointer_for(
                MethodKind::Static,
                "()I",
                MethodImplementation::InstanceI32(instance_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "instance method ()I".to_owned(),
                actual: "InstanceI32",
            })
        );
    }

    #[test]
    fn rejects_unsupported_facade_signatures() {
        assert_eq!(
            replacement_pointer_for(
                MethodKind::Static,
                "(I)I",
                MethodImplementation::StaticI32(static_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "StaticI32",
            })
        );
    }

    #[test]
    fn original_method_captures_metadata_and_rejects_constructors() {
        let original = OriginalMethod::from_parts(MethodKind::Instance, "answer", "()I")
            .expect("instance original method should be captured");
        assert_eq!(original.kind(), MethodKind::Instance);
        assert_eq!(original.name(), "answer");
        assert_eq!(original.signature(), "()I");

        assert_eq!(
            OriginalMethod::from_parts(MethodKind::Constructor, "<init>", "()V"),
            Err(Error::WrongMethodKind {
                operation: "OriginalMethod::new",
            })
        );
    }

    #[test]
    fn validates_reference_to_reference_signatures() {
        validate_reference_to_reference_signature("(Ljava/lang/Object;)Ljava/lang/Object;", "test")
            .expect("object signature should be accepted");
        validate_reference_to_reference_signature(
            "(Lfrida/java/bridge/rs/smoke/SmokeSubject;)Lfrida/java/bridge/rs/smoke/SmokeSubject;",
            "test",
        )
        .expect("custom object signature should be accepted");
        validate_reference_to_reference_signature("([I)[Ljava/lang/Object;", "test")
            .expect("array signature should be accepted");
    }

    #[test]
    fn rejects_non_reference_replacement_signatures() {
        assert_eq!(
            validate_reference_to_reference_signature("(I)Ljava/lang/Object;", "test"),
            Err(Error::InvalidArgumentType {
                index: 0,
                expected: "reference".to_owned(),
                actual: "int",
            })
        );
        assert_eq!(
            validate_reference_to_reference_signature("(Ljava/lang/Object;)I", "test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "reference",
                actual: "I".to_owned(),
            })
        );
        assert_eq!(
            validate_reference_to_reference_signature(
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                "test",
            ),
            Err(Error::InvalidArguments {
                expected: 1,
                actual: 2,
            })
        );
    }
}
