//! VM-scoped Java class-loader references.

use std::{fmt, sync::Arc};

use crate::{
    env::Env,
    error::{Error, Result},
    jni,
    refs::{AsJClass, AsJObject, GlobalRef, ObjectKind},
    vm::Vm,
};

/// Describes how a `ClassLoaderRef` entered this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClassLoaderKind {
    /// The process system class loader returned by `ClassLoader.getSystemClassLoader()`.
    System,
    /// The app class loader selected from `ActivityThread.currentApplication()`.
    App,
    /// A loader explicitly wrapped from a Java object.
    Object,
    /// A loader discovered by ART class-loader enumeration.
    Enumerated,
}

/// An owned global reference to a `java.lang.ClassLoader`.
///
/// Loader references are VM-scoped and may be cloned cheaply. They are validated as
/// `java.lang.ClassLoader` instances when constructed.
#[derive(Clone)]
pub struct ClassLoaderRef {
    vm: Vm,
    object: Arc<GlobalRef<ObjectKind>>,
    kind: ClassLoaderKind,
}

impl ClassLoaderRef {
    pub fn kind(&self) -> ClassLoaderKind {
        self.kind
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    /// Returns the raw JNI global class-loader reference.
    ///
    /// # Safety
    ///
    /// The caller must not delete the returned reference or use it with a different VM.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        unsafe { self.object.raw_jobject() }
    }

    pub(crate) unsafe fn from_global_raw_attached(
        env: &Env<'_>,
        vm: Vm,
        raw: jni::jobject,
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        let object = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        let loader = Self {
            vm,
            object: Arc::new(object),
            kind,
        };
        validate_class_loader(env, &loader, "ClassLoaderRef::from_global_raw")?;
        Ok(loader)
    }

    pub(crate) fn from_object_ref(
        env: &Env<'_>,
        vm: &Vm,
        object: &(impl AsJObject + ?Sized),
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
        let object = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
        let loader = Self {
            vm: vm.clone(),
            object: Arc::new(object),
            kind,
        };
        validate_class_loader(env, &loader, "Java::class_loader_from_object")?;
        Ok(loader)
    }

    #[cfg(test)]
    pub(crate) unsafe fn dangling_for_tests(vm: Vm, kind: ClassLoaderKind) -> Self {
        let object = unsafe { GlobalRef::from_raw(vm.clone(), std::ptr::dangling_mut()).unwrap() };
        Self {
            vm,
            object: Arc::new(object),
            kind,
        }
    }
}

impl fmt::Debug for ClassLoaderRef {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("ClassLoaderRef")
            .field("kind", &self.kind)
            .field("object", &unsafe { self.raw_jobject() })
            .finish()
    }
}

fn validate_class_loader(
    env: &Env<'_>,
    loader: &ClassLoaderRef,
    operation: &'static str,
) -> Result<()> {
    let class_loader_class = env.find_class("java/lang/ClassLoader")?;
    if env.is_instance_of(loader, &class_loader_class)? {
        Ok(())
    } else {
        let actual = env.get_object_class(loader)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "java.lang.ClassLoader",
            actual: format!("{:p}", actual.as_jclass()),
        })
    }
}

impl crate::refs::sealed::JavaObjectRefSealed for ClassLoaderRef {
    fn as_jobject(&self) -> jni::jobject {
        unsafe { self.raw_jobject() }
    }
}

impl crate::refs::JavaObjectRef for ClassLoaderRef {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_loader_kinds_are_distinct() {
        assert_ne!(ClassLoaderKind::App, ClassLoaderKind::Object);
        assert_ne!(ClassLoaderKind::App, ClassLoaderKind::System);
        assert_ne!(ClassLoaderKind::App, ClassLoaderKind::Enumerated);
        assert_eq!(format!("{:?}", ClassLoaderKind::App), "App");
    }

    #[test]
    fn formats_loader_errors() {
        let unsupported = Error::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            reason: "missing symbol".to_owned(),
        };
        assert_eq!(
            unsupported.to_string(),
            "ART class-loader enumeration is not supported: missing symbol"
        );

        let invalid = Error::InvalidObjectType {
            operation: "Java::class_loader_from_object",
            expected: "java.lang.ClassLoader",
            actual: "java.lang.String".to_owned(),
        };
        assert_eq!(
            invalid.to_string(),
            "Java::class_loader_from_object expected java.lang.ClassLoader, got java.lang.String"
        );
    }
}
