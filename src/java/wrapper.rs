use super::*;

impl JavaClassWrapper {
    pub(super) fn new(class: JavaClass) -> Self {
        Self {
            class,
            methods: Rc::new(RefCell::new(None)),
            fields: Rc::new(RefCell::new(None)),
        }
    }

    pub fn name(&self) -> &str {
        self.class.name()
    }

    pub fn class(&self) -> &JavaClass {
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

    pub fn method(&self, name: &str) -> Result<JavaMethodHandle> {
        self.method_handle(MethodKind::Instance, name)
    }

    pub fn static_method(&self, name: &str) -> Result<JavaMethodHandle> {
        self.method_handle(MethodKind::Static, name)
    }

    pub fn fields(&self, name: &str) -> Result<Vec<JavaFieldMetadata>> {
        Ok(self
            .declared_fields_cached()?
            .into_iter()
            .filter(|field| field.name == name)
            .collect())
    }

    pub fn constructor_overload(&self, arguments: &[JavaType]) -> Result<JavaConstructorOverload> {
        let metadata =
            self.resolve_method_overload(MethodKind::Constructor, "<init>", arguments)?;
        Ok(JavaConstructorOverload {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn constructor_overload_by_name(
        &self,
        arguments: &[&str],
    ) -> Result<JavaConstructorOverload> {
        let arguments = parse_type_names(arguments)?;
        self.constructor_overload(&arguments)
    }

    pub fn constructor<const N: usize>(
        &self,
        arguments: [&str; N],
    ) -> Result<JavaConstructorOverload> {
        self.constructor_overload_by_name(&arguments)
    }

    pub fn new_instance<const N: usize, A: IntoJavaCallArgs>(
        &self,
        arguments: [&str; N],
        args: A,
    ) -> Result<JavaObject> {
        self.constructor(arguments)?.new_object(args)
    }

    pub fn method_overload(
        &self,
        name: &str,
        arguments: &[JavaType],
    ) -> Result<JavaMethodOverload> {
        let metadata = self.resolve_method_overload(MethodKind::Instance, name, arguments)?;
        Ok(JavaMethodOverload {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn method_overload_by_name(
        &self,
        name: &str,
        arguments: &[&str],
    ) -> Result<JavaMethodOverload> {
        let arguments = parse_type_names(arguments)?;
        self.method_overload(name, &arguments)
    }

    pub fn overload<const N: usize>(
        &self,
        name: &str,
        arguments: [&str; N],
    ) -> Result<JavaMethodOverload> {
        self.method_overload_by_name(name, &arguments)
    }

    pub fn static_method_overload(
        &self,
        name: &str,
        arguments: &[JavaType],
    ) -> Result<JavaMethodOverload> {
        let metadata = self.resolve_method_overload(MethodKind::Static, name, arguments)?;
        Ok(JavaMethodOverload {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn static_method_overload_by_name(
        &self,
        name: &str,
        arguments: &[&str],
    ) -> Result<JavaMethodOverload> {
        let arguments = parse_type_names(arguments)?;
        self.static_method_overload(name, &arguments)
    }

    pub fn static_overload<const N: usize>(
        &self,
        name: &str,
        arguments: [&str; N],
    ) -> Result<JavaMethodOverload> {
        self.static_method_overload_by_name(name, &arguments)
    }

    pub fn field_handle(&self, name: &str) -> Result<JavaFieldHandle> {
        let metadata = self.resolve_field(FieldKind::Instance, name)?;
        Ok(JavaFieldHandle {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn static_field_handle(&self, name: &str) -> Result<JavaFieldHandle> {
        let metadata = self.resolve_field(FieldKind::Static, name)?;
        Ok(JavaFieldHandle {
            class: self.class.clone(),
            metadata,
        })
    }

    pub fn new_object<A: IntoJavaCallArgs>(&self, signature: &str, args: A) -> Result<JavaObject> {
        self.ensure_method(MethodKind::Constructor, "<init>", signature)?;
        let signature = MethodSignature::parse(signature)?;
        let args = PreparedJavaArgs::new(self.class.vm(), signature.arguments(), args)?;
        self.class.new_object(&signature.to_string(), args.values())
    }

    pub fn call<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
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

    pub fn call_static<A: IntoJavaCallArgs>(
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

    pub fn get_field(
        &self,
        object: &(impl AsJObject + ?Sized),
        name: &str,
        ty: &str,
    ) -> Result<JavaReturn> {
        self.ensure_field(FieldKind::Instance, name, ty)?;
        self.class.get_field(object, name, ty)
    }

    pub fn set_field(
        &self,
        object: &(impl AsJObject + ?Sized),
        name: &str,
        ty: &str,
        value: JavaValue,
    ) -> Result<()> {
        self.ensure_field(FieldKind::Instance, name, ty)?;
        self.class.set_field(object, name, ty, value)
    }

    pub fn get_static_field(&self, name: &str, ty: &str) -> Result<JavaReturn> {
        self.ensure_field(FieldKind::Static, name, ty)?;
        self.class.get_static_field(name, ty)
    }

    pub fn set_static_field(&self, name: &str, ty: &str, value: JavaValue) -> Result<()> {
        self.ensure_field(FieldKind::Static, name, ty)?;
        self.class.set_static_field(name, ty, value)
    }

    pub fn is_instance(&self, object: &(impl AsJObject + ?Sized)) -> Result<bool> {
        self.class.is_instance(object)
    }

    pub fn cast(&self, object: &(impl AsJObject + ?Sized)) -> Result<JavaObject> {
        if self.is_instance(object)? {
            let env = self.class.inner.vm.attach_current_thread()?;
            object_from_ref(&env, &self.class.inner.vm, object)
        } else {
            let env = self.class.inner.vm.attach_current_thread()?;
            let actual = env.get_object_class(object)?;
            Err(Error::InvalidObjectType {
                operation: "JavaClassWrapper::cast",
                expected: "JavaClassWrapper target class",
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
        let matches = self
            .declared_methods_cached()?
            .into_iter()
            .filter(|method| {
                method.kind == kind
                    && method.name == name
                    && method.signature.arguments() == arguments
            })
            .collect::<Vec<_>>();

        match matches.len() {
            0 => Err(Error::OverloadNotFound {
                class: self.name().to_owned(),
                kind: method_kind_name(kind),
                name: wrapper_method_name(kind, name).to_owned(),
                arguments: format_argument_list(arguments),
            }),
            1 => Ok(matches.into_iter().next().expect("one overload match")),
            matches => Err(Error::AmbiguousOverload {
                class: self.name().to_owned(),
                kind: method_kind_name(kind),
                name: wrapper_method_name(kind, name).to_owned(),
                arguments: format_argument_list(arguments),
                matches,
            }),
        }
    }

    fn method_handle(&self, kind: MethodKind, name: &str) -> Result<JavaMethodHandle> {
        let overloads = self
            .declared_methods_cached()?
            .into_iter()
            .filter(|method| method.kind == kind && method.name == name)
            .collect::<Vec<_>>();

        if overloads.is_empty() {
            Err(Error::MethodNameNotFound {
                class: self.name().to_owned(),
                kind: method_kind_name(kind),
                name: name.to_owned(),
            })
        } else {
            Ok(JavaMethodHandle {
                class: self.class.clone(),
                kind,
                name: name.to_owned(),
                overloads,
            })
        }
    }

    fn resolve_field(&self, kind: FieldKind, name: &str) -> Result<JavaFieldMetadata> {
        let matches = self
            .declared_fields_cached()?
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

    fn declared_fields_cached(&self) -> Result<Vec<JavaFieldMetadata>> {
        let mut fields = self.fields.borrow_mut();
        if fields.is_none() {
            *fields = Some(self.class.declared_fields()?);
        }
        Ok(fields.as_ref().expect("field cache initialized").clone())
    }
}

impl JavaMethodHandle {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn overloads(&self) -> &[JavaMethodMetadata] {
        &self.overloads
    }

    pub fn overload<const N: usize>(&self, arguments: [&str; N]) -> Result<JavaMethodOverload> {
        let arguments = parse_type_names(&arguments)?;
        self.overload_by_type(&arguments)
    }

    pub fn overload_by_type(&self, arguments: &[JavaType]) -> Result<JavaMethodOverload> {
        let matches = self
            .overloads
            .iter()
            .filter(|method| method.signature.arguments() == arguments)
            .cloned()
            .collect::<Vec<_>>();

        match matches.len() {
            0 => Err(Error::OverloadNotFound {
                class: self.class.name().to_owned(),
                kind: method_kind_name(self.kind),
                name: self.name.clone(),
                arguments: format_argument_list(arguments),
            }),
            1 => Ok(JavaMethodOverload {
                class: self.class.clone(),
                metadata: matches.into_iter().next().expect("one overload match"),
            }),
            matches => Err(Error::AmbiguousOverload {
                class: self.class.name().to_owned(),
                kind: method_kind_name(self.kind),
                name: self.name.clone(),
                arguments: format_argument_list(arguments),
                matches,
            }),
        }
    }

    /// Installs a guarded Rust closure implementation for this method's sole overload.
    ///
    /// Use [`overload`](Self::overload) first when this method name has multiple overloads.
    ///
    /// # Safety
    ///
    /// This forwards to [`JavaMethodOverload::install_implementation`] and has the same ART
    /// method-replacement safety requirements.
    pub unsafe fn install_implementation<F, R>(
        &self,
        callback: F,
    ) -> Result<crate::replacement::ImplementationGuard>
    where
        F: for<'a> Fn(crate::replacement::ImplementationInvocation<'a>) -> Result<R>
            + Send
            + Sync
            + 'static,
        R: crate::replacement::IntoImplementationReturn,
    {
        unsafe { self.unique_overload()?.install_implementation(callback) }
    }

    pub fn call<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<JavaReturn> {
        self.unique_overload()?.call(object, args)
    }

    pub fn call_void<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<()> {
        self.call(object, args)?
            .into_void("JavaMethodHandle::call_void")
    }

    pub fn call_boolean<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<bool> {
        self.call(object, args)?
            .into_boolean("JavaMethodHandle::call_boolean")
    }

    pub fn call_byte<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jbyte> {
        self.call(object, args)?
            .into_byte("JavaMethodHandle::call_byte")
    }

    pub fn call_char<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jchar> {
        self.call(object, args)?
            .into_char("JavaMethodHandle::call_char")
    }

    pub fn call_short<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jshort> {
        self.call(object, args)?
            .into_short("JavaMethodHandle::call_short")
    }

    pub fn call_int<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jint> {
        self.call(object, args)?
            .into_int("JavaMethodHandle::call_int")
    }

    pub fn call_long<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jlong> {
        self.call(object, args)?
            .into_long("JavaMethodHandle::call_long")
    }

    pub fn call_float<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jfloat> {
        self.call(object, args)?
            .into_float("JavaMethodHandle::call_float")
    }

    pub fn call_double<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jdouble> {
        self.call(object, args)?
            .into_double("JavaMethodHandle::call_double")
    }

    pub fn call_object<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<Option<JavaObject>> {
        self.call(object, args)?
            .into_object("JavaMethodHandle::call_object")
    }

    pub fn call_array<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<Option<JavaArray>> {
        self.call(object, args)?
            .into_array("JavaMethodHandle::call_array")
    }

    pub fn call_string<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<Option<String>> {
        self.call_object(object, args)?
            .map(|object| object.get_string())
            .transpose()
    }

    pub fn call_static<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaReturn> {
        self.unique_overload()?.call_static(args)
    }

    pub fn call_static_void<A: IntoJavaCallArgs>(&self, args: A) -> Result<()> {
        self.call_static(args)?
            .into_void("JavaMethodHandle::call_static_void")
    }

    pub fn call_static_boolean<A: IntoJavaCallArgs>(&self, args: A) -> Result<bool> {
        self.call_static(args)?
            .into_boolean("JavaMethodHandle::call_static_boolean")
    }

    pub fn call_static_byte<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jbyte> {
        self.call_static(args)?
            .into_byte("JavaMethodHandle::call_static_byte")
    }

    pub fn call_static_char<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jchar> {
        self.call_static(args)?
            .into_char("JavaMethodHandle::call_static_char")
    }

    pub fn call_static_short<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jshort> {
        self.call_static(args)?
            .into_short("JavaMethodHandle::call_static_short")
    }

    pub fn call_static_int<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jint> {
        self.call_static(args)?
            .into_int("JavaMethodHandle::call_static_int")
    }

    pub fn call_static_long<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jlong> {
        self.call_static(args)?
            .into_long("JavaMethodHandle::call_static_long")
    }

    pub fn call_static_float<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jfloat> {
        self.call_static(args)?
            .into_float("JavaMethodHandle::call_static_float")
    }

    pub fn call_static_double<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jdouble> {
        self.call_static(args)?
            .into_double("JavaMethodHandle::call_static_double")
    }

    pub fn call_static_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<JavaObject>> {
        self.call_static(args)?
            .into_object("JavaMethodHandle::call_static_object")
    }

    pub fn call_static_array<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<JavaArray>> {
        self.call_static(args)?
            .into_array("JavaMethodHandle::call_static_array")
    }

    pub fn call_static_string<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<String>> {
        self.call_static_object(args)?
            .map(|object| object.get_string())
            .transpose()
    }

    fn unique_overload(&self) -> Result<JavaMethodOverload> {
        match self.overloads.len() {
            0 => Err(Error::MethodNameNotFound {
                class: self.class.name().to_owned(),
                kind: method_kind_name(self.kind),
                name: self.name.clone(),
            }),
            1 => Ok(JavaMethodOverload {
                class: self.class.clone(),
                metadata: self.overloads.first().expect("one overload match").clone(),
            }),
            _ => Err(Error::AmbiguousMethod {
                class: self.class.name().to_owned(),
                kind: method_kind_name(self.kind),
                name: self.name.clone(),
                candidates: self
                    .overloads
                    .iter()
                    .map(|method| method.signature.to_string())
                    .collect(),
            }),
        }
    }
}

impl JavaConstructorOverload {
    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub(crate) fn class(&self) -> &JavaClass {
        &self.class
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.metadata.signature
    }

    /// Installs a guarded Rust closure implementation for this selected constructor overload.
    ///
    /// The callback receives [`ImplementationInvocation`](crate::replacement::ImplementationInvocation)
    /// with `kind()` set to [`MethodKind::Constructor`], `name()`
    /// set to `"<init>"`, and `receiver()` pointing at the object being initialized. The callback
    /// may call the original constructor through `call_original*()` helpers; original constructor
    /// calls return void. Keep the returned guard alive while the replacement should remain active;
    /// reverting or dropping it restores the original constructor.
    ///
    /// # Safety
    ///
    /// This is backed by the hidden ART method-replacement prototype. Constructor callbacks must
    /// initialize the receiver consistently enough for Java code that observes the object, and must
    /// return `()` or [`ImplementationReturn::Void`](crate::replacement::ImplementationReturn::Void).
    pub unsafe fn install_implementation<F, R>(
        &self,
        callback: F,
    ) -> Result<crate::replacement::ImplementationGuard>
    where
        F: for<'a> Fn(crate::replacement::ImplementationInvocation<'a>) -> Result<R>
            + Send
            + Sync
            + 'static,
        R: crate::replacement::IntoImplementationReturn,
    {
        unsafe { crate::replacement::install_implementation_constructor(self, callback) }
    }

    #[allow(dead_code)]
    pub(crate) unsafe fn replace_closure<F>(
        &self,
        callback: F,
    ) -> Result<crate::replacement::ClosureMethodReplacement>
    where
        F: for<'a> Fn(
                crate::replacement::ReplacementInvocation<'a>,
            ) -> Result<crate::replacement::RawJavaReturn>
            + Send
            + Sync
            + 'static,
    {
        unsafe { crate::replacement::replace_constructor_closure(self, callback) }
    }

    pub fn new_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        let args =
            PreparedJavaArgs::new(self.class.vm(), self.metadata.signature.arguments(), args)?;
        self.class
            .new_object(&self.metadata.signature.to_string(), args.values())
    }
}

impl JavaMethodOverload {
    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub(crate) fn class(&self) -> &JavaClass {
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

    /// Captures this overload's original implementation metadata for internal replacement tests.
    #[allow(dead_code)]
    pub(crate) fn original(&self) -> Result<crate::replacement::OriginalMethod> {
        crate::replacement::OriginalMethod::new(self)
    }

    /// Replaces this selected overload using the internal raw JNI-native test facade.
    #[allow(dead_code)]
    pub(crate) unsafe fn replace(
        &self,
        implementation: crate::replacement::MethodImplementation,
    ) -> Result<crate::replacement::MethodReplacement> {
        unsafe { crate::replacement::replace_method(self, implementation) }
    }

    /// Replaces this selected overload using an internal descriptor-driven JNI-native helper.
    #[allow(dead_code)]
    pub(crate) unsafe fn replace_native(
        &self,
        implementation: crate::replacement::NativeMethodImplementation,
    ) -> Result<crate::replacement::MethodReplacement> {
        unsafe { crate::replacement::replace_native_method(self, implementation) }
    }

    /// Replaces this selected overload with an internal raw closure-backed helper.
    #[allow(dead_code)]
    pub(crate) unsafe fn replace_closure<F>(
        &self,
        callback: F,
    ) -> Result<crate::replacement::ClosureMethodReplacement>
    where
        F: for<'a> Fn(
                crate::replacement::ReplacementInvocation<'a>,
            ) -> Result<crate::replacement::RawJavaReturn>
            + Send
            + Sync
            + 'static,
    {
        unsafe { crate::replacement::replace_closure_method(self, callback) }
    }

    /// Installs a guarded Rust closure implementation for this selected overload.
    ///
    /// The callback receives [`ImplementationInvocation`](crate::replacement::ImplementationInvocation),
    /// can call the original method through that invocation, and must return a value implementing
    /// [`IntoImplementationReturn`](crate::replacement::IntoImplementationReturn). Keep the
    /// returned guard alive while the replacement should remain active; reverting or dropping it
    /// restores the original method.
    ///
    /// # Safety
    ///
    /// This is backed by the hidden ART method-replacement prototype. Object and array values
    /// returned by the closure must remain valid until the callback returns.
    pub unsafe fn install_implementation<F, R>(
        &self,
        callback: F,
    ) -> Result<crate::replacement::ImplementationGuard>
    where
        F: for<'a> Fn(crate::replacement::ImplementationInvocation<'a>) -> Result<R>
            + Send
            + Sync
            + 'static,
        R: crate::replacement::IntoImplementationReturn,
    {
        unsafe { crate::replacement::install_implementation_method(self, callback) }
    }

    pub fn call<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<JavaReturn> {
        if self.metadata.kind != MethodKind::Instance {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethodOverload::call",
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

    pub fn call_static<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaReturn> {
        if self.metadata.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethodOverload::call_static",
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

    pub fn call_void<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<()> {
        self.call(object, args)?
            .into_void("JavaMethodOverload::call_void")
    }

    pub fn call_boolean<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<bool> {
        self.call(object, args)?
            .into_boolean("JavaMethodOverload::call_boolean")
    }

    pub fn call_int<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<jni::jint> {
        self.call(object, args)?
            .into_int("JavaMethodOverload::call_int")
    }

    pub fn call_object<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<Option<JavaObject>> {
        self.call(object, args)?
            .into_object("JavaMethodOverload::call_object")
    }

    pub fn call_array<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<Option<JavaArray>> {
        self.call(object, args)?
            .into_array("JavaMethodOverload::call_array")
    }

    pub fn call_string<A: IntoJavaCallArgs>(
        &self,
        object: &(impl AsJObject + ?Sized),
        args: A,
    ) -> Result<Option<String>> {
        self.call_object(object, args)?
            .map(|object| object.get_string())
            .transpose()
    }

    pub fn call_static_void<A: IntoJavaCallArgs>(&self, args: A) -> Result<()> {
        self.call_static(args)?
            .into_void("JavaMethodOverload::call_static_void")
    }

    pub fn call_static_boolean<A: IntoJavaCallArgs>(&self, args: A) -> Result<bool> {
        self.call_static(args)?
            .into_boolean("JavaMethodOverload::call_static_boolean")
    }

    pub fn call_static_int<A: IntoJavaCallArgs>(&self, args: A) -> Result<jni::jint> {
        self.call_static(args)?
            .into_int("JavaMethodOverload::call_static_int")
    }

    pub fn call_static_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<JavaObject>> {
        self.call_static(args)?
            .into_object("JavaMethodOverload::call_static_object")
    }

    pub fn call_static_array<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<JavaArray>> {
        self.call_static(args)?
            .into_array("JavaMethodOverload::call_static_array")
    }

    pub fn call_static_string<A: IntoJavaCallArgs>(&self, args: A) -> Result<Option<String>> {
        self.call_static_object(args)?
            .map(|object| object.get_string())
            .transpose()
    }
}

impl JavaFieldHandle {
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

    pub fn get(&self, object: &(impl AsJObject + ?Sized)) -> Result<JavaReturn> {
        if self.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaFieldHandle::get",
            });
        }
        self.class
            .get_field(object, &self.metadata.name, &self.metadata.ty.to_string())
    }

    pub fn get_boolean(&self, object: &(impl AsJObject + ?Sized)) -> Result<bool> {
        self.get(object)?
            .into_boolean("JavaFieldHandle::get_boolean")
    }

    pub fn get_byte(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jbyte> {
        self.get(object)?.into_byte("JavaFieldHandle::get_byte")
    }

    pub fn get_char(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jchar> {
        self.get(object)?.into_char("JavaFieldHandle::get_char")
    }

    pub fn get_short(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jshort> {
        self.get(object)?.into_short("JavaFieldHandle::get_short")
    }

    pub fn get_int(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jint> {
        self.get(object)?.into_int("JavaFieldHandle::get_int")
    }

    pub fn get_long(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jlong> {
        self.get(object)?.into_long("JavaFieldHandle::get_long")
    }

    pub fn get_float(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jfloat> {
        self.get(object)?.into_float("JavaFieldHandle::get_float")
    }

    pub fn get_double(&self, object: &(impl AsJObject + ?Sized)) -> Result<jni::jdouble> {
        self.get(object)?.into_double("JavaFieldHandle::get_double")
    }

    pub fn get_object(&self, object: &(impl AsJObject + ?Sized)) -> Result<Option<JavaObject>> {
        self.get(object)?.into_object("JavaFieldHandle::get_object")
    }

    pub fn get_array(&self, object: &(impl AsJObject + ?Sized)) -> Result<Option<JavaArray>> {
        self.get(object)?.into_array("JavaFieldHandle::get_array")
    }

    pub fn set(&self, object: &(impl AsJObject + ?Sized), value: JavaValue) -> Result<()> {
        if self.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaFieldHandle::set",
            });
        }
        self.class.set_field(
            object,
            &self.metadata.name,
            &self.metadata.ty.to_string(),
            value,
        )
    }

    pub fn set_boolean(&self, object: &(impl AsJObject + ?Sized), value: bool) -> Result<()> {
        self.set(object, JavaValue::Boolean(value))
    }

    pub fn set_byte(&self, object: &(impl AsJObject + ?Sized), value: jni::jbyte) -> Result<()> {
        self.set(object, JavaValue::Byte(value))
    }

    pub fn set_char(&self, object: &(impl AsJObject + ?Sized), value: jni::jchar) -> Result<()> {
        self.set(object, JavaValue::Char(value))
    }

    pub fn set_short(&self, object: &(impl AsJObject + ?Sized), value: jni::jshort) -> Result<()> {
        self.set(object, JavaValue::Short(value))
    }

    pub fn set_int(&self, object: &(impl AsJObject + ?Sized), value: jni::jint) -> Result<()> {
        self.set(object, JavaValue::Int(value))
    }

    pub fn set_long(&self, object: &(impl AsJObject + ?Sized), value: jni::jlong) -> Result<()> {
        self.set(object, JavaValue::Long(value))
    }

    pub fn set_float(&self, object: &(impl AsJObject + ?Sized), value: jni::jfloat) -> Result<()> {
        self.set(object, JavaValue::Float(value))
    }

    pub fn set_double(
        &self,
        object: &(impl AsJObject + ?Sized),
        value: jni::jdouble,
    ) -> Result<()> {
        self.set(object, JavaValue::Double(value))
    }

    pub fn set_object<T: AsJObject + ?Sized>(
        &self,
        object: &(impl AsJObject + ?Sized),
        value: Option<&T>,
    ) -> Result<()> {
        self.set(
            object,
            value.map_or(JavaValue::Null, |value| {
                JavaValue::Object(value.as_jobject())
            }),
        )
    }

    pub fn set_array<T: AsJObject + ?Sized>(
        &self,
        object: &(impl AsJObject + ?Sized),
        value: Option<&T>,
    ) -> Result<()> {
        self.set(
            object,
            value.map_or(JavaValue::Null, |value| {
                JavaValue::Object(value.as_jobject())
            }),
        )
    }

    pub fn get_static(&self) -> Result<JavaReturn> {
        if self.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaFieldHandle::get_static",
            });
        }
        self.class
            .get_static_field(&self.metadata.name, &self.metadata.ty.to_string())
    }

    pub fn get_static_int(&self) -> Result<jni::jint> {
        self.get_static()?
            .into_int("JavaFieldHandle::get_static_int")
    }

    pub fn get_static_boolean(&self) -> Result<bool> {
        self.get_static()?
            .into_boolean("JavaFieldHandle::get_static_boolean")
    }

    pub fn get_static_byte(&self) -> Result<jni::jbyte> {
        self.get_static()?
            .into_byte("JavaFieldHandle::get_static_byte")
    }

    pub fn get_static_char(&self) -> Result<jni::jchar> {
        self.get_static()?
            .into_char("JavaFieldHandle::get_static_char")
    }

    pub fn get_static_short(&self) -> Result<jni::jshort> {
        self.get_static()?
            .into_short("JavaFieldHandle::get_static_short")
    }

    pub fn get_static_long(&self) -> Result<jni::jlong> {
        self.get_static()?
            .into_long("JavaFieldHandle::get_static_long")
    }

    pub fn get_static_float(&self) -> Result<jni::jfloat> {
        self.get_static()?
            .into_float("JavaFieldHandle::get_static_float")
    }

    pub fn get_static_double(&self) -> Result<jni::jdouble> {
        self.get_static()?
            .into_double("JavaFieldHandle::get_static_double")
    }

    pub fn get_static_object(&self) -> Result<Option<JavaObject>> {
        self.get_static()?
            .into_object("JavaFieldHandle::get_static_object")
    }

    pub fn get_static_array(&self) -> Result<Option<JavaArray>> {
        self.get_static()?
            .into_array("JavaFieldHandle::get_static_array")
    }

    pub fn set_static(&self, value: JavaValue) -> Result<()> {
        if self.metadata.kind != FieldKind::Static {
            return Err(Error::WrongFieldKind {
                operation: "JavaFieldHandle::set_static",
            });
        }
        self.class
            .set_static_field(&self.metadata.name, &self.metadata.ty.to_string(), value)
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

    pub fn set_static_object<T: AsJObject + ?Sized>(&self, value: Option<&T>) -> Result<()> {
        self.set_static(value.map_or(JavaValue::Null, |value| {
            JavaValue::Object(value.as_jobject())
        }))
    }

    pub fn set_static_array<T: AsJObject + ?Sized>(&self, value: Option<&T>) -> Result<()> {
        self.set_static(value.map_or(JavaValue::Null, |value| {
            JavaValue::Object(value.as_jobject())
        }))
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
