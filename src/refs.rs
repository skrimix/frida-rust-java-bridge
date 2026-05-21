use std::{marker::PhantomData, ptr, rc::Rc};

use crate::{
    env::Env,
    error::{Error, Result},
    jni,
    value::JavaValue,
    vm::Vm,
};

pub enum ObjectKind {}
pub enum ClassKind {}
pub enum StringKind {}
pub enum ThrowableKind {}
pub enum ArrayKind {}
pub enum ObjectArrayKind {}

pub type ObjectRef<'env> = LocalRef<'env, ObjectKind>;
pub type ClassRef<'env> = LocalRef<'env, ClassKind>;
pub type StringRef<'env> = LocalRef<'env, StringKind>;
pub type ThrowableRef<'env> = LocalRef<'env, ThrowableKind>;
pub type ArrayRef<'env> = LocalRef<'env, ArrayKind>;
pub type ObjectArrayRef<'env> = LocalRef<'env, ObjectArrayKind>;

pub struct LocalRef<'env, K> {
    raw: jni::jobject,
    env: *mut jni::JNIEnv,
    _env: PhantomData<&'env ()>,
    _kind: PhantomData<K>,
    _thread_affine: PhantomData<Rc<()>>,
}

pub struct GlobalRef<K> {
    raw: jni::jobject,
    vm: Vm,
    _kind: PhantomData<K>,
}

pub(crate) mod sealed {
    use crate::jni;

    pub trait JavaObjectRefSealed {
        fn as_jobject(&self) -> jni::jobject;
    }

    pub trait JavaClassRefSealed: JavaObjectRefSealed {
        fn as_jclass(&self) -> jni::jclass;
    }
}

/// Marker for crate-owned wrappers that may be passed to safe Java object operations.
///
/// This trait is sealed: external callers cannot implement it for arbitrary raw JNI handles.
pub trait JavaObjectRef: sealed::JavaObjectRefSealed {}

/// Marker for crate-owned wrappers that may be passed to safe Java class operations.
///
/// This trait is sealed: external callers cannot implement it for arbitrary raw JNI handles.
pub trait JavaClassRef: JavaObjectRef + sealed::JavaClassRefSealed {}

pub(crate) trait AsJObject {
    fn as_jobject(&self) -> jni::jobject;
}

pub(crate) trait AsJClass: AsJObject {
    fn as_jclass(&self) -> jni::jclass;
}

impl<T: JavaObjectRef + ?Sized> AsJObject for T {
    fn as_jobject(&self) -> jni::jobject {
        sealed::JavaObjectRefSealed::as_jobject(self)
    }
}

impl<T: JavaClassRef + ?Sized> AsJClass for T {
    fn as_jclass(&self) -> jni::jclass {
        sealed::JavaClassRefSealed::as_jclass(self)
    }
}

impl<'env, K> LocalRef<'env, K> {
    pub(crate) unsafe fn from_raw(env: &'env Env<'_>, raw: jni::jobject) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JNI local reference",
            });
        }

        Ok(Self {
            raw,
            env: env.handle().as_ptr(),
            _env: PhantomData,
            _kind: PhantomData,
            _thread_affine: PhantomData,
        })
    }

    pub(crate) unsafe fn from_nullable(env: &'env Env<'_>, raw: jni::jobject) -> Option<Self> {
        if raw.is_null() {
            None
        } else {
            Some(Self {
                raw,
                env: env.handle().as_ptr(),
                _env: PhantomData,
                _kind: PhantomData,
                _thread_affine: PhantomData,
            })
        }
    }

    /// Returns the raw JNI local reference.
    ///
    /// # Safety
    ///
    /// The returned handle is only valid on this attached thread and while this local reference's
    /// JNI frame remains alive.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        self.raw
    }

    /// Leaks ownership of the local JNI reference and returns the raw handle.
    ///
    /// # Safety
    ///
    /// The caller becomes responsible for deleting the local reference in the correct JNI frame.
    pub unsafe fn into_raw(mut self) -> jni::jobject {
        let raw = self.raw;
        self.raw = ptr::null_mut();
        raw
    }
}

