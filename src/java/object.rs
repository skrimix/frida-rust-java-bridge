#[cfg(test)]
use std::ptr;

use crate::{
    env::FieldKind,
    error::{Error, Result},
    jni,
    metadata::{self, JavaMethodMetadata},
    refs::{AsJClass, AsJObject, BorrowedLocalRef, GlobalRef, JavaObjectRef, ObjectKind},
    vm::Vm,
};

use super::{
    FromJavaReturn, IntoJavaCallArgs, IntoJavaFieldValue, JavaReturn,
    class::JavaClass,
    members::{JavaField, JavaMethod, JavaMethodGroup},
    raw,
};

/// A named Java method group bound to one borrowed Java receiver.
pub struct JavaBoundMethodGroup<'object> {
    pub(super) object: &'object (dyn JavaObjectRef + 'object),
    pub(super) group: JavaMethodGroup,
}

/// A selected method bound to one borrowed Java receiver.
pub struct JavaBoundMethodOverload<'object> {
    pub(super) object: &'object (dyn JavaObjectRef + 'object),
    pub(super) overload: JavaMethod,
}

/// A selected field bound to one borrowed Java receiver.
pub struct JavaBoundFieldHandle<'object> {
    pub(super) object: &'object (dyn JavaObjectRef + 'object),
    pub(super) field: JavaField,
}

/// Java object instance.
///
/// The normal `JavaObject` owns a global JNI reference, so the Java object stays alive while the
/// Rust wrapper is held. It can move across Rust threads; each thread still needs to attach to the
/// VM before doing Java work.
///
/// The wrapper also keeps its [`JavaClass`], which is used for instance methods, fields, casts, and
/// metadata lookups.
///
/// ### Callback-Local Views
///
/// Replacement hooks receive borrowed objects such as [`JavaLocalObject`]. Those views are only
/// valid during the callback. Call [`.retain()`](JavaObject::retain) to keep one afterwards.
pub struct JavaObject<R = GlobalRef<ObjectKind>> {
    pub(super) class: JavaClass,
    pub(super) reference: R,
}

/// Callback-local borrowed Java object.
///
/// These are typically passed as `this` or as object arguments inside replacement callbacks. Call
/// [`.retain()`](JavaObject::retain) to promote one into an owned global [`JavaObject`].
pub type JavaLocalObject<'local> = JavaObject<BorrowedLocalRef<'local, ObjectKind>>;

impl JavaObject {
    #[cfg(test)]
    pub(crate) unsafe fn from_global_raw(class: JavaClass, raw: jni::jobject) -> Result<Self> {
        let vm = class.class.vm().clone();
        let reference = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        Ok(Self { class, reference })
    }

    pub(crate) fn from_global_ref(class: JavaClass, reference: GlobalRef<ObjectKind>) -> Self {
        Self { class, reference }
    }
}

