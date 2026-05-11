#![cfg_attr(not(target_os = "android"), allow(unused))]

#[cfg(not(target_os = "android"))]
compile_error!("frida-java-bridge-rs currently targets Android ART only; build with cargo ndk");

#[cfg(target_os = "android")]
pub mod env;
#[cfg(target_os = "android")]
pub mod error;
#[cfg(target_os = "android")]
pub mod jni;
#[cfg(target_os = "android")]
pub mod runtime;
#[cfg(target_os = "android")]
pub mod vm;

#[cfg(target_os = "android")]
pub use env::{AttachedEnv, Env};
#[cfg(target_os = "android")]
pub use error::{Error, Result};
#[cfg(target_os = "android")]
pub use runtime::{Runtime, RuntimeFlavor};
#[cfg(target_os = "android")]
pub use vm::Vm;
