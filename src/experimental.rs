use std::ffi::c_void;

use crate::{Result, art::ArtMethodReplacementGuard, java::JavaClass, jni};

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

#[doc(hidden)]
pub struct StaticMethodReplacement {
    inner: Option<ArtMethodReplacementGuard>,
}

impl StaticMethodReplacement {
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

impl Drop for StaticMethodReplacement {
    fn drop(&mut self) {
        if let Some(inner) = &mut self.inner {
            let _ = inner.revert();
        }
    }
}

#[doc(hidden)]
pub type StaticNoArgReplacement = StaticMethodReplacement;
#[doc(hidden)]
pub type StaticI32Replacement = StaticMethodReplacement;

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

fn replace_static_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<StaticMethodReplacement> {
    let method = class.resolve_static_method(name, signature)?;
    let inner = class.vm().replace_static_method(&method, replacement)?;
    Ok(StaticMethodReplacement { inner: Some(inner) })
}
