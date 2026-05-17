#[cfg(not(target_os = "android"))]
fn main() {
    eprintln!("art_test only runs on Android; use `just art-test` to build and run it there");
}

#[cfg(target_os = "android")]
fn main() {
    android::main();
}

#[cfg(target_os = "android")]
mod android {
    use std::{
        error::Error,
        ffi::{CStr, CString, c_char, c_int, c_void},
        mem,
    };

    use frida_java_bridge_rs::{Java, JavaValue, jni};

    const RTLD_NOW: c_int = 2;
    const RTLD_GLOBAL: c_int = 0x100;
    const LIBART: &str = "libart.so";
    const JNI_CREATE_JAVA_VM: &str = "JNI_CreateJavaVM";
    const PROP_VALUE_MAX: usize = 92;

    #[link(name = "dl")]
    unsafe extern "C" {
        fn dlopen(filename: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlerror() -> *const c_char;
        fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
    }

    // Some ART builds expect sigchain hooks to be exported by the main executable when ART is created
    // outside app_process. The test binary does not install special signal handlers, so no-op hooks
    // are enough to let ART complete native bootstrap.
    #[unsafe(no_mangle)]
    pub extern "C" fn AddSpecialSignalHandlerFn(_signal: c_int, _action: *mut c_void) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn RemoveSpecialSignalHandlerFn(_signal: c_int, _handler: *mut c_void) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn EnsureFrontOfChain(_signal: c_int) {}

    #[unsafe(no_mangle)]
    pub extern "C" fn SkipAddSignalHandler(_value: bool) {}

    pub(super) fn main() {
        if let Err(error) = run() {
            eprintln!("art_test: {error}");
            std::process::exit(1);
        }
    }

    fn run() -> Result<(), Box<dyn Error>> {
        println!("art_test: pid {}", std::process::id());
        println!("art_test: device {}", device_label());

        println!("art_test: loading ART");
        let art = dlopen_global(LIBART)?;
        let create_java_vm = resolve_create_java_vm(art)?;

        println!("art_test: creating Java VM");
        create_vm(create_java_vm)?;

        println!("art_test: obtaining Java bridge");
        let java = Java::obtain()?;
        let vm = java.vm();
        let env = vm.get_env()?;
        println!("art_test: JNI version 0x{:08x}", env.version());

        println!("art_test: attaching current thread");
        let env = vm.attach_current_thread()?;

        println!("art_test: checking bootstrap JNI path");
        let string_class = env.find_class("java/lang/String")?;
        let math_class = env.find_class("java/lang/Math")?;
        let string = env.new_string_utf("frida-java-bridge-rs")?;
        let copied = env.get_string(&string)?;
        if copied != "frida-java-bridge-rs" {
            return Err(format!("string round-trip mismatch: {copied:?}").into());
        }
        let string_length = env.lookup_instance_method(&string_class, "length", "()I")?;
        let length = env.call_instance_int_method(&string, &string_length, &[])?;
        if length != "frida-java-bridge-rs".len() as i32 {
            return Err(format!("string length mismatch: {length}").into());
        }
        let abs = env.lookup_static_method(&math_class, "abs", "(I)I")?;
        let abs_value = env.call_static_int_method(&math_class, &abs, &[JavaValue::Int(-42)])?;
        if abs_value != 42 {
            return Err(format!("Math.abs result mismatch: {abs_value}").into());
        }

        println!("art_test: checking bootstrap convenience path");
        let string_class = java.find_class("java.lang.String")?;
        let string = java.new_string_utf("bootstrap-wrapper")?;
        let length = string_class
            .call_method(&string, "length", "()I", &[])?
            .into_int("String.length")?;
        if length != "bootstrap-wrapper".len() as i32 {
            return Err(format!("JavaClass String.length mismatch: {length}").into());
        }

        println!("art_test: ok");
        Ok(())
    }

    fn dlopen_global(name: &str) -> Result<*mut c_void, Box<dyn Error>> {
        let name = CString::new(name)?;
        let handle = unsafe { dlopen(name.as_ptr(), RTLD_NOW | RTLD_GLOBAL) };
        if handle.is_null() {
            return Err(format!("dlopen failed: {}", dlerror_message()).into());
        }
        Ok(handle)
    }

    fn resolve_create_java_vm(handle: *mut c_void) -> Result<jni::JNICreateJavaVM, Box<dyn Error>> {
        let symbol_name = CString::new(JNI_CREATE_JAVA_VM)?;
        let symbol = unsafe { dlsym(handle, symbol_name.as_ptr()) };
        if symbol.is_null() {
            return Err(format!("{JNI_CREATE_JAVA_VM} not found: {}", dlerror_message()).into());
        }
        Ok(unsafe { mem::transmute::<*mut c_void, jni::JNICreateJavaVM>(symbol) })
    }

    fn create_vm(create_java_vm: jni::JNICreateJavaVM) -> Result<(), Box<dyn Error>> {
        let option = CString::new("-Xcheck:jni")?;
        let mut options = [jni::JavaVMOption {
            option_string: option.as_ptr() as *mut c_char,
            extra_info: std::ptr::null_mut(),
        }];
        let mut args = jni::JavaVMInitArgs {
            version: jni::JNI_VERSION_1_6,
            n_options: options.len() as jni::jint,
            options: options.as_mut_ptr(),
            ignore_unrecognized: jni::JNI_TRUE,
        };
        let mut vm: *mut jni::JavaVM = std::ptr::null_mut();
        let mut env: *mut jni::JNIEnv = std::ptr::null_mut();
        let result = unsafe {
            create_java_vm(
                &mut vm,
                &mut env as *mut *mut jni::JNIEnv as *mut *mut c_void,
                &mut args,
            )
        };
        if result != jni::JNI_OK {
            return Err(format!("JNI_CreateJavaVM failed with JNI result {result}").into());
        }
        if vm.is_null() || env.is_null() {
            return Err("JNI_CreateJavaVM returned a null VM or Env".into());
        }
        Ok(())
    }

    fn device_label() -> String {
        let manufacturer = system_property("ro.product.manufacturer").unwrap_or_default();
        let model = system_property("ro.product.model").unwrap_or_default();
        let sdk = system_property("ro.build.version.sdk").unwrap_or_default();
        format!("{manufacturer} {model} SDK {sdk}")
            .trim()
            .to_owned()
    }

    fn system_property(name: &str) -> Option<String> {
        let name = CString::new(name).ok()?;
        let mut value = [0 as c_char; PROP_VALUE_MAX];
        let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
        if len <= 0 {
            return None;
        }
        let value = unsafe { CStr::from_ptr(value.as_ptr()) }
            .to_string_lossy()
            .into_owned();
        Some(value)
    }

    fn dlerror_message() -> String {
        let error = unsafe { dlerror() };
        if error.is_null() {
            return "unknown dlerror".to_owned();
        }
        unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned()
    }
}
