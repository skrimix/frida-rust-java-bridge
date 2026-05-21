use super::*;

impl JavaClass {
    pub(super) fn new(class: RawJavaClass) -> Self {
        Self {
            class,
            methods: Rc::new(RefCell::new(None)),
            instance_methods: Rc::new(RefCell::new(None)),
            fields: Rc::new(RefCell::new(None)),
            instance_fields: Rc::new(RefCell::new(None)),
        }
    }

    pub fn name(&self) -> &str {
        self.class.name()
    }

    pub fn class(&self) -> &RawJavaClass {
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

    pub fn methods(&self, name: &str) -> Result<Vec<JavaMethodMetadata>> {
        Ok(self
            .declared_methods_cached()?
            .into_iter()
            .filter(|method| method.name == name && method.kind != MethodKind::Constructor)
            .collect())
    }

    pub fn method<S: JavaMethodSelector>(&self, selector: S) -> Result<S::Output> {
        selector.resolve(self, MethodKind::Instance)
    }

    pub fn static_method<S: JavaMethodSelector>(&self, selector: S) -> Result<S::Output> {
        selector.resolve(self, MethodKind::Static)
    }

    pub fn fields(&self, name: &str) -> Result<Vec<JavaFieldMetadata>> {
        Ok(self
            .declared_fields_cached()?
            .into_iter()
            .filter(|field| field.name == name)
            .collect())
    }

    pub fn constructor_overload(&self, arguments: &[JavaType]) -> Result<JavaConstructor> {
        let metadata =
            self.resolve_method_overload(MethodKind::Constructor, "<init>", arguments)?;
        Ok(JavaConstructor {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn constructor_overload_by_name(&self, arguments: &[&str]) -> Result<JavaConstructor> {
        let arguments = parse_type_names(arguments)?;
        self.constructor_overload(&arguments)
    }

    pub fn constructor<const N: usize>(&self, arguments: [&str; N]) -> Result<JavaConstructor> {
        self.constructor_overload_by_name(&arguments)
    }

    pub fn new_instance<const N: usize, A: IntoJavaCallArgs>(
        &self,
        arguments: [&str; N],
        args: A,
    ) -> Result<JavaObject> {
        self.constructor(arguments)?.new_object(args)
    }

    pub fn method_overload(&self, name: &str, arguments: &[JavaType]) -> Result<JavaMethod> {
        let metadata = self.resolve_method_overload(MethodKind::Instance, name, arguments)?;
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn method_overload_by_name(&self, name: &str, arguments: &[&str]) -> Result<JavaMethod> {
        let arguments = parse_type_names(arguments)?;
        self.method_overload(name, &arguments)
    }

    pub fn overload<const N: usize>(&self, name: &str, arguments: [&str; N]) -> Result<JavaMethod> {
        self.method_overload_by_name(name, &arguments)
    }

    pub fn static_method_overload(&self, name: &str, arguments: &[JavaType]) -> Result<JavaMethod> {
        let metadata = self.resolve_method_overload(MethodKind::Static, name, arguments)?;
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn static_method_overload_by_name(
        &self,
        name: &str,
        arguments: &[&str],
    ) -> Result<JavaMethod> {
        let arguments = parse_type_names(arguments)?;
        self.static_method_overload(name, &arguments)
    }

    pub fn static_overload<const N: usize>(
        &self,
        name: &str,
        arguments: [&str; N],
    ) -> Result<JavaMethod> {
        self.static_method_overload_by_name(name, &arguments)
    }

    pub fn field_handle(&self, name: &str) -> Result<JavaField> {
        let metadata = self.resolve_field(FieldKind::Instance, name)?;
        Ok(JavaField {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn static_field_handle(&self, name: &str) -> Result<JavaField> {
        let metadata = self.resolve_field(FieldKind::Static, name)?;
        Ok(JavaField {
            class: self.class.clone(),
            metadata,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn new_object_raw<A: IntoJavaCallArgs>(
        &self,
        signature: &str,
        args: A,
    ) -> Result<JavaObject> {
        self.ensure_method(MethodKind::Constructor, "<init>", signature)?;
        let signature = MethodSignature::parse(signature)?;
        let args = PreparedJavaArgs::new(self.class.vm(), signature.arguments(), args)?;
        self.class.new_object(&signature.to_string(), args.values())
    }

    #[allow(dead_code)]
    pub(crate) fn call_raw<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        name: &str,
        signature: &str,
        args: A,
    ) -> Result<JavaReturn> {
        self.ensure_method(MethodKind::Instance, name, signature)?;
        let signature = MethodSignature::parse(signature)?;
        let args = PreparedJavaArgs::new(self.class.vm(), signature.arguments(), args)?;
        self.class
            .call_method(object, name, &signature.to_string(), args.values())
    }

    #[allow(dead_code)]
    pub(crate) fn call_static_raw<A: IntoJavaCallArgs>(
        &self,
        name: &str,
        signature: &str,
        args: A,
    ) -> Result<JavaReturn> {
        self.ensure_method(MethodKind::Static, name, signature)?;
        let signature = MethodSignature::parse(signature)?;
        let args = PreparedJavaArgs::new(self.class.vm(), signature.arguments(), args)?;
        self.class
            .call_static(name, &signature.to_string(), args.values())
    }

    #[allow(dead_code)]
    pub(crate) fn get_static_field_raw(&self, name: &str, ty: &str) -> Result<JavaReturn> {
        self.ensure_field(FieldKind::Static, name, ty)?;
        self.class.get_static_field(name, ty)
    }

    pub fn is_instance(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        self.class.is_instance(object)
    }

    pub fn cast(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaObject> {
        if self.is_instance(object)? {
            let env = self.class.vm().attach_current_thread()?;
            object_from_ref(&env, self.class.vm(), object)
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

    #[allow(dead_code)]
    fn ensure_method(&self, kind: MethodKind, name: &str, signature: &str) -> Result<()> {
        let signature = MethodSignature::parse(signature)?.to_string();
        if self.declared_methods_cached()?.iter().any(|method| {
            method.kind == kind && method.name == name && method.signature.to_string() == signature
        }) {
            Ok(())
        } else {
            Err(Error::MethodNotFound {
                class: self.name().to_owned(),
                kind: method_kind_name(kind),
                name: name.to_owned(),
                signature,
            })
        }
    }

    #[allow(dead_code)]
    fn ensure_field(&self, kind: FieldKind, name: &str, ty: &str) -> Result<()> {
        let ty = JavaType::parse(ty)?.to_string();
        if self
            .declared_fields_cached()?
            .iter()
            .any(|field| field.kind == kind && field.name == name && field.ty.to_string() == ty)
        {
            Ok(())
        } else {
            Err(Error::FieldNotFound {
                class: self.name().to_owned(),
                kind: field_kind_name(kind),
                name: name.to_owned(),
                ty,
            })
        }
    }

    fn resolve_method_overload(
        &self,
        kind: MethodKind,
        name: &str,
        arguments: &[JavaType],
    ) -> Result<JavaMethodMetadata> {
        let methods = match kind {
            MethodKind::Instance => self.instance_methods_cached()?,
            MethodKind::Constructor | MethodKind::Static => self.declared_methods_cached()?,
        };
        select_method_by_arguments(self.name(), kind, name, arguments, methods)
    }

    fn resolve_named_method(&self, kind: MethodKind, name: &str) -> Result<JavaMethod> {
        let methods = match kind {
            MethodKind::Instance => self.instance_methods_cached()?,
            MethodKind::Constructor | MethodKind::Static => self.declared_methods_cached()?,
        };
        Ok(JavaMethod {
            class: self.class.clone(),
            metadata: select_method_by_name(self.name(), kind, name, methods)?,
        })
    }

    fn resolve_field(&self, kind: FieldKind, name: &str) -> Result<JavaFieldMetadata> {
        let fields = match kind {
            FieldKind::Instance => self.instance_fields_cached()?,
            FieldKind::Static => self.declared_fields_cached()?,
        };
        let matches = fields
            .into_iter()
            .filter(|field| field.kind == kind && field.name == name)
            .collect::<Vec<_>>();

        match matches.len() {
            0 => Err(Error::FieldNameNotFound {
                class: self.name().to_owned(),
                kind: field_kind_name(kind),
                name: name.to_owned(),
            }),
            1 => Ok(matches.into_iter().next().expect("one field match")),
            matches => Err(Error::FieldNameNotFound {
                class: self.name().to_owned(),
                kind: field_kind_name(kind),
                name: format!("{name} ({matches} matches)"),
            }),
        }
    }

    fn declared_methods_cached(&self) -> Result<Vec<JavaMethodMetadata>> {
        let mut methods = self.methods.borrow_mut();
        if methods.is_none() {
            *methods = Some(self.class.declared_methods()?);
        }
        Ok(methods.as_ref().expect("method cache initialized").clone())
    }

    fn instance_methods_cached(&self) -> Result<Vec<JavaMethodMetadata>> {
        let mut methods = self.instance_methods.borrow_mut();
        if methods.is_none() {
            *methods = Some(metadata::inherited_instance_methods(
                &self.class.vm().java(),
                &self.class,
            )?);
        }
        Ok(methods
            .as_ref()
            .expect("instance method cache initialized")
            .clone())
    }

    fn declared_fields_cached(&self) -> Result<Vec<JavaFieldMetadata>> {
        let mut fields = self.fields.borrow_mut();
        if fields.is_none() {
            *fields = Some(self.class.declared_fields()?);
        }
        Ok(fields.as_ref().expect("field cache initialized").clone())
    }

    fn instance_fields_cached(&self) -> Result<Vec<JavaFieldMetadata>> {
        let mut fields = self.instance_fields.borrow_mut();
        if fields.is_none() {
            *fields = Some(metadata::inherited_instance_fields(
                &self.class.vm().java(),
                &self.class,
            )?);
        }
        Ok(fields
            .as_ref()
            .expect("instance field cache initialized")
            .clone())
    }
}

pub trait JavaMethodSelector {
    type Output;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output>;
}

impl JavaMethodSelector for &str {
    type Output = JavaMethod;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output> {
        class.resolve_named_method(kind, self)
    }
}

impl JavaMethodSelector for &String {
    type Output = JavaMethod;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output> {
        class.resolve_named_method(kind, self)
    }
}

impl<const N: usize> JavaMethodSelector for (&str, [&str; N]) {
    type Output = JavaMethod;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output> {
        let (name, arguments) = self;
        let arguments = parse_type_names(&arguments)?;
        let metadata = class.resolve_method_overload(kind, name, &arguments)?;
        Ok(JavaMethod {
            class: class.class.clone(),
            metadata,
        })
    }
}

impl<const N: usize> JavaMethodSelector for (&String, [&str; N]) {
    type Output = JavaMethod;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output> {
        let (name, arguments) = self;
        (name.as_str(), arguments).resolve(class, kind)
    }
}

impl JavaMethodSelector for (&str, usize) {
    type Output = JavaMethod;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output> {
        let (name, arity) = self;
        let methods = match kind {
            MethodKind::Instance => class.instance_methods_cached()?,
            MethodKind::Constructor | MethodKind::Static => class.declared_methods_cached()?,
        };
        Ok(JavaMethod {
            class: class.class.clone(),
            metadata: select_method_by_arity(class.name(), kind, name, arity, methods)?,
        })
    }
}

impl JavaMethodSelector for (&String, usize) {
    type Output = JavaMethod;

    fn resolve(self, class: &JavaClass, kind: MethodKind) -> Result<Self::Output> {
        let (name, arity) = self;
        (name.as_str(), arity).resolve(class, kind)
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
    /// The callback receives [`JavaHookContext`](crate::replacement::JavaHookContext)
    /// with `kind()` set to [`MethodKind::Constructor`], `name()`
    /// set to `"<init>"`, and `this_object()` pointing at the object being initialized. The
    /// callback may call the original constructor through `call_original*()` helpers; original constructor
    /// calls return void. Keep the returned guard alive while the replacement should remain active;
    /// reverting or dropping it restores the original constructor.
    ///
    /// # Safety
    ///
    /// This is backed by the hidden ART method-replacement prototype. Constructor callbacks must
    /// initialize the receiver consistently enough for Java code that observes the object, and must
    /// return `()` or [`JavaHookReturn::Void`](crate::replacement::JavaHookReturn::Void).
    pub unsafe fn replace<F, R>(&self, callback: F) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        unsafe { crate::replacement::install_constructor_hook(self, callback) }
    }

    pub fn new_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        let args =
            PreparedJavaArgs::new(self.class.vm(), self.metadata.signature.arguments(), args)?;
        self.class
            .new_object(&self.metadata.signature.to_string(), args.values())
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
                id: method.raw(),
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
    /// # Safety
    ///
    /// This is backed by the hidden ART method-replacement prototype. Object and array values
    /// returned by the closure must remain valid until the callback returns.
    pub unsafe fn replace<F, R>(&self, callback: F) -> Result<crate::replacement::JavaHookGuard>
    where
        F: for<'a> Fn(crate::replacement::JavaHookContext<'a>) -> Result<R> + Send + Sync + 'static,
        R: crate::replacement::IntoJavaHookReturn,
    {
        unsafe { crate::replacement::install_method_hook(self, callback) }
    }

    pub fn call_raw<A: IntoJavaCallArgs>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: A,
    ) -> Result<JavaReturn> {
        if self.metadata.kind != MethodKind::Instance {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethod::call_raw",
            });
        }
        let args =
            PreparedJavaArgs::new(self.class.vm(), self.metadata.signature.arguments(), args)?;
        self.class.call_method(
            object,
            &self.metadata.name,
            &self.metadata.signature.to_string(),
            args.values(),
        )
    }

    pub fn call<T: FromJavaReturn>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        T::from_java_return(self.call_raw(object, args)?, "JavaMethod::call")
    }

    pub fn call_static_raw<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaReturn> {
        if self.metadata.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethod::call_static_raw",
            });
        }
        let args =
            PreparedJavaArgs::new(self.class.vm(), self.metadata.signature.arguments(), args)?;
        self.class.call_static(
            &self.metadata.name,
            &self.metadata.signature.to_string(),
            args.values(),
        )
    }

    pub fn call_static<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        T::from_java_return(self.call_static_raw(args)?, "JavaMethod::call_static")
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
        self.call_raw(object, args)?
            .into_object("JavaMethod::call_object")
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

    pub fn call_static_void<A: IntoJavaCallArgs>(&self, args: A) -> Result<()> {
        self.call_static_raw(args)?
            .into_void("JavaMethod::call_static_void")
    }

    pub fn call_static_boolean<A: IntoJavaCallArgs>(&self, args: A) -> Result<bool> {
        self.call_static_raw(args)?
            .into_boolean("JavaMethod::call_static_boolean")
    }

    pub fn call_static_int<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jint> {
        self.call_static_raw(args)?
            .into_int("JavaMethod::call_static_int")
    }

    pub fn call_static_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<JavaObject>> {
        self.call_static_raw(args)?
            .into_object("JavaMethod::call_static_object")
    }

    pub fn call_static_array<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<JavaArray>> {
        self.call_static_raw(args)?
            .into_array("JavaMethod::call_static_array")
    }

    pub fn call_static_string<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<String>> {
        self.call_static_object(args)?
            .map(|object| object.get_string())
            .transpose()
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

    pub fn get_raw(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaReturn> {
        if self.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::get_raw",
            });
        }
        self.class
            .get_field(object, &self.metadata.name, &self.metadata.ty.to_string())
    }

    pub fn get<T: FromJavaReturn>(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<T> {
        T::from_java_return(self.get_raw(object)?, "JavaField::get")
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
        self.get_raw(object)?.into_object("JavaField::get_object")
    }

    pub fn get_array(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<Option<JavaArray>> {
        self.get_raw(object)?.into_array("JavaField::get_array")
    }

    pub fn set<V: IntoJavaFieldValue>(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        value: V,
    ) -> Result<()> {
        if self.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::set",
            });
        }
        let env = self.class.vm().attach_current_thread()?;
        let value = value.into_java_field_value(&env, &self.metadata.ty, "JavaField::set")?;
        let result = self.class.set_field(
            object,
            &self.metadata.name,
            &self.metadata.ty.to_string(),
            value.value(),
        );
        value.delete_local_ref(&env);
        result
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

    pub fn get_static_raw(&self) -> Result<JavaReturn> {
        if self.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::get_static_raw",
            });
        }
        self.class
            .get_static_field(&self.metadata.name, &self.metadata.ty.to_string())
    }

    pub fn get_static<T: FromJavaReturn>(&self) -> Result<T> {
        T::from_java_return(self.get_static_raw()?, "JavaField::get_static")
    }

    pub fn get_static_int(&self) -> Result<jni::jint> {
        self.get_static_raw()?.into_int("JavaField::get_static_int")
    }

    pub fn get_static_boolean(&self) -> Result<bool> {
        self.get_static_raw()?
            .into_boolean("JavaField::get_static_boolean")
    }

    pub fn get_static_byte(&self) -> Result<jni::jbyte> {
        self.get_static_raw()?
            .into_byte("JavaField::get_static_byte")
    }

    pub fn get_static_char(&self) -> Result<jni::jchar> {
        self.get_static_raw()?
            .into_char("JavaField::get_static_char")
    }

    pub fn get_static_short(&self) -> Result<jni::jshort> {
        self.get_static_raw()?
            .into_short("JavaField::get_static_short")
    }

    pub fn get_static_long(&self) -> Result<jni::jlong> {
        self.get_static_raw()?
            .into_long("JavaField::get_static_long")
    }

    pub fn get_static_float(&self) -> Result<jni::jfloat> {
        self.get_static_raw()?
            .into_float("JavaField::get_static_float")
    }

    pub fn get_static_double(&self) -> Result<jni::jdouble> {
        self.get_static_raw()?
            .into_double("JavaField::get_static_double")
    }

    pub fn get_static_object(&self) -> Result<Option<JavaObject>> {
        self.get_static_raw()?
            .into_object("JavaField::get_static_object")
    }

    pub fn get_static_array(&self) -> Result<Option<JavaArray>> {
        self.get_static_raw()?
            .into_array("JavaField::get_static_array")
    }

    pub fn set_static<V: IntoJavaFieldValue>(&self, value: V) -> Result<()> {
        if self.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaField::set_static",
            });
        }
        let env = self.class.vm().attach_current_thread()?;
        let value =
            value.into_java_field_value(&env, &self.metadata.ty, "JavaField::set_static")?;
        let result = self.class.set_static_field(
            &self.metadata.name,
            &self.metadata.ty.to_string(),
            value.value(),
        );
        value.delete_local_ref(&env);
        result
    }

    pub fn set_static_int(&self, value: jni::jint) -> Result<()> {
        self.set_static(JavaValue::Int(value))
    }

    pub fn set_static_boolean(&self, value: bool) -> Result<()> {
        self.set_static(JavaValue::Boolean(value))
    }

    pub fn set_static_byte(&self, value: jni::jbyte) -> Result<()> {
        self.set_static(JavaValue::Byte(value))
    }

    pub fn set_static_char(&self, value: jni::jchar) -> Result<()> {
        self.set_static(JavaValue::Char(value))
    }

    pub fn set_static_short(&self, value: jni::jshort) -> Result<()> {
        self.set_static(JavaValue::Short(value))
    }

    pub fn set_static_long(&self, value: jni::jlong) -> Result<()> {
        self.set_static(JavaValue::Long(value))
    }

    pub fn set_static_float(&self, value: jni::jfloat) -> Result<()> {
        self.set_static(JavaValue::Float(value))
    }

    pub fn set_static_double(&self, value: jni::jdouble) -> Result<()> {
        self.set_static(JavaValue::Double(value))
    }

    pub fn set_static_object<T: JavaObjectRef + ?Sized>(&self, value: Option<&T>) -> Result<()> {
        self.set_static(value.map_or(JavaValue::Null, |value| {
            JavaValue::object_ref(value.as_jobject())
        }))
    }
}

