use std::ffi::c_void;

use crate::{Result, art::ArtMethodReplacementGuard, java::JavaClass, jni};

pub type StaticI32ReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jint;

#[doc(hidden)]
pub struct StaticI32Replacement {
    inner: Option<ArtMethodReplacementGuard>,
}

impl StaticI32Replacement {
    pub fn revert(mut self) {
        if let Some(mut inner) = self.inner.take() {
            inner.revert();
        }
    }
}

impl Drop for StaticI32Replacement {
    fn drop(&mut self) {
        if let Some(inner) = &mut self.inner {
            inner.revert();
        }
    }
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
    let method = class.resolve_static_method(name, "()I")?;
    let replacement = replacement as *const () as *mut c_void;
    let inner = class.vm().replace_static_i32_method(&method, replacement)?;
    Ok(StaticI32Replacement { inner: Some(inner) })
}
