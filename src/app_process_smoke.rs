use std::{
    ptr::{self, NonNull},
    sync::atomic::{AtomicI32, AtomicPtr, Ordering},
};

use crate::{
    ClassLoaderKind, ClassLoaderRef, Error, FieldKind, Java, JavaFieldMetadata, JavaMethodMetadata,
    JavaObject, JavaReturn, JavaType, JavaValue, MethodKind, Result, Runtime, RuntimeFlavor,
    env::Env, experimental, jni, refs::AsJObject,
};

const SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.SmokeSubject";
const DEX_SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.DexSmokeSubject";
const DEX_SMOKE_PATH: &str = "/data/local/tmp/frida-java-bridge-rs/dex-smoke-fixture.dex";
const DEX_SMOKE_OPT: &str = "/data/local/tmp/frida-java-bridge-rs/dex-cache";

static REPLACEMENT_STRING: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());
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

    run_low_level_checks(&call_env).map_err(error_string)?;
    run_convenience_checks(&runtime, &java, &app_java).map_err(error_string)?;
    run_replacement_checks(&java, &app_java).map_err(error_string)?;
    Ok(())
}

fn run_low_level_checks(env: &Env) -> Result<()> {
    println!("app_process_smoke: checking low-level JNI helpers");
    let string_class = env.find_class("java/lang/String")?;
    let object_class = env.find_class("java/lang/Object")?;
    let math_class = env.find_class("java/lang/Math")?;
    let integer_class = env.find_class("java/lang/Integer")?;
    let atomic_integer_class = env.find_class("java/util/concurrent/atomic/AtomicInteger")?;
    let throwable_class = env.find_class("java/lang/Throwable")?;
    let runtime_exception_class = env.find_class("java/lang/RuntimeException")?;

    let string = env.new_string_utf("frida-java-bridge-rs")?;
    let copied = env.get_string(&string)?;
    if copied != "frida-java-bridge-rs" {
        return smoke_error(format!("string round-trip mismatch: {copied:?}"));
    }

    let object_ctor = env.get_constructor(&object_class, "()V")?;
    let object = env.new_object(&object_class, &object_ctor, &[])?;
    let hash_code = env.get_method(&object_class, "hashCode", "()I")?;
    let _ = env.call_int_method(&object, &hash_code, &[])?;

    let string_length = env.get_method(&string_class, "length", "()I")?;
    let length = env.call_int_method(&string, &string_length, &[])?;
    if length != "frida-java-bridge-rs".len() as i32 {
        return smoke_error(format!("string length mismatch: {length}"));
    }

    let abs = env.get_static_method(&math_class, "abs", "(I)I")?;
    let abs_value = env.call_static_int_method(&math_class, &abs, &[JavaValue::Int(-42)])?;
    if abs_value != 42 {
        return smoke_error(format!("Math.abs result mismatch: {abs_value}"));
    }

    let max_value = env.get_static_field(&integer_class, "MAX_VALUE", "I")?;
    let max_value = env.get_static_int_field(&integer_class, &max_value)?;
    if max_value != i32::MAX {
        return smoke_error(format!("Integer.MAX_VALUE mismatch: {max_value}"));
    }

    let atomic_ctor = env.get_constructor(&atomic_integer_class, "(I)V")?;
    let atomic = env.new_object(&atomic_integer_class, &atomic_ctor, &[JavaValue::Int(7)])?;
    let atomic_value = env.get_field(&atomic_integer_class, "value", "I")?;
    let value = env.get_int_field(&atomic, &atomic_value)?;
    if value != 7 {
        return smoke_error(format!("AtomicInteger.value mismatch: {value}"));
    }
    env.set_int_field(&atomic, &atomic_value, 19)?;
    let atomic_get = env.get_method(&atomic_integer_class, "get", "()I")?;
    let value = env.call_int_method(&atomic, &atomic_get, &[])?;
    if value != 19 {
        return smoke_error(format!(
            "AtomicInteger.get mismatch after field set: {value}"
        ));
    }

    let initial_message = env.new_string_utf("initial")?;
    let exception_ctor = env.get_constructor(&runtime_exception_class, "(Ljava/lang/String;)V")?;
    let exception = env.new_object(
        &runtime_exception_class,
        &exception_ctor,
        &[JavaValue::from(&initial_message)],
    )?;
    let detail_message = env.get_field(&throwable_class, "detailMessage", "Ljava/lang/String;")?;
    let message = env
        .get_object_field(&exception, &detail_message)?
        .ok_or_else(|| smoke_failure("Throwable.detailMessage unexpectedly null"))?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "initial" {
        return smoke_error(format!("Throwable.detailMessage mismatch: {message:?}"));
    }
    let updated_message = env.new_string_utf("updated")?;
    env.set_object_field(&exception, &detail_message, Some(&updated_message))?;
    let get_message = env.get_method(&throwable_class, "getMessage", "()Ljava/lang/String;")?;
    let message = env
        .call_object_method(&exception, &get_message, &[])?
        .ok_or_else(|| smoke_failure("Throwable.getMessage unexpectedly returned null"))?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "updated" {
        return smoke_error(format!(
            "Throwable.getMessage mismatch after field set: {message:?}"
        ));
    }

    match env.find_class("frida/java/bridge/rs/MissingSmokeClass") {
        Err(Error::JavaException {
            operation: "JNIEnv::FindClass",
        }) => {}
        Err(error) => return Err(error),
        Ok(_class) => return smoke_error("missing class unexpectedly resolved"),
    }
    if env.exception_check() {
        env.exception_clear();
        return smoke_error("pending exception was not cleared after failed FindClass");
    }

    Ok(())
}

