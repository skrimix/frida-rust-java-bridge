use crate::{
    env::Env,
    error::{Error, Result},
    jni, metadata,
    refs::{ArrayKind, AsJClass, BorrowedLocalRef, GlobalRef, JavaObjectRef},
    signature::JavaType,
    vm::Vm,
};

use super::{
    Java,
    class::JavaClass,
    object::{JavaLocalObject, JavaObject},
    raw,
};

/// Java array instance.
///
/// The normal `JavaArray` owns a global JNI reference, so the array stays alive while the Rust
/// wrapper is held and can move across Rust threads.
///
/// ### Working with Arrays
///
/// - Primitive arrays have copy-in and copy-out helpers for Rust slices.
/// - Object arrays support reading and writing nullable object references.
///
/// Callback-local array views, such as [`JavaLocalArray`], only borrow the array for the
/// replacement callback. Use [`.retain()`](JavaArray::retain) to keep one afterwards.
pub struct JavaArray<R = GlobalRef<ArrayKind>> {
    pub(super) object: JavaObject<R>,
    pub(super) element_type: JavaType,
}

/// Callback-local borrowed Java array.
///
/// Local array views mirror standard [`JavaArray`] operations but only live for the replacement
/// callback where they were provided.
pub type JavaLocalArray<'local> = JavaArray<BorrowedLocalRef<'local, ArrayKind>>;

trait JavaArrayStorage: JavaObjectRef {
    const OPERATION_NAME: &'static str;
}

impl JavaArrayStorage for GlobalRef<ArrayKind> {
    const OPERATION_NAME: &'static str = "JavaArray";
}

impl JavaArray {
    /// Returns this array as an owned Java object.
    ///
    /// The returned object owns a separate global reference to the same Java array.
    pub fn as_object(&self) -> Result<JavaObject> {
        let env = self.vm().attach_current_thread()?;
        object_from_ref_with_class(&env, self.object.class.clone(), self)
    }

    /// Converts this owned array wrapper into an owned Java object wrapper.
    ///
    /// The existing global reference is reused.
    pub fn into_object(self) -> Result<JavaObject> {
        let JavaArray { object, .. } = self;
        let JavaObject { class, reference } = object;
        let raw = unsafe { reference.into_raw() };
        let reference = unsafe { GlobalRef::from_raw(class.class.vm().clone(), raw)? };
        Ok(JavaObject { class, reference })
    }
}

impl<'local> JavaArray<BorrowedLocalRef<'local, ArrayKind>> {
    pub(crate) unsafe fn from_raw_with_class(
        class: JavaClass,
        raw: jni::jobject,
        element_type: JavaType,
    ) -> Result<Self> {
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local array view")? };
        Ok(Self {
            object: JavaObject { class, reference },
            element_type,
        })
    }

    /// Returns this local array view as a local Java object view.
    ///
    /// The returned object is borrowed from the same JNI frame as this array.
    pub fn as_object(&self) -> Result<JavaLocalObject<'local>> {
        unsafe {
            JavaLocalObject::from_raw_with_class(self.object.class.clone(), self.raw_jobject())
        }
    }
}

impl<'local> JavaArrayStorage for BorrowedLocalRef<'local, ArrayKind> {
    const OPERATION_NAME: &'static str = "JavaLocalArray";
}

impl<R> JavaArray<R>
where
    R: JavaObjectRef,
{
    pub(crate) fn vm_ref(&self) -> &Vm {
        self.object.vm()
    }
}