pub trait JavaBoundMethodSelector<'object> {
    type Output;

    fn resolve_bound(
        self,
        bound: &JavaBoundObject<'object>,
        kind: MethodKind,
    ) -> Result<Self::Output>;
}

impl<'object, S> JavaBoundMethodSelector<'object> for S
where
    S: JavaMethodSelector,
    S::Output: IntoBoundMethod<'object>,
{
    type Output = <S::Output as IntoBoundMethod<'object>>::Bound;

    fn resolve_bound(
        self,
        bound: &JavaBoundObject<'object>,
        kind: MethodKind,
    ) -> Result<Self::Output> {
        self.resolve(&bound.class, kind)?
            .into_bound_method(bound.object)
    }
}

pub trait IntoBoundMethod<'object> {
    type Bound;

    fn into_bound_method(self, object: &'object dyn JavaObjectRef) -> Result<Self::Bound>;
}

impl<'object> IntoBoundMethod<'object> for JavaMethod {
    type Bound = JavaBoundMethodOverload<'object>;

    fn into_bound_method(self, object: &'object dyn JavaObjectRef) -> Result<Self::Bound> {
        Ok(JavaBoundMethodOverload {
            object,
            overload: self,
        })
    }
}

impl<'object> JavaBoundObject<'object> {
    pub fn class(&self) -> &JavaClass {
        &self.class
    }

