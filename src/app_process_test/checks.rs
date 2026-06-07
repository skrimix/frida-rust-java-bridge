use super::assertions::*;
use super::*;

pub(super) fn run_low_level_checks(env: &Env) -> Result<()> {
    println!("app_process_test: checking low-level JNI helpers");
    let string_class = env.find_class("java/lang/String")?;
    let object_class = env.find_class("java/lang/Object")?;
    let math_class = env.find_class("java/lang/Math")?;
    let integer_class = env.find_class("java/lang/Integer")?;
    let atomic_integer_class = env.find_class("java/util/concurrent/atomic/AtomicInteger")?;
    let throwable_class = env.find_class("java/lang/Throwable")?;
    let runtime_exception_class = env.find_class("java/lang/RuntimeException")?;

    let string = env.new_string_utf("frida-rust-java-bridge")?;
    let copied = env.get_string(&string)?;
    if copied != "frida-rust-java-bridge" {
        return test_error(format!("string round-trip mismatch: {copied:?}"));
    }
    match unsafe { env.get_string_raw(std::ptr::null_mut()) } {
        Err(Error::NullReturn {
            operation: "JNIEnv::GetStringLength",
        }) => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!("null raw jstring unexpectedly copied as {value:?}"));
        }
    }

    let object_ctor = env.lookup_constructor(&object_class, "()V")?;
    // SAFETY: each low-level ID in this check is resolved from the class/receiver used with it.
    let object = unsafe { env.new_object(&object_class, &object_ctor, &[])? };
    let hash_code = env.lookup_instance_method(&object_class, "hashCode", "()I")?;
    let _ = unsafe { env.call_instance_int_method(&object, &hash_code, &[])? };

    let object_array = env.new_object_array(2, &object_class, None::<&RawObject>)?;
    if env.object_array_length(&object_array)? != 2 {
        return test_error("object array length mismatch");
    }
    env.set_object_array_element(&object_array, 0, Some(&object))?;
    if env
        .get_object_array_element_nullable(&object_array, 1)?
        .is_some()
    {
        return test_error("object array null element unexpectedly present");
    }
    let first = env
        .get_object_array_element_nullable(&object_array, 0)?
        .ok_or_else(|| test_failure("object array first element unexpectedly null"))?;
    if !env.is_same_object(&first, &object)? {
        return test_error("object array first element mismatch");
    }

    let int_array = env.new_int_array(&[1, 2, 3])?;
    if env.array_length(&int_array)? != 3 {
        return test_error("int array length mismatch");
    }
    let mut ints = [0; 3];
    env.get_int_array_region(&int_array, 0, &mut ints)?;
    if ints != [1, 2, 3] {
        return test_error(format!("int array region mismatch: {ints:?}"));
    }
    env.set_int_array_region(&int_array, 1, &[9, 10])?;
    env.get_int_array_region(&int_array, 0, &mut ints)?;
    if ints != [1, 9, 10] {
        return test_error(format!("int array region after set mismatch: {ints:?}"));
    }
    let mut empty_ints = [];
    env.get_int_array_region(&int_array, 3, &mut empty_ints)?;
    env.set_int_array_region(&int_array, 3, &[])?;
    match env.get_int_array_region(&int_array, 4, &mut empty_ints) {
        Err(Error::InvalidArgumentValue { index: 0, .. }) => {}
        Err(error) => return Err(error),
        Ok(()) => return test_error("empty int array get accepted out-of-bounds start"),
    }
    match env.set_int_array_region(&int_array, 4, &[]) {
        Err(Error::InvalidArgumentValue { index: 0, .. }) => {}
        Err(error) => return Err(error),
        Ok(()) => return test_error("empty int array set accepted out-of-bounds start"),
    }

    let boolean_array = env.new_boolean_array(&[jni::JNI_TRUE, jni::JNI_FALSE])?;
    env.get_int_array_region(&boolean_array, 0, &mut empty_ints)?;
    env.set_int_array_region(&boolean_array, 0, &[])?;
    let null_array = RawObject(std::ptr::null_mut());
    match env.get_int_array_region(&null_array, 0, &mut empty_ints) {
        Err(Error::NullReturn {
            operation: "primitive array region",
        }) => {}
        Err(error) => return Err(error),
        Ok(()) => return test_error("empty int array get accepted null array"),
    }
    match env.set_int_array_region(&null_array, 0, &[]) {
        Err(Error::NullReturn {
            operation: "primitive array region",
        }) => {}
        Err(error) => return Err(error),
        Ok(()) => return test_error("empty int array set accepted null array"),
    }
    let mut booleans = [jni::JNI_FALSE; 2];
    env.get_boolean_array_region(&boolean_array, 0, &mut booleans)?;
    if booleans != [jni::JNI_TRUE, jni::JNI_FALSE] {
        return test_error(format!("boolean array region mismatch: {booleans:?}"));
    }

    let string_length = env.lookup_instance_method(&string_class, "length", "()I")?;
    let length = unsafe { env.call_instance_int_method(&string, &string_length, &[])? };
    if length != "frida-rust-java-bridge".len() as i32 {
        return test_error(format!("string length mismatch: {length}"));
    }

    let abs = env.lookup_static_method(&math_class, "abs", "(I)I")?;
    let abs_value =
        unsafe { env.call_static_int_method(&math_class, &abs, &[JavaValue::Int(-42)])? };
    if abs_value != 42 {
        return test_error(format!("Math.abs result mismatch: {abs_value}"));
    }

    let max_value = env.lookup_static_field(&integer_class, "MAX_VALUE", "I")?;
    let max_value = unsafe { env.get_static_int_field(&integer_class, &max_value)? };
    if max_value != i32::MAX {
        return test_error(format!("Integer.MAX_VALUE mismatch: {max_value}"));
    }

    let atomic_ctor = env.lookup_constructor(&atomic_integer_class, "(I)V")?;
    let atomic =
        unsafe { env.new_object(&atomic_integer_class, &atomic_ctor, &[JavaValue::Int(7)])? };
    let atomic_value = env.lookup_instance_field(&atomic_integer_class, "value", "I")?;
    let value = unsafe { env.get_instance_int_field(&atomic, &atomic_value)? };
    if value != 7 {
        return test_error(format!("AtomicInteger.value mismatch: {value}"));
    }
    unsafe { env.set_instance_int_field(&atomic, &atomic_value, 19)? };
    let atomic_get = env.lookup_instance_method(&atomic_integer_class, "get", "()I")?;
    let value = unsafe { env.call_instance_int_method(&atomic, &atomic_get, &[])? };
    if value != 19 {
        return test_error(format!(
            "AtomicInteger.get mismatch after field set: {value}"
        ));
    }

    let initial_message = env.new_string_utf("initial")?;
    let exception_ctor =
        env.lookup_constructor(&runtime_exception_class, "(Ljava/lang/String;)V")?;
    let exception = unsafe {
        env.new_object(
            &runtime_exception_class,
            &exception_ctor,
            &[JavaValue::object_ref(initial_message.as_jobject())],
        )?
    };
    let detail_message =
        env.lookup_instance_field(&throwable_class, "detailMessage", "Ljava/lang/String;")?;
    let message = unsafe { env.get_instance_object_field(&exception, &detail_message)? }
        .ok_or_else(|| test_failure("Throwable.detailMessage unexpectedly null"))?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "initial" {
        return test_error(format!("Throwable.detailMessage mismatch: {message:?}"));
    }
    let updated_message = env.new_string_utf("updated")?;
    unsafe {
        env.set_instance_object_field(&exception, &detail_message, Some(&updated_message))?;
    }
    let get_message =
        env.lookup_instance_method(&throwable_class, "getMessage", "()Ljava/lang/String;")?;
    let message = unsafe { env.call_instance_object_method(&exception, &get_message, &[])? }
        .ok_or_else(|| test_failure("Throwable.getMessage unexpectedly returned null"))?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "updated" {
        return test_error(format!(
            "Throwable.getMessage mismatch after field set: {message:?}"
        ));
    }

    match env.find_class("frida/rust/java/bridge/MissingTestClass") {
        Err(Error::JavaException {
            operation: "JNIEnv::FindClass",
            ..
        }) => {}
        Err(error) => return Err(error),
        Ok(_class) => return test_error("missing class unexpectedly resolved"),
    }
    if env.exception_check() {
        env.exception_clear();
        return test_error("pending exception was not cleared after failed FindClass");
    }

    Ok(())
}

