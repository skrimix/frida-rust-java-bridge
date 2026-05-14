use std::{
    error::Error,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs, mem, ptr,
    sync::atomic::{AtomicPtr, Ordering},
};

use frida_gum_sys::siginfo_t;
use frida_java_bridge_rs::{
    Error as BridgeError, FieldKind, JavaClass, JavaClassWrapper, JavaReturn, JavaType, JavaValue,
    MethodKind, Runtime, RuntimeFlavor, jni,
};

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;
const LIBART: &str = "libart.so";
const JNI_CREATE_JAVA_VM: &str = "JNI_CreateJavaVM";
const PROP_VALUE_MAX: usize = 92;
const SMOKE_DIR: &str = "/data/local/tmp/frida-java-bridge-rs";
const SMOKE_DEX: &str = "/data/local/tmp/frida-java-bridge-rs/smoke-fixture.dex";
const SMOKE_DEX_OPT: &str = "/data/local/tmp/frida-java-bridge-rs/dex-cache";
const SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.SmokeSubject";
const SMOKE_DEX_BYTES: &[u8] = include_bytes!("../../smoke-fixtures/dex/classes.dex");
static REPLACEMENT_STATIC_STRING: AtomicPtr<jni::_jobject> = AtomicPtr::new(ptr::null_mut());

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
}

macro_rules! check_static_no_arg_replacement {
    ($class:expr, $replace:path, $method:literal, $replacement:path, $read:ident, $original:expr, $patched:expr) => {{
        let value = $read($class, $method, concat!($method, " original"))?;
        if value != $original {
            return Err(format!("{} original mismatch: {:?}", $method, value).into());
        }

        let replacement = unsafe { $replace($class, $method, $replacement)? };
        let value = $read($class, $method, concat!($method, " replacement"))?;
        if value != $patched {
            return Err(format!("{} replacement mismatch: {:?}", $method, value).into());
        }
        replacement.revert()?;
        let value = $read($class, $method, concat!($method, " restored"))?;
        if value != $original {
            return Err(format!("{} restored mismatch: {:?}", $method, value).into());
        }

        {
            let _drop_replacement = unsafe { $replace($class, $method, $replacement)? };
            let value = $read($class, $method, concat!($method, " drop-replacement"))?;
            if value != $patched {
                return Err(format!("{} drop-replacement mismatch: {:?}", $method, value).into());
            }
        }
        let value = $read($class, $method, concat!($method, " drop-restored"))?;
        if value != $original {
            return Err(format!("{} drop-restored mismatch: {:?}", $method, value).into());
        }

        let replacement = unsafe { $replace($class, $method, $replacement)? };
        let value = $read($class, $method, concat!($method, " second replacement"))?;
        if value != $patched {
            return Err(format!("{} second replacement mismatch: {:?}", $method, value).into());
        }
        replacement.revert()?;
        let value = $read($class, $method, concat!($method, " second restored"))?;
        if value != $original {
            return Err(format!("{} second restored mismatch: {:?}", $method, value).into());
        }
    }};
}

