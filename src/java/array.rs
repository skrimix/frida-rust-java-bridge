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
        let env = self.vm.attach_current_thread()?;
        env.array_length(self)
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
        self.ensure_reference_array("JavaArray::get_object")?;
        let env = self.vm.attach_current_thread()?;
        env.get_object_array_element_nullable(self, index)?
            .map(|object| object_from_ref(&env, &self.vm, &object))
            .transpose()
    }

    pub fn set_object(&self, index: jni::jsize, value: Option<&JavaObject>) -> Result<()> {
        self.ensure_reference_array("JavaArray::set_object")?;
        let env = self.vm.attach_current_thread()?;
        env.set_object_array_element_raw(self, index, value)
    }

    pub fn get_booleans(&self) -> Result<Vec<bool>> {
        self.ensure_element_type(JavaType::Boolean, "JavaArray::get_booleans")?;
        let env = self.vm.attach_current_thread()?;
        let mut values = vec![jni::JNI_FALSE; self.len()? as usize];
        env.get_boolean_array_region(self, 0, &mut values)?;
        Ok(values
            .into_iter()
            .map(|value| value == jni::JNI_TRUE)
            .collect())
    }

    pub fn set_booleans(&self, values: &[bool]) -> Result<()> {
        self.ensure_element_type(JavaType::Boolean, "JavaArray::set_booleans")?;
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
        let env = self.vm.attach_current_thread()?;
        env.set_boolean_array_region(self, 0, &values)
    }

    java_primitive_array_accessors! {
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

    fn ensure_reference_array(&self, operation: &'static str) -> Result<()> {
        if self.element_type.is_reference() {
            Ok(())
        } else {
            Err(Error::InvalidObjectType {
                operation,
                expected: "object array",
                actual: format!("{} array", self.element_type),
            })
        }
    }

    fn ensure_element_type(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        if self.element_type == expected {
            Ok(())
        } else {
            Err(Error::InvalidObjectType {
                operation,
                expected: "matching array element type",
                actual: self.element_type.to_string(),
            })
        }
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
