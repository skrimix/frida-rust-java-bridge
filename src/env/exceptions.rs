use super::*;

impl Env<'_> {
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

    pub fn exception_occurred(&self) -> Option<ThrowableRef<'_>> {
        let throwable = unsafe { self.exception_occurred_raw() };
        unsafe { LocalRef::from_nullable(self, throwable) }
    }

    /// Returns the pending exception local reference, if any.
    ///
    /// # Safety
    ///
    /// The returned local reference follows JNI local reference rules and must only be used on
    /// the current attached thread.
    pub unsafe fn exception_occurred_raw(&self) -> jni::jthrowable {
        let exception_occurred =
            self.function::<jni::ExceptionOccurred>(jni::ENV_EXCEPTION_OCCURRED);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_occurred(self.handle.as_ptr()) }
    }
}
