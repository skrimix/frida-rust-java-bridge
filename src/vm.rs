use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::Arc,
};

use crate::{
    art::ArtMethodReplacementGuard,
    env::{AttachedEnv, Env, MethodId},
    error::{Error, Result},
    java::{ClassLoaderRef, Java, JavaChooseControl, JavaClass, JavaObject},
    jni,
    metadata::JavaMethodQueryGroup,
    runtime::{JavaCapabilities, RuntimeInner},
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
        Error::check_jni_result("JavaVM::AttachCurrentThread", result)?;

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
        Error::check_jni_result("JavaVM::DetachCurrentThread", result)
    }

    pub(crate) fn java(&self) -> Java {
        Java::new(self.clone())
    }

    pub(crate) fn capabilities(&self) -> JavaCapabilities {
        self.runtime.capabilities(self)
    }

    pub(crate) fn enumerate_class_loaders(&self) -> Result<Vec<ClassLoaderRef>> {
        self.runtime.enumerate_class_loaders(self)
    }

    pub(crate) fn enumerate_loaded_classes(&self) -> Result<Vec<JavaClass>> {
        self.runtime.enumerate_loaded_classes(self)
    }

    pub(crate) fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        self.runtime.enumerate_methods(self, query)
    }

    pub(crate) fn choose_instances(
        &self,
        class: &JavaClass,
        callback: &mut dyn FnMut(&JavaObject) -> Result<JavaChooseControl>,
    ) -> Result<()> {
        self.runtime.choose_instances(self, class, callback)
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

    pub(crate) fn gum(&self) -> &frida_gum::Gum {
        self.runtime._gum
    }

    #[cfg(test)]
    pub(crate) fn dangling_for_tests() -> Self {
        Self {
            runtime: Arc::new(RuntimeInner {
                _gum: crate::runtime::process_gum(),
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