impl<'env> ClassRef<'env> {
    /// Returns the raw JNI class reference.
    ///
    /// # Safety
    ///
    /// The returned handle has the same local-reference lifetime as `self`.
    pub unsafe fn raw_jclass(&self) -> jni::jclass {
        self.raw
    }
}

impl<'env> StringRef<'env> {
    /// Returns the raw JNI string reference.
    ///
    /// # Safety
    ///
    /// The returned handle has the same local-reference lifetime as `self`.
    pub unsafe fn raw_jstring(&self) -> jni::jstring {
        self.raw
    }
}

impl<'env> ThrowableRef<'env> {
    /// Returns the raw JNI throwable reference.
    ///
    /// # Safety
    ///
    /// The returned handle has the same local-reference lifetime as `self`.
    pub unsafe fn raw_jthrowable(&self) -> jni::jthrowable {
        self.raw
    }
}

impl<K> GlobalRef<K> {
    pub(crate) unsafe fn from_raw(vm: Vm, raw: jni::jobject) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JNI global reference",
            });
        }

        Ok(Self {
            raw,
            vm,
            _kind: PhantomData,
        })
    }

    /// Returns the raw JNI global reference.
    ///
    /// # Safety
    ///
    /// The caller must not delete the returned reference or use it with a different VM.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        self.raw
    }

    /// Leaks ownership of the global JNI reference and returns the raw handle.
    ///
    /// # Safety
    ///
    /// The caller becomes responsible for deleting the global reference with the correct VM.
    pub unsafe fn into_raw(mut self) -> jni::jobject {
        let raw = self.raw;
        self.raw = ptr::null_mut();
        raw
    }
}

impl GlobalRef<ClassKind> {
    /// Returns the raw JNI class reference.
    ///
    /// # Safety
    ///
    /// The caller must not delete the returned reference or use it with a different VM.
    pub unsafe fn raw_jclass(&self) -> jni::jclass {
        self.raw
    }
}

impl<'env, K> sealed::JavaObjectRefSealed for LocalRef<'env, K> {
    fn as_jobject(&self) -> jni::jobject {
        self.raw
    }
}

impl<'env, K> JavaObjectRef for LocalRef<'env, K> {}

impl<K> sealed::JavaObjectRefSealed for GlobalRef<K> {
    fn as_jobject(&self) -> jni::jobject {
        self.raw
    }
}

impl<K> JavaObjectRef for GlobalRef<K> {}

impl<'env> sealed::JavaClassRefSealed for ClassRef<'env> {
    fn as_jclass(&self) -> jni::jclass {
        self.raw
    }
}

impl<'env> JavaClassRef for ClassRef<'env> {}

impl sealed::JavaClassRefSealed for GlobalRef<ClassKind> {
    fn as_jclass(&self) -> jni::jclass {
        self.raw
    }
}

impl JavaClassRef for GlobalRef<ClassKind> {}

// JNI global references are VM-scoped handles and may be used from any attached thread.
// Local references remain thread-affine through `LocalRef`'s Rc marker.
unsafe impl<K> Send for GlobalRef<K> {}
unsafe impl<K> Sync for GlobalRef<K> {}

impl<'env, K> Drop for LocalRef<'env, K> {
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }

        let env = unsafe { std::ptr::NonNull::new_unchecked(self.env) };
        let delete_local_ref =
            unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
        unsafe { delete_local_ref(self.env, self.raw) };
    }
}

impl<K> Drop for GlobalRef<K> {
    fn drop(&mut self) {
        if self.raw.is_null() {
            return;
        }

        if let Ok(env) = self.vm.attach_current_thread() {
            unsafe { env.delete_global_ref_raw(self.raw) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    assert_not_impl_any!(LocalRef<'static, ObjectKind>: Send, Sync);
    assert_not_impl_any!(LocalRef<'static, ClassKind>: Send, Sync);
    assert_impl_all!(GlobalRef<ObjectKind>: Send, Sync);
    assert_impl_all!(GlobalRef<ClassKind>: Send, Sync);
}

impl<'env, K> From<&LocalRef<'env, K>> for JavaValue {
    fn from(value: &LocalRef<'env, K>) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

impl<K> From<&GlobalRef<K>> for JavaValue {
    fn from(value: &GlobalRef<K>) -> Self {
        Self::object_ref(value.as_jobject())
    }
}