fn main() {
    if let Err(error) = run() {
        eprintln!("art_smoke: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    println!("art_smoke: pid {}", std::process::id());
    println!("art_smoke: device {}", device_label());

    println!("art_smoke: loading ART");
    let art = dlopen_global(LIBART)?;
    let create_java_vm = resolve_create_java_vm(art)?;

    println!("art_smoke: creating Java VM");
    create_vm(create_java_vm)?;

    println!("art_smoke: obtaining runtime");
    let runtime = Runtime::obtain()?;
    let vm = runtime.vm();
    let env = vm.get_env()?;
    println!("art_smoke: JNI version 0x{:08x}", env.version());

    println!("art_smoke: attaching current thread");
    let env = vm.attach_current_thread()?;

    println!("art_smoke: finding boot class");
    let string_class = env.find_class("java/lang/String")?;
    let object_class = env.find_class("java/lang/Object")?;
    let math_class = env.find_class("java/lang/Math")?;
    let integer_class = env.find_class("java/lang/Integer")?;
    let atomic_integer_class = env.find_class("java/util/concurrent/atomic/AtomicInteger")?;
    let throwable_class = env.find_class("java/lang/Throwable")?;
    let runtime_exception_class = env.find_class("java/lang/RuntimeException")?;

    println!("art_smoke: round-tripping string");
    let string = env.new_string_utf("frida-java-bridge-rs")?;
    let copied = env.get_string(&string)?;
    if copied != "frida-java-bridge-rs" {
        return Err(format!("string round-trip mismatch: {copied:?}").into());
    }

    println!("art_smoke: constructing object and calling instance methods");
    let object_ctor = env.get_constructor(&object_class, "()V")?;
    let object = env.new_object(&object_class, &object_ctor, &[])?;
    let hash_code = env.get_method(&object_class, "hashCode", "()I")?;
    let _ = env.call_int_method(&object, &hash_code, &[])?;

    let string_length = env.get_method(&string_class, "length", "()I")?;
    let length = env.call_int_method(&string, &string_length, &[])?;
    if length != "frida-java-bridge-rs".len() as i32 {
        return Err(format!("string length mismatch: {length}").into());
    }

    println!("art_smoke: calling static method");
    let abs = env.get_static_method(&math_class, "abs", "(I)I")?;
    let abs_value = env.call_static_int_method(&math_class, &abs, &[JavaValue::Int(-42)])?;
    if abs_value != 42 {
        return Err(format!("Math.abs result mismatch: {abs_value}").into());
    }

    println!("art_smoke: accessing fields");
    let max_value = env.get_static_field(&integer_class, "MAX_VALUE", "I")?;
    let max_value = env.get_static_int_field(&integer_class, &max_value)?;
    if max_value != i32::MAX {
        return Err(format!("Integer.MAX_VALUE mismatch: {max_value}").into());
    }

    let atomic_ctor = env.get_constructor(&atomic_integer_class, "(I)V")?;
    let atomic = env.new_object(&atomic_integer_class, &atomic_ctor, &[JavaValue::Int(7)])?;
    let atomic_value = env.get_field(&atomic_integer_class, "value", "I")?;
    let value = env.get_int_field(&atomic, &atomic_value)?;
    if value != 7 {
        return Err(format!("AtomicInteger.value mismatch: {value}").into());
    }
    env.set_int_field(&atomic, &atomic_value, 19)?;
    let atomic_get = env.get_method(&atomic_integer_class, "get", "()I")?;
    let value = env.call_int_method(&atomic, &atomic_get, &[])?;
    if value != 19 {
        return Err(format!("AtomicInteger.get mismatch after field set: {value}").into());
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
        .ok_or("Throwable.detailMessage unexpectedly null")?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "initial" {
        return Err(format!("Throwable.detailMessage mismatch: {message:?}").into());
    }
    let updated_message = env.new_string_utf("updated")?;
    env.set_object_field(&exception, &detail_message, Some(&updated_message))?;
    let get_message = env.get_method(&throwable_class, "getMessage", "()Ljava/lang/String;")?;
    let message = env
        .call_object_method(&exception, &get_message, &[])?
        .ok_or("Throwable.getMessage unexpectedly returned null")?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "updated" {
        return Err(format!("Throwable.getMessage mismatch after field set: {message:?}").into());
    }

    println!("art_smoke: checking exception handling");
    match env.find_class("frida/java/bridge/rs/MissingSmokeClass") {
        Err(BridgeError::JavaException {
            operation: "JNIEnv::FindClass",
        }) => {}
        Err(error) => return Err(format!("unexpected missing-class error: {error}").into()),
        Ok(_class) => return Err("missing class unexpectedly resolved".into()),
    }

    if env.exception_check() {
        env.exception_clear();
        return Err("pending exception was not cleared after failed FindClass".into());
    }

    println!("art_smoke: checking convenience layer");
    let java = runtime.java();
    let capabilities = java.capabilities();
    if capabilities.flavor != RuntimeFlavor::Art {
        return Err(format!("unexpected runtime flavor {:?}", capabilities.flavor).into());
    }
    if runtime.capabilities() != capabilities || vm.capabilities() != capabilities {
        return Err("runtime, VM, and Java capability reports diverged".into());
    }
    println!("art_smoke: capabilities {capabilities:?}");
    if capabilities.heap_enumeration.is_supported()
        || capabilities
            .heap_enumeration
            .unsupported_reason()
            .is_none_or(|reason| !reason.contains("not implemented yet"))
    {
        return Err(format!(
            "heap enumeration capability was not explicitly deferred: {:?}",
            capabilities.heap_enumeration
        )
        .into());
    }
    if capabilities.deoptimization.is_supported()
        || capabilities
            .deoptimization
            .unsupported_reason()
            .is_none_or(|reason| !reason.contains("not implemented yet"))
    {
        return Err(format!(
            "deoptimization capability was not explicitly deferred: {:?}",
            capabilities.deoptimization
        )
        .into());
    }
    let method_replacement_reason = capabilities.method_replacement.unsupported_reason();
    println!("art_smoke: method replacement capability reason {method_replacement_reason:?}");
    if capabilities.method_replacement.is_supported() || method_replacement_reason.is_none() {
        return Err(format!(
            "method replacement capability was not explicitly unsupported: {:?}",
            capabilities.method_replacement
        )
        .into());
    }

    let string_class = java.find_class("java.lang.String")?;
    let math_class = java.find_class("java.lang.Math")?;
    let class_loader_class = java.find_class("java.lang.ClassLoader")?;
    let atomic_integer_class = java.find_class("java.util.concurrent.atomic.AtomicInteger")?;
    let throwable_class = java.find_class("java.lang.Throwable")?;
    let runtime_exception_class = java.find_class("java.lang.RuntimeException")?;

    let string = java.new_string_utf("frida-java-bridge-rs")?;
    let length = expect_int(
        string_class.call_method(&string, "length", "()I", &[])?,
        "String.length",
    )?;
    if length != "frida-java-bridge-rs".len() as i32 {
        return Err(format!("JavaClass String.length mismatch: {length}").into());
    }
    let cached_length = expect_int(
        string_class.call_method(&string, "length", "()I", &[])?,
        "String.length cached",
    )?;
    if cached_length != length {
        return Err(format!("JavaClass cached String.length mismatch: {cached_length}").into());
    }

    let abs_value = expect_int(
        math_class.call_static("abs", "(I)I", &[JavaValue::Int(-42)])?,
        "Math.abs",
    )?;
    if abs_value != 42 {
        return Err(format!("JavaClass Math.abs result mismatch: {abs_value}").into());
    }

    let atomic = atomic_integer_class.new_object("(I)V", &[JavaValue::Int(7)])?;
    let value = expect_int(
        atomic_integer_class.get_field(&atomic, "value", "I")?,
        "AtomicInteger.value",
    )?;
    if value != 7 {
        return Err(format!("JavaClass AtomicInteger.value mismatch: {value}").into());
    }
    atomic_integer_class.set_field(&atomic, "value", "I", JavaValue::Int(19))?;
    let value = expect_int(
        atomic_integer_class.call_method(&atomic, "get", "()I", &[])?,
        "AtomicInteger.get",
    )?;
    if value != 19 {
        return Err(
            format!("JavaClass AtomicInteger.get mismatch after field set: {value}").into(),
        );
    }

    let initial_message = java.new_string_utf("initial")?;
    let exception = runtime_exception_class.new_object(
        "(Ljava/lang/String;)V",
        &[JavaValue::from(&initial_message)],
    )?;
    let message = expect_object(
        throwable_class.get_field(&exception, "detailMessage", "Ljava/lang/String;")?,
        "Throwable.detailMessage",
    )?
    .ok_or("JavaClass Throwable.detailMessage unexpectedly null")?;
    let message = message.get_string()?;
    if message != "initial" {
        return Err(format!("JavaClass Throwable.detailMessage mismatch: {message:?}").into());
    }
    let updated_message = java.new_string_utf("updated")?;
    throwable_class.set_field(
        &exception,
        "detailMessage",
        "Ljava/lang/String;",
        JavaValue::from(&updated_message),
    )?;
    let message = expect_object(
        throwable_class.call_method(&exception, "getMessage", "()Ljava/lang/String;", &[])?,
        "Throwable.getMessage",
    )?
    .ok_or("JavaClass Throwable.getMessage unexpectedly returned null")?;
    let message = message.get_string()?;
    if message != "updated" {
        return Err(format!(
            "JavaClass Throwable.getMessage mismatch after field set: {message:?}"
        )
        .into());
    }

    println!("art_smoke: checking Java.use-style wrapper");
    let string_wrapper = java.use_class("java.lang.String")?;
    let cached_string_wrapper = java.use_class("java.lang.String")?;
    if string_wrapper.name() != "java.lang.String"
        || cached_string_wrapper.class().name() != "java.lang.String"
    {
        return Err("JavaClassWrapper String name mismatch".into());
    }
    if !string_wrapper
        .methods("length")?
        .iter()
        .any(|method| method.signature.to_string() == "()I")
    {
        return Err("JavaClassWrapper String.length metadata was not found".into());
    }
    let string = java.new_string_utf("wrapper")?;
    let length = expect_int(
        string_wrapper.call(&string, "length", "()I", &[])?,
        "JavaClassWrapper String.length",
    )?;
    if length != "wrapper".len() as i32 {
        return Err(format!("JavaClassWrapper String.length mismatch: {length}").into());
    }

    let math_wrapper = java.use_class("java.lang.Math")?;
    let abs_value = expect_int(
        math_wrapper.call_static("abs", "(I)I", &[JavaValue::Int(-7)])?,
        "JavaClassWrapper Math.abs",
    )?;
    if abs_value != 7 {
        return Err(format!("JavaClassWrapper Math.abs mismatch: {abs_value}").into());
    }

    let integer_wrapper = java.use_class("java.lang.Integer")?;
    let max_value = expect_int(
        integer_wrapper.get_static_field("MAX_VALUE", "I")?,
        "JavaClassWrapper Integer.MAX_VALUE",
    )?;
    if max_value != i32::MAX {
        return Err(format!("JavaClassWrapper Integer.MAX_VALUE mismatch: {max_value}").into());
    }

    let atomic_wrapper = java.use_class("java.util.concurrent.atomic.AtomicInteger")?;
    let atomic = atomic_wrapper.new_object("(I)V", &[JavaValue::Int(11)])?;
    let value = expect_int(
        atomic_wrapper.get_field(&atomic, "value", "I")?,
        "JavaClassWrapper AtomicInteger.value",
    )?;
    if value != 11 {
        return Err(format!("JavaClassWrapper AtomicInteger.value mismatch: {value}").into());
    }
    atomic_wrapper.set_field(&atomic, "value", "I", JavaValue::Int(23))?;
    let value = expect_int(
        atomic_wrapper.call(&atomic, "get", "()I", &[])?,
        "JavaClassWrapper AtomicInteger.get",
    )?;
    if value != 23 {
        return Err(format!("JavaClassWrapper AtomicInteger.get mismatch: {value}").into());
    }

    match string_wrapper.call(&string, "length", "(I)I", &[JavaValue::Int(1)]) {
        Err(BridgeError::MethodNotFound {
            class,
            name,
            signature,
            ..
        }) if class == "java.lang.String" && name == "length" && signature == "(I)I" => {}
        Err(error) => {
            return Err(
                format!("unexpected JavaClassWrapper missing-overload error: {error}").into(),
            );
        }
        Ok(_value) => return Err("JavaClassWrapper missing overload unexpectedly resolved".into()),
    }

    println!("art_smoke: checking explicit class-loader lookup");
    write_dex_fixture()?;
    let system_loader = java.system_class_loader()?;
    let loader_java = java.with_loader(&system_loader);
    if loader_java.loader().is_none() {
        return Err("loader-backed Java unexpectedly lost its loader".into());
    }

    let loader_string_class = loader_java.find_class("java.lang.String")?;
    let cached_loader_string_class = loader_java.find_class("java.lang.String")?;
    let loader_descriptor_string_class = loader_java.find_class("Ljava/lang/String;")?;
    let loader_string_array_class = loader_java.find_class("[Ljava/lang/String;")?;
    let loader_descriptor_string_array_class = loader_java.find_class("[Ljava.lang.String;")?;
    let loader_int_array_class = loader_java.find_class("[I")?;
    if cached_loader_string_class.name() != "java.lang.String" {
        return Err(format!(
            "cached loader-backed String class name mismatch: {}",
            cached_loader_string_class.name()
        )
        .into());
    }

    let string = loader_java.new_string_utf("loader-backed")?;
    let length = expect_int(
        loader_string_class.call_method(&string, "length", "()I", &[])?,
        "loader-backed String.length",
    )?;
    if length != "loader-backed".len() as i32 {
        return Err(format!("loader-backed String.length mismatch: {length}").into());
    }
    let _ = loader_descriptor_string_class.call_static(
        "valueOf",
        "(I)Ljava/lang/String;",
        &[JavaValue::Int(123)],
    )?;
    if loader_string_array_class.name() != "[Ljava.lang.String;" {
        return Err(format!(
            "loader-backed array class name mismatch: {}",
            loader_string_array_class.name()
        )
        .into());
    }
    if loader_descriptor_string_array_class.name() != "[Ljava.lang.String;" {
        return Err(format!(
            "loader-backed dotted array class name mismatch: {}",
            loader_descriptor_string_array_class.name()
        )
        .into());
    }
    if loader_int_array_class.name() != "[I" {
        return Err(format!(
            "loader-backed primitive array class name mismatch: {}",
            loader_int_array_class.name()
        )
        .into());
    }

    let system_loader_object = expect_object(
        class_loader_class.call_static("getSystemClassLoader", "()Ljava/lang/ClassLoader;", &[])?,
        "ClassLoader.getSystemClassLoader",
    )?
    .ok_or("ClassLoader.getSystemClassLoader unexpectedly returned null")?;
    let _system_loader_from_object = java.class_loader_from_object(&system_loader_object)?;

    println!("art_smoke: checking DexClassLoader explicit lookup");
    let dex_class_loader_class = java.find_class("dalvik.system.DexClassLoader")?;
    let dex_path = java.new_string_utf(SMOKE_DEX)?;
    let dex_opt = java.new_string_utf(SMOKE_DEX_OPT)?;
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
    let smoke_subject = dex_java.find_class(SMOKE_SUBJECT)?;
    let cached_smoke_subject = dex_java.find_class(SMOKE_SUBJECT)?;
    if cached_smoke_subject.name() != SMOKE_SUBJECT {
        return Err(format!(
            "cached SmokeSubject class name mismatch: {}",
            cached_smoke_subject.name()
        )
        .into());
    }
    let answer = smoke_subject_answer(&smoke_subject, "SmokeSubject.answer")?;
    if answer != 42 {
        return Err(format!("SmokeSubject.answer mismatch: {answer}").into());
    }
    let smoke_wrapper = dex_java.use_class(SMOKE_SUBJECT)?;
    let answer = smoke_wrapper_answer(&smoke_wrapper, "JavaClassWrapper SmokeSubject.answer")?;
    if answer != 42 {
        return Err(format!("JavaClassWrapper SmokeSubject.answer mismatch: {answer}").into());
    }

    if method_replacement_reason
        .is_some_and(|reason| reason.contains("prerequisites are available"))
    {
        println!("art_smoke: checking experimental static method replacement");
        let replacement = unsafe {
            frida_java_bridge_rs::experimental::replace_static_i32_method(
                &smoke_subject,
                "answer",
                replacement_smoke_answer,
            )?
        };
        if let Some(summary) = replacement.debug_summary() {
            println!("art_smoke: experimental replacement layout {summary}");
        }
        let answer = smoke_subject_answer(&smoke_subject, "SmokeSubject.answer replacement")?;
        if answer != 1337 {
            return Err(format!("SmokeSubject.answer replacement mismatch: {answer}").into());
        }
        let answer = smoke_subject_answer(
            &cached_smoke_subject,
            "cached SmokeSubject.answer replacement",
        )?;
        if answer != 1337 {
            return Err(
                format!("cached SmokeSubject.answer replacement mismatch: {answer}").into(),
            );
        }
        let answer = smoke_wrapper_answer(
            &smoke_wrapper,
            "JavaClassWrapper SmokeSubject.answer replacement",
        )?;
        if answer != 1337 {
            return Err(format!(
                "JavaClassWrapper SmokeSubject.answer replacement mismatch: {answer}"
            )
            .into());
        }

        replacement.revert()?;
        let answer = smoke_subject_answer(&smoke_subject, "SmokeSubject.answer restored")?;
        if answer != 42 {
            return Err(format!("SmokeSubject.answer restored mismatch: {answer}").into());
        }
        let answer = smoke_wrapper_answer(
            &smoke_wrapper,
            "JavaClassWrapper SmokeSubject.answer restored",
        )?;
        if answer != 42 {
            return Err(format!(
                "JavaClassWrapper SmokeSubject.answer restored mismatch: {answer}"
            )
            .into());
        }

        {
            let _drop_replacement = unsafe {
                frida_java_bridge_rs::experimental::replace_static_i32_method(
                    &smoke_subject,
                    "answer",
                    replacement_smoke_answer,
                )?
            };
            let answer = smoke_subject_answer(
                &smoke_subject,
                "SmokeSubject.answer drop-revert replacement",
            )?;
            if answer != 1337 {
                return Err(format!(
                    "SmokeSubject.answer drop-revert replacement mismatch: {answer}"
                )
                .into());
            }
        }
        let answer = smoke_subject_answer(&smoke_subject, "SmokeSubject.answer drop-restored")?;
        if answer != 42 {
            return Err(format!("SmokeSubject.answer drop-restored mismatch: {answer}").into());
        }

        let replacement = unsafe {
            frida_java_bridge_rs::experimental::replace_static_i32_method(
                &smoke_subject,
                "answer",
                replacement_smoke_answer,
            )?
        };
        if let Some(summary) = replacement.debug_summary() {
            println!("art_smoke: experimental second replacement layout {summary}");
        }
        let answer = smoke_subject_answer(
            &cached_smoke_subject,
            "SmokeSubject.answer second replacement",
        )?;
        if answer != 1337 {
            return Err(
                format!("SmokeSubject.answer second replacement mismatch: {answer}").into(),
            );
        }
        replacement.revert()?;
        let answer = smoke_subject_answer(&smoke_subject, "SmokeSubject.answer second restored")?;
        if answer != 42 {
            return Err(format!("SmokeSubject.answer second restored mismatch: {answer}").into());
        }

        println!("art_smoke: checking experimental static primitive replacement matrix");
        check_static_void_replacement(&smoke_subject)?;
        check_static_boolean_cached_and_wrapper_replacement(
            &smoke_subject,
            &cached_smoke_subject,
            &smoke_wrapper,
        )?;
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_boolean_method,
            "staticBoolean",
            replacement_smoke_boolean,
            smoke_static_boolean,
            true,
            false
        );
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_byte_method,
            "staticByte",
            replacement_smoke_byte,
            smoke_static_byte,
            7,
            -8
        );
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_char_method,
            "staticChar",
            replacement_smoke_char,
            smoke_static_char,
            'A' as jni::jchar,
            'Z' as jni::jchar
        );
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_short_method,
            "staticShort",
            replacement_smoke_short,
            smoke_static_short,
            1234,
            -1234
        );
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_i64_method,
            "staticLong",
            replacement_smoke_long,
            smoke_static_long,
            1234567890123,
            -987654321012
        );
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_f32_method,
            "staticFloat",
            replacement_smoke_float,
            smoke_static_float,
            1.25,
            2.5
        );
        check_static_no_arg_replacement!(
            &smoke_subject,
            frida_java_bridge_rs::experimental::replace_static_f64_method,
            "staticDouble",
            replacement_smoke_double,
            smoke_static_double,
            3.5,
            9.25
        );
        check_static_string_replacement(&java, &smoke_subject)?;
        check_static_argument_replacements(&smoke_subject)?;
        check_static_replacement_negative_cases(&smoke_subject)?;
    } else {
        println!(
            "art_smoke: skipping experimental static method replacement: {:?}",
            method_replacement_reason
        );
    }
    let smoke_object = smoke_subject.new_object("()V", &[])?;
    let message = expect_object(
        smoke_subject.call_method(&smoke_object, "message", "()Ljava/lang/String;", &[])?,
        "SmokeSubject.message",
    )?
    .ok_or("SmokeSubject.message unexpectedly returned null")?;
    let message = message.get_string()?;
    if message != "dex-smoke" {
        return Err(format!("SmokeSubject.message mismatch: {message:?}").into());
    }

    if !smoke_wrapper
        .constructors()?
        .iter()
        .any(|method| method.signature.to_string() == "()V")
    {
        return Err("JavaClassWrapper SmokeSubject default constructor was not found".into());
    }
    let answer = expect_int(
        smoke_wrapper.call_static("answer", "()I", &[])?,
        "JavaClassWrapper SmokeSubject.answer",
    )?;
    if answer != 42 {
        return Err(format!("JavaClassWrapper SmokeSubject.answer mismatch: {answer}").into());
    }
    let smoke_object = smoke_wrapper.new_object("()V", &[])?;
    let message = expect_object(
        smoke_wrapper.call(&smoke_object, "message", "()Ljava/lang/String;", &[])?,
        "JavaClassWrapper SmokeSubject.message",
    )?
    .ok_or("JavaClassWrapper SmokeSubject.message unexpectedly returned null")?;
    let message = message.get_string()?;
    if message != "dex-smoke" {
        return Err(format!("JavaClassWrapper SmokeSubject.message mismatch: {message:?}").into());
    }

    println!("art_smoke: checking Java.use-style object helpers");
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
        return Err("JavaClassWrapper SmokeSubject did not recognize its instance".into());
    }
    let object_wrapper = java.use_class("java.lang.Object")?;
    if !object_wrapper.is_instance(&smoke_object)? {
        return Err("JavaClassWrapper Object did not recognize SmokeSubject instance".into());
    }
    let retained_object = object_wrapper.cast(&smoke_object)?;
    let _ = object_wrapper
        .call(&retained_object, "hashCode", "()I", &[])?
        .into_int("JavaClassWrapper retained Object.hashCode")?;

    let string_wrapper = java.use_class("java.lang.String")?;
    match string_wrapper.cast(&smoke_object) {
        Err(BridgeError::InvalidObjectType {
            operation: "JavaClassWrapper::cast",
            expected: "JavaClassWrapper target class",
            actual,
        }) if actual.contains("java.lang.String") => {}
        Err(error) => {
            return Err(format!("unexpected JavaClassWrapper cast error: {error}").into());
        }
        Ok(_value) => {
            return Err("JavaClassWrapper incompatible cast unexpectedly succeeded".into());
        }
    }

    println!("art_smoke: checking Java.use-style overload handles");
    let default_constructor = smoke_wrapper.constructor_overload(&[])?;
    if default_constructor.signature().to_string() != "()V" {
        return Err(format!(
            "JavaConstructorOverload default signature mismatch: {}",
            default_constructor.signature()
        )
        .into());
    }
    let smoke_object = default_constructor.new_object(&[])?;
    let int_constructor = smoke_wrapper.constructor_overload_by_name(&["int"])?;
    let numbered_object = int_constructor.new_object(&[JavaValue::Int(31)])?;
    let number_field = smoke_wrapper.field_handle("number")?;
    let number = expect_int(
        number_field.get(&numbered_object)?,
        "JavaFieldHandle SmokeSubject.number",
    )?;
    if number != 31 {
        return Err(format!("JavaFieldHandle SmokeSubject.number mismatch: {number}").into());
    }
    number_field.set(&numbered_object, JavaValue::Int(37))?;
    let number = expect_int(
        number_field.get(&numbered_object)?,
        "JavaFieldHandle SmokeSubject.number after set",
    )?;
    if number != 37 {
        return Err(
            format!("JavaFieldHandle SmokeSubject.number after set mismatch: {number}").into(),
        );
    }

    let message_overload = smoke_wrapper.method_overload("message", &[])?;
    let message = expect_object(
        message_overload.call(&smoke_object, &[])?,
        "JavaMethodOverload SmokeSubject.message",
    )?
    .ok_or("JavaMethodOverload SmokeSubject.message unexpectedly returned null")?;
    let message = message.get_string()?;
    if message != "dex-smoke" {
        return Err(
            format!("JavaMethodOverload SmokeSubject.message mismatch: {message:?}").into(),
        );
    }

    let overload_no_args = smoke_wrapper.method_overload("overload", &[])?;
    let value = expect_object(
        overload_no_args.call(&smoke_object, &[])?,
        "JavaMethodOverload SmokeSubject.overload()",
    )?
    .ok_or("JavaMethodOverload SmokeSubject.overload() unexpectedly returned null")?;
    let value = value.get_string()?;
    if value != "no-args" {
        return Err(
            format!("JavaMethodOverload SmokeSubject.overload() mismatch: {value:?}").into(),
        );
    }

    let overload_string =
        smoke_wrapper.method_overload_by_name("overload", &["java.lang.String"])?;
    let input = dex_java.new_string_utf("typed")?;
    let value = expect_object(
        overload_string.call(&smoke_object, &[JavaValue::from(&input)])?,
        "JavaMethodOverload SmokeSubject.overload(String)",
    )?
    .ok_or("JavaMethodOverload SmokeSubject.overload(String) unexpectedly returned null")?;
    let value = value.get_string()?;
    if value != "typed" {
        return Err(format!(
            "JavaMethodOverload SmokeSubject.overload(String) mismatch: {value:?}"
        )
        .into());
    }

    let answer_overload = smoke_wrapper.static_method_overload_by_name("answer", &[])?;
    let answer = expect_int(
        answer_overload.call_static(&[])?,
        "JavaMethodOverload SmokeSubject.answer",
    )?;
    if answer != 42 {
        return Err(format!("JavaMethodOverload SmokeSubject.answer mismatch: {answer}").into());
    }

    let static_text = smoke_wrapper.static_field_handle("STATIC_TEXT")?;
    let text = expect_object(
        static_text.get_static()?,
        "JavaFieldHandle SmokeSubject.STATIC_TEXT",
    )?
    .ok_or("JavaFieldHandle SmokeSubject.STATIC_TEXT unexpectedly returned null")?;
    let text = text.get_string()?;
    if text != "static-smoke" {
        return Err(format!("JavaFieldHandle SmokeSubject.STATIC_TEXT mismatch: {text:?}").into());
    }

    match smoke_wrapper.method_overload_by_name("overload", &["int"]) {
        Err(BridgeError::OverloadNotFound {
            class,
            name,
            arguments,
            ..
        }) if class == SMOKE_SUBJECT && name == "overload" && arguments == "(I)" => {}
        Err(error) => {
            return Err(format!(
                "unexpected JavaClassWrapper missing-overload-handle error: {error}"
            )
            .into());
        }
        Ok(_value) => {
            return Err("JavaClassWrapper missing overload handle unexpectedly resolved".into());
        }
    }

    match smoke_wrapper.field_handle("missing") {
        Err(BridgeError::FieldNameNotFound { class, name, .. })
            if class == SMOKE_SUBJECT && name == "missing" => {}
        Err(error) => {
            return Err(
                format!("unexpected JavaClassWrapper missing-field-handle error: {error}").into(),
            );
        }
        Ok(_value) => {
            return Err("JavaClassWrapper missing field handle unexpectedly resolved".into());
        }
    }

    println!("art_smoke: checking metadata reflection");
    let smoke_metadata = smoke_subject.metadata()?;
    if smoke_metadata.name != SMOKE_SUBJECT {
        return Err(format!(
            "SmokeSubject metadata name mismatch: {}",
            smoke_metadata.name
        )
        .into());
    }
    if smoke_metadata.descriptor != format!("L{};", SMOKE_SUBJECT.replace('.', "/")) {
        return Err(format!(
            "SmokeSubject metadata descriptor mismatch: {}",
            smoke_metadata.descriptor
        )
        .into());
    }
    if smoke_metadata.loader.is_none() {
        return Err("SmokeSubject metadata unexpectedly had no class loader".into());
    }

    let methods = smoke_subject.declared_methods()?;
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
        return Err("SmokeSubject.answer metadata did not report static modifier".into());
    }
    let hidden_static = require_method(
        &methods,
        "hiddenStatic",
        MethodKind::Static,
        "()Ljava/lang/String;",
        "SmokeSubject hiddenStatic",
    )?;
    if hidden_static.modifiers & 0x0002 == 0 {
        return Err("SmokeSubject.hiddenStatic metadata did not report private modifier".into());
    }

    let fields = smoke_subject.declared_fields()?;
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
        return Err("SmokeSubject.hidden metadata did not report private modifier".into());
    }

    println!("art_smoke: checking loaded-class and method query metadata");
    match java.enumerate_loaded_classes() {
        Ok(classes) => {
            if !capabilities.loaded_class_enumeration.is_supported() {
                return Err(format!(
                    "loaded-class enumeration succeeded despite unsupported capability: {:?}",
                    capabilities.loaded_class_enumeration
                )
                .into());
            }
            if !classes
                .iter()
                .any(|class| class.name() == "java.lang.String")
            {
                return Err("loaded-class enumeration did not include java.lang.String".into());
            }
            if !classes.iter().any(|class| class.name() == SMOKE_SUBJECT) {
                return Err("loaded-class enumeration did not include SmokeSubject".into());
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
                return Err(format!(
                    "method query did not include both overload signatures: {overload_signatures:?}"
                )
                .into());
            }

            let user_groups = java.enumerate_methods("java.lang.String!length/u")?;
            if user_groups
                .iter()
                .flat_map(|group| &group.classes)
                .any(|class| class.name == "java.lang.String")
            {
                return Err("method query /u did not skip bootstrap java.lang.String".into());
            }
        }
        Err(BridgeError::UnsupportedFeature {
            feature: "ART loaded-class enumeration",
            reason,
        }) => {
            if capabilities.loaded_class_enumeration.is_supported() {
                return Err(format!(
                    "loaded-class enumeration was unsupported despite supported capability: {reason}"
                )
                .into());
            }
        }
        Err(error) => {
            return Err(format!("unexpected loaded-class enumeration error: {error}").into());
        }
    }

    match java.find_class(SMOKE_SUBJECT) {
        Err(BridgeError::JavaException {
            operation: "JNIEnv::FindClass",
        }) => {}
        Err(error) => {
            return Err(format!("unexpected bootstrap SmokeSubject error: {error}").into());
        }
        Ok(_class) => {
            return Err("SmokeSubject unexpectedly resolved through bootstrap lookup".into());
        }
    }

    match java.find_class("frida.java.bridge.rs.MissingSmokeClass") {
        Err(BridgeError::JavaException {
            operation: "JNIEnv::FindClass",
        }) => {}
        Err(error) => {
            return Err(format!("unexpected JavaClass missing-class error: {error}").into());
        }
        Ok(_class) => return Err("JavaClass missing class unexpectedly resolved".into()),
    }

    println!("art_smoke: checking class-loader enumeration capability");
    match java.enumerate_class_loaders() {
        Ok(loaders) => {
            if !capabilities.class_loader_enumeration.is_supported() {
                return Err(format!(
                    "class-loader enumeration succeeded despite unsupported capability: {:?}",
                    capabilities.class_loader_enumeration
                )
                .into());
            }
            if loaders.is_empty() {
                return Err("class-loader enumeration returned no loaders".into());
            }
            let mut resolved = false;
            let mut resolved_dex = false;
            for loader in loaders {
                if loader.kind() != frida_java_bridge_rs::ClassLoaderKind::Enumerated {
                    return Err(format!(
                        "enumerated class loader had unexpected kind {:?}",
                        loader.kind()
                    )
                    .into());
                }
                let loader_java = java.with_loader(&loader);
                if loader_java.find_class("java.lang.String").is_ok() {
                    resolved = true;
                }
                if let Ok(smoke_subject) = loader_java.find_class(SMOKE_SUBJECT) {
                    let cached_smoke_subject = loader_java.find_class(SMOKE_SUBJECT)?;
                    if cached_smoke_subject.name() != SMOKE_SUBJECT {
                        return Err(format!(
                            "enumerated cached SmokeSubject class name mismatch: {}",
                            cached_smoke_subject.name()
                        )
                        .into());
                    }
                    let smoke_subject_array =
                        loader_java.find_class("[Lfrida.java.bridge.rs.smoke.SmokeSubject;")?;
                    if smoke_subject_array.name() != "[Lfrida.java.bridge.rs.smoke.SmokeSubject;" {
                        return Err(format!(
                            "enumerated SmokeSubject array name mismatch: {}",
                            smoke_subject_array.name()
                        )
                        .into());
                    }
                    let answer = expect_int(
                        smoke_subject.call_static("answer", "()I", &[])?,
                        "enumerated SmokeSubject.answer",
                    )?;
                    if answer != 42 {
                        return Err(
                            format!("enumerated SmokeSubject.answer mismatch: {answer}").into()
                        );
                    }
                    resolved_dex = true;
                }
            }
            if !resolved {
                return Err("no enumerated class loader resolved java.lang.String".into());
            }
            if !resolved_dex {
                return Err("no enumerated class loader resolved SmokeSubject".into());
            }
        }
        Err(BridgeError::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            reason,
        }) => {
            if capabilities.class_loader_enumeration.is_supported() {
                return Err(format!(
                    "class-loader enumeration was unsupported despite supported capability: {reason}"
                )
                .into());
            }
        }
        Err(error) => {
            return Err(format!("unexpected class-loader enumeration error: {error}").into());
        }
    }

    println!("art_smoke: ok");
    Ok(())
}

