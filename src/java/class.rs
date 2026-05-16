use super::*;

impl JavaClass {
    pub(crate) fn from_global(vm: Vm, name: String, class: GlobalRef<ClassKind>) -> Self {
        Self {
            inner: Arc::new(JavaClassInner {
                vm,
                name,
                class,
                methods: Mutex::new(HashMap::new()),
                fields: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn as_jclass(&self) -> jni::jclass {
        self.inner.class.as_jclass()
    }

    pub(crate) fn vm(&self) -> &Vm {
        &self.inner.vm
    }

    pub(crate) fn resolve_static_method(&self, name: &str, signature: &str) -> Result<MethodId> {
        let env = self.inner.vm.attach_current_thread()?;
        self.static_method(&env, name, signature)
    }

    pub(crate) fn resolve_instance_method(&self, name: &str, signature: &str) -> Result<MethodId> {
        let env = self.inner.vm.attach_current_thread()?;
        self.method(&env, name, signature)
    }

    pub fn new_object(&self, signature: &str, args: &[JavaValue]) -> Result<JavaObject> {
        let env = self.inner.vm.attach_current_thread()?;
        let constructor = self.constructor(&env, signature)?;
        let object = env.new_object(&self.inner.class, &constructor, args)?;
        object_from_ref(&env, &self.inner.vm, &object)
    }

    pub fn call_method(
        &self,
        object: &JavaObject,
        name: &str,
        signature: &str,
        args: &[JavaValue],
    ) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let method = self.method(&env, name, signature)?;
        call_instance_return(&env, object, &method, args)
    }

    pub fn call_static(
        &self,
        name: &str,
        signature: &str,
        args: &[JavaValue],
    ) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let method = self.static_method(&env, name, signature)?;
        call_static_return(&env, &self.inner.class, &method, args)
    }

    pub fn get_field(&self, object: &JavaObject, name: &str, ty: &str) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let field = self.field(&env, name, ty)?;
        get_instance_field(&env, object, &field)
    }

    pub fn set_field(
        &self,
        object: &JavaObject,
        name: &str,
        ty: &str,
        value: JavaValue,
    ) -> Result<()> {
        let env = self.inner.vm.attach_current_thread()?;
        let field = self.field(&env, name, ty)?;
        set_instance_field(&env, object, &field, value)
    }

    pub fn get_static_field(&self, name: &str, ty: &str) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let field = self.static_field(&env, name, ty)?;
        get_static_field(&env, &self.inner.class, &field)
    }

    pub fn set_static_field(&self, name: &str, ty: &str, value: JavaValue) -> Result<()> {
        let env = self.inner.vm.attach_current_thread()?;
        let field = self.static_field(&env, name, ty)?;
        set_static_field(&env, &self.inner.class, &field, value)
    }

    pub fn metadata(&self) -> Result<JavaClassMetadata> {
        metadata::class_metadata(&self.inner.vm.java(), self)
    }

    pub fn declared_methods(&self) -> Result<Vec<JavaMethodMetadata>> {
        metadata::declared_methods(&self.inner.vm.java(), self)
    }

    pub fn declared_fields(&self) -> Result<Vec<JavaFieldMetadata>> {
        metadata::declared_fields(&self.inner.vm.java(), self)
    }

    pub fn is_instance(&self, object: &JavaObject) -> Result<bool> {
        let env = self.inner.vm.attach_current_thread()?;
        env.is_instance_of(object, &self.inner.class)
    }

    fn constructor(&self, env: &Env<'_>, signature: &str) -> Result<MethodId> {
        self.cached_method(env, MethodKind::Constructor, "<init>", signature)
    }

    fn method(&self, env: &Env<'_>, name: &str, signature: &str) -> Result<MethodId> {
        self.cached_method(env, MethodKind::Instance, name, signature)
    }

    fn static_method(&self, env: &Env<'_>, name: &str, signature: &str) -> Result<MethodId> {
        self.cached_method(env, MethodKind::Static, name, signature)
    }

    fn field(&self, env: &Env<'_>, name: &str, ty: &str) -> Result<FieldId> {
        self.cached_field(env, FieldKind::Instance, name, ty)
    }

    fn static_field(&self, env: &Env<'_>, name: &str, ty: &str) -> Result<FieldId> {
        self.cached_field(env, FieldKind::Static, name, ty)
    }

    fn cached_method(
        &self,
        env: &Env<'_>,
        kind: MethodKind,
        name: &str,
        signature: &str,
    ) -> Result<MethodId> {
        let signature = MethodSignature::parse(signature)?.to_string();
        let key = MethodKey {
            kind,
            name: name.to_owned(),
            signature,
        };

        if let Some(method) = self
            .inner
            .methods
            .lock()
            .expect("JavaClass method cache mutex poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(method);
        }

        let method = match kind {
            MethodKind::Constructor => env.lookup_constructor(&self.inner.class, &key.signature)?,
            MethodKind::Instance => {
                env.lookup_instance_method(&self.inner.class, name, &key.signature)?
            }
            MethodKind::Static => {
                env.lookup_static_method(&self.inner.class, name, &key.signature)?
            }
        };

        self.inner
            .methods
            .lock()
            .expect("JavaClass method cache mutex poisoned")
            .insert(key, method.clone());

        Ok(method)
    }

    fn cached_field(
        &self,
        env: &Env<'_>,
        kind: FieldKind,
        name: &str,
        ty: &str,
    ) -> Result<FieldId> {
        let ty = JavaType::parse(ty)?.to_string();
        let key = FieldKey {
            kind,
            name: name.to_owned(),
            ty,
        };

        if let Some(field) = self
            .inner
            .fields
            .lock()
            .expect("JavaClass field cache mutex poisoned")
            .get(&key)
            .cloned()
        {
            return Ok(field);
        }

        let field = match kind {
            FieldKind::Instance => env.lookup_instance_field(&self.inner.class, name, &key.ty)?,
            FieldKind::Static => env.lookup_static_field(&self.inner.class, name, &key.ty)?,
        };

        self.inner
            .fields
            .lock()
            .expect("JavaClass field cache mutex poisoned")
            .insert(key, field.clone());

        Ok(field)
    }
}

impl AsJObject for JavaClass {
    fn as_jobject(&self) -> jni::jobject {
        self.inner.class.as_jobject()
    }
}

impl AsJClass for JavaClass {
    fn as_jclass(&self) -> jni::jclass {
        self.as_jclass()
    }
}
