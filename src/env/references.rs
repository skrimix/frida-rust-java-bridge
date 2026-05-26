use super::*;

impl Env<'_> {
    pub fn new_global_ref<K>(&self, object: &LocalRef<'_, K>) -> Result<GlobalRef<K>> {
        let reference = unsafe { self.new_global_ref_raw(object.as_jobject())? };
        unsafe { GlobalRef::from_raw(self.vm.clone(), reference) }
    }

    pub fn get_object_class(&self, object: &(impl AsJObject + ?Sized)) -> Result<ClassRef<'_>> {
        let get_object_class = self.function::<jni::GetObjectClass>(jni::ENV_GET_OBJECT_CLASS);
        let class = unsafe { get_object_class(self.handle.as_ptr(), object.as_jobject()) };
        self.check_pending_exception("JNIEnv::GetObjectClass")?;
        unsafe { LocalRef::from_raw(self, class) }
    }

    pub fn is_instance_of(
        &self,
        object: &(impl AsJObject + ?Sized),
        class: &impl AsJClass,
    ) -> Result<bool> {
        let is_instance_of = self.function::<jni::IsInstanceOf>(jni::ENV_IS_INSTANCE_OF);
        let result =
            unsafe { is_instance_of(self.handle.as_ptr(), object.as_jobject(), class.as_jclass()) };
        self.check_pending_exception("JNIEnv::IsInstanceOf")?;
        Ok(result == jni::JNI_TRUE)
    }

    pub fn is_same_object(
        &self,
        a: &(impl AsJObject + ?Sized),
        b: &(impl AsJObject + ?Sized),
    ) -> Result<bool> {
        let is_same_object = self.function::<jni::IsSameObject>(jni::ENV_IS_SAME_OBJECT);
        let result =
            unsafe { is_same_object(self.handle.as_ptr(), a.as_jobject(), b.as_jobject()) };
        self.check_pending_exception("JNIEnv::IsSameObject")?;
        Ok(result == jni::JNI_TRUE)
    }

    /// Creates a global reference for a JNI object.
    ///
    /// # Safety
    ///
    /// `object` must be a valid JNI local or global reference for this VM.
    pub unsafe fn new_global_ref_raw(&self, object: jni::jobject) -> Result<jni::jobject> {
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

    /// Creates a local reference for a JNI object in the current JNI frame.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid JNI local/global reference for this VM.
    pub unsafe fn new_local_ref_raw(&self, object: jni::jobject) -> Result<jni::jobject> {
        if object.is_null() {
            return Ok(std::ptr::null_mut());
        }

        let new_local_ref = self.function::<jni::NewLocalRef>(jni::ENV_NEW_LOCAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is a valid JNI reference.
        let reference = unsafe { new_local_ref(self.handle.as_ptr(), object) };
        self.check_pending_exception("JNIEnv::NewLocalRef")?;
        if reference.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::NewLocalRef",
            })
        } else {
            Ok(reference)
        }
    }

    /// Deletes a global JNI reference.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid global reference for this VM.
    pub unsafe fn delete_global_ref_raw(&self, object: jni::jobject) {
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
    pub unsafe fn delete_local_ref_raw(&self, object: jni::jobject) {
        let delete_local_ref = self.function::<jni::DeleteLocalRef>(jni::ENV_DELETE_LOCAL_REF);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table, and the caller
        // guarantees that `object` is null or a valid local reference.
        unsafe { delete_local_ref(self.handle.as_ptr(), object) };
    }
}
