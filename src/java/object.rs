use super::*;

impl JavaRef {
    pub(crate) unsafe fn from_global_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        let reference = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        Ok(Self { vm, reference })
    }
}

impl<R> JavaRef<R>
where
    R: JavaObjectRef,
{
    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    /// Returns the raw JNI object reference.
    ///
    /// # Safety
    ///
    /// The caller must honor this wrapper's reference storage rules: global references must not be
    /// deleted by the caller, and borrowed local references are valid only in their producing
    /// callback/JNI frame on the current thread.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        self.reference.as_jobject()
    }

    pub fn retain(&self) -> Result<JavaRef> {
        let env = self.vm.attach_current_thread()?;
        object_ref_from_ref(&env, &self.vm, self)
    }

    pub fn runtime_class(&self) -> Result<JavaClass> {
        runtime_class(&self.vm, self)
    }

    pub fn bind_runtime(&self) -> Result<JavaObject> {
        self.retain()?.into_object_runtime()
    }

    pub fn into_object_runtime(self) -> Result<JavaObject<R>> {
        let class = self.runtime_class()?;
        Ok(JavaObject {
            class,
            reference: self,
        })
    }

    pub fn cast(&self, class: &JavaClass) -> Result<JavaObject> {
        class.cast(self)
    }

    pub fn get_string(&self) -> Result<String> {
        let env = self.vm.attach_current_thread()?;
        unsafe { env.get_string_raw(self.raw_jobject()) }
    }

    pub fn java_to_string(&self) -> Result<String> {
        object_to_string(&self.vm, self)
    }

    pub fn java_display(&self) -> Result<String> {
        self.java_to_string()
    }
}

impl<'local> JavaRef<BorrowedLocalRef<'local, ObjectKind>> {
    pub(crate) unsafe fn from_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local object reference")? };
        Ok(Self { vm, reference })
    }
}

impl JavaObject {
    pub(crate) fn from_ref(class: JavaClass, reference: JavaRef) -> Self {
        Self { class, reference }
    }
}

impl<R> JavaObject<R>
where
    R: JavaObjectRef,
{
    pub fn vm(&self) -> &Vm {
        self.reference.vm()
    }

    pub fn class(&self) -> &JavaClass {
        &self.class
    }

    pub fn reference(&self) -> &JavaRef<R> {
        &self.reference
    }

    pub fn into_ref(self) -> JavaRef<R> {
        self.reference
    }

    /// Returns the raw JNI object reference.
    ///
    /// # Safety
    ///
    /// The caller must honor this wrapper's reference storage rules: global references must not be
    /// deleted by the caller, and borrowed local references are valid only in their producing
    /// callback/JNI frame on the current thread.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        unsafe { self.reference.raw_jobject() }
    }

    pub fn retain(&self) -> Result<JavaObject> {
        Ok(JavaObject {
            class: self.class.clone(),
            reference: self.reference.retain()?,
        })
    }

    pub fn runtime_class(&self) -> Result<JavaClass> {
        self.reference.runtime_class()
    }

    pub fn cast(&self, class: &JavaClass) -> Result<JavaObject> {
        class.cast(self)
    }

    pub fn method<'object, S: JavaBoundMethodSelector<'object>>(
        &'object self,
        selector: S,
    ) -> Result<S::Output> {
        self.class.bind(self)?.method(selector)
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.class.bind(self)?.call(name, args)
    }

    pub fn call_ref(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<JavaRef> {
        self.call(name, args)
    }

    pub fn call_overload<'a, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'a str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.class.bind(self)?.call_overload(name, arguments, args)
    }

    pub fn field<'object>(&'object self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        self.class.bind(self)?.field(name)
    }

    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.class.bind(self)?.get_field(name)
    }

    pub fn get_ref_field(&self, name: &str) -> Result<JavaRef> {
        self.get_field(name)
    }

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.class.bind(self)?.set_field(name, value)
    }

    pub fn get_string(&self) -> Result<String> {
        self.reference.get_string()
    }

    pub fn java_to_string(&self) -> Result<String> {
        self.reference.java_to_string()
    }

    pub fn java_display(&self) -> Result<String> {
        self.java_to_string()
    }

    pub fn as_hook_return(&self) -> replacement::JavaHookReturn {
        replacement::JavaHookReturn::from(self)
    }
}

impl<'local> JavaObject<BorrowedLocalRef<'local, ObjectKind>> {
    pub(crate) unsafe fn from_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        unsafe { JavaLocalRef::from_raw(vm, raw) }?.into_object_runtime()
    }
}

impl<R> std::fmt::Debug for JavaRef<R>
where
    R: JavaObjectRef,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_tuple("JavaRef")
            .field(&unsafe { self.raw_jobject() })
            .finish()
    }
}

impl<R> std::fmt::Debug for JavaObject<R>
where
    R: JavaObjectRef,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaObject")
            .field("class", &self.class.name())
            .field("object", &unsafe { self.raw_jobject() })
            .finish()
    }
}

impl<R> crate::refs::sealed::JavaObjectRefSealed for JavaRef<R>
where
    R: JavaObjectRef,
{
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.raw_jobject() }
    }
}

impl<R> crate::refs::JavaObjectRef for JavaRef<R> where R: JavaObjectRef {}

impl<R> crate::refs::sealed::JavaObjectRefSealed for JavaObject<R>
where
    R: JavaObjectRef,
{
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.raw_jobject() }
    }
}

impl<R> crate::refs::JavaObjectRef for JavaObject<R> where R: JavaObjectRef {}

pub(super) fn object_to_string(vm: &Vm, object: &(impl JavaObjectRef + ?Sized)) -> Result<String> {
    let env = vm.attach_current_thread()?;
    let object_class = env.find_class("java/lang/Object")?;
    let to_string =
        env.lookup_instance_method(&object_class, "toString", "()Ljava/lang/String;")?;
    let string = env
        .call_instance_object_method(object, &to_string, &[])?
        .ok_or(Error::NullReturn {
            operation: "Object.toString",
        })?;
    unsafe { env.get_string_raw(string.as_jobject()) }
}

pub(super) fn runtime_class(vm: &Vm, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaClass> {
    let env = vm.attach_current_thread()?;
    let class = env.get_object_class(object)?;
    let descriptor = metadata::class_descriptor(&env, &class)?;
    let name = metadata::class_name_from_descriptor(&descriptor);
    let class = env.new_global_ref(&class)?;
    Ok(JavaClass::from_raw(RawJavaClass::from_global(
        vm.clone(),
        name,
        class,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_ref_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let object = unsafe { JavaLocalRef::from_raw(Vm::dangling_for_tests(), raw) }.unwrap();
        assert_eq!(unsafe { object.raw_jobject() }, raw);
        assert_eq!(JavaValue::from(&object), JavaValue::object_ref(raw));
    }

    #[test]
    fn global_ref_wrapper_keeps_default_java_value_conversion() {
        let raw = std::ptr::dangling_mut();
        let object = unsafe { JavaRef::from_global_raw(Vm::dangling_for_tests(), raw) }.unwrap();

        assert_eq!(unsafe { object.raw_jobject() }, raw);
        assert_eq!(JavaValue::from(&object), JavaValue::object_ref(raw));
    }

    #[test]
    fn local_ref_view_rejects_null_raw() {
        assert_eq!(
            unsafe { JavaLocalRef::from_raw(Vm::dangling_for_tests(), ptr::null_mut()) }
                .unwrap_err(),
            Error::NullReturn {
                operation: "JNI local object reference",
            }
        );
    }
}
