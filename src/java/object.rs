use super::*;

impl JavaObject {
    #[cfg(test)]
    pub(crate) unsafe fn from_global_raw(class: JavaClass, raw: jni::jobject) -> Result<Self> {
        let vm = class.class.vm().clone();
        let reference = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        Ok(Self {
            class,
            vm,
            reference,
        })
    }

    pub(crate) fn from_global_ref(class: JavaClass, reference: GlobalRef<ObjectKind>) -> Self {
        let vm = class.class.vm().clone();
        Self {
            class,
            vm,
            reference,
        }
    }

    pub(crate) unsafe fn from_global_raw_runtime(vm: Vm, raw: jni::jobject) -> Result<Self> {
        let reference = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        let class = runtime_class(&vm, &reference)?;
        Ok(Self {
            class,
            vm,
            reference,
        })
    }
}

impl<R> JavaObject<R>
where
    R: JavaObjectRef,
{
    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn class(&self) -> &JavaClass {
        &self.class
    }

    pub(crate) fn rebind(self, class: JavaClass) -> Self {
        Self {
            class,
            vm: self.vm,
            reference: self.reference,
        }
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

    pub fn retain(&self) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        let reference = unsafe { env.new_global_ref_raw(self.reference.as_jobject())? };
        let reference = unsafe { GlobalRef::from_raw(self.vm.clone(), reference)? };
        Ok(JavaObject {
            class: self.class.clone(),
            vm: self.vm.clone(),
            reference,
        })
    }

    pub fn runtime_class(&self) -> Result<JavaClass> {
        runtime_class(&self.vm, self)
    }

    pub fn cast(&self, class: &JavaClass) -> Result<JavaObject> {
        class.cast(self)
    }

    pub fn method<'object>(&'object self, name: &str) -> Result<JavaBoundMethodGroup<'object>> {
        self.class.bind(self)?.method(name)
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.class.bind(self)?.call(name, args)
    }

    pub fn call_with<'a, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'a str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.class.bind(self)?.call_with(name, arguments, args)
    }

    pub fn field<'object>(&'object self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        self.class.bind(self)?.field(name)
    }

    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.class.bind(self)?.get_field(name)
    }

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.class.bind(self)?.set_field(name, value)
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
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local object reference")? };
        let class = runtime_class(&vm, &reference)?;
        Ok(Self {
            class,
            vm,
            reference,
        })
    }

    pub(crate) unsafe fn from_raw_with_class(class: JavaClass, raw: jni::jobject) -> Result<Self> {
        let vm = class.class.vm().clone();
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local object reference")? };
        Ok(Self {
            class,
            vm,
            reference,
        })
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
    // SAFETY: `to_string` was resolved from `object`'s runtime class immediately above.
    let string = unsafe { env.call_instance_object_method(object, &to_string, &[])? }.ok_or(
        Error::NullReturn {
            operation: "Object.toString",
        },
    )?;
    unsafe { env.get_string_raw(string.as_jobject()) }
}

pub(super) fn runtime_class(vm: &Vm, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaClass> {
    let env = vm.attach_current_thread()?;
    let class = env.get_object_class(object)?;
    let descriptor = metadata::class_descriptor(&env, &class)?;
    let name = metadata::class_name_from_descriptor(&descriptor);
    let class = env.new_global_ref(&class)?;
    Ok(JavaClass::from_raw(raw::Class::from_global(
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
        let class = JavaClass::from_raw(raw::Class::from_global(
            Vm::dangling_for_tests(),
            "java.lang.Object".to_owned(),
            unsafe {
                GlobalRef::from_raw(Vm::dangling_for_tests(), std::ptr::dangling_mut()).unwrap()
            },
        ));
        let object = unsafe { JavaLocalObject::from_raw_with_class(class, raw) }.unwrap();
        assert_eq!(unsafe { object.raw_jobject() }, raw);
    }

    #[test]
    fn local_object_view_rejects_null_raw() {
        let class = JavaClass::from_raw(raw::Class::from_global(
            Vm::dangling_for_tests(),
            "java.lang.Object".to_owned(),
            unsafe {
                GlobalRef::from_raw(Vm::dangling_for_tests(), std::ptr::dangling_mut()).unwrap()
            },
        ));
        assert_eq!(
            unsafe { JavaLocalObject::from_raw_with_class(class, ptr::null_mut()) }.unwrap_err(),
            Error::NullReturn {
                operation: "JNI local object reference",
            }
        );
    }
}
