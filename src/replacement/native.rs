use std::{
    ffi::{CString, c_void},
    ptr,
    ptr::NonNull,
};

use crate::{
    Error, Result,
    art::{ArtMethodReplacementGuard, original_method_call_bypass},
    env::MethodKind,
    java::{IntoJavaArgs, JavaClass, JavaMethodOverload},
    jni,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
};

use super::original::{RawJavaReturn, invalid_raw_return};

pub(crate) type StaticVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass);
pub(crate) type StaticStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jstring;
pub(crate) type StaticBooleanReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jboolean;
pub(crate) type StaticByteReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jbyte;
pub(crate) type StaticCharReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jchar;
pub(crate) type StaticShortReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jshort;
pub(crate) type StaticI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jint;
pub(crate) type StaticI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jlong;
pub(crate) type StaticF32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jfloat;
pub(crate) type StaticF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jdouble;
pub(crate) type StaticStringToStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jstring) -> jni::jstring;
pub(crate) type StaticReferenceToReferenceReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jobject) -> jni::jobject;
pub(crate) type StaticI32I32ToI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jint, jni::jint) -> jni::jint;
pub(crate) type StaticZBCSToI32ReplacementFn = unsafe extern "C" fn(
    *mut jni::JNIEnv,
    jni::jclass,
    jni::jboolean,
    jni::jbyte,
    jni::jchar,
    jni::jshort,
) -> jni::jint;
pub(crate) type StaticI64F64ToI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jlong, jni::jdouble) -> jni::jlong;
pub(crate) type StaticF32F64ToF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jfloat, jni::jdouble) -> jni::jdouble;
pub(crate) type InstanceVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject);
pub(crate) type InstanceBooleanReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jboolean;
pub(crate) type InstanceByteReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jbyte;
pub(crate) type InstanceCharReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jchar;
pub(crate) type InstanceShortReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jshort;
pub(crate) type InstanceI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jint;
pub(crate) type InstanceI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jlong;
pub(crate) type InstanceF32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jfloat;
pub(crate) type InstanceF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jdouble;
pub(crate) type InstanceStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jstring;
pub(crate) type InstanceStringToStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jstring) -> jni::jstring;
pub(crate) type InstanceReferenceToReferenceReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jobject) -> jni::jobject;
pub(crate) type InstanceReferenceToVoidReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jobject);
pub(crate) type InstanceI32I32ToI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jint, jni::jint) -> jni::jint;
pub(crate) type InstanceZBCSToI32ReplacementFn = unsafe extern "C" fn(
    *mut jni::JNIEnv,
    jni::jobject,
    jni::jboolean,
    jni::jbyte,
    jni::jchar,
    jni::jshort,
) -> jni::jint;
pub(crate) type InstanceI64F64ToI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jlong, jni::jdouble) -> jni::jlong;
pub(crate) type InstanceF32F64ToF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jfloat, jni::jdouble) -> jni::jdouble;

/// A JNI-native implementation supported by the current replacement facade.
///
/// Each variant names the exact method kind and ABI shape accepted by the hidden ART backend. This
/// intentionally keeps unsupported signatures visible instead of weakening type checks.
#[derive(Clone, Copy)]
#[allow(dead_code)]
pub(crate) enum MethodImplementation {
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
    InstanceReferenceToVoid(InstanceReferenceToVoidReplacementFn),
    InstanceI32I32ToI32(InstanceI32I32ToI32ReplacementFn),
    InstanceZBCSToI32(InstanceZBCSToI32ReplacementFn),
    InstanceI64F64ToI64(InstanceI64F64ToI64ReplacementFn),
    InstanceF32F64ToF64(InstanceF32F64ToF64ReplacementFn),
}

