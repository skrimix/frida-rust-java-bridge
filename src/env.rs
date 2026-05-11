use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
};

use crate::{
    error::{Error, Result},
    jni,
    vm::Vm,
};

#[derive(Clone, Copy)]
pub struct Env<'vm> {
    handle: NonNull<jni::JNIEnv>,
    vm: &'vm Vm,
    _thread_affine: PhantomData<Rc<()>>,
}

pub struct AttachedEnv<'vm> {
    env: Env<'vm>,
    vm: &'vm Vm,
    detach_on_drop: bool,
}

impl<'vm> Env<'vm> {
    pub(crate) fn from_raw(handle: NonNull<jni::JNIEnv>, vm: &'vm Vm) -> Self {
        Self {
            handle,
            vm,
            _thread_affine: PhantomData,
        }
    }

    pub fn handle(&self) -> NonNull<jni::JNIEnv> {
        self.handle
    }

    pub fn vm(&self) -> &'vm Vm {
        self.vm
    }

    pub fn version(&self) -> jni::jint {
        let get_version = self.function::<jni::GetVersion>(jni::ENV_GET_VERSION);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { get_version(self.handle.as_ptr()) }
    }

    pub fn find_class(&self, name: &str) -> Result<jni::jclass> {
        let name = CString::new(name)?;
        let find_class = self.function::<jni::FindClass>(jni::ENV_FIND_CLASS);

        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        let class = unsafe { find_class(self.handle.as_ptr(), name.as_ptr()) };
        self.check_pending_exception("JNIEnv::FindClass")?;

        if class.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::FindClass",
            })
        } else {
            Ok(class)
        }
    }

    pub fn new_string_utf(&self, text: &str) -> Result<jni::jstring> {
        let text = CString::new(text)?;
        let new_string_utf = self.function::<jni::NewStringUtf>(jni::ENV_NEW_STRING_UTF);

        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        let string = unsafe { new_string_utf(self.handle.as_ptr(), text.as_ptr()) };
        self.check_pending_exception("JNIEnv::NewStringUTF")?;

        if string.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::NewStringUTF",
            })
        } else {
            Ok(string)
        }
    }

    /// Copies a Java string into a Rust `String`.
    ///
    /// # Safety
    ///
    /// `string` must be a valid `jstring` local or global reference for this VM.
    pub unsafe fn get_string_utf(&self, string: jni::jstring) -> Result<String> {
        let get_string_utf_chars =
            self.function::<jni::GetStringUtfChars>(jni::ENV_GET_STRING_UTF_CHARS);
        let release_string_utf_chars =
            self.function::<jni::ReleaseStringUtfChars>(jni::ENV_RELEASE_STRING_UTF_CHARS);
        let mut is_copy = jni::JNI_FALSE;

        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and `string`
        // is expected to be a valid jstring owned by the caller.
        let chars = unsafe { get_string_utf_chars(self.handle.as_ptr(), string, &mut is_copy) };
        if chars.is_null() {
            self.check_pending_exception("JNIEnv::GetStringUTFChars")?;
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetStringUTFChars",
            });
        }

        // SAFETY: JNI returned a non-null, NUL-terminated modified UTF-8 buffer.
        let result = unsafe { CStr::from_ptr(chars) }
            .to_str()
            .map(str::to_owned)
            .map_err(Error::from);

        // SAFETY: The buffer came from GetStringUTFChars for the same jstring/env pair.
        unsafe { release_string_utf_chars(self.handle.as_ptr(), string, chars) };

        result
    }

    pub fn exception_check(&self) -> bool {
        let exception_check = self.function::<jni::ExceptionCheck>(jni::ENV_EXCEPTION_CHECK);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_check(self.handle.as_ptr()) == jni::JNI_TRUE }
    }

    pub fn exception_clear(&self) {
        let exception_clear = self.function::<jni::ExceptionClear>(jni::ENV_EXCEPTION_CLEAR);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_clear(self.handle.as_ptr()) };
    }

    /// Returns the pending exception local reference, if any.
    ///
    /// # Safety
    ///
    /// The returned local reference follows JNI local reference rules and must only be used on
    /// the current attached thread.
    pub unsafe fn exception_occurred(&self) -> jni::jthrowable {
        let exception_occurred =
            self.function::<jni::ExceptionOccurred>(jni::ENV_EXCEPTION_OCCURRED);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_occurred(self.handle.as_ptr()) }
    }

    /// Creates a global reference for a JNI object.
    ///
    /// # Safety
    ///
    /// `object` must be a valid JNI local or global reference for this VM.
    pub unsafe fn new_global_ref(&self, object: jni::jobject) -> Result<jni::jobject> {
        let new_global_ref = self.function::<jni::NewGlobalRef>(jni::ENV_NEW_GLOBAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is a valid JNI reference.
        let reference = unsafe { new_global_ref(self.handle.as_ptr(), object) };
        self.check_pending_exception("JNIEnv::NewGlobalRef")?;

        if object.is_null() || !reference.is_null() {
            Ok(reference)
        } else {
            Err(Error::NullReturn {
                operation: "JNIEnv::NewGlobalRef",
            })
        }
    }

    /// Deletes a global JNI reference.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid global reference for this VM.
    pub unsafe fn delete_global_ref(&self, object: jni::jobject) {
        let delete_global_ref = self.function::<jni::DeleteGlobalRef>(jni::ENV_DELETE_GLOBAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is null or a valid global reference.
        unsafe { delete_global_ref(self.handle.as_ptr(), object) };
    }

    /// Deletes a local JNI reference.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid local reference on the current JNI frame.
    pub unsafe fn delete_local_ref(&self, object: jni::jobject) {
        let delete_local_ref = self.function::<jni::DeleteLocalRef>(jni::ENV_DELETE_LOCAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is null or a valid local reference.
        unsafe { delete_local_ref(self.handle.as_ptr(), object) };
    }

    fn check_pending_exception(&self, operation: &'static str) -> Result<()> {
        if self.exception_check() {
            self.exception_clear();
            Err(Error::JavaException { operation })
        } else {
            Ok(())
        }
    }

    fn function<T: Copy>(&self, slot: usize) -> T {
        unsafe { jni::env_function(self.handle, slot) }
    }
}

impl<'vm> AttachedEnv<'vm> {
    pub(crate) fn new(vm: &'vm Vm, env: Env<'vm>, detach_on_drop: bool) -> Self {
        Self {
            env,
            vm,
            detach_on_drop,
        }
    }

    pub fn env(&self) -> Env<'vm> {
        self.env
    }

    pub fn detach_on_drop(&self) -> bool {
        self.detach_on_drop
    }
}

impl<'vm> std::ops::Deref for AttachedEnv<'vm> {
    type Target = Env<'vm>;

    fn deref(&self) -> &Self::Target {
        &self.env
    }
}

impl Drop for AttachedEnv<'_> {
    fn drop(&mut self) {
        if self.detach_on_drop {
            let _ = self.vm.detach_current_thread();
        }
    }
}