fn run_convenience_checks(runtime: &Runtime, java: &Java, app_java: &Java) -> Result<()> {
    println!("app_process_smoke: checking convenience layer");
    let vm = runtime.vm();
    let capabilities = java.capabilities();
    if capabilities.flavor != RuntimeFlavor::Art {
        return smoke_error(format!(
            "unexpected runtime flavor {:?}",
            capabilities.flavor
        ));
    }
    if runtime.capabilities() != capabilities || vm.capabilities() != capabilities {
        return smoke_error("runtime, VM, and Java capability reports diverged");
    }
    if capabilities.heap_enumeration.is_supported()
        || capabilities
            .heap_enumeration
            .unsupported_reason()
            .is_none_or(|reason| !reason.contains("not implemented yet"))
    {
        return smoke_error(format!(
            "heap enumeration capability was not explicitly deferred: {:?}",
            capabilities.heap_enumeration
        ));
    }
    if capabilities.deoptimization.is_supported()
        || capabilities
            .deoptimization
            .unsupported_reason()
            .is_none_or(|reason| !reason.contains("not implemented yet"))
    {
        return smoke_error(format!(
            "deoptimization capability was not explicitly deferred: {:?}",
            capabilities.deoptimization
        ));
    }
    let method_replacement_reason = capabilities.method_replacement.unsupported_reason();
    println!("app_process_smoke: capabilities {capabilities:?}");
    println!(
        "app_process_smoke: method replacement capability reason {method_replacement_reason:?}"
    );
    if capabilities.method_replacement.is_supported() || method_replacement_reason.is_none() {
        return smoke_error(format!(
            "method replacement capability was not explicitly unsupported: {:?}",
            capabilities.method_replacement
        ));
    }

    check_bootstrap_convenience(java)?;
    check_app_loader_surface(java, app_java)?;
    check_dex_class_loader(java)?;
    check_metadata_and_enumeration(
        java,
        app_java,
        capabilities.loaded_class_enumeration.is_supported(),
        capabilities.class_loader_enumeration.is_supported(),
    )?;
    Ok(())
}