pub(super) fn run_convenience_checks(java: &Java, app_java: &Java) -> Result<()> {
    println!("app_process_test: checking convenience layer");
    let capabilities = java.capabilities();
    let method_replacement_reason = capabilities.method_replacement.unsupported_reason();
    let app_loader_deferral_reason = capabilities.app_loader_deferral.unsupported_reason();
    let main_thread_scheduling_reason = capabilities.main_thread_scheduling.unsupported_reason();
    println!("app_process_test: capabilities {capabilities:?}");
    println!(
        "app_process_test: method replacement unsupported reason {method_replacement_reason:?}"
    );
    println!(
        "app_process_test: app-loader deferral unsupported reason {app_loader_deferral_reason:?}"
    );
    println!(
        "app_process_test: main-thread scheduling unsupported reason {main_thread_scheduling_reason:?}"
    );

    check_android_version_and_perform_now(java, app_java)?;
    check_bootstrap_convenience(java)?;
    check_automatic_app_loader_surface(java)?;
    super::app_loader_checks::check_app_loader_surface(java, app_java)?;
    check_deoptimization_surface(java, app_java, &capabilities)?;
    check_main_thread_scheduling_surface(app_java, &capabilities)?;
    check_dex_class_loader(java)?;
    check_metadata_and_enumeration(
        java,
        app_java,
        capabilities.loaded_class_enumeration.is_supported(),
        capabilities.class_loader_enumeration.is_supported(),
    )?;
    Ok(())
}

