use std::{
    cell::RefCell,
    collections::HashMap,
    fmt,
    rc::Rc,
    sync::{Arc, Mutex},
};

use crate::{
    env::{Env, FieldKind, FieldRef, MethodKind, MethodRef},
    error::{Error, Result},
    jni,
    metadata::{
        self, JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryGroup,
    },
    refs::{AsJClass, AsJObject, ClassKind, ClassRef, GlobalRef, LocalRef, ObjectKind},
    runtime::RuntimeCapabilities,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

/// A convenience handle for Java operations in one VM and one optional class-loader scope.
///
/// A plain `Java` value performs bootstrap-style `FindClass` lookups. `with_loader()` creates a
/// new `Java` value that resolves names through the supplied `ClassLoaderRef`. Class lookup caches
/// are intentionally per-`Java` instance so bootstrap and loader-backed lookups cannot share class
/// identity by accident.
#[derive(Clone)]
pub struct Java {
    vm: Vm,
    loader: Option<ClassLoaderRef>,
    classes: Arc<Mutex<HashMap<String, JavaClass>>>,
}

/// Describes how a `ClassLoaderRef` entered this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassLoaderKind {
    /// The process system class loader returned by `ClassLoader.getSystemClassLoader()`.
    System,
    /// A loader explicitly wrapped from a Java object.
    Object,
    /// A loader discovered by ART class-loader enumeration.
    Enumerated,
}

/// An owned global reference to a `java.lang.ClassLoader`.
///
/// Loader references are VM-scoped and may be cloned cheaply. They are validated as
/// `java.lang.ClassLoader` instances when constructed.
#[derive(Clone)]
pub struct ClassLoaderRef {
    vm: Vm,
    object: Arc<GlobalRef<ObjectKind>>,
    kind: ClassLoaderKind,
}

/// An owned global reference to a Java class plus cached method and field IDs.
///
/// The cached JNI IDs are tied to this class' defining identity. Instances from a different loader
/// should be resolved through that loader's `Java` value instead of reusing this class wrapper.
/// `name()` returns a Java binary name such as `java.lang.String`, matching the upstream
/// `frida-java-bridge` user-facing class-name convention. Descriptors and `JavaType` values still
/// use JNI slash-style names such as `Ljava/lang/String;`.
#[derive(Clone)]
pub struct JavaClass {
    inner: Arc<JavaClassInner>,
}

/// A GumJS-inspired class wrapper backed by the crate's explicit Rust-native API.
///
/// `JavaClassWrapper` is intentionally not a drop-in clone of JavaScript `Java.use()`. It provides
/// a permanent wrapper surface for class/member metadata and explicit overload calls, while method
/// replacement, automatic overload dispatch, and JavaScript object semantics remain separate
/// milestones.
#[derive(Clone)]
pub struct JavaClassWrapper {
    class: JavaClass,
    methods: Rc<RefCell<Option<Vec<JavaMethodMetadata>>>>,
    fields: Rc<RefCell<Option<Vec<JavaFieldMetadata>>>>,
}

/// A selected constructor overload on a `JavaClassWrapper`.
#[derive(Clone)]
pub struct JavaConstructorOverload {
    class: JavaClass,
    metadata: JavaMethodMetadata,
}

/// A selected method overload on a `JavaClassWrapper`.
#[derive(Clone)]
pub struct JavaMethodOverload {
    class: JavaClass,
    metadata: JavaMethodMetadata,
}

/// A selected field on a `JavaClassWrapper`.
#[derive(Clone)]
pub struct JavaFieldHandle {
    class: JavaClass,
    metadata: JavaFieldMetadata,
}

/// An owned global reference to a Java object.
///
/// Object wrappers retain the VM and JNI reference ownership only. They do not currently record the
/// defining class loader of the object's class; callers should keep using the relevant `JavaClass`
/// or loader-backed `Java` value for follow-up class and member lookup.
pub struct JavaObject {
    vm: Vm,
    object: GlobalRef<ObjectKind>,
}

#[derive(Debug)]
pub enum JavaReturn {
    Void,
    Boolean(bool),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(Option<JavaObject>),
}

/// Converts common Rust argument containers into explicit JNI argument values.
///
/// This keeps low-level JNI marshaling visible through `JavaValue`, while letting wrapper and
/// overload call sites pass tuples, arrays, slices, or vectors without hand-building temporary
/// slices every time.
pub trait IntoJavaArgs {
    fn into_java_args(self) -> Vec<JavaValue>;
}

