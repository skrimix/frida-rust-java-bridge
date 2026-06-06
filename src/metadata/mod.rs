//! Metadata returned by Java class, method, field, and query operations.
//!
//! These types describe Java declarations in Rust-friendly form. They are snapshots of what the
//! runtime reported at query time; invoking methods or reading fields still goes through the
//! high-level wrappers unless you explicitly opt into raw JNI IDs.

mod reflection;

use std::collections::HashSet;

use crate::{
    env::{FieldKind, MethodKind},
    error::Result,
    java::{ClassLoaderRef, Java, raw},
    jni,
    method_query::{
        glob_matches, is_platform_class, normalize_case, parse_method_query, query_method_name,
    },
    refs::AsJObject,
    signature::{JavaType, MethodSignature},
};

pub(crate) use crate::signature::class_name_from_descriptor;
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

pub(crate) fn class_metadata(java: &Java, class: &raw::Class) -> Result<JavaClassMetadata> {
    let env = java.vm().attach_current_thread()?;
    let reflection = reflection::Reflection::new(&env)?;
    let descriptor = reflection.class_descriptor(class)?;
    let loader = reflection.class_loader(java, class)?;
    Ok(JavaClassMetadata {
        name: class_name_from_descriptor(&descriptor),
        descriptor,
        loader,
    })
}

pub(crate) fn declared_methods(java: &Java, class: &raw::Class) -> Result<Vec<JavaMethodMetadata>> {
    let env = java.vm().attach_current_thread()?;
    let reflection = reflection::Reflection::new(&env)?;
    reflection.declared_methods(class)
}

pub(crate) fn visible_methods(java: &Java, class: &raw::Class) -> Result<Vec<JavaMethodMetadata>> {
    let env = java.vm().attach_current_thread()?;
    let reflection = reflection::Reflection::new(&env)?;
    reflection.visible_methods(class)
}

pub(crate) fn declared_fields(java: &Java, class: &raw::Class) -> Result<Vec<JavaFieldMetadata>> {
    let env = java.vm().attach_current_thread()?;
    let reflection = reflection::Reflection::new(&env)?;
    reflection.declared_fields(class)
}

pub(crate) fn visible_fields(java: &Java, class: &raw::Class) -> Result<Vec<JavaFieldMetadata>> {
    let env = java.vm().attach_current_thread()?;
    let reflection = reflection::Reflection::new(&env)?;
    reflection.visible_fields(class)
}

pub(crate) fn enumerate_methods(
    java: &Java,
    classes: &[raw::Class],
    query: &str,
) -> Result<Vec<JavaMethodQueryGroup>> {
    let query = parse_method_query(query)?;
    let env = java.vm().attach_current_thread()?;
    let reflection = reflection::Reflection::new(&env)?;
    let mut groups: Vec<JavaMethodQueryGroup> = Vec::new();

    for class in classes {
        let class_name = class.name();
        if query.skip_system_classes && is_platform_class(class_name) {
            continue;
        }

        let class_match_name = normalize_case(class_name, query.ignore_case);
        if !glob_matches(&query.class_pattern, &class_match_name) {
            continue;
        }

        let mut loader = None;
        if query.skip_system_classes {
            loader = reflection.class_loader(java, class)?;
            if loader.is_none() {
                continue;
            }
        }

        let mut seen = HashSet::new();
        let mut methods = Vec::new();
        for method in reflection.declared_methods(class)? {
            if method.name == "<clinit>" {
                continue;
            }
            let display_name = query_method_name(
                method.kind,
                &method.name,
                &method.signature,
                query.include_signature,
            );
            if !query.include_signature && !seen.insert(display_name.clone()) {
                continue;
            }

            let method_match_name = normalize_case(&display_name, query.ignore_case);
            if glob_matches(&query.method_pattern, &method_match_name) {
                methods.push(method);
            }
        }

        if methods.is_empty() {
            continue;
        }

        if loader.is_none() {
            loader = reflection.class_loader(java, class)?;
        }

        let group_index = find_group(&groups, loader.as_ref());
        let class_group = JavaMethodQueryClass {
            name: class_name.to_owned(),
            methods,
        };
        if let Some(index) = group_index {
            groups[index].classes.push(class_group);
        } else {
            groups.push(JavaMethodQueryGroup {
                loader,
                classes: vec![class_group],
            });
        }
    }

    Ok(groups)
}

fn find_group(groups: &[JavaMethodQueryGroup], loader: Option<&ClassLoaderRef>) -> Option<usize> {
    groups
        .iter()
        .position(|group| match (&group.loader, loader) {
            (None, None) => true,
            (Some(a), Some(b)) => a.as_jobject() == b.as_jobject(),
            _ => false,
        })
}
