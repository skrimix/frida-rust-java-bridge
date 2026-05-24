use super::*;

impl JavaClass {
    pub(super) fn from_raw(class: RawJavaClass) -> Self {
        Self {
            class,
            methods: Arc::new(Mutex::new(None)),
            visible_methods: Arc::new(Mutex::new(None)),
            fields: Arc::new(Mutex::new(None)),
            visible_fields: Arc::new(Mutex::new(None)),
        }
    }

    pub fn name(&self) -> &str {
        self.class.name()
    }

    pub fn class(&self) -> &raw::Class {
        &self.class
    }

    pub fn declared_methods(&self) -> Result<Vec<JavaMethodMetadata>> {
        self.declared_methods_cached()
    }

    pub fn declared_fields(&self) -> Result<Vec<JavaFieldMetadata>> {
        self.declared_fields_cached()
    }

    pub fn constructors(&self) -> Result<Vec<JavaMethodMetadata>> {
        Ok(self
            .declared_methods_cached()?
            .into_iter()
            .filter(|method| method.kind == MethodKind::Constructor)
            .collect())
    }

    pub fn method(&self, name: &str) -> Result<JavaMethodGroup> {
        let overloads = self.visible_methods_by_name(name)?;
        if overloads.is_empty() {
            return Err(Error::MethodNameNotFound {
                class: self.name().to_owned(),
                kind: "method",
                name: name.to_owned(),
            });
        }
        Ok(JavaMethodGroup {
            class: self.class.clone(),
            name: name.to_owned(),
            overloads,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.method(name)?.call(args)
    }

    pub fn call_ref(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<JavaRef> {
        self.call(name, args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.method(name)?.overload(arguments)?.call((), args)
    }

    pub fn constructor_by_types(&self, arguments: &[JavaType]) -> Result<JavaConstructor> {
        let metadata =
            self.resolve_method_overload(MethodKind::Constructor, "<init>", arguments)?;
        Ok(JavaConstructor {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn constructor<'types>(
        &self,
        arguments: impl AsRef<[&'types str]>,
    ) -> Result<JavaConstructor> {
        let arguments = parse_type_names(arguments.as_ref())?;
        self.constructor_by_types(&arguments)
    }

    /// Creates an object through the constructor overload with the given argument types.
    pub fn new_with<'types, A: IntoJavaCallArgs>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: A,
    ) -> Result<JavaObject> {
        self.constructor(arguments)?.new_object(args)
    }

    /// Creates an object by dispatching to the best compatible constructor overload.
    #[allow(clippy::new_ret_no_self)]
    pub fn new<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        let args = args.into_java_dispatch_args();
        let constructor = self.resolve_constructor_for_dispatch(&args)?;
        constructor.new_object(args)
    }

    pub fn field(&self, name: &str) -> Result<JavaField> {
        let metadata = select_field_by_name(self.name(), name, self.field_matches_by_name(name)?)?;
        Ok(JavaField {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.field(name)?.get(())
    }

    pub fn get_ref_field(&self, name: &str) -> Result<JavaRef> {
        self.get_field(name)
    }

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.field(name)?.set((), value)
    }

    pub fn replace<F, R>(
        &self,
        name: &str,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        self.method(name)?.replace(callback)
    }

    pub fn replace_with<'types, F, R>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        self.method(name)?.overload(arguments)?.replace(callback)
    }

    /// Replaces the selected constructor overload with a guarded Rust closure hook.
    ///
    /// The callback must call the selected original constructor through the supplied constructor
    /// context and return the resulting initialization token.
    pub fn replace_constructor<'types, F>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(
                crate::replacement::JavaConstructorHookContext<'a>,
            ) -> Result<crate::replacement::JavaConstructorInitialized<'a>>
            + Send
            + Sync
            + 'static,
    {
        let constructor = self.constructor(arguments)?;
        constructor.replace(callback)
    }

    /// Replaces the selected constructor overload without enforcing original-constructor
    /// initialization.
    ///
    /// # Safety
    ///
    /// Constructor callbacks must initialize the receiver consistently enough for Java code that
    /// observes the object, and must return void.
    pub unsafe fn replace_constructor_unchecked<'types, F, R>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        let constructor = self.constructor(arguments)?;
        unsafe { constructor.replace_unchecked(callback) }
    }

    pub fn is_instance(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        self.class.is_instance(object)
    }

    pub fn cast(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaObject> {
        if self.is_instance(object)? {
            let env = self.class.vm().attach_current_thread()?;
            Ok(JavaObject::from_ref(
                self.clone(),
                object_ref_from_ref(&env, self.class.vm(), object)?,
            ))
        } else {
            let env = self.class.vm().attach_current_thread()?;
            let actual = env.get_object_class(object)?;
            Err(Error::InvalidObjectType {
                operation: "JavaClass::cast",
                expected: "JavaClass target class",
                actual: format!("{:p} is not {}", actual.as_jclass(), self.name()),
            })
        }
    }

    pub fn bind<'object>(
        &self,
        object: &'object impl JavaObjectRef,
    ) -> Result<JavaBoundObject<'object>> {
        if self.is_instance(object)? {
            Ok(JavaBoundObject {
                class: self.clone(),
                object,
            })
        } else {
            let env = self.class.vm().attach_current_thread()?;
            let actual = env.get_object_class(object)?;
            Err(Error::InvalidObjectType {
                operation: "JavaClass::bind",
                expected: "JavaClass target class",
                actual: format!("{:p} is not {}", actual.as_jclass(), self.name()),
            })
        }
    }

    pub fn choose_instances<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(&JavaObject) -> Result<JavaChooseControl>,
    {
        self.class.vm().choose_instances(&self.class, &mut callback)
    }

    fn resolve_method_overload(
        &self,
        kind: MethodKind,
        name: &str,
        arguments: &[JavaType],
    ) -> Result<JavaMethodMetadata> {
        let methods = match kind {
            MethodKind::Constructor => self.declared_methods_cached()?,
            MethodKind::Instance | MethodKind::Static => self.visible_methods_cached()?,
        };
        select_method_by_arguments(self.name(), kind, name, arguments, methods)
    }

    fn resolve_constructor_for_dispatch(
        &self,
        args: &[JavaDispatchArg],
    ) -> Result<JavaConstructor> {
        Ok(JavaConstructor {
            class: self.class.clone(),
            metadata: select_method_by_dispatch_args(
                &self.class,
                MethodDispatchTarget::Constructor,
                "<init>",
                args,
                self.declared_methods_cached()?,
            )?,
        })
    }

    fn visible_methods_cached(&self) -> Result<Vec<JavaMethodMetadata>> {
        if let Some(methods) = self
            .visible_methods
            .lock()
            .expect("JavaClass visible method cache mutex poisoned")
            .as_ref()
        {
            return Ok(methods.clone());
        }

        let loaded = metadata::visible_methods(&self.class.vm().java(), &self.class)?;
        let mut methods = self
            .visible_methods
            .lock()
            .expect("JavaClass visible method cache mutex poisoned");
        Ok(methods.get_or_insert_with(|| loaded).clone())
    }

    fn visible_methods_by_name(&self, name: &str) -> Result<Vec<JavaMethodMetadata>> {
        Ok(self
            .visible_methods_cached()?
            .into_iter()
            .filter(|method| method.name == name)
            .collect())
    }

    fn field_matches_by_name(&self, name: &str) -> Result<Vec<JavaFieldMetadata>> {
        Ok(self
            .visible_fields_cached()?
            .into_iter()
            .filter(|field| field.name == name)
            .collect())
    }

    fn declared_methods_cached(&self) -> Result<Vec<JavaMethodMetadata>> {
        if let Some(methods) = self
            .methods
            .lock()
            .expect("JavaClass declared method cache mutex poisoned")
            .as_ref()
        {
            return Ok(methods.clone());
        }

        let loaded = self.class.declared_methods()?;
        let mut methods = self
            .methods
            .lock()
            .expect("JavaClass declared method cache mutex poisoned");
        Ok(methods.get_or_insert_with(|| loaded).clone())
    }

    fn declared_fields_cached(&self) -> Result<Vec<JavaFieldMetadata>> {
        if let Some(fields) = self
            .fields
            .lock()
            .expect("JavaClass declared field cache mutex poisoned")
            .as_ref()
        {
            return Ok(fields.clone());
        }

        let loaded = self.class.declared_fields()?;
        let mut fields = self
            .fields
            .lock()
            .expect("JavaClass declared field cache mutex poisoned");
        Ok(fields.get_or_insert_with(|| loaded).clone())
    }

    fn visible_fields_cached(&self) -> Result<Vec<JavaFieldMetadata>> {
        if let Some(fields) = self
            .visible_fields
            .lock()
            .expect("JavaClass visible field cache mutex poisoned")
            .as_ref()
        {
            return Ok(fields.clone());
        }

        let loaded = metadata::visible_fields(&self.class.vm().java(), &self.class)?;
        let mut fields = self
            .visible_fields
            .lock()
            .expect("JavaClass visible field cache mutex poisoned");
        Ok(fields.get_or_insert_with(|| loaded).clone())
    }
}

impl JavaMethodGroup {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn overloads(&self) -> &[JavaMethodMetadata] {
        &self.overloads
    }

    pub fn overload<'types>(&self, arguments: impl AsRef<[&'types str]>) -> Result<JavaMethod> {
        let arguments = parse_type_names(arguments.as_ref())?;
        self.overload_by_types(&arguments)
    }