impl<R> JavaArray<R>
where
    R: JavaArrayStorage,
{
    /// Returns the Java VM that owns this array reference.
    pub fn vm(&self) -> &Vm {
        self.object.vm()
    }

    /// Returns the raw JNI array reference.
    ///
    /// # Safety
    ///
    /// The caller must honor this wrapper's reference storage rules: global references must not be
    /// deleted by the caller, and borrowed local references are valid only in their producing
    /// callback/JNI frame on the current thread.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        unsafe { self.object.raw_jobject() }
    }

    /// Returns the declared element type for this array.
    pub fn element_type(&self) -> &JavaType {
        &self.element_type
    }

    /// Returns the number of elements in this Java array.
    pub fn len(&self) -> Result<jni::jsize> {
        array_len(self.vm(), self)
    }

    /// Returns `true` when this Java array has no elements.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Creates an owned global reference to this array.
    ///
    /// Use this to keep a callback-local [`JavaLocalArray`] after the callback returns.
    pub fn retain(&self) -> Result<JavaArray> {
        let env = self.vm().attach_current_thread()?;
        array_from_ref_with_class(
            &env,
            self.object.class.clone(),
            self,
            self.element_type.clone(),
        )
    }

    /// Calls Java `Object.toString()` for this array and returns the resulting Rust string.
    pub fn java_to_string(&self) -> Result<String> {
        self.object.java_to_string()
    }

    /// Reads one nullable object element from this object array.
    ///
    /// Returns [`Error::InvalidObjectType`] if this is not an object array.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaObject, Result};
    ///
    /// fn first_string(java: &Java) -> Result<Option<JavaObject>> {
    ///     let string_class = java.find_class("java.lang.String")?;
    ///     let first = java.new_string_utf("one")?;
    ///     let array = java.new_object_array(&string_class, &[Some(&first)])?;
    ///     array.get_object(0)
    /// }
    /// ```
    pub fn get_object(&self, index: jni::jsize) -> Result<Option<JavaObject>> {
        get_array_object(
            self.vm(),
            &self.object.class,
            self,
            &self.element_type,
            index,
            operation_name::<R>("get_object"),
        )
    }

    /// Writes one nullable object reference into this object array.
    ///
    /// Returns [`Error::InvalidObjectType`] if this is not an object array.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn replace_first_string(java: &Java) -> Result<()> {
    ///     let string_class = java.find_class("java.lang.String")?;
    ///     let first = java.new_string_utf("one")?;
    ///     let second = java.new_string_utf("two")?;
    ///     let array = java.new_object_array(&string_class, &[Some(&first)])?;
    ///     array.set_object(0, Some(&second))
    /// }
    /// ```
    pub fn set_object<T: JavaObjectRef + ?Sized>(
        &self,
        index: jni::jsize,
        value: Option<&T>,
    ) -> Result<()> {
        set_array_object(
            self.vm(),
            self,
            &self.element_type,
            index,
            value,
            operation_name::<R>("set_object"),
        )
    }

    /// Copies all elements out of this Java `boolean[]`.
    pub fn get_booleans(&self) -> Result<Vec<bool>> {
        get_boolean_array(
            self.vm(),
            self,
            &self.element_type,
            operation_name::<R>("get_booleans"),
        )
    }

    /// Copies `values` into this Java `boolean[]` starting at index 0.
    ///
    /// The JNI call fails if `values` is longer than the Java array.
    pub fn set_booleans(&self, values: &[bool]) -> Result<()> {
        set_boolean_array(
            self.vm(),
            self,
            &self.element_type,
            values,
            operation_name::<R>("set_booleans"),
        )
    }

    java_primitive_array_accessors! {
        R;

        get_bytes, set_bytes, jni::jbyte, JavaType::Byte,
        get_byte_array_region, set_byte_array_region;

        get_chars, set_chars, jni::jchar, JavaType::Char,
        get_char_array_region, set_char_array_region;

        get_shorts, set_shorts, jni::jshort, JavaType::Short,
        get_short_array_region, set_short_array_region;

        get_ints, set_ints, jni::jint, JavaType::Int,
        get_int_array_region, set_int_array_region;

        get_longs, set_longs, jni::jlong, JavaType::Long,
        get_long_array_region, set_long_array_region;

        get_floats, set_floats, jni::jfloat, JavaType::Float,
        get_float_array_region, set_float_array_region;

        get_doubles, set_doubles, jni::jdouble, JavaType::Double,
        get_double_array_region, set_double_array_region;
    }
}

