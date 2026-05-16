use super::*;

impl Env<'_> {
    pub fn array_length(&self, array: &impl AsJObject) -> Result<jni::jsize> {
        let get_array_length = self.function::<jni::GetArrayLength>(jni::ENV_GET_ARRAY_LENGTH);
        let length = unsafe { get_array_length(self.handle.as_ptr(), array.as_jobject()) };
        self.check_pending_exception("JNIEnv::GetArrayLength")?;
        Ok(length)
    }

    pub fn object_array_length(&self, array: &ObjectArrayRef<'_>) -> Result<jni::jsize> {
        self.array_length(array)
    }

    pub fn new_object_array(
        &self,
        length: jni::jsize,
        element_class: &impl AsJClass,
        initial_element: Option<&impl AsJObject>,
    ) -> Result<ObjectArrayRef<'_>> {
        let new_object_array = self.function::<jni::NewObjectArray>(jni::ENV_NEW_OBJECT_ARRAY);
        let initial_element = initial_element.map_or(ptr::null_mut(), |object| object.as_jobject());
        let array = unsafe {
            new_object_array(
                self.handle.as_ptr(),
                length,
                element_class.as_jclass(),
                initial_element,
            )
        };
        self.check_pending_exception("JNIEnv::NewObjectArray")?;
        unsafe { LocalRef::from_raw(self, array) }
    }

    pub fn get_object_array_element(
        &self,
        array: &ObjectArrayRef<'_>,
        index: jni::jsize,
    ) -> Result<ObjectRef<'_>> {
        self.get_object_array_element_nullable(array, index)?
            .ok_or(Error::NullReturn {
                operation: "JNIEnv::GetObjectArrayElement",
            })
    }

    pub fn get_object_array_element_nullable(
        &self,
        array: &impl AsJObject,
        index: jni::jsize,
    ) -> Result<Option<ObjectRef<'_>>> {
        let get_object_array_element =
            self.function::<jni::GetObjectArrayElement>(jni::ENV_GET_OBJECT_ARRAY_ELEMENT);
        let element =
            unsafe { get_object_array_element(self.handle.as_ptr(), array.as_jobject(), index) };
        self.check_pending_exception("JNIEnv::GetObjectArrayElement")?;
        Ok(unsafe { LocalRef::from_nullable(self, element) })
    }

    pub fn set_object_array_element(
        &self,
        array: &ObjectArrayRef<'_>,
        index: jni::jsize,
        value: Option<&impl AsJObject>,
    ) -> Result<()> {
        self.set_object_array_element_raw(array, index, value)
    }

    pub fn set_object_array_element_raw(
        &self,
        array: &impl AsJObject,
        index: jni::jsize,
        value: Option<&impl AsJObject>,
    ) -> Result<()> {
        let set_object_array_element =
            self.function::<jni::SetObjectArrayElement>(jni::ENV_SET_OBJECT_ARRAY_ELEMENT);
        let value = value.map_or(ptr::null_mut(), |object| object.as_jobject());
        unsafe { set_object_array_element(self.handle.as_ptr(), array.as_jobject(), index, value) };
        self.check_pending_exception("JNIEnv::SetObjectArrayElement")
    }

    primitive_arrays! {
        new_boolean_array, get_boolean_array_region, set_boolean_array_region, jni::jboolean,
        "JNIEnv::NewBooleanArray", jni::ENV_NEW_BOOLEAN_ARRAY,
        "JNIEnv::GetBooleanArrayRegion", jni::ENV_GET_BOOLEAN_ARRAY_REGION,
        "JNIEnv::SetBooleanArrayRegion", jni::ENV_SET_BOOLEAN_ARRAY_REGION;

        new_byte_array, get_byte_array_region, set_byte_array_region, jni::jbyte,
        "JNIEnv::NewByteArray", jni::ENV_NEW_BYTE_ARRAY,
        "JNIEnv::GetByteArrayRegion", jni::ENV_GET_BYTE_ARRAY_REGION,
        "JNIEnv::SetByteArrayRegion", jni::ENV_SET_BYTE_ARRAY_REGION;

        new_char_array, get_char_array_region, set_char_array_region, jni::jchar,
        "JNIEnv::NewCharArray", jni::ENV_NEW_CHAR_ARRAY,
        "JNIEnv::GetCharArrayRegion", jni::ENV_GET_CHAR_ARRAY_REGION,
        "JNIEnv::SetCharArrayRegion", jni::ENV_SET_CHAR_ARRAY_REGION;

        new_short_array, get_short_array_region, set_short_array_region, jni::jshort,
        "JNIEnv::NewShortArray", jni::ENV_NEW_SHORT_ARRAY,
        "JNIEnv::GetShortArrayRegion", jni::ENV_GET_SHORT_ARRAY_REGION,
        "JNIEnv::SetShortArrayRegion", jni::ENV_SET_SHORT_ARRAY_REGION;

        new_int_array, get_int_array_region, set_int_array_region, jni::jint,
        "JNIEnv::NewIntArray", jni::ENV_NEW_INT_ARRAY,
        "JNIEnv::GetIntArrayRegion", jni::ENV_GET_INT_ARRAY_REGION,
        "JNIEnv::SetIntArrayRegion", jni::ENV_SET_INT_ARRAY_REGION;

        new_long_array, get_long_array_region, set_long_array_region, jni::jlong,
        "JNIEnv::NewLongArray", jni::ENV_NEW_LONG_ARRAY,
        "JNIEnv::GetLongArrayRegion", jni::ENV_GET_LONG_ARRAY_REGION,
        "JNIEnv::SetLongArrayRegion", jni::ENV_SET_LONG_ARRAY_REGION;

        new_float_array, get_float_array_region, set_float_array_region, jni::jfloat,
        "JNIEnv::NewFloatArray", jni::ENV_NEW_FLOAT_ARRAY,
        "JNIEnv::GetFloatArrayRegion", jni::ENV_GET_FLOAT_ARRAY_REGION,
        "JNIEnv::SetFloatArrayRegion", jni::ENV_SET_FLOAT_ARRAY_REGION;

        new_double_array, get_double_array_region, set_double_array_region, jni::jdouble,
        "JNIEnv::NewDoubleArray", jni::ENV_NEW_DOUBLE_ARRAY,
        "JNIEnv::GetDoubleArrayRegion", jni::ENV_GET_DOUBLE_ARRAY_REGION,
        "JNIEnv::SetDoubleArrayRegion", jni::ENV_SET_DOUBLE_ARRAY_REGION;
    }

    fn new_primitive_array(
        &self,
        length: usize,
        slot: usize,
        operation: &'static str,
    ) -> Result<ArrayRef<'_>> {
        let new_array = self
            .function::<unsafe extern "C" fn(*mut jni::JNIEnv, jni::jsize) -> jni::jarray>(slot);
        let array = unsafe { new_array(self.handle.as_ptr(), length as jni::jsize) };
        self.check_pending_exception(operation)?;
        unsafe { LocalRef::from_raw(self, array) }
    }

    fn get_primitive_array_region<T>(
        &self,
        array: &impl AsJObject,
        start: jni::jsize,
        output: &mut [T],
        slot: usize,
        operation: &'static str,
    ) -> Result<()>
    where
        T: Copy,
    {
        if output.is_empty() {
            return Ok(());
        }
        let get_region = self.function::<unsafe extern "C" fn(
            *mut jni::JNIEnv,
            jni::jarray,
            jni::jsize,
            jni::jsize,
            *mut T,
        )>(slot);
        unsafe {
            get_region(
                self.handle.as_ptr(),
                array.as_jobject(),
                start,
                output.len() as jni::jsize,
                output.as_mut_ptr(),
            )
        };
        self.check_pending_exception(operation)
    }

    fn set_primitive_array_region<T>(
        &self,
        array: &impl AsJObject,
        start: jni::jsize,
        input: &[T],
        slot: usize,
        operation: &'static str,
    ) -> Result<()>
    where
        T: Copy,
    {
        if input.is_empty() {
            return Ok(());
        }
        let set_region = self.function::<unsafe extern "C" fn(
            *mut jni::JNIEnv,
            jni::jarray,
            jni::jsize,
            jni::jsize,
            *const T,
        )>(slot);
        unsafe {
            set_region(
                self.handle.as_ptr(),
                array.as_jobject(),
                start,
                input.len() as jni::jsize,
                input.as_ptr(),
            )
        };
        self.check_pending_exception(operation)
    }
}