/// A raw JNI-native implementation for a supported replacement ABI.
///
/// This is the descriptor-driven layer underneath the signature-specific helpers above. It still
/// requires an exact JNI-native callback ABI and only accepts the ABI shapes tested by the current
/// hidden backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeMethodImplementation {
    kind: MethodKind,
    signature: NativeImplementationSignature,
    function: *mut c_void,
    implementation_name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeImplementationSignature {
    Exact(String),
    OneReferenceToReference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeReplacementSignature {
    normalized: String,
    _abi: NativeReplacementAbi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeReplacementAbi {
    NoArguments,
    OneReferenceToReference,
    OneReferenceToVoid,
    ExactPrimitiveArguments,
    StartupHook,
}

impl NativeMethodImplementation {
    /// Creates a raw static-method implementation for a supported replacement signature.
    ///
    /// # Safety
    ///
    /// `function` must point to a valid JNI native function matching `signature` exactly and must
    /// remain valid until the returned replacement guard is reverted or dropped.
    #[cfg(test)]
    pub(crate) unsafe fn static_method(signature: &str, function: *mut c_void) -> Result<Self> {
        Self::new(
            MethodKind::Static,
            signature,
            function,
            "NativeMethodImplementation",
            "NativeMethodImplementation::static_method",
        )
    }

    /// Creates a raw instance-method implementation for a supported replacement signature.
    ///
    /// # Safety
    ///
    /// `function` must point to a valid JNI native function matching `signature` exactly and must
    /// remain valid until the returned replacement guard is reverted or dropped.
    pub(crate) unsafe fn instance_method(signature: &str, function: *mut c_void) -> Result<Self> {
        Self::new(
            MethodKind::Instance,
            signature,
            function,
            "NativeMethodImplementation",
            "NativeMethodImplementation::instance_method",
        )
    }

    pub(crate) fn signature(&self) -> &str {
        match &self.signature {
            NativeImplementationSignature::Exact(signature) => signature,
            NativeImplementationSignature::OneReferenceToReference => {
                "one-reference-argument/reference-return"
            }
        }
    }

    fn typed(
        kind: MethodKind,
        signature: &'static str,
        function: *mut c_void,
        implementation_name: &'static str,
    ) -> Result<Self> {
        Self::new(
            kind,
            signature,
            function,
            implementation_name,
            "replacement::replace_method",
        )
    }

    fn new(
        kind: MethodKind,
        signature: &str,
        function: *mut c_void,
        implementation_name: &'static str,
        operation: &'static str,
    ) -> Result<Self> {
        let signature = supported_replacement_signature(kind, signature, operation)?;
        Ok(Self {
            kind,
            signature: NativeImplementationSignature::Exact(signature),
            function,
            implementation_name,
        })
    }

    fn typed_reference_to_reference(
        kind: MethodKind,
        function: *mut c_void,
        implementation_name: &'static str,
    ) -> Self {
        Self {
            kind,
            signature: NativeImplementationSignature::OneReferenceToReference,
            function,
            implementation_name,
        }
    }
}

impl MethodImplementation {
    fn into_native(self) -> Result<NativeMethodImplementation> {
        match self {
            Self::StaticVoid(function) => typed_native(
                MethodKind::Static,
                "()V",
                function as *const () as *mut c_void,
                "StaticVoid",
            ),
            Self::StaticString(function) => typed_native(
                MethodKind::Static,
                "()Ljava/lang/String;",
                function as *const () as *mut c_void,
                "StaticString",
            ),
            Self::StaticBoolean(function) => typed_native(
                MethodKind::Static,
                "()Z",
                function as *const () as *mut c_void,
                "StaticBoolean",
            ),
            Self::StaticByte(function) => typed_native(
                MethodKind::Static,
                "()B",
                function as *const () as *mut c_void,
                "StaticByte",
            ),
            Self::StaticChar(function) => typed_native(
                MethodKind::Static,
                "()C",
                function as *const () as *mut c_void,
                "StaticChar",
            ),
            Self::StaticShort(function) => typed_native(
                MethodKind::Static,
                "()S",
                function as *const () as *mut c_void,
                "StaticShort",
            ),
            Self::StaticI32(function) => typed_native(
                MethodKind::Static,
                "()I",
                function as *const () as *mut c_void,
                "StaticI32",
            ),
            Self::StaticI64(function) => typed_native(
                MethodKind::Static,
                "()J",
                function as *const () as *mut c_void,
                "StaticI64",
            ),
            Self::StaticF32(function) => typed_native(
                MethodKind::Static,
                "()F",
                function as *const () as *mut c_void,
                "StaticF32",
            ),
            Self::StaticF64(function) => typed_native(
                MethodKind::Static,
                "()D",
                function as *const () as *mut c_void,
                "StaticF64",
            ),
            Self::StaticStringToString(function) => typed_native(
                MethodKind::Static,
                "(Ljava/lang/String;)Ljava/lang/String;",
                function as *const () as *mut c_void,
                "StaticStringToString",
            ),
            Self::StaticReferenceToReference(function) => Ok(typed_reference_native(
                MethodKind::Static,
                function as *const () as *mut c_void,
                "StaticReferenceToReference",
            )),
            Self::StaticI32I32ToI32(function) => typed_native(
                MethodKind::Static,
                "(II)I",
                function as *const () as *mut c_void,
                "StaticI32I32ToI32",
            ),
            Self::StaticZBCSToI32(function) => typed_native(
                MethodKind::Static,
                "(ZBCS)I",
                function as *const () as *mut c_void,
                "StaticZBCSToI32",
            ),
            Self::StaticI64F64ToI64(function) => typed_native(
                MethodKind::Static,
                "(JD)J",
                function as *const () as *mut c_void,
                "StaticI64F64ToI64",
            ),
            Self::StaticF32F64ToF64(function) => typed_native(
                MethodKind::Static,
                "(FD)D",
                function as *const () as *mut c_void,
                "StaticF32F64ToF64",
            ),
            Self::InstanceVoid(function) => typed_native(
                MethodKind::Instance,
                "()V",
                function as *const () as *mut c_void,
                "InstanceVoid",
            ),
            Self::InstanceBoolean(function) => typed_native(
                MethodKind::Instance,
                "()Z",
                function as *const () as *mut c_void,
                "InstanceBoolean",
            ),
            Self::InstanceByte(function) => typed_native(
                MethodKind::Instance,
                "()B",
                function as *const () as *mut c_void,
                "InstanceByte",
            ),
            Self::InstanceChar(function) => typed_native(
                MethodKind::Instance,
                "()C",
                function as *const () as *mut c_void,
                "InstanceChar",
            ),
            Self::InstanceShort(function) => typed_native(
                MethodKind::Instance,
                "()S",
                function as *const () as *mut c_void,
                "InstanceShort",
            ),
            Self::InstanceI32(function) => typed_native(
                MethodKind::Instance,
                "()I",
                function as *const () as *mut c_void,
                "InstanceI32",
            ),
            Self::InstanceI64(function) => typed_native(
                MethodKind::Instance,
                "()J",
                function as *const () as *mut c_void,
                "InstanceI64",
            ),
            Self::InstanceF32(function) => typed_native(
                MethodKind::Instance,
                "()F",
                function as *const () as *mut c_void,
                "InstanceF32",
            ),
            Self::InstanceF64(function) => typed_native(
                MethodKind::Instance,
                "()D",
                function as *const () as *mut c_void,
                "InstanceF64",
            ),
            Self::InstanceString(function) => typed_native(
                MethodKind::Instance,
                "()Ljava/lang/String;",
                function as *const () as *mut c_void,
                "InstanceString",
            ),
            Self::InstanceStringToString(function) => typed_native(
                MethodKind::Instance,
                "(Ljava/lang/String;)Ljava/lang/String;",
                function as *const () as *mut c_void,
                "InstanceStringToString",
            ),
            Self::InstanceReferenceToReference(function) => Ok(typed_reference_native(
                MethodKind::Instance,
                function as *const () as *mut c_void,
                "InstanceReferenceToReference",
            )),
            Self::InstanceReferenceToVoid(function) => typed_native(
                MethodKind::Instance,
                "(Ljava/lang/Object;)V",
                function as *const () as *mut c_void,
                "InstanceReferenceToVoid",
            ),
            Self::InstanceI32I32ToI32(function) => typed_native(
                MethodKind::Instance,
                "(II)I",
                function as *const () as *mut c_void,
                "InstanceI32I32ToI32",
            ),
            Self::InstanceZBCSToI32(function) => typed_native(
                MethodKind::Instance,
                "(ZBCS)I",
                function as *const () as *mut c_void,
                "InstanceZBCSToI32",
            ),
            Self::InstanceI64F64ToI64(function) => typed_native(
                MethodKind::Instance,
                "(JD)J",
                function as *const () as *mut c_void,
                "InstanceI64F64ToI64",
            ),
            Self::InstanceF32F64ToF64(function) => typed_native(
                MethodKind::Instance,
                "(FD)D",
                function as *const () as *mut c_void,
                "InstanceF32F64ToF64",
            ),
        }
    }
}

fn typed_native(
    kind: MethodKind,
    signature: &'static str,
    function: *mut c_void,
    implementation_name: &'static str,
) -> Result<NativeMethodImplementation> {
    NativeMethodImplementation::typed(kind, signature, function, implementation_name)
}

fn typed_reference_native(
    kind: MethodKind,
    function: *mut c_void,
    implementation_name: &'static str,
) -> NativeMethodImplementation {
    NativeMethodImplementation::typed_reference_to_reference(kind, function, implementation_name)
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
        pub(crate) unsafe fn $function(
            class: &JavaClass,
            name: &str,
            replacement: $replacement_type,
        ) -> Result<$guard_type> {
            unsafe {
                replace_static_native_method(
                    class,
                    name,
                    $signature,
                    replacement as *const () as *mut c_void,
                )
            }
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
        pub(crate) unsafe fn $function(
            class: &JavaClass,
            name: &str,
            replacement: $replacement_type,
        ) -> Result<$guard_type> {
            unsafe {
                replace_instance_native_method(
                    class,
                    name,
                    $signature,
                    replacement as *const () as *mut c_void,
                )
            }
        }
    };
}

pub(crate) struct MethodReplacement {
    inner: Option<ArtMethodReplacementGuard>,
}

impl MethodReplacement {
    pub(crate) fn revert(&mut self) -> Result<()> {
        if let Some(mut inner) = self.inner.take()
            && let Err(error) = inner.revert()
        {
            self.inner = Some(inner);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn debug_summary(&self) -> Option<String> {
        self.inner.as_ref().map(|inner| inner.debug_summary())
    }
}

impl Drop for MethodReplacement {
    fn drop(&mut self) {
        if let Some(mut inner) = self.inner.take()
            && inner.revert().is_err()
        {
            std::mem::forget(inner);
        }
    }
}

#[doc(hidden)]
pub(crate) type StaticMethodReplacement = MethodReplacement;
#[doc(hidden)]
pub(crate) type StaticI32Replacement = MethodReplacement;
#[doc(hidden)]
pub(crate) type InstanceMethodReplacement = MethodReplacement;
#[doc(hidden)]
pub(crate) type InstanceI32Replacement = MethodReplacement;

pub(crate) unsafe fn replace_method(
    overload: &JavaMethodOverload,
    implementation: MethodImplementation,
) -> Result<MethodReplacement> {
    unsafe { replace_native_method(overload, implementation.into_native()?) }
}

/// Replaces a selected overload using a descriptor-driven raw JNI-native implementation.
///
/// # Safety
///
/// The selected `implementation` function must be a valid JNI native function for `overload` and
/// must remain valid until the returned guard is reverted or dropped.
#[doc(hidden)]
pub(crate) unsafe fn replace_native_method(
    overload: &JavaMethodOverload,
    implementation: NativeMethodImplementation,
) -> Result<MethodReplacement> {
    if overload.kind() == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "replacement::replace_native_method",
        });
    }

    let signature = overload.signature().to_string();
    let replacement = native_replacement_pointer_for(overload.kind(), &signature, implementation)?;
    match overload.kind() {
        MethodKind::Static => unsafe {
            replace_static_native_method(overload.class(), overload.name(), &signature, replacement)
        },
        MethodKind::Instance => unsafe {
            replace_instance_native_method(
                overload.class(),
                overload.name(),
                &signature,
                replacement,
            )
        },
        MethodKind::Constructor => Err(Error::WrongMethodKind {
            operation: "replacement::replace_native_method",
        }),
    }
}

pub(crate) unsafe fn call_original_static_i32_method(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
) -> Result<jni::jint> {
    match unsafe { call_original_static_method(env, class, name, "()I", [])? } {
        RawJavaReturn::Int(value) => Ok(value),
        other => Err(invalid_raw_return(
            "call_original_static_i32_method",
            "int",
            other,
        )),
    }
}

#[doc(hidden)]
pub(crate) unsafe fn call_original_instance_i32_method(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
) -> Result<jni::jint> {
    match unsafe { call_original_instance_method(env, receiver, name, "()I", [])? } {
        RawJavaReturn::Int(value) => Ok(value),
        other => Err(invalid_raw_return(
            "call_original_instance_i32_method",
            "int",
            other,
        )),
    }
}

#[doc(hidden)]
pub(crate) unsafe fn call_original_static_method<A: IntoJavaArgs>(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
    signature: &str,
    args: A,
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement class",
        });
    }

    let (parsed, args) = prepare_original_call_args(signature, args)?;
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

    unsafe { call_original_static_by_return(env, class, method, parsed.return_type(), &args) }
}