    pub fn object(&self) -> &'object dyn JavaObjectRef {
        self.object
    }

    pub fn method<S: JavaBoundMethodSelector<'object>>(&self, selector: S) -> Result<S::Output> {
        selector.resolve_bound(self, MethodKind::Instance)
    }

    pub fn field(&self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        Ok(JavaBoundFieldHandle {
            object: self.object,
            field: self.class.field_handle(name)?,
        })
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
        T::from_java_return(self.call_raw(args)?, "JavaBoundMethodOverload::call")
    }
}

impl JavaBoundFieldHandle<'_> {
    pub fn field(&self) -> &JavaField {
        &self.field
    }

    pub fn get_raw(&self) -> Result<JavaReturn> {
        self.field.get_raw(self.object)
    }

    pub fn get<T: FromJavaReturn>(&self) -> Result<T> {
        T::from_java_return(self.get_raw()?, "JavaBoundFieldHandle::get")
    }

    pub fn set<V: IntoJavaFieldValue>(&self, value: V) -> Result<()> {
        self.field.set(self.object, value)
    }
}

fn method_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Constructor => "constructor",
        MethodKind::Instance => "instance",
        MethodKind::Static => "static",
    }
}

fn field_kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Instance => "instance",
        FieldKind::Static => "static",
    }
}