fn check_deoptimization_surface(
    java: &Java,
    app_java: &Java,
    capabilities: &crate::JavaCapabilities,
) -> Result<()> {
    println!("app_process_test: checking deoptimization surface");

    let subject = app_java.use_class(TEST_SUBJECT)?;
    let identity = subject.method("staticIdentity")?.overload(["int"])?;
    let int_constructor = subject.constructor(["int"])?;

    if !capabilities.deoptimization.is_supported() {
        println!(
            "app_process_test: skipping live deoptimization checks: {:?}",
            capabilities.deoptimization.unsupported_reason()
        );
        expect_deoptimization_unsupported(java.deoptimize_boot_image(), "deoptimizeBootImage")?;
        expect_deoptimization_unsupported(java.deoptimize_everything(), "deoptimizeEverything")?;
        expect_deoptimization_unsupported(identity.deoptimize(), "method deoptimize")?;
        expect_deoptimization_unsupported(int_constructor.deoptimize(), "constructor deoptimize")?;
        return Ok(());
    }

    println!("app_process_test: deoptimizing boot image");
    java.deoptimize_boot_image()?;
    println!("app_process_test: deoptimizing everything");
    java.deoptimize_everything()?;
    println!("app_process_test: deoptimizing selected staticIdentity method");
    identity.deoptimize()?;
    expect_int(
        identity.call((), (21 as jni::jint,))?,
        21,
        "staticIdentity after selected method deoptimization",
    )?;

    println!("app_process_test: deoptimizing selected constructor");
    int_constructor.deoptimize()?;
    let object = int_constructor.new_object((33 as jni::jint,))?;
    let number = subject.field("number")?.get_int(&object)?;
    if number != 33 {
        return test_error(format!(
            "constructor after deoptimization initialized number as {number}"
        ));
    }

    if capabilities.method_replacement.is_supported() {
        let mut replacement = identity.replace(|ctx| ctx.ret(909))?;
        expect_deoptimization_unsupported(
            identity.deoptimize(),
            "active replacement target deoptimize",
        )?;
        expect_int(
            identity.call((), (21 as jni::jint,))?,
            909,
            "staticIdentity replacement after selected method deoptimization",
        )?;
        replacement.revert()?;
        expect_int(
            identity.call((), (21 as jni::jint,))?,
            21,
            "staticIdentity restored after replacement and deoptimization",
        )?;
    }

    Ok(())
}

fn expect_deoptimization_unsupported(result: Result<()>, operation: &'static str) -> Result<()> {
    match result {
        Err(Error::UnsupportedFeature {
            feature: "ART deoptimization",
            ..
        }) => Ok(()),
        Err(error) => Err(error),
        Ok(()) => test_error(format!(
            "{operation} succeeded despite unsupported capability"
        )),
    }
}

