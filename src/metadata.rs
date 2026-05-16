use std::collections::HashSet;

use crate::{
    env::{Env, FieldKind, MethodKind},
    error::{Error, Result},
    java::{ClassLoaderKind, ClassLoaderRef, Java, JavaClass},
    jni,
    refs::{AsJClass, AsJObject, LocalRef, ObjectArrayKind, ObjectArrayRef},
    signature::{JavaType, MethodSignature},
};

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

#[derive(Debug, Clone)]
pub struct JavaFieldMetadata {
    pub name: String,
    pub kind: FieldKind,
    pub ty: JavaType,
    pub modifiers: jni::jint,
    pub id: jni::jfieldID,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MethodQuery {
    pub(crate) class_pattern: String,
    pub(crate) method_pattern: String,
    pub(crate) include_signature: bool,
    pub(crate) ignore_case: bool,
    pub(crate) skip_system_classes: bool,
}

pub(crate) fn class_metadata(java: &Java, class: &JavaClass) -> Result<JavaClassMetadata> {
    let env = java.vm().attach_current_thread()?;
    let descriptor = class_descriptor(&env, class)?;
    let loader = class_loader(&env, java, class)?;
    Ok(JavaClassMetadata {
        name: class_name_from_descriptor(&descriptor),
        descriptor,
        loader,
    })
}

pub(crate) fn declared_methods(java: &Java, class: &JavaClass) -> Result<Vec<JavaMethodMetadata>> {
    let env = java.vm().attach_current_thread()?;
    let mut methods = Vec::new();
    let method_objects = call_class_object_array_method(
        &env,
        class,
        "getDeclaredMethods",
        "()[Ljava/lang/reflect/Method;",
    )?;
    for method in object_array_elements(&env, &method_objects)? {
        methods.push(method_metadata_from_reflection(
            &env,
            &method,
            MethodKind::Instance,
        )?);
    }

    let constructor_objects = call_class_object_array_method(
        &env,
        class,
        "getDeclaredConstructors",
        "()[Ljava/lang/reflect/Constructor;",
    )?;
    for constructor in object_array_elements(&env, &constructor_objects)? {
        methods.push(method_metadata_from_reflection(
            &env,
            &constructor,
            MethodKind::Constructor,
        )?);
    }

    methods.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.signature.to_string().cmp(&b.signature.to_string()))
            .then_with(|| (a.id as usize).cmp(&(b.id as usize)))
    });
    Ok(methods)
}

pub(crate) fn declared_fields(java: &Java, class: &JavaClass) -> Result<Vec<JavaFieldMetadata>> {
    let env = java.vm().attach_current_thread()?;
    let field_objects = call_class_object_array_method(
        &env,
        class,
        "getDeclaredFields",
        "()[Ljava/lang/reflect/Field;",
    )?;
    let mut fields = Vec::new();
    for field in object_array_elements(&env, &field_objects)? {
        fields.push(field_metadata_from_reflection(&env, &field)?);
    }

    fields.sort_by(|a, b| {
        a.name
            .cmp(&b.name)
            .then_with(|| a.ty.to_string().cmp(&b.ty.to_string()))
            .then_with(|| (a.id as usize).cmp(&(b.id as usize)))
    });
    Ok(fields)
}

pub(crate) fn enumerate_methods(
    java: &Java,
    classes: &[JavaClass],
    query: &str,
) -> Result<Vec<JavaMethodQueryGroup>> {
    let query = parse_method_query(query)?;
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
            let env = java.vm().attach_current_thread()?;
            loader = class_loader(&env, java, class)?;
            if loader.is_none() {
                continue;
            }
        }

        let mut seen = HashSet::new();
        let mut methods = Vec::new();
        for method in declared_methods(java, class)? {
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
            let env = java.vm().attach_current_thread()?;
            loader = class_loader(&env, java, class)?;
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

fn call_class_object_array_method<'env>(
    env: &'env Env<'_>,
    class: &JavaClass,
    name: &str,
    signature: &str,
) -> Result<ObjectArrayRef<'env>> {
    let class_class = env.find_class("java/lang/Class")?;
    let method = env.get_method(&class_class, name, signature)?;
    let array = env
        .call_object_method(class, &method, &[])?
        .ok_or(Error::NullReturn {
            operation: "java.lang.Class reflection array",
        })?;
    unsafe { LocalRef::<ObjectArrayKind>::from_raw(env, array.into_raw()) }
}