fn write_dex_fixture() -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(SMOKE_DIR)?;
    fs::create_dir_all(SMOKE_DEX_OPT)?;
    fs::write(SMOKE_DEX, SMOKE_DEX_BYTES)?;
    Ok(())
}

fn expect_int(value: JavaReturn, operation: &'static str) -> Result<i32, Box<dyn Error>> {
    match value {
        JavaReturn::Int(value) => Ok(value),
        other => Err(format!("{operation} returned unexpected value {other:?}").into()),
    }
}

fn smoke_static_boolean(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<bool, Box<dyn Error>> {
    Ok(class
        .call_static(name, "()Z", &[])?
        .into_boolean(operation)?)
}

fn smoke_static_byte(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<jni::jbyte, Box<dyn Error>> {
    Ok(class.call_static(name, "()B", &[])?.into_byte(operation)?)
}

fn smoke_static_char(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<jni::jchar, Box<dyn Error>> {
    Ok(class.call_static(name, "()C", &[])?.into_char(operation)?)
}

fn smoke_static_short(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<jni::jshort, Box<dyn Error>> {
    Ok(class.call_static(name, "()S", &[])?.into_short(operation)?)
}

fn smoke_static_int(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<i32, Box<dyn Error>> {
    expect_int(class.call_static(name, "()I", &[])?, operation)
}

fn smoke_subject_answer(class: &JavaClass, operation: &'static str) -> Result<i32, Box<dyn Error>> {
    smoke_static_int(class, "answer", operation)
}

fn smoke_static_long(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<jni::jlong, Box<dyn Error>> {
    Ok(class.call_static(name, "()J", &[])?.into_long(operation)?)
}

fn smoke_static_float(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<jni::jfloat, Box<dyn Error>> {
    Ok(class.call_static(name, "()F", &[])?.into_float(operation)?)
}

fn smoke_static_double(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<jni::jdouble, Box<dyn Error>> {
    Ok(class
        .call_static(name, "()D", &[])?
        .into_double(operation)?)
}

fn smoke_wrapper_answer(
    wrapper: &JavaClassWrapper,
    operation: &'static str,
) -> Result<i32, Box<dyn Error>> {
    expect_int(wrapper.call_static("answer", "()I", &[])?, operation)
}

fn smoke_wrapper_static_boolean(
    wrapper: &JavaClassWrapper,
    operation: &'static str,
) -> Result<bool, Box<dyn Error>> {
    Ok(wrapper
        .call_static("staticBoolean", "()Z", &[])?
        .into_boolean(operation)?)
}

fn smoke_static_void(
    class: &JavaClass,
    name: &str,
    operation: &'static str,
) -> Result<(), Box<dyn Error>> {
    Ok(class.call_static(name, "()V", &[])?.into_void(operation)?)
}

fn reset_void_counter(class: &JavaClass) -> Result<(), Box<dyn Error>> {
    smoke_static_void(class, "resetVoidCounter", "SmokeSubject.resetVoidCounter")
}

fn void_counter(class: &JavaClass, operation: &'static str) -> Result<i32, Box<dyn Error>> {
    smoke_static_int(class, "voidCounter", operation)
}

fn smoke_static_string(
    class: &JavaClass,
    operation: &'static str,
) -> Result<String, Box<dyn Error>> {
    let object = class
        .call_static("staticString", "()Ljava/lang/String;", &[])?
        .into_object(operation)?
        .ok_or_else(|| format!("{operation} unexpectedly returned null"))?;
    object.get_string().map_err(Into::into)
}

fn smoke_static_add(class: &JavaClass, operation: &'static str) -> Result<i32, Box<dyn Error>> {
    expect_int(
        class.call_static(
            "staticAdd",
            "(II)I",
            &[JavaValue::Int(5), JavaValue::Int(7)],
        )?,
        operation,
    )
}

fn smoke_static_primitive_mix(
    class: &JavaClass,
    operation: &'static str,
) -> Result<i32, Box<dyn Error>> {
    expect_int(
        class.call_static(
            "staticPrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(-2),
                JavaValue::Char('C' as jni::jchar),
                JavaValue::Short(30),
            ],
        )?,
        operation,
    )
}

fn smoke_static_wide(class: &JavaClass, operation: &'static str) -> Result<i64, Box<dyn Error>> {
    Ok(class
        .call_static(
            "staticWide",
            "(JD)J",
            &[JavaValue::Long(1000), JavaValue::Double(3.75)],
        )?
        .into_long(operation)?)
}

fn smoke_static_float_mix(
    class: &JavaClass,
    operation: &'static str,
) -> Result<f64, Box<dyn Error>> {
    Ok(class
        .call_static(
            "staticFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?
        .into_double(operation)?)
}

fn check_static_void_replacement(class: &JavaClass) -> Result<(), Box<dyn Error>> {
    reset_void_counter(class)?;
    smoke_static_void(
        class,
        "bumpVoidCounter",
        "SmokeSubject.bumpVoidCounter original",
    )?;
    let count = void_counter(class, "SmokeSubject.voidCounter original")?;
    if count != 1 {
        return Err(format!("SmokeSubject.voidCounter original mismatch: {count}").into());
    }

    reset_void_counter(class)?;
    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_void_method(
            class,
            "bumpVoidCounter",
            replacement_smoke_void,
        )?
    };
    smoke_static_void(
        class,
        "bumpVoidCounter",
        "SmokeSubject.bumpVoidCounter replacement",
    )?;
    let count = void_counter(class, "SmokeSubject.voidCounter replacement")?;
    if count != 0 {
        return Err(format!("SmokeSubject.voidCounter replacement mismatch: {count}").into());
    }
    replacement.revert()?;
    smoke_static_void(
        class,
        "bumpVoidCounter",
        "SmokeSubject.bumpVoidCounter restored",
    )?;
    let count = void_counter(class, "SmokeSubject.voidCounter restored")?;
    if count != 1 {
        return Err(format!("SmokeSubject.voidCounter restored mismatch: {count}").into());
    }

    reset_void_counter(class)?;
    {
        let _drop_replacement = unsafe {
            frida_java_bridge_rs::experimental::replace_static_void_method(
                class,
                "bumpVoidCounter",
                replacement_smoke_void,
            )?
        };
        smoke_static_void(
            class,
            "bumpVoidCounter",
            "SmokeSubject.bumpVoidCounter drop-replacement",
        )?;
        let count = void_counter(class, "SmokeSubject.voidCounter drop-replacement")?;
        if count != 0 {
            return Err(
                format!("SmokeSubject.voidCounter drop-replacement mismatch: {count}").into(),
            );
        }
    }
    smoke_static_void(
        class,
        "bumpVoidCounter",
        "SmokeSubject.bumpVoidCounter drop-restored",
    )?;
    let count = void_counter(class, "SmokeSubject.voidCounter drop-restored")?;
    if count != 1 {
        return Err(format!("SmokeSubject.voidCounter drop-restored mismatch: {count}").into());
    }

    reset_void_counter(class)?;
    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_void_method(
            class,
            "bumpVoidCounter",
            replacement_smoke_void,
        )?
    };
    smoke_static_void(
        class,
        "bumpVoidCounter",
        "SmokeSubject.bumpVoidCounter second replacement",
    )?;
    let count = void_counter(class, "SmokeSubject.voidCounter second replacement")?;
    if count != 0 {
        return Err(
            format!("SmokeSubject.voidCounter second replacement mismatch: {count}").into(),
        );
    }
    replacement.revert()?;
    smoke_static_void(
        class,
        "bumpVoidCounter",
        "SmokeSubject.bumpVoidCounter second restored",
    )?;
    let count = void_counter(class, "SmokeSubject.voidCounter second restored")?;
    if count != 1 {
        return Err(format!("SmokeSubject.voidCounter second restored mismatch: {count}").into());
    }

    Ok(())
}

fn check_static_boolean_cached_and_wrapper_replacement(
    class: &JavaClass,
    cached_class: &JavaClass,
    wrapper: &JavaClassWrapper,
) -> Result<(), Box<dyn Error>> {
    let value = smoke_static_boolean(class, "staticBoolean", "SmokeSubject.staticBoolean")?;
    if !value {
        return Err(format!("SmokeSubject.staticBoolean original mismatch: {value}").into());
    }
    let cached_value = smoke_static_boolean(
        cached_class,
        "staticBoolean",
        "cached SmokeSubject.staticBoolean",
    )?;
    if !cached_value {
        return Err(
            format!("cached SmokeSubject.staticBoolean original mismatch: {cached_value}").into(),
        );
    }
    let wrapper_value =
        smoke_wrapper_static_boolean(wrapper, "JavaClassWrapper SmokeSubject.staticBoolean")?;
    if !wrapper_value {
        return Err(format!(
            "JavaClassWrapper SmokeSubject.staticBoolean original mismatch: {wrapper_value}"
        )
        .into());
    }

    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_boolean_method(
            class,
            "staticBoolean",
            replacement_smoke_boolean,
        )?
    };
    if let Some(summary) = replacement.debug_summary() {
        println!("art_smoke: experimental boolean replacement layout {summary}");
    }

    let value = smoke_static_boolean(
        class,
        "staticBoolean",
        "SmokeSubject.staticBoolean replacement",
    )?;
    if value {
        return Err(format!("SmokeSubject.staticBoolean replacement mismatch: {value}").into());
    }
    let cached_value = smoke_static_boolean(
        cached_class,
        "staticBoolean",
        "cached SmokeSubject.staticBoolean replacement",
    )?;
    if cached_value {
        return Err(format!(
            "cached SmokeSubject.staticBoolean replacement mismatch: {cached_value}"
        )
        .into());
    }
    let wrapper_value = smoke_wrapper_static_boolean(
        wrapper,
        "JavaClassWrapper SmokeSubject.staticBoolean replacement",
    )?;
    if wrapper_value {
        return Err(format!(
            "JavaClassWrapper SmokeSubject.staticBoolean replacement mismatch: {wrapper_value}"
        )
        .into());
    }

    replacement.revert()?;

    let value = smoke_static_boolean(
        class,
        "staticBoolean",
        "SmokeSubject.staticBoolean restored",
    )?;
    if !value {
        return Err(format!("SmokeSubject.staticBoolean restored mismatch: {value}").into());
    }
    let cached_value = smoke_static_boolean(
        cached_class,
        "staticBoolean",
        "cached SmokeSubject.staticBoolean restored",
    )?;
    if !cached_value {
        return Err(
            format!("cached SmokeSubject.staticBoolean restored mismatch: {cached_value}").into(),
        );
    }
    let wrapper_value = smoke_wrapper_static_boolean(
        wrapper,
        "JavaClassWrapper SmokeSubject.staticBoolean restored",
    )?;
    if !wrapper_value {
        return Err(format!(
            "JavaClassWrapper SmokeSubject.staticBoolean restored mismatch: {wrapper_value}"
        )
        .into());
    }

    Ok(())
}

fn check_static_string_replacement(
    java: &frida_java_bridge_rs::Java,
    class: &JavaClass,
) -> Result<(), Box<dyn Error>> {
    let value = smoke_static_string(class, "SmokeSubject.staticString original")?;
    if value != "original-string" {
        return Err(format!("SmokeSubject.staticString original mismatch: {value:?}").into());
    }

    let replacement_string = java.new_string_utf("replacement-string")?;
    REPLACEMENT_STATIC_STRING.store(replacement_string.as_jobject(), Ordering::SeqCst);

    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_string_method(
            class,
            "staticString",
            replacement_smoke_string,
        )?
    };
    let value = smoke_static_string(class, "SmokeSubject.staticString replacement")?;
    if value != "replacement-string" {
        return Err(format!("SmokeSubject.staticString replacement mismatch: {value:?}").into());
    }
    replacement.revert()?;
    let value = smoke_static_string(class, "SmokeSubject.staticString restored")?;
    if value != "original-string" {
        return Err(format!("SmokeSubject.staticString restored mismatch: {value:?}").into());
    }

    {
        let _drop_replacement = unsafe {
            frida_java_bridge_rs::experimental::replace_static_string_method(
                class,
                "staticString",
                replacement_smoke_string,
            )?
        };
        let value = smoke_static_string(class, "SmokeSubject.staticString drop-replacement")?;
        if value != "replacement-string" {
            return Err(
                format!("SmokeSubject.staticString drop-replacement mismatch: {value:?}").into(),
            );
        }
    }
    let value = smoke_static_string(class, "SmokeSubject.staticString drop-restored")?;
    if value != "original-string" {
        return Err(format!("SmokeSubject.staticString drop-restored mismatch: {value:?}").into());
    }

    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_string_method(
            class,
            "staticString",
            replacement_smoke_string,
        )?
    };
    let value = smoke_static_string(class, "SmokeSubject.staticString second replacement")?;
    if value != "replacement-string" {
        return Err(
            format!("SmokeSubject.staticString second replacement mismatch: {value:?}").into(),
        );
    }
    replacement.revert()?;
    let value = smoke_static_string(class, "SmokeSubject.staticString second restored")?;
    if value != "original-string" {
        return Err(
            format!("SmokeSubject.staticString second restored mismatch: {value:?}").into(),
        );
    }

    REPLACEMENT_STATIC_STRING.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}

fn check_static_argument_replacements(class: &JavaClass) -> Result<(), Box<dyn Error>> {
    let value = smoke_static_add(class, "SmokeSubject.staticAdd original")?;
    if value != 12 {
        return Err(format!("SmokeSubject.staticAdd original mismatch: {value}").into());
    }

    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_i32_i32_to_i32_method(
            class,
            "staticAdd",
            replacement_smoke_add,
        )?
    };
    let value = smoke_static_add(class, "SmokeSubject.staticAdd replacement")?;
    if value != 507 {
        return Err(format!("SmokeSubject.staticAdd replacement mismatch: {value}").into());
    }
    replacement.revert()?;
    let value = smoke_static_add(class, "SmokeSubject.staticAdd restored")?;
    if value != 12 {
        return Err(format!("SmokeSubject.staticAdd restored mismatch: {value}").into());
    }

    {
        let _drop_replacement = unsafe {
            frida_java_bridge_rs::experimental::replace_static_i32_i32_to_i32_method(
                class,
                "staticAdd",
                replacement_smoke_add,
            )?
        };
        let value = smoke_static_add(class, "SmokeSubject.staticAdd drop-replacement")?;
        if value != 507 {
            return Err(
                format!("SmokeSubject.staticAdd drop-replacement mismatch: {value}").into(),
            );
        }
    }
    let value = smoke_static_add(class, "SmokeSubject.staticAdd drop-restored")?;
    if value != 12 {
        return Err(format!("SmokeSubject.staticAdd drop-restored mismatch: {value}").into());
    }

    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_i32_i32_to_i32_method(
            class,
            "staticAdd",
            replacement_smoke_add,
        )?
    };
    let value = smoke_static_add(class, "SmokeSubject.staticAdd second replacement")?;
    if value != 507 {
        return Err(format!("SmokeSubject.staticAdd second replacement mismatch: {value}").into());
    }
    replacement.revert()?;
    let value = smoke_static_add(class, "SmokeSubject.staticAdd second restored")?;
    if value != 12 {
        return Err(format!("SmokeSubject.staticAdd second restored mismatch: {value}").into());
    }

    let value = smoke_static_primitive_mix(class, "SmokeSubject.staticPrimitiveMix original")?;
    if value != 95 {
        return Err(format!("SmokeSubject.staticPrimitiveMix original mismatch: {value}").into());
    }
    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_z_b_c_s_to_i32_method(
            class,
            "staticPrimitiveMix",
            replacement_smoke_primitive_mix,
        )?
    };
    let value = smoke_static_primitive_mix(class, "SmokeSubject.staticPrimitiveMix replacement")?;
    if value != 2024 {
        return Err(
            format!("SmokeSubject.staticPrimitiveMix replacement mismatch: {value}").into(),
        );
    }
    replacement.revert()?;

    let value = smoke_static_wide(class, "SmokeSubject.staticWide original")?;
    if value != 1003 {
        return Err(format!("SmokeSubject.staticWide original mismatch: {value}").into());
    }
    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_i64_f64_to_i64_method(
            class,
            "staticWide",
            replacement_smoke_wide,
        )?
    };
    let value = smoke_static_wide(class, "SmokeSubject.staticWide replacement")?;
    if value != 987 {
        return Err(format!("SmokeSubject.staticWide replacement mismatch: {value}").into());
    }
    replacement.revert()?;

    let value = smoke_static_float_mix(class, "SmokeSubject.staticFloatMix original")?;
    if (value - 3.75).abs() > f64::EPSILON {
        return Err(format!("SmokeSubject.staticFloatMix original mismatch: {value}").into());
    }
    let replacement = unsafe {
        frida_java_bridge_rs::experimental::replace_static_f32_f64_to_f64_method(
            class,
            "staticFloatMix",
            replacement_smoke_float_mix,
        )?
    };
    let value = smoke_static_float_mix(class, "SmokeSubject.staticFloatMix replacement")?;
    if (value - 13.75).abs() > f64::EPSILON {
        return Err(format!("SmokeSubject.staticFloatMix replacement mismatch: {value}").into());
    }
    replacement.revert()?;

    Ok(())
}