#[doc(hidden)]
pub(crate) unsafe fn call_original_instance_method<A: IntoJavaArgs>(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
    signature: &str,
    args: A,
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
        let (parsed, args) = prepare_original_call_args(signature, args)?;
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

        call_original_instance_by_return(env, receiver, method, parsed.return_type(), &args)
    };

    let delete_local_ref =
        unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
    unsafe { delete_local_ref(env.as_ptr(), class) };
    result
}

static_replacement!(
    /// Replaces a static Java method with signature `()V` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()Ljava/lang/String;` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()Z` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()B` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()C` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()S` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()I` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()J` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()F` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `()D` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `(Ljava/lang/String;)Ljava/lang/String;` using the current replacement ART backend.
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
pub(crate) unsafe fn replace_static_reference_to_reference_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: StaticReferenceToReferenceReplacementFn,
) -> Result<StaticMethodReplacement> {
    unsafe {
        replace_static_native_method(
            class,
            name,
            signature,
            replacement as *const () as *mut c_void,
        )
    }
}

/// Replaces a static Java method with a raw JNI-native callback for a supported signature.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for `signature` and must remain valid until
/// the returned guard is reverted or dropped.
#[doc(hidden)]
pub(crate) unsafe fn replace_static_native_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<StaticMethodReplacement> {
    let signature = supported_replacement_signature(
        MethodKind::Static,
        signature,
        "replace_static_native_method",
    )?;
    replace_static_method(class, name, &signature, replacement)
}

