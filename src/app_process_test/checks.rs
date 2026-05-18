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

    let string = env.new_string_utf("frida-java-bridge-rs")?;
    let copied = env.get_string(&string)?;
    if copied != "frida-java-bridge-rs" {
        return test_error(format!("string round-trip mismatch: {copied:?}"));
    }

    let object_ctor = env.lookup_constructor(&object_class, "()V")?;
    let object = env.new_object(&object_class, &object_ctor, &[])?;
    let hash_code = env.lookup_instance_method(&object_class, "hashCode", "()I")?;
    let _ = env.call_instance_int_method(&object, &hash_code, &[])?;

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

    let boolean_array = env.new_boolean_array(&[jni::JNI_TRUE, jni::JNI_FALSE])?;
    let mut booleans = [jni::JNI_FALSE; 2];
    env.get_boolean_array_region(&boolean_array, 0, &mut booleans)?;
    if booleans != [jni::JNI_TRUE, jni::JNI_FALSE] {
        return test_error(format!("boolean array region mismatch: {booleans:?}"));
    }

    let string_length = env.lookup_instance_method(&string_class, "length", "()I")?;
    let length = env.call_instance_int_method(&string, &string_length, &[])?;
    if length != "frida-java-bridge-rs".len() as i32 {
        return test_error(format!("string length mismatch: {length}"));
    }

    let abs = env.lookup_static_method(&math_class, "abs", "(I)I")?;
    let abs_value = env.call_static_int_method(&math_class, &abs, &[JavaValue::Int(-42)])?;
    if abs_value != 42 {
        return test_error(format!("Math.abs result mismatch: {abs_value}"));
    }

    let max_value = env.lookup_static_field(&integer_class, "MAX_VALUE", "I")?;
    let max_value = env.get_static_int_field(&integer_class, &max_value)?;
    if max_value != i32::MAX {
        return test_error(format!("Integer.MAX_VALUE mismatch: {max_value}"));
    }

    let atomic_ctor = env.lookup_constructor(&atomic_integer_class, "(I)V")?;
    let atomic = env.new_object(&atomic_integer_class, &atomic_ctor, &[JavaValue::Int(7)])?;
    let atomic_value = env.lookup_instance_field(&atomic_integer_class, "value", "I")?;
    let value = env.get_instance_int_field(&atomic, &atomic_value)?;
    if value != 7 {
        return test_error(format!("AtomicInteger.value mismatch: {value}"));
    }
    env.set_instance_int_field(&atomic, &atomic_value, 19)?;
    let atomic_get = env.lookup_instance_method(&atomic_integer_class, "get", "()I")?;
    let value = env.call_instance_int_method(&atomic, &atomic_get, &[])?;
    if value != 19 {
        return test_error(format!(
            "AtomicInteger.get mismatch after field set: {value}"
        ));
    }

    let initial_message = env.new_string_utf("initial")?;
    let exception_ctor =
        env.lookup_constructor(&runtime_exception_class, "(Ljava/lang/String;)V")?;
    let exception = env.new_object(
        &runtime_exception_class,
        &exception_ctor,
        &[JavaValue::from(&initial_message)],
    )?;
    let detail_message =
        env.lookup_instance_field(&throwable_class, "detailMessage", "Ljava/lang/String;")?;
    let message = env
        .get_instance_object_field(&exception, &detail_message)?
        .ok_or_else(|| test_failure("Throwable.detailMessage unexpectedly null"))?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "initial" {
        return test_error(format!("Throwable.detailMessage mismatch: {message:?}"));
    }
    let updated_message = env.new_string_utf("updated")?;
    env.set_instance_object_field(&exception, &detail_message, Some(&updated_message))?;
    let get_message =
        env.lookup_instance_method(&throwable_class, "getMessage", "()Ljava/lang/String;")?;
    let message = env
        .call_instance_object_method(&exception, &get_message, &[])?
        .ok_or_else(|| test_failure("Throwable.getMessage unexpectedly returned null"))?;
    let message = unsafe { env.get_string_raw(message.as_jobject())? };
    if message != "updated" {
        return test_error(format!(
            "Throwable.getMessage mismatch after field set: {message:?}"
        ));
    }

    match env.find_class("frida/java/bridge/rs/MissingTestClass") {
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
    if capabilities.flavor != RuntimeFlavor::Art {
        return test_error(format!(
            "unexpected runtime flavor {:?}",
            capabilities.flavor
        ));
    }
    if capabilities.deoptimization.is_supported()
        || capabilities
            .deoptimization
            .unsupported_reason()
            .is_none_or(|reason| !reason.contains("not implemented yet"))
    {
        return test_error(format!(
            "deoptimization capability was not explicitly deferred: {:?}",
            capabilities.deoptimization
        ));
    }
    let method_replacement_reason = capabilities.method_replacement.reason();
    let app_loader_deferral_reason = capabilities.app_loader_deferral.reason();
    let main_thread_scheduling_reason = capabilities.main_thread_scheduling.reason();
    println!("app_process_test: capabilities {capabilities:?}");
    println!(
        "app_process_test: method replacement capability reason {method_replacement_reason:?}"
    );
    println!(
        "app_process_test: app-loader deferral capability reason {app_loader_deferral_reason:?}"
    );
    println!(
        "app_process_test: main-thread scheduling capability reason {main_thread_scheduling_reason:?}"
    );
    if capabilities.method_replacement.is_supported() || method_replacement_reason.is_none() {
        return test_error(format!(
            "method replacement capability was not explicitly unsupported or experimental: {:?}",
            capabilities.method_replacement
        ));
    }
    if capabilities.app_loader_deferral.is_supported() || app_loader_deferral_reason.is_none() {
        return test_error(format!(
            "app-loader deferral capability was not explicitly unsupported or experimental: {:?}",
            capabilities.app_loader_deferral
        ));
    }
    if capabilities.main_thread_scheduling.is_supported() || main_thread_scheduling_reason.is_none()
    {
        return test_error(format!(
            "main-thread scheduling capability was not explicitly unsupported or experimental: {:?}",
            capabilities.main_thread_scheduling
        ));
    }

    check_android_version_and_perform_now(java, app_java)?;
    check_bootstrap_convenience(java)?;
    check_automatic_app_loader_surface(java)?;
    check_app_loader_surface(java, app_java)?;
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
            return Err(Error::UnsupportedFeature {
                feature: "app-process perform_now check",
                reason: "Java::perform_now callback received a loader-scoped Java handle"
                    .to_owned(),
            });
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
            return Err(Error::UnsupportedFeature {
                feature: "app-process perform_now check",
                reason: "Java::perform_now did not preserve loader scope".to_owned(),
            });
        }
        let subject = scoped_java.find_class(TEST_SUBJECT)?;
        let answer = read_int(
            subject.call_static("answer", "()I", &[])?,
            "Java::perform_now TestSubject.answer",
        )?;
        if answer != 42 {
            return Err(Error::UnsupportedFeature {
                feature: "app-process perform_now check",
                reason: format!("Java::perform_now TestSubject.answer mismatch: {answer}"),
            });
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
            let bare_wrapper_answer = read_int(
                bare_wrapper_subject.call_static("answer", "()I", ())?,
                "bare Java::use_class TestSubject.answer",
            )?;
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
                let loader = app_java.loader().ok_or_else(|| Error::UnsupportedFeature {
                    feature: "app-process perform check",
                    reason: "perform callback received a bootstrap Java handle".to_owned(),
                })?;
                if loader.kind() != ClassLoaderKind::App {
                    return Err(Error::UnsupportedFeature {
                        feature: "app-process perform check",
                        reason: format!(
                            "perform callback loader had unexpected kind {:?}",
                            loader.kind()
                        ),
                    });
                }
                let subject = app_java.find_class(TEST_SUBJECT)?;
                let answer = read_int(
                    subject.call_static("answer", "()I", &[])?,
                    "Java::perform TestSubject.answer",
                )?;
                if answer != 42 {
                    return Err(Error::UnsupportedFeature {
                        feature: "app-process perform check",
                        reason: format!("Java::perform TestSubject.answer mismatch: {answer}"),
                    });
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

    let string = java.new_string_utf("frida-java-bridge-rs")?;
    let length = read_int(
        string_class.call_method(&string, "length", "()I", &[])?,
        "String.length",
    )?;
    if length != "frida-java-bridge-rs".len() as i32 {
        return test_error(format!("JavaClass String.length mismatch: {length}"));
    }
    let abs_value = read_int(
        math_class.call_static("abs", "(I)I", &[JavaValue::Int(-42)])?,
        "Math.abs",
    )?;
    if abs_value != 42 {
        return test_error(format!("JavaClass Math.abs result mismatch: {abs_value}"));
    }

    let atomic = atomic_integer_class.new_object("(I)V", &[JavaValue::Int(7)])?;
    let value = read_int(
        atomic_integer_class.get_field(&atomic, "value", "I")?,
        "AtomicInteger.value",
    )?;
    if value != 7 {
        return test_error(format!("JavaClass AtomicInteger.value mismatch: {value}"));
    }
    atomic_integer_class.set_field(&atomic, "value", "I", JavaValue::Int(19))?;
    let value = read_int(
        atomic_integer_class.call_method(&atomic, "get", "()I", &[])?,
        "AtomicInteger.get",
    )?;
    if value != 19 {
        return test_error(format!(
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
    .ok_or_else(|| test_failure("JavaClass Throwable.detailMessage unexpectedly null"))?;
    let message = message.get_string()?;
    if message != "initial" {
        return test_error(format!(
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
    .ok_or_else(|| test_failure("JavaClass Throwable.getMessage unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "updated" {
        return test_error(format!(
            "JavaClass Throwable.getMessage mismatch after field set: {message:?}"
        ));
    }

    println!("app_process_test: checking bootstrap Java.use-style wrapper");
    let string_wrapper = java.use_class("java.lang.String")?;
    let cached_string_wrapper = java.use_class("java.lang.String")?;
    if string_wrapper.name() != "java.lang.String"
        || cached_string_wrapper.class().name() != "java.lang.String"
    {
        return test_error("JavaClassWrapper String name mismatch");
    }
    if !string_wrapper
        .methods("length")?
        .iter()
        .any(|method| method.signature.to_string() == "()I")
    {
        return test_error("JavaClassWrapper String.length metadata was not found");
    }
    let string = java.new_string_utf("wrapper")?;
    let length = read_int(
        string_wrapper.call(&string, "length", "()I", ())?,
        "JavaClassWrapper String.length",
    )?;
    if length != "wrapper".len() as i32 {
        return test_error(format!("JavaClassWrapper String.length mismatch: {length}"));
    }

    let math_wrapper = java.use_class("java.lang.Math")?;
    let abs_value = read_int(
        math_wrapper.call_static("abs", "(I)I", [JavaValue::Int(-7)])?,
        "JavaClassWrapper Math.abs",
    )?;
    if abs_value != 7 {
        return test_error(format!("JavaClassWrapper Math.abs mismatch: {abs_value}"));
    }
    let integer_wrapper = java.use_class("java.lang.Integer")?;
    let max_value = read_int(
        integer_wrapper.get_static_field("MAX_VALUE", "I")?,
        "JavaClassWrapper Integer.MAX_VALUE",
    )?;
    if max_value != i32::MAX {
        return test_error(format!(
            "JavaClassWrapper Integer.MAX_VALUE mismatch: {max_value}"
        ));
    }

    let runtime_exception_wrapper = java.use_class("java.lang.RuntimeException")?;
    let exception =
        runtime_exception_wrapper.new_instance(["java.lang.String"], ("wrapper constructor",))?;
    let message = read_object(
        throwable_class.call_method(&exception, "getMessage", "()Ljava/lang/String;", &[])?,
        "JavaClassWrapper RuntimeException.getMessage",
    )?
    .ok_or_else(|| {
        test_failure("JavaClassWrapper RuntimeException.getMessage unexpectedly returned null")
    })?;
    let message = message.get_string()?;
    if message != "wrapper constructor" {
        return test_error(format!(
            "JavaClassWrapper RuntimeException.getMessage mismatch: {message:?}"
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
    let Some(reason) = capabilities.app_loader_deferral.experimental_reason() else {
        println!(
            "app_process_test: skipping deferred perform hook check: {:?}",
            capabilities.app_loader_deferral.reason()
        );
        return Ok(());
    };
    if !reason.contains("prerequisites are available") {
        println!("app_process_test: skipping deferred perform hook check: {reason}");
        return Ok(());
    }

    println!("app_process_test: checking deferred perform hook setup");
    let perform_counter = Arc::new(AtomicUsize::new(0));
    let perform_counter_for_callback = perform_counter.clone();
    let handle = java.perform(move |_| {
        perform_counter_for_callback.fetch_add(1, Ordering::SeqCst);
        Err(Error::UnsupportedFeature {
            feature: "app-process deferred perform check",
            reason: "deferred perform callback ran before app loader was available".to_owned(),
        })
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
    let Some(reason) = capabilities.main_thread_scheduling.experimental_reason() else {
        println!(
            "app_process_test: skipping main-thread scheduling check: {:?}",
            capabilities.main_thread_scheduling.reason()
        );
        return Ok(());
    };
    println!("app_process_test: checking main-thread scheduling: {reason}");

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
            return Err(Error::UnsupportedFeature {
                feature: "app-process main-thread scheduling check",
                reason: format!("main-thread callback ran {count} times"),
            });
        }
        if !main_java.is_main_thread()? {
            return Err(Error::UnsupportedFeature {
                feature: "app-process main-thread scheduling check",
                reason: "scheduled callback did not run on the main thread".to_owned(),
            });
        }
        let loader = main_java
            .loader()
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: "app-process main-thread scheduling check",
                reason: "scheduled callback received a bootstrap Java handle".to_owned(),
            })?;
        if loader.kind() != ClassLoaderKind::App {
            return Err(Error::UnsupportedFeature {
                feature: "app-process main-thread scheduling check",
                reason: format!(
                    "scheduled callback loader had unexpected kind {:?}",
                    loader.kind()
                ),
            });
        }
        let subject = main_java.find_class(TEST_SUBJECT)?;
        let answer = read_int(
            subject.call_static("answer", "()I", &[])?,
            "scheduled TestSubject.answer",
        )?;
        if answer != 42 {
            return Err(Error::UnsupportedFeature {
                feature: "app-process main-thread scheduling check",
                reason: format!("scheduled TestSubject.answer mismatch: {answer}"),
            });
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

pub(super) fn check_app_loader_surface(java: &Java, app_java: &Java) -> Result<()> {
    println!("app_process_test: checking app-loader class and wrapper surface");
    let capabilities = java.capabilities();
    let heap_enumeration_available = capabilities.heap_enumeration.is_supported()
        || capabilities.heap_enumeration.is_experimental();
    if app_java.loader().is_none() {
        return test_error("app-loader Java unexpectedly lost its loader");
    }

    let subject = app_java.find_class(TEST_SUBJECT)?;
    let cached_subject = app_java.find_class(TEST_SUBJECT)?;
    if cached_subject.name() != TEST_SUBJECT {
        return test_error(format!(
            "cached TestSubject class name mismatch: {}",
            cached_subject.name()
        ));
    }
    let answer = read_int(
        subject.call_static("answer", "()I", &[])?,
        "TestSubject.answer",
    )?;
    if answer != 42 {
        return test_error(format!("TestSubject.answer mismatch: {answer}"));
    }
    let test_object = subject.new_object("()V", &[])?;
    let message = read_object(
        subject.call_method(&test_object, "message", "()Ljava/lang/String;", &[])?,
        "TestSubject.message",
    )?
    .ok_or_else(|| test_failure("TestSubject.message unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "dex-test" {
        return test_error(format!("TestSubject.message mismatch: {message:?}"));
    }

    let test_wrapper = app_java.use_class(TEST_SUBJECT)?;
    if !test_wrapper
        .constructors()?
        .iter()
        .any(|method| method.signature.to_string() == "()V")
    {
        return test_error("JavaClassWrapper TestSubject default constructor was not found");
    }
    let answer = read_int(
        test_wrapper.call_static("answer", "()I", ())?,
        "JavaClassWrapper TestSubject.answer",
    )?;
    if answer != 42 {
        return test_error(format!(
            "JavaClassWrapper TestSubject.answer mismatch: {answer}"
        ));
    }
    let test_object = test_wrapper.new_object("()V", ())?;
    let message = read_object(
        test_wrapper.call(&test_object, "message", "()Ljava/lang/String;", ())?,
        "JavaClassWrapper TestSubject.message",
    )?
    .ok_or_else(|| {
        test_failure("JavaClassWrapper TestSubject.message unexpectedly returned null")
    })?;
    let message = message.get_string()?;
    if message != "dex-test" {
        return test_error(format!(
            "JavaClassWrapper TestSubject.message mismatch: {message:?}"
        ));
    }

    let wrapper_methods = test_wrapper.declared_methods()?;
    require_method(
        &wrapper_methods,
        "message",
        MethodKind::Instance,
        "()Ljava/lang/String;",
        "JavaClassWrapper declared TestSubject.message",
    )?;
    let wrapper_fields = test_wrapper.declared_fields()?;
    require_field(
        &wrapper_fields,
        "number",
        FieldKind::Instance,
        &JavaType::Int,
        "JavaClassWrapper declared TestSubject.number",
    )?;
    if !test_wrapper.is_instance(&test_object)? {
        return test_error("JavaClassWrapper TestSubject did not recognize its instance");
    }
    let object_wrapper = java.use_class("java.lang.Object")?;
    if !object_wrapper.is_instance(&test_object)? {
        return test_error("JavaClassWrapper Object did not recognize TestSubject instance");
    }
    let retained_object = object_wrapper.cast(&test_object)?;
    let _ = object_wrapper
        .call(&retained_object, "hashCode", "()I", ())?
        .into_int("JavaClassWrapper retained Object.hashCode")?;

    println!("app_process_test: checking app-loader overload handles");
    let default_constructor = test_wrapper.constructor_overload(&[])?;
    if default_constructor.signature().to_string() != "()V" {
        return test_error(format!(
            "JavaConstructorOverload default signature mismatch: {}",
            default_constructor.signature()
        ));
    }
    let test_object = default_constructor.new_object(())?;
    let constructor_alias_object = test_wrapper.constructor([])?.new_object(())?;
    let alias_message = read_object(
        test_wrapper.call(
            &constructor_alias_object,
            "message",
            "()Ljava/lang/String;",
            (),
        )?,
        "JavaClassWrapper constructor alias TestSubject.message",
    )?
    .ok_or_else(|| test_failure("JavaClassWrapper constructor alias message null"))?;
    let alias_message = alias_message.get_string()?;
    if alias_message != "dex-test" {
        return test_error(format!(
            "JavaClassWrapper constructor alias message mismatch: {alias_message:?}"
        ));
    }
    let int_constructor = test_wrapper.constructor_overload_by_name(&["int"])?;
    let numbered_object = int_constructor.new_object((31 as jni::jint,))?;
    let alias_numbered_object = test_wrapper.new_instance(["int"], (31 as jni::jint,))?;
    let number_field = test_wrapper.field_handle("number")?;
    let number = number_field.get_int(&numbered_object)?;
    if number != 31 {
        return test_error(format!(
            "JavaFieldHandle TestSubject.number mismatch: {number}"
        ));
    }
    let alias_number = number_field.get_int(&alias_numbered_object)?;
    if alias_number != 31 {
        return test_error(format!(
            "JavaClassWrapper new_instance TestSubject.number mismatch: {alias_number}"
        ));
    }
    match test_wrapper.constructor(["java.lang.String"]) {
        Err(Error::OverloadNotFound {
            class,
            kind: "constructor",
            name,
            arguments,
        }) if class == TEST_SUBJECT && name == "$init" && arguments == "(Ljava/lang/String;)" => {}
        Err(error) => return Err(error),
        Ok(_) => {
            return test_error("missing TestSubject(String) constructor unexpectedly resolved");
        }
    }
    number_field.set_int(&numbered_object, 37)?;
    let number = number_field.get_int(&numbered_object)?;
    if number != 37 {
        return test_error(format!(
            "JavaFieldHandle TestSubject.number after set mismatch: {number}"
        ));
    }
    let flag_field = test_wrapper.field_handle("flag")?;
    if !flag_field.get_boolean(&numbered_object)? {
        return test_error("JavaFieldHandle TestSubject.flag mismatch");
    }
    flag_field.set_boolean(&numbered_object, false)?;
    if flag_field.get_boolean(&numbered_object)? {
        return test_error("JavaFieldHandle TestSubject.flag after set mismatch");
    }
    let small_field = test_wrapper.field_handle("small")?;
    if small_field.get_byte(&numbered_object)? != 2 {
        return test_error("JavaFieldHandle TestSubject.small mismatch");
    }
    small_field.set_byte(&numbered_object, 3)?;
    if small_field.get_byte(&numbered_object)? != 3 {
        return test_error("JavaFieldHandle TestSubject.small after set mismatch");
    }
    let letter_field = test_wrapper.field_handle("letter")?;
    if letter_field.get_char(&numbered_object)? != 'C' as jni::jchar {
        return test_error("JavaFieldHandle TestSubject.letter mismatch");
    }
    letter_field.set_char(&numbered_object, 'D' as jni::jchar)?;
    if letter_field.get_char(&numbered_object)? != 'D' as jni::jchar {
        return test_error("JavaFieldHandle TestSubject.letter after set mismatch");
    }
    let short_field = test_wrapper.field_handle("shortNumber")?;
    if short_field.get_short(&numbered_object)? != 123 {
        return test_error("JavaFieldHandle TestSubject.shortNumber mismatch");
    }
    short_field.set_short(&numbered_object, 124)?;
    if short_field.get_short(&numbered_object)? != 124 {
        return test_error("JavaFieldHandle TestSubject.shortNumber after set mismatch");
    }
    let wide_field = test_wrapper.field_handle("wideNumber")?;
    if wide_field.get_long(&numbered_object)? != 1000 {
        return test_error("JavaFieldHandle TestSubject.wideNumber mismatch");
    }
    wide_field.set_long(&numbered_object, 1001)?;
    if wide_field.get_long(&numbered_object)? != 1001 {
        return test_error("JavaFieldHandle TestSubject.wideNumber after set mismatch");
    }
    let ratio_field = test_wrapper.field_handle("ratio")?;
    if (ratio_field.get_float(&numbered_object)? - 1.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.ratio mismatch");
    }
    ratio_field.set_float(&numbered_object, 2.5)?;
    if (ratio_field.get_float(&numbered_object)? - 2.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.ratio after set mismatch");
    }
    let precise_field = test_wrapper.field_handle("precise")?;
    if (precise_field.get_double(&numbered_object)? - 2.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.precise mismatch");
    }
    precise_field.set_double(&numbered_object, 3.5)?;
    if (precise_field.get_double(&numbered_object)? - 3.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.precise after set mismatch");
    }

    let static_flag_field = test_wrapper.static_field_handle("staticFlag")?;
    if !static_flag_field.get_static_boolean()? {
        return test_error("JavaFieldHandle TestSubject.staticFlag mismatch");
    }
    static_flag_field.set_static_boolean(false)?;
    if static_flag_field.get_static_boolean()? {
        return test_error("JavaFieldHandle TestSubject.staticFlag after set mismatch");
    }
    let static_small_field = test_wrapper.static_field_handle("staticSmall")?;
    if static_small_field.get_static_byte()? != 2 {
        return test_error("JavaFieldHandle TestSubject.staticSmall mismatch");
    }
    static_small_field.set_static_byte(3)?;
    if static_small_field.get_static_byte()? != 3 {
        return test_error("JavaFieldHandle TestSubject.staticSmall after set mismatch");
    }
    let static_letter_field = test_wrapper.static_field_handle("staticLetter")?;
    if static_letter_field.get_static_char()? != 'C' as jni::jchar {
        return test_error("JavaFieldHandle TestSubject.staticLetter mismatch");
    }
    static_letter_field.set_static_char('D' as jni::jchar)?;
    if static_letter_field.get_static_char()? != 'D' as jni::jchar {
        return test_error("JavaFieldHandle TestSubject.staticLetter after set mismatch");
    }
    let static_short_field = test_wrapper.static_field_handle("staticShortNumber")?;
    if static_short_field.get_static_short()? != 123 {
        return test_error("JavaFieldHandle TestSubject.staticShortNumber mismatch");
    }
    static_short_field.set_static_short(124)?;
    if static_short_field.get_static_short()? != 124 {
        return test_error("JavaFieldHandle TestSubject.staticShortNumber after set mismatch");
    }
    let static_wide_field = test_wrapper.static_field_handle("staticWideNumber")?;
    if static_wide_field.get_static_long()? != 1000 {
        return test_error("JavaFieldHandle TestSubject.staticWideNumber mismatch");
    }
    static_wide_field.set_static_long(1001)?;
    if static_wide_field.get_static_long()? != 1001 {
        return test_error("JavaFieldHandle TestSubject.staticWideNumber after set mismatch");
    }
    let static_ratio_field = test_wrapper.static_field_handle("staticRatio")?;
    if (static_ratio_field.get_static_float()? - 1.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.staticRatio mismatch");
    }
    static_ratio_field.set_static_float(2.5)?;
    if (static_ratio_field.get_static_float()? - 2.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.staticRatio after set mismatch");
    }
    let static_precise_field = test_wrapper.static_field_handle("staticPrecise")?;
    if (static_precise_field.get_static_double()? - 2.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.staticPrecise mismatch");
    }
    static_precise_field.set_static_double(3.5)?;
    if (static_precise_field.get_static_double()? - 3.5).abs() > 0.0001 {
        return test_error("JavaFieldHandle TestSubject.staticPrecise after set mismatch");
    }

    println!("app_process_test: checking heap instance enumeration capability");
    let heap_subject_a = int_constructor.new_object((8101 as jni::jint,))?;
    let heap_subject_b = int_constructor.new_object((8102 as jni::jint,))?;
    if heap_enumeration_available {
        let mut numbers = Vec::new();
        app_java.choose_instances(TEST_SUBJECT, |object| {
            numbers.push(number_field.get_int(object)?);
            Ok(JavaChooseControl::Continue)
        })?;
        if !numbers.contains(&8101) || !numbers.contains(&8102) {
            return test_error(format!(
                "heap enumeration did not include both retained TestSubject instances: {numbers:?}"
            ));
        }

        let mut stop_count = 0;
        test_wrapper.choose_instances(|_object| {
            stop_count += 1;
            Ok(JavaChooseControl::Stop)
        })?;
        if stop_count != 1 {
            return test_error(format!(
                "heap enumeration stop callback count mismatch: {stop_count}"
            ));
        }
    } else {
        match app_java.choose_instances(TEST_SUBJECT, |_object| Ok(JavaChooseControl::Continue)) {
            Err(Error::UnsupportedFeature {
                feature: "ART heap enumeration",
                ..
            }) => {}
            Err(error) => return Err(error),
            Ok(()) => {
                return test_error("heap enumeration succeeded despite unsupported capability");
            }
        }
    }
    let _ = (heap_subject_a, heap_subject_b);

    let answer_overload = test_wrapper.static_method_overload("answer", &[])?;
    let answer = answer_overload.call_static_int(())?;
    if answer != 42 {
        return test_error(format!(
            "JavaMethodOverload TestSubject.answer mismatch: {answer}"
        ));
    }
    let message_overload = test_wrapper.method_overload("message", &[])?;
    let message = message_overload
        .call_string(&test_object, ())?
        .ok_or_else(|| test_failure("JavaMethodOverload TestSubject.message unexpectedly null"))?;
    if message != "dex-test" {
        return test_error(format!(
            "JavaMethodOverload TestSubject.message mismatch: {message:?}"
        ));
    }
    let answer_handle = test_wrapper.static_method("answer")?;
    if answer_handle.kind() != MethodKind::Static
        || answer_handle.name() != "answer"
        || answer_handle.overloads().len() != 1
    {
        return test_error("JavaMethodHandle TestSubject.answer metadata mismatch");
    }
    let answer = answer_handle.call_static_int(())?;
    if answer != 42 {
        return test_error(format!(
            "JavaMethodHandle TestSubject.answer mismatch: {answer}"
        ));
    }
    let message_handle = test_wrapper.method("message")?;
    if message_handle.kind() != MethodKind::Instance
        || message_handle.name() != "message"
        || message_handle.overloads().len() != 1
    {
        return test_error("JavaMethodHandle TestSubject.message metadata mismatch");
    }
    let message = message_handle
        .call_string(&test_object, ())?
        .ok_or_else(|| test_failure("JavaMethodHandle TestSubject.message unexpectedly null"))?;
    if message != "dex-test" {
        return test_error(format!(
            "JavaMethodHandle TestSubject.message mismatch: {message:?}"
        ));
    }
    let overload_handle = test_wrapper.method("overload")?;
    match overload_handle.call_string(&test_object, ("typed",)) {
        Err(Error::AmbiguousMethod {
            class,
            kind: "instance",
            name,
            candidates,
        }) if class == TEST_SUBJECT
            && name == "overload"
            && candidates
                .iter()
                .any(|candidate| candidate == "()Ljava/lang/String;")
            && candidates
                .iter()
                .any(|candidate| candidate == "(Ljava/lang/String;)Ljava/lang/String;") => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "ambiguous JavaMethodHandle TestSubject.overload unexpectedly resolved: {value:?}"
            ));
        }
    }
    let overload_string_from_handle = overload_handle.overload(["java.lang.String"])?;
    let value = overload_string_from_handle
        .call_string(&test_object, ("typed",))?
        .ok_or_else(|| {
            test_failure("JavaMethodHandle TestSubject.overload(String) unexpectedly null")
        })?;
    if value != "typed" {
        return test_error(format!(
            "JavaMethodHandle TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let overload_string = test_wrapper.overload("overload", ["java.lang.String"])?;
    let value = overload_string
        .call_string(&test_object, ["typed"])?
        .ok_or_else(|| test_failure("JavaMethodOverload TestSubject.overload(String) null"))?;
    if value != "typed" {
        return test_error(format!(
            "JavaMethodOverload TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let input = app_java.new_string_utf("typed-object")?;
    let value = overload_string
        .call_string(&test_object, (&input,))?
        .ok_or_else(|| {
            test_failure("JavaMethodOverload TestSubject.overload(String object arg) null")
        })?;
    if value != "typed-object" {
        return test_error(format!(
            "JavaMethodOverload TestSubject.overload(String object arg) mismatch: {value:?}"
        ));
    }
    let static_echo = test_wrapper.static_overload("staticEcho", ["java.lang.String"])?;
    let value = static_echo
        .call_static_string(["typed-static"])?
        .ok_or_else(|| test_failure("JavaMethodOverload TestSubject.staticEcho(String) null"))?;
    if value != "typed-static" {
        return test_error(format!(
            "JavaMethodOverload TestSubject.staticEcho(String) mismatch: {value:?}"
        ));
    }
    let static_object_echo =
        test_wrapper.static_overload("staticObjectEcho", ["java.lang.Object"])?;
    let value = static_object_echo
        .call_static_string(["typed-object-param"])?
        .ok_or_else(|| {
            test_failure("JavaMethodOverload TestSubject.staticObjectEcho(Object) null")
        })?;
    if value != "typed-object-param" {
        return test_error(format!(
            "JavaMethodOverload TestSubject.staticObjectEcho(Object) mismatch: {value:?}"
        ));
    }
    let instance_add = test_wrapper.overload("instanceAdd", ["int", "int"])?;
    match instance_add.call_int(&test_object, ["typed", "wrong"]) {
        Err(Error::InvalidArgumentType {
            index: 0,
            expected,
            actual: "string",
        }) if expected == "I" => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "JavaMethodOverload TestSubject.instanceAdd unexpectedly accepted string args: {value}"
            ));
        }
    }

    println!("app_process_test: checking JavaArray ergonomics");
    let object_class = app_java.find_class("java.lang.Object")?;
    let object_array = app_java.new_object_array(&object_class, &[Some(&test_object), None])?;
    if object_array.len()? != 2 {
        return test_error("JavaArray object length mismatch");
    }
    if object_array.element_type() != &JavaType::Object("java/lang/Object".to_owned()) {
        return test_error(format!(
            "JavaArray object element type mismatch: {}",
            object_array.element_type()
        ));
    }
    let first = object_array
        .get_object(0)?
        .ok_or_else(|| test_failure("JavaArray object first element unexpectedly null"))?;
    let env = app_java.vm().attach_current_thread()?;
    if !env.is_same_object(&first, &test_object)? {
        return test_error("JavaArray object first element mismatch");
    }
    if object_array.get_object(1)?.is_some() {
        return test_error("JavaArray object null element unexpectedly present");
    }
    object_array.set_object(1, Some(&test_object))?;
    if object_array.get_object(1)?.is_none() {
        return test_error("JavaArray object set did not store the element");
    }

    let echoed = subject.call_static(
        "staticObjectArrayEcho",
        "([Ljava/lang/Object;)[Ljava/lang/Object;",
        &[JavaValue::from(&object_array)],
    )?;
    expect_object_same(
        &env,
        echoed,
        Some(object_array.as_jobject()),
        "TestSubject.staticObjectArrayEcho",
    )?;
    let object_array_overload = test_wrapper
        .static_method_overload_by_name("staticObjectArrayEcho", &["java.lang.Object[]"])?;
    let echoed = object_array_overload
        .call_static_array((&object_array,))?
        .ok_or_else(|| test_failure("JavaMethodOverload staticObjectArrayEcho null"))?;
    if !env.is_same_object(&echoed, &object_array)? {
        return test_error("JavaMethodOverload staticObjectArrayEcho mismatch");
    }

    let ints = app_java.new_int_array(&[1, 2, 3])?;
    if ints.element_type() != &JavaType::Int || ints.get_ints()? != [1, 2, 3] {
        return test_error(format!(
            "JavaArray int values mismatch: {:?}",
            ints.get_ints()?
        ));
    }
    ints.set_ints(&[4, 5, 6])?;
    if ints.get_ints()? != [4, 5, 6] {
        return test_error(format!(
            "JavaArray int values after set mismatch: {:?}",
            ints.get_ints()?
        ));
    }
    let echoed = subject.call_static("staticIntArrayEcho", "([I)[I", &[JavaValue::from(&ints)])?;
    let echoed = echoed
        .into_array("TestSubject.staticIntArrayEcho")?
        .ok_or_else(|| test_failure("TestSubject.staticIntArrayEcho unexpectedly null"))?;
    if echoed.get_ints()? != [4, 5, 6] {
        return test_error(format!(
            "TestSubject.staticIntArrayEcho values mismatch: {:?}",
            echoed.get_ints()?
        ));
    }
    let sum = read_int(
        subject.call_method(
            &test_object,
            "sumIntArray",
            "([I)I",
            &[JavaValue::from(&ints)],
        )?,
        "TestSubject.sumIntArray",
    )?;
    if sum != 15 {
        return test_error(format!("TestSubject.sumIntArray mismatch: {sum}"));
    }
    let int_array_overload = test_wrapper.method_overload_by_name("intArrayEcho", &["int[]"])?;
    let echoed = int_array_overload
        .call_array(&test_object, (&ints,))?
        .ok_or_else(|| test_failure("JavaMethodOverload intArrayEcho null"))?;
    if echoed.get_ints()? != [4, 5, 6] {
        return test_error(format!(
            "JavaMethodOverload intArrayEcho values mismatch: {:?}",
            echoed.get_ints()?
        ));
    }

    let booleans = app_java.new_boolean_array(&[true, false, true])?;
    if booleans.get_booleans()? != [true, false, true] {
        return test_error(format!(
            "JavaArray boolean values mismatch: {:?}",
            booleans.get_booleans()?
        ));
    }
    booleans.set_booleans(&[false, true])?;
    if booleans.get_booleans()? != [false, true, true] {
        return test_error(format!(
            "JavaArray boolean values after set mismatch: {:?}",
            booleans.get_booleans()?
        ));
    }
    let echoed = test_wrapper
        .static_method_overload_by_name("staticBooleanArrayEcho", &["boolean[]"])?
        .call_static_array((&booleans,))?
        .ok_or_else(|| test_failure("JavaMethodOverload staticBooleanArrayEcho null"))?;
    if echoed.get_booleans()? != [false, true, true] {
        return test_error(format!(
            "JavaMethodOverload staticBooleanArrayEcho mismatch: {:?}",
            echoed.get_booleans()?
        ));
    }

    if app_java.new_byte_array(&[-1, 2])?.get_bytes()? != [-1, 2] {
        return test_error("JavaArray byte round-trip mismatch");
    }
    if app_java.new_char_array(&[65, 66])?.get_chars()? != [65, 66] {
        return test_error("JavaArray char round-trip mismatch");
    }
    if app_java.new_short_array(&[-3, 4])?.get_shorts()? != [-3, 4] {
        return test_error("JavaArray short round-trip mismatch");
    }
    if app_java.new_long_array(&[5, 6])?.get_longs()? != [5, 6] {
        return test_error("JavaArray long round-trip mismatch");
    }
    if app_java.new_float_array(&[1.25, 2.5])?.get_floats()? != [1.25, 2.5] {
        return test_error("JavaArray float round-trip mismatch");
    }
    if app_java.new_double_array(&[3.5, 4.75])?.get_doubles()? != [3.5, 4.75] {
        return test_error("JavaArray double round-trip mismatch");
    }
    Ok(())
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
            JavaValue::Null,
            JavaValue::Object(system_loader.as_jobject()),
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
    let wrapper_answer = read_int(
        wrapper_subject.call_static("answer", "()I", ())?,
        "DexTestSubject wrapper answer",
    )?;
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
                java.enumerate_methods("frida.java.bridge.rs.test.TestSubject!overload*/s")?;
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