fn check_bootstrap_convenience(java: &Java) -> Result<()> {
    let string_class = java.find_class("java.lang.String")?;
    let math_class = java.find_class("java.lang.Math")?;
    let atomic_integer_class = java.find_class("java.util.concurrent.atomic.AtomicInteger")?;
    let throwable_class = java.find_class("java.lang.Throwable")?;
    let runtime_exception_class = java.find_class("java.lang.RuntimeException")?;

    let string = java.new_string_utf("frida-java-bridge-rs")?;
    let length = read_int(
        string_class.call_method(&string, "length", "()I", &[])?,
        "String.length",
    )?;
    if length != "frida-java-bridge-rs".len() as i32 {
        return smoke_error(format!("JavaClass String.length mismatch: {length}"));
    }
    let abs_value = read_int(
        math_class.call_static("abs", "(I)I", &[JavaValue::Int(-42)])?,
        "Math.abs",
    )?;
    if abs_value != 42 {
        return smoke_error(format!("JavaClass Math.abs result mismatch: {abs_value}"));
    }

    let atomic = atomic_integer_class.new_object("(I)V", &[JavaValue::Int(7)])?;
    let value = read_int(
        atomic_integer_class.get_field(&atomic, "value", "I")?,
        "AtomicInteger.value",
    )?;
    if value != 7 {
        return smoke_error(format!("JavaClass AtomicInteger.value mismatch: {value}"));
    }
    atomic_integer_class.set_field(&atomic, "value", "I", JavaValue::Int(19))?;
    let value = read_int(
        atomic_integer_class.call_method(&atomic, "get", "()I", &[])?,
        "AtomicInteger.get",
    )?;
    if value != 19 {
        return smoke_error(format!(
            "JavaClass AtomicInteger.get mismatch after field set: {value}"
        ));
    }

    let initial_message = java.new_string_utf("initial")?;
    let exception = runtime_exception_class.new_object(
        "(Ljava/lang/String;)V",
        &[JavaValue::from(&initial_message)],
    )?;
    let message = read_object(
        throwable_class.get_field(&exception, "detailMessage", "Ljava/lang/String;")?,
        "Throwable.detailMessage",
    )?
    .ok_or_else(|| smoke_failure("JavaClass Throwable.detailMessage unexpectedly null"))?;
    let message = message.get_string()?;
    if message != "initial" {
        return smoke_error(format!(
            "JavaClass Throwable.detailMessage mismatch: {message:?}"
        ));
    }
    let updated_message = java.new_string_utf("updated")?;
    throwable_class.set_field(
        &exception,
        "detailMessage",
        "Ljava/lang/String;",
        JavaValue::from(&updated_message),
    )?;
    let message = read_object(
        throwable_class.call_method(&exception, "getMessage", "()Ljava/lang/String;", &[])?,
        "Throwable.getMessage",
    )?
    .ok_or_else(|| smoke_failure("JavaClass Throwable.getMessage unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "updated" {
        return smoke_error(format!(
            "JavaClass Throwable.getMessage mismatch after field set: {message:?}"
        ));
    }

    println!("app_process_smoke: checking bootstrap Java.use-style wrapper");
    let string_wrapper = java.use_class("java.lang.String")?;
    let cached_string_wrapper = java.use_class("java.lang.String")?;
    if string_wrapper.name() != "java.lang.String"
        || cached_string_wrapper.class().name() != "java.lang.String"
    {
        return smoke_error("JavaClassWrapper String name mismatch");
    }
    if !string_wrapper
        .methods("length")?
        .iter()
        .any(|method| method.signature.to_string() == "()I")
    {
        return smoke_error("JavaClassWrapper String.length metadata was not found");
    }
    let string = java.new_string_utf("wrapper")?;
    let length = read_int(
        string_wrapper.call(&string, "length", "()I", &[])?,
        "JavaClassWrapper String.length",
    )?;
    if length != "wrapper".len() as i32 {
        return smoke_error(format!("JavaClassWrapper String.length mismatch: {length}"));
    }

    let math_wrapper = java.use_class("java.lang.Math")?;
    let abs_value = read_int(
        math_wrapper.call_static("abs", "(I)I", &[JavaValue::Int(-7)])?,
        "JavaClassWrapper Math.abs",
    )?;
    if abs_value != 7 {
        return smoke_error(format!("JavaClassWrapper Math.abs mismatch: {abs_value}"));
    }
    let integer_wrapper = java.use_class("java.lang.Integer")?;
    let max_value = read_int(
        integer_wrapper.get_static_field("MAX_VALUE", "I")?,
        "JavaClassWrapper Integer.MAX_VALUE",
    )?;
    if max_value != i32::MAX {
        return smoke_error(format!(
            "JavaClassWrapper Integer.MAX_VALUE mismatch: {max_value}"
        ));
    }
    Ok(())
}

fn check_app_loader_surface(java: &Java, app_java: &Java) -> Result<()> {
    println!("app_process_smoke: checking app-loader class and wrapper surface");
    if app_java.loader().is_none() {
        return smoke_error("app-loader Java unexpectedly lost its loader");
    }

    let subject = app_java.find_class(SMOKE_SUBJECT)?;
    let cached_subject = app_java.find_class(SMOKE_SUBJECT)?;
    if cached_subject.name() != SMOKE_SUBJECT {
        return smoke_error(format!(
            "cached SmokeSubject class name mismatch: {}",
            cached_subject.name()
        ));
    }
    let answer = read_int(
        subject.call_static("answer", "()I", &[])?,
        "SmokeSubject.answer",
    )?;
    if answer != 42 {
        return smoke_error(format!("SmokeSubject.answer mismatch: {answer}"));
    }
    let smoke_object = subject.new_object("()V", &[])?;
    let message = read_object(
        subject.call_method(&smoke_object, "message", "()Ljava/lang/String;", &[])?,
        "SmokeSubject.message",
    )?
    .ok_or_else(|| smoke_failure("SmokeSubject.message unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "dex-smoke" {
        return smoke_error(format!("SmokeSubject.message mismatch: {message:?}"));
    }

    let smoke_wrapper = app_java.use_class(SMOKE_SUBJECT)?;
    if !smoke_wrapper
        .constructors()?
        .iter()
        .any(|method| method.signature.to_string() == "()V")
    {
        return smoke_error("JavaClassWrapper SmokeSubject default constructor was not found");
    }
    let answer = read_int(
        smoke_wrapper.call_static("answer", "()I", &[])?,
        "JavaClassWrapper SmokeSubject.answer",
    )?;
    if answer != 42 {
        return smoke_error(format!(
            "JavaClassWrapper SmokeSubject.answer mismatch: {answer}"
        ));
    }
    let smoke_object = smoke_wrapper.new_object("()V", &[])?;
    let message = read_object(
        smoke_wrapper.call(&smoke_object, "message", "()Ljava/lang/String;", &[])?,
        "JavaClassWrapper SmokeSubject.message",
    )?
    .ok_or_else(|| {
        smoke_failure("JavaClassWrapper SmokeSubject.message unexpectedly returned null")
    })?;
    let message = message.get_string()?;
    if message != "dex-smoke" {
        return smoke_error(format!(
            "JavaClassWrapper SmokeSubject.message mismatch: {message:?}"
        ));
    }

    let wrapper_methods = smoke_wrapper.declared_methods()?;
    require_method(
        &wrapper_methods,
        "message",
        MethodKind::Instance,
        "()Ljava/lang/String;",
        "JavaClassWrapper declared SmokeSubject.message",
    )?;
    let wrapper_fields = smoke_wrapper.declared_fields()?;
    require_field(
        &wrapper_fields,
        "number",
        FieldKind::Instance,
        &JavaType::Int,
        "JavaClassWrapper declared SmokeSubject.number",
    )?;
    if !smoke_wrapper.is_instance(&smoke_object)? {
        return smoke_error("JavaClassWrapper SmokeSubject did not recognize its instance");
    }
    let object_wrapper = java.use_class("java.lang.Object")?;
    if !object_wrapper.is_instance(&smoke_object)? {
        return smoke_error("JavaClassWrapper Object did not recognize SmokeSubject instance");
    }
    let retained_object = object_wrapper.cast(&smoke_object)?;
    let _ = object_wrapper
        .call(&retained_object, "hashCode", "()I", &[])?
        .into_int("JavaClassWrapper retained Object.hashCode")?;

    println!("app_process_smoke: checking app-loader overload handles");
    let default_constructor = smoke_wrapper.constructor_overload(&[])?;
    if default_constructor.signature().to_string() != "()V" {
        return smoke_error(format!(
            "JavaConstructorOverload default signature mismatch: {}",
            default_constructor.signature()
        ));
    }
    let smoke_object = default_constructor.new_object(&[])?;
    let int_constructor = smoke_wrapper.constructor_overload_by_name(&["int"])?;
    let numbered_object = int_constructor.new_object(&[JavaValue::Int(31)])?;
    let number_field = smoke_wrapper.field_handle("number")?;
    let number = read_int(
        number_field.get(&numbered_object)?,
        "JavaFieldHandle SmokeSubject.number",
    )?;
    if number != 31 {
        return smoke_error(format!(
            "JavaFieldHandle SmokeSubject.number mismatch: {number}"
        ));
    }
    number_field.set(&numbered_object, JavaValue::Int(37))?;
    let number = read_int(
        number_field.get(&numbered_object)?,
        "JavaFieldHandle SmokeSubject.number after set",
    )?;
    if number != 37 {
        return smoke_error(format!(
            "JavaFieldHandle SmokeSubject.number after set mismatch: {number}"
        ));
    }
    let message_overload = smoke_wrapper.method_overload("message", &[])?;
    let message = read_object(
        message_overload.call(&smoke_object, &[])?,
        "JavaMethodOverload SmokeSubject.message",
    )?
    .ok_or_else(|| smoke_failure("JavaMethodOverload SmokeSubject.message unexpectedly null"))?;
    let message = message.get_string()?;
    if message != "dex-smoke" {
        return smoke_error(format!(
            "JavaMethodOverload SmokeSubject.message mismatch: {message:?}"
        ));
    }
    let overload_string =
        smoke_wrapper.method_overload_by_name("overload", &["java.lang.String"])?;
    let input = app_java.new_string_utf("typed")?;
    let value = read_object(
        overload_string.call(&smoke_object, &[JavaValue::from(&input)])?,
        "JavaMethodOverload SmokeSubject.overload(String)",
    )?
    .ok_or_else(|| smoke_failure("JavaMethodOverload SmokeSubject.overload(String) null"))?;
    let value = value.get_string()?;
    if value != "typed" {
        return smoke_error(format!(
            "JavaMethodOverload SmokeSubject.overload(String) mismatch: {value:?}"
        ));
    }
    Ok(())
}

fn check_dex_class_loader(java: &Java) -> Result<()> {
    println!("app_process_smoke: checking DexClassLoader explicit lookup");
    let class_loader_class = java.find_class("java.lang.ClassLoader")?;
    let system_loader_object = read_object(
        class_loader_class.call_static("getSystemClassLoader", "()Ljava/lang/ClassLoader;", &[])?,
        "ClassLoader.getSystemClassLoader",
    )?
    .ok_or_else(|| smoke_failure("ClassLoader.getSystemClassLoader unexpectedly returned null"))?;
    let system_loader = java.class_loader_from_object(&system_loader_object)?;

    let dex_class_loader_class = java.find_class("dalvik.system.DexClassLoader")?;
    let dex_path = java.new_string_utf(DEX_SMOKE_PATH)?;
    let dex_opt = java.new_string_utf(DEX_SMOKE_OPT)?;
    let dex_loader = dex_class_loader_class.new_object(
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/ClassLoader;)V",
        &[
            JavaValue::from(&dex_path),
            JavaValue::from(&dex_opt),
            JavaValue::Null,
            JavaValue::Object(system_loader.as_jobject()),
        ],
    )?;
    let dex_loader = java.class_loader_from_object(&dex_loader)?;
    let dex_java = java.with_loader(&dex_loader);
    let subject = dex_java.find_class(DEX_SMOKE_SUBJECT)?;
    let cached_subject = dex_java.find_class(DEX_SMOKE_SUBJECT)?;
    if cached_subject.name() != DEX_SMOKE_SUBJECT {
        return smoke_error(format!(
            "cached DexSmokeSubject class name mismatch: {}",
            cached_subject.name()
        ));
    }
    let answer = read_int(
        subject.call_static("answer", "()I", &[])?,
        "DexSmokeSubject.answer",
    )?;
    if answer != 4242 {
        return smoke_error(format!("DexSmokeSubject.answer mismatch: {answer}"));
    }
    let message = read_object(
        subject.call_static("message", "()Ljava/lang/String;", &[])?,
        "DexSmokeSubject.message",
    )?
    .ok_or_else(|| smoke_failure("DexSmokeSubject.message unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "dex-only-smoke" {
        return smoke_error(format!("DexSmokeSubject.message mismatch: {message:?}"));
    }

    match java.find_class(DEX_SMOKE_SUBJECT) {
        Err(Error::JavaException {
            operation: "JNIEnv::FindClass",
        }) => {}
        Err(error) => return Err(error),
        Ok(_class) => return smoke_error("DexSmokeSubject unexpectedly resolved without loader"),
    }
    Ok(())
}

fn check_metadata_and_enumeration(
    java: &Java,
    app_java: &Java,
    loaded_class_enumeration_supported: bool,
    class_loader_enumeration_supported: bool,
) -> Result<()> {
    println!("app_process_smoke: checking metadata reflection");
    let subject = app_java.find_class(SMOKE_SUBJECT)?;
    let smoke_metadata = subject.metadata()?;
    if smoke_metadata.name != SMOKE_SUBJECT {
        return smoke_error(format!(
            "SmokeSubject metadata name mismatch: {}",
            smoke_metadata.name
        ));
    }
    if smoke_metadata.descriptor != format!("L{};", SMOKE_SUBJECT.replace('.', "/")) {
        return smoke_error(format!(
            "SmokeSubject metadata descriptor mismatch: {}",
            smoke_metadata.descriptor
        ));
    }
    if smoke_metadata.loader.is_none() {
        return smoke_error("SmokeSubject metadata unexpectedly had no class loader");
    }

    let methods = subject.declared_methods()?;
    require_method(
        &methods,
        "<init>",
        MethodKind::Constructor,
        "()V",
        "SmokeSubject default constructor",
    )?;
    require_method(
        &methods,
        "<init>",
        MethodKind::Constructor,
        "(I)V",
        "SmokeSubject int constructor",
    )?;
    require_method(
        &methods,
        "overload",
        MethodKind::Instance,
        "()Ljava/lang/String;",
        "SmokeSubject overload()",
    )?;
    require_method(
        &methods,
        "overload",
        MethodKind::Instance,
        "(Ljava/lang/String;)Ljava/lang/String;",
        "SmokeSubject overload(String)",
    )?;
    let answer_method = require_method(
        &methods,
        "answer",
        MethodKind::Static,
        "()I",
        "SmokeSubject answer",
    )?;
    if answer_method.modifiers & 0x0008 == 0 {
        return smoke_error("SmokeSubject.answer metadata did not report static modifier");
    }
    let hidden_static = require_method(
        &methods,
        "hiddenStatic",
        MethodKind::Static,
        "()Ljava/lang/String;",
        "SmokeSubject hiddenStatic",
    )?;
    if hidden_static.modifiers & 0x0002 == 0 {
        return smoke_error("SmokeSubject.hiddenStatic metadata did not report private modifier");
    }

    let fields = subject.declared_fields()?;
    require_field(
        &fields,
        "STATIC_TEXT",
        FieldKind::Static,
        &JavaType::Object("java/lang/String".to_owned()),
        "SmokeSubject STATIC_TEXT",
    )?;
    require_field(
        &fields,
        "number",
        FieldKind::Instance,
        &JavaType::Int,
        "SmokeSubject number",
    )?;
    let hidden_field = require_field(
        &fields,
        "hidden",
        FieldKind::Instance,
        &JavaType::Long,
        "SmokeSubject hidden",
    )?;
    if hidden_field.modifiers & 0x0002 == 0 {
        return smoke_error("SmokeSubject.hidden metadata did not report private modifier");
    }

    println!("app_process_smoke: checking loaded-class and method query metadata");
    match java.enumerate_loaded_classes() {
        Ok(classes) => {
            if !loaded_class_enumeration_supported {
                return smoke_error(
                    "loaded-class enumeration succeeded despite unsupported capability",
                );
            }
            if !classes
                .iter()
                .any(|class| class.name() == "java.lang.String")
            {
                return smoke_error("loaded-class enumeration did not include java.lang.String");
            }
            if !classes.iter().any(|class| class.name() == SMOKE_SUBJECT) {
                return smoke_error("loaded-class enumeration did not include SmokeSubject");
            }
            drop(classes);

            let groups =
                java.enumerate_methods("frida.java.bridge.rs.smoke.SmokeSubject!overload*/s")?;
            let mut overload_signatures = Vec::new();
            for group in &groups {
                for class in &group.classes {
                    if class.name == SMOKE_SUBJECT {
                        overload_signatures.extend(
                            class
                                .methods
                                .iter()
                                .map(|method| method.signature.to_string()),
                        );
                    }
                }
            }
            if !overload_signatures
                .iter()
                .any(|sig| sig == "()Ljava/lang/String;")
                || !overload_signatures
                    .iter()
                    .any(|sig| sig == "(Ljava/lang/String;)Ljava/lang/String;")
            {
                return smoke_error(format!(
                    "method query did not include both overload signatures: {overload_signatures:?}"
                ));
            }
        }
        Err(Error::UnsupportedFeature {
            feature: "ART loaded-class enumeration",
            reason,
        }) => {
            if loaded_class_enumeration_supported {
                return smoke_error(format!(
                    "loaded-class enumeration was unsupported despite supported capability: {reason}"
                ));
            }
        }
        Err(error) => return Err(error),
    }

    println!("app_process_smoke: checking class-loader enumeration capability");
    match java.enumerate_class_loaders() {
        Ok(loaders) => {
            if !class_loader_enumeration_supported {
                return smoke_error(
                    "class-loader enumeration succeeded despite unsupported capability",
                );
            }
            if loaders.is_empty() {
                return smoke_error("class-loader enumeration returned no loaders");
            }
            let mut resolved_string = false;
            let mut resolved_subject = false;
            for loader in loaders {
                if loader.kind() != ClassLoaderKind::Enumerated {
                    return smoke_error(format!(
                        "enumerated class loader had unexpected kind {:?}",
                        loader.kind()
                    ));
                }
                let loader_java = java.with_loader(&loader);
                if loader_java.find_class("java.lang.String").is_ok() {
                    resolved_string = true;
                }
                if let Ok(subject) = loader_java.find_class(SMOKE_SUBJECT) {
                    let answer = read_int(
                        subject.call_static("answer", "()I", &[])?,
                        "enumerated SmokeSubject.answer",
                    )?;
                    if answer != 42 {
                        return smoke_error(format!(
                            "enumerated SmokeSubject.answer mismatch: {answer}"
                        ));
                    }
                    resolved_subject = true;
                }
            }
            if !resolved_string {
                return smoke_error("no enumerated class loader resolved java.lang.String");
            }
            if !resolved_subject {
                return smoke_error("no enumerated class loader resolved SmokeSubject");
            }
        }
        Err(Error::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            reason,
        }) => {
            if class_loader_enumeration_supported {
                return smoke_error(format!(
                    "class-loader enumeration was unsupported despite supported capability: {reason}"
                ));
            }
        }
        Err(error) => return Err(error),
    }

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
        expect_clone_backend_summary(&summary)?;
    } else {
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: "replacement debug summary was unavailable".to_owned(),
        });
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
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        1337,
        "answer replacement after System.gc",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        42,
        "answer restored",
    )?;

    println!("app_process_smoke: checking static original call from replacement");
    let replacement = unsafe {
        experimental::replace_static_i32_method(
            &subject,
            "answer",
            replacement_answer_calling_original,
        )?
    };
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        1042,
        "answer replacement calling original",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        42,
        "answer restored after original call replacement",
    )?;

    println!("app_process_smoke: checking app-loader primitive and argument replacements");
    subject.call_static("resetVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_static("voidCounter", "()I", &[])?,
        0,
        "voidCounter reset",
    )?;
    subject.call_static("bumpVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_static("voidCounter", "()I", &[])?,
        1,
        "bumpVoidCounter original",
    )?;
    VOID_REPLACEMENT_COUNTER.store(0, Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_void_method(&subject, "bumpVoidCounter", replacement_void)?
    };
    subject.call_static("bumpVoidCounter", "()V", &[])?;
    subject.call_static("bumpVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_static("voidCounter", "()I", &[])?,
        1,
        "bumpVoidCounter Java state during replacement",
    )?;
    if VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst) != 2 {
        return replacement_counter_mismatch(
            "bumpVoidCounter replacement counter",
            2,
            VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst),
        );
    }
    replacement.revert()?;
    subject.call_static("bumpVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_static("voidCounter", "()I", &[])?,
        2,
        "bumpVoidCounter restored",
    )?;

    expect_bool(
        subject.call_static("staticBoolean", "()Z", &[])?,
        true,
        "staticBoolean original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_boolean_method(&subject, "staticBoolean", replacement_boolean)?
    };
    expect_bool(
        subject.call_static("staticBoolean", "()Z", &[])?,
        false,
        "staticBoolean replacement",
    )?;
    replacement.revert()?;
    expect_bool(
        subject.call_static("staticBoolean", "()Z", &[])?,
        true,
        "staticBoolean restored",
    )?;

    expect_byte(
        subject.call_static("staticByte", "()B", &[])?,
        7,
        "staticByte original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_byte_method(&subject, "staticByte", replacement_byte)?
    };
    expect_byte(
        subject.call_static("staticByte", "()B", &[])?,
        -8,
        "staticByte replacement",
    )?;
    replacement.revert()?;
    expect_byte(
        subject.call_static("staticByte", "()B", &[])?,
        7,
        "staticByte restored",
    )?;

    expect_char(
        subject.call_static("staticChar", "()C", &[])?,
        b'A' as jni::jchar,
        "staticChar original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_char_method(&subject, "staticChar", replacement_char)?
    };
    expect_char(
        subject.call_static("staticChar", "()C", &[])?,
        b'Z' as jni::jchar,
        "staticChar replacement",
    )?;
    replacement.revert()?;
    expect_char(
        subject.call_static("staticChar", "()C", &[])?,
        b'A' as jni::jchar,
        "staticChar restored",
    )?;

    expect_short(
        subject.call_static("staticShort", "()S", &[])?,
        1234,
        "staticShort original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_short_method(&subject, "staticShort", replacement_short)?
    };
    expect_short(
        subject.call_static("staticShort", "()S", &[])?,
        -1234,
        "staticShort replacement",
    )?;
    replacement.revert()?;
    expect_short(
        subject.call_static("staticShort", "()S", &[])?,
        1234,
        "staticShort restored",
    )?;

    expect_long(
        subject.call_static("staticLong", "()J", &[])?,
        1234567890123,
        "staticLong original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_i64_method(&subject, "staticLong", replacement_long)?
    };
    expect_long(
        subject.call_static("staticLong", "()J", &[])?,
        -9876543210,
        "staticLong replacement",
    )?;
    replacement.revert()?;
    expect_long(
        subject.call_static("staticLong", "()J", &[])?,
        1234567890123,
        "staticLong restored",
    )?;

    expect_float(
        subject.call_static("staticFloat", "()F", &[])?,
        1.25,
        "staticFloat original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_f32_method(&subject, "staticFloat", replacement_float)?
    };
    expect_float(
        subject.call_static("staticFloat", "()F", &[])?,
        -2.5,
        "staticFloat replacement",
    )?;
    replacement.revert()?;
    expect_float(
        subject.call_static("staticFloat", "()F", &[])?,
        1.25,
        "staticFloat restored",
    )?;

    expect_double(
        subject.call_static("staticDouble", "()D", &[])?,
        3.5,
        "staticDouble original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_f64_method(&subject, "staticDouble", replacement_double)?
    };
    expect_double(
        subject.call_static("staticDouble", "()D", &[])?,
        -6.25,
        "staticDouble replacement",
    )?;
    replacement.revert()?;
    expect_double(
        subject.call_static("staticDouble", "()D", &[])?,
        3.5,
        "staticDouble restored",
    )?;

    let string_output = java.new_string_utf("app-process-static-string")?;
    REPLACEMENT_STRING.store(string_output.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_string_method(&subject, "staticString", replacement_string)?
    };
    expect_string(
        subject.call_static("staticString", "()Ljava/lang/String;", &[])?,
        Some("app-process-static-string"),
        "staticString replacement",
    )?;
    replacement.revert()?;
    expect_string(
        subject.call_static("staticString", "()Ljava/lang/String;", &[])?,
        Some("original-string"),
        "staticString restored",
    )?;

    let input = java.new_string_utf("app-process-static-argument")?;
    let output = java.new_string_utf("app-process-static-echo")?;
    EXPECTED_ARGUMENT.store(input.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_STRING.store(output.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_string_to_string_method(
            &subject,
            "staticEcho",
            replacement_static_echo,
        )?
    };
    expect_string(
        subject.call_static(
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-static-echo"),
        "staticEcho replacement",
    )?;
    expect_string(
        wrapper.call_static(
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-static-echo"),
        "wrapper staticEcho replacement",
    )?;
    replacement.revert()?;
    expect_string(
        subject.call_static(
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-static-argument"),
        "staticEcho restored",
    )?;

    expect_int(
        subject.call_static(
            "staticAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        7,
        "staticAdd original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_i32_i32_to_i32_method(
            &subject,
            "staticAdd",
            replacement_static_add,
        )?
    };
    expect_int(
        subject.call_static(
            "staticAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        52,
        "staticAdd replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static(
            "staticAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        7,
        "staticAdd restored",
    )?;

    expect_int(
        subject.call_static(
            "staticPrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        74,
        "staticPrimitiveMix original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_z_b_c_s_to_i32_method(
            &subject,
            "staticPrimitiveMix",
            replacement_static_primitive_mix,
        )?
    };
    expect_int(
        subject.call_static(
            "staticPrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        4242,
        "staticPrimitiveMix replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static(
            "staticPrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        74,
        "staticPrimitiveMix restored",
    )?;

    expect_long(
        subject.call_static(
            "staticWide",
            "(JD)J",
            &[JavaValue::Long(40), JavaValue::Double(2.0)],
        )?,
        42,
        "staticWide original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_i64_f64_to_i64_method(
            &subject,
            "staticWide",
            replacement_static_wide,
        )?
    };
    expect_long(
        subject.call_static(
            "staticWide",
            "(JD)J",
            &[JavaValue::Long(40), JavaValue::Double(2.0)],
        )?,
        9001,
        "staticWide replacement",
    )?;
    replacement.revert()?;
    expect_long(
        subject.call_static(
            "staticWide",
            "(JD)J",
            &[JavaValue::Long(40), JavaValue::Double(2.0)],
        )?,
        42,
        "staticWide restored",
    )?;

    expect_double(
        subject.call_static(
            "staticFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?,
        3.75,
        "staticFloatMix original",
    )?;
    let replacement = unsafe {
        experimental::replace_static_f32_f64_to_f64_method(
            &subject,
            "staticFloatMix",
            replacement_static_float_mix,
        )?
    };
    expect_double(
        subject.call_static(
            "staticFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?,
        8.5,
        "staticFloatMix replacement",
    )?;
    replacement.revert()?;
    expect_double(
        subject.call_static(
            "staticFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?,
        3.75,
        "staticFloatMix restored",
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

    let output = java.new_string_utf("app-process-instance-string")?;
    REPLACEMENT_STRING.store(output.as_jobject(), Ordering::SeqCst);
    expect_string(
        subject.call_method(&object, "message", "()Ljava/lang/String;", &[])?,
        Some("dex-smoke"),
        "message original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_string_method(
            &subject,
            "message",
            replacement_instance_string,
        )?
    };
    expect_string(
        subject.call_method(&object, "message", "()Ljava/lang/String;", &[])?,
        Some("app-process-instance-string"),
        "message replacement",
    )?;
    replacement.revert()?;
    expect_string(
        subject.call_method(&object, "message", "()Ljava/lang/String;", &[])?,
        Some("dex-smoke"),
        "message restored",
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
        expect_clone_backend_summary(&summary)?;
    } else {
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: "replacement debug summary was unavailable".to_owned(),
        });
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

    println!("app_process_smoke: checking instance original call from replacement");
    let replacement = unsafe {
        experimental::replace_instance_i32_method(
            &subject,
            "instanceNumber",
            replacement_instance_number_calling_original,
        )?
    };
    expect_int(
        subject.call_method(&object, "instanceNumber", "()I", &[])?,
        131,
        "instanceNumber replacement calling original",
    )?;
    expect_int(
        subject.call_method(&second_object, "instanceNumber", "()I", &[])?,
        132,
        "second receiver instanceNumber replacement calling original",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(&object, "instanceNumber", "()I", &[])?,
        31,
        "instanceNumber restored after original call replacement",
    )?;
    expect_int(
        subject.call_method(&second_object, "instanceNumber", "()I", &[])?,
        32,
        "second receiver instanceNumber restored after original call replacement",
    )?;

    println!("app_process_smoke: checking private static replacement");
    let hidden_output = java.new_string_utf("app-process-replacement")?;
    REPLACEMENT_STRING.store(hidden_output.as_jobject(), Ordering::SeqCst);
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

fn expect_bool(value: JavaReturn, expected: bool, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Boolean(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("boolean {expected}"), other),
    }
}

fn expect_byte(value: JavaReturn, expected: jni::jbyte, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Byte(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("byte {expected}"), other),
    }
}

fn expect_char(value: JavaReturn, expected: jni::jchar, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Char(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("char {expected}"), other),
    }
}

fn expect_short(value: JavaReturn, expected: jni::jshort, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Short(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("short {expected}"), other),
    }
}

fn expect_long(value: JavaReturn, expected: jni::jlong, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Long(value) if value == expected => Ok(()),
        other => replacement_mismatch(operation, format!("long {expected}"), other),
    }
}

fn expect_float(value: JavaReturn, expected: jni::jfloat, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Float(value) if (value - expected).abs() < 0.0001 => Ok(()),
        other => replacement_mismatch(operation, format!("float {expected}"), other),
    }
}

fn expect_double(value: JavaReturn, expected: jni::jdouble, operation: &'static str) -> Result<()> {
    match value {
        JavaReturn::Double(value) if (value - expected).abs() < 0.0001 => Ok(()),
        other => replacement_mismatch(operation, format!("double {expected}"), other),
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

fn read_int(value: JavaReturn, operation: &'static str) -> Result<i32> {
    match value {
        JavaReturn::Int(value) => Ok(value),
        other => smoke_error(format!("{operation} returned unexpected value {other:?}")),
    }
}

fn read_object(value: JavaReturn, operation: &'static str) -> Result<Option<JavaObject>> {
    match value {
        JavaReturn::Object(value) => Ok(value),
        other => smoke_error(format!("{operation} returned unexpected value {other:?}")),
    }
}

fn require_method<'a>(
    methods: &'a [JavaMethodMetadata],
    name: &str,
    kind: MethodKind,
    signature: &str,
    operation: &'static str,
) -> Result<&'a JavaMethodMetadata> {
    methods
        .iter()
        .find(|method| {
            method.name == name && method.kind == kind && method.signature.to_string() == signature
        })
        .ok_or_else(|| smoke_failure(format!("{operation} metadata was not found")))
}

fn require_field<'a>(
    fields: &'a [JavaFieldMetadata],
    name: &str,
    kind: FieldKind,
    ty: &JavaType,
    operation: &'static str,
) -> Result<&'a JavaFieldMetadata> {
    fields
        .iter()
        .find(|field| field.name == name && field.kind == kind && &field.ty == ty)
        .ok_or_else(|| smoke_failure(format!("{operation} metadata was not found")))
}