impl<R> JavaObject<R>
where
    R: JavaObjectRef,
{
    /// Returns the Java VM that owns this object reference.
    pub fn vm(&self) -> &Vm {
        self.class.class.vm()
    }

    /// Returns the class handle associated with this object wrapper.
    ///
    /// Use [`JavaObject::runtime_class`] when you need the object's actual runtime class.
    pub fn class(&self) -> &JavaClass {
        &self.class
    }

    pub(crate) fn rebind(self, class: JavaClass) -> Self {
        Self {
            class,
            reference: self.reference,
        }
    }

    /// Returns the raw JNI object reference.
    ///
    /// # Safety
    ///
    /// The caller must honor this wrapper's reference storage rules: global references must not be
    /// deleted by the caller, and borrowed local references are valid only in their producing
    /// callback/JNI frame on the current thread.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        self.reference.as_jobject()
    }

    /// Creates an owned global reference to this object.
    ///
    /// Use this to keep a callback-local [`JavaLocalObject`] after the callback returns.
    pub fn retain(&self) -> Result<JavaObject> {
        let env = self.vm().attach_current_thread()?;
        let reference = unsafe { env.new_global_ref_raw(self.reference.as_jobject())? };
        let reference = unsafe { GlobalRef::from_raw(self.vm().clone(), reference)? };
        Ok(JavaObject {
            class: self.class.clone(),
            reference,
        })
    }

    /// Returns the object's actual runtime class.
    pub fn runtime_class(&self) -> Result<JavaClass> {
        runtime_class(self.vm(), self)
    }

    /// Returns the visible method overloads with the given name, bound to this object.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaObject, Result};
    ///
    /// fn append_text(java: &Java) -> Result<String> {
    ///     let builder = java.use_class("java.lang.StringBuilder")?;
    ///     let object = builder.new_object("hello")?;
    ///     let _: JavaObject = object.method("append")?.overload(["java.lang.String"])?.call(" world")?;
    ///     object.call("toString", ())
    /// }
    /// ```
    pub fn method<'object>(&'object self, name: &str) -> Result<JavaBoundMethodGroup<'object>> {
        Ok(JavaBoundMethodGroup {
            object: self,
            group: self.class.method(name)?,
        })
    }

    /// Calls an instance method, selecting an overload from the provided arguments.
    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.method(name)?.call(args)
    }

    /// Calls an instance method using the overload with the given argument type names.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn starts_with_at(java: &Java) -> Result<bool> {
    ///     let string = java.use_class("java.lang.String")?;
    ///     let text = string.new_object("prefix-value")?;
    ///     text.call_with("startsWith", ["java.lang.String", "int"], ("value", 7))
    /// }
    /// ```
    pub fn call_with<'a, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'a str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.method(name)?.call_with(arguments, args)
    }

    /// Returns the visible field with the given name, bound to this object.
    pub fn field<'object>(&'object self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        Ok(JavaBoundFieldHandle {
            object: self,
            field: self.class.field(name)?,
        })
    }

    /// Reads an instance field selected by name.
    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.field(name)?.get()
    }

    /// Writes an instance field selected by name.
    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.field(name)?.set(value)
    }

    /// Reads this object as a Java string.
    ///
    /// Use this only when the object is a `java.lang.String`.
    pub fn get_string(&self) -> Result<String> {
        let env = self.vm().attach_current_thread()?;
        unsafe { env.get_string_raw(self.raw_jobject()) }
    }

    /// Calls Java `Object.toString()` and returns the resulting Rust string.
    pub fn java_to_string(&self) -> Result<String> {
        object_to_string(self.vm(), self)
    }

    /// Returns the result of Java `toString()` for display.
    pub fn java_display(&self) -> Result<String> {
        self.java_to_string()
    }
}

impl JavaObject {
    /// Returns this object as an owned [`JavaObject`] of `class`.
    ///
    /// Returns [`Error::InvalidObjectType`] if the object is not an instance of `class`.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaObject, Result};
    ///
    /// fn as_wifi_manager(java: &Java, context: &JavaObject) -> Result<JavaObject> {
    ///     let wifi_manager = java.use_class("android.net.wifi.WifiManager")?;
    ///     let service: JavaObject = context.call("getSystemService", "wifi")?;
    ///     service.cast(&wifi_manager)
    /// }
    /// ```
    pub fn cast(&self, class: &JavaClass) -> Result<JavaObject> {
        class.cast(self)
    }
}

impl<'local> JavaObject<BorrowedLocalRef<'local, ObjectKind>> {
    pub(crate) unsafe fn from_raw_with_class(class: JavaClass, raw: jni::jobject) -> Result<Self> {
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local object reference")? };
        Ok(Self { class, reference })
    }

    /// Returns this local object view as `class`.
    ///
    /// Returns [`Error::InvalidObjectType`] if the object is not an instance of `class`.
    pub fn cast(&self, class: &JavaClass) -> Result<JavaLocalObject<'local>> {
        if class.is_instance(self)? {
            unsafe { JavaLocalObject::from_raw_with_class(class.clone(), self.raw_jobject()) }
        } else {
            let env = class.class.vm().attach_current_thread()?;
            let actual = env.get_object_class(self)?;
            Err(Error::InvalidObjectType {
                operation: "JavaLocalObject::cast",
                expected: "JavaClass target class",
                actual: format!("{:p} is not {}", actual.as_jclass(), class.name()),
            })
        }
    }
}

impl<R> std::fmt::Debug for JavaObject<R>
where
    R: JavaObjectRef,
{
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_struct("JavaObject")
            .field("class", &self.class.name())
            .field("object", &unsafe { self.raw_jobject() })
            .finish()
    }
}

impl<R> crate::refs::sealed::JavaObjectRefSealed for JavaObject<R>
where
    R: JavaObjectRef,
{
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.raw_jobject() }
    }
}

impl<R> crate::refs::JavaObjectRef for JavaObject<R> where R: JavaObjectRef {}

pub(super) fn object_to_string(vm: &Vm, object: &(impl JavaObjectRef + ?Sized)) -> Result<String> {
    let env = vm.attach_current_thread()?;
    let object_class = env.find_class("java/lang/Object")?;
    let to_string =
        env.lookup_instance_method(&object_class, "toString", "()Ljava/lang/String;")?;
    // SAFETY: `to_string` was resolved from `object`'s runtime class immediately above.
    let string = unsafe { env.call_instance_object_method(object, &to_string, &[])? }.ok_or(
        Error::NullReturn {
            operation: "Object.toString",
        },
    )?;
    unsafe { env.get_string_raw(string.as_jobject()) }
}

