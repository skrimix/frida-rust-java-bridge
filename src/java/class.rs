use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex},
};

use crate::{
    env::{Env, FieldId, FieldKind, MethodId, MethodKind},
    error::{Error, Result},
    jni,
    metadata::{self, JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata},
    refs::{AsJClass, AsJObject, ClassKind, GlobalRef, JavaObjectRef},
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

use super::{
    FromJavaReturn, IntoJavaCallArgs, IntoJavaFieldValue, JavaOverloadArg, JavaReturn,
    dispatch::{
        call_instance_return, call_static_return, get_instance_field, get_static_field,
        set_instance_field, set_static_field,
    },
    members::{
        JavaConstructor, JavaField, JavaMethodGroup, MethodDispatchTarget, parse_type_names,
        select_field_by_name, select_method_by_arguments, select_method_by_dispatch_args,
    },
    object::JavaObject,
    raw,
};

/// Java class wrapper.
///
/// Use `JavaClass` for class-level operations:
/// - Creating new instances (constructors).
/// - Calling static methods.
/// - Reading and writing static fields.
/// - Casting or checking if objects are instances of this class.
/// - Finding live heap instances of this class.
/// - Installing method or constructor replacements.
///
/// ### Overload Selection
///
/// Wrapper calls choose an overload from the arguments you pass. Use the explicit `*_with` methods
/// when you need to select a specific signature.
#[derive(Clone)]
pub struct JavaClass {
    pub(super) class: raw::Class,
    methods: Arc<Mutex<Option<Vec<JavaMethodMetadata>>>>,
    visible_methods: Arc<Mutex<Option<Vec<JavaMethodMetadata>>>>,
    fields: Arc<Mutex<Option<Vec<JavaFieldMetadata>>>>,
    visible_fields: Arc<Mutex<Option<Vec<JavaFieldMetadata>>>>,
}

/// Controls whether heap instance enumeration should keep delivering matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JavaChooseControl {
    Continue,
    Stop,
}

impl fmt::Display for raw::Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Debug for raw::Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Class")
            .field("name", &self.name())
            .field("class", &self.as_jclass())
            .finish()
    }
}

