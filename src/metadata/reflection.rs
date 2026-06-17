use std::collections::HashSet;

/// Java reflection modifier flag constants.
///
/// These values match the bit flags returned by Java reflection for classes, methods, and fields.
/// Use them to inspect metadata returned by the high-level Java facade.
pub mod modifiers {
    use crate::jni;

    pub const ACC_PUBLIC: jni::jint = 0x0001;
    pub const ACC_PRIVATE: jni::jint = 0x0002;
    pub const ACC_PROTECTED: jni::jint = 0x0004;
    pub const ACC_STATIC: jni::jint = 0x0008;
    pub const ACC_FINAL: jni::jint = 0x0010;
    pub const ACC_SYNCHRONIZED: jni::jint = 0x0020;
    pub const ACC_BRIDGE: jni::jint = 0x0040;
    pub const ACC_VARARGS: jni::jint = 0x0080;
    pub const ACC_NATIVE: jni::jint = 0x0100;
    pub const ACC_ABSTRACT: jni::jint = 0x0400;
    pub const ACC_STRICT: jni::jint = 0x0800;
    pub const ACC_SYNTHETIC: jni::jint = 0x1000;

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn modifier_constants_match_java_reflection_flags() {
            assert_eq!(ACC_PUBLIC, 0x0001);
            assert_eq!(ACC_PRIVATE, 0x0002);
            assert_eq!(ACC_PROTECTED, 0x0004);
            assert_eq!(ACC_STATIC, 0x0008);
            assert_eq!(ACC_FINAL, 0x0010);
            assert_eq!(ACC_SYNCHRONIZED, 0x0020);
            assert_eq!(ACC_BRIDGE, 0x0040);
            assert_eq!(ACC_VARARGS, 0x0080);
            assert_eq!(ACC_NATIVE, 0x0100);
            assert_eq!(ACC_ABSTRACT, 0x0400);
            assert_eq!(ACC_STRICT, 0x0800);
            assert_eq!(ACC_SYNTHETIC, 0x1000);
        }
    }
}

use crate::{
    env::{Env, FieldKind, MethodKind},
    error::{Error, Result},
    jni,
    loader::{ClassLoaderKind, ClassLoaderRef},
    refs::{AsJClass, AsJObject, ClassRef, LocalRef, ObjectArrayKind, ObjectArrayRef},
    signature::{JavaType, MethodSignature},
    vm::Vm,
};

use self::modifiers::ACC_STATIC;
use super::{JavaFieldMetadata, JavaMethodMetadata};

pub(super) struct Reflection<'env, 'scope> {
    env: &'env Env<'scope>,
    class_class: ClassRef<'env>,
}

impl<'env, 'scope> Reflection<'env, 'scope> {
    pub(super) fn new(env: &'env Env<'scope>) -> Result<Self> {
        Ok(Self {
            env,
            class_class: env.find_class("java/lang/Class")?,
        })
    }

    pub(super) fn declared_methods(
        &self,
        class: &impl AsJObject,
    ) -> Result<Vec<JavaMethodMetadata>> {
        let mut methods = Vec::new();
        let method_objects = self.call_class_object_array_method(
            class,
            "getDeclaredMethods",
            "()[Ljava/lang/reflect/Method;",
        )?;
        for method in object_array_elements(self.env, &method_objects)? {
            methods.push(self.method_metadata_from_reflection(&method, MethodKind::Instance)?);
        }

        let constructor_objects = self.call_class_object_array_method(
            class,
            "getDeclaredConstructors",
            "()[Ljava/lang/reflect/Constructor;",
        )?;
        for constructor in object_array_elements(self.env, &constructor_objects)? {
            methods
                .push(self.method_metadata_from_reflection(&constructor, MethodKind::Constructor)?);
        }

        methods.sort_by(sort_methods);
        Ok(methods)
    }

    pub(super) fn visible_methods(
        &self,
        class: &impl AsJObject,
    ) -> Result<Vec<JavaMethodMetadata>> {
        let mut declared = self
            .declared_method_metadata_for_class(class)?
            .into_iter()
            .filter(|method| method.kind != MethodKind::Constructor)
            .collect::<Vec<_>>();
        declared.sort_by(sort_methods);
        let declared_names = method_names(&declared);
        let mut methods = declared;

        if let Some(superclass) = self.class_superclass(class)? {
            append_unshadowed_methods(
                &mut methods,
                &declared_names,
                self.visible_methods(&superclass)?,
            );
        }

        Ok(methods)
    }