fn object_array_elements<'env>(
    env: &'env Env<'_>,
    array: &ObjectArrayRef<'env>,
) -> Result<Vec<crate::refs::ObjectRef<'env>>> {
    let length = env.object_array_length(array)?;
    let mut elements = Vec::with_capacity(length as usize);
    for index in 0..length {
        elements.push(env.object_array_element(array, index)?);
    }
    Ok(elements)
}

fn method_metadata_from_reflection(
    env: &Env<'_>,
    reflected: &impl AsJObject,
    fallback_kind: MethodKind,
) -> Result<JavaMethodMetadata> {
    let executable_class = env.get_object_class(reflected)?;
    let modifiers = call_int(env, &executable_class, reflected, "getModifiers", "()I")?;
    let name = if fallback_kind == MethodKind::Constructor {
        "<init>".to_owned()
    } else {
        call_string(
            env,
            &executable_class,
            reflected,
            "getName",
            "()Ljava/lang/String;",
        )?
    };
    let kind = if fallback_kind == MethodKind::Constructor {
        MethodKind::Constructor
    } else if modifiers & 0x0008 != 0 {
        MethodKind::Static
    } else {
        MethodKind::Instance
    };
    let parameters = call_class_array(env, &executable_class, reflected, "getParameterTypes")?;
    let return_type = if kind == MethodKind::Constructor {
        JavaType::Void
    } else {
        let return_class = call_object(
            env,
            &executable_class,
            reflected,
            "getReturnType",
            "()Ljava/lang/Class;",
        )?
        .ok_or(Error::NullReturn {
            operation: "java.lang.reflect.Method.getReturnType",
        })?;
        class_type(env, &return_class)?
    };
    let signature = MethodSignature::new(parameters, return_type);
    let method = env.from_reflected_method(reflected, kind, signature.clone())?;

    Ok(JavaMethodMetadata {
        name,
        kind,
        signature,
        modifiers,
        id: method.raw(),
    })
}

fn field_metadata_from_reflection(
    env: &Env<'_>,
    reflected: &impl AsJObject,
) -> Result<JavaFieldMetadata> {
    let field_class = env.get_object_class(reflected)?;
    let modifiers = call_int(env, &field_class, reflected, "getModifiers", "()I")?;
    let name = call_string(
        env,
        &field_class,
        reflected,
        "getName",
        "()Ljava/lang/String;",
    )?;
    let kind = if modifiers & 0x0008 != 0 {
        FieldKind::Static
    } else {
        FieldKind::Instance
    };
    let ty_class = call_object(
        env,
        &field_class,
        reflected,
        "getType",
        "()Ljava/lang/Class;",
    )?
    .ok_or(Error::NullReturn {
        operation: "java.lang.reflect.Field.getType",
    })?;
    let ty = class_type(env, &ty_class)?;
    let field = env.from_reflected_field(reflected, kind, ty.clone())?;

    Ok(JavaFieldMetadata {
        name,
        kind,
        ty,
        modifiers,
        id: field.raw(),
    })
}

fn call_class_array(
    env: &Env<'_>,
    class: &impl AsJClass,
    object: &impl AsJObject,
    name: &str,
) -> Result<Vec<JavaType>> {
    let parameters = call_object(env, class, object, name, "()[Ljava/lang/Class;")?.ok_or(
        Error::NullReturn {
            operation: "java.lang.reflect.Executable.getParameterTypes",
        },
    )?;
    let parameters = unsafe { LocalRef::<ObjectArrayKind>::from_raw(env, parameters.into_raw())? };
    object_array_elements(env, &parameters)?
        .iter()
        .map(|parameter| class_type(env, parameter))
        .collect()
}

pub(crate) fn class_descriptor(env: &Env<'_>, class: &impl AsJObject) -> Result<String> {
    let class_class = env.find_class("java/lang/Class")?;
    let name = call_string(env, &class_class, class, "getName", "()Ljava/lang/String;")?;
    Ok(class_name_to_descriptor(&name))
}