    pub fn overload_by_types(&self, arguments: &[JavaType]) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_group_by_arguments(
                self.class.name(),
                &self.name,
                arguments,
                self.overloads.clone(),
            )?,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        let args = args.into_java_dispatch_args();
        self.dispatch_static(&args)?.call((), args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.overload(arguments)?.call((), args)
    }

    pub fn replace<F, R>(&self, callback: F) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        self.unambiguous()?.replace(callback)
    }

    pub fn replace_with<'types, F, R>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        self.overload(arguments)?.replace(callback)
    }

    fn unambiguous(&self) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_group_by_name(
                self.class.name(),
                &self.name,
                self.overloads.clone(),
            )?,
        })
    }

    fn dispatch_static(&self, args: &[JavaDispatchArg]) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_by_dispatch_args(
                &self.class,
                MethodDispatchTarget::StaticMethod,
                &self.name,
                args,
                self.overloads.clone(),
            )?,
        })
    }

    fn dispatch_bound(&self, args: &[JavaDispatchArg]) -> Result<JavaMethod> {
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_by_dispatch_args(
                &self.class,
                MethodDispatchTarget::BoundMethod,
                &self.name,
                args,
                self.overloads.clone(),
            )?,
        })
    }
}

impl JavaConstructor {
    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub(crate) fn class(&self) -> &RawJavaClass {
        &self.class
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.metadata.signature
    }

    /// Replaces this selected constructor overload with a guarded Rust closure hook.
    ///
    /// The callback receives
    /// [`JavaConstructorHookContext`](crate::replacement::JavaConstructorHookContext)
    /// with `kind()` set to [`MethodKind::Constructor`], `name()`
    /// set to `"<init>"`, and `this_object()` pointing at the object being initialized. The
    /// callback must call the original constructor through `call_original()` or
    /// `call_original_current()` and return the resulting initialization token. Keep the returned
    /// guard alive while the replacement should remain active; reverting or dropping it restores the
    /// original constructor.
    pub fn replace<F>(&self, callback: F) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(
                crate::replacement::JavaConstructorHookContext<'a>,
            ) -> Result<crate::replacement::JavaConstructorInitialized<'a>>
            + Send
            + Sync
            + 'static,
    {
        unsafe { crate::replacement::install_constructor_hook(self, callback) }
    }

    /// Replaces this selected constructor overload without enforcing original-constructor
    /// initialization.
    ///
    /// # Safety
    ///
    /// This is backed by ART method replacement. Constructor callbacks must
    /// initialize the receiver consistently enough for Java code that observes the object, and must
    /// return `()` or [`JavaHookReturn::void()`](crate::replacement::JavaHookReturn::void).
    pub unsafe fn replace_unchecked<F, R>(
        &self,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        unsafe { crate::replacement::install_constructor_hook_unchecked(self, callback) }
    }

