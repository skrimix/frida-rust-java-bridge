//! Rust-safe JNI reference wrappers and ownership types.
//!
//! When working with Java from Rust, references to Java objects must be managed carefully to avoid
//! memory leaks or accessing reclaimed memory. This module provides safe Rust types that manage JNI local
//! and global reference lifecycles.
//!
//! Most of the time, you should use the high-level wrappers like [`crate::JavaObject`], [`crate::JavaArray`],
//! and [`crate::JavaClass`]. Use this module when you are working at the raw JNI boundary or building
//! custom low-level abstractions.
//!
//! ### Types of References
//!
//! - **Local References (`LocalRef`, `BorrowedLocalRef`):** Bound to the current Java execution frame and thread.
//!   They cannot be sent to other threads and are automatically cleaned up when the current scope ends.
//! - **Global References (`GlobalRef`):** Kept alive indefinitely by the process ART runtime. They can be safely
//!   moved across Rust threads, although performing Java operations on them still requires the thread to attach to the VM.

use std::{marker::PhantomData, ptr, ptr::NonNull, rc::Rc};

use crate::{
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

/// Owning local reference to any Java object.
pub type ObjectRef<'env> = LocalRef<'env, ObjectKind>;
/// Owning local reference to a Java `Class`.
pub type ClassRef<'env> = LocalRef<'env, ClassKind>;
/// Owning local reference to a Java `String`.
pub type StringRef<'env> = LocalRef<'env, StringKind>;
/// Owning local reference to a Java `Throwable`.
pub type ThrowableRef<'env> = LocalRef<'env, ThrowableKind>;
/// Owning local reference to a Java array.
pub type ArrayRef<'env> = LocalRef<'env, ArrayKind>;
/// Owning local reference to a Java object array.
pub type ObjectArrayRef<'env> = LocalRef<'env, ObjectArrayKind>;

/// A borrowed JNI local reference view that is valid only for the producing callback/JNI frame.
///
/// Unlike [`LocalRef`], this type does not own the local reference and does not delete it on drop.
/// It is used for references handed to replacement callbacks by ART/JNI.
pub struct BorrowedLocalRef<'local, K> {
    raw: jni::jobject,
    _local: PhantomData<&'local ()>,
    _kind: PhantomData<K>,
    _thread_affine: PhantomData<Rc<()>>,
}

/// An owning JNI local reference tied to an [`crate::env::Env`] lifetime.
///
/// The reference is deleted when this value is dropped. It is thread-affine and must not escape the
/// JNI frame that produced it.
pub struct LocalRef<'env, K> {
    raw: jni::jobject,
    env: *mut jni::JNIEnv,
    _env: PhantomData<&'env ()>,
    _kind: PhantomData<K>,
    _thread_affine: PhantomData<Rc<()>>,
}

/// Raw JNI local-reference scope tied to an attached environment lifetime.
pub(crate) struct LocalRefScope<'env> {
    env: NonNull<jni::JNIEnv>,
    _env: PhantomData<&'env ()>,
    _thread_affine: PhantomData<Rc<()>>,
}

/// An owning JNI global reference for the process ART runtime.
///
/// Global references can be moved across Rust threads, but Java operations still require an
/// attached thread.
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

// Internal helpers intentionally mirror the sealed public traits. They let crate-only raw wrapper
// views participate in JNI helper bounds without making those views public JavaObjectRef values.
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

impl<'env> LocalRefScope<'env> {
    pub(crate) fn from_raw(env: NonNull<jni::JNIEnv>) -> Self {
        Self {
            env,
            _env: PhantomData,
            _thread_affine: PhantomData,
        }
    }
}

impl<'env> Copy for LocalRefScope<'env> {}