fn check_static_replacement_negative_cases(class: &JavaClass) -> Result<(), Box<dyn Error>> {
    println!("art_smoke: checking experimental replacement negative cases");
    expect_replacement_method_not_found(
        unsafe {
            frida_java_bridge_rs::experimental::replace_static_i32_method(
                class,
                "missingStaticInt",
                replacement_smoke_answer,
            )
        },
        "missingStaticInt",
        "()I",
        "missing static replacement target",
    )?;
    expect_replacement_method_not_found(
        unsafe {
            frida_java_bridge_rs::experimental::replace_static_i32_method(
                class,
                "staticBoolean",
                replacement_smoke_answer,
            )
        },
        "staticBoolean",
        "()I",
        "wrong-signature static replacement target",
    )?;
    expect_replacement_method_not_found(
        unsafe {
            frida_java_bridge_rs::experimental::replace_static_i32_method(
                class,
                "instanceNumber",
                replacement_smoke_answer,
            )
        },
        "instanceNumber",
        "()I",
        "non-static replacement target",
    )?;
    let answer = smoke_subject_answer(class, "SmokeSubject.answer after negative replacement")?;
    if answer != 42 {
        return Err(
            format!("SmokeSubject.answer changed after negative replacement: {answer}").into(),
        );
    }
    Ok(())
}

