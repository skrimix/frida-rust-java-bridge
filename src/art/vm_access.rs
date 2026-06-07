use std::{ptr::NonNull, sync::Arc};

use crate::{env::AttachedEnv, error::Result, jni};

pub(crate) trait ArtVmAccess: Send + Sync {
    unsafe fn handle(&self) -> NonNull<jni::JavaVM>;
    fn attach_current_thread(&self) -> Result<AttachedEnv<'_>>;
}

#[derive(Clone)]
pub(crate) struct ArtVmHandle {
    inner: Arc<dyn ArtVmAccess>,
}

impl ArtVmHandle {
    pub(crate) fn new(vm: impl ArtVmAccess + 'static) -> Self {
        Self {
            inner: Arc::new(vm),
        }
    }
}

impl ArtVmAccess for ArtVmHandle {
    unsafe fn handle(&self) -> NonNull<jni::JavaVM> {
        unsafe { self.inner.handle() }
    }

    fn attach_current_thread(&self) -> Result<AttachedEnv<'_>> {
        self.inner.attach_current_thread()
    }
}