fn smoke_error<T>(reason: impl Into<String>) -> Result<T> {
    Err(smoke_failure(reason))
}

fn smoke_failure(reason: impl Into<String>) -> Error {
    Error::UnsupportedFeature {
        feature: "app_process smoke",
        reason: reason.into(),
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

fn replacement_counter_mismatch<T>(
    operation: &'static str,
    expected: i32,
    actual: i32,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature: "ART method replacement",
        reason: format!("{operation} mismatch: expected counter {expected}, got {actual}"),
    })
}

fn expect_clone_backend_summary(summary: &str) -> Result<()> {
    if summary.contains("backend=clone-active")
        && summary.contains("original_patched=")
        && summary.contains("clone_patched=")
    {
        return Ok(());
    }
    Err(Error::UnsupportedFeature {
        feature: "ART method replacement",
        reason: format!("replacement did not use cloned-method backend: {summary}"),
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

unsafe extern "C" fn replacement_answer_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
) -> jni::jint {
    match unsafe { experimental::call_original_static_i32_method(env, class, "answer") } {
        Ok(value) => value + 1000,
        Err(error) => {
            println!("app_process_smoke: static original call failed: {error}");
            -1000
        }
    }
}

unsafe extern "C" fn replacement_void(_env: *mut jni::JNIEnv, _class: jni::jclass) {
    VOID_REPLACEMENT_COUNTER.fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn replacement_string(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jstring {
    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

unsafe extern "C" fn replacement_boolean(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jboolean {
    jni::JNI_FALSE
}

unsafe extern "C" fn replacement_byte(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jbyte {
    -8
}

unsafe extern "C" fn replacement_char(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jchar {
    b'Z' as jni::jchar
}

unsafe extern "C" fn replacement_short(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jshort {
    -1234
}

unsafe extern "C" fn replacement_long(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jlong {
    -9876543210
}

unsafe extern "C" fn replacement_float(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jfloat {
    -2.5
}

unsafe extern "C" fn replacement_double(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jdouble {
    -6.25
}

unsafe extern "C" fn replacement_static_echo(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    argument: jni::jstring,
) -> jni::jstring {
    let expected_argument = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
    if env.is_null()
        || expected_argument.is_null()
        || argument.is_null()
        || !unsafe { raw_is_same_object(env, argument, expected_argument) }
    {
        return ptr::null_mut();
    }

    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

unsafe extern "C" fn replacement_static_add(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    left + right + 45
}

unsafe extern "C" fn replacement_static_primitive_mix(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    flag: jni::jboolean,
    value: jni::jbyte,
    letter: jni::jchar,
    extra: jni::jshort,
) -> jni::jint {
    if flag == jni::JNI_TRUE && value == 2 && letter == b'C' as jni::jchar && extra == 5 {
        4242
    } else {
        -4242
    }
}

unsafe extern "C" fn replacement_static_wide(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    value: jni::jlong,
    extra: jni::jdouble,
) -> jni::jlong {
    if value == 40 && (extra - 2.0).abs() < 0.0001 {
        9001
    } else {
        -9001
    }
}

unsafe extern "C" fn replacement_static_float_mix(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    value: jni::jfloat,
    extra: jni::jdouble,
) -> jni::jdouble {
    if (value - 1.5).abs() < 0.0001 && (extra - 2.25).abs() < 0.0001 {
        8.5
    } else {
        -8.5
    }
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

unsafe extern "C" fn replacement_instance_number_calling_original(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jint {
    match unsafe {
        experimental::call_original_instance_i32_method(env, receiver, "instanceNumber")
    } {
        Ok(value) => value + 100,
        Err(error) => {
            println!("app_process_smoke: instance original call failed: {error}");
            -100
        }
    }
}

unsafe extern "C" fn replacement_instance_string(
    _env: *mut jni::JNIEnv,
    _receiver: jni::jobject,
) -> jni::jstring {
    REPLACEMENT_STRING.load(Ordering::SeqCst)
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
