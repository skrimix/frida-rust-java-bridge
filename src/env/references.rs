use crate::{
    env::Env,
    error::{Error, Result},
    jni,
    refs::{AsJClass, AsJObject, ClassRef, GlobalRef, LocalRef},
};

impl Env<'_> {
    /// Creates a global reference that keeps a local Java object alive beyond this JNI frame.
    pub fn new_global_ref<K>(&self, object: &LocalRef<'_, K>) -> Result<GlobalRef<K>> {
        let reference = unsafe { self.new_global_ref_raw(object.as_jobject())? };
        unsafe { GlobalRef::from_raw(self.vm.clone(), reference) }
    }

    /// Returns the runtime class of a Java object.
    pub fn get_object_class(&self, object: &(impl AsJObject + ?Sized)) -> Result<ClassRef<'_>> {
        let get_object_class = self.function::<jni::GetObjectClass>(jni::ENV_GET_OBJECT_CLASS);
        let class = unsafe { get_object_class(self.handle.as_ptr(), object.as_jobject()) };
        self.check_pending_exception("JNIEnv::GetObjectClass")?;
        unsafe { LocalRef::from_raw(self.local_ref_scope(), class) }
    }

    /// Returns whether `object` is an instance of `class`.
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

    /// Returns whether two Java references point to the same Java object.
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
    /// `object` must be a valid JNI local or global reference for this process ART runtime.
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
    /// `object` must be null or a valid JNI local/global reference for this process ART runtime.
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

    pub(crate) fn push_local_frame_raw(&self, capacity: jni::jint) -> Result<()> {
        let push_local_frame = self.function::<jni::PushLocalFrame>(jni::ENV_PUSH_LOCAL_FRAME);
        let result = unsafe { push_local_frame(self.handle.as_ptr(), capacity) };
        self.check_pending_exception("JNIEnv::PushLocalFrame")?;
        Error::check_jni_result("JNIEnv::PushLocalFrame", result)
    }

    /// Pops the current JNI local frame and optionally promotes one survivor reference.
    ///
    /// # Safety
    ///
    /// A local frame must be active on this thread. `survivor` must be null or a valid local
    /// reference in the current frame.
    pub(crate) unsafe fn pop_local_frame_raw(&self, survivor: jni::jobject) -> jni::jobject {
        let pop_local_frame = self.function::<jni::PopLocalFrame>(jni::ENV_POP_LOCAL_FRAME);
        unsafe { pop_local_frame(self.handle.as_ptr(), survivor) }
    }

    /// Deletes a global JNI reference.
    ///
    /// # Safety
    ///
    /// `object` must be null or a valid global reference for this process ART runtime.
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