    pub(super) fn declared_fields(&self, class: &impl AsJObject) -> Result<Vec<JavaFieldMetadata>> {
        let mut fields = self.declared_field_metadata_for_class(class)?;
        fields.sort_by(sort_fields);
        Ok(fields)
    }

    pub(super) fn visible_fields(&self, class: &impl AsJObject) -> Result<Vec<JavaFieldMetadata>> {
        let mut declared = self.declared_field_metadata_for_class(class)?;
        declared.sort_by(sort_fields);
        let declared_names = field_names(&declared);
        let mut fields = declared;

        if let Some(superclass) = self.class_superclass(class)? {
            append_unshadowed_fields(
                &mut fields,
                &declared_names,
                self.visible_fields(&superclass)?,
            );
        }

        Ok(fields)
    }

    fn call_class_object_array_method(
        &self,
        class: &impl AsJObject,
        name: &str,
        signature: &str,
    ) -> Result<ObjectArrayRef<'_>> {
        let method = self
            .env
            .lookup_instance_method(&self.class_class, name, signature)?;
        // SAFETY: `method` was resolved from `java.lang.Class`, and `class` is a class object.
        let array = unsafe { self.env.call_instance_object_method(class, &method, &[])? }.ok_or(
            Error::NullReturn {
                operation: "java.lang.Class reflection array",
            },
        )?;
        unsafe {
            LocalRef::<ObjectArrayKind>::from_raw(self.env.local_ref_scope(), array.into_raw())
        }
    }

    fn declared_method_metadata_for_class(
        &self,
        class: &impl AsJObject,
    ) -> Result<Vec<JavaMethodMetadata>> {
        let method_objects = self.call_class_object_array_method(
            class,
            "getDeclaredMethods",
            "()[Ljava/lang/reflect/Method;",
        )?;
        object_array_elements(self.env, &method_objects)?
            .iter()
            .map(|method| self.method_metadata_from_reflection(method, MethodKind::Instance))
            .collect()
    }

    fn declared_field_metadata_for_class(
        &self,
        class: &impl AsJObject,
    ) -> Result<Vec<JavaFieldMetadata>> {
        let field_objects = self.call_class_object_array_method(
            class,
            "getDeclaredFields",
            "()[Ljava/lang/reflect/Field;",
        )?;
        object_array_elements(self.env, &field_objects)?
            .iter()
            .map(|field| self.field_metadata_from_reflection(field))
            .collect()
    }

    fn class_superclass(
        &self,
        class: &impl AsJObject,
    ) -> Result<Option<crate::refs::ObjectRef<'_>>> {
        self.call_object(class, "getSuperclass", "()Ljava/lang/Class;")
    }

    fn method_metadata_from_reflection(
        &self,
        reflected: &impl AsJObject,
        fallback_kind: MethodKind,
    ) -> Result<JavaMethodMetadata> {
        let executable_class = self.env.get_object_class(reflected)?;
        let modifiers = self.call_int(&executable_class, reflected, "getModifiers", "()I")?;
        let name = if fallback_kind == MethodKind::Constructor {
            "<init>".to_owned()
        } else {
            self.call_string(
                &executable_class,
                reflected,
                "getName",
                "()Ljava/lang/String;",
            )?
        };
        let kind = if fallback_kind == MethodKind::Constructor {
            MethodKind::Constructor
        } else if modifiers & ACC_STATIC != 0 {
            MethodKind::Static
        } else {
            MethodKind::Instance
        };
        let parameters =
            self.call_class_array(&executable_class, reflected, "getParameterTypes")?;
        let return_type = if kind == MethodKind::Constructor {
            JavaType::Void
        } else {
            let return_class = self
                .call_object_with_class(
                    &executable_class,
                    reflected,
                    "getReturnType",
                    "()Ljava/lang/Class;",
                )?
                .ok_or(Error::NullReturn {
                    operation: "java.lang.reflect.Method.getReturnType",
                })?;
            self.class_type(&return_class)?
        };
        let signature = MethodSignature::new(parameters, return_type);
        // SAFETY: `kind` and `signature` were derived from this reflected executable immediately
        // above using Java reflection.
        let method = unsafe {
            self.env
                .from_reflected_method(reflected, kind, signature.clone())?
        };

        Ok(JavaMethodMetadata {
            name,
            kind,
            signature,
            modifiers,
            id: unsafe { method.raw() },
        })
    }

    fn field_metadata_from_reflection(
        &self,
        reflected: &impl AsJObject,
    ) -> Result<JavaFieldMetadata> {
        let field_class = self.env.get_object_class(reflected)?;
        let modifiers = self.call_int(&field_class, reflected, "getModifiers", "()I")?;
        let name = self.call_string(&field_class, reflected, "getName", "()Ljava/lang/String;")?;
        let kind = if modifiers & ACC_STATIC != 0 {
            FieldKind::Static
        } else {
            FieldKind::Instance
        };
        let ty_class = self
            .call_object_with_class(&field_class, reflected, "getType", "()Ljava/lang/Class;")?
            .ok_or(Error::NullReturn {
                operation: "java.lang.reflect.Field.getType",
            })?;
        let ty = self.class_type(&ty_class)?;
        // SAFETY: `kind` and `ty` were derived from this reflected field immediately above using
        // Java reflection.
        let field = unsafe { self.env.from_reflected_field(reflected, kind, ty.clone())? };

        Ok(JavaFieldMetadata {
            name,
            kind,
            ty,
            modifiers,
            id: unsafe { field.raw() },
        })
    }

    fn call_class_array(
        &self,
        class: &impl AsJClass,
        object: &impl AsJObject,
        name: &str,
    ) -> Result<Vec<JavaType>> {
        let parameters = self
            .call_object_with_class(class, object, name, "()[Ljava/lang/Class;")?
            .ok_or(Error::NullReturn {
                operation: "java.lang.reflect.Executable.getParameterTypes",
            })?;
        let parameters = unsafe {
            LocalRef::<ObjectArrayKind>::from_raw(
                self.env.local_ref_scope(),
                parameters.into_raw(),
            )?
        };
        object_array_elements(self.env, &parameters)?
            .iter()
            .map(|parameter| self.class_type(parameter))
            .collect()
    }

    pub(super) fn class_descriptor(&self, class: &impl AsJObject) -> Result<String> {
        let name = self.call_string(
            &self.class_class,
            class,
            "getNameNative",
            "()Ljava/lang/String;",
        )?;
        Ok(class_name_to_descriptor(&name))
    }

    fn class_type(&self, class: &impl AsJObject) -> Result<JavaType> {
        let descriptor = self.class_descriptor(class)?;
        if descriptor == "V" {
            Ok(JavaType::Void)
        } else {
            JavaType::parse(&descriptor)
        }
    }

    pub(super) fn class_loader(
        &self,
        vm: &Vm,
        class: &impl AsJObject,
    ) -> Result<Option<ClassLoaderRef>> {
        let loader = self.call_object(class, "getClassLoader", "()Ljava/lang/ClassLoader;")?;
        loader
            .as_ref()
            .map(|loader| {
                ClassLoaderRef::from_object_ref(self.env, vm, loader, ClassLoaderKind::Object)
            })
            .transpose()
    }

    fn call_string(
        &self,
        class: &impl AsJClass,
        object: &impl AsJObject,
        name: &str,
        signature: &str,
    ) -> Result<String> {
        let value = self
            .call_object_with_class(class, object, name, signature)?
            .ok_or(Error::NullReturn {
                operation: "reflection string method",
            })?;
        unsafe { self.env.get_string_raw(value.as_jobject()) }
    }

    fn call_object(
        &self,
        object: &impl AsJObject,
        name: &str,
        signature: &str,
    ) -> Result<Option<crate::refs::ObjectRef<'_>>> {
        self.call_object_with_class(&self.class_class, object, name, signature)
    }

    fn call_object_with_class(
        &self,
        class: &impl AsJClass,
        object: &impl AsJObject,
        name: &str,
        signature: &str,
    ) -> Result<Option<crate::refs::ObjectRef<'_>>> {
        let method = self.env.lookup_instance_method(class, name, signature)?;
        // SAFETY: callers pass the declaring reflection class for this receiver immediately before
        // invoking the selected reflection helper.
        unsafe { self.env.call_instance_object_method(object, &method, &[]) }
    }

    fn call_int(
        &self,
        class: &impl AsJClass,
        object: &impl AsJObject,
        name: &str,
        signature: &str,
    ) -> Result<jni::jint> {
        let method = self.env.lookup_instance_method(class, name, signature)?;
        // SAFETY: callers pass the declaring reflection class for this receiver immediately before
        // invoking the selected reflection helper.
        unsafe { self.env.call_instance_int_method(object, &method, &[]) }
    }
}

