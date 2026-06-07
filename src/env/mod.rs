//! Safe, typesafe Rust abstractions over the active JNI environment.
//!
//! While high-level code should always prefer [`crate::Java::use_class`] and high-level wrapper types,
//! the `Env` struct represents a safe, idiomatic Rust translation of the standard JNI environment (`JNIEnv`).
//!
//! Use `Env` when your code requires standard, JNI-style operations such as:
//! - Direct method and field ID lookups.
//! - Manual method invocations and field access.
//! - Creating and managing local or global JNI references.
//! - Direct manipulation of JNI strings, arrays, and exceptions.
//!
//! ### Thread Affinity & Lifecycles
//!
//! A JNI environment is strictly bound to the thread that attached to the Java Virtual Machine.
//! Consequently, `Env` is thread-affine (it is `!Send` and `!Sync`) and is valid only within the lexical
//! scope of an attached thread scope.

use std::{
    ffi::{CStr, CString},
    marker::PhantomData,
    ptr::{self, NonNull},
    rc::Rc,
    sync::Arc,
};

use crate::{
    error::{Error, JavaThrowableOwner, Result},
    jni,
    refs::{
        ArrayRef, AsJClass, AsJObject, ClassRef, GlobalRef, GlobalRefOwner, LocalRef,
        LocalRefScope, ObjectArrayRef, ObjectRef, StringRef, ThrowableRef,
    },
    signature::{JavaType, MethodSignature},
    value::JavaValue,
};

#[macro_use]
pub(crate) mod macros;

mod arrays;
mod calls;
mod exceptions;
mod fields;
mod ids;
mod members;
mod references;
mod strings;

pub(crate) use exceptions::{
    PendingJavaException, check_pending_exception_preserve_raw, check_pending_exception_raw,
    throw_new_illegal_state_exception_if_clear_raw,
};
pub use ids::{FieldId, FieldKind, MethodId, MethodKind};

/// JNI environment view for one attached thread.
///
/// An `Env` does not detach the thread. It is borrowed from an existing attachment or callback and
/// is valid only on the current thread while that attachment/local frame remains alive.
pub struct Env<'vm> {
    handle: NonNull<jni::JNIEnv>,
    owner: Arc<dyn EnvOwner>,
    _thread_affine: PhantomData<Rc<()>>,
    _vm: PhantomData<&'vm ()>,
}

/// Owns or borrows a thread attachment and exposes its [`Env`].
///
/// Values returned by high-level attachment helpers detach on drop only when this crate attached
/// the thread for the caller. If the thread was already attached, drop leaves it attached.
pub struct AttachedEnv<'vm> {
    env: Env<'vm>,
    detach_on_drop: bool,
}

pub(crate) trait EnvOwner: GlobalRefOwner + JavaThrowableOwner {
    unsafe fn detach_current_thread(&self) -> Result<()>;
}

impl<'vm> Env<'vm> {
    pub(crate) fn from_raw(handle: NonNull<jni::JNIEnv>, owner: impl EnvOwner + 'static) -> Self {
        Self {
            handle,
            owner: Arc::new(owner),
            _thread_affine: PhantomData,
            _vm: PhantomData,
        }
    }

    /// Returns the raw JNI environment pointer for the current thread.
    ///
    /// # Safety
    ///
    /// The caller must not use the returned pointer after this `Env`'s JNI attachment or local
    /// frame has ended, must not use it from a different thread, and must uphold the JNI contract
    /// for any raw calls made with it.
    pub unsafe fn handle(&self) -> NonNull<jni::JNIEnv> {
        self.handle
    }

    pub(crate) fn local_ref_scope(&self) -> LocalRefScope<'_> {
        LocalRefScope::from_raw(self.handle)
    }

    /// Returns the JNI version reported by this environment.
    pub fn version(&self) -> jni::jint {
        let get_version = self.function::<jni::GetVersion>(jni::ENV_GET_VERSION);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { get_version(self.handle.as_ptr()) }
    }

    fn check_pending_exception(&self, operation: &'static str) -> Result<()> {
        unsafe { exceptions::check_pending_exception(self.handle, self.owner.clone(), operation) }
    }

    fn function<T: Copy>(&self, slot: usize) -> T {
        unsafe { jni::env_function(self.handle, slot) }
    }
}

fn jni_args(args: &[JavaValue]) -> Vec<jni::jvalue> {
    args.iter().copied().map(JavaValue::to_jvalue).collect()
}