fn check_android_version_and_perform_now(java: &Java, app_java: &Java) -> Result<()> {
    println!("app_process_test: checking Android version and Java::perform_now");

    let version = java.android_version()?;
    if version.release.is_empty() {
        return test_error("Android release version was empty");
    }
    if version.api_level <= 0 {
        return test_error(format!(
            "Android API level was not positive: {}",
            version.api_level
        ));
    }
    if java.android_api_level()? != version.api_level {
        return test_error("Java Android API-level helper diverged from Android version");
    }

    let perform_now_counter = Arc::new(AtomicUsize::new(0));
    let counter_for_callback = perform_now_counter.clone();
    let string_name = java.perform_now(move |bootstrap_java| {
        if bootstrap_java.loader().is_some() {
            return Err(test_failure(
                "Java::perform_now callback received a loader-scoped Java handle",
            ));
        }
        let string_class = bootstrap_java.find_class("java.lang.String")?;
        counter_for_callback.fetch_add(1, Ordering::SeqCst);
        Ok(string_class.name().to_owned())
    })?;
    if string_name != "java.lang.String" {
        return test_error(format!(
            "Java::perform_now String class mismatch: {string_name}"
        ));
    }
    if perform_now_counter.load(Ordering::SeqCst) != 1 {
        return test_error("Java::perform_now callback did not run synchronously exactly once");
    }

    app_java.perform_now(|scoped_java| {
        if scoped_java.loader().is_none() {
            return Err(test_failure(
                "Java::perform_now did not preserve loader scope",
            ));
        }
        let subject = scoped_java.find_class(TEST_SUBJECT)?;
        let answer = read_int(
            subject.call_static("answer", "()I", &[])?,
            "Java::perform_now TestSubject.answer",
        )?;
        if answer != 42 {
            return test_error(format!(
                "Java::perform_now TestSubject.answer mismatch: {answer}"
            ));
        }
        Ok(())
    })?;

    Ok(())
}

pub(super) fn check_automatic_app_loader_surface(java: &Java) -> Result<()> {
    println!("app_process_test: checking automatic app-loader selection");

    match java.with_app_loader() {
        Ok(app_java) => {
            let loader = app_java.loader().ok_or_else(|| {
                test_failure("Java::with_app_loader returned a bootstrap Java handle")
            })?;
            if loader.kind() != ClassLoaderKind::App {
                return test_error(format!(
                    "Java::with_app_loader loader had unexpected kind {:?}",
                    loader.kind()
                ));
            }

            let subject = app_java.find_class(TEST_SUBJECT)?;
            let answer = read_int(
                subject.call_static("answer", "()I", &[])?,
                "Java::with_app_loader TestSubject.answer",
            )?;
            if answer != 42 {
                return test_error(format!(
                    "Java::with_app_loader TestSubject.answer mismatch: {answer}"
                ));
            }

            let default_loader = java
                .default_app_loader()
                .ok_or_else(|| test_failure("Java::default_app_loader was not published"))?;
            if default_loader.kind() != ClassLoaderKind::App {
                return test_error(format!(
                    "Java::default_app_loader had unexpected kind {:?}",
                    default_loader.kind()
                ));
            }

            let bare_wrapper_subject = java.use_class(TEST_SUBJECT)?;
            let bare_wrapper_answer = bare_wrapper_subject.call::<jni::jint>("answer", ())?;
            if bare_wrapper_answer != 42 {
                return test_error(format!(
                    "bare Java::use_class TestSubject.answer mismatch: {bare_wrapper_answer}"
                ));
            }

            let app_loader = java.app_class_loader()?;
            if app_loader.kind() != ClassLoaderKind::App {
                return test_error(format!(
                    "Java::app_class_loader had unexpected kind {:?}",
                    app_loader.kind()
                ));
            }

            let direct_app_java = java.with_loader(&app_loader);
            let loader = direct_app_java.loader().ok_or_else(|| {
                test_failure("Java::with_loader returned a bootstrap Java handle")
            })?;
            if loader.kind() != ClassLoaderKind::App {
                return test_error(format!(
                    "Java::with_loader app loader had unexpected kind {:?}",
                    loader.kind()
                ));
            }

            let perform_counter = Arc::new(AtomicUsize::new(0));
            let perform_counter_for_callback = perform_counter.clone();
            let handle = java.perform(move |app_java| {
                let loader = app_java.loader().ok_or_else(|| {
                    test_failure("perform callback received a bootstrap Java handle")
                })?;
                if loader.kind() != ClassLoaderKind::App {
                    return test_error(format!(
                        "perform callback loader had unexpected kind {:?}",
                        loader.kind()
                    ));
                }
                let subject = app_java.find_class(TEST_SUBJECT)?;
                let answer = read_int(
                    subject.call_static("answer", "()I", &[])?,
                    "Java::perform TestSubject.answer",
                )?;
                if answer != 42 {
                    return test_error(format!(
                        "Java::perform TestSubject.answer mismatch: {answer}"
                    ));
                }
                perform_counter_for_callback.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })?;
            if handle.status() != PerformStatus::Completed {
                return test_error(format!(
                    "Java::perform did not complete immediately: {:?}",
                    handle.status()
                ));
            }
            if perform_counter.load(Ordering::SeqCst) != 1 {
                return test_error("Java::perform callback did not run exactly once".to_owned());
            }
        }
        Err(Error::AppClassLoaderUnavailable { reason }) => {
            if !reason.contains("ActivityThread.currentApplication() returned null") {
                return test_error(format!(
                    "automatic app-loader unavailable for unexpected reason: {reason}"
                ));
            }

            require_app_loader_unavailable(java.app_class_loader(), "Java::app_class_loader")?;
            require_app_loader_unavailable(java.with_app_loader(), "Java::with_app_loader")?;
            check_deferred_perform_installs_pending_hook(java)?;
        }
        Err(error) => return Err(error),
    }

    Ok(())
}

