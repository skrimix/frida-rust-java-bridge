#![cfg_attr(not(target_os = "android"), allow(unused))]

#[cfg(target_os = "android")]
pub mod env;
pub mod error;
#[cfg(target_os = "android")]
pub mod java;
pub mod jni;
#[cfg(target_os = "android")]
pub mod refs;
#[cfg(target_os = "android")]
pub mod runtime;
pub mod signature;
pub mod value;
#[cfg(target_os = "android")]
pub mod vm;

#[cfg(target_os = "android")]
pub use env::{AttachedEnv, Env, FieldKind, FieldRef, MethodKind, MethodRef};
pub use error::{Error, Result};
#[cfg(target_os = "android")]
pub use java::{Java, JavaClass, JavaObject, JavaReturn};
#[cfg(target_os = "android")]
pub use refs::{
    AsJClass, AsJObject, ClassKind, ClassRef, GlobalRef, LocalRef, ObjectKind, ObjectRef,
    StringKind, StringRef, ThrowableKind, ThrowableRef,
};
#[cfg(target_os = "android")]
pub use runtime::{Runtime, RuntimeFlavor};
pub use signature::{JavaType, MethodSignature};
pub use value::JavaValue;
#[cfg(target_os = "android")]
pub use vm::Vm;
