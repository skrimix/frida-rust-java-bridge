#![cfg(target_os = "android")]

use std::ffi::{c_char, c_void};

use frida_rust_java_bridge::{art_selftest, jni};

/// JVM agent entrypoint used by the APK startup self-test.
///
/// # Safety
///
/// ART calls this with the normal `Agent_OnAttach` ABI. `options` must be null or a valid
/// null-terminated string for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Agent_OnAttach(
    _vm: *mut jni::JavaVM,
    options: *mut c_char,
    _reserved: *mut c_void,
) -> jni::jint {
    unsafe { art_selftest::apk_perform::agent_on_attach(options) }
}

/// JNI entrypoint used by the app_process self-test.
///
/// # Safety
///
/// ART calls this through JNI. `env` must be a valid `JNIEnv` for the current thread, and `loader`
/// must be a valid class-loader object reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_frida_rust_java_bridge_test_AppProcessTest_nativeRun(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    loader: jni::jobject,
) -> jni::jstring {
    unsafe { art_selftest::app_process::native_run(env, class, loader) }
}
