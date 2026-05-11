use std::{
    error::Error,
    ffi::{CStr, CString, c_char, c_int, c_void},
    mem, ptr,
};

use frida_java_bridge_rs::{Error as BridgeError, Runtime, jni};

const RTLD_NOW: c_int = 2;
const RTLD_GLOBAL: c_int = 0x100;
const LIBART: &str = "libart.so";
const JNI_CREATE_JAVA_VM: &str = "JNI_CreateJavaVM";

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

    println!("art_smoke: round-tripping string");
    let string = env.new_string_utf("frida-java-bridge-rs")?;
    let copied = unsafe { env.get_string_utf(string)? };
    if copied != "frida-java-bridge-rs" {
        return Err(format!("string round-trip mismatch: {copied:?}").into());
    }

    println!("art_smoke: checking exception handling");
    match env.find_class("frida/java/bridge/rs/MissingSmokeClass") {
        Err(BridgeError::JavaException {
            operation: "JNIEnv::FindClass",
        }) => {}
        Err(error) => return Err(format!("unexpected missing-class error: {error}").into()),
        Ok(class) => {
            unsafe { env.delete_local_ref(class) };
            return Err("missing class unexpectedly resolved".into());
        }
    }

    if env.exception_check() {
        env.exception_clear();
        return Err("pending exception was not cleared after failed FindClass".into());
    }

    unsafe {
        env.delete_local_ref(string_class);
        env.delete_local_ref(string);
    }

    println!("art_smoke: ok");
    Ok(())
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
