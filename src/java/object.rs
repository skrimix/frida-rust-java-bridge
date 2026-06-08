use std::fmt;

#[cfg(test)]
use std::ptr;

use crate::{
    env::FieldKind,
    error::{Error, Result},
    jni,
    metadata::{self, JavaMethodMetadata},
    refs::{AsJObject, BorrowedLocalRef, GlobalRef, JavaObjectRef, ObjectKind},
    vm::Vm,
};

use super::{
    FromJavaReturn, IntoJavaCallArgs, IntoJavaFieldValue, JavaReturn,
    class::JavaClass,
    members::{JavaField, JavaMethod, JavaMethodGroup},
    raw,
};

/// A borrowed Java object bound to an explicit class wrapper for ergonomic instance calls.
///
/// This borrows the object reference and keeps the caller-selected class/loader context visible.
pub struct JavaBoundObject<'object> {
    pub(super) class: JavaClass,
    pub(super) object: &'object (dyn JavaObjectRef + 'object),
}

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

/// A safe wrapper representing a Java object instance.
///
/// By default, a `JavaObject` owns an underlying global JNI reference. This means the object is kept alive
/// in Java as long as the Rust wrapper is held, and it can be safely sent and moved across different Rust threads
/// (as long as those threads attach to the Java VM when performing operations).
///
/// It also stores a reference to its wrapper [`JavaClass`] to enable convenient instance method calls
/// and field access.
///
/// ### Callback-Local Views
///
/// In replacement hooks, Java objects are often passed as callback-local views (such as `JavaLocalObject`).
/// These views borrow the underlying JNI reference and are valid *only* for the duration of the callback.
/// If you need to keep a callback-local object alive after the hook returns, call `.retain()` to promote
/// it to an owned global `JavaObject`.
pub struct JavaObject<R = GlobalRef<ObjectKind>> {
    pub(super) class: JavaClass,
    pub(super) reference: R,
}

/// A callback-local borrowed view of a Java object.
///
/// Unlike a standard [`JavaObject`], a local object view only borrows the underlying JNI reference and
/// does not clean it up on drop. These are typically passed as `this` or as argument values inside replacement
/// hook callbacks, and are valid only while that callback is running. Call `.retain()` to promote this local
/// view into an owned global reference.
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
    pub fn vm(&self) -> &Vm {
        self.class.class.vm()
    }

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

    pub fn retain(&self) -> Result<JavaObject> {
        let env = self.vm().attach_current_thread()?;
        let reference = unsafe { env.new_global_ref_raw(self.reference.as_jobject())? };
        let reference = unsafe { GlobalRef::from_raw(self.vm().clone(), reference)? };
        Ok(JavaObject {
            class: self.class.clone(),
            reference,
        })
    }

    pub fn runtime_class(&self) -> Result<JavaClass> {
        runtime_class(self.vm(), self)
    }

    pub fn cast(&self, class: &JavaClass) -> Result<JavaObject> {
        class.cast(self)
    }

    pub fn method<'object>(&'object self, name: &str) -> Result<JavaBoundMethodGroup<'object>> {
        self.class.bind(self)?.method(name)
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.class.bind(self)?.call(name, args)
    }

    pub fn call_with<'a, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'a str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.class.bind(self)?.call_with(name, arguments, args)
    }

    pub fn field<'object>(&'object self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        self.class.bind(self)?.field(name)
    }

    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.class.bind(self)?.get_field(name)
    }

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.class.bind(self)?.set_field(name, value)
    }

    pub fn get_string(&self) -> Result<String> {
        let env = self.vm().attach_current_thread()?;
        unsafe { env.get_string_raw(self.raw_jobject()) }
    }

    pub fn java_to_string(&self) -> Result<String> {
        object_to_string(self.vm(), self)
    }

    pub fn java_display(&self) -> Result<String> {
        self.java_to_string()
    }
}

impl<'local> JavaObject<BorrowedLocalRef<'local, ObjectKind>> {
    pub(crate) unsafe fn from_raw_with_class(class: JavaClass, raw: jni::jobject) -> Result<Self> {
        let reference = unsafe { BorrowedLocalRef::from_raw(raw, "JNI local object reference")? };
        Ok(Self { class, reference })
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

impl fmt::Debug for JavaBoundObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundObject")
            .field("class", &self.class)
            .field("object", &self.object.as_jobject())
            .finish()
    }
}

impl<'object> JavaBoundObject<'object> {
    pub fn class(&self) -> &JavaClass {
        &self.class
    }

    pub fn object(&self) -> &'object dyn JavaObjectRef {
        self.object
    }

    pub fn method(&self, name: &str) -> Result<JavaBoundMethodGroup<'object>> {
        Ok(JavaBoundMethodGroup {
            object: self.object,
            group: self.class.method(name)?,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, name: &str, args: impl IntoJavaCallArgs) -> Result<T> {
        self.method(name)?.call(args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.method(name)?.overload(arguments)?.call(args)
    }

    pub fn field(&self, name: &str) -> Result<JavaBoundFieldHandle<'object>> {
        Ok(JavaBoundFieldHandle {
            object: self.object,
            field: self.class.field(name)?,
        })
    }

    pub fn get_field<T: FromJavaReturn>(&self, name: &str) -> Result<T> {
        self.field(name)?.get()
    }

    pub fn set_field<V: IntoJavaFieldValue>(&self, name: &str, value: V) -> Result<()> {
        self.field(name)?.set(value)
    }
}

impl<'object> JavaBoundMethodGroup<'object> {
    pub fn name(&self) -> &str {
        self.group.name()
    }

    pub fn overloads(&self) -> &[JavaMethodMetadata] {
        self.group.overloads()
    }

    pub fn overload<'types>(
        &self,
        arguments: impl AsRef<[&'types str]>,
    ) -> Result<JavaBoundMethodOverload<'object>> {
        Ok(JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.overload(arguments)?,
        })
    }

    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        let args = args.into_java_overload_args();
        JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.dispatch_bound(&args)?,
        }
        .call(args)
    }

    pub fn call_with<'types, T: FromJavaReturn>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        args: impl IntoJavaCallArgs,
    ) -> Result<T> {
        self.overload(arguments)?.call(args)
    }
}

impl JavaBoundMethodOverload<'_> {
    pub fn overload(&self) -> &JavaMethod {
        &self.overload
    }

    pub fn call_raw<A: IntoJavaCallArgs>(&self, args: A) -> Result<JavaReturn> {
        self.overload.call_raw(self.object, args)
    }

    pub fn call<T: FromJavaReturn>(&self, args: impl IntoJavaCallArgs) -> Result<T> {
        T::from_java_return(
            self.overload.bind_declared_return(self.call_raw(args)?)?,
            "JavaBoundMethodOverload::call",
        )
    }
}

impl JavaBoundFieldHandle<'_> {
    pub fn field(&self) -> &JavaField {
        &self.field
    }

    pub fn get_raw(&self) -> Result<JavaReturn> {
        match self.field.kind() {
            FieldKind::Static => self.field.get_raw(()),
            FieldKind::Instance => self.field.get_raw(self.object),
        }
    }

    pub fn get<T: FromJavaReturn>(&self) -> Result<T> {
        T::from_java_return(
            self.field.bind_declared_return(self.get_raw()?)?,
            "JavaBoundFieldHandle::get",
        )
    }

    pub fn set<V: IntoJavaFieldValue>(&self, value: V) -> Result<()> {
        match self.field.kind() {
            FieldKind::Static => self.field.set((), value),
            FieldKind::Instance => self.field.set(self.object, value),
        }
    }
}
