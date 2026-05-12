use std::{
    error::Error,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs, mem, ptr,
};

use frida_java_bridge_rs::{Error as BridgeError, JavaReturn, JavaValue, Runtime, jni};

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;
const LIBART: &str = "libart.so";
const JNI_CREATE_JAVA_VM: &str = "JNI_CreateJavaVM";
const SMOKE_DIR: &str = "/data/local/tmp/frida-java-bridge-rs";
const SMOKE_DEX: &str = "/data/local/tmp/frida-java-bridge-rs/smoke-fixture.dex";
const SMOKE_DEX_OPT: &str = "/data/local/tmp/frida-java-bridge-rs/dex-cache";
const SMOKE_SUBJECT: &str = "frida.java.bridge.rs.smoke.SmokeSubject";
const SMOKE_DEX_BYTES: &[u8] = include_bytes!("../../smoke-fixtures/dex/classes.dex");

#[link(name = "dl")]
unsafe extern "C" {
    fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlerror() -> *const c_char;
}

fn main() {
    if let Err(error) = run() {
        eprintln!("art_smoke: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
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

    println!("art_smoke: checking explicit class-loader lookup");
    write_dex_fixture()?;
    let system_loader = java.system_class_loader()?;
    let loader_java = java.with_loader(&system_loader);
    if loader_java.loader().is_none() {
        return Err("loader-backed Java unexpectedly lost its loader".into());
    }

    let loader_string_class = loader_java.find_class("java.lang.String")?;
    let loader_descriptor_string_class = loader_java.find_class("Ljava/lang/String;")?;
    let loader_string_array_class = loader_java.find_class("[Ljava/lang/String;")?;
    let _loader_int_array_class = loader_java.find_class("[I")?;

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
    if loader_string_array_class.name() != "[Ljava/lang/String;" {
        return Err(format!(
            "loader-backed array class name mismatch: {}",
            loader_string_array_class.name()
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
    let answer = expect_int(
        smoke_subject.call_static("answer", "()I", &[])?,
        "SmokeSubject.answer",
    )?;
    if answer != 42 {
        return Err(format!("SmokeSubject.answer mismatch: {answer}").into());
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
            if loaders.is_empty() {
                return Err("class-loader enumeration returned no loaders".into());
            }
            let mut resolved = false;
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
                    break;
                }
            }
            if !resolved {
                return Err("no enumerated class loader resolved java.lang.String".into());
            }
        }
        Err(BridgeError::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            ..
        }) => {}
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

fn expect_object(
    value: JavaReturn,
    operation: &'static str,
) -> Result<Option<frida_java_bridge_rs::JavaObject>, Box<dyn Error>> {
    match value {
        JavaReturn::Object(value) => Ok(value),
        other => Err(format!("{operation} returned unexpected value {other:?}").into()),
    }
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
