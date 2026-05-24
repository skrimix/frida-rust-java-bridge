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
pub use error::{Error, Result};
#[cfg(target_os = "android")]
pub use java::{
    ClassLoaderKind, ClassLoaderRef, FromJavaReturn, IntoJavaFieldValue, Java, JavaArgs, JavaArray,
    JavaChooseControl, JavaClass, JavaConstructor, JavaField, JavaLocalArray, JavaLocalObject,
    JavaLocalRef, JavaLocalReturn, JavaMethod, JavaObject, JavaRef, JavaReturn, JavaScope,
    MainThreadTaskHandle, MainThreadTaskStatus, PerformHandle, PerformResult, PerformStatus,
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
pub use replacement::{
    FromJavaHookReturn, FromJavaValue, IntoJavaHookReturn, JavaConstructorHookContext,
    JavaConstructorInitialized, JavaHookArgument, JavaHookArguments, JavaHookContext,
    JavaHookError, JavaHookGuard, JavaHookReturn, JavaHookSet, JavaHookTarget,
    UnsafeJavaHookTarget,
};
#[cfg(target_os = "android")]
pub use runtime::{FeatureSupport, JavaCapabilities, RuntimeFlavor};
pub use signature::{JavaType, MethodSignature};
pub use value::JavaValue;

/// Builds an explicit Java argument list for calls with many arguments.
///
/// Most method calls can pass `()`, a single value, a tuple, an array, or a slice directly. Use this
/// macro when an argument list is longer than the supported tuple arities or when building a list
/// incrementally would be less clear.
#[cfg(target_os = "android")]
#[macro_export]
macro_rules! java_args {
    ($($value:expr),* $(,)?) => {{
        let mut args = $crate::JavaArgs::with_capacity(0usize $(+ {
            let _ = stringify!($value);
            1usize
        })*);
        $(
            args.push($value);
        )*
        args
    }};
}
