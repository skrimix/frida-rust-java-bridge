use std::{
    ptr::{self, NonNull},
    sync::{
        Arc,
        atomic::{AtomicI32, AtomicPtr, AtomicUsize, Ordering},
    },
};

use crate::{
    ACC_PRIVATE, ACC_STATIC, AsJavaHookReturn, ClassLoaderKind, ClassLoaderRef, Error, Java,
    JavaArray, JavaChooseControl, JavaClass, JavaFieldMetadata, JavaLocalArray, JavaLocalObject,
    JavaMethod, JavaMethodMetadata, JavaObject, JavaReturn, JavaType, JavaValue,
    MainThreadTaskStatus, PerformResult, PerformStatus, Result,
    env::{Env, FieldKind, MethodKind},
    java::RawJavaClass,
    jni,
    refs::AsJObject,
    replacement,
};

mod assertions;
mod checks;
mod replacement_checks;
mod replacement_lifecycle;

use assertions::{error_string, new_raw_string};

const TEST_SUBJECT: &str = "frida.java.bridge.rs.test.TestSubject";
const DEX_TEST_SUBJECT: &str = "frida.java.bridge.rs.test.DexTestSubject";
const DEX_TEST_PATH: &str = "/data/local/tmp/frida-java-bridge-rs/dex-test-fixture.dex";
const DEX_TEST_OPT: &str = "/data/local/tmp/frida-java-bridge-rs/dex-cache";

static REPLACEMENT_STRING: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static REPLACEMENT_OBJECT: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static EXPECTED_RECEIVER: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static EXPECTED_ARGUMENT: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static VOID_REPLACEMENT_COUNTER: AtomicI32 = AtomicI32::new(0);
struct RawObject(jni::jobject);

impl AsJObject for RawObject {
    fn as_jobject(&self) -> jni::jobject {
        self.0
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn Java_frida_java_bridge_rs_test_AppProcessTest_nativeRun(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    loader: jni::jobject,
) -> jni::jstring {
    match run(env, loader) {
        Ok(()) => new_raw_string(env, "ok"),
        Err(error) => new_raw_string(env, &format!("app_process_test: {error}")),
    }
}

fn run(env: *mut jni::JNIEnv, loader: jni::jobject) -> std::result::Result<(), String> {
    let env = NonNull::new(env).ok_or("JNIEnv was null".to_owned())?;
    if loader.is_null() {
        return Err("ClassLoader argument was null".to_owned());
    }

    let java = Java::obtain().map_err(error_string)?;
    // app_process is a short-lived test target, and some ART/Gum teardown paths run after
    // runtime shutdown has started. Keep the process-global Java state alive until exit.
    std::mem::forget(java.clone());
    let call_env = Env::from_raw(env, java.vm());
    let loader = ClassLoaderRef::from_object_ref(
        &call_env,
        java.vm(),
        &RawObject(loader),
        ClassLoaderKind::Object,
    )
    .map_err(error_string)?;
    let app_java = java.with_loader(&loader);

    checks::run_low_level_checks(&call_env).map_err(error_string)?;
    checks::run_convenience_checks(&java, &app_java).map_err(error_string)?;
    replacement_checks::run_replacement_checks(&java, &app_java).map_err(error_string)?;
    Ok(())
}
