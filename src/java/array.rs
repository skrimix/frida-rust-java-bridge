use super::*;

trait JavaArrayStorage: JavaObjectRef {
    const OPERATION_NAME: &'static str;
}

impl JavaArrayStorage for GlobalRef<ArrayKind> {
    const OPERATION_NAME: &'static str = "JavaArray";
}

impl JavaArray {
    pub fn as_object(&self) -> Result<JavaObject> {
        let env = self.vm().attach_current_thread()?;
        object_from_ref(&env, self.vm(), self)
    }

    pub fn into_object(self) -> Result<JavaObject> {
        let JavaArray { object, .. } = self;
        let JavaObject { vm, reference, .. } = object;
        let raw = unsafe { reference.into_raw() };
        unsafe { JavaObject::from_global_raw_runtime(vm, raw) }
    }
}

impl<'local> JavaArray<BorrowedLocalRef<'local, ArrayKind>> {
    pub(crate) unsafe fn from_raw(
        vm: Vm,
        raw: jni::jobject,
        element_type: JavaType,
    ) -> Result<Self> {
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local array view")? };
        let class = runtime_class(&vm, &reference)?;
        Ok(Self {
            object: JavaObject {
                class,
                vm,
                reference,
            },
            element_type,
        })
    }

    pub fn as_object(&self) -> Result<JavaLocalObject<'local>> {
        unsafe { JavaLocalObject::from_raw(self.vm().clone(), self.raw_jobject()) }
    }
}

impl<'local> JavaArrayStorage for BorrowedLocalRef<'local, ArrayKind> {
    const OPERATION_NAME: &'static str = "JavaLocalArray";
}

impl<R> JavaArray<R>
where
    R: JavaObjectRef,
{
    pub(crate) fn vm_ref(&self) -> &Vm {
        self.object.vm()
    }
}

impl<R> JavaArray<R>
where
    R: JavaArrayStorage,
{
    pub fn vm(&self) -> &Vm {
        self.object.vm()
    }

    /// Returns the raw JNI array reference.
    ///
    /// # Safety
    ///
    /// The caller must honor this wrapper's reference storage rules: global references must not be
    /// deleted by the caller, and borrowed local references are valid only in their producing
    /// callback/JNI frame on the current thread.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        unsafe { self.object.raw_jobject() }
    }

    pub fn element_type(&self) -> &JavaType {
        &self.element_type
    }

    pub fn len(&self) -> Result<jni::jsize> {
        array_len(self.vm(), self)
    }

    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    pub fn retain(&self) -> Result<JavaArray> {
        let env = self.vm().attach_current_thread()?;
        array_from_ref(&env, self.vm(), self, self.element_type.clone())
    }

    pub fn java_display(&self) -> Result<String> {
        self.object.java_display()
    }

    pub fn get_object(&self, index: jni::jsize) -> Result<Option<JavaObject>> {
        get_array_object(
            self.vm(),
            self,
            &self.element_type,
            index,
            operation_name::<R>("get_object"),
        )
    }

    pub fn set_object<T: JavaObjectRef + ?Sized>(
        &self,
        index: jni::jsize,
        value: Option<&T>,
    ) -> Result<()> {
        set_array_object(
            self.vm(),
            self,
            &self.element_type,
            index,
            value,
            operation_name::<R>("set_object"),
        )
    }

    pub fn get_booleans(&self) -> Result<Vec<bool>> {
        get_boolean_array(
            self.vm(),
            self,
            &self.element_type,
            operation_name::<R>("get_booleans"),
        )
    }

    pub fn set_booleans(&self, values: &[bool]) -> Result<()> {
        set_boolean_array(
            self.vm(),
            self,
            &self.element_type,
            values,
            operation_name::<R>("set_booleans"),
        )
    }

    java_primitive_array_accessors! {
        R;

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

impl<R> std::fmt::Debug for JavaArray<R>
where
    R: JavaObjectRef,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaArray")
            .field("array", &unsafe { self.object.raw_jobject() })
            .field("element_type", &self.element_type)
            .finish()
    }
}

impl<R> crate::refs::sealed::JavaObjectRefSealed for JavaArray<R>
where
    R: JavaObjectRef,
{
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.object.raw_jobject() }
    }
}

impl<R> crate::refs::JavaObjectRef for JavaArray<R> where R: JavaObjectRef {}

pub(super) fn object_from_ref(
    env: &Env<'_>,
    vm: &Vm,
    object: &(impl JavaObjectRef + ?Sized),
) -> Result<JavaObject> {
    let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
    unsafe { JavaObject::from_global_raw_runtime(vm.clone(), reference) }
}

pub(super) fn array_from_ref(
    env: &Env<'_>,
    vm: &Vm,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: JavaType,
) -> Result<JavaArray> {
    let reference = unsafe { env.new_global_ref_raw(array.as_jobject())? };
    let reference = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
    let class = runtime_class(vm, &reference)?;
    Ok(JavaArray {
        object: JavaObject {
            class,
            vm: vm.clone(),
            reference,
        },
        element_type,
    })
}

