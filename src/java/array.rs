use super::*;

impl JavaArray {
    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.array.as_jobject()
    }

    pub fn element_type(&self) -> &JavaType {
        &self.element_type
    }

    pub fn len(&self) -> Result<jni::jsize> {
        array_len(&self.vm, self)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn as_object(&self) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        object_from_ref(&env, &self.vm, self)
    }

    pub fn into_object(self) -> Result<JavaObject> {
        let JavaArray { vm, array, .. } = self;
        let raw = array.into_raw();
        let object = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        Ok(JavaObject { vm, object })
    }

    pub fn get_object(&self, index: jni::jsize) -> Result<Option<JavaObject>> {
        get_array_object(
            &self.vm,
            self,
            &self.element_type,
            index,
            "JavaArray::get_object",
        )
    }

    pub fn set_object<T: AsJObject + ?Sized>(
        &self,
        index: jni::jsize,
        value: Option<&T>,
    ) -> Result<()> {
        set_array_object(
            &self.vm,
            self,
            &self.element_type,
            index,
            value,
            "JavaArray::set_object",
        )
    }

    pub fn get_booleans(&self) -> Result<Vec<bool>> {
        get_boolean_array(
            &self.vm,
            self,
            &self.element_type,
            "JavaArray::get_booleans",
        )
    }

    pub fn set_booleans(&self, values: &[bool]) -> Result<()> {
        set_boolean_array(
            &self.vm,
            self,
            &self.element_type,
            values,
            "JavaArray::set_booleans",
        )
    }

    java_primitive_array_accessors! {
        "JavaArray";

        get_bytes, set_bytes, jni::jbyte, JavaType::Byte,
        get_byte_array_region, set_byte_array_region;

        get_chars, set_chars, jni::jchar, JavaType::Char,
        get_char_array_region, set_char_array_region;

        get_shorts, set_shorts, jni::jshort, JavaType::Short,
        get_short_array_region, set_short_array_region;

        get_ints, set_ints, jni::jint, JavaType::Int,
        get_int_array_region, set_int_array_region;

        get_longs, set_longs, jni::jlong, JavaType::Long,
        get_long_array_region, set_long_array_region;

        get_floats, set_floats, jni::jfloat, JavaType::Float,
        get_float_array_region, set_float_array_region;

        get_doubles, set_doubles, jni::jdouble, JavaType::Double,
        get_double_array_region, set_double_array_region;
    }
}

impl<'local> JavaLocalArray<'local> {
    pub(crate) unsafe fn from_raw(
        vm: Vm,
        raw: jni::jobject,
        element_type: JavaType,
    ) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JNI local array view",
            });
        }

        Ok(Self {
            vm,
            array: raw,
            element_type,
            _local: PhantomData,
            _thread_affine: PhantomData,
        })
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.array
    }

    pub fn element_type(&self) -> &JavaType {
        &self.element_type
    }

    pub fn len(&self) -> Result<jni::jsize> {
        array_len(&self.vm, self)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn retain(&self) -> Result<JavaArray> {
        let env = self.vm.attach_current_thread()?;
        array_from_ref(&env, &self.vm, self, self.element_type.clone())
    }

    pub fn as_object(&self) -> Result<JavaLocalObject<'local>> {
        unsafe { JavaLocalObject::from_raw(self.vm.clone(), self.as_jobject()) }
    }

    pub fn get_object(&self, index: jni::jsize) -> Result<Option<JavaObject>> {
        get_array_object(
            &self.vm,
            self,
            &self.element_type,
            index,
            "JavaLocalArray::get_object",
        )
    }

    pub fn set_object<T: AsJObject + ?Sized>(
        &self,
        index: jni::jsize,
        value: Option<&T>,
    ) -> Result<()> {
        set_array_object(
            &self.vm,
            self,
            &self.element_type,
            index,
            value,
            "JavaLocalArray::set_object",
        )
    }

    pub fn get_booleans(&self) -> Result<Vec<bool>> {
        get_boolean_array(
            &self.vm,
            self,
            &self.element_type,
            "JavaLocalArray::get_booleans",
        )
    }

    pub fn set_booleans(&self, values: &[bool]) -> Result<()> {
        set_boolean_array(
            &self.vm,
            self,
            &self.element_type,
            values,
            "JavaLocalArray::set_booleans",
        )
    }

    java_primitive_array_accessors! {
        "JavaLocalArray";

        get_bytes, set_bytes, jni::jbyte, JavaType::Byte,
        get_byte_array_region, set_byte_array_region;

        get_chars, set_chars, jni::jchar, JavaType::Char,
        get_char_array_region, set_char_array_region;

        get_shorts, set_shorts, jni::jshort, JavaType::Short,
        get_short_array_region, set_short_array_region;

        get_ints, set_ints, jni::jint, JavaType::Int,
        get_int_array_region, set_int_array_region;

        get_longs, set_longs, jni::jlong, JavaType::Long,
        get_long_array_region, set_long_array_region;

        get_floats, set_floats, jni::jfloat, JavaType::Float,
        get_float_array_region, set_float_array_region;

        get_doubles, set_doubles, jni::jdouble, JavaType::Double,
        get_double_array_region, set_double_array_region;
    }
}