fn jni_args_ptr(args: &[jni::jvalue]) -> *const jni::jvalue {
    if args.is_empty() {
        std::ptr::null()
    } else {
        args.as_ptr()
    }
}

impl<'vm> AttachedEnv<'vm> {
    pub(crate) fn new(env: Env<'vm>, detach_on_drop: bool) -> Self {
        Self {
            env,
            detach_on_drop,
        }
    }

    /// Returns the attached thread's JNI environment.
    pub fn env(&self) -> &Env<'vm> {
        &self.env
    }

    /// Returns whether this guard will detach the current thread when dropped.
    pub fn detach_on_drop(&self) -> bool {
        self.detach_on_drop
    }
}

impl<'vm> std::ops::Deref for AttachedEnv<'vm> {
    type Target = Env<'vm>;

    fn deref(&self) -> &Self::Target {
        &self.env
    }
}

impl Drop for AttachedEnv<'_> {
    fn drop(&mut self) {
        if self.detach_on_drop {
            // SAFETY: `AttachedEnv` owns the attachment it created and drops after its contained
            // `Env` has stopped being externally accessible through safe references.
            let _ = unsafe { self.env.owner.detach_current_thread() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use static_assertions::assert_not_impl_any;

    assert_not_impl_any!(Env<'static>: Send, Sync);
    assert_not_impl_any!(AttachedEnv<'static>: Send, Sync);

    fn method(kind: MethodKind, return_type: JavaType) -> MethodId {
        MethodId {
            raw: std::ptr::dangling_mut(),
            kind,
            signature: MethodSignature::new(Vec::new(), return_type),
        }
    }

    fn field(kind: FieldKind, ty: JavaType) -> FieldId {
        FieldId {
            raw: std::ptr::dangling_mut(),
            kind,
            ty,
        }
    }

    #[test]
    fn method_return_guards_accept_matching_kinds_and_reference_returns() {
        let instance_object = method(
            MethodKind::Instance,
            JavaType::Object("java/lang/String".to_owned()),
        );
        assert_eq!(
            instance_object.ensure_instance_return(JavaType::Object(String::new()), "test"),
            Ok(())
        );

        let static_array = method(MethodKind::Static, JavaType::Array(Box::new(JavaType::Int)));
        assert_eq!(
            static_array.ensure_static_return(JavaType::Object(String::new()), "test"),
            Ok(())
        );
    }

    #[test]
    fn method_return_guards_report_kind_and_type_mismatches() {
        let static_int = method(MethodKind::Static, JavaType::Int);
        assert_eq!(
            static_int.ensure_instance_return(JavaType::Int, "test"),
            Err(Error::WrongMethodKind { operation: "test" })
        );

        let instance_long = method(MethodKind::Instance, JavaType::Long);
        assert_eq!(
            instance_long.ensure_instance_return(JavaType::Int, "test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "int",
                actual: "J".to_owned(),
            })
        );
    }

    #[test]
    fn field_type_guards_accept_matching_kinds_and_reference_fields() {
        let instance_object = field(
            FieldKind::Instance,
            JavaType::Object("java/lang/String".to_owned()),
        );
        assert_eq!(
            instance_object.ensure_instance_type(JavaType::Object(String::new()), "test"),
            Ok(())
        );

        let static_array = field(FieldKind::Static, JavaType::Array(Box::new(JavaType::Int)));
        assert_eq!(
            static_array.ensure_static_type(JavaType::Object(String::new()), "test"),
            Ok(())
        );
    }

    #[test]
    fn field_type_guards_report_kind_and_type_mismatches() {
        let static_int = field(FieldKind::Static, JavaType::Int);
        assert_eq!(
            static_int.ensure_instance_type(JavaType::Int, "test"),
            Err(Error::WrongFieldKind { operation: "test" })
        );

        let instance_long = field(FieldKind::Instance, JavaType::Long);
        assert_eq!(
            instance_long.ensure_instance_type(JavaType::Int, "test"),
            Err(Error::InvalidFieldType {
                operation: "test",
                expected: "int",
                actual: "J".to_owned(),
            })
        );
    }

    #[test]
    fn jni_argument_buffers_use_null_for_empty_slices() {
        let empty = jni_args(&[]);
        assert!(jni_args_ptr(&empty).is_null());

        let args = jni_args(&[JavaValue::Int(42), JavaValue::NULL]);
        assert_eq!(args.len(), 2);
        assert!(!jni_args_ptr(&args).is_null());
        assert_eq!(unsafe { args[0].i }, 42);
        assert!(unsafe { args[1].l }.is_null());
    }
}
