use super::*;

impl JavaObject {
    pub(crate) unsafe fn from_global_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        let object = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        Ok(Self { vm, object })
    }
}

impl<R> JavaObject<R>
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
        self.object.as_jobject()
    }

    pub fn retain(&self) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        object_from_ref(&env, &self.vm, self)
    }

    pub fn runtime_class(&self) -> Result<JavaClass> {
        runtime_class(&self.vm, self)
    }

    pub fn method<'object, S: JavaBoundMethodSelector<'object>>(
        &'object self,
        selector: S,
    ) -> Result<S::Output> {
        self.runtime_class()?.bind(self)?.method(selector)
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.runtime_class()?.bind(self)?.call(name, args)
    }

    pub fn call_overload<'a, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'a str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.runtime_class()?
            .bind(self)?
            .call_overload(name, arguments, args)
    }

    pub fn field<'object>(&'object self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        self.runtime_class()?.bind(self)?.field(name)
    }

    pub fn get<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.runtime_class()?.bind(self)?.get(name)
    }

    pub fn set<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.runtime_class()?.bind(self)?.set(name, value)
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

impl<'local> JavaObject<BorrowedLocalRef<'local, ObjectKind>> {
    pub(crate) unsafe fn from_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        let object = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local object view")? };
        Ok(Self { vm, object })
    }
}

impl<R> std::fmt::Debug for JavaObject<R>
where
    R: JavaObjectRef,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_tuple("JavaObject")
            .field(&unsafe { self.raw_jobject() })
            .finish()
    }
}

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

fn runtime_class(vm: &Vm, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaClass> {
    let env = vm.attach_current_thread()?;
    let class = env.get_object_class(object)?;
    let descriptor = metadata::class_descriptor(&env, &class)?;
    let name = metadata::class_name_from_descriptor(&descriptor);
    let class = env.new_global_ref(&class)?;
    Ok(JavaClass::new(RawJavaClass::from_global(
        vm.clone(),
        name,
        class,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_object_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let object = unsafe { JavaLocalObject::from_raw(Vm::dangling_for_tests(), raw) }.unwrap();
        assert_eq!(unsafe { object.raw_jobject() }, raw);
        assert_eq!(JavaValue::from(&object), JavaValue::object_ref(raw));
    }

    #[test]
    fn global_object_wrapper_keeps_default_java_value_conversion() {
        let raw = std::ptr::dangling_mut();
        let object = unsafe { JavaObject::from_global_raw(Vm::dangling_for_tests(), raw) }.unwrap();

        assert_eq!(unsafe { object.raw_jobject() }, raw);
        assert_eq!(JavaValue::from(&object), JavaValue::object_ref(raw));
    }

    #[test]
    fn local_object_view_rejects_null_raw() {
        assert_eq!(
            unsafe { JavaLocalObject::from_raw(Vm::dangling_for_tests(), ptr::null_mut()) }
                .unwrap_err(),
            Error::NullReturn {
                operation: "JNI local object view",
            }
        );
    }
}
