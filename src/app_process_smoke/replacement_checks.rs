use super::assertions::*;
use super::replacement_callbacks::*;
use super::replacement_lifecycle::run_replacement_lifecycle_checks;
use super::*;

pub(super) fn run_replacement_checks(java: &Java, app_java: &Java) -> Result<()> {
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
    expect_replacement_clone_backend(&replacement, "static replacement")?;
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
        wrapper.call_static("answer", "()I", [])?,
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
            [JavaValue::from(&input)],
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

    let input = java.new_string_utf("app-process-static-original-argument")?;
    let output = java.new_string_utf("app-process-static-original-call")?;
    EXPECTED_ARGUMENT.store(input.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_STRING.store(output.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_string_to_string_method(
            &subject,
            "staticEcho",
            replacement_static_echo_calling_original,
        )?
    };
    expect_string(
        subject.call_static(
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-static-original-call"),
        "staticEcho replacement calling original",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    let null_output = java.new_string_utf("app-process-static-original-null")?;
    REPLACEMENT_STRING.store(null_output.as_jobject(), Ordering::SeqCst);
    expect_string(
        subject.call_static(
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::Null],
        )?,
        Some("app-process-static-original-null"),
        "staticEcho null replacement calling original",
    )?;
    replacement.revert()?;
    expect_string(
        subject.call_static(
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-static-original-argument"),
        "staticEcho original-call replacement restored",
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
        experimental::replace_static_native_method(
            &subject,
            "staticAdd",
            "(II)I",
            replacement_static_add as *const () as *mut std::ffi::c_void,
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

    let replacement = unsafe {
        experimental::replace_static_i32_i32_to_i32_method(
            &subject,
            "staticAdd",
            replacement_static_add_calling_original,
        )?
    };
    expect_int(
        subject.call_static(
            "staticAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        1007,
        "staticAdd replacement calling original",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static(
            "staticAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        7,
        "staticAdd restored after original-call replacement",
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

    let object = subject.new_object("(I)V", &[JavaValue::Int(31)])?;
    let second_object = subject.new_object("(I)V", &[JavaValue::Int(32)])?;
    let compare_env = java.vm().attach_current_thread()?;
    let object_echo_signature = "(Ljava/lang/Object;)Ljava/lang/Object;";
    let object_array_echo_signature = "([Ljava/lang/Object;)[Ljava/lang/Object;";
    let subject_echo_signature =
        "(Lfrida/java/bridge/rs/smoke/SmokeSubject;)Lfrida/java/bridge/rs/smoke/SmokeSubject;";
    let object_class = java.find_class("java.lang.Object")?;
    let object_array =
        java.new_object_array(&object_class, &[Some(&object), Some(&second_object)])?;
    let second_object_array = java.new_object_array(&object_class, &[Some(&second_object)])?;

    println!("app_process_smoke: checking overload facade replacements");
    let answer_overload = wrapper.static_method_overload("facadeAnswer", &[])?;
    let replacement = unsafe {
        experimental::replace_method(
            &answer_overload,
            experimental::MethodImplementation::StaticI32(replacement_answer),
        )?
    };
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        1337,
        "facadeAnswer replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        314,
        "facadeAnswer restored",
    )?;

    let original_answer = experimental::OriginalMethod::new(&answer_overload)?;
    let _ = FACADE_STATIC_ANSWER_ORIGINAL.set(original_answer);
    let replacement = unsafe {
        experimental::replace_method(
            &answer_overload,
            experimental::MethodImplementation::StaticI32(
                replacement_facade_answer_calling_original,
            ),
        )?
    };
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        2314,
        "facadeAnswer replacement calling original",
    )?;
    replacement.revert()?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    let instance_number_overload = wrapper.method_overload("facadeInstanceNumber", &[])?;
    let replacement = unsafe {
        experimental::replace_method(
            &instance_number_overload,
            experimental::MethodImplementation::InstanceI32(replacement_instance_number),
        )?
    };
    expect_int(
        instance_number_overload.call(&object, [])?,
        2026,
        "facadeInstanceNumber replacement",
    )?;
    expect_int(
        instance_number_overload.call(&second_object, [])?,
        -2,
        "facade second receiver facadeInstanceNumber replacement",
    )?;
    replacement.revert()?;

    let facade_output = java.new_string_utf("facade-replacement")?;
    REPLACEMENT_STRING.store(facade_output.as_jobject(), Ordering::SeqCst);
    let overload_string =
        wrapper.method_overload_by_name("facadeOverload", &["java.lang.String"])?;
    let facade_input = java.new_string_utf("facade-input")?;
    EXPECTED_ARGUMENT.store(facade_input.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_method(
            &overload_string,
            experimental::MethodImplementation::InstanceStringToString(replacement_overload),
        )?
    };
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        Some("facade-replacement"),
        "facade overload(String) replacement",
    )?;
    replacement.revert()?;

    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);
    let static_object_echo =
        wrapper.static_method_overload_by_name("facadeStaticObjectEcho", &["java.lang.Object"])?;
    let replacement = unsafe {
        experimental::replace_method(
            &static_object_echo,
            experimental::MethodImplementation::StaticReferenceToReference(
                replacement_static_object_echo,
            ),
        )?
    };
    expect_object_same(
        &compare_env,
        static_object_echo.call_static([JavaValue::from(&object)])?,
        Some(second_object.as_jobject()),
        "facade staticObjectEcho replacement",
    )?;
    replacement.revert()?;

    EXPECTED_ARGUMENT.store(object_array.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object_array.as_jobject(), Ordering::SeqCst);
    let static_object_array_echo = wrapper
        .static_method_overload_by_name("facadeStaticObjectArrayEcho", &["java.lang.Object[]"])?;
    let replacement = unsafe {
        experimental::replace_method(
            &static_object_array_echo,
            experimental::MethodImplementation::StaticReferenceToReference(
                replacement_static_object_array_echo,
            ),
        )?
    };
    expect_object_same(
        &compare_env,
        static_object_array_echo.call_static([JavaValue::from(&object_array)])?,
        Some(second_object_array.as_jobject()),
        "facade staticObjectArrayEcho replacement",
    )?;
    replacement.revert()?;

    run_replacement_lifecycle_checks(java, &subject, &wrapper, &object)?;

    println!("app_process_smoke: checking app-loader static object replacements");
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(object.as_jobject()),
        "staticObjectEcho original",
    )?;
    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_reference_to_reference_method(
            &subject,
            "staticObjectEcho",
            object_echo_signature,
            replacement_static_object_echo,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(second_object.as_jobject()),
        "staticObjectEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        cached_subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(second_object.as_jobject()),
        "cached staticObjectEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        wrapper.call_static(
            "staticObjectEcho",
            object_echo_signature,
            [JavaValue::from(&object)],
        )?,
        Some(second_object.as_jobject()),
        "wrapper staticObjectEcho replacement",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "staticObjectEcho null replacement",
    )?;
    replacement.revert()?;
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(object.as_jobject()),
        "staticObjectEcho restored",
    )?;

    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_reference_to_reference_method(
            &subject,
            "staticObjectEcho",
            object_echo_signature,
            replacement_static_object_echo_calling_original,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(second_object.as_jobject()),
        "staticObjectEcho replacement calling original",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectEcho",
            object_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "staticObjectEcho null replacement calling original",
    )?;
    replacement.revert()?;

    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(object_array.as_jobject()),
        "staticObjectArrayEcho original",
    )?;
    EXPECTED_ARGUMENT.store(object_array.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object_array.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_reference_to_reference_method(
            &subject,
            "staticObjectArrayEcho",
            object_array_echo_signature,
            replacement_static_object_array_echo,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "staticObjectArrayEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        cached_subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "cached staticObjectArrayEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        wrapper.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            [JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "wrapper staticObjectArrayEcho replacement",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "staticObjectArrayEcho null replacement",
    )?;
    replacement.revert()?;
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(object_array.as_jobject()),
        "staticObjectArrayEcho restored",
    )?;

    EXPECTED_ARGUMENT.store(object_array.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object_array.as_jobject(), Ordering::SeqCst);
    let static_object_array_echo_original =
        wrapper.static_method_overload_by_name("staticObjectArrayEcho", &["java.lang.Object[]"])?;
    let _ = STATIC_OBJECT_ARRAY_ECHO_ORIGINAL.set(experimental::OriginalMethod::new(
        &static_object_array_echo_original,
    )?);
    let replacement = unsafe {
        experimental::replace_static_reference_to_reference_method(
            &subject,
            "staticObjectArrayEcho",
            object_array_echo_signature,
            replacement_static_object_array_echo_calling_original,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "staticObjectArrayEcho replacement calling original",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticObjectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "staticObjectArrayEcho null replacement calling original",
    )?;
    replacement.revert()?;

    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_static_reference_to_reference_method(
            &subject,
            "staticSubjectEcho",
            subject_echo_signature,
            replacement_static_object_echo,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticSubjectEcho",
            subject_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(second_object.as_jobject()),
        "staticSubjectEcho replacement",
    )?;
    replacement.revert()?;
    expect_object_same(
        &compare_env,
        subject.call_static(
            "staticSubjectEcho",
            subject_echo_signature,
            &[JavaValue::from(&object)],
        )?,
        Some(object.as_jobject()),
        "staticSubjectEcho restored",
    )?;

    println!("app_process_smoke: checking app-loader instance object replacements");
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(second_object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(object.as_jobject(), Ordering::SeqCst);
    let object_echo_overload =
        wrapper.method_overload_by_name("objectEcho", &["java.lang.Object"])?;
    let replacement = unsafe {
        experimental::replace_native_method(
            &object_echo_overload,
            experimental::NativeMethodImplementation::instance_method(
                object_echo_signature,
                replacement_instance_object_echo as *const () as *mut std::ffi::c_void,
            )?,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectEcho",
            object_echo_signature,
            &[JavaValue::from(&second_object)],
        )?,
        Some(object.as_jobject()),
        "objectEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        cached_subject.call_method(
            &object,
            "objectEcho",
            object_echo_signature,
            &[JavaValue::from(&second_object)],
        )?,
        Some(object.as_jobject()),
        "cached objectEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        wrapper.call(
            &object,
            "objectEcho",
            object_echo_signature,
            [JavaValue::from(&second_object)],
        )?,
        Some(object.as_jobject()),
        "wrapper objectEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        subject.call_method(
            &second_object,
            "objectEcho",
            object_echo_signature,
            &[JavaValue::from(&second_object)],
        )?,
        None,
        "second receiver objectEcho replacement",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectEcho",
            object_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "objectEcho null replacement",
    )?;
    replacement.revert()?;
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectEcho",
            object_echo_signature,
            &[JavaValue::from(&second_object)],
        )?,
        Some(second_object.as_jobject()),
        "objectEcho restored",
    )?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(second_object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(object.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_instance_reference_to_reference_method(
            &subject,
            "subjectEcho",
            subject_echo_signature,
            replacement_instance_subject_echo_calling_original,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "subjectEcho",
            subject_echo_signature,
            &[JavaValue::from(&second_object)],
        )?,
        Some(object.as_jobject()),
        "subjectEcho replacement calling original",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "subjectEcho",
            subject_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "subjectEcho null replacement calling original",
    )?;
    replacement.revert()?;
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "subjectEcho",
            subject_echo_signature,
            &[JavaValue::from(&second_object)],
        )?,
        Some(second_object.as_jobject()),
        "subjectEcho restored after original-call replacement",
    )?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(object_array.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object_array.as_jobject(), Ordering::SeqCst);
    let instance_object_array_echo_original =
        wrapper.method_overload_by_name("objectArrayEcho", &["java.lang.Object[]"])?;
    let _ = INSTANCE_OBJECT_ARRAY_ECHO_ORIGINAL.set(experimental::OriginalMethod::new(
        &instance_object_array_echo_original,
    )?);
    let replacement = unsafe {
        experimental::replace_instance_reference_to_reference_method(
            &subject,
            "objectArrayEcho",
            object_array_echo_signature,
            replacement_instance_object_array_echo,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "objectArrayEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        cached_subject.call_method(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "cached objectArrayEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        wrapper.call(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            [JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "wrapper objectArrayEcho replacement",
    )?;
    expect_object_same(
        &compare_env,
        subject.call_method(
            &second_object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        None,
        "second receiver objectArrayEcho replacement",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "objectArrayEcho null replacement",
    )?;
    replacement.revert()?;
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(object_array.as_jobject()),
        "objectArrayEcho restored",
    )?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(object_array.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object_array.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_instance_reference_to_reference_method(
            &subject,
            "objectArrayEcho",
            object_array_echo_signature,
            replacement_instance_object_array_echo_calling_original,
        )?
    };
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::from(&object_array)],
        )?,
        Some(second_object_array.as_jobject()),
        "objectArrayEcho replacement calling original",
    )?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    expect_object_same(
        &compare_env,
        subject.call_method(
            &object,
            "objectArrayEcho",
            object_array_echo_signature,
            &[JavaValue::Null],
        )?,
        None,
        "objectArrayEcho null replacement calling original",
    )?;
    replacement.revert()?;

    println!("app_process_smoke: checking app-loader overload isolation");
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
            [JavaValue::from(&input)],
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

    let input = java.new_string_utf("app-process-instance-original-argument")?;
    let output = java.new_string_utf("app-process-instance-original-call")?;
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(input.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_STRING.store(output.as_jobject(), Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_instance_string_to_string_method(
            &subject,
            "overload",
            replacement_overload_calling_original,
        )?
    };
    expect_string(
        subject.call_method(
            &object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-instance-original-call"),
        "overload(String) replacement calling original",
    )?;
    expect_string(
        subject.call_method(
            &second_object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        None,
        "second receiver overload(String) replacement calling original",
    )?;
    replacement.revert()?;
    expect_string(
        subject.call_method(
            &object,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JavaValue::from(&input)],
        )?,
        Some("app-process-instance-original-argument"),
        "overload(String) restored after original-call replacement",
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
    expect_replacement_clone_backend(&replacement, "instance replacement")?;
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

    println!("app_process_smoke: checking app-loader instance primitive replacements");
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    subject.call_method(&object, "bumpInstanceVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_method(&object, "instanceVoidCounter", "()I", &[])?,
        1,
        "bumpInstanceVoidCounter original",
    )?;
    VOID_REPLACEMENT_COUNTER.store(0, Ordering::SeqCst);
    let replacement = unsafe {
        experimental::replace_instance_void_method(
            &subject,
            "bumpInstanceVoidCounter",
            replacement_instance_void,
        )?
    };
    subject.call_method(&object, "bumpInstanceVoidCounter", "()V", &[])?;
    subject.call_method(&object, "bumpInstanceVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_method(&object, "instanceVoidCounter", "()I", &[])?,
        1,
        "bumpInstanceVoidCounter Java state during replacement",
    )?;
    if VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst) != 2 {
        return replacement_counter_mismatch(
            "bumpInstanceVoidCounter replacement counter",
            2,
            VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst),
        );
    }
    replacement.revert()?;
    subject.call_method(&object, "bumpInstanceVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_method(&object, "instanceVoidCounter", "()I", &[])?,
        2,
        "bumpInstanceVoidCounter restored",
    )?;

    expect_bool(
        subject.call_method(&object, "instanceBoolean", "()Z", &[])?,
        true,
        "instanceBoolean original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_boolean_method(
            &subject,
            "instanceBoolean",
            replacement_instance_boolean,
        )?
    };
    expect_bool(
        subject.call_method(&object, "instanceBoolean", "()Z", &[])?,
        false,
        "instanceBoolean replacement",
    )?;
    expect_bool(
        subject.call_method(&second_object, "instanceBoolean", "()Z", &[])?,
        true,
        "second receiver instanceBoolean replacement",
    )?;
    replacement.revert()?;
    expect_bool(
        subject.call_method(&object, "instanceBoolean", "()Z", &[])?,
        true,
        "instanceBoolean restored",
    )?;

    expect_byte(
        subject.call_method(&object, "instanceByte", "()B", &[])?,
        7,
        "instanceByte original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_byte_method(
            &subject,
            "instanceByte",
            replacement_instance_byte,
        )?
    };
    expect_byte(
        subject.call_method(&object, "instanceByte", "()B", &[])?,
        -8,
        "instanceByte replacement",
    )?;
    replacement.revert()?;
    expect_byte(
        subject.call_method(&object, "instanceByte", "()B", &[])?,
        7,
        "instanceByte restored",
    )?;

    expect_char(
        subject.call_method(&object, "instanceChar", "()C", &[])?,
        b'A' as jni::jchar,
        "instanceChar original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_char_method(
            &subject,
            "instanceChar",
            replacement_instance_char,
        )?
    };
    expect_char(
        subject.call_method(&object, "instanceChar", "()C", &[])?,
        b'Z' as jni::jchar,
        "instanceChar replacement",
    )?;
    replacement.revert()?;
    expect_char(
        subject.call_method(&object, "instanceChar", "()C", &[])?,
        b'A' as jni::jchar,
        "instanceChar restored",
    )?;

    expect_short(
        subject.call_method(&object, "instanceShort", "()S", &[])?,
        1234,
        "instanceShort original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_short_method(
            &subject,
            "instanceShort",
            replacement_instance_short,
        )?
    };
    expect_short(
        subject.call_method(&object, "instanceShort", "()S", &[])?,
        -1234,
        "instanceShort replacement",
    )?;
    replacement.revert()?;
    expect_short(
        subject.call_method(&object, "instanceShort", "()S", &[])?,
        1234,
        "instanceShort restored",
    )?;

    expect_long(
        subject.call_method(&object, "instanceLong", "()J", &[])?,
        1234567890154,
        "instanceLong original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_i64_method(
            &subject,
            "instanceLong",
            replacement_instance_long,
        )?
    };
    expect_long(
        subject.call_method(&object, "instanceLong", "()J", &[])?,
        -9876543210,
        "instanceLong replacement",
    )?;
    replacement.revert()?;
    expect_long(
        subject.call_method(&object, "instanceLong", "()J", &[])?,
        1234567890154,
        "instanceLong restored",
    )?;

    expect_float(
        subject.call_method(&object, "instanceFloat", "()F", &[])?,
        31.25,
        "instanceFloat original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_f32_method(
            &subject,
            "instanceFloat",
            replacement_instance_float,
        )?
    };
    expect_float(
        subject.call_method(&object, "instanceFloat", "()F", &[])?,
        -2.5,
        "instanceFloat replacement",
    )?;
    replacement.revert()?;
    expect_float(
        subject.call_method(&object, "instanceFloat", "()F", &[])?,
        31.25,
        "instanceFloat restored",
    )?;

    expect_double(
        subject.call_method(&object, "instanceDouble", "()D", &[])?,
        31.5,
        "instanceDouble original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_f64_method(
            &subject,
            "instanceDouble",
            replacement_instance_double,
        )?
    };
    expect_double(
        subject.call_method(&object, "instanceDouble", "()D", &[])?,
        -6.25,
        "instanceDouble replacement",
    )?;
    replacement.revert()?;
    expect_double(
        subject.call_method(&object, "instanceDouble", "()D", &[])?,
        31.5,
        "instanceDouble restored",
    )?;

    expect_int(
        subject.call_method(
            &object,
            "instanceAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        38,
        "instanceAdd original",
    )?;
    let instance_add_original = wrapper.method_overload_by_name("instanceAdd", &["int", "int"])?;
    let _ = INSTANCE_ADD_ORIGINAL.set(experimental::OriginalMethod::new(&instance_add_original)?);
    let replacement = unsafe {
        experimental::replace_instance_i32_i32_to_i32_method(
            &subject,
            "instanceAdd",
            replacement_instance_add,
        )?
    };
    expect_int(
        subject.call_method(
            &object,
            "instanceAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        52,
        "instanceAdd replacement",
    )?;
    expect_int(
        cached_subject.call_method(
            &object,
            "instanceAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        52,
        "cached instanceAdd replacement",
    )?;
    expect_int(
        wrapper.call(
            &object,
            "instanceAdd",
            "(II)I",
            [JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        52,
        "wrapper instanceAdd replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(
            &object,
            "instanceAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        38,
        "instanceAdd restored",
    )?;

    let replacement = unsafe {
        experimental::replace_instance_i32_i32_to_i32_method(
            &subject,
            "instanceAdd",
            replacement_instance_add_calling_original,
        )?
    };
    expect_int(
        subject.call_method(
            &object,
            "instanceAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        1038,
        "instanceAdd replacement calling original",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(
            &object,
            "instanceAdd",
            "(II)I",
            &[JavaValue::Int(2), JavaValue::Int(5)],
        )?,
        38,
        "instanceAdd restored after original-call replacement",
    )?;

    expect_int(
        subject.call_method(
            &object,
            "instancePrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        105,
        "instancePrimitiveMix original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_z_b_c_s_to_i32_method(
            &subject,
            "instancePrimitiveMix",
            replacement_instance_primitive_mix,
        )?
    };
    expect_int(
        subject.call_method(
            &object,
            "instancePrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        4242,
        "instancePrimitiveMix replacement",
    )?;
    replacement.revert()?;
    expect_int(
        subject.call_method(
            &object,
            "instancePrimitiveMix",
            "(ZBCS)I",
            &[
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        105,
        "instancePrimitiveMix restored",
    )?;

    expect_long(
        subject.call_method(
            &object,
            "instanceWide",
            "(JD)J",
            &[JavaValue::Long(40), JavaValue::Double(2.0)],
        )?,
        73,
        "instanceWide original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_i64_f64_to_i64_method(
            &subject,
            "instanceWide",
            replacement_instance_wide,
        )?
    };
    expect_long(
        subject.call_method(
            &object,
            "instanceWide",
            "(JD)J",
            &[JavaValue::Long(40), JavaValue::Double(2.0)],
        )?,
        9001,
        "instanceWide replacement",
    )?;
    replacement.revert()?;
    expect_long(
        subject.call_method(
            &object,
            "instanceWide",
            "(JD)J",
            &[JavaValue::Long(40), JavaValue::Double(2.0)],
        )?,
        73,
        "instanceWide restored",
    )?;

    expect_double(
        subject.call_method(
            &object,
            "instanceFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?,
        34.75,
        "instanceFloatMix original",
    )?;
    let replacement = unsafe {
        experimental::replace_instance_f32_f64_to_f64_method(
            &subject,
            "instanceFloatMix",
            replacement_instance_float_mix,
        )?
    };
    expect_double(
        subject.call_method(
            &object,
            "instanceFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?,
        8.5,
        "instanceFloatMix replacement",
    )?;
    replacement.revert()?;
    expect_double(
        subject.call_method(
            &object,
            "instanceFloatMix",
            "(FD)D",
            &[JavaValue::Float(1.5), JavaValue::Double(2.25)],
        )?,
        34.75,
        "instanceFloatMix restored",
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
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_RECEIVER.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}
