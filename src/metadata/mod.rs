//! Metadata returned by Java class, method, field, and query operations.
//!
//! These types describe Java declarations in Rust-friendly form. They are snapshots of what the
//! runtime reported at query time; invoking methods or reading fields still goes through the
//! high-level wrappers unless you explicitly opt into raw JNI IDs.

mod reflection;

use crate::{
    env::{Env, FieldKind, MethodKind},
    error::Result,
    jni,
    loader::ClassLoaderRef,
    refs::AsJObject,
    signature::{JavaType, MethodSignature},
    vm::Vm,
};

pub(crate) use crate::signature::class_name_from_descriptor;
pub use reflection::modifiers;
pub(crate) use reflection::{class_descriptor, class_loader};

/// Metadata describing one Java class.
#[derive(Debug, Clone)]
pub struct JavaClassMetadata {
    /// Java binary class name, for example `java.lang.String`.
    ///
    /// Array names follow `Class.getName()` style, for example `[Ljava.lang.String;`.
    pub name: String,
    /// JNI descriptor, for example `Ljava/lang/String;`.
    pub descriptor: String,
    /// Class loader that defined the class, or `None` for bootstrap classes.
    pub loader: Option<ClassLoaderRef>,
}

/// Metadata describing one Java method or constructor.
#[derive(Debug, Clone)]
pub struct JavaMethodMetadata {
    /// Method name, or a JVM special name such as `<init>` for constructors.
    pub name: String,
    /// Whether the ID refers to a constructor, instance method, or static method.
    pub kind: MethodKind,
    /// Parsed JNI method signature.
    pub signature: MethodSignature,
    /// Java reflection modifier flags.
    pub modifiers: jni::jint,
    pub(crate) id: jni::jmethodID,
}

// JNI method IDs are VM-owned opaque identifiers. They are not thread-affine; callers still need a
// valid thread-local JNIEnv to invoke them.
unsafe impl Send for JavaMethodMetadata {}
unsafe impl Sync for JavaMethodMetadata {}

impl JavaMethodMetadata {
    /// Returns the raw JNI method ID for low-level ART/JNI operations.
    ///
    /// # Safety
    ///
    /// The ID is tied to the declaring class and VM that produced this metadata. The caller must
    /// not combine it with an unrelated receiver, class, VM, or forged signature.
    pub unsafe fn raw_id(&self) -> jni::jmethodID {
        self.id
    }
}

/// Metadata describing one Java field.
#[derive(Debug, Clone)]
pub struct JavaFieldMetadata {
    /// Field name.
    pub name: String,
    /// Whether the field is an instance field or static field.
    pub kind: FieldKind,
    /// Parsed Java field type.
    pub ty: JavaType,
    /// Java reflection modifier flags.
    pub modifiers: jni::jint,
    pub(crate) id: jni::jfieldID,
}

// JNI field IDs have the same VM-owned lifetime model as method IDs.
unsafe impl Send for JavaFieldMetadata {}
unsafe impl Sync for JavaFieldMetadata {}

impl JavaFieldMetadata {
    /// Returns the raw JNI field ID for low-level ART/JNI operations.
    ///
    /// # Safety
    ///
    /// The ID is tied to the declaring class and VM that produced this metadata. The caller must
    /// not combine it with an unrelated receiver, class, VM, or forged field type.
    pub unsafe fn raw_id(&self) -> jni::jfieldID {
        self.id
    }
}

/// Methods matching a query, grouped by class loader.
#[derive(Debug, Clone)]
pub struct JavaMethodQueryGroup {
    /// Loader shared by all classes in this group, or `None` for bootstrap classes.
    pub loader: Option<ClassLoaderRef>,
    /// Classes in this loader that matched the method query.
    pub classes: Vec<JavaMethodQueryClass>,
}

/// Methods matching a query within one Java class.
#[derive(Debug, Clone)]
pub struct JavaMethodQueryClass {
    /// Java binary class name, for example `java.lang.String`.
    pub name: String,
    /// Methods in this class that matched the query.
    pub methods: Vec<JavaMethodMetadata>,
}

pub(crate) fn class_metadata(
    env: &Env<'_>,
    vm: &Vm,
    class: &impl AsJObject,
) -> Result<JavaClassMetadata> {
    let reflection = reflection::Reflection::new(env)?;
    let descriptor = reflection.class_descriptor(class)?;
    let loader = reflection.class_loader(vm, class)?;
    Ok(JavaClassMetadata {
        name: class_name_from_descriptor(&descriptor),
        descriptor,
        loader,
    })
}

pub(crate) fn declared_methods(
    env: &Env<'_>,
    class: &impl AsJObject,
) -> Result<Vec<JavaMethodMetadata>> {
    let reflection = reflection::Reflection::new(env)?;
    reflection.declared_methods(class)
}

pub(crate) fn visible_methods(
    env: &Env<'_>,
    class: &impl AsJObject,
) -> Result<Vec<JavaMethodMetadata>> {
    let reflection = reflection::Reflection::new(env)?;
    reflection.visible_methods(class)
}

pub(crate) fn declared_fields(
    env: &Env<'_>,
    class: &impl AsJObject,
) -> Result<Vec<JavaFieldMetadata>> {
    let reflection = reflection::Reflection::new(env)?;
    reflection.declared_fields(class)
}

pub(crate) fn visible_fields(
    env: &Env<'_>,
    class: &impl AsJObject,
) -> Result<Vec<JavaFieldMetadata>> {
    let reflection = reflection::Reflection::new(env)?;
    reflection.visible_fields(class)
}