    /// Requests ART deoptimization for this selected constructor overload.
    ///
    /// The operation is process-runtime state, so it succeeds only when the current Android ART
    /// backend reports deoptimization support.
    pub fn deoptimize(&self) -> Result<()> {
        self.class.vm().deoptimize_method_id(self.metadata.id)
    }

    pub fn new_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        let args =
            PreparedJavaArgs::new(self.class.vm(), self.metadata.signature.arguments(), args)?;
        Ok(JavaObject::from_ref(
            JavaClass::from_raw(self.class.clone()),
            self.class
                .new_object_ref(&self.metadata.signature.to_string(), args.values())?,
        ))
    }

    #[allow(clippy::new_ret_no_self)]
    pub fn new<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        self.new_object(args)
    }
}

impl JavaMethod {
    pub(crate) fn from_raw_exact(
        class: &RawJavaClass,
        kind: MethodKind,
        name: &str,
        signature: &str,
    ) -> Result<Self> {
        if kind == MethodKind::Constructor {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethod::from_raw_exact",
            });
        }

        let signature = MethodSignature::parse(signature)?;
        let normalized = signature.to_string();
        let method = match kind {
            MethodKind::Static => class.resolve_static_method(name, &normalized)?,
            MethodKind::Instance => class.resolve_instance_method(name, &normalized)?,
            MethodKind::Constructor => unreachable!("constructor was rejected above"),
        };

        Ok(Self {
            class: class.clone(),
            metadata: JavaMethodMetadata {
                name: name.to_owned(),
                kind,
                signature,
                modifiers: 0,
                id: unsafe { method.raw() },
            },
        })
    }

    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub(crate) fn class(&self) -> &RawJavaClass {
        &self.class
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn kind(&self) -> MethodKind {
        self.metadata.kind
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.metadata.signature
    }

    /// Replaces this selected overload with a guarded Rust closure hook.
    ///
    /// The callback receives [`JavaHookContext`](crate::replacement::JavaHookContext),
    /// can call the original method through that invocation, and must return a value implementing
    /// [`IntoJavaHookReturn`](crate::replacement::IntoJavaHookReturn). Keep the
    /// returned guard alive while the replacement should remain active; reverting or dropping it
    /// restores the original method.
    ///
    pub fn replace<F, R>(&self, callback: F) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        unsafe { crate::replacement::install_method_hook(self, callback) }
    }

    /// Requests ART deoptimization for this selected method overload.
    ///
    /// This mirrors upstream selected-method deoptimization while keeping raw `ArtMethod` access
    /// inside the backend.
    pub fn deoptimize(&self) -> Result<()> {
        self.class.vm().deoptimize_method_id(self.metadata.id)
    }

    pub fn call_raw<A: IntoJavaCallArgs>(
        &self,
        receiver: impl JavaMethodReceiver,
        args: A,
    ) -> Result<JavaReturn> {
        receiver.call(self, args)
    }

    pub fn call<T: FromJavaReturn>(
        &self,
        receiver: impl JavaMethodReceiver,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        T::from_java_return(
            self.bind_declared_return(self.call_raw(receiver, args)?)?,
            "JavaMethod::call",
        )
    }

    pub fn call_void<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<()> {
        self.call_raw(object, args)?
            .into_void("JavaMethod::call_void")
    }

    pub fn call_boolean<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<bool> {
        self.call_raw(object, args)?
            .into_boolean("JavaMethod::call_boolean")
    }

    pub fn call_int<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<jni::jint> {
        self.call_raw(object, args)?
            .into_int("JavaMethod::call_int")
    }

    pub fn call_object<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<JavaObject>> {
        self.bind_declared_return(self.call_raw(object, args)?)?
            .into_object("JavaMethod::call_object")
    }

    pub fn call_ref<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<JavaRef>> {
        Ok(self.call_object(object, args)?.map(JavaObject::into_ref))
    }

    pub fn call_array<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<JavaArray>> {
        self.call_raw(object, args)?
            .into_array("JavaMethod::call_array")
    }

    pub fn call_string<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<Option<String>> {
        self.call_object(object, args)?
            .map(|object| object.get_string())
            .transpose()
    }
}

impl JavaMethod {
    fn bind_declared_return(&self, value: JavaReturn) -> Result<JavaReturn> {
        bind_declared_return(
            &self.class,
            self.metadata.signature.return_type(),
            value,
            "JavaMethod::call",
        )
    }
}

pub trait JavaMethodReceiver {
    fn call<A: IntoJavaCallArgs>(&self, method: &JavaMethod, args: A) -> Result<JavaReturn>;
}

impl JavaMethodReceiver for () {
    fn call<A: IntoJavaCallArgs>(&self, method: &JavaMethod, args: A) -> Result<JavaReturn> {
        if method.metadata.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethod::call",
            });
        }
        let args = PreparedJavaArgs::new(
            method.class.vm(),
            method.metadata.signature.arguments(),
            args,
        )?;
        method.class.call_static(
            &method.metadata.name,
            &method.metadata.signature.to_string(),
            args.values(),
        )
    }
}

impl<T: JavaObjectRef + ?Sized> JavaMethodReceiver for &T {
    fn call<A: IntoJavaCallArgs>(&self, method: &JavaMethod, args: A) -> Result<JavaReturn> {
        let args = PreparedJavaArgs::new(
            method.class.vm(),
            method.metadata.signature.arguments(),
            args,
        )?;
        match method.metadata.kind {
            MethodKind::Instance => method.class.call_method(
                *self,
                &method.metadata.name,
                &method.metadata.signature.to_string(),
                args.values(),
            ),
            MethodKind::Static => method.class.call_static(
                &method.metadata.name,
                &method.metadata.signature.to_string(),
                args.values(),
            ),
            MethodKind::Constructor => Err(Error::WrongMethodKind {
                operation: "JavaMethod::call",
            }),
        }
    }
}

impl JavaField {
    pub fn metadata(&self) -> &JavaFieldMetadata {
        &self.metadata
    }

    pub fn name(&self) -> &str {
        &self.metadata.name
    }

