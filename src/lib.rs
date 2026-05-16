#![cfg_attr(not(target_os = "android"), allow(unused))]

#[cfg(all(target_os = "android", feature = "apk-perform-test"))]
mod apk_perform_test;
#[cfg(all(target_os = "android", feature = "app-process-test"))]
mod app_process_test;
#[cfg(target_os = "android")]
pub(crate) mod art;
#[cfg(target_os = "android")]
pub mod env;
pub mod error;
#[cfg(target_os = "android")]
pub mod experimental;
#[cfg(target_os = "android")]
pub mod java;
pub mod jni;
#[cfg(target_os = "android")]
pub mod metadata;
#[cfg(target_os = "android")]
pub mod refs;
#[cfg(target_os = "android")]
pub mod runtime;
pub mod signature;
pub mod value;
#[cfg(target_os = "android")]
pub mod vm;

#[cfg(target_os = "android")]
pub use env::{AttachedEnv, Env, FieldId, FieldKind, MethodId, MethodKind};
pub use error::{Error, Result};
#[cfg(target_os = "android")]
pub use java::{
    ClassLoaderKind, ClassLoaderRef, Java, JavaArray, JavaClass, JavaClassWrapper,
    JavaConstructorOverload, JavaFieldHandle, JavaMethodOverload, JavaObject, JavaReturn,
    PerformHandle, PerformStatus,
};
#[cfg(target_os = "android")]
pub use metadata::{
    JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryClass,
    JavaMethodQueryGroup,
};
#[cfg(target_os = "android")]
pub use refs::{
    ArrayKind, ArrayRef, AsJClass, AsJObject, ClassKind, ClassRef, GlobalRef, LocalRef, ObjectKind,
    ObjectRef, StringKind, StringRef, ThrowableKind, ThrowableRef,
};
#[cfg(target_os = "android")]
pub use runtime::{FeatureSupport, Runtime, RuntimeCapabilities, RuntimeFlavor};
pub use signature::{JavaType, MethodSignature};
pub use value::JavaValue;
#[cfg(target_os = "android")]
pub use vm::Vm;