pub(super) struct JavaClassInner {
    pub(super) name: String,
    pub(super) class: GlobalRef<ClassKind>,
    methods: Mutex<HashMap<MethodKey, MethodId>>,
    fields: Mutex<HashMap<FieldKey, FieldId>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct MethodKey {
    kind: MethodKind,
    name: String,
    signature: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct FieldKey {
    kind: FieldKind,
    name: String,
    ty: String,
}

impl raw::Class {
    pub(crate) fn from_global(name: String, class: GlobalRef<ClassKind>) -> Self {
        Self {
            inner: Arc::new(JavaClassInner {
                name,
                class,
                methods: Mutex::new(HashMap::new()),
                fields: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Returns the Java binary name for this class.
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    /// Returns the raw JNI global class reference.
    ///
    /// # Safety
    ///
    /// The caller must not delete the returned reference. It is valid for this process' ART runtime.
    pub unsafe fn raw_jclass(&self) -> jni::jclass {
        unsafe { self.inner.class.raw_jclass() }
    }

    pub(crate) fn vm(&self) -> &Vm {
        self.inner.class.vm()
    }

    pub(crate) fn resolve_static_method(&self, name: &str, signature: &str) -> Result<MethodId> {
        let env = self.vm().attach_current_thread()?;
        self.static_method(&env, name, signature)
    }

    pub(crate) fn resolve_instance_method(&self, name: &str, signature: &str) -> Result<MethodId> {
        let env = self.vm().attach_current_thread()?;
        self.method(&env, name, signature)
    }

    pub(crate) fn resolve_constructor(&self, signature: &str) -> Result<MethodId> {
        let env = self.vm().attach_current_thread()?;
        self.constructor(&env, signature)
    }

    /// Creates a Java object using the constructor with the exact JNI method signature.
    ///
    /// Use [`JavaClass::new_object`] or [`JavaClass::new_object_with`] when working with the
    /// high-level wrapper.
    pub fn new_object(&self, signature: &str, args: &[JavaValue]) -> Result<JavaObject> {
        let env = self.vm().attach_current_thread()?;
        let constructor = self.constructor(&env, signature)?;
        // SAFETY: the constructor ID is resolved from `self.inner.class` immediately above.
        let object = unsafe { env.new_object(&self.inner.class, &constructor, args)? };
        let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
        let reference = unsafe { GlobalRef::from_raw(self.vm().clone(), reference)? };
        Ok(JavaObject::from_global_ref(
            JavaClass::from_raw(self.clone()),
            reference,
        ))
    }

    /// Calls an instance method using an exact JNI method signature.
    ///
    /// `object` must be an instance of this class or a compatible subclass.
    pub fn call_method(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        name: &str,
        signature: &str,
        args: &[JavaValue],
    ) -> Result<JavaReturn> {
        let env = self.vm().attach_current_thread()?;
        let method = self.method(&env, name, signature)?;
        call_instance_return(&env, self, object, &method, args)
    }

    /// Calls a static method using an exact JNI method signature.
    pub fn call_static(
        &self,
        name: &str,
        signature: &str,
        args: &[JavaValue],
    ) -> Result<JavaReturn> {
        let env = self.vm().attach_current_thread()?;
        let method = self.static_method(&env, name, signature)?;
        call_static_return(&env, self, &method, args)
    }

    /// Reads an instance field using an exact JNI field type descriptor.
    ///
    /// `object` must be an instance of this class or a compatible subclass.
    pub fn get_field(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        name: &str,
        ty: &str,
    ) -> Result<JavaReturn> {
        let env = self.vm().attach_current_thread()?;
        let field = self.field(&env, name, ty)?;
        get_instance_field(&env, self, object, &field)
    }

    /// Writes an instance field using an exact JNI field type descriptor.
    ///
    /// `object` must be an instance of this class or a compatible subclass.
    pub fn set_field(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        name: &str,
        ty: &str,
        value: JavaValue,
    ) -> Result<()> {
        let env = self.vm().attach_current_thread()?;
        let field = self.field(&env, name, ty)?;
        set_instance_field(&env, object, &field, value)
    }

    /// Reads a static field using an exact JNI field type descriptor.
    pub fn get_static_field(&self, name: &str, ty: &str) -> Result<JavaReturn> {
        let env = self.vm().attach_current_thread()?;
        let field = self.static_field(&env, name, ty)?;
        get_static_field(&env, self, &field)
    }

    /// Writes a static field using an exact JNI field type descriptor.
    pub fn set_static_field(&self, name: &str, ty: &str, value: JavaValue) -> Result<()> {
        let env = self.vm().attach_current_thread()?;
        let field = self.static_field(&env, name, ty)?;
        set_static_field(&env, &self.inner.class, &field, value)
    }

    /// Returns reflection metadata for this class.
    pub fn metadata(&self) -> Result<JavaClassMetadata> {
        let env = self.vm().attach_current_thread()?;
        metadata::class_metadata(&env, self.vm(), self)
    }

    /// Returns methods declared directly by this class.
    pub fn declared_methods(&self) -> Result<Vec<JavaMethodMetadata>> {
        let env = self.vm().attach_current_thread()?;
        metadata::declared_methods(&env, self)
    }

    /// Returns fields declared directly by this class.
    pub fn declared_fields(&self) -> Result<Vec<JavaFieldMetadata>> {
        let env = self.vm().attach_current_thread()?;
        metadata::declared_fields(&env, self)
    }

    /// Returns whether `object` is an instance of this class.
    pub fn is_instance(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        let env = self.vm().attach_current_thread()?;
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
            .expect("java::raw::Class method cache mutex poisoned")
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
            .expect("java::raw::Class method cache mutex poisoned")
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
            .expect("java::raw::Class field cache mutex poisoned")
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
            .expect("java::raw::Class field cache mutex poisoned")
            .insert(key, field.clone());

        Ok(field)
    }
}

impl crate::refs::sealed::JavaObjectRefSealed for raw::Class {
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.inner.class.raw_jobject() }
    }
}

impl crate::refs::JavaObjectRef for raw::Class {}

impl crate::refs::sealed::JavaClassRefSealed for raw::Class {
    fn as_jclass(&self) -> jni::jclass {
        unsafe { self.raw_jclass() }
    }
}

impl crate::refs::JavaClassRef for raw::Class {}

impl fmt::Display for JavaClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.class, f)
    }
}

impl fmt::Debug for JavaClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaClass")
            .field("class", &self.class)
            .finish()
    }
}