fn expect_replacement_method_not_found(
    result: frida_java_bridge_rs::Result<frida_java_bridge_rs::experimental::StaticI32Replacement>,
    expected_name: &str,
    expected_signature: &str,
    operation: &'static str,
) -> Result<(), Box<dyn Error>> {
    match result {
        Err(BridgeError::MethodNotFound {
            class,
            name,
            signature,
            ..
        }) if class == SMOKE_SUBJECT
            && name == expected_name
            && signature == expected_signature =>
        {
            Ok(())
        }
        Err(BridgeError::JavaException {
            operation: "JNIEnv::GetStaticMethodID",
        }) => Ok(()),
        Err(error) => Err(format!("unexpected {operation} error: {error}").into()),
        Ok(replacement) => {
            replacement.revert()?;
            Err(format!("{operation} unexpectedly installed a replacement").into())
        }
    }
}

fn expect_object(
    value: JavaReturn,
    operation: &'static str,
) -> Result<Option<frida_java_bridge_rs::JavaObject>, Box<dyn Error>> {
    match value {
        JavaReturn::Object(value) => Ok(value),
        other => Err(format!("{operation} returned unexpected value {other:?}").into()),
    }
}

fn require_method<'a>(
    methods: &'a [frida_java_bridge_rs::JavaMethodMetadata],
    name: &str,
    kind: MethodKind,
    signature: &str,
    operation: &'static str,
) -> Result<&'a frida_java_bridge_rs::JavaMethodMetadata, Box<dyn Error>> {
    methods
        .iter()
        .find(|method| {
            method.name == name && method.kind == kind && method.signature.to_string() == signature
        })
        .ok_or_else(|| format!("{operation} metadata was not found").into())
}

