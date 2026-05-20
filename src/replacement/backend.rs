use std::ffi::c_void;

use crate::{
    Result, art::ArtMethodReplacementGuard, java::RawJavaClass, signature::MethodSignature,
};

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

pub(crate) unsafe fn replace_static_closure_trampoline_method(
    class: &RawJavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<MethodReplacement> {
    let signature = MethodSignature::parse(signature)?.to_string();
    let method = class.resolve_static_method(name, &signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}

pub(crate) unsafe fn replace_instance_closure_trampoline_method(
    class: &RawJavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<MethodReplacement> {
    let signature = MethodSignature::parse(signature)?.to_string();
    let method = class.resolve_instance_method(name, &signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}

pub(crate) unsafe fn replace_constructor_closure_trampoline_method(
    class: &RawJavaClass,
    signature: &str,
    replacement: *mut c_void,
) -> Result<MethodReplacement> {
    let signature = MethodSignature::parse(signature)?.to_string();
    let method = class.resolve_constructor(&signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}