fn class_type(env: &Env<'_>, class: &impl AsJObject) -> Result<JavaType> {
    let descriptor = class_descriptor(env, class)?;
    if descriptor == "V" {
        Ok(JavaType::Void)
    } else {
        JavaType::parse(&descriptor)
    }
}

fn class_loader(
    env: &Env<'_>,
    java: &Java,
    class: &impl AsJObject,
) -> Result<Option<ClassLoaderRef>> {
    let class_class = env.find_class("java/lang/Class")?;
    let loader = call_object(
        env,
        &class_class,
        class,
        "getClassLoader",
        "()Ljava/lang/ClassLoader;",
    )?;
    loader
        .as_ref()
        .map(|loader| {
            ClassLoaderRef::from_object_ref(env, java.vm(), loader, ClassLoaderKind::Object)
        })
        .transpose()
}

fn call_string(
    env: &Env<'_>,
    class: &impl AsJClass,
    object: &impl AsJObject,
    name: &str,
    signature: &str,
) -> Result<String> {
    let value = call_object(env, class, object, name, signature)?.ok_or(Error::NullReturn {
        operation: "reflection string method",
    })?;
    unsafe { env.get_string_raw(value.as_jobject()) }
}

fn call_object<'env>(
    env: &'env Env<'_>,
    class: &impl AsJClass,
    object: &impl AsJObject,
    name: &str,
    signature: &str,
) -> Result<Option<crate::refs::ObjectRef<'env>>> {
    let method = env.get_method(class, name, signature)?;
    env.call_object_method(object, &method, &[])
}

fn call_int(
    env: &Env<'_>,
    class: &impl AsJClass,
    object: &impl AsJObject,
    name: &str,
    signature: &str,
) -> Result<jni::jint> {
    let method = env.get_method(class, name, signature)?;
    env.call_int_method(object, &method, &[])
}

fn class_name_to_descriptor(name: &str) -> String {
    match name {
        "boolean" => "Z".to_owned(),
        "byte" => "B".to_owned(),
        "char" => "C".to_owned(),
        "short" => "S".to_owned(),
        "int" => "I".to_owned(),
        "long" => "J".to_owned(),
        "float" => "F".to_owned(),
        "double" => "D".to_owned(),
        "void" => "V".to_owned(),
        _ if name.starts_with('[') => name.replace('.', "/"),
        _ => format!("L{};", name.replace('.', "/")),
    }
}

fn class_name_from_descriptor(descriptor: &str) -> String {
    if descriptor.starts_with('L') && descriptor.ends_with(';') {
        descriptor[1..descriptor.len() - 1].replace('/', ".")
    } else {
        descriptor.replace('/', ".")
    }
}

pub(crate) fn parse_method_query(query: &str) -> Result<MethodQuery> {
    let Some((class_pattern, rest)) = query.split_once('!') else {
        return Err(Error::InvalidQuery {
            query: query.to_owned(),
            message: "expected class!method query",
        });
    };
    if class_pattern.is_empty() {
        return Err(Error::InvalidQuery {
            query: query.to_owned(),
            message: "class pattern cannot be empty",
        });
    }

    let (method_pattern, modifiers) = if let Some((method, modifiers)) = rest.rsplit_once('/') {
        if modifiers.chars().all(|ch| matches!(ch, 'i' | 's' | 'u')) {
            (method, modifiers)
        } else {
            (rest, "")
        }
    } else {
        (rest, "")
    };
    if method_pattern.is_empty() {
        return Err(Error::InvalidQuery {
            query: query.to_owned(),
            message: "method pattern cannot be empty",
        });
    }

    let ignore_case = modifiers.contains('i');
    Ok(MethodQuery {
        class_pattern: normalize_case(class_pattern, ignore_case),
        method_pattern: normalize_case(method_pattern, ignore_case),
        include_signature: modifiers.contains('s'),
        ignore_case,
        skip_system_classes: modifiers.contains('u'),
    })
}

