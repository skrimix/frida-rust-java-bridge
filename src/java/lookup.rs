use super::*;

pub(super) fn find_class_with_loader<'env, 'vm>(
    env: &'env Env<'vm>,
    loader: &ClassLoaderRef,
    lookup: &ClassLookupName,
) -> Result<ClassRef<'env>> {
    if lookup.is_array_descriptor {
        let class_class = env.find_class("java/lang/Class")?;
        let for_name = env.lookup_static_method(
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
                    JavaValue::object_ref(loader.as_jobject()),
                ],
            )?
            .ok_or(Error::NullReturn {
                operation: "Class.forName",
            })?;
        unsafe { LocalRef::from_raw(env, class.into_raw()) }
    } else {
        let class_loader_class = env.find_class("java/lang/ClassLoader")?;
        let load_class = env.lookup_instance_method(
            &class_loader_class,
            "loadClass",
            "(Ljava/lang/String;)Ljava/lang/Class;",
        )?;
        let name = env.new_string_utf(&lookup.loader_name)?;
        let class = env
            .call_instance_object_method(loader, &load_class, &[JavaValue::from(&name)])?
            .ok_or(Error::NullReturn {
                operation: "ClassLoader.loadClass",
            })?;
        unsafe { LocalRef::from_raw(env, class.into_raw()) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClassLookupName {
    pub(super) find_class_name: String,
    pub(super) loader_name: String,
    pub(super) is_array_descriptor: bool,
}

pub(super) fn normalize_class_lookup_name(name: &str) -> ClassLookupName {
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
    ClassLookupName {
        find_class_name,
        loader_name,
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
        assert_eq!(dotted.loader_name, "java.lang.String");

        let internal = normalize_class_lookup_name("java/lang/String");
        assert_eq!(internal.find_class_name, "java/lang/String");
        assert_eq!(internal.loader_name, "java.lang.String");

        let descriptor = normalize_class_lookup_name("Ljava/lang/String;");
        assert_eq!(descriptor.find_class_name, "java/lang/String");
        assert_eq!(descriptor.loader_name, "java.lang.String");

        let dotted_descriptor = normalize_class_lookup_name("Ljava.lang.String;");
        assert_eq!(dotted_descriptor.find_class_name, "java/lang/String");
        assert_eq!(dotted_descriptor.loader_name, "java.lang.String");

        let inner = normalize_class_lookup_name("com.example.Outer$Inner");
        assert_eq!(inner.find_class_name, "com/example/Outer$Inner");
        assert_eq!(inner.loader_name, "com.example.Outer$Inner");
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
        assert!(object.is_array_descriptor);

        let dotted = normalize_class_lookup_name("[Ljava.lang.String;");
        assert_eq!(dotted.find_class_name, "[Ljava/lang/String;");
        assert_eq!(dotted.loader_name, "[Ljava.lang.String;");
    }

    #[test]
    fn normalizes_multi_dimensional_array_descriptors() {
        let object = normalize_class_lookup_name("[[Ljava/lang/String;");
        assert_eq!(object.find_class_name, "[[Ljava/lang/String;");
        assert_eq!(object.loader_name, "[[Ljava.lang.String;");
        assert!(object.is_array_descriptor);

        let primitive = normalize_class_lookup_name("[[I");
        assert_eq!(primitive.find_class_name, "[[I");
        assert_eq!(primitive.loader_name, "[[I");
        assert!(primitive.is_array_descriptor);
    }

    #[test]
    fn preserves_inner_class_binary_names() {
        let lookup = normalize_class_lookup_name("Lcom.example.Outer$Inner;");
        assert_eq!(lookup.find_class_name, "com/example/Outer$Inner");
        assert_eq!(lookup.loader_name, "com.example.Outer$Inner");
        assert!(!lookup.is_array_descriptor);
    }
}