impl JavaReturn {
    pub fn into_void(self, operation: &'static str) -> Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(invalid_return(operation, "void", other)),
        }
    }

    pub fn into_boolean(self, operation: &'static str) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(value),
            other => Err(invalid_return(operation, "boolean", other)),
        }
    }

    pub fn into_byte(self, operation: &'static str) -> Result<jni::jbyte> {
        match self {
            Self::Byte(value) => Ok(value),
            other => Err(invalid_return(operation, "byte", other)),
        }
    }

    pub fn into_char(self, operation: &'static str) -> Result<jni::jchar> {
        match self {
            Self::Char(value) => Ok(value),
            other => Err(invalid_return(operation, "char", other)),
        }
    }

    pub fn into_short(self, operation: &'static str) -> Result<jni::jshort> {
        match self {
            Self::Short(value) => Ok(value),
            other => Err(invalid_return(operation, "short", other)),
        }
    }

    pub fn into_int(self, operation: &'static str) -> Result<jni::jint> {
        match self {
            Self::Int(value) => Ok(value),
            other => Err(invalid_return(operation, "int", other)),
        }
    }

    pub fn into_long(self, operation: &'static str) -> Result<jni::jlong> {
        match self {
            Self::Long(value) => Ok(value),
            other => Err(invalid_return(operation, "long", other)),
        }
    }

    pub fn into_float(self, operation: &'static str) -> Result<jni::jfloat> {
        match self {
            Self::Float(value) => Ok(value),
            other => Err(invalid_return(operation, "float", other)),
        }
    }

    pub fn into_double(self, operation: &'static str) -> Result<jni::jdouble> {
        match self {
            Self::Double(value) => Ok(value),
            other => Err(invalid_return(operation, "double", other)),
        }
    }

    pub fn into_object(self, operation: &'static str) -> Result<Option<JavaObject>> {
        match self {
            Self::Object(value) => Ok(value),
            other => Err(invalid_return(operation, "object", other)),
        }
    }
}

impl IntoJavaArgs for () {
    fn into_java_args(self) -> Vec<JavaValue> {
        Vec::new()
    }
}

impl IntoJavaArgs for Vec<JavaValue> {
    fn into_java_args(self) -> Vec<JavaValue> {
        self
    }
}

impl IntoJavaArgs for &[JavaValue] {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.to_vec()
    }
}

impl<const N: usize> IntoJavaArgs for [JavaValue; N] {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.to_vec()
    }
}

impl<const N: usize> IntoJavaArgs for &[JavaValue; N] {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.to_vec()
    }
}

macro_rules! impl_into_java_args_for_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> IntoJavaArgs for ($($name,)+)
        where
            $($name: Into<JavaValue>),+
        {
            fn into_java_args(self) -> Vec<JavaValue> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                vec![$($name.into()),+]
            }
        }
    };
}

impl_into_java_args_for_tuple!(A);
impl_into_java_args_for_tuple!(A, B);
impl_into_java_args_for_tuple!(A, B, C);
impl_into_java_args_for_tuple!(A, B, C, D);
impl_into_java_args_for_tuple!(A, B, C, D, E);
impl_into_java_args_for_tuple!(A, B, C, D, E, F);
impl_into_java_args_for_tuple!(A, B, C, D, E, F, G);
impl_into_java_args_for_tuple!(A, B, C, D, E, F, G, H);

struct JavaClassInner {
    vm: Vm,
    name: String,
    class: GlobalRef<ClassKind>,
    methods: Mutex<HashMap<MethodKey, MethodRef>>,
    fields: Mutex<HashMap<FieldKey, FieldRef>>,
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

struct RawObject(jni::jobject);

impl Java {
    pub(crate) fn new(vm: Vm) -> Self {
        Self {
            vm,
            loader: None,
            classes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn loader(&self) -> Option<&ClassLoaderRef> {
        self.loader.as_ref()
    }

    /// Returns a new `Java` handle that resolves classes through `loader`.
    ///
    /// The returned handle starts with an empty class cache. This keeps bootstrap, system-loader,
    /// DexClassLoader, and enumerated-loader lookups isolated even when the same binary class name
    /// is requested.
    pub fn with_loader(&self, loader: &ClassLoaderRef) -> Self {
        Self {
            vm: self.vm.clone(),
            loader: Some(loader.clone()),
            classes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn capabilities(&self) -> RuntimeCapabilities {
        self.vm.capabilities()
    }

    pub fn system_class_loader(&self) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        let class_loader_class = env.find_class("java/lang/ClassLoader")?;
        let get_system_class_loader = env.get_static_method(
            &class_loader_class,
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
        )?;
        let loader = env
            .call_static_object_method(&class_loader_class, &get_system_class_loader, &[])?
            .ok_or(Error::NullReturn {
                operation: "ClassLoader.getSystemClassLoader",
            })?;
        ClassLoaderRef::from_object_ref(&env, &self.vm, &loader, ClassLoaderKind::System)
    }

    /// Wraps a Java object as a class-loader reference after validating its runtime type.
    pub fn class_loader_from_object(&self, object: &JavaObject) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        ClassLoaderRef::from_java_object(&env, &self.vm, object, ClassLoaderKind::Object)
    }

    /// Enumerates ART class loaders when the current runtime layout is supported.
    ///
    /// This is currently an Android ART API 26+ arm64 milestone feature. Unsupported layouts,
    /// missing ART symbols, and unsupported architectures return `Error::UnsupportedFeature`
    /// instead of silently falling back.
    pub fn enumerate_class_loaders(&self) -> Result<Vec<ClassLoaderRef>> {
        self.vm.enumerate_class_loaders()
    }

    /// Enumerates loaded Java classes when the ART backend supports it.
    pub fn enumerate_loaded_classes(&self) -> Result<Vec<JavaClass>> {
        self.vm.enumerate_loaded_classes()
    }

    /// Enumerates methods matching an upstream-inspired `class!method` query.
    ///
    /// Class patterns use Java binary names such as `java.lang.String` and
    /// `com.example.*`. Constructor methods are exposed as `$init`.
    /// Supported modifiers are `/i` for case-insensitive matching, `/s` for signature-aware
    /// matching, and `/u` for skipping bootstrap/platform classes. Signatures included by `/s`
    /// remain JNI descriptors, for example `$init(I)V`.
    pub fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        match self.vm.enumerate_methods(query) {
            Ok(groups) => Ok(groups),
            Err(Error::UnsupportedFeature {
                feature: "ART direct method enumeration",
                ..
            }) => {
                let classes = self.enumerate_loaded_classes()?;
                metadata::enumerate_methods(self, &classes, query)
            }
            Err(error) => Err(error),
        }
    }

    /// Finds a class in this handle's class-loader scope.
    ///
    /// Accepted names include dotted binary names (`java.lang.String`), JNI internal names
    /// (`java/lang/String`), object descriptors (`Ljava/lang/String;`), and array descriptors
    /// (`[I`, `[Ljava/lang/String;`). Bootstrap lookups use JNI internal names with
    /// `FindClass`; loader-backed lookups use binary names through `ClassLoader.loadClass()` and
    /// array descriptors through `Class.forName(name, false, loader)`.
    pub fn find_class(&self, name: &str) -> Result<JavaClass> {
        let env = self.vm.attach_current_thread()?;
        let lookup = normalize_class_lookup_name(name);

        if let Some(class) = self
            .classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .get(&lookup.cache_key)
            .cloned()
        {
            return Ok(class);
        }

        let local = match &self.loader {
            Some(loader) => find_class_with_loader(&env, loader, &lookup)?,
            None => env.find_class(&lookup.find_class_name)?,
        };
        let class = env.new_global_ref(&local)?;

        let class = JavaClass {
            inner: Arc::new(JavaClassInner {
                vm: self.vm.clone(),
                name: lookup.public_name,
                class,
                methods: Mutex::new(HashMap::new()),
                fields: Mutex::new(HashMap::new()),
            }),
        };

        self.classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .insert(lookup.cache_key, class.clone());

        Ok(class)
    }

    /// Builds a Java.use-style class wrapper in this handle's class-loader scope.
    ///
    /// The wrapper exposes reflection-backed member metadata and explicit overload invocation on
    /// top of `JavaClass`. It preserves this `Java` handle's loader boundary.
    pub fn use_class(&self, name: &str) -> Result<JavaClassWrapper> {
        Ok(JavaClassWrapper::new(self.find_class(name)?))
    }

    pub fn new_string_utf(&self, text: &str) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        let string = env.new_string_utf(text)?;
        object_from_ref(&env, &self.vm, &string)
    }