pub(crate) fn query_method_name(method: &JavaMethodMetadata, include_signature: bool) -> String {
    let name = if method.kind == MethodKind::Constructor {
        "$init"
    } else {
        &method.name
    };
    if include_signature {
        format!("{name}{}", method.signature)
    } else {
        name.to_owned()
    }
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

pub(crate) fn normalize_case(value: &str, ignore_case: bool) -> String {
    if ignore_case {
        value.to_ascii_lowercase()
    } else {
        value.to_owned()
    }
}

pub(crate) fn is_platform_class(name: &str) -> bool {
    name.starts_with("java.")
        || name.starts_with("javax.")
        || name.starts_with("android.")
        || name.starts_with("androidx.")
        || name.starts_with("dalvik.")
        || name.starts_with("com.android.")
}

pub(crate) fn glob_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    let (mut p, mut v) = (0, 0);
    let mut star = None;
    let mut star_value = 0;

    while v < value.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == value[v]) {
            p += 1;
            v += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            star_value = v;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            star_value += 1;
            v = star_value;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_reflection_class_names_to_descriptors() {
        assert_eq!(class_name_to_descriptor("int"), "I");
        assert_eq!(
            class_name_to_descriptor("java.lang.String"),
            "Ljava/lang/String;"
        );
        assert_eq!(
            class_name_to_descriptor("[Ljava.lang.String;"),
            "[Ljava/lang/String;"
        );
    }

    #[test]
    fn converts_descriptors_to_public_dotted_names() {
        assert_eq!(
            class_name_from_descriptor("Ljava/lang/String;"),
            "java.lang.String"
        );
        assert_eq!(
            class_name_from_descriptor("[Ljava/lang/String;"),
            "[Ljava.lang.String;"
        );
        assert_eq!(class_name_from_descriptor("[I"), "[I");
    }

    #[test]
    fn parses_method_query_flags() {
        assert_eq!(
            parse_method_query("com.example.*!foo*/isu"),
            Ok(MethodQuery {
                class_pattern: "com.example.*".to_owned(),
                method_pattern: "foo*".to_owned(),
                include_signature: true,
                ignore_case: true,
                skip_system_classes: true,
            })
        );
    }

    #[test]
    fn rejects_method_queries_missing_required_parts() {
        assert_eq!(
            parse_method_query("com.example.*").unwrap_err(),
            Error::InvalidQuery {
                query: "com.example.*".to_owned(),
                message: "expected class!method query",
            }
        );
        assert_eq!(
            parse_method_query("!foo").unwrap_err(),
            Error::InvalidQuery {
                query: "!foo".to_owned(),
                message: "class pattern cannot be empty",
            }
        );
        assert_eq!(
            parse_method_query("com.example.*!").unwrap_err(),
            Error::InvalidQuery {
                query: "com.example.*!".to_owned(),
                message: "method pattern cannot be empty",
            }
        );
    }

    #[test]
    fn treats_unknown_query_suffix_as_part_of_method_pattern() {
        assert_eq!(
            parse_method_query("com.example.*!foo/bar"),
            Ok(MethodQuery {
                class_pattern: "com.example.*".to_owned(),
                method_pattern: "foo/bar".to_owned(),
                include_signature: false,
                ignore_case: false,
                skip_system_classes: false,
            })
        );
    }

    #[test]
    fn normalizes_case_when_query_is_case_insensitive() {
        assert_eq!(
            parse_method_query("Com.Example.*!Foo*/i"),
            Ok(MethodQuery {
                class_pattern: "com.example.*".to_owned(),
                method_pattern: "foo*".to_owned(),
                include_signature: false,
                ignore_case: true,
                skip_system_classes: false,
            })
        );
    }

    #[test]
    fn matches_simple_globs() {
        assert!(glob_matches("foo*", "foobar"));
        assert!(glob_matches("f?o", "foo"));
        assert!(!glob_matches("foo", "foobar"));
    }

    #[test]
    fn identifies_platform_classes_for_user_queries() {
        assert!(is_platform_class("java.lang.String"));
        assert!(is_platform_class("android.os.Process"));
        assert!(!is_platform_class(
            "frida.java.bridge.rs.test.TestSubject"
        ));
    }

    #[test]
    fn formats_query_constructor_names() {
        let method = JavaMethodMetadata {
            name: "<init>".to_owned(),
            kind: MethodKind::Constructor,
            signature: MethodSignature::parse("(I)V").unwrap(),
            modifiers: 0,
            id: std::ptr::dangling_mut(),
        };
        assert_eq!(query_method_name(&method, false), "$init");
        assert_eq!(query_method_name(&method, true), "$init(I)V");
    }
}