    pub fn kind(&self) -> FieldKind {
        self.metadata.kind
    }

    pub fn ty(&self) -> &JavaType {
        &self.metadata.ty
    }

    pub fn get_raw(&self, receiver: impl JavaFieldReceiver) -> Result<JavaReturn> {
        receiver.get(self)
    }

    pub fn get<T: FromJavaReturn>(&self, receiver: impl JavaFieldReceiver) -> Result<T> {
        T::from_java_return(
            self.bind_declared_return(self.get_raw(receiver)?)?,
            "JavaField::get",
        )
    }

    pub fn get_boolean(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        self.get_raw(object)?.into_boolean("JavaField::get_boolean")
    }

    pub fn get_byte(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jbyte> {
        self.get_raw(object)?.into_byte("JavaField::get_byte")
    }

    pub fn get_char(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jchar> {
        self.get_raw(object)?.into_char("JavaField::get_char")
    }

    pub fn get_short(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jshort> {
        self.get_raw(object)?.into_short("JavaField::get_short")
    }

    pub fn get_int(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jint> {
        self.get_raw(object)?.into_int("JavaField::get_int")
    }

    pub fn get_long(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jlong> {
        self.get_raw(object)?.into_long("JavaField::get_long")
    }

    pub fn get_float(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jfloat> {
        self.get_raw(object)?.into_float("JavaField::get_float")
    }

    pub fn get_double(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jdouble> {
        self.get_raw(object)?.into_double("JavaField::get_double")
    }

    pub fn get_object(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<Option<JavaObject>> {
        self.bind_declared_return(self.get_raw(object)?)?
            .into_object("JavaField::get_object")
    }

    pub fn get_ref(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<Option<JavaRef>> {
        Ok(self.get_object(object)?.map(JavaObject::into_ref))
    }

    pub fn get_array(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<Option<JavaArray>> {
        self.get_raw(object)?.into_array("JavaField::get_array")
    }

    pub fn set<V: IntoJavaFieldValue>(
        &self,
        receiver: impl JavaFieldReceiver,
        value: V,
    ) -> Result<()> {
        receiver.set(self, value)
    }

    pub fn set_boolean(&self, object: &(impl JavaObjectRef + ?Sized), value: bool) -> Result<()> {
        self.set(object, JavaValue::Boolean(value))
    }

    pub fn set_byte(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jbyte,
    ) -> Result<()> {
        self.set(object, JavaValue::Byte(value))
    }

    pub fn set_char(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jchar,
    ) -> Result<()> {
        self.set(object, JavaValue::Char(value))
    }

    pub fn set_short(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jshort,
    ) -> Result<()> {
        self.set(object, JavaValue::Short(value))
    }

    pub fn set_int(&self, object: &(impl JavaObjectRef + ?Sized), value: jni::jint) -> Result<()> {
        self.set(object, JavaValue::Int(value))
    }

    pub fn set_long(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jlong,
    ) -> Result<()> {
        self.set(object, JavaValue::Long(value))
    }

    pub fn set_float(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jfloat,
    ) -> Result<()> {
        self.set(object, JavaValue::Float(value))
    }

    pub fn set_double(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: jni::jdouble,
    ) -> Result<()> {
        self.set(object, JavaValue::Double(value))
    }

    pub fn set_object<T: JavaObjectRef + ?Sized>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: Option<&T>,
    ) -> Result<()> {
        self.set(
            object,
            value.map_or(JavaValue::Null, |value| {
                JavaValue::object_ref(value.as_jobject())
            }),
        )
    }
}

impl JavaField {
    fn bind_declared_return(&self, value: JavaReturn) -> Result<JavaReturn> {
        bind_declared_return(&self.class, &self.metadata.ty, value, "JavaField::get")
    }
}

pub trait JavaFieldReceiver {
    fn get(&self, field: &JavaField) -> Result<JavaReturn>;
    fn set<V: IntoJavaFieldValue>(&self, field: &JavaField, value: V) -> Result<()>;
}

impl JavaFieldReceiver for () {
    fn get(&self, field: &JavaField) -> Result<JavaReturn> {
        if field.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::get",
            });
        }
        field
            .class
            .get_static_field(&field.metadata.name, &field.metadata.ty.to_string())
    }

    fn set<V: IntoJavaFieldValue>(&self, field: &JavaField, value: V) -> Result<()> {
        if field.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::set",
            });
        }
        let env = field.class.vm().attach_current_thread()?;
        let value = value.into_java_field_value(&env, &field.metadata.ty, "JavaField::set")?;
        let result = field.class.set_static_field(
            &field.metadata.name,
            &field.metadata.ty.to_string(),
            value.value(),
        );
        value.delete_local_ref(&env);
        result
    }
}

impl<T: JavaObjectRef + ?Sized> JavaFieldReceiver for &T {
    fn get(&self, field: &JavaField) -> Result<JavaReturn> {
        if field.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::get",
            });
        }
        field
            .class
            .get_field(*self, &field.metadata.name, &field.metadata.ty.to_string())
    }

    fn set<V: IntoJavaFieldValue>(&self, field: &JavaField, value: V) -> Result<()> {
        if field.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::set",
            });
        }
        let env = field.class.vm().attach_current_thread()?;
        let value = value.into_java_field_value(&env, &field.metadata.ty, "JavaField::set")?;
        let result = field.class.set_field(
            *self,
            &field.metadata.name,
            &field.metadata.ty.to_string(),
            value.value(),
        );
        value.delete_local_ref(&env);
        result
    }
}

impl<'object> JavaBoundObject<'object> {
    pub fn class(&self) -> &JavaClass {
        &self.class
    }

    pub fn object(&self) -> &'object dyn JavaObjectRef {
        self.object
    }

    pub fn method(&self, name: &str) -> Result<JavaBoundMethodGroup<'object>> {
        Ok(JavaBoundMethodGroup {
            object: self.object,
            group: self.class.method(name)?,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.method(name)?.call(args)
    }

    pub fn call_ref(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<JavaRef> {
        self.call(name, args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.method(name)?.overload(arguments)?.call(args)
    }

    pub fn field(&self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        Ok(JavaBoundFieldHandle {
            object: self.object,
            field: self.class.field(name)?,
        })
    }

    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.field(name)?.get()
    }

    pub fn get_ref_field(&self, name: &str) -> Result<JavaRef> {
        self.get_field(name)
    }

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.field(name)?.set(value)
    }
}

