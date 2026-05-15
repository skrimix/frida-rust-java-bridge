use std::{
    ptr::{self, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use crate::{
    ClassLoaderKind, ClassLoaderRef, Error, Java, JavaReturn, JavaValue, Result, Runtime, env::Env,
    experimental, jni, refs::AsJObject,
};

const SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.SmokeSubject";

static REPLACEMENT_STRING: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static EXPECTED_RECEIVER: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
static EXPECTED_ARGUMENT: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());

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

    run_replacement_checks(&java, &app_java).map_err(error_string)?;
    Ok(())
}

fn run_replacement_checks(java: &Java, app_java: &Java) -> Result<()> {
    let capabilities = java.capabilities();
    let Some(reason) = capabilities.method_replacement.unsupported_reason() else {
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: "method replacement capability unexpectedly reported supported".to_owned(),
        });
    };
    if !reason.contains("prerequisites are available") {
        println!("app_process_smoke: skipping replacement checks: {reason}");
        return Ok(());
    }

    let subject = app_java.find_class(SMOKE_SUBJECT)?;
    let cached_subject = app_java.find_class(SMOKE_SUBJECT)?;
    let wrapper = app_java.use_class(SMOKE_SUBJECT)?;

    println!("app_process_smoke: checking app-loader static replacement");
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        42,
        "answer original",
    )?;
    let replacement =
        unsafe { experimental::replace_static_i32_method(&subject, "answer", replacement_answer)? };
    if let Some(summary) = replacement.debug_summary() {
        println!("app_process_smoke: static replacement layout {summary}");
    }
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        1337,
        "answer replacement",
    )?;
    expect_int(
        cached_subject.call_static("answer", "()I", &[])?,
        1337,
        "cached answer replacement",
    )?;
    expect_int(
        wrapper.call_static("answer", "()I", &[])?,
        1337,
        "wrapper answer replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        42,
        "answer restored",
    )?;

    println!("app_process_smoke: checking app-loader overload isolation");
    let object = subject.new_object("(I)V", &[JavaValue::Int(31)])?;
    let second_object = subject.new_object("(I)V", &[JavaValue::Int(32)])?;
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    expect_string(
        subject.call_method(&object, "overload", "()Ljava/lang/String;", &[])?,
        Some("no-args"),
        "overload() original",
    )?;
    let input = java.new_string_utf("app-process-argument")?;
    let output = java.new_string_utf("app-process-replacement")?;
    REPLACEMENT_STRING.store(output.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(input.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_instance_string_to_string_method(
            &subject,
            "overload",
            replacement_overload,
        )?
    };
    expect_string(
        subject.call_method(&object, "overload", "()Ljava/lang/String;", &[])?,
        Some("no-args"),
        "overload() during overload(String) replacement",
    )?;
    expect_string(
        subject.call_method(
            &object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-replacement"),
        "overload(String) replacement",
    )?;
    expect_string(
        cached_subject.call_method(
            &object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-replacement"),
        "cached overload(String) replacement",
    )?;
    expect_string(
        wrapper.call(
            &object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-replacement"),
        "wrapper overload(String) replacement",
    )?;
    expect_string(
        subject.call_method(
            &second_object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        None,
        "second receiver overload(String) replacement",
    )?;
    replacement.revert()?;
    expect_string(
        subject.call_method(
            &object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-argument"),
        "overload(String) restored",
    )?;

    println!("app_process_smoke: checking app-loader instance replacement across receivers");
    let replacement = unsafe {
        experimental::replace_instance_i32_method(
            &subject,
            "instanceNumber",
            replacement_instance_number,
        )?
    };
    if let Some(summary) = replacement.debug_summary() {
        println!("app_process_smoke: instance replacement layout {summary}");
    }
    expect_int(
        subject.call_method(&object, "instanceNumber", "()I", &[])?,
        2026,
        "instanceNumber replacement",
    )?;
    expect_int(
        subject.call_method(&second_object, "instanceNumber", "()I", &[])?,
        -2,
        "second receiver instanceNumber replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(&object, "instanceNumber", "()I", &[])?,
        31,
        "instanceNumber restored",
    )?;
    expect_int(
        subject.call_method(&second_object, "instanceNumber", "()I", &[])?,
        32,
        "second receiver instanceNumber restored",
    )?;

    println!("app_process_smoke: checking private static replacement");
    match unsafe {
        experimental::replace_static_string_method(&subject, "hiddenStatic", replacement_string)
    } {
        Ok(replacement) => {
            let hidden = subject.call_static("hiddenStatic", "()Ljava/lang/String;", &[])?;
            expect_string(
                hidden,
                Some("app-process-replacement"),
                "hiddenStatic replacement",
            )?;
            replacement.revert()?;
        }
        Err(Error::MethodNotFound { .. })
        | Err(Error::JavaException {
            operation: "JNIEnv::GetStaticMethodID",
        }) => {
            println!("app_process_smoke: private static replacement lookup unavailable");
        }
        Err(error) => return Err(error),
    }

    REPLACEMENT_STRING.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_RECEIVER.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}

fn expect_int(value: JavaReturn, expected: i32, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Int(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("int {expected}"), other),
    }
}

fn expect_string(value: JavaReturn, expected: Option<&str>, operation: &'static str) -> Result<()> {
    match (value, expected) {
        (JavaReturn::Object(None), None) => Ok(()),
        (JavaReturn::Object(Some(value)), Some(expected)) if value.get_string()? == expected => {
            Ok(())
        }
        (other, expected) => replacement_mismatch(operation, format!("string {expected:?}"), other),
    }
}

fn replacement_mismatch<T>(
    operation: &'static str,
    expected: String,
    actual: JavaReturn,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature: "ART method replacement",
        reason: format!("{operation} mismatch: expected {expected}, got {actual:?}"),
    })
}

fn error_string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn new_raw_string(env: *mut jni::JNIEnv, text: &str) -> jni::jstring {
    let Some(env) = NonNull::new(env) else {
        return ptr::null_mut();
    };
    let Ok(runtime) = Runtime::obtain() else {
        return ptr::null_mut();
    };
    let vm = runtime.vm();
    let env = Env::from_raw(env, &vm);
    env.new_string_utf_raw(text).unwrap_or(ptr::null_mut())
}

unsafe extern "C" fn replacement_answer(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jint {
    1337
}

unsafe extern "C" fn replacement_string(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jstring {
    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

unsafe extern "C" fn replacement_instance_number(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jint {
    let expected = EXPECTED_RECEIVER.load(Ordering::SeqCst);
    if expected.is_null() || env.is_null() {
        return -1;
    }
    if unsafe { raw_is_same_object(env, receiver, expected) } {
        2026
    } else {
        -2
    }
}

unsafe extern "C" fn replacement_overload(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jstring,
) -> jni::jstring {
    let expected_receiver = EXPECTED_RECEIVER.load(Ordering::SeqCst);
    if expected_receiver.is_null()
        || env.is_null()
        || !unsafe { raw_is_same_object(env, receiver, expected_receiver) }
    {
        return ptr::null_mut();
    }

    if argument.is_null() {
        return REPLACEMENT_STRING.load(Ordering::SeqCst);
    }

    let expected_argument = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
    if expected_argument.is_null()
        || !unsafe { raw_is_same_object(env, argument, expected_argument) }
    {
        return ptr::null_mut();
    }

    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

unsafe fn raw_is_same_object(
    env: *mut jni::JNIEnv,
    left: jni::jobject,
    right: jni::jobject,
) -> bool {
    let env = unsafe { NonNull::new_unchecked(env) };
    let is_same_object =
        unsafe { jni::env_function::<jni::IsSameObject>(env, jni::ENV_IS_SAME_OBJECT) };
    unsafe { is_same_object(env.as_ptr(), left, right) == jni::JNI_TRUE }
}