    pub fn new_object_array(
        &self,
        element_class: &JavaClass,
        elements: &[Option<&JavaObject>],
    ) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        let array = env.new_object_array(
            elements.len() as jni::jsize,
            element_class,
            None::<&JavaObject>,
        )?;
        for (index, element) in elements.iter().enumerate() {
            env.set_object_array_element(&array, index as jni::jsize, *element)?;
        }
        object_from_ref(&env, &self.vm, &array)
    }
}

impl ClassLoaderRef {
    pub fn kind(&self) -> ClassLoaderKind {
        self.kind
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.object.as_jobject()
    }

    #[allow(dead_code)]
    pub(crate) unsafe fn from_global_raw(
        vm: Vm,
        raw: jni::jobject,
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        let attached_vm = vm.clone();
        let env = attached_vm.attach_current_thread()?;
        let object = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        let loader = Self {
            vm,
            object: Arc::new(object),
            kind,
        };
        validate_class_loader(&env, &loader, "ClassLoaderRef::from_global_raw")?;
        Ok(loader)
    }

    pub(crate) fn from_object_ref(
        env: &Env<'_>,
        vm: &Vm,
        object: &(impl AsJObject + ?Sized),
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
        let object = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
        let loader = Self {
            vm: vm.clone(),
            object: Arc::new(object),
            kind,
        };
        validate_class_loader(env, &loader, "Java::class_loader_from_object")?;
        Ok(loader)
    }

    fn from_java_object(
        env: &Env<'_>,
        vm: &Vm,
        object: &JavaObject,
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        Self::from_object_ref(env, vm, object, kind)
    }
}

impl fmt::Debug for ClassLoaderRef {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("ClassLoaderRef")
            .field("kind", &self.kind)
            .field("object", &self.as_jobject())
            .finish()
    }
}

