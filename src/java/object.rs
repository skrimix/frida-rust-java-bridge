use super::*;

impl JavaObject {
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
