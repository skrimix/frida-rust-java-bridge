use std::ffi::c_void;

use crate::{Result, art::ArtMethodReplacementGuard, java::JavaClass, jni};

pub type StaticVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass);
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

#[doc(hidden)]
pub struct StaticNoArgReplacement {
    inner: Option<ArtMethodReplacementGuard>,
}

#[doc(hidden)]
pub type StaticI32Replacement = StaticNoArgReplacement;

impl StaticNoArgReplacement {
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

impl Drop for StaticNoArgReplacement {
    fn drop(&mut self) {
        if let Some(inner) = &mut self.inner {
            let _ = inner.revert();
        }
    }
}

/// Replaces a static Java method with signature `()V` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_void_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticVoidReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()V", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()Z` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_boolean_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticBooleanReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()Z", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()B` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_byte_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticByteReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()B", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()C` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_char_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticCharReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()C", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()S` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_short_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticShortReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()S", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()I` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_i32_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticI32ReplacementFn,
) -> Result<StaticI32Replacement> {
    replace_static_no_arg_method(class, name, "()I", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()J` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_i64_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticI64ReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()J", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()F` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_f32_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticF32ReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()F", replacement as *const () as *mut c_void)
}

/// Replaces a static Java method with signature `()D` using the current experimental ART backend.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_f64_method(
    class: &JavaClass,
    name: &str,
    replacement: StaticF64ReplacementFn,
) -> Result<StaticNoArgReplacement> {
    replace_static_no_arg_method(class, name, "()D", replacement as *const () as *mut c_void)
}

fn replace_static_no_arg_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<StaticNoArgReplacement> {
    let method = class.resolve_static_method(name, signature)?;
    let inner = class
        .vm()
        .replace_static_no_arg_method(&method, replacement)?;
    Ok(StaticNoArgReplacement { inner: Some(inner) })
}