impl<R> std::fmt::Debug for JavaArray<R>
where
    R: JavaObjectRef,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaArray")
            .field("array", &unsafe { self.object.raw_jobject() })
            .field("element_type", &self.element_type)
            .finish()
    }
}

impl<R> crate::refs::sealed::JavaObjectRefSealed for JavaArray<R>
where
    R: JavaObjectRef,
{
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.object.raw_jobject() }
    }
}

impl<R> crate::refs::JavaObjectRef for JavaArray<R> where R: JavaObjectRef {}

pub(super) fn object_from_ref_with_class(
    env: &Env<'_>,
    class: JavaClass,
    object: &(impl JavaObjectRef + ?Sized),
) -> Result<JavaObject> {
    let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
    let reference = unsafe { GlobalRef::from_raw(class.class.vm().clone(), reference)? };
    Ok(JavaObject::from_global_ref(class, reference))
}

pub(super) fn array_from_ref_with_class(
    env: &Env<'_>,
    class: JavaClass,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: JavaType,
) -> Result<JavaArray> {
    let reference = unsafe { env.new_global_ref_raw(array.as_jobject())? };
    let vm = class.class.vm().clone();
    let reference = unsafe { GlobalRef::from_raw(vm, reference)? };
    Ok(JavaArray {
        object: JavaObject { class, reference },
        element_type,
    })
}

pub(super) fn object_from_ref_with_declared(
    env: &Env<'_>,
    holder: &raw::Class,
    object: &(impl JavaObjectRef + ?Sized),
    name: &str,
    operation: &'static str,
) -> Result<JavaObject> {
    let declared_type = JavaType::Object(name.to_owned());
    let class = declared_class(env, holder, &declared_type)?;
    if class.is_instance(object)? {
        object_from_ref_with_class(env, class, object)
    } else {
        let actual = env.get_object_class(object)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "declared object type",
            actual: format!("{:p} is not {}", actual.as_jclass(), declared_type),
        })
    }
}

pub(super) fn array_from_ref_with_declared(
    env: &Env<'_>,
    holder: &raw::Class,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: JavaType,
    operation: &'static str,
) -> Result<JavaArray> {
    let array_type = JavaType::Array(Box::new(element_type.clone()));
    let class = declared_class(env, holder, &array_type)?;
    if class.is_instance(array)? {
        array_from_ref_with_class(env, class, array, element_type)
    } else {
        let actual = env.get_object_class(array)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "declared array type",
            actual: format!("{:p} is not {}", actual.as_jclass(), array_type),
        })
    }
}

pub(super) fn declared_class(
    env: &Env<'_>,
    holder: &raw::Class,
    ty: &JavaType,
) -> Result<JavaClass> {
    let java = Java::new(holder.vm().clone());
    let scoped_java = match metadata::class_loader(env, holder.vm(), holder)? {
        Some(loader) => java.with_loader(&loader),
        None => java,
    };
    Ok(JavaClass::from_raw(
        scoped_java.find_class(&ty.descriptor())?,
    ))
}

fn array_len(vm: &Vm, array: &(impl JavaObjectRef + ?Sized)) -> Result<jni::jsize> {
    let env = vm.attach_current_thread()?;
    env.array_length(array)
}

fn get_array_object(
    vm: &Vm,
    array_class: &JavaClass,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: &JavaType,
    index: jni::jsize,
    operation: &'static str,
) -> Result<Option<JavaObject>> {
    ensure_reference_array(element_type, operation)?;
    let env = vm.attach_current_thread()?;
    let class = declared_class(&env, &array_class.class, element_type)?;
    env.get_object_array_element_nullable(array, index)?
        .map(|object| {
            if class.is_instance(&object)? {
                object_from_ref_with_class(&env, class.clone(), &object)
            } else {
                let actual = env.get_object_class(&object)?;
                Err(Error::InvalidObjectType {
                    operation,
                    expected: "array element type",
                    actual: format!("{:p} is not {}", actual.as_jclass(), element_type),
                })
            }
        })
        .transpose()
}

