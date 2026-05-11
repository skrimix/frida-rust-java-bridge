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

pub type ObjectRef<'env> = LocalRef<'env, ObjectKind>;
pub type ClassRef<'env> = LocalRef<'env, ClassKind>;
pub type StringRef<'env> = LocalRef<'env, StringKind>;
pub type ThrowableRef<'env> = LocalRef<'env, ThrowableKind>;

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

pub trait AsJObject {
    fn as_jobject(&self) -> jni::jobject;
}

pub trait AsJClass: AsJObject {
    fn as_jclass(&self) -> jni::jclass;
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

    pub fn as_raw(&self) -> jni::jobject {
        self.raw
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.raw
    }

    pub fn into_raw(mut self) -> jni::jobject {
        let raw = self.raw;
        self.raw = ptr::null_mut();
        raw
    }
}

impl<'env> ClassRef<'env> {
    pub fn as_jclass(&self) -> jni::jclass {
        self.raw
    }
}

impl<'env> StringRef<'env> {
    pub fn as_jstring(&self) -> jni::jstring {
        self.raw
    }
}

impl<'env> ThrowableRef<'env> {
    pub fn as_jthrowable(&self) -> jni::jthrowable {
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

    pub fn as_raw(&self) -> jni::jobject {
        self.raw
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.raw
    }

    pub fn into_raw(mut self) -> jni::jobject {
        let raw = self.raw;
        self.raw = ptr::null_mut();
        raw
    }
}

impl GlobalRef<ClassKind> {
    pub fn as_jclass(&self) -> jni::jclass {
        self.raw
    }
}

impl<'env, K> AsJObject for LocalRef<'env, K> {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

impl<K> AsJObject for GlobalRef<K> {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

impl<'env> AsJClass for ClassRef<'env> {
    fn as_jclass(&self) -> jni::jclass {
        self.as_jclass()
    }
}

impl AsJClass for GlobalRef<ClassKind> {
    fn as_jclass(&self) -> jni::jclass {
        self.as_jclass()
    }
}

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

impl<'env, K> From<&LocalRef<'env, K>> for JavaValue {
    fn from(value: &LocalRef<'env, K>) -> Self {
        Self::Object(value.as_jobject())
    }
}

impl<K> From<&GlobalRef<K>> for JavaValue {
    fn from(value: &GlobalRef<K>) -> Self {
        Self::Object(value.as_jobject())
    }
}