pub(super) fn check_bootstrap_convenience(java: &Java) -> Result<()> {
    let string_class = java.find_class("java.lang.String")?;
    let math_class = java.find_class("java.lang.Math")?;
    let atomic_integer_class = java.find_class("java.util.concurrent.atomic.AtomicInteger")?;
    let throwable_class = java.find_class("java.lang.Throwable")?;
    let runtime_exception_class = java.find_class("java.lang.RuntimeException")?;

    let string = java.new_string_utf("frida-rust-java-bridge")?;
    let length = read_int(
        string_class.call_method(&string, "length", "()I", &[])?,
        "String.length",
    )?;
    if length != "frida-rust-java-bridge".len() as i32 {
        return test_error(format!("java::raw::Class String.length mismatch: {length}"));
    }
    let abs_value = read_int(
        math_class.call_static("abs", "(I)I", &[JavaValue::Int(-42)])?,
        "Math.abs",
    )?;
    if abs_value != 42 {
        return test_error(format!(
            "java::raw::Class Math.abs result mismatch: {abs_value}"
        ));
    }

    let atomic = atomic_integer_class.new_object("(I)V", &[JavaValue::Int(7)])?;
    let value = read_int(
        atomic_integer_class.get_field(&atomic, "value", "I")?,
        "AtomicInteger.value",
    )?;
    if value != 7 {
        return test_error(format!(
            "java::raw::Class AtomicInteger.value mismatch: {value}"
        ));
    }
    atomic_integer_class.set_field(&atomic, "value", "I", JavaValue::Int(19))?;
    let value = read_int(
        atomic_integer_class.call_method(&atomic, "get", "()I", &[])?,
        "AtomicInteger.get",
    )?;
    if value != 19 {
        return test_error(format!(
            "java::raw::Class AtomicInteger.get mismatch after field set: {value}"
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
    .ok_or_else(|| test_failure("java::raw::Class Throwable.detailMessage unexpectedly null"))?;
    let message = message.get_string()?;
    if message != "initial" {
        return test_error(format!(
            "java::raw::Class Throwable.detailMessage mismatch: {message:?}"
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
    .ok_or_else(|| {
        test_failure("java::raw::Class Throwable.getMessage unexpectedly returned null")
    })?;
    let message = message.get_string()?;
    if message != "updated" {
        return test_error(format!(
            "java::raw::Class Throwable.getMessage mismatch after field set: {message:?}"
        ));
    }

    println!("app_process_test: checking bootstrap Java.use-style wrapper");
    let string_wrapper = java.use_class("java.lang.String")?;
    let cached_string_wrapper = java.use_class("java.lang.String")?;
    if string_wrapper.name() != "java.lang.String"
        || cached_string_wrapper.class().name() != "java.lang.String"
    {
        return test_error("JavaClass String name mismatch");
    }
    if !string_wrapper
        .method("length")?
        .overloads()
        .iter()
        .any(|method| method.signature.to_string() == "()I")
    {
        return test_error("JavaClass String.length metadata was not found");
    }
    let string = java.new_string_utf("wrapper")?;
    let length_method = string_wrapper.method("length")?.overload([] as [&str; 0])?;
    let length = read_int(
        length_method.call_raw(&string, ())?,
        "JavaClass String.length",
    )?;
    if length != "wrapper".len() as i32 {
        return test_error(format!("JavaClass String.length mismatch: {length}"));
    }

    let math_wrapper = java.use_class("java.lang.Math")?;
    let abs_value = math_wrapper.call_with::<jni::jint>("abs", ["int"], -7)?;
    if abs_value != 7 {
        return test_error(format!("JavaClass Math.abs mismatch: {abs_value}"));
    }
    let integer_wrapper = java.use_class("java.lang.Integer")?;
    let max_value = integer_wrapper.get_field::<jni::jint>("MAX_VALUE")?;
    if max_value != i32::MAX {
        return test_error(format!("JavaClass Integer.MAX_VALUE mismatch: {max_value}"));
    }

    let runtime_exception_wrapper = java.use_class("java.lang.RuntimeException")?;
    let exception =
        runtime_exception_wrapper.new_with(["java.lang.String"], ("wrapper constructor",))?;
    let message = read_object(
        throwable_class.call_method(&exception, "getMessage", "()Ljava/lang/String;", &[])?,
        "JavaClass RuntimeException.getMessage",
    )?
    .ok_or_else(|| {
        test_failure("JavaClass RuntimeException.getMessage unexpectedly returned null")
    })?;
    let message = message.get_string()?;
    if message != "wrapper constructor" {
        return test_error(format!(
            "JavaClass RuntimeException.getMessage mismatch: {message:?}"
        ));
    }
    Ok(())
}

fn require_app_loader_unavailable<T>(result: Result<T>, operation: &'static str) -> Result<()> {
    match result {
        Err(Error::AppClassLoaderUnavailable { reason })
            if reason.contains("ActivityThread.currentApplication() returned null") =>
        {
            Ok(())
        }
        Err(error) => Err(error),
        Ok(_) => test_error(format!("{operation} unexpectedly resolved an app loader")),
    }
}

fn check_deferred_perform_installs_pending_hook(java: &Java) -> Result<()> {
    let capabilities = java.capabilities();
    if !capabilities.app_loader_deferral.is_supported() {
        println!(
            "app_process_test: skipping deferred perform hook check: {:?}",
            capabilities.app_loader_deferral.unsupported_reason()
        );
        return Ok(());
    }

    println!("app_process_test: checking deferred perform hook setup");
    let perform_counter = Arc::new(AtomicUsize::new(0));
    let perform_counter_for_callback = perform_counter.clone();
    let handle: PerformResult<()> = java.perform(move |_ctx| {
        perform_counter_for_callback.fetch_add(1, Ordering::SeqCst);
        Err(test_failure(
            "deferred perform callback ran before app loader was available",
        ))
    })?;

    if handle.status() != PerformStatus::Pending {
        return test_error(format!(
            "Java::perform did not stay pending before Application was available: {:?}",
            handle.status()
        ));
    }
    if perform_counter.load(Ordering::SeqCst) != 0 {
        return test_error(
            "Java::perform callback ran before Application was available".to_owned(),
        );
    }

    Ok(())
}

fn check_main_thread_scheduling_surface(
    app_java: &Java,
    capabilities: &crate::JavaCapabilities,
) -> Result<()> {
    if !capabilities.main_thread_scheduling.is_supported() {
        println!(
            "app_process_test: skipping main-thread scheduling check: {:?}",
            capabilities.main_thread_scheduling.unsupported_reason()
        );
        return Ok(());
    }
    println!("app_process_test: checking main-thread scheduling");

    if app_java.is_main_thread()? {
        println!(
            "app_process_test: skipping live main-thread drain because app_process nativeRun is executing on the main thread"
        );
        return Ok(());
    }

    let callback_counter = Arc::new(AtomicUsize::new(0));
    let callback_counter_for_callback = callback_counter.clone();
    let handle = app_java.schedule_on_main_thread(move |main_java| {
        let count = callback_counter_for_callback.fetch_add(1, Ordering::SeqCst) + 1;
        if count != 1 {
            return test_error(format!("main-thread callback ran {count} times"));
        }
        if !main_java.is_main_thread()? {
            return Err(test_failure(
                "scheduled callback did not run on the main thread",
            ));
        }
        let loader = main_java
            .loader()
            .ok_or_else(|| test_failure("scheduled callback received a bootstrap Java handle"))?;
        if loader.kind() != ClassLoaderKind::App {
            return test_error(format!(
                "scheduled callback loader had unexpected kind {:?}",
                loader.kind()
            ));
        }
        let subject = main_java.find_class(TEST_SUBJECT)?;
        let answer = read_int(
            subject.call_static("answer", "()I", &[])?,
            "scheduled TestSubject.answer",
        )?;
        if answer != 42 {
            return test_error(format!("scheduled TestSubject.answer mismatch: {answer}"));
        }
        Ok(())
    })?;

    if handle.status() != MainThreadTaskStatus::Pending {
        return test_error(format!(
            "main-thread callback was not queued: {:?}",
            handle.status()
        ));
    }

    for _ in 0..100 {
        match handle.status() {
            MainThreadTaskStatus::Pending => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            MainThreadTaskStatus::Completed => {
                if callback_counter.load(Ordering::SeqCst) != 1 {
                    return test_error("main-thread callback did not run exactly once".to_owned());
                }
                return Ok(());
            }
            MainThreadTaskStatus::Failed(error) => return Err(error),
        }
    }

    test_error(format!(
        "main-thread callback did not drain before timeout: {:?}",
        handle.status()
    ))
}

pub(super) fn check_dex_class_loader(java: &Java) -> Result<()> {
    println!("app_process_test: checking DexClassLoader explicit lookup");
    let class_loader_class = java.find_class("java.lang.ClassLoader")?;
    let system_loader_object = read_object(
        class_loader_class.call_static("getSystemClassLoader", "()Ljava/lang/ClassLoader;", &[])?,
        "ClassLoader.getSystemClassLoader",
    )?
    .ok_or_else(|| test_failure("ClassLoader.getSystemClassLoader unexpectedly returned null"))?;
    let system_loader = java.class_loader_from_object(&system_loader_object)?;

    let dex_class_loader_class = java.find_class("dalvik.system.DexClassLoader")?;
    let dex_path = java.new_string_utf(DEX_TEST_PATH)?;
    let dex_opt = java.new_string_utf(DEX_TEST_OPT)?;
    let dex_loader = dex_class_loader_class.new_object(
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/ClassLoader;)V",
        &[
            JavaValue::from(&dex_path),
            JavaValue::from(&dex_opt),
            JavaValue::NULL,
            JavaValue::object_ref(system_loader.as_jobject()),
        ],
    )?;
    let dex_loader = java.class_loader_from_object(&dex_loader)?;
    let dex_java = java.with_loader(&dex_loader);
    let subject = dex_java.find_class(DEX_TEST_SUBJECT)?;
    let cached_subject = dex_java.find_class(DEX_TEST_SUBJECT)?;
    if cached_subject.name() != DEX_TEST_SUBJECT {
        return test_error(format!(
            "cached DexTestSubject class name mismatch: {}",
            cached_subject.name()
        ));
    }
    let answer = read_int(
        subject.call_static("answer", "()I", &[])?,
        "DexTestSubject.answer",
    )?;
    if answer != 4242 {
        return test_error(format!("DexTestSubject.answer mismatch: {answer}"));
    }
    let message = read_object(
        subject.call_static("message", "()Ljava/lang/String;", &[])?,
        "DexTestSubject.message",
    )?
    .ok_or_else(|| test_failure("DexTestSubject.message unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "dex-only-test" {
        return test_error(format!("DexTestSubject.message mismatch: {message:?}"));
    }

    let wrapper_subject = dex_java.use_class(DEX_TEST_SUBJECT)?;
    let wrapper_answer = wrapper_subject.call::<jni::jint>("answer", ())?;
    if wrapper_answer != 4242 {
        return test_error(format!(
            "DexTestSubject wrapper answer mismatch: {wrapper_answer}"
        ));
    }

    match java.find_class(DEX_TEST_SUBJECT) {
        Err(Error::JavaException {
            operation: "JNIEnv::FindClass",
            ..
        }) => {}
        Err(error) => return Err(error),
        Ok(_class) => return test_error("DexTestSubject unexpectedly resolved without loader"),
    }
    Ok(())
}

pub(super) fn check_metadata_and_enumeration(
    java: &Java,
    app_java: &Java,
    loaded_class_enumeration_supported: bool,
    class_loader_enumeration_supported: bool,
) -> Result<()> {
    println!("app_process_test: checking metadata reflection");
    let subject = app_java.find_class(TEST_SUBJECT)?;
    let test_metadata = subject.metadata()?;
    if test_metadata.name != TEST_SUBJECT {
        return test_error(format!(
            "TestSubject metadata name mismatch: {}",
            test_metadata.name
        ));
    }
    if test_metadata.descriptor != format!("L{};", TEST_SUBJECT.replace('.', "/")) {
        return test_error(format!(
            "TestSubject metadata descriptor mismatch: {}",
            test_metadata.descriptor
        ));
    }
    if test_metadata.loader.is_none() {
        return test_error("TestSubject metadata unexpectedly had no class loader");
    }

    let methods = subject.declared_methods()?;
    require_method(
        &methods,
        "<init>",
        MethodKind::Constructor,
        "()V",
        "TestSubject default constructor",
    )?;
    require_method(
        &methods,
        "<init>",
        MethodKind::Constructor,
        "(I)V",
        "TestSubject int constructor",
    )?;
    require_method(
        &methods,
        "overload",
        MethodKind::Instance,
        "()Ljava/lang/String;",
        "TestSubject overload()",
    )?;
    require_method(
        &methods,
        "overload",
        MethodKind::Instance,
        "(Ljava/lang/String;)Ljava/lang/String;",
        "TestSubject overload(String)",
    )?;
    let answer_method = require_method(
        &methods,
        "answer",
        MethodKind::Static,
        "()I",
        "TestSubject answer",
    )?;
    if answer_method.modifiers & ACC_STATIC == 0 {
        return test_error("TestSubject.answer metadata did not report static modifier");
    }
    let hidden_static = require_method(
        &methods,
        "hiddenStatic",
        MethodKind::Static,
        "()Ljava/lang/String;",
        "TestSubject hiddenStatic",
    )?;
    if hidden_static.modifiers & ACC_PRIVATE == 0 {
        return test_error("TestSubject.hiddenStatic metadata did not report private modifier");
    }

    let fields = subject.declared_fields()?;
    require_field(
        &fields,
        "STATIC_TEXT",
        FieldKind::Static,
        &JavaType::Object("java/lang/String".to_owned()),
        "TestSubject STATIC_TEXT",
    )?;
    require_field(
        &fields,
        "number",
        FieldKind::Instance,
        &JavaType::Int,
        "TestSubject number",
    )?;
    let hidden_field = require_field(
        &fields,
        "hidden",
        FieldKind::Instance,
        &JavaType::Long,
        "TestSubject hidden",
    )?;
    if hidden_field.modifiers & ACC_PRIVATE == 0 {
        return test_error("TestSubject.hidden metadata did not report private modifier");
    }

    println!("app_process_test: checking loaded-class and method query metadata");
    match java.enumerate_loaded_classes() {
        Ok(classes) => {
            if !loaded_class_enumeration_supported {
                return test_error(
                    "loaded-class enumeration succeeded despite unsupported capability",
                );
            }
            if !classes
                .iter()
                .any(|class| class.name() == "java.lang.String")
            {
                return test_error("loaded-class enumeration did not include java.lang.String");
            }
            if !classes.iter().any(|class| class.name() == TEST_SUBJECT) {
                return test_error("loaded-class enumeration did not include TestSubject");
            }
            drop(classes);

            let groups =
                java.enumerate_methods("frida.rust.java.bridge.test.TestSubject!overload*/s")?;
            let mut overload_signatures = Vec::new();
            for group in &groups {
                for class in &group.classes {
                    if class.name == TEST_SUBJECT {
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
                return test_error(format!(
                    "method query did not include both overload signatures: {overload_signatures:?}"
                ));
            }
        }
        Err(Error::UnsupportedFeature {
            feature: "ART loaded-class enumeration",
            reason,
        }) => {
            if loaded_class_enumeration_supported {
                return test_error(format!(
                    "loaded-class enumeration was unsupported despite supported capability: {reason}"
                ));
            }
        }
        Err(error) => return Err(error),
    }

    println!("app_process_test: checking class-loader enumeration capability");
    match java.enumerate_class_loaders() {
        Ok(loaders) => {
            if !class_loader_enumeration_supported {
                return test_error(
                    "class-loader enumeration succeeded despite unsupported capability",
                );
            }
            if loaders.is_empty() {
                return test_error("class-loader enumeration returned no loaders");
            }
            let mut resolved_string = false;
            let mut resolved_subject = false;
            for loader in loaders {
                if loader.kind() != ClassLoaderKind::Enumerated {
                    return test_error(format!(
                        "enumerated class loader had unexpected kind {:?}",
                        loader.kind()
                    ));
                }
                let loader_java = java.with_loader(&loader);
                if loader_java.find_class("java.lang.String").is_ok() {
                    resolved_string = true;
                }
                if let Ok(subject) = loader_java.find_class(TEST_SUBJECT) {
                    let answer = read_int(
                        subject.call_static("answer", "()I", &[])?,
                        "enumerated TestSubject.answer",
                    )?;
                    if answer != 42 {
                        return test_error(format!(
                            "enumerated TestSubject.answer mismatch: {answer}"
                        ));
                    }
                    resolved_subject = true;
                }
            }
            if !resolved_string {
                return test_error("no enumerated class loader resolved java.lang.String");
            }
            if !resolved_subject {
                return test_error("no enumerated class loader resolved TestSubject");
            }
        }
        Err(Error::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            reason,
        }) => {
            if class_loader_enumeration_supported {
                return test_error(format!(
                    "class-loader enumeration was unsupported despite supported capability: {reason}"
                ));
            }
        }
        Err(error) => return Err(error),
    }

    Ok(())
}