impl<'env> Clone for LocalRefScope<'env> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<'env, K> LocalRef<'env, K> {
    pub(crate) unsafe fn from_raw(scope: LocalRefScope<'env>, raw: jni::jobject) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JNI local reference",
            });
        }

        Ok(Self {
            raw,
            env: scope.env.as_ptr(),
            _env: PhantomData,
            _kind: PhantomData,
            _thread_affine: PhantomData,
        })
    }

    pub(crate) unsafe fn from_nullable(
        scope: LocalRefScope<'env>,
        raw: jni::jobject,
    ) -> Option<Self> {
        if raw.is_null() {
            None
        } else {
            Some(Self {
                raw,
                env: scope.env.as_ptr(),
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

impl<'local, K> BorrowedLocalRef<'local, K> {
    pub(crate) unsafe fn from_raw(raw: jni::jobject, operation: &'static str) -> Result<Self> {
        if raw.is_null() {
            return Err(Error::NullReturn { operation });
        }

        Ok(Self {
            raw,
            _local: PhantomData,
            _kind: PhantomData,
            _thread_affine: PhantomData,
        })
    }

    /// Returns the raw borrowed JNI local reference.
    ///
    /// # Safety
    ///
    /// The returned handle is valid only for the producing callback/JNI frame on the current
    /// thread. The caller must not delete it.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        self.raw
    }
}

impl<K> std::fmt::Debug for BorrowedLocalRef<'_, K> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fmt.debug_tuple("BorrowedLocalRef")
            .field(&self.raw)
            .finish()
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

    #[cfg(test)]
    pub(crate) fn null_for_tests(vm: Vm) -> Self {
        Self {
            raw: ptr::null_mut(),
            vm,
            _kind: PhantomData,
        }
    }

    pub(crate) fn vm(&self) -> &Vm {
        &self.vm
    }

    /// Returns the raw JNI global reference.
    ///
    /// # Safety
    ///
    /// The caller must not delete the returned reference. It is valid for this process' ART runtime.
    pub unsafe fn raw_jobject(&self) -> jni::jobject {
        self.raw
    }

    /// Leaks ownership of the global JNI reference and returns the raw handle.
    ///
    /// # Safety
    ///
    /// The caller becomes responsible for deleting the global reference on an attached thread.
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
    /// The caller must not delete the returned reference. It is valid for this process' ART runtime.
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

impl<'local, K> sealed::JavaObjectRefSealed for BorrowedLocalRef<'local, K> {
    fn as_jobject(&self) -> jni::jobject {
        self.raw
    }
}

impl<'local, K> JavaObjectRef for BorrowedLocalRef<'local, K> {}

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

// JNI global references may be used from any thread attached to the process ART runtime.
// Local references remain thread-affine through `LocalRef`'s Rc marker.
unsafe impl<K> Send for GlobalRef<K> {}
unsafe impl<K> Sync for GlobalRef<K> {}

impl<K> From<&GlobalRef<K>> for JavaValue {
    fn from(value: &GlobalRef<K>) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

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

        self.vm.delete_global_ref_best_effort(self.raw);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::{assert_impl_all, assert_not_impl_any};

    assert_not_impl_any!(LocalRef<'static, ObjectKind>: Send, Sync);
    assert_not_impl_any!(LocalRef<'static, ClassKind>: Send, Sync);
    assert_not_impl_any!(BorrowedLocalRef<'static, ObjectKind>: Send, Sync);
    assert_not_impl_any!(BorrowedLocalRef<'static, ClassKind>: Send, Sync);
    assert_impl_all!(GlobalRef<ObjectKind>: Send, Sync);
    assert_impl_all!(GlobalRef<ClassKind>: Send, Sync);

    #[test]
    fn borrowed_local_ref_wraps_raw_without_owning_it() {
        let raw = std::ptr::dangling_mut();
        let reference =
            unsafe { BorrowedLocalRef::<ObjectKind>::from_raw(raw, "test borrowed local") }
                .unwrap();

        assert_eq!(unsafe { reference.raw_jobject() }, raw);
    }

    #[test]
    fn borrowed_local_ref_rejects_null_raw() {
        assert_eq!(
            unsafe {
                BorrowedLocalRef::<ObjectKind>::from_raw(ptr::null_mut(), "test borrowed local")
            }
            .unwrap_err(),
            Error::NullReturn {
                operation: "test borrowed local",
            }
        );
    }
}
