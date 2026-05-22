#![cfg_attr(not(target_os = "android"), allow(unused))]
#![allow(private_bounds)]
#![allow(private_interfaces)]

#[cfg(target_os = "android")]
pub(crate) mod android;
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
pub mod java;
pub mod jni;
#[cfg(target_os = "android")]
pub mod metadata;
pub mod modifiers;
#[cfg(target_os = "android")]
pub mod refs;
#[cfg(target_os = "android")]
pub mod replacement;
#[cfg(target_os = "android")]
mod runtime;
pub mod signature;
pub mod value;
#[cfg(target_os = "android")]
pub mod vm;

#[cfg(target_os = "android")]
pub use android::AndroidVersion;
#[cfg(target_os = "android")]
pub use env::{AttachedEnv, Env, FieldId, FieldKind, MethodId, MethodKind};
pub use error::{Error, Result};
#[cfg(target_os = "android")]
pub use java::{
    AttachedJava, ClassLoaderKind, ClassLoaderRef, FromJavaReturn, IntoJavaFieldValue, Java,
    JavaArray, JavaChooseControl, JavaClass, JavaConstructor, JavaField, JavaLocalArray,
    JavaLocalObject, JavaLocalRef, JavaMethod, JavaObject, JavaRef, JavaReturn,
    MainThreadTaskHandle, MainThreadTaskStatus, PerformHandle, PerformStatus,
};
#[cfg(target_os = "android")]
pub use metadata::{
    JavaClassMetadata, JavaFieldMetadata, JavaMethodMetadata, JavaMethodQueryClass,
    JavaMethodQueryGroup,
};
pub use modifiers::{
    ACC_ABSTRACT, ACC_BRIDGE, ACC_FINAL, ACC_NATIVE, ACC_PRIVATE, ACC_PROTECTED, ACC_PUBLIC,
    ACC_STATIC, ACC_STRICT, ACC_SYNCHRONIZED, ACC_SYNTHETIC, ACC_VARARGS,
};
#[cfg(target_os = "android")]
pub use runtime::{FeatureSupport, JavaCapabilities, RuntimeFlavor};
pub use signature::{JavaType, MethodSignature};
pub use value::{JavaValue, RawJavaObject};
#[cfg(target_os = "android")]
pub use vm::Vm;