impl JavaClass {
    /// Returns a JavaScript-style display string for this class.
    pub fn java_display(&self) -> String {
        format!("<class: {}>", self.name())
    }
}

impl JavaClass {
    pub(crate) fn from_raw(class: raw::Class) -> Self {
        Self {
            class,
            methods: Arc::new(Mutex::new(None)),
            visible_methods: Arc::new(Mutex::new(None)),
            fields: Arc::new(Mutex::new(None)),
            visible_fields: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns the Java binary name for this class.
    pub fn name(&self) -> &str {
        self.class.name()
    }

    /// Returns the lower-level raw class handle.
    ///
    /// Use this when code needs exact-signature JNI-style calls or raw class access.
    pub fn class(&self) -> &raw::Class {
        &self.class
    }

    /// Returns methods declared directly by this class.
    pub fn declared_methods(&self) -> Result<Vec<JavaMethodMetadata>> {
        self.declared_methods_cached()
    }

    /// Returns fields declared directly by this class.
    pub fn declared_fields(&self) -> Result<Vec<JavaFieldMetadata>> {
        self.declared_fields_cached()
    }

    /// Returns constructors declared by this class.
    pub fn constructors(&self) -> Result<Vec<JavaMethodMetadata>> {
        Ok(self
            .declared_methods_cached()?
            .into_iter()
            .filter(|method| method.kind == MethodKind::Constructor)
            .collect())
    }

    /// Returns the visible method overloads with the given name.
    ///
    /// Use [`JavaMethodGroup::overload`] or [`JavaMethodGroup::call`] to select an overload.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn call_selected(java: &Java) -> Result<i32> {
    ///     let integer = java.use_class("java.lang.Integer")?;
    ///     let parse_int = integer.method("parseInt")?.overload(["java.lang.String"])?;
    ///     parse_int.call((), "42")
    /// }
    /// ```
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

    /// Calls a static method, selecting an overload from the provided arguments.
    ///
    /// Pass `()` for no arguments. Use `()` as the return type for Java `void` methods.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn run_gc(java: &Java) -> Result<()> {
    ///     let system = java.use_class("java.lang.System")?;
    ///     system.call("gc", ())
    /// }
    /// ```
    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.method(name)?.call(args)
    }

    /// Calls a static method using the overload with the given argument type names.
    ///
    /// Pass `()` for no arguments. Use `()` as the return type for Java `void` methods.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn parse_with_radix(java: &Java) -> Result<i32> {
    ///     let integer = java.use_class("java.lang.Integer")?;
    ///     integer.call_with("parseInt", ["java.lang.String", "int"], ("ff", 16))
    /// }
    /// ```
    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.method(name)?.overload(arguments)?.call((), args)
    }

    /// Returns the constructor overload with the given parsed argument types.
    pub fn constructor_by_types(&self, arguments: &[JavaType]) -> Result<JavaConstructor> {
        let metadata =
            self.resolve_method_overload(MethodKind::Constructor, "<init>", arguments)?;
        Ok(JavaConstructor {
            class: self.class.clone(),
            metadata,
        })
    }

    /// Returns the constructor overload with the given argument type names.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn new_builder(java: &Java) -> Result<()> {
    ///     let builder = java.use_class("java.lang.StringBuilder")?;
    ///     let constructor = builder.constructor(["java.lang.String"])?;
    ///     let object = constructor.new_object("prefix")?;
    ///     let _ = object;
    ///     Ok(())
    /// }
    /// ```
    pub fn constructor<'types>(
        &self,
        arguments: impl AsRef<[&'types str]>,
    ) -> Result<JavaConstructor> {
        let arguments = parse_type_names(arguments.as_ref())?;
        self.constructor_by_types(&arguments)
    }