fn set_array_object<T: JavaObjectRef + ?Sized>(
    vm: &Vm,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: &JavaType,
    index: jni::jsize,
    value: Option<&T>,
    operation: &'static str,
) -> Result<()> {
    ensure_reference_array(element_type, operation)?;
    let env = vm.attach_current_thread()?;
    env.set_object_array_element_raw(array, index, value)
}

fn get_boolean_array(
    vm: &Vm,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: &JavaType,
    operation: &'static str,
) -> Result<Vec<bool>> {
    ensure_element_type(element_type, &JavaType::Boolean, operation)?;
    let env = vm.attach_current_thread()?;
    let mut values = vec![jni::JNI_FALSE; env.array_length(array)? as usize];
    env.get_boolean_array_region(array, 0, &mut values)?;
    Ok(values
        .into_iter()
        .map(|value| value == jni::JNI_TRUE)
        .collect())
}

fn set_boolean_array(
    vm: &Vm,
    array: &(impl JavaObjectRef + ?Sized),
    element_type: &JavaType,
    values: &[bool],
    operation: &'static str,
) -> Result<()> {
    ensure_element_type(element_type, &JavaType::Boolean, operation)?;
    let values = values
        .iter()
        .map(|value| {
            if *value {
                jni::JNI_TRUE
            } else {
                jni::JNI_FALSE
            }
        })
        .collect::<Vec<_>>();
    let env = vm.attach_current_thread()?;
    env.set_boolean_array_region(array, 0, &values)
}

fn ensure_reference_array(element_type: &JavaType, operation: &'static str) -> Result<()> {
    if element_type.is_reference() {
        Ok(())
    } else {
        Err(Error::InvalidObjectType {
            operation,
            expected: "object array",
            actual: format!("{element_type} array"),
        })
    }
}

fn ensure_element_type(
    actual: &JavaType,
    expected: &JavaType,
    operation: &'static str,
) -> Result<()> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::InvalidObjectType {
            operation,
            expected: "matching array element type",
            actual: actual.descriptor(),
        })
    }
}