impl JavaClassWrapper {
    fn new(class: JavaClass) -> Self {
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

    pub fn new_object<A: IntoJavaArgs>(&self, signature: &str, args: A) -> Result<JavaObject> {
        self.ensure_method(MethodKind::Constructor, "<init>", signature)?;
        let args = args.into_java_args();
        self.class.new_object(signature, &args)
    }

    pub fn call<A: IntoJavaArgs>(
        &self,
        object: &JavaObject,
        name: &str,
        signature: &str,
        args: A,
    ) -> Result<JavaReturn> {
        self.ensure_method(MethodKind::Instance, name, signature)?;
        let args = args.into_java_args();
        self.class.call_method(object, name, signature, &args)
    }

    pub fn call_static<A: IntoJavaArgs>(
        &self,
        name: &str,
        signature: &str,
        args: A,
    ) -> Result<JavaReturn> {
        self.ensure_method(MethodKind::Static, name, signature)?;
        let args = args.into_java_args();
        self.class.call_static(name, signature, &args)
    }

    pub fn get_field(&self, object: &JavaObject, name: &str, ty: &str) -> Result<JavaReturn> {
        self.ensure_field(FieldKind::Instance, name, ty)?;
        self.class.get_field(object, name, ty)
    }

    pub fn set_field(
        &self,
        object: &JavaObject,
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

    pub fn is_instance(&self, object: &JavaObject) -> Result<bool> {
        self.class.is_instance(object)
    }

    pub fn cast(&self, object: &JavaObject) -> Result<JavaObject> {
        if self.is_instance(object)? {
            object.retain()
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

impl JavaConstructorOverload {
    pub fn metadata(&self) -> &JavaMethodMetadata {
        &self.metadata
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.metadata.signature
    }

    pub fn new_object<A: IntoJavaArgs>(&self, args: A) -> Result<JavaObject> {
        let args = args.into_java_args();
        self.class
            .new_object(&self.metadata.signature.to_string(), &args)
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

    pub fn call<A: IntoJavaArgs>(&self, object: &JavaObject, args: A) -> Result<JavaReturn> {
        if self.metadata.kind != MethodKind::Instance {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethodOverload::call",
            });
        }
        let args = args.into_java_args();
        self.class.call_method(
            object,
            &self.metadata.name,
            &self.metadata.signature.to_string(),
            &args,
        )
    }

    pub fn call_static<A: IntoJavaArgs>(&self, args: A) -> Result<JavaReturn> {
        if self.metadata.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "JavaMethodOverload::call_static",
            });
        }
        let args = args.into_java_args();
        self.class.call_static(
            &self.metadata.name,
            &self.metadata.signature.to_string(),
            &args,
        )
    }

    pub fn call_void<A: IntoJavaArgs>(&self, object: &JavaObject, args: A) -> Result<()> {
        self.call(object, args)?
            .into_void("JavaMethodOverload::call_void")
    }

    pub fn call_boolean<A: IntoJavaArgs>(&self, object: &JavaObject, args: A) -> Result<bool> {
        self.call(object, args)?
            .into_boolean("JavaMethodOverload::call_boolean")
    }

    pub fn call_int<A: IntoJavaArgs>(&self, object: &JavaObject, args: A) -> Result<jni::jint> {
        self.call(object, args)?
            .into_int("JavaMethodOverload::call_int")
    }

    pub fn call_object<A: IntoJavaArgs>(
        &self,
        object: &JavaObject,
        args: A,
    ) -> Result<Option<JavaObject>> {
        self.call(object, args)?
            .into_object("JavaMethodOverload::call_object")
    }

    pub fn call_string<A: IntoJavaArgs>(
        &self,
        object: &JavaObject,
        args: A,
    ) -> Result<Option<String>> {
        self.call_object(object, args)?
            .map(|object| object.get_string())
            .transpose()
    }

    pub fn call_static_void<A: IntoJavaArgs>(&self, args: A) -> Result<()> {
        self.call_static(args)?
            .into_void("JavaMethodOverload::call_static_void")
    }

    pub fn call_static_boolean<A: IntoJavaArgs>(&self, args: A) -> Result<bool> {
        self.call_static(args)?
            .into_boolean("JavaMethodOverload::call_static_boolean")
    }

    pub fn call_static_int<A: IntoJavaArgs>(&self, args: A) -> Result<jni::jint> {
        self.call_static(args)?
            .into_int("JavaMethodOverload::call_static_int")
    }

    pub fn call_static_object<A: IntoJavaArgs>(&self, args: A) -> Result<Option<JavaObject>> {
        self.call_static(args)?
            .into_object("JavaMethodOverload::call_static_object")
    }

    pub fn call_static_string<A: IntoJavaArgs>(&self, args: A) -> Result<Option<String>> {
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

    pub fn get(&self, object: &JavaObject) -> Result<JavaReturn> {
        if self.metadata.kind != FieldKind::Instance {
            return Err(Error::WrongFieldKind {
                operation: "JavaFieldHandle::get",
            });
        }
        self.class
            .get_field(object, &self.metadata.name, &self.metadata.ty.to_string())
    }

    pub fn get_int(&self, object: &JavaObject) -> Result<jni::jint> {
        self.get(object)?.into_int("JavaFieldHandle::get_int")
    }

    pub fn get_object(&self, object: &JavaObject) -> Result<Option<JavaObject>> {
        self.get(object)?.into_object("JavaFieldHandle::get_object")
    }

    pub fn set(&self, object: &JavaObject, value: JavaValue) -> Result<()> {
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

    pub fn set_int(&self, object: &JavaObject, value: jni::jint) -> Result<()> {
        self.set(object, JavaValue::Int(value))
    }

    pub fn set_object(&self, object: &JavaObject, value: Option<&JavaObject>) -> Result<()> {
        self.set(object, JavaValue::from(value))
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

    pub fn get_static_object(&self) -> Result<Option<JavaObject>> {
        self.get_static()?
            .into_object("JavaFieldHandle::get_static_object")
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

    pub fn set_static_object(&self, value: Option<&JavaObject>) -> Result<()> {
        self.set_static(JavaValue::from(value))
    }
}

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

    pub(crate) fn resolve_static_method(&self, name: &str, signature: &str) -> Result<MethodRef> {
        let env = self.inner.vm.attach_current_thread()?;
        self.static_method(&env, name, signature)
    }

    pub(crate) fn resolve_instance_method(&self, name: &str, signature: &str) -> Result<MethodRef> {
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

    fn constructor(&self, env: &Env<'_>, signature: &str) -> Result<MethodRef> {
        self.cached_method(env, MethodKind::Constructor, "<init>", signature)
    }

    fn method(&self, env: &Env<'_>, name: &str, signature: &str) -> Result<MethodRef> {
        self.cached_method(env, MethodKind::Instance, name, signature)
    }

    fn static_method(&self, env: &Env<'_>, name: &str, signature: &str) -> Result<MethodRef> {
        self.cached_method(env, MethodKind::Static, name, signature)
    }

    fn field(&self, env: &Env<'_>, name: &str, ty: &str) -> Result<FieldRef> {
        self.cached_field(env, FieldKind::Instance, name, ty)
    }

    fn static_field(&self, env: &Env<'_>, name: &str, ty: &str) -> Result<FieldRef> {
        self.cached_field(env, FieldKind::Static, name, ty)
    }

    fn cached_method(
        &self,
        env: &Env<'_>,
        kind: MethodKind,
        name: &str,
        signature: &str,
    ) -> Result<MethodRef> {
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
            MethodKind::Constructor => env.get_constructor(&self.inner.class, &key.signature)?,
            MethodKind::Instance => env.get_method(&self.inner.class, name, &key.signature)?,
            MethodKind::Static => env.get_static_method(&self.inner.class, name, &key.signature)?,
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
    ) -> Result<FieldRef> {
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
            FieldKind::Instance => env.get_field(&self.inner.class, name, &key.ty)?,
            FieldKind::Static => env.get_static_field(&self.inner.class, name, &key.ty)?,
        };

        self.inner
            .fields
            .lock()
            .expect("JavaClass field cache mutex poisoned")
            .insert(key, field.clone());

        Ok(field)
    }
}

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

impl AsJObject for JavaClass {
    fn as_jobject(&self) -> jni::jobject {
        self.inner.class.as_jobject()
    }
}

impl AsJObject for ClassLoaderRef {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

impl AsJClass for JavaClass {
    fn as_jclass(&self) -> jni::jclass {
        self.as_jclass()
    }
}

impl AsJObject for RawObject {
    fn as_jobject(&self) -> jni::jobject {
        self.0
    }
}

impl From<&JavaObject> for JavaValue {
    fn from(value: &JavaObject) -> Self {
        Self::Object(value.as_jobject())
    }
}

impl From<Option<&JavaObject>> for JavaValue {
    fn from(value: Option<&JavaObject>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

fn call_instance_return(
    env: &Env<'_>,
    object: &JavaObject,
    method: &MethodRef,
    args: &[JavaValue],
) -> Result<JavaReturn> {
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            env.call_void_method(object, method, args)?;
            JavaReturn::Void
        }
        JavaType::Boolean => JavaReturn::Boolean(env.call_boolean_method(object, method, args)?),
        JavaType::Byte => JavaReturn::Byte(env.call_byte_method(object, method, args)?),
        JavaType::Char => JavaReturn::Char(env.call_char_method(object, method, args)?),
        JavaType::Short => JavaReturn::Short(env.call_short_method(object, method, args)?),
        JavaType::Int => JavaReturn::Int(env.call_int_method(object, method, args)?),
        JavaType::Long => JavaReturn::Long(env.call_long_method(object, method, args)?),
        JavaType::Float => JavaReturn::Float(env.call_float_method(object, method, args)?),
        JavaType::Double => JavaReturn::Double(env.call_double_method(object, method, args)?),
        JavaType::Object(_) | JavaType::Array(_) => JavaReturn::Object(
            env.call_object_method(object, method, args)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
    })
}

fn call_static_return(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    method: &MethodRef,
    args: &[JavaValue],
) -> Result<JavaReturn> {
    Ok(match method.signature().return_type() {
        JavaType::Void => {
            env.call_static_void_method(class, method, args)?;
            JavaReturn::Void
        }
        JavaType::Boolean => {
            JavaReturn::Boolean(env.call_static_boolean_method(class, method, args)?)
        }
        JavaType::Byte => JavaReturn::Byte(env.call_static_byte_method(class, method, args)?),
        JavaType::Char => JavaReturn::Char(env.call_static_char_method(class, method, args)?),
        JavaType::Short => JavaReturn::Short(env.call_static_short_method(class, method, args)?),
        JavaType::Int => JavaReturn::Int(env.call_static_int_method(class, method, args)?),
        JavaType::Long => JavaReturn::Long(env.call_static_long_method(class, method, args)?),
        JavaType::Float => JavaReturn::Float(env.call_static_float_method(class, method, args)?),
        JavaType::Double => JavaReturn::Double(env.call_static_double_method(class, method, args)?),
        JavaType::Object(_) | JavaType::Array(_) => JavaReturn::Object(
            env.call_static_object_method(class, method, args)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
    })
}

fn get_instance_field(env: &Env<'_>, object: &JavaObject, field: &FieldRef) -> Result<JavaReturn> {
    Ok(match field.ty() {
        JavaType::Boolean => JavaReturn::Boolean(env.get_boolean_field(object, field)?),
        JavaType::Byte => JavaReturn::Byte(env.get_byte_field(object, field)?),
        JavaType::Char => JavaReturn::Char(env.get_char_field(object, field)?),
        JavaType::Short => JavaReturn::Short(env.get_short_field(object, field)?),
        JavaType::Int => JavaReturn::Int(env.get_int_field(object, field)?),
        JavaType::Long => JavaReturn::Long(env.get_long_field(object, field)?),
        JavaType::Float => JavaReturn::Float(env.get_float_field(object, field)?),
        JavaType::Double => JavaReturn::Double(env.get_double_field(object, field)?),
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "JavaClass::get_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(_) | JavaType::Array(_) => JavaReturn::Object(
            env.get_object_field(object, field)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
    })
}

fn set_instance_field(
    env: &Env<'_>,
    object: &JavaObject,
    field: &FieldRef,
    value: JavaValue,
) -> Result<()> {
    validate_field_value(field, value)?;
    match value {
        JavaValue::Boolean(value) => env.set_boolean_field(object, field, value),
        JavaValue::Byte(value) => env.set_byte_field(object, field, value),
        JavaValue::Char(value) => env.set_char_field(object, field, value),
        JavaValue::Short(value) => env.set_short_field(object, field, value),
        JavaValue::Int(value) => env.set_int_field(object, field, value),
        JavaValue::Long(value) => env.set_long_field(object, field, value),
        JavaValue::Float(value) => env.set_float_field(object, field, value),
        JavaValue::Double(value) => env.set_double_field(object, field, value),
        JavaValue::Object(value) if !value.is_null() => {
            let value = RawObject(value);
            env.set_object_field(object, field, Some(&value))
        }
        JavaValue::Object(_) | JavaValue::Null => env.set_object_field(object, field, None),
    }
}

fn get_static_field(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    field: &FieldRef,
) -> Result<JavaReturn> {
    Ok(match field.ty() {
        JavaType::Boolean => JavaReturn::Boolean(env.get_static_boolean_field(class, field)?),
        JavaType::Byte => JavaReturn::Byte(env.get_static_byte_field(class, field)?),
        JavaType::Char => JavaReturn::Char(env.get_static_char_field(class, field)?),
        JavaType::Short => JavaReturn::Short(env.get_static_short_field(class, field)?),
        JavaType::Int => JavaReturn::Int(env.get_static_int_field(class, field)?),
        JavaType::Long => JavaReturn::Long(env.get_static_long_field(class, field)?),
        JavaType::Float => JavaReturn::Float(env.get_static_float_field(class, field)?),
        JavaType::Double => JavaReturn::Double(env.get_static_double_field(class, field)?),
        JavaType::Void => {
            return Err(Error::InvalidFieldType {
                operation: "JavaClass::get_static_field",
                expected: "non-void",
                actual: field.ty().to_string(),
            });
        }
        JavaType::Object(_) | JavaType::Array(_) => JavaReturn::Object(
            env.get_static_object_field(class, field)?
                .map(|object| object_from_ref(env, env.vm(), &object))
                .transpose()?,
        ),
    })
}

fn set_static_field(
    env: &Env<'_>,
    class: &GlobalRef<ClassKind>,
    field: &FieldRef,
    value: JavaValue,
) -> Result<()> {
    validate_field_value(field, value)?;
    match value {
        JavaValue::Boolean(value) => env.set_static_boolean_field(class, field, value),
        JavaValue::Byte(value) => env.set_static_byte_field(class, field, value),
        JavaValue::Char(value) => env.set_static_char_field(class, field, value),
        JavaValue::Short(value) => env.set_static_short_field(class, field, value),
        JavaValue::Int(value) => env.set_static_int_field(class, field, value),
        JavaValue::Long(value) => env.set_static_long_field(class, field, value),
        JavaValue::Float(value) => env.set_static_float_field(class, field, value),
        JavaValue::Double(value) => env.set_static_double_field(class, field, value),
        JavaValue::Object(value) if !value.is_null() => {
            let value = RawObject(value);
            env.set_static_object_field(class, field, Some(&value))
        }
        JavaValue::Object(_) | JavaValue::Null => env.set_static_object_field(class, field, None),
    }
}

fn validate_field_value(field: &FieldRef, value: JavaValue) -> Result<()> {
    if value.matches_type(field.ty()) {
        Ok(())
    } else {
        Err(Error::InvalidArgumentType {
            index: 0,
            expected: field.ty().to_string(),
            actual: value.type_name(),
        })
    }
}

fn invalid_return(operation: &'static str, expected: &'static str, actual: JavaReturn) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: return_type_name(&actual).to_owned(),
    }
}

fn return_type_name(value: &JavaReturn) -> &'static str {
    match value {
        JavaReturn::Void => "void",
        JavaReturn::Boolean(_) => "boolean",
        JavaReturn::Byte(_) => "byte",
        JavaReturn::Char(_) => "char",
        JavaReturn::Short(_) => "short",
        JavaReturn::Int(_) => "int",
        JavaReturn::Long(_) => "long",
        JavaReturn::Float(_) => "float",
        JavaReturn::Double(_) => "double",
        JavaReturn::Object(_) => "object",
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

pub(crate) fn object_from_ref(
    env: &Env<'_>,
    vm: &Vm,
    object: &(impl AsJObject + ?Sized),
) -> Result<JavaObject> {
    let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
    let object = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
    Ok(JavaObject {
        vm: vm.clone(),
        object,
    })
}

fn validate_class_loader(
    env: &Env<'_>,
    loader: &ClassLoaderRef,
    operation: &'static str,
) -> Result<()> {
    let class_loader_class = env.find_class("java/lang/ClassLoader")?;
    if env.is_instance_of(loader, &class_loader_class)? {
        Ok(())
    } else {
        let actual = env.get_object_class(loader)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "java.lang.ClassLoader",
            actual: format!("{:p}", actual.as_jclass()),
        })
    }
}

fn find_class_with_loader<'env, 'vm>(
    env: &'env Env<'vm>,
    loader: &ClassLoaderRef,
    lookup: &ClassLookupName,
) -> Result<ClassRef<'env>> {
    if lookup.is_array_descriptor {
        let class_class = env.find_class("java/lang/Class")?;
        let for_name = env.get_static_method(
            &class_class,
            "forName",
            "(Ljava/lang/String;ZLjava/lang/ClassLoader;)Ljava/lang/Class;",
        )?;
        let name = env.new_string_utf(&lookup.loader_name)?;
        let class = env
            .call_static_object_method(
                &class_class,
                &for_name,
                &[
                    JavaValue::from(&name),
                    JavaValue::Boolean(false),
                    JavaValue::Object(loader.as_jobject()),
                ],
            )?
            .ok_or(Error::NullReturn {
                operation: "Class.forName",
            })?;
        unsafe { LocalRef::from_raw(env, class.into_raw()) }
    } else {
        let class_loader_class = env.find_class("java/lang/ClassLoader")?;
        let load_class = env.get_method(
            &class_loader_class,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
        )?;
        let name = env.new_string_utf(&lookup.loader_name)?;
        let class = env
            .call_object_method(loader, &load_class, &[JavaValue::from(&name)])?
            .ok_or(Error::NullReturn {
                operation: "ClassLoader.loadClass",
            })?;
        unsafe { LocalRef::from_raw(env, class.into_raw()) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassLookupName {
    cache_key: String,
    find_class_name: String,
    loader_name: String,
    public_name: String,
    is_array_descriptor: bool,
}

fn normalize_class_lookup_name(name: &str) -> ClassLookupName {
    let is_array_descriptor = name.starts_with('[');
    let stripped = if !is_array_descriptor && name.starts_with('L') && name.ends_with(';') {
        &name[1..name.len() - 1]
    } else {
        name
    };
    let find_class_name = stripped.replace('.', "/");
    let loader_name = if is_array_descriptor {
        normalize_array_descriptor_for_loader(name)
    } else {
        stripped.replace('/', ".")
    };
    let public_name = loader_name.clone();

    ClassLookupName {
        cache_key: find_class_name.clone(),
        find_class_name,
        loader_name,
        public_name,
        is_array_descriptor,
    }
}

fn normalize_array_descriptor_for_loader(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut in_object = false;
    for ch in name.chars() {
        match ch {
            'L' if !in_object => {
                in_object = true;
                result.push(ch);
            }
            ';' if in_object => {
                in_object = false;
                result.push(ch);
            }
            '/' if in_object => result.push('.'),
            _ => result.push(ch),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_jni_internal_names_for_bootstrap_lookup() {
        let dotted = normalize_class_lookup_name("java.lang.String");
        assert_eq!(dotted.find_class_name, "java/lang/String");
        assert_eq!(dotted.public_name, "java.lang.String");

        let internal = normalize_class_lookup_name("java/lang/String");
        assert_eq!(internal.find_class_name, "java/lang/String");
        assert_eq!(internal.public_name, "java.lang.String");

        let descriptor = normalize_class_lookup_name("Ljava/lang/String;");
        assert_eq!(descriptor.find_class_name, "java/lang/String");
        assert_eq!(descriptor.public_name, "java.lang.String");

        let dotted_descriptor = normalize_class_lookup_name("Ljava.lang.String;");
        assert_eq!(dotted_descriptor.find_class_name, "java/lang/String");
        assert_eq!(dotted_descriptor.public_name, "java.lang.String");

        let inner = normalize_class_lookup_name("com.example.Outer$Inner");
        assert_eq!(inner.find_class_name, "com/example/Outer$Inner");
        assert_eq!(inner.public_name, "com.example.Outer$Inner");
    }

    #[test]
    fn normalizes_loader_binary_names() {
        assert_eq!(
            normalize_class_lookup_name("java/lang/String").loader_name,
            "java.lang.String"
        );
        assert_eq!(
            normalize_class_lookup_name("Ljava/lang/String;").loader_name,
            "java.lang.String"
        );
        assert_eq!(
            normalize_class_lookup_name("com.example.Outer$Inner").loader_name,
            "com.example.Outer$Inner"
        );
    }

    #[test]
    fn normalizes_array_descriptors_for_each_lookup_path() {
        let primitive = normalize_class_lookup_name("[I");
        assert_eq!(primitive.find_class_name, "[I");
        assert_eq!(primitive.loader_name, "[I");
        assert!(primitive.is_array_descriptor);

        let object = normalize_class_lookup_name("[Ljava/lang/String;");
        assert_eq!(object.find_class_name, "[Ljava/lang/String;");
        assert_eq!(object.loader_name, "[Ljava.lang.String;");
        assert_eq!(object.public_name, "[Ljava.lang.String;");
        assert!(object.is_array_descriptor);

        let dotted = normalize_class_lookup_name("[Ljava.lang.String;");
        assert_eq!(dotted.find_class_name, "[Ljava/lang/String;");
        assert_eq!(dotted.loader_name, "[Ljava.lang.String;");
    }

    #[test]
    fn normalizes_multi_dimensional_array_descriptors() {
        let object = normalize_class_lookup_name("[[Ljava/lang/String;");
        assert_eq!(object.cache_key, "[[Ljava/lang/String;");
        assert_eq!(object.find_class_name, "[[Ljava/lang/String;");
        assert_eq!(object.loader_name, "[[Ljava.lang.String;");
        assert!(object.is_array_descriptor);

        let primitive = normalize_class_lookup_name("[[I");
        assert_eq!(primitive.cache_key, "[[I");
        assert_eq!(primitive.find_class_name, "[[I");
        assert_eq!(primitive.loader_name, "[[I");
        assert!(primitive.is_array_descriptor);
    }

    #[test]
    fn preserves_inner_class_binary_names() {
        let lookup = normalize_class_lookup_name("Lcom.example.Outer$Inner;");
        assert_eq!(lookup.cache_key, "com/example/Outer$Inner");
        assert_eq!(lookup.find_class_name, "com/example/Outer$Inner");
        assert_eq!(lookup.loader_name, "com.example.Outer$Inner");
        assert_eq!(lookup.public_name, "com.example.Outer$Inner");
        assert!(!lookup.is_array_descriptor);
    }

    #[test]
    fn caches_are_isolated_per_java_instance() {
        let bootstrap = Java::new(Vm::dangling_for_tests());
        let other = Java::new(Vm::dangling_for_tests());
        assert!(!Arc::ptr_eq(&bootstrap.classes, &other.classes));
        assert!(bootstrap.loader().is_none());
        assert!(other.loader().is_none());
    }

    #[test]
    fn extracts_java_return_values() {
        JavaReturn::Void.into_void("void").unwrap();
        assert!(JavaReturn::Boolean(true).into_boolean("boolean").unwrap());
        assert_eq!(JavaReturn::Byte(-7).into_byte("byte").unwrap(), -7);
        assert_eq!(JavaReturn::Char(65).into_char("char").unwrap(), 65);
        assert_eq!(JavaReturn::Short(-300).into_short("short").unwrap(), -300);
        assert_eq!(JavaReturn::Int(42).into_int("int").unwrap(), 42);
        assert_eq!(JavaReturn::Long(9001).into_long("long").unwrap(), 9001);
        assert_eq!(JavaReturn::Float(1.5).into_float("float").unwrap(), 1.5);
        assert_eq!(JavaReturn::Double(2.5).into_double("double").unwrap(), 2.5);
        assert!(
            JavaReturn::Object(None)
                .into_object("object")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn converts_common_java_argument_containers() {
        assert_eq!(().into_java_args(), Vec::<JavaValue>::new());

        let values = [JavaValue::Int(7), JavaValue::Null];
        assert_eq!(
            values.into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );
        assert_eq!(
            (&values).into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );

        let slice: &[JavaValue] = &values;
        assert_eq!(
            slice.into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );

        assert_eq!(
            vec![JavaValue::Boolean(true)].into_java_args(),
            vec![JavaValue::Boolean(true)]
        );
    }

    #[test]
    fn converts_tuple_java_arguments() {
        assert_eq!(
            (7 as jni::jint, true, JavaValue::Null).into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Boolean(true), JavaValue::Null]
        );
    }

    #[test]
    fn converts_optional_java_object_arguments() {
        assert_eq!(JavaValue::from(None::<&JavaObject>), JavaValue::Null);
        assert_eq!(
            (None::<&JavaObject>,).into_java_args(),
            vec![JavaValue::Null]
        );
    }

    #[test]
    fn reports_java_return_type_mismatches() {
        let error = JavaReturn::Int(7).into_object("TestSubject.message");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.message",
                expected: "object",
                actual: "int".to_owned(),
            }
        );

        let error = JavaReturn::Object(None).into_int("TestSubject.answer");
        assert_eq!(
            error.unwrap_err(),
            Error::InvalidReturnType {
                operation: "TestSubject.answer",
                expected: "int",
                actual: "object".to_owned(),
            }
        );
    }

    #[test]
    fn formats_loader_errors() {
        let unsupported = Error::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            reason: "missing symbol".to_owned(),
        };
        assert_eq!(
            unsupported.to_string(),
            "ART class-loader enumeration is not supported: missing symbol"
        );

        let invalid = Error::InvalidObjectType {
            operation: "Java::class_loader_from_object",
            expected: "java.lang.ClassLoader",
            actual: "java.lang.String".to_owned(),
        };
        assert_eq!(
            invalid.to_string(),
            "Java::class_loader_from_object expected java.lang.ClassLoader, got java.lang.String"
        );
    }
}
