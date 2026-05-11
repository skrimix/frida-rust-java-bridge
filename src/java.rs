use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use crate::{
    env::{Env, FieldKind, FieldRef, MethodKind, MethodRef},
    error::{Error, Result},
    jni,
    refs::{AsJClass, AsJObject, ClassKind, GlobalRef, ObjectKind},
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

#[derive(Clone)]
pub struct Java {
    vm: Vm,
}

#[derive(Clone)]
pub struct JavaClass {
    inner: Arc<JavaClassInner>,
}

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
        Self { vm }
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn find_class(&self, name: &str) -> Result<JavaClass> {
        let env = self.vm.attach_current_thread()?;
        let name = normalize_class_name(name);
        let local = env.find_class(&name)?;
        let class = env.new_global_ref(&local)?;

        Ok(JavaClass {
            inner: Arc::new(JavaClassInner {
                vm: self.vm.clone(),
                name,
                class,
                methods: Mutex::new(HashMap::new()),
                fields: Mutex::new(HashMap::new()),
            }),
        })
    }

    pub fn new_string_utf(&self, text: &str) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        let string = env.new_string_utf(text)?;
        object_from_ref(&env, &self.vm, &string)
    }
}

impl JavaClass {
    pub fn name(&self) -> &str {
        &self.inner.name
    }

    pub fn as_jclass(&self) -> jni::jclass {
        self.inner.class.as_jclass()
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

fn object_from_ref(
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

fn normalize_class_name(name: &str) -> String {
    let name = if name.starts_with('L') && name.ends_with(';') {
        &name[1..name.len() - 1]
    } else {
        name
    };
    name.replace('.', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_class_names_for_find_class() {
        assert_eq!(normalize_class_name("java.lang.String"), "java/lang/String");
        assert_eq!(normalize_class_name("java/lang/String"), "java/lang/String");
        assert_eq!(
            normalize_class_name("Ljava/lang/String;"),
            "java/lang/String"
        );
        assert_eq!(
            normalize_class_name("Ljava.lang.String;"),
            "java/lang/String"
        );
        assert_eq!(
            normalize_class_name("com.example.Outer$Inner"),
            "com/example/Outer$Inner"
        );
        assert_eq!(normalize_class_name("[I"), "[I");
        assert_eq!(
            normalize_class_name("[Ljava.lang.String;"),
            "[Ljava/lang/String;"
        );
    }
}
