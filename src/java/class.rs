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
    object::{JavaBoundObject, JavaObject},
    raw,
};

/// A high-level, reflection-backed wrapper for a Java class.
///
/// A `JavaClass` lets you perform class-level operations in Rust, such as:
/// - Creating new instances (constructors).
/// - Calling static methods.
/// - Reading and writing static fields.
/// - Casting or checking if objects are instances of this class.
/// - Finding existing instances of this class currently alive on the heap.
/// - Installing method or constructor replacements (hooks).
///
/// ### Overload Selection
///
/// When calling methods or constructors, this wrapper will automatically resolve and choose the best
/// overload based on the arguments you pass. If you need to target a highly specific overload or resolve
/// ambiguity, you can use the explicit `*_with` methods to select a signature manually.
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
    pub(super) vm: Vm,
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

    /// Returns the raw JNI global class reference.
    ///
    /// # Safety
    ///
    /// The caller must not delete the returned reference or use it with a different VM.
    pub unsafe fn raw_jclass(&self) -> jni::jclass {
        unsafe { self.inner.class.raw_jclass() }
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

    pub(crate) fn resolve_constructor(&self, signature: &str) -> Result<MethodId> {
        let env = self.inner.vm.attach_current_thread()?;
        self.constructor(&env, signature)
    }

    pub fn new_object(&self, signature: &str, args: &[JavaValue]) -> Result<JavaObject> {
        let env = self.inner.vm.attach_current_thread()?;
        let constructor = self.constructor(&env, signature)?;
        // SAFETY: the constructor ID is resolved from `self.inner.class` immediately above.
        let object = unsafe { env.new_object(&self.inner.class, &constructor, args)? };
        let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
        let reference = unsafe { GlobalRef::from_raw(self.inner.vm.clone(), reference)? };
        Ok(JavaObject::from_global_ref(
            JavaClass::from_raw(self.clone()),
            reference,
        ))
    }

    pub fn call_method(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        name: &str,
        signature: &str,
        args: &[JavaValue],
    ) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let method = self.method(&env, name, signature)?;
        call_instance_return(&env, self, object, &method, args)
    }

    pub fn call_static(
        &self,
        name: &str,
        signature: &str,
        args: &[JavaValue],
    ) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let method = self.static_method(&env, name, signature)?;
        call_static_return(&env, self, &method, args)
    }

    pub fn get_field(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
        name: &str,
        ty: &str,
    ) -> Result<JavaReturn> {
        let env = self.inner.vm.attach_current_thread()?;
        let field = self.field(&env, name, ty)?;
        get_instance_field(&env, self, object, &field)
    }

    pub fn set_field(
        &self,
        object: &(impl JavaObjectRef + ?Sized),
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
        get_static_field(&env, self, &field)
    }

    pub fn set_static_field(&self, name: &str, ty: &str, value: JavaValue) -> Result<()> {
        let env = self.inner.vm.attach_current_thread()?;
        let field = self.static_field(&env, name, ty)?;
        set_static_field(&env, &self.inner.class, &field, value)
    }

    pub fn metadata(&self) -> Result<JavaClassMetadata> {
        let env = self.inner.vm.attach_current_thread()?;
        metadata::class_metadata(&env, &self.inner.vm, self)
    }

    pub fn declared_methods(&self) -> Result<Vec<JavaMethodMetadata>> {
        let env = self.inner.vm.attach_current_thread()?;
        metadata::declared_methods(&env, self)
    }

    pub fn declared_fields(&self) -> Result<Vec<JavaFieldMetadata>> {
        let env = self.inner.vm.attach_current_thread()?;
        metadata::declared_fields(&env, self)
    }

    pub fn is_instance(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
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
        let args = args.into_java_overload_args();
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

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.field(name)?.set((), value)
    }

    pub fn is_instance(&self, object: &(impl JavaObjectRef + ?Sized)) -> Result<bool> {
        self.class.is_instance(object)
    }

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