pub(crate) fn class_descriptor(env: &Env<'_>, class: &impl AsJObject) -> Result<String> {
    Reflection::new(env)?.class_descriptor(class)
}

pub(crate) fn class_loader(
    env: &Env<'_>,
    vm: &Vm,
    class: &impl AsJObject,
) -> Result<Option<ClassLoaderRef>> {
    Reflection::new(env)?.class_loader(vm, class)
}

fn object_array_elements<'env>(
    env: &'env Env<'_>,
    array: &ObjectArrayRef<'env>,
) -> Result<Vec<crate::refs::ObjectRef<'env>>> {
    let length = env.array_length(array)?;
    let mut elements = Vec::with_capacity(length as usize);
    for index in 0..length {
        elements.push(env.get_object_array_element(array, index)?);
    }
    Ok(elements)
}

fn method_names(methods: &[JavaMethodMetadata]) -> HashSet<String> {
    methods.iter().map(|method| method.name.clone()).collect()
}

fn field_names(fields: &[JavaFieldMetadata]) -> HashSet<String> {
    fields.iter().map(|field| field.name.clone()).collect()
}

fn append_unshadowed_methods(
    methods: &mut Vec<JavaMethodMetadata>,
    declared_names: &HashSet<String>,
    inherited: Vec<JavaMethodMetadata>,
) {
    for method in inherited {
        if !declared_names.contains(&method.name) {
            methods.push(method);
        }
    }
}

