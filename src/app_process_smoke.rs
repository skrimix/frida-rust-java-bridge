use std::{
    ptr::{self, NonNull},
    sync::{
        OnceLock,
        atomic::{AtomicI32, AtomicPtr, Ordering},
    },
};

use crate::{
    ClassLoaderKind, ClassLoaderRef, Error, FieldKind, Java, JavaClass, JavaClassWrapper,
    JavaFieldMetadata, JavaMethodMetadata, JavaObject, JavaReturn, JavaType, JavaValue, MethodKind,
    Result, Runtime, RuntimeFlavor, env::Env, experimental, jni, refs::AsJObject,
};

mod assertions;
mod checks;
mod replacement_callbacks;
mod replacement_checks;
mod replacement_lifecycle;

use assertions::{error_string, new_raw_string};

const SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.SmokeSubject";
const DEX_SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.DexSmokeSubject";
const DEX_SMOKE_PATH: &str = "/data/local/tmp/frida-java-bridge-rs/dex-smoke-fixture.dex";
const DEX_SMOKE_OPT: &str = "/data/local/tmp/frida-java-bridge-rs/dex-cache";

static REPLACEMENT_STRING: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static REPLACEMENT_OBJECT: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static EXPECTED_RECEIVER: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static EXPECTED_ARGUMENT: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static VOID_REPLACEMENT_COUNTER: AtomicI32 = AtomicI32::new(0);
static FACADE_STATIC_ANSWER_ORIGINAL: OnceLock<experimental::OriginalMethod> = OnceLock::new();
static STATIC_OBJECT_ARRAY_ECHO_ORIGINAL: OnceLock<experimental::OriginalMethod> = OnceLock::new();
static INSTANCE_ADD_ORIGINAL: OnceLock<experimental::OriginalMethod> = OnceLock::new();
static INSTANCE_OBJECT_ARRAY_ECHO_ORIGINAL: OnceLock<experimental::OriginalMethod> =
    OnceLock::new();

struct RawObject(jni::jobject);

impl AsJObject for RawObject {
    fn as_jobject(&self) -> jni::jobject {
        self.0
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn Java_frida_java_bridge_rs_smoke_AppProcessSmoke_nativeRun(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    loader: jni::jobject,
) -> jni::jstring {
    match run(env, loader) {
        Ok(()) => new_raw_string(env, "ok"),
        Err(error) => new_raw_string(env, &format!("app_process_smoke: {error}")),
    }
}

fn run(env: *mut jni::JNIEnv, loader: jni::jobject) -> std::result::Result<(), String> {
    let env = NonNull::new(env).ok_or("JNIEnv was null".to_owned())?;
    if loader.is_null() {
        return Err("ClassLoader argument was null".to_owned());
    }

    let runtime = Runtime::obtain().map_err(error_string)?;
    // app_process is a short-lived smoke target, and some ART/Gum teardown paths run after
    // runtime shutdown has started. Keep the process-global runtime state alive until exit.
    std::mem::forget(runtime.clone());
    let vm = runtime.vm();
    let call_env = Env::from_raw(env, &vm);
    let loader = ClassLoaderRef::from_object_ref(
        &call_env,
        &vm,
        &RawObject(loader),
        ClassLoaderKind::Object,
    )
    .map_err(error_string)?;
    let java = runtime.java();
    let app_java = java.with_loader(&loader);

    checks::run_low_level_checks(&call_env).map_err(error_string)?;
    checks::run_convenience_checks(&runtime, &java, &app_java).map_err(error_string)?;
    replacement_checks::run_replacement_checks(&java, &app_java).map_err(error_string)?;
    Ok(())
}