impl<'object> JavaBoundMethodGroup<'object> {
    pub fn name(&self) -> &str {
        self.group.name()
    }

    pub fn overloads(&self) -> &[JavaMethodMetadata] {
        self.group.overloads()
    }

    pub fn overload<'types>(
        &self,
        arguments: impl AsRef<[&'types str]>,
    ) -> Result<JavaBoundMethodOverload<'object>> {
        Ok(JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.overload(arguments)?,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        let args = args.into_java_dispatch_args();
        JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.dispatch_bound(&args)?,
        }
        .call(args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.overload(arguments)?.call(args)
    }

    pub fn replace<F, R>(&self, callback: F) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        self.group.replace(callback)
    }

    pub fn replace_with<'types, F, R>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        self.group.replace_with(arguments, callback)
    }
}

impl JavaBoundMethodOverload<'_> {
    pub fn overload(&self) -> &JavaMethod {
        &self.overload
    }

    pub fn call_raw<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaReturn> {
        self.overload.call_raw(self.object, args)
    }

    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        T::from_java_return(
            self.overload.bind_declared_return(self.call_raw(args)?)?,
            "JavaBoundMethodOverload::call",
        )
    }
}

impl JavaBoundFieldHandle<'_> {
    pub fn field(&self) -> &JavaField {
        &self.field
    }

    pub fn get_raw(&self) -> Result<JavaReturn> {
        match self.field.kind() {
            FieldKind::Static => self.field.get_raw(()),
            FieldKind::Instance => self.field.get_raw(self.object),
        }
    }

    pub fn get<T: FromJavaReturn>(&self) -> Result<T> {
        T::from_java_return(
            self.field.bind_declared_return(self.get_raw()?)?,
            "JavaBoundFieldHandle::get",
        )
    }

    pub fn set<V: IntoJavaFieldValue>(&self, value: V) -> Result<()> {
        match self.field.kind() {
            FieldKind::Static => self.field.set((), value),
            FieldKind::Instance => self.field.set(self.object, value),
        }
    }
}

#[derive(Clone, Copy)]
enum MethodDispatchTarget {
    Constructor,
    StaticMethod,
    BoundMethod,
}

fn select_method_by_dispatch_args(
    holder: &RawJavaClass,
    target: MethodDispatchTarget,
    name: &str,
    args: &[JavaDispatchArg],
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let mut candidates = Vec::new();
    let mut best: Option<(i32, usize, JavaMethodMetadata)> = None;

    for (index, method) in methods.into_iter().enumerate() {
        if !dispatch_target_accepts(target, name, &method) {
            continue;
        }

        candidates.push(format!(
            "{} {}",
            method_kind_name(method.kind),
            method.signature
        ));

        let Some(score) = dispatch_score(holder, args, method.signature.arguments())? else {
            continue;
        };

        if best
            .as_ref()
            .is_none_or(|(best_score, best_index, _)| (score, index) < (*best_score, *best_index))
        {
            best = Some((score, index, method));
        }
    }

    best.map(|(_, _, method)| method)
        .ok_or_else(|| Error::NoCompatibleOverload {
            class: holder.name().to_owned(),
            kind: dispatch_target_kind_name(target),
            name: dispatch_target_method_name(target, name).to_owned(),
            arguments: format_dispatch_argument_list(args),
            candidates,
        })
}

fn dispatch_target_accepts(
    target: MethodDispatchTarget,
    name: &str,
    method: &JavaMethodMetadata,
) -> bool {
    match target {
        MethodDispatchTarget::Constructor => method.kind == MethodKind::Constructor,
        MethodDispatchTarget::StaticMethod => {
            method.kind == MethodKind::Static && method.name == name
        }
        MethodDispatchTarget::BoundMethod => {
            method.kind != MethodKind::Constructor && method.name == name
        }
    }
}

fn dispatch_target_kind_name(target: MethodDispatchTarget) -> &'static str {
    match target {
        MethodDispatchTarget::Constructor => method_kind_name(MethodKind::Constructor),
        MethodDispatchTarget::StaticMethod => method_kind_name(MethodKind::Static),
        MethodDispatchTarget::BoundMethod => "method",
    }
}

fn dispatch_target_method_name(target: MethodDispatchTarget, name: &str) -> &str {
    match target {
        MethodDispatchTarget::Constructor => "$init",
        MethodDispatchTarget::StaticMethod | MethodDispatchTarget::BoundMethod => name,
    }
}

fn dispatch_score(
    holder: &RawJavaClass,
    args: &[JavaDispatchArg],
    expected: &[JavaType],
) -> Result<Option<i32>> {
    if args.len() != expected.len() {
        return Ok(None);
    }

    let mut score = 0;
    for (arg, expected) in args.iter().zip(expected) {
        let Some(arg_score) = dispatch_arg_score(holder, arg, expected)? else {
            return Ok(None);
        };
        score += arg_score;
    }
    Ok(Some(score))
}

fn dispatch_arg_score(
    holder: &RawJavaClass,
    arg: &JavaDispatchArg,
    expected: &JavaType,
) -> Result<Option<i32>> {
    match arg {
        JavaDispatchArg::RustString(_) => Ok(rust_string_dispatch_score(expected)),
        JavaDispatchArg::Value(JavaValue::Null) => Ok(expected.is_reference().then_some(50)),
        JavaDispatchArg::Value(JavaValue::Object(value)) if value.is_null() => {
            Ok(expected.is_reference().then_some(50))
        }
        JavaDispatchArg::Value(JavaValue::Object(value)) => {
            reference_dispatch_score(holder, value.as_jobject(), expected)
        }
        JavaDispatchArg::Value(value) if primitive_exact_match(*value, expected) => Ok(Some(0)),
        JavaDispatchArg::Value(value) if super::args::can_coerce_java_value(*value, expected) => {
            Ok(Some(10))
        }
        JavaDispatchArg::Value(_) => Ok(None),
    }
}