static_replacement!(
    /// Replaces a static Java method with signature `(II)I` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `(ZBCS)I` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `(JD)J` using the current replacement ART backend.
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
    /// Replaces a static Java method with signature `(FD)D` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()V` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()Z` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()B` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()C` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()S` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()I` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()J` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()F` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()D` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `()Ljava/lang/String;` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `(II)I` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `(ZBCS)I` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `(JD)J` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `(FD)D` using the current replacement ART backend.
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
    /// Replaces an instance Java method with signature `(Ljava/lang/String;)Ljava/lang/String;` using the current replacement ART backend.
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
pub(crate) unsafe fn replace_instance_reference_to_reference_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: InstanceReferenceToReferenceReplacementFn,
) -> Result<InstanceMethodReplacement> {
    unsafe {
        replace_instance_native_method(
            class,
            name,
            signature,
            replacement as *const () as *mut c_void,
        )
    }
}

/// Replaces an instance Java method with one reference argument and a `void` return value.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
#[allow(dead_code)]
pub(crate) unsafe fn replace_instance_reference_to_void_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: InstanceReferenceToVoidReplacementFn,
) -> Result<InstanceMethodReplacement> {
    unsafe {
        replace_instance_native_method(
            class,
            name,
            signature,
            replacement as *const () as *mut c_void,
        )
    }
}