pub(super) fn runtime_class(vm: &Vm, object: &(impl JavaObjectRef + ?Sized)) -> Result<JavaClass> {
    let env = vm.attach_current_thread()?;
    let class = env.get_object_class(object)?;
    let descriptor = metadata::class_descriptor(&env, &class)?;
    let name = metadata::class_name_from_descriptor(&descriptor);
    let class = env.new_global_ref(&class)?;
    Ok(JavaClass::from_raw(raw::Class::from_global(name, class)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_object_view_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let vm = Vm::dangling_for_tests();
        let class = JavaClass::from_raw(raw::Class::from_global(
            "java.lang.Object".to_owned(),
            unsafe { GlobalRef::from_raw(vm, std::ptr::dangling_mut()).unwrap() },
        ));
        let object = unsafe { JavaLocalObject::from_raw_with_class(class, raw) }.unwrap();
        assert_eq!(unsafe { object.raw_jobject() }, raw);
    }

    #[test]
    fn local_object_view_rejects_null_raw() {
        let vm = Vm::dangling_for_tests();
        let class = JavaClass::from_raw(raw::Class::from_global(
            "java.lang.Object".to_owned(),
            unsafe { GlobalRef::from_raw(vm, std::ptr::dangling_mut()).unwrap() },
        ));
        assert_eq!(
            unsafe { JavaLocalObject::from_raw_with_class(class, ptr::null_mut()) }.unwrap_err(),
            Error::NullReturn {
                operation: "JNI local object reference",
            }
        );
    }
}

impl<'object> JavaBoundMethodGroup<'object> {
    /// Returns the Java method name shared by these overloads.
    pub fn name(&self) -> &str {
        self.group.name()
    }

    /// Returns metadata for the overloads in this method group.
    pub fn overloads(&self) -> &[JavaMethodMetadata] {
        self.group.overloads()
    }

    /// Selects the overload with the given argument type names.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, Result};
    ///
    /// fn substring(java: &Java) -> Result<String> {
    ///     let string = java.use_class("java.lang.String")?;
    ///     let text = string.new_object("abcdef")?;
    ///     text.method("substring")?.overload(["int", "int"])?.call((1, 4))
    /// }
    /// ```
    pub fn overload<'types>(
        &self,
        arguments: impl AsRef<[&'types str]>,
    ) -> Result<JavaBoundMethodOverload<'object>> {
        Ok(JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.overload(arguments)?,
        })
    }

    /// Calls this method group, selecting an overload from the provided arguments.
    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        let args = args.into_java_overload_args();
        JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.dispatch_bound(&args)?,
        }
        .call(args)
    }

    /// Calls the overload with the given argument type names.
    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.overload(arguments)?.call(args)
    }
}

impl JavaBoundMethodOverload<'_> {
    /// Returns the selected method overload.
    pub fn overload(&self) -> &JavaMethod {
        &self.overload
    }

    /// Calls this selected method and returns the raw Java return value.
    pub fn call_raw<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaReturn> {
        self.overload.call_raw(self.object, args)
    }

    /// Calls this selected method and converts the return value to `T`.
    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        T::from_java_return(
            self.overload.bind_declared_return(self.call_raw(args)?)?,
            "JavaBoundMethodOverload::call",
        )
    }
}

impl JavaBoundFieldHandle<'_> {
    /// Returns the selected field.
    pub fn field(&self) -> &JavaField {
        &self.field
    }

    /// Reads this field and returns the raw Java value.
    pub fn get_raw(&self) -> Result<JavaReturn> {
        match self.field.kind() {
            FieldKind::Static => self.field.get_raw(()),
            FieldKind::Instance => self.field.get_raw(self.object),
        }
    }

    /// Reads this field and converts the value to `T`.
    pub fn get<T: FromJavaReturn>(&self) -> Result<T> {
        T::from_java_return(
            self.field.bind_declared_return(self.get_raw()?)?,
            "JavaBoundFieldHandle::get",
        )
    }

    /// Writes this field.
    pub fn set<V: IntoJavaFieldValue>(&self, value: V) -> Result<()> {
        match self.field.kind() {
            FieldKind::Static => self.field.set((), value),
            FieldKind::Instance => self.field.set(self.object, value),
        }
    }
}