fn primitive_exact_match(value: JavaValue, expected: &JavaType) -> bool {
    matches!(
        (value, expected),
        (JavaValue::Boolean(_), JavaType::Boolean)
            | (JavaValue::Byte(_), JavaType::Byte)
            | (JavaValue::Char(_), JavaType::Char)
            | (JavaValue::Short(_), JavaType::Short)
            | (JavaValue::Int(_), JavaType::Int)
            | (JavaValue::Long(_), JavaType::Long)
            | (JavaValue::Float(_), JavaType::Float)
            | (JavaValue::Double(_), JavaType::Double)
    )
}

fn rust_string_dispatch_score(expected: &JavaType) -> Option<i32> {
    match expected {
        JavaType::Object(class) if class == "java/lang/String" => Some(0),
        JavaType::Object(class) if class == "java/lang/CharSequence" => Some(1),
        JavaType::Object(class) if class == "java/lang/Object" => Some(2),
        _ => None,
    }
}

fn reference_dispatch_score(
    holder: &RawJavaClass,
    object: jni::jobject,
    expected: &JavaType,
) -> Result<Option<i32>> {
    if !expected.is_reference() {
        return Ok(None);
    }

    let actual_descriptor = object_class_descriptor(holder, object)?;
    if let Some(score) = reference_descriptor_dispatch_score(&actual_descriptor, expected) {
        return Ok(Some(score));
    }

    let expected_class = class_for_dispatch_type(holder, expected)?;
    let env = holder.vm().attach_current_thread()?;
    if !env.is_instance_of(&RawObject(object), &expected_class.inner.class)? {
        return Ok(None);
    }

    Ok(Some(match expected {
        JavaType::Array(_) => 1,
        JavaType::Object(class) if class == "java/lang/Object" => 30,
        JavaType::Object(_) => 10,
        JavaType::Void
        | JavaType::Boolean
        | JavaType::Byte
        | JavaType::Char
        | JavaType::Short
        | JavaType::Int
        | JavaType::Long
        | JavaType::Float
        | JavaType::Double => unreachable!("non-reference types were rejected above"),
    }))
}

fn reference_descriptor_dispatch_score(
    actual_descriptor: &str,
    expected: &JavaType,
) -> Option<i32> {
    if actual_descriptor == expected.to_string() {
        return Some(0);
    }

    match expected {
        JavaType::Object(class)
            if class == "java/lang/Object"
                && (actual_descriptor.starts_with('L') || actual_descriptor.starts_with('[')) =>
        {
            Some(30)
        }
        _ => None,
    }
}

fn object_class_descriptor(holder: &RawJavaClass, object: jni::jobject) -> Result<String> {
    let env = holder.vm().attach_current_thread()?;
    let class = env.get_object_class(&RawObject(object))?;
    metadata::class_descriptor(&env, &class)
}

fn class_for_dispatch_type(holder: &RawJavaClass, ty: &JavaType) -> Result<RawJavaClass> {
    let env = holder.vm().attach_current_thread()?;
    let java = holder.vm().java();
    let scoped_java = match metadata::class_loader(&env, &java, holder)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    scoped_java.find_class(&dispatch_class_lookup_name(ty))
}

fn dispatch_class_lookup_name(ty: &JavaType) -> String {
    match ty {
        JavaType::Object(name) => name.replace('/', "."),
        JavaType::Array(_) => ty.to_string(),
        JavaType::Void
        | JavaType::Boolean
        | JavaType::Byte
        | JavaType::Char
        | JavaType::Short
        | JavaType::Int
        | JavaType::Long
        | JavaType::Float
        | JavaType::Double => ty.to_string(),
    }
}

fn format_dispatch_argument_list(args: &[JavaDispatchArg]) -> String {
    format!(
        "({})",
        args.iter()
            .map(JavaDispatchArg::type_name)
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn method_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Constructor => "constructor",
        MethodKind::Instance => "instance",
        MethodKind::Static => "static",
    }
}

fn bind_declared_return(
    holder: &RawJavaClass,
    ty: &JavaType,
    value: JavaReturn,
    operation: &'static str,
) -> Result<JavaReturn> {
    let JavaType::Object(name) = ty else {
        return Ok(value);
    };
    let JavaReturn::Object(object) = value else {
        return Ok(value);
    };
    let Some(object) = object else {
        return Ok(JavaReturn::Object(None));
    };

    let env = holder.vm().attach_current_thread()?;
    let java = holder.vm().java();
    let scoped_java = match metadata::class_loader(&env, &java, holder)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    let class = JavaClass::from_raw(scoped_java.find_class(&name.replace('/', "."))?);
    if class.is_instance(&object)? {
        Ok(JavaReturn::Object(Some(JavaObject::from_ref(
            class,
            object.into_ref(),
        ))))
    } else {
        let actual = env.get_object_class(&object)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "declared return type",
            actual: format!("{:p} is not {}", actual.as_jclass(), name.replace('/', ".")),
        })
    }
}

fn field_kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Instance => "instance",
        FieldKind::Static => "static",
    }
}

fn select_field_by_name(
    class: &str,
    name: &str,
    fields: Vec<JavaFieldMetadata>,
) -> Result<JavaFieldMetadata> {
    match fields.len() {
        0 => Err(Error::FieldNameNotFound {
            class: class.to_owned(),
            kind: "field",
            name: name.to_owned(),
        }),
        1 => Ok(fields.into_iter().next().expect("one field match")),
        _ => Err(Error::AmbiguousField {
            class: class.to_owned(),
            kind: "field",
            name: name.to_owned(),
            candidates: fields
                .iter()
                .map(|field| format!("{} {}", field_kind_name(field.kind), field.ty))
                .collect(),
        }),
    }
}

fn wrapper_method_name(kind: MethodKind, name: &str) -> &str {
    if kind == MethodKind::Constructor {
        "$init"
    } else {
        name
    }
}

