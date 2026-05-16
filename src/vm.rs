use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::Arc,
};

use crate::{
    art::ArtMethodReplacementGuard,
    env::{AttachedEnv, Env, MethodId},
    error::{Error, Result},
    java::{ClassLoaderRef, Java, JavaClass, PerformHandle},
    jni,
    metadata::JavaMethodQueryGroup,
    runtime::{RuntimeCapabilities, RuntimeInner},
};

#[derive(Clone)]
pub struct Vm {
    runtime: Arc<RuntimeInner>,
}

impl Vm {
    pub(crate) fn from_runtime(runtime: Arc<RuntimeInner>) -> Self {
        Self { runtime }
    }

    pub fn handle(&self) -> NonNull<jni::JavaVM> {
        self.runtime.vm
    }

    pub fn try_get_env(&self) -> Result<Option<Env<'_>>> {
        let get_env = self.function::<jni::GetEnv>(jni::JVM_GET_ENV);
        let mut env = ptr::null_mut::<c_void>();

        // SAFETY: The function pointer is read from this JavaVM's JNI invoke table.
        let result = unsafe { get_env(self.handle().as_ptr(), &mut env, jni::JNI_VERSION_1_6) };
        match result {
            jni::JNI_OK => NonNull::new(env.cast::<jni::JNIEnv>())
                .map(|env| Some(Env::from_raw(env, self)))
                .ok_or(Error::NullReturn {
                    operation: "JavaVM::GetEnv",
                }),
            jni::JNI_EDETACHED => Ok(None),
            code => Err(Error::JniCallFailed {
                operation: "JavaVM::GetEnv",
                code,
            }),
        }
    }

    pub fn get_env(&self) -> Result<Env<'_>> {
        self.try_get_env()?.ok_or(Error::JniCallFailed {
            operation: "JavaVM::GetEnv",
            code: jni::JNI_EDETACHED,
        })
    }

    pub fn attach_current_thread(&self) -> Result<AttachedEnv<'_>> {
        if let Some(env) = self.try_get_env()? {
            return Ok(AttachedEnv::new(self, env, false));
        }

        let attach_current_thread =
            self.function::<jni::AttachCurrentThread>(jni::JVM_ATTACH_CURRENT_THREAD);
        let mut env = ptr::null_mut();

        // SAFETY: The function pointer is read from this JavaVM's JNI invoke table.
        let result =
            unsafe { attach_current_thread(self.handle().as_ptr(), &mut env, ptr::null_mut()) };
        Error::jni_result("JavaVM::AttachCurrentThread", result)?;

        let env = NonNull::new(env).ok_or(Error::NullReturn {
            operation: "JavaVM::AttachCurrentThread",
        })?;

        Ok(AttachedEnv::new(self, Env::from_raw(env, self), true))
    }

    pub fn detach_current_thread(&self) -> Result<()> {
        let detach_current_thread =
            self.function::<jni::DetachCurrentThread>(jni::JVM_DETACH_CURRENT_THREAD);

        // SAFETY: The function pointer is read from this JavaVM's JNI invoke table.
        let result = unsafe { detach_current_thread(self.handle().as_ptr()) };
        Error::jni_result("JavaVM::DetachCurrentThread", result)
    }

    pub fn java(&self) -> Java {
        Java::new(self.clone())
    }

    pub fn app_java(&self) -> Result<Java> {
        self.java().with_app_loader()
    }

    pub fn app_class_loader(&self) -> Result<ClassLoaderRef> {
        self.java().app_class_loader()
    }

    pub fn perform<F>(&self, callback: F) -> Result<PerformHandle>
    where
        F: FnOnce(Java) -> Result<()> + Send + 'static,
    {
        self.java().perform(callback)
    }

    pub fn capabilities(&self) -> RuntimeCapabilities {
        self.runtime.capabilities(self)
    }

    pub fn enumerate_class_loaders(&self) -> Result<Vec<ClassLoaderRef>> {
        self.runtime.enumerate_class_loaders(self)
    }

    pub fn enumerate_loaded_classes(&self) -> Result<Vec<JavaClass>> {
        self.runtime.enumerate_loaded_classes(self)
    }

    pub fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        self.runtime.enumerate_methods(self, query)
    }

    pub(crate) fn replace_method(
        &self,
        method: &MethodId,
        replacement: *mut c_void,
    ) -> Result<ArtMethodReplacementGuard> {
        self.runtime
            .art
            .replace_method(self, method.kind(), method.raw(), replacement)
    }

    fn function<T: Copy>(&self, slot: usize) -> T {
        unsafe { jni::vm_function(self.handle(), slot) }
    }

    #[cfg(test)]
    pub(crate) fn dangling_for_tests() -> Self {
        Self {
            runtime: Arc::new(RuntimeInner {
                _gum: frida_gum::Gum::obtain(),
                vm: NonNull::dangling(),
                flavor: crate::runtime::RuntimeFlavor::Art,
                art: crate::art::ArtBackend::empty_for_tests(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jni_detached_is_the_get_env_detached_code() {
        assert_eq!(jni::JNI_EDETACHED, -2);
    }
}