fn array_len(vm: &Vm, array: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jsize> {
    let env = vm.attach_current_thread()?;
    env.array_length(array)
}

fn get_array_object(
    vm: &Vm,
    array: &(impl JavaObjectRef + ?Sized),
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

fn set_array_object<T: JavaObjectRef + ?Sized>(
    vm: &Vm,
    array: &(impl JavaObjectRef + ?Sized),
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
    array: &(impl JavaObjectRef + ?Sized),
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
    array: &(impl JavaObjectRef + ?Sized),
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

fn operation_name<R: JavaArrayStorage>(method: &'static str) -> &'static str {
    match (R::OPERATION_NAME, method) {
        ("JavaArray", "get_object") => "JavaArray::get_object",
        ("JavaArray", "set_object") => "JavaArray::set_object",
        ("JavaArray", "get_booleans") => "JavaArray::get_booleans",
        ("JavaArray", "set_booleans") => "JavaArray::set_booleans",
        ("JavaArray", "get_bytes") => "JavaArray::get_bytes",
        ("JavaArray", "set_bytes") => "JavaArray::set_bytes",
        ("JavaArray", "get_chars") => "JavaArray::get_chars",
        ("JavaArray", "set_chars") => "JavaArray::set_chars",
        ("JavaArray", "get_shorts") => "JavaArray::get_shorts",
        ("JavaArray", "set_shorts") => "JavaArray::set_shorts",
        ("JavaArray", "get_ints") => "JavaArray::get_ints",
        ("JavaArray", "set_ints") => "JavaArray::set_ints",
        ("JavaArray", "get_longs") => "JavaArray::get_longs",
        ("JavaArray", "set_longs") => "JavaArray::set_longs",
        ("JavaArray", "get_floats") => "JavaArray::get_floats",
        ("JavaArray", "set_floats") => "JavaArray::set_floats",
        ("JavaArray", "get_doubles") => "JavaArray::get_doubles",
        ("JavaArray", "set_doubles") => "JavaArray::set_doubles",
        ("JavaLocalArray", "get_object") => "JavaLocalArray::get_object",
        ("JavaLocalArray", "set_object") => "JavaLocalArray::set_object",
        ("JavaLocalArray", "get_booleans") => "JavaLocalArray::get_booleans",
        ("JavaLocalArray", "set_booleans") => "JavaLocalArray::set_booleans",
        ("JavaLocalArray", "get_bytes") => "JavaLocalArray::get_bytes",
        ("JavaLocalArray", "set_bytes") => "JavaLocalArray::set_bytes",
        ("JavaLocalArray", "get_chars") => "JavaLocalArray::get_chars",
        ("JavaLocalArray", "set_chars") => "JavaLocalArray::set_chars",
        ("JavaLocalArray", "get_shorts") => "JavaLocalArray::get_shorts",
        ("JavaLocalArray", "set_shorts") => "JavaLocalArray::set_shorts",
        ("JavaLocalArray", "get_ints") => "JavaLocalArray::get_ints",
        ("JavaLocalArray", "set_ints") => "JavaLocalArray::set_ints",
        ("JavaLocalArray", "get_longs") => "JavaLocalArray::get_longs",
        ("JavaLocalArray", "set_longs") => "JavaLocalArray::set_longs",
        ("JavaLocalArray", "get_floats") => "JavaLocalArray::get_floats",
        ("JavaLocalArray", "set_floats") => "JavaLocalArray::set_floats",
        ("JavaLocalArray", "get_doubles") => "JavaLocalArray::get_doubles",
        ("JavaLocalArray", "set_doubles") => "JavaLocalArray::set_doubles",
        _ => R::OPERATION_NAME,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_array_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let vm = Vm::dangling_for_tests();
        let class = JavaClass::from_raw(raw::Class::from_global(
            vm.clone(),
            "[I".to_owned(),
            unsafe { GlobalRef::from_raw(vm.clone(), std::ptr::dangling_mut()).unwrap() },
        ));
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "test array").unwrap() };
        let array = JavaLocalArray {
            object: JavaObject {
                class,
                vm,
                reference,
            },
            element_type: JavaType::Int,
        };
        assert_eq!(unsafe { array.raw_jobject() }, raw);
        assert_eq!(array.element_type(), &JavaType::Int);
    }

    #[test]
    fn global_array_wrapper_keeps_default_java_value_conversion() {
        let raw = std::ptr::dangling_mut();
        let vm = Vm::dangling_for_tests();
        let reference = unsafe { GlobalRef::from_raw(vm.clone(), raw) }.unwrap();
        let class = JavaClass::from_raw(raw::Class::from_global(
            vm.clone(),
            "[I".to_owned(),
            unsafe { GlobalRef::from_raw(vm.clone(), std::ptr::dangling_mut()).unwrap() },
        ));
        let array = JavaArray {
            object: JavaObject {
                class,
                vm,
                reference,
            },
            element_type: JavaType::Int,
        };

        assert_eq!(unsafe { array.raw_jobject() }, raw);
        assert_eq!(array.element_type(), &JavaType::Int);
        assert_eq!(JavaValue::from(&array), JavaValue::object_ref(raw));
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
