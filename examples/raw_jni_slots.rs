#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, Result, jni};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;

    java.perform_now(|java| {
        let (register_natives, find_class) = raw_jni_slot_probe(&java)?;
        println!("RegisterNatives = {register_natives:p}");
        println!("FindClass = {find_class:p}");
        Ok(())
    })
}

#[cfg(target_os = "android")]
fn raw_jni_slot_probe(java: &Java) -> Result<(*const std::ffi::c_void, *const std::ffi::c_void)> {
    use std::{ffi::c_void, ptr::NonNull};

    unsafe fn get_native_address(env: NonNull<jni::JNIEnv>, slot: usize) -> *const c_void {
        let functions = unsafe { *(env.as_ptr().cast::<*const *const c_void>()) };
        unsafe { *functions.add(slot) }
    }

    let env = java.vm().attach_current_thread()?;
    let env_handle = unsafe { env.handle() };

    const REGISTER_NATIVES: usize = 215;
    const FIND_CLASS: usize = 6;

    let register_natives = unsafe { get_native_address(env_handle, REGISTER_NATIVES) };
    let find_class = unsafe { get_native_address(env_handle, FIND_CLASS) };

    Ok((register_natives, find_class))
}
