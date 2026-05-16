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
        string_wrapper.call(&string, "length", "()I", [])?,
        "JavaClassWrapper String.length",
    )?;
    if length != "wrapper".len() as i32 {
        return smoke_error(format!("JavaClassWrapper String.length mismatch: {length}"));
    }

    let math_wrapper = java.use_class("java.lang.Math")?;
    let abs_value = read_int(
        math_wrapper.call_static("abs", "(I)I", [JavaValue::Int(-7)])?,
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
        smoke_wrapper.call_static("answer", "()I", ())?,
        "JavaClassWrapper SmokeSubject.answer",
    )?;
    if answer != 42 {
        return smoke_error(format!(
            "JavaClassWrapper SmokeSubject.answer mismatch: {answer}"
        ));
    }
    let smoke_object = smoke_wrapper.new_object("()V", ())?;
    let message = read_object(
        smoke_wrapper.call(&smoke_object, "message", "()Ljava/lang/String;", ())?,
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
        .call(&retained_object, "hashCode", "()I", ())?
        .into_int("JavaClassWrapper retained Object.hashCode")?;

    println!("app_process_smoke: checking app-loader overload handles");
    let default_constructor = smoke_wrapper.constructor_overload(&[])?;
    if default_constructor.signature().to_string() != "()V" {
        return smoke_error(format!(
            "JavaConstructorOverload default signature mismatch: {}",
            default_constructor.signature()
        ));
    }
    let smoke_object = default_constructor.new_object(())?;
    let int_constructor = smoke_wrapper.constructor_overload_by_name(&["int"])?;
    let numbered_object = int_constructor.new_object((31 as jni::jint,))?;
    let number_field = smoke_wrapper.field_handle("number")?;
    let number = number_field.get_int(&numbered_object)?;
    if number != 31 {
        return smoke_error(format!(
            "JavaFieldHandle SmokeSubject.number mismatch: {number}"
        ));
    }
    number_field.set_int(&numbered_object, 37)?;
    let number = number_field.get_int(&numbered_object)?;
    if number != 37 {
        return smoke_error(format!(
            "JavaFieldHandle SmokeSubject.number after set mismatch: {number}"
        ));
    }
    let answer_overload = smoke_wrapper.static_method_overload("answer", &[])?;
    let answer = answer_overload.call_static_int(())?;
    if answer != 42 {
        return smoke_error(format!(
            "JavaMethodOverload SmokeSubject.answer mismatch: {answer}"
        ));
    }
    let message_overload = smoke_wrapper.method_overload("message", &[])?;
    let message = message_overload
        .call_string(&smoke_object, ())?
        .ok_or_else(|| {
            smoke_failure("JavaMethodOverload SmokeSubject.message unexpectedly null")
        })?;
    if message != "dex-smoke" {
        return smoke_error(format!(
            "JavaMethodOverload SmokeSubject.message mismatch: {message:?}"
        ));
    }
    let overload_string =
        smoke_wrapper.method_overload_by_name("overload", &["java.lang.String"])?;
    let input = app_java.new_string_utf("typed")?;
    let value = overload_string
        .call_string(&smoke_object, (&input,))?
        .ok_or_else(|| smoke_failure("JavaMethodOverload SmokeSubject.overload(String) null"))?;
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