fn select_method_group_by_name(
    class: &str,
    name: &str,
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| method.kind != MethodKind::Constructor && method.name == name)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::MethodNameNotFound {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
        }),
        1 => Ok(matches.into_iter().next().expect("one method match")),
        _ => Err(Error::AmbiguousMethod {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
            candidates: matches
                .iter()
                .map(|method| format!("{} {}", method_kind_name(method.kind), method.signature))
                .collect(),
        }),
    }
}

#[cfg(test)]
fn select_constructor_by_name(
    class: &str,
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| method.kind == MethodKind::Constructor)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::MethodNameNotFound {
            class: class.to_owned(),
            kind: method_kind_name(MethodKind::Constructor),
            name: "$init".to_owned(),
        }),
        1 => Ok(matches.into_iter().next().expect("one constructor match")),
        _ => Err(Error::AmbiguousMethod {
            class: class.to_owned(),
            kind: method_kind_name(MethodKind::Constructor),
            name: "$init".to_owned(),
            candidates: matches
                .iter()
                .map(|method| method.signature.to_string())
                .collect(),
        }),
    }
}

fn select_method_by_arguments(
    class: &str,
    kind: MethodKind,
    name: &str,
    arguments: &[JavaType],
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| {
            method.kind == kind && method.name == name && method.signature.arguments() == arguments
        })
        .collect::<Vec<_>>();

    select_method_overload_match(class, kind, name, format_argument_list(arguments), matches)
}

fn select_method_group_by_arguments(
    class: &str,
    name: &str,
    arguments: &[JavaType],
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| {
            method.kind != MethodKind::Constructor
                && method.name == name
                && method.signature.arguments() == arguments
        })
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::OverloadNotFound {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
            arguments: format_argument_list(arguments),
        }),
        1 => Ok(matches.into_iter().next().expect("one overload match")),
        matches => Err(Error::AmbiguousOverload {
            class: class.to_owned(),
            kind: "method",
            name: name.to_owned(),
            arguments: format_argument_list(arguments),
            matches,
        }),
    }
}

fn select_method_overload_match(
    class: &str,
    kind: MethodKind,
    name: &str,
    arguments: String,
    matches: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    match matches.len() {
        0 => Err(Error::OverloadNotFound {
            class: class.to_owned(),
            kind: method_kind_name(kind),
            name: wrapper_method_name(kind, name).to_owned(),
            arguments,
        }),
        1 => Ok(matches.into_iter().next().expect("one overload match")),
        matches => Err(Error::AmbiguousOverload {
            class: class.to_owned(),
            kind: method_kind_name(kind),
            name: wrapper_method_name(kind, name).to_owned(),
            arguments,
            matches,
        }),
    }
}

fn parse_type_names(names: &[&str]) -> Result<Vec<JavaType>> {
    names.iter().map(|name| JavaType::from_name(name)).collect()
}