/// Replaces an instance Java method with a raw JNI-native callback for a supported signature.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for `signature` and must remain valid until
/// the returned guard is reverted or dropped.
#[doc(hidden)]
pub(crate) unsafe fn replace_instance_native_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<InstanceMethodReplacement> {
    let signature = supported_replacement_signature(
        MethodKind::Instance,
        signature,
        "replace_instance_native_method",
    )?;
    replace_instance_method(class, name, &signature, replacement)
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

#[cfg(test)]
pub(crate) fn replacement_pointer_for(
    kind: MethodKind,
    signature: &str,
    implementation: MethodImplementation,
) -> Result<*mut c_void> {
    native_replacement_pointer_for(kind, signature, implementation.into_native()?)
}

pub(crate) fn native_replacement_pointer_for(
    actual_kind: MethodKind,
    actual_signature: &str,
    implementation: NativeMethodImplementation,
) -> Result<*mut c_void> {
    let actual_signature = MethodSignature::parse(actual_signature)?.to_string();
    if actual_kind != implementation.kind {
        return Err(replacement_mismatch(
            implementation.kind,
            implementation.signature(),
            implementation.implementation_name,
        ));
    }

    match &implementation.signature {
        NativeImplementationSignature::Exact(expected_signature)
            if actual_signature == expected_signature.as_str() =>
        {
            supported_replacement_signature(
                actual_kind,
                &actual_signature,
                "replacement::replace_method",
            )?;
            Ok(implementation.function)
        }
        NativeImplementationSignature::Exact(expected_signature) => Err(replacement_mismatch(
            implementation.kind,
            expected_signature,
            implementation.implementation_name,
        )),
        NativeImplementationSignature::OneReferenceToReference => {
            validate_reference_to_reference_signature(
                &actual_signature,
                "replacement::replace_method",
            )?;
            Ok(implementation.function)
        }
    }
}

pub(crate) fn supported_replacement_signature(
    kind: MethodKind,
    signature: &str,
    operation: &'static str,
) -> Result<String> {
    classify_native_replacement_signature(kind, signature, operation)
        .map(|signature| signature.normalized)
}

fn classify_native_replacement_signature(
    kind: MethodKind,
    signature: &str,
    operation: &'static str,
) -> Result<NativeReplacementSignature> {
    let parsed = MethodSignature::parse(signature)?;
    let args = parsed.arguments();
    let return_type = parsed.return_type();
    let abi = if startup_hook_abi_is_supported(&parsed) {
        NativeReplacementAbi::StartupHook
    } else if args.is_empty()
        && (matches!(
            return_type,
            JavaType::Void
                | JavaType::Boolean
                | JavaType::Byte
                | JavaType::Char
                | JavaType::Short
                | JavaType::Int
                | JavaType::Long
                | JavaType::Float
                | JavaType::Double
        ) || is_java_lang_string(return_type))
    {
        NativeReplacementAbi::NoArguments
    } else if args.len() == 1 && args[0].is_reference() && return_type.is_reference() {
        NativeReplacementAbi::OneReferenceToReference
    } else if args.len() == 1 && args[0].is_reference() && return_type == &JavaType::Void {
        NativeReplacementAbi::OneReferenceToVoid
    } else if matches!(
        parsed.to_string().as_str(),
        "(II)I" | "(ZBCS)I" | "(JD)J" | "(FD)D"
    ) {
        NativeReplacementAbi::ExactPrimitiveArguments
    } else {
        return Err(Error::InvalidReplacementImplementation {
            operation,
            expected: format!(
                "supported {} method replacement ABI",
                replacement_kind_name(kind)
            ),
            actual: "NativeMethodImplementation",
        });
    };
    Ok(NativeReplacementSignature {
        normalized: parsed.to_string(),
        _abi: abi,
    })
}

fn startup_hook_abi_is_supported(signature: &MethodSignature) -> bool {
    matches!(
        signature.to_string().as_str(),
        "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZZ)Landroid/app/LoadedApk;"
            | "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZ)Landroid/app/LoadedApk;"
            | "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;"
            | "(Ljava/lang/String;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;"
            | "(ZLandroid/app/Instrumentation;)Landroid/app/Application;"
            | "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZZ)Ljava/lang/Object;"
            | "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;"
            | "(Ljava/lang/Object;Ljava/lang/Object;I)Ljava/lang/Object;"
            | "(Ljava/lang/String;Ljava/lang/Object;I)Ljava/lang/Object;"
            | "(ZLjava/lang/Object;)Ljava/lang/Object;"
    )
}

fn is_java_lang_string(ty: &JavaType) -> bool {
    matches!(ty, JavaType::Object(name) if name == "java/lang/String")
}

pub(super) fn replacement_mismatch(
    expected_kind: MethodKind,
    expected_signature: &str,
    actual_implementation: &'static str,
) -> Error {
    Error::InvalidReplacementImplementation {
        operation: "replacement::replace_method",
        expected: format!(
            "{} method {expected_signature}",
            replacement_kind_name(expected_kind)
        ),
        actual: actual_implementation,
    }
}

pub(super) fn replacement_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Constructor => "constructor",
        MethodKind::Instance => "instance",
        MethodKind::Static => "static",
    }
}

pub(crate) fn validate_reference_to_reference_signature(
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

pub(crate) fn prepare_original_call_args<A: IntoJavaArgs>(
    signature: &str,
    args: A,
) -> Result<(MethodSignature, Vec<JavaValue>)> {
    let parsed = MethodSignature::parse(signature)?;
    let args = args.into_java_args();
    parsed.validate_arguments(&args)?;
    Ok((parsed, args))
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

unsafe fn art_thread_from_env(env: NonNull<jni::JNIEnv>) -> Result<usize> {
    let thread = unsafe { env.as_ptr().cast::<*mut c_void>().add(1).read() as usize };
    if thread == 0 {
        Err(Error::NullReturn {
            operation: "replacement ART thread",
        })
    } else {
        Ok(thread)
    }
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
    let thread = unsafe { art_thread_from_env(env)? };
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
    let thread = unsafe { art_thread_from_env(env)? };
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
