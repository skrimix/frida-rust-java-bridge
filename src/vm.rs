use std::{
    ffi::c_void,
    ptr::{self, NonNull},
    sync::Arc,
};

use crate::{
    art::ArtBackend,
    env::{AttachedEnv, Env},
    error::{Error, JavaThrowableOwner, Result},
    jni,
    runtime::RuntimeInner,
};

/// Low-level handle to the process Java VM.
///
/// For standard interactions (like looking up classes or calling methods), you should prefer the
/// high-level [`Java`] interface. Use `Vm` when you need direct, fine-grained control over the JNI
/// thread boundary—such as manually attaching or detaching native threads, managing thread-local
/// environment lifetimes, or calling raw JNI functions directly.
#[derive(Clone)]
pub struct Vm {
    runtime: Arc<RuntimeInner>,
}

impl Vm {
    pub(crate) fn from_runtime(runtime: Arc<RuntimeInner>) -> Self {
        Self { runtime }
    }

    /// Returns the raw JNI VM pointer.
    ///
    /// # Safety
    ///
    /// The caller must only use the returned pointer with this process' live Java VM and must
    /// uphold the JNI invocation-interface contract for any raw calls made with it.
    pub unsafe fn handle(&self) -> NonNull<jni::JavaVM> {
        self.runtime.vm
    }

    /// Returns the current thread's `JNIEnv` if it is already attached to this VM.
    ///
    /// # Safety
    ///
    /// The caller must ensure the returned [`Env`] does not outlive the current thread's JNI
    /// attachment. Prefer [`Vm::attach_current_thread`] for safe code that needs to create or
    /// borrow an attachment guard.
    pub unsafe fn try_get_env(&self) -> Result<Option<Env<'_>>> {
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

    /// Returns the current thread's `JNIEnv`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the returned [`Env`] does not outlive the current thread's JNI
    /// attachment. Prefer [`Vm::attach_current_thread`] for safe code that needs to create or
    /// borrow an attachment guard.
    pub unsafe fn get_env(&self) -> Result<Env<'_>> {
        unsafe { self.try_get_env()? }.ok_or(Error::JniCallFailed {
            operation: "JavaVM::GetEnv",
            code: jni::JNI_EDETACHED,
        })
    }

    /// Attaches the current native thread to Java VM and returns an environment guard.
    ///
    /// Interacting with Java objects requires the calling thread to be attached to the VM.
    /// - If this thread is already attached, the guard simply borrows the existing environment and does
    ///   nothing when dropped.
    /// - If the thread is not attached, this attaches it automatically, and dropping the guard will safely
    ///   detach the thread once all Java values borrowed from it go out of scope.
    pub fn attach_current_thread(&self) -> Result<AttachedEnv<'_>> {
        #[cfg(test)]
        if self.runtime.vm == NonNull::dangling() {
            return Err(Error::UnsupportedFeature {
                feature: "JavaVM::AttachCurrentThread",
                reason: "Java VM handle is unavailable in unit tests".to_owned(),
            });
        }

        // SAFETY: The returned `Env` is immediately wrapped in `AttachedEnv`, tying it to a guard
        // that keeps the borrowed attachment visible for the lexical scope.
        if let Some(env) = unsafe { self.try_get_env()? } {
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

    /// Detaches the current thread from this VM.
    ///
    /// # Safety
    ///
    /// The caller must ensure there are no live [`Env`], [`AttachedEnv`], JNI local references, or
    /// other thread-local JNI values created on the current thread. Detaching while any such value
    /// is still usable invalidates the raw `JNIEnv` and local-reference state behind safe wrappers.
    pub unsafe fn detach_current_thread(&self) -> Result<()> {
        let detach_current_thread =
            self.function::<jni::DetachCurrentThread>(jni::JVM_DETACH_CURRENT_THREAD);

        // SAFETY: The function pointer is read from this JavaVM's JNI invoke table.
        let result = unsafe { detach_current_thread(self.handle().as_ptr()) };
        Error::check_jni_result("JavaVM::DetachCurrentThread", result)
    }

    pub(crate) fn art(&self) -> &ArtBackend {
        &self.runtime.art
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
                art: crate::art::ArtBackend::empty_for_tests(),
            }),
        }
    }
}

impl JavaThrowableOwner for Vm {
    fn delete_global_throwable(&self, throwable: jni::jthrowable) {
        if let Ok(env) = self.attach_current_thread() {
            unsafe { env.delete_global_ref_raw(throwable) };
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
