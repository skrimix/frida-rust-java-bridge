use super::*;

impl JavaObject {
    pub(crate) unsafe fn from_global_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        let object = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        Ok(Self { vm, object })
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.object.as_jobject()
    }

    pub fn retain(&self) -> Result<Self> {
        let env = self.vm.attach_current_thread()?;
        let reference = unsafe { env.new_global_ref_raw(self.as_jobject())? };
        let object = unsafe { GlobalRef::from_raw(self.vm.clone(), reference)? };
        Ok(Self {
            vm: self.vm.clone(),
            object,
        })
    }

    pub fn get_string(&self) -> Result<String> {
        let env = self.vm.attach_current_thread()?;
        unsafe { env.get_string_raw(self.as_jobject()) }
    }

    pub fn java_to_string(&self) -> Result<String> {
        object_to_string(&self.vm, self)
    }
}

impl std::fmt::Debug for JavaObject {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_tuple("JavaObject")
            .field(&self.as_jobject())
            .finish()
    }
}

impl AsJObject for JavaObject {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

impl<'local> JavaLocalObject<'local> {
    pub(crate) unsafe fn from_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JNI local object view",
            });
        }

        Ok(Self {
            vm,
            object: raw,
            _local: PhantomData,
            _thread_affine: PhantomData,
        })
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.object
    }

    pub fn retain(&self) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        object_from_ref(&env, &self.vm, self)
    }

    pub fn get_string(&self) -> Result<String> {
        let env = self.vm.attach_current_thread()?;
        unsafe { env.get_string_raw(self.as_jobject()) }
    }

    pub fn java_to_string(&self) -> Result<String> {
        object_to_string(&self.vm, self)
    }
}

impl std::fmt::Debug for JavaLocalObject<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_tuple("JavaLocalObject")
            .field(&self.as_jobject())
            .finish()
    }
}

impl AsJObject for JavaLocalObject<'_> {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

fn object_to_string(vm: &Vm, object: &(impl AsJObject + ?Sized)) -> Result<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_object_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let object = unsafe { JavaLocalObject::from_raw(Vm::dangling_for_tests(), raw) }.unwrap();
        assert_eq!(object.as_jobject(), raw);
        assert_eq!(JavaValue::from(&object), JavaValue::Object(raw));
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
