mod query;
mod reflection;

use std::collections::HashSet;

use crate::{
    env::{FieldKind, MethodKind},
    error::Result,
    java::{ClassLoaderRef, Java, raw},
    jni,
    refs::AsJObject,
    signature::{JavaType, MethodSignature},
};

pub(crate) use query::{
    MethodQuery, glob_matches, is_platform_class, normalize_case, parse_method_query,
    query_method_name,
};
pub(crate) use reflection::{class_descriptor, class_loader, class_name_from_descriptor};

#[derive(Debug, Clone)]
pub struct JavaClassMetadata {
    /// Java binary class name, for example `java.lang.String`.
    ///
    /// Array names follow `Class.getName()` style, for example `[Ljava.lang.String;`.
    pub name: String,
    /// JNI descriptor, for example `Ljava/lang/String;`.
    pub descriptor: String,
    pub loader: Option<ClassLoaderRef>,
}

#[derive(Debug, Clone)]
pub struct JavaMethodMetadata {
    pub name: String,
    pub kind: MethodKind,
    pub signature: MethodSignature,
    pub modifiers: jni::jint,
    pub id: jni::jmethodID,
}

// JNI method IDs are VM-owned opaque identifiers. They are not thread-affine; callers still need a
// valid thread-local JNIEnv to invoke them.
unsafe impl Send for JavaMethodMetadata {}
unsafe impl Sync for JavaMethodMetadata {}

#[derive(Debug, Clone)]
pub struct JavaFieldMetadata {
    pub name: String,
    pub kind: FieldKind,
    pub ty: JavaType,
    pub modifiers: jni::jint,
    pub id: jni::jfieldID,
}

// JNI field IDs have the same VM-owned lifetime model as method IDs.
unsafe impl Send for JavaFieldMetadata {}
unsafe impl Sync for JavaFieldMetadata {}

#[derive(Debug, Clone)]
pub struct JavaMethodQueryGroup {
    pub loader: Option<ClassLoaderRef>,
    pub classes: Vec<JavaMethodQueryClass>,
}

#[derive(Debug, Clone)]
pub struct JavaMethodQueryClass {
    /// Java binary class name, for example `java.lang.String`.
    pub name: String,
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
            let display_name = query_method_name(&method, query.include_signature);
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