fn require_field<'a>(
    fields: &'a [frida_java_bridge_rs::JavaFieldMetadata],
    name: &str,
    kind: FieldKind,
    ty: &JavaType,
    operation: &'static str,
) -> Result<&'a frida_java_bridge_rs::JavaFieldMetadata, Box<dyn Error>> {
    fields
        .iter()
        .find(|field| field.name == name && field.kind == kind && &field.ty == ty)
        .ok_or_else(|| format!("{operation} metadata was not found").into())
}

fn dlopen_global(name: &str) -> Result<*mut c_void, Box<dyn Error>> {
    let name = CString::new(name)?;
    let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
    if handle.is_null() {
        Err(format!("dlopen({}) failed: {}", LIBART, dlerror_message()).into())
    } else {
        Ok(handle)
    }
}

fn resolve_create_java_vm(handle: *mut c_void) -> Result<jni::JNICreateJavaVM, Box<dyn Error>> {
    let symbol = CString::new(JNI_CREATE_JAVA_VM)?;
    let pointer = unsafe { dlsym(handle, symbol.as_ptr()) };
    if pointer.is_null() {
        return Err(format!("dlsym({JNI_CREATE_JAVA_VM}) failed: {}", dlerror_message()).into());
    }

    debug_assert_eq!(
        mem::size_of::<jni::JNICreateJavaVM>(),
        mem::size_of::<*mut c_void>()
    );
    Ok(unsafe { mem::transmute_copy(&pointer) })
}