fn operation_name<R: JavaArrayStorage>(method: &'static str) -> &'static str {
    match (R::OPERATION_NAME, method) {
        ("JavaArray", "get_object") => "JavaArray::get_object",
        ("JavaArray", "set_object") => "JavaArray::set_object",
        ("JavaArray", "get_booleans") => "JavaArray::get_booleans",
        ("JavaArray", "set_booleans") => "JavaArray::set_booleans",
        ("JavaArray", "get_bytes") => "JavaArray::get_bytes",
        ("JavaArray", "set_bytes") => "JavaArray::set_bytes",
        ("JavaArray", "get_chars") => "JavaArray::get_chars",
        ("JavaArray", "set_chars") => "JavaArray::set_chars",
        ("JavaArray", "get_shorts") => "JavaArray::get_shorts",
        ("JavaArray", "set_shorts") => "JavaArray::set_shorts",
        ("JavaArray", "get_ints") => "JavaArray::get_ints",
        ("JavaArray", "set_ints") => "JavaArray::set_ints",
        ("JavaArray", "get_longs") => "JavaArray::get_longs",
        ("JavaArray", "set_longs") => "JavaArray::set_longs",
        ("JavaArray", "get_floats") => "JavaArray::get_floats",
        ("JavaArray", "set_floats") => "JavaArray::set_floats",
        ("JavaArray", "get_doubles") => "JavaArray::get_doubles",
        ("JavaArray", "set_doubles") => "JavaArray::set_doubles",
        ("JavaLocalArray", "get_object") => "JavaLocalArray::get_object",
        ("JavaLocalArray", "set_object") => "JavaLocalArray::set_object",
        ("JavaLocalArray", "get_booleans") => "JavaLocalArray::get_booleans",
        ("JavaLocalArray", "set_booleans") => "JavaLocalArray::set_booleans",
        ("JavaLocalArray", "get_bytes") => "JavaLocalArray::get_bytes",
        ("JavaLocalArray", "set_bytes") => "JavaLocalArray::set_bytes",
        ("JavaLocalArray", "get_chars") => "JavaLocalArray::get_chars",
        ("JavaLocalArray", "set_chars") => "JavaLocalArray::set_chars",
        ("JavaLocalArray", "get_shorts") => "JavaLocalArray::get_shorts",
        ("JavaLocalArray", "set_shorts") => "JavaLocalArray::set_shorts",
        ("JavaLocalArray", "get_ints") => "JavaLocalArray::get_ints",
        ("JavaLocalArray", "set_ints") => "JavaLocalArray::set_ints",
        ("JavaLocalArray", "get_longs") => "JavaLocalArray::get_longs",
        ("JavaLocalArray", "set_longs") => "JavaLocalArray::set_longs",
        ("JavaLocalArray", "get_floats") => "JavaLocalArray::get_floats",
        ("JavaLocalArray", "set_floats") => "JavaLocalArray::set_floats",
        ("JavaLocalArray", "get_doubles") => "JavaLocalArray::get_doubles",
        ("JavaLocalArray", "set_doubles") => "JavaLocalArray::set_doubles",
        _ => R::OPERATION_NAME,
    }
}

#[cfg(test)]
mod tests {
    use std::ptr;

    use crate::JavaValue;

    use super::*;

    #[test]
    fn local_array_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let vm = Vm::dangling_for_tests();
        let class = JavaClass::from_raw(raw::Class::from_global("[I".to_owned(), unsafe {
            GlobalRef::from_raw(vm.clone(), std::ptr::dangling_mut()).unwrap()
        }));
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "test array").unwrap() };
        let array = JavaLocalArray {
            object: JavaObject { class, reference },
            element_type: JavaType::Int,
        };
        assert_eq!(unsafe { array.raw_jobject() }, raw);
        assert_eq!(array.element_type(), &JavaType::Int);
    }

    #[test]
    fn global_array_wrapper_keeps_default_java_value_conversion() {
        let raw = std::ptr::dangling_mut();
        let vm = Vm::dangling_for_tests();
        let reference = unsafe { GlobalRef::from_raw(vm.clone(), raw) }.unwrap();
        let class = JavaClass::from_raw(raw::Class::from_global("[I".to_owned(), unsafe {
            GlobalRef::from_raw(vm.clone(), std::ptr::dangling_mut()).unwrap()
        }));
        let array = JavaArray {
            object: JavaObject { class, reference },
            element_type: JavaType::Int,
        };

        assert_eq!(unsafe { array.raw_jobject() }, raw);
        assert_eq!(array.element_type(), &JavaType::Int);
        assert_eq!(JavaValue::from(&array), JavaValue::object_ref(raw));
    }

    #[test]
    fn local_array_view_rejects_null_raw() {
        let vm = Vm::dangling_for_tests();
        let class = JavaClass::from_raw(raw::Class::from_global("[I".to_owned(), unsafe {
            GlobalRef::from_raw(vm.clone(), std::ptr::dangling_mut()).unwrap()
        }));
        assert_eq!(
            unsafe { JavaLocalArray::from_raw_with_class(class, ptr::null_mut(), JavaType::Int) }
                .unwrap_err(),
            Error::NullReturn {
                operation: "JNI local array view",
            }
        );
    }

    #[test]
    fn local_array_type_checks_report_expected_errors() {
        assert_eq!(
            ensure_reference_array(&JavaType::Int, "test").unwrap_err(),
            Error::InvalidObjectType {
                operation: "test",
                expected: "object array",
                actual: "I array".to_owned(),
            }
        );
        assert_eq!(
            ensure_element_type(&JavaType::Byte, &JavaType::Int, "test").unwrap_err(),
            Error::InvalidObjectType {
                operation: "test",
                expected: "matching array element type",
                actual: "B".to_owned(),
            }
        );
    }
}