impl std::fmt::Debug for JavaArray {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaArray")
            .field("array", &self.as_jobject())
            .field("element_type", &self.element_type)
            .finish()
    }
}

impl AsJObject for JavaArray {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

impl std::fmt::Debug for JavaLocalArray<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaLocalArray")
            .field("array", &self.as_jobject())
            .field("element_type", &self.element_type)
            .finish()
    }
}

impl AsJObject for JavaLocalArray<'_> {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

pub(super) fn object_from_ref(
    env: &Env<'_>,
    vm: &Vm,
    object: &(impl AsJObject + ?Sized),
) -> Result<JavaObject> {
    let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
    let object = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
    Ok(JavaObject {
        vm: vm.clone(),
        object,
    })
}

pub(super) fn array_from_ref(
    env: &Env<'_>,
    vm: &Vm,
    array: &(impl AsJObject + ?Sized),
    element_type: JavaType,
) -> Result<JavaArray> {
    let reference = unsafe { env.new_global_ref_raw(array.as_jobject())? };
    let array = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
    Ok(JavaArray {
        vm: vm.clone(),
        array,
        element_type,
    })
}

fn array_len(vm: &Vm, array: &(impl AsJObject + ?Sized)) -> Result<jni::jsize> {
    let env = vm.attach_current_thread()?;
    env.array_length(array)
}

fn get_array_object(
    vm: &Vm,
    array: &(impl AsJObject + ?Sized),
    element_type: &JavaType,
    index: jni::jsize,
    operation: &'static str,
) -> Result<Option<JavaObject>> {
    ensure_reference_array(element_type, operation)?;
    let env = vm.attach_current_thread()?;
    env.get_object_array_element_nullable(array, index)?
        .map(|object| object_from_ref(&env, vm, &object))
        .transpose()
}

fn set_array_object<T: AsJObject + ?Sized>(
    vm: &Vm,
    array: &(impl AsJObject + ?Sized),
    element_type: &JavaType,
    index: jni::jsize,
    value: Option<&T>,
    operation: &'static str,
) -> Result<()> {
    ensure_reference_array(element_type, operation)?;
    let env = vm.attach_current_thread()?;
    env.set_object_array_element_raw(array, index, value)
}

fn get_boolean_array(
    vm: &Vm,
    array: &(impl AsJObject + ?Sized),
    element_type: &JavaType,
    operation: &'static str,
) -> Result<Vec<bool>> {
    ensure_element_type(element_type, &JavaType::Boolean, operation)?;
    let env = vm.attach_current_thread()?;
    let mut values = vec![jni::JNI_FALSE; env.array_length(array)? as usize];
    env.get_boolean_array_region(array, 0, &mut values)?;
    Ok(values
        .into_iter()
        .map(|value| value == jni::JNI_TRUE)
        .collect())
}

fn set_boolean_array(
    vm: &Vm,
    array: &(impl AsJObject + ?Sized),
    element_type: &JavaType,
    values: &[bool],
    operation: &'static str,
) -> Result<()> {
    ensure_element_type(element_type, &JavaType::Boolean, operation)?;
    let values = values
        .iter()
        .map(|value| {
            if *value {
                jni::JNI_TRUE
            } else {
                jni::JNI_FALSE
            }
        })
        .collect::<Vec<_>>();
    let env = vm.attach_current_thread()?;
    env.set_boolean_array_region(array, 0, &values)
}

fn ensure_reference_array(element_type: &JavaType, operation: &'static str) -> Result<()> {
    if element_type.is_reference() {
        Ok(())
    } else {
        Err(Error::InvalidObjectType {
            operation,
            expected: "object array",
            actual: format!("{element_type} array"),
        })
    }
}

fn ensure_element_type(
    actual: &JavaType,
    expected: &JavaType,
    operation: &'static str,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::InvalidObjectType {
            operation,
            expected: "matching array element type",
            actual: actual.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_array_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let array =
            unsafe { JavaLocalArray::from_raw(Vm::dangling_for_tests(), raw, JavaType::Int) }
                .unwrap();
        assert_eq!(array.as_jobject(), raw);
        assert_eq!(array.element_type(), &JavaType::Int);
        assert_eq!(JavaValue::from(&array), JavaValue::Object(raw));
    }

    #[test]
    fn local_array_view_rejects_null_raw() {
        assert_eq!(
            unsafe {
                JavaLocalArray::from_raw(Vm::dangling_for_tests(), ptr::null_mut(), JavaType::Int)
            }
            .unwrap_err(),
            Error::NullReturn {
                operation: "JNI local array view",
            }
        );
    }

    #[test]
    fn local_array_type_checks_report_expected_errors() {
        assert_eq!(
            ensure_reference_array(&JavaType::Int, "test").unwrap_err(),
            Error::InvalidObjectType {
                operation: "test",
                expected: "object array",
                actual: "I array".to_owned(),
            }
        );
        assert_eq!(
            ensure_element_type(&JavaType::Byte, &JavaType::Int, "test").unwrap_err(),
            Error::InvalidObjectType {
                operation: "test",
                expected: "matching array element type",
                actual: "B".to_owned(),
            }
        );
    }
}