    /// Creates an object through the constructor overload with the given argument types.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn make_proxy_info(java: &Java) -> Result<()> {
    ///     let proxy_info = java.use_class("android.net.ProxyInfo")?;
    ///     let proxy = proxy_info.new_object_with(
    ///         ["java.lang.String", "int", "java.lang.String"],
    ///         ("192.168.1.10", 8080, ""),
    ///     )?;
    ///     let _ = proxy;
    ///     Ok(())
    /// }
    /// ```
    pub fn new_object_with<'types, A: IntoJavaCallArgs>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: A,
    ) -> Result<JavaObject> {
        self.constructor(arguments)?.new_object(args)
    }

    /// Creates an object by dispatching to the best compatible constructor overload.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn make_builder(java: &Java) -> Result<String> {
    ///     let builder = java.use_class("java.lang.StringBuilder")?;
    ///     let object = builder.new_object("hello")?;
    ///     object.call("toString", ())
    /// }
    /// ```
    pub fn new_object<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaObject> {
        let args = args.into_java_overload_args();
        let constructor = self.resolve_constructor_for_dispatch(&args)?;
        constructor.new_object(args)
    }

    /// Returns the visible field with the given name.
    ///
    /// If more than one inherited field has this name, this returns an ambiguity error.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn sdk_field(java: &Java) -> Result<i32> {
    ///     let version = java.use_class("android.os.Build$VERSION")?;
    ///     version.field("SDK_INT")?.get(())
    /// }
    /// ```
    pub fn field(&self, name: &str) -> Result<JavaField> {
        let metadata = select_field_by_name(self.name(), name, self.field_matches_by_name(name)?)?;
        Ok(JavaField {
            class: self.class.clone(),
            metadata,
        })
    }

    /// Reads a static field selected by name.
    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.field(name)?.get(())
    }

    /// Writes a static field selected by name.
    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.field(name)?.set((), value)
    }

    /// Returns whether `object` is an instance of this class.
    pub fn is_instance(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        self.class.is_instance(object)
    }

    /// Returns `object` as an owned [`JavaObject`] of this class.
    ///
    /// Returns [`Error::InvalidObjectType`] if the object is not an instance of this class.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaObject, Result};
    ///
    /// fn system_service(java: &Java, context: &JavaObject) -> Result<JavaObject> {
    ///     let connectivity_manager = java.use_class("android.net.ConnectivityManager")?;
    ///     let service: JavaObject = context.call("getSystemService", "connectivity")?;
    ///     connectivity_manager.cast(&service)
    /// }
    /// ```
    pub fn cast(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaObject> {
        if self.is_instance(object)? {
            let env = self.class.vm().attach_current_thread()?;
            let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
            let reference = unsafe { GlobalRef::from_raw(self.class.vm().clone(), reference)? };
            Ok(JavaObject::from_global_ref(self.clone(), reference))
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

    /// Enumerates live heap instances of this class.
    ///
    /// Return [`JavaChooseControl::Stop`] from the callback to stop early.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaChooseControl, Result};
    ///
    /// fn print_first_string(java: &Java) -> Result<()> {
    ///     let string = java.use_class("java.lang.String")?;
    ///     string.choose_instances(|object| {
    ///         println!("{}", object.get_string()?);
    ///         Ok(JavaChooseControl::Stop)
    ///     })
    /// }
    /// ```
    pub fn choose_instances<F>(&self, mut callback: F) -> Result<()>
    where
        F: FnMut(&JavaObject) -> Result<JavaChooseControl>,
    {
        let vm = self.class.vm();
        let env = vm.attach_current_thread()?;
        let handles = vm
            .art()
            .enumerate_heap_instance_handles(vm, self.class.as_jobject())?;
        super::handle::deliver_heap_instance_handles(&env, self.clone(), handles, &mut callback)
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
        args: &[JavaOverloadArg],
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

        let env = self.class.vm().attach_current_thread()?;
        let loaded = metadata::visible_methods(&env, &self.class)?;
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

        let env = self.class.vm().attach_current_thread()?;
        let loaded = metadata::visible_fields(&env, &self.class)?;
        let mut fields = self
            .visible_fields
            .lock()
            .expect("JavaClass visible field cache mutex poisoned");
        Ok(fields.get_or_insert_with(|| loaded).clone())
    }
}