fn wrapper_method_name(kind: MethodKind, name: &str) -> &str {
    if kind == MethodKind::Constructor {
        "$init"
    } else {
        name
    }
}

fn select_method_by_name(
    class: &str,
    kind: MethodKind,
    name: &str,
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| method.kind == kind && method.name == name)
        .collect::<Vec<_>>();

    match matches.len() {
        0 => Err(Error::MethodNameNotFound {
            class: class.to_owned(),
            kind: method_kind_name(kind),
            name: name.to_owned(),
        }),
        1 => Ok(matches.into_iter().next().expect("one method match")),
        _ => Err(Error::AmbiguousMethod {
            class: class.to_owned(),
            kind: method_kind_name(kind),
            name: name.to_owned(),
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

fn select_method_by_arity(
    class: &str,
    kind: MethodKind,
    name: &str,
    arity: usize,
    methods: Vec<JavaMethodMetadata>,
) -> Result<JavaMethodMetadata> {
    let matches = methods
        .into_iter()
        .filter(|method| {
            method.kind == kind
                && method.name == name
                && method.signature.arguments().len() == arity
        })
        .collect::<Vec<_>>();

    select_method_overload_match(class, kind, name, format!("({arity} args)"), matches)
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

    #[test]
    fn resolves_string_selector_for_unambiguous_method() {
        let selected = select_method_by_name(
            CLASS,
            MethodKind::Instance,
            "onResume",
            vec![method("onResume", MethodKind::Instance, "()V")],
        )
        .unwrap();

        assert_eq!(selected.name, "onResume");
        assert_eq!(selected.signature.to_string(), "()V");
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
    fn resolves_arity_selector_for_overload() {
        let selected = select_method_by_arity(
            CLASS,
            MethodKind::Static,
            "make",
            2,
            vec![
                method("make", MethodKind::Static, "(I)I"),
                method("make", MethodKind::Static, "(II)I"),
            ],
        )
        .unwrap();

        assert_eq!(selected.signature.to_string(), "(II)I");
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
        let error = select_method_by_name(
            CLASS,
            MethodKind::Instance,
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
                kind: "instance",
                name,
                candidates,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "overload");
                assert_eq!(candidates, vec!["()I".to_owned(), "(I)I".to_owned()]);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reports_ambiguous_arity_overload() {
        let error = select_method_by_arity(
            CLASS,
            MethodKind::Instance,
            "overload",
            1,
            vec![
                method("overload", MethodKind::Instance, "(I)I"),
                method("overload", MethodKind::Instance, "(Ljava/lang/String;)I"),
            ],
        )
        .unwrap_err();

        match error {
            Error::AmbiguousOverload {
                class,
                kind: "instance",
                name,
                arguments,
                matches,
            } => {
                assert_eq!(class, CLASS);
                assert_eq!(name, "overload");
                assert_eq!(arguments, "(1 args)");
                assert_eq!(matches, 2);
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