fn format_argument_list(arguments: &[JavaType]) -> String {
    let mut formatted = String::from("(");
    for argument in arguments {
        formatted.push_str(&argument.to_string());
    }
    formatted.push(')');
    formatted
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use super::*;

    const CLASS: &str = "com.example.Subject";

    fn method(name: &str, kind: MethodKind, signature: &str) -> JavaMethodMetadata {
        JavaMethodMetadata {
            name: name.to_owned(),
            kind,
            signature: MethodSignature::parse(signature).unwrap(),
            modifiers: 0,
            id: ptr::null_mut(),
        }
    }

    fn field(name: &str, kind: FieldKind, ty: &str) -> JavaFieldMetadata {
        JavaFieldMetadata {
            name: name.to_owned(),
            kind,
            ty: JavaType::parse(ty).unwrap(),
            modifiers: 0,
            id: ptr::null_mut(),
        }
    }

    fn holder() -> RawJavaClass {
        let vm = Vm::dangling_for_tests();
        let class = unsafe { GlobalRef::from_raw(vm.clone(), ptr::dangling_mut()) }.unwrap();
        RawJavaClass::from_global(vm, CLASS.to_owned(), class)
    }

    #[test]
    fn resolves_string_selector_for_unambiguous_method() {
        let selected = select_method_group_by_name(
            CLASS,
            "onResume",
            vec![method("onResume", MethodKind::Instance, "()V")],
        )
        .unwrap();

        assert_eq!(selected.name, "onResume");
        assert_eq!(selected.signature.to_string(), "()V");
    }

    #[test]
    fn resolves_unambiguous_instance_field_selector() {
        let selected = select_field_by_name(
            CLASS,
            "number",
            vec![field("number", FieldKind::Instance, "I")],
        )
        .unwrap();

        assert_eq!(selected.name, "number");
        assert_eq!(selected.kind, FieldKind::Instance);
        assert_eq!(selected.ty, JavaType::Int);
    }

    #[test]
    fn resolves_unambiguous_static_field_selector() {
        let selected = select_field_by_name(
            CLASS,
            "answer",
            vec![field("answer", FieldKind::Static, "Ljava/lang/String;")],
        )
        .unwrap();

        assert_eq!(selected.name, "answer");
        assert_eq!(selected.kind, FieldKind::Static);
        assert_eq!(selected.ty, JavaType::Object("java/lang/String".to_owned()));
    }

    #[test]
    fn reports_missing_field_selector() {
        let error = select_field_by_name(CLASS, "missing", vec![]).unwrap_err();

        match error {
            Error::FieldNameNotFound {
                class,
                kind: "field",
                name,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "missing");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_same_tier_field_selector_with_candidate_kinds() {
        let selected = select_field_by_name(
            CLASS,
            "sameName",
            vec![
                field("sameName", FieldKind::Instance, "I"),
                field("sameName", FieldKind::Static, "J"),
            ],
        )
        .unwrap_err();

        match selected {
            Error::AmbiguousField {
                class,
                kind: "field",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "sameName");
                assert_eq!(
                    candidates,
                    vec!["instance I".to_owned(), "static J".to_owned()]
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn resolves_type_list_selector_for_overload() {
        let arguments = parse_type_names(&["java.lang.String", "int"]).unwrap();
        let selected = select_method_by_arguments(
            CLASS,
            MethodKind::Instance,
            "set",
            &arguments,
            vec![
                method("set", MethodKind::Instance, "(I)V"),
                method("set", MethodKind::Instance, "(Ljava/lang/String;I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(Ljava/lang/String;I)V");
    }

    #[test]
    fn reports_missing_type_list_overload() {
        let arguments = parse_type_names(&["java.lang.String"]).unwrap();
        let error = select_method_by_arguments(
            CLASS,
            MethodKind::Instance,
            "set",
            &arguments,
            vec![method("set", MethodKind::Instance, "(I)V")],
        )
        .unwrap_err();

        match error {
            Error::OverloadNotFound {
                class,
                kind: "instance",
                name,
                arguments,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "set");
                assert_eq!(arguments, "(Ljava/lang/String;)");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_bare_name_with_candidate_signatures() {
        let error = select_method_group_by_name(
            CLASS,
            "overload",
            vec![
                method("overload", MethodKind::Instance, "()I"),
                method("overload", MethodKind::Instance, "(I)I"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousMethod {
                class,
                kind: "method",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "overload");
                assert_eq!(
                    candidates,
                    vec!["instance ()I".to_owned(), "instance (I)I".to_owned()]
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn resolves_unambiguous_constructor_for_class_new() {
        let selected = select_constructor_by_name(
            CLASS,
            vec![
                method("ignored", MethodKind::Static, "()I"),
                method("<init>", MethodKind::Constructor, "(I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.name, "<init>");
        assert_eq!(selected.signature.to_string(), "(I)V");
    }

    #[test]
    fn reports_missing_constructor_for_class_new() {
        let error =
            select_constructor_by_name(CLASS, vec![method("answer", MethodKind::Static, "()I")])
                .unwrap_err();

        match error {
            Error::MethodNameNotFound {
                class,
                kind: "constructor",
                name,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "$init");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_constructor_for_class_new() {
        let error = select_constructor_by_name(
            CLASS,
            vec![
                method("<init>", MethodKind::Constructor, "()V"),
                method("<init>", MethodKind::Constructor, "(I)V"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousMethod {
                class,
                kind: "constructor",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "$init");
                assert_eq!(candidates, vec!["()V".to_owned(), "(I)V".to_owned()]);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn replacement_selector_accepts_static_or_instance_method() {
        let selected = select_method_group_by_name(
            CLASS,
            "answer",
            vec![method("answer", MethodKind::Static, "()I")],
        )
        .unwrap();
        assert_eq!(selected.kind, MethodKind::Static);
        assert_eq!(selected.signature.to_string(), "()I");

        let arguments = parse_type_names(&["java.lang.String"]).unwrap();
        let selected = select_method_group_by_arguments(
            CLASS,
            "message",
            &arguments,
            vec![method(
                "message",
                MethodKind::Instance,
                "(Ljava/lang/String;)Ljava/lang/String;",
            )],
        )
        .unwrap();
        assert_eq!(selected.kind, MethodKind::Instance);
        assert_eq!(
            selected.signature.to_string(),
            "(Ljava/lang/String;)Ljava/lang/String;"
        );
    }

    #[test]
    fn replacement_selector_reports_static_instance_ambiguity() {
        let error = select_method_group_by_arguments(
            CLASS,
            "sameShape",
            &[],
            vec![
                method("sameShape", MethodKind::Instance, "()I"),
                method("sameShape", MethodKind::Static, "()I"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousOverload {
                class,
                kind: "method",
                name,
                arguments,
                matches: 2,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "sameShape");
                assert_eq!(arguments, "()");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn dispatch_filters_by_arity() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "set",
            &[JavaDispatchArg::Value(JavaValue::Int(7))],
            vec![
                method("set", MethodKind::Instance, "()V"),
                method("set", MethodKind::Instance, "(I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(I)V");
    }

    #[test]
    fn bound_dispatch_reports_method_failures() {
        let error = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "set",
            &[JavaDispatchArg::Value(JavaValue::Int(7))],
            vec![
                method("set", MethodKind::Instance, "()V"),
                method("set", MethodKind::Static, "()V"),
            ],
        )
        .unwrap_err();

        match error {
            Error::NoCompatibleOverload {
                class,
                kind: "method",
                name,
                arguments,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "set");
                assert_eq!(arguments, "(int)");
                assert_eq!(
                    candidates,
                    vec!["instance ()V".to_owned(), "static ()V".to_owned()]
                );
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn dispatch_prefers_exact_primitive_over_coercion() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::StaticMethod,
            "number",
            &[JavaDispatchArg::Value(JavaValue::Int(7))],
            vec![
                method("number", MethodKind::Static, "(J)V"),
                method("number", MethodKind::Static, "(I)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(I)V");
    }

    #[test]
    fn dispatch_ranks_rust_string_targets() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "text",
            &[JavaDispatchArg::RustString("hello".to_owned())],
            vec![
                method("text", MethodKind::Instance, "(Ljava/lang/Object;)V"),
                method("text", MethodKind::Instance, "(Ljava/lang/CharSequence;)V"),
                method("text", MethodKind::Instance, "(Ljava/lang/String;)V"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(Ljava/lang/String;)V");
    }

    #[test]
    fn dispatch_preserves_order_for_tied_scores() {
        let selected = select_method_by_dispatch_args(
            &holder(),
            MethodDispatchTarget::BoundMethod,
            "nullable",
            &[JavaDispatchArg::Value(JavaValue::Null)],
            vec![
                method(
                    "nullable",
                    MethodKind::Instance,
                    "(Ljava/lang/CharSequence;)V",
                ),
                method("nullable", MethodKind::Instance, "(Ljava/lang/String;)V"),
            ],
        )
        .unwrap();

        assert_eq!(
            selected.signature.to_string(),
            "(Ljava/lang/CharSequence;)V"
        );
    }

    #[test]
    fn array_descriptor_exact_match_scores_before_object() {
        assert_eq!(
            reference_descriptor_dispatch_score("[I", &JavaType::Array(Box::new(JavaType::Int))),
            Some(0)
        );
        assert_eq!(
            reference_descriptor_dispatch_score(
                "[I",
                &JavaType::Object("java/lang/Object".to_owned())
            ),
            Some(30)
        );
    }
}