fn append_unshadowed_fields(
    fields: &mut Vec<JavaFieldMetadata>,
    declared_names: &HashSet<String>,
    inherited: Vec<JavaFieldMetadata>,
) {
    for field in inherited {
        if !declared_names.contains(&field.name) {
            fields.push(field);
        }
    }
}

fn sort_methods(a: &JavaMethodMetadata, b: &JavaMethodMetadata) -> std::cmp::Ordering {
    a.name
        .cmp(&b.name)
        .then_with(|| a.signature.descriptor().cmp(&b.signature.descriptor()))
        .then_with(|| (a.id as usize).cmp(&(b.id as usize)))
}

fn sort_fields(a: &JavaFieldMetadata, b: &JavaFieldMetadata) -> std::cmp::Ordering {
    a.name
        .cmp(&b.name)
        .then_with(|| a.ty.descriptor().cmp(&b.ty.descriptor()))
        .then_with(|| (a.id as usize).cmp(&(b.id as usize)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn method(name: &str, kind: MethodKind, signature: &str) -> JavaMethodMetadata {
        JavaMethodMetadata {
            name: name.to_owned(),
            kind,
            signature: MethodSignature::parse(signature).unwrap(),
            modifiers: 0,
            id: std::ptr::dangling_mut(),
        }
    }

    fn field(name: &str, kind: FieldKind, ty: &str) -> JavaFieldMetadata {
        JavaFieldMetadata {
            name: name.to_owned(),
            kind,
            ty: JavaType::parse(ty).unwrap(),
            modifiers: 0,
            id: std::ptr::dangling_mut(),
        }
    }

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
    fn visible_method_collection_keeps_declared_name_shadowing() {
        let mut methods = vec![method(
            "value",
            MethodKind::Instance,
            "()Ljava/lang/String;",
        )];
        let declared_names = method_names(&methods);

        append_unshadowed_methods(
            &mut methods,
            &declared_names,
            vec![
                method("value", MethodKind::Instance, "(I)Ljava/lang/Object;"),
                method("value", MethodKind::Static, "()I"),
                method("baseValue", MethodKind::Static, "()I"),
            ],
        );

        assert_eq!(methods.len(), 2);
        assert_eq!(methods[0].name, "value");
        assert_eq!(methods[0].signature.descriptor(), "()Ljava/lang/String;");
        assert_eq!(methods[1].name, "baseValue");
        assert_eq!(methods[1].kind, MethodKind::Static);
    }

    #[test]
    fn visible_field_collection_keeps_declared_name_shadowing() {
        let mut fields = vec![field("number", FieldKind::Static, "I")];
        let declared_names = field_names(&fields);

        append_unshadowed_fields(
            &mut fields,
            &declared_names,
            vec![
                field("number", FieldKind::Instance, "J"),
                field("staticNumber", FieldKind::Static, "I"),
            ],
        );

        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "number");
        assert_eq!(fields[0].kind, FieldKind::Static);
        assert_eq!(fields[1].name, "staticNumber");
        assert_eq!(fields[1].kind, FieldKind::Static);
    }
}