fn create_vm(create_java_vm: jni::JNICreateJavaVM) -> Result<(), Box<dyn Error>> {
    let option_strings = [
        CString::new("-Xcheck:jni")?,
        CString::new("-Xint")?,
        CString::new("-Djava.class.path=")?,
    ];
    let mut options = option_strings
        .iter()
        .map(|option| jni::JavaVMOption {
            option_string: option.as_ptr().cast_mut(),
            extra_info: ptr::null_mut(),
        })
        .collect::<Vec<_>>();

    let mut args = jni::JavaVMInitArgs {
        version: jni::JNI_VERSION_1_6,
        n_options: options
            .len()
            .try_into()
            .map_err(|_| "too many Java VM options")?,
        options: options.as_mut_ptr(),
        ignore_unrecognized: jni::JNI_FALSE,
    };
    let mut vm = ptr::null_mut();
    let mut env = ptr::null_mut();

    let result = unsafe { create_java_vm(&mut vm, &mut env, &mut args) };
    if result != jni::JNI_OK {
        return Err(format!("JNI_CreateJavaVM failed with JNI result {result}").into());
    }
    if vm.is_null() || env.is_null() {
        return Err("JNI_CreateJavaVM returned a null VM or Env".into());
    }

    Ok(())
}

unsafe extern "C" fn replacement_smoke_answer(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jint {
    1337
}

unsafe extern "C" fn replacement_smoke_void(_env: *mut jni::JNIEnv, _class: jni::jclass) {}

unsafe extern "C" fn replacement_smoke_string(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jstring {
    REPLACEMENT_STATIC_STRING.load(Ordering::SeqCst)
}

unsafe extern "C" fn replacement_smoke_boolean(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jboolean {
    jni::JNI_FALSE
}

unsafe extern "C" fn replacement_smoke_byte(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jbyte {
    -8
}

unsafe extern "C" fn replacement_smoke_char(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jchar {
    'Z' as jni::jchar
}

unsafe extern "C" fn replacement_smoke_short(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jshort {
    -1234
}

unsafe extern "C" fn replacement_smoke_long(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jlong {
    -987654321012
}

unsafe extern "C" fn replacement_smoke_float(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jfloat {
    2.5
}

unsafe extern "C" fn replacement_smoke_double(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jdouble {
    9.25
}

unsafe extern "C" fn replacement_smoke_add(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    left * 100 + right
}

unsafe extern "C" fn replacement_smoke_primitive_mix(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    flag: jni::jboolean,
    value: jni::jbyte,
    letter: jni::jchar,
    extra: jni::jshort,
) -> jni::jint {
    if flag == jni::JNI_TRUE {
        2024
    } else {
        value as jni::jint + letter as jni::jint + extra as jni::jint
    }
}

unsafe extern "C" fn replacement_smoke_wide(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    value: jni::jlong,
    extra: jni::jdouble,
) -> jni::jlong {
    value - extra as jni::jlong - 10
}

unsafe extern "C" fn replacement_smoke_float_mix(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    value: jni::jfloat,
    extra: jni::jdouble,
) -> jni::jdouble {
    value as jni::jdouble + extra + 10.0
}

fn device_label() -> String {
    let model = system_property("ro.product.model").unwrap_or_else(|| "unknown".to_owned());
    let device = system_property("ro.product.device").unwrap_or_else(|| "unknown".to_owned());
    let sdk = system_property("ro.build.version.sdk").unwrap_or_else(|| "unknown".to_owned());
    format!("{model} ({device}), SDK {sdk}")
}

fn system_property(name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return None;
    }
    Some(
        unsafe { CStr::from_ptr(value.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn dlerror_message() -> String {
    let error = unsafe { dlerror() };
    if error.is_null() {
        "unknown dlerror".to_owned()
    } else {
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}

// TODO: use `app_process` or a real app as the target for testing full ART behavior.

// Some Android ART builds load libsigchain and expect the main executable to
// export these callbacks.
#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn AddSpecialSignalHandlerFn(_signal: c_int, _action: *mut c_void) {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn RemoveSpecialSignalHandlerFn(_signal: c_int, _handler: *mut c_void) {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn EnsureFrontOfChain(_signal: c_int) {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SkipAddSignalHandler(_value: bool) {}

// Older ART-ish names, harmless to export too.
#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn InitializeSignalChain() {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn SetSpecialSignalHandlerFn(_signal: c_int, _handler: *mut c_void) {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ClaimSignalChain(_signal: c_int, _old_action: *mut c_void) {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn UnclaimSignalChain(_signal: c_int) {}

#[allow(clippy::missing_safety_doc)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn InvokeUserSignalHandler(
    _signal: c_int,
    _info: *mut siginfo_t,
    _context: *mut c_void,
) {
}
