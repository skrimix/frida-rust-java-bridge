use super::assertions::*;
use super::replacement_callbacks::*;
use super::replacement_lifecycle::run_replacement_lifecycle_checks;
use super::*;

pub(super) fn run_replacement_checks(java: &Java, app_java: &Java) -> Result<()> {
    let capabilities = java.capabilities();
    let Some(reason) = capabilities.method_replacement.experimental_reason() else {
        if let Some(reason) = capabilities.method_replacement.unsupported_reason() {
            println!("app_process_test: skipping replacement checks: {reason}");
            return Ok(());
        }
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: "method replacement capability unexpectedly reported stable supported"
                .to_owned(),
        });
    };
    if !reason.contains("prerequisites are available") {
        println!("app_process_test: skipping replacement checks: {reason}");
        return Ok(());
    }

    let subject = app_java.find_class(TEST_SUBJECT)?;
    let cached_subject = app_java.find_class(TEST_SUBJECT)?;
    let wrapper = app_java.use_class(TEST_SUBJECT)?;

    println!("app_process_test: checking app-loader static replacement");
    expect_int(
        subject.call_static("answer", "()I", &[])?,
        42,
        "answer original",
    )?;
    let mut replacement =
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

    println!("app_process_test: checking static original call from replacement");
    let mut replacement = unsafe {
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

    println!("app_process_test: checking app-loader primitive and argument replacements");
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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

    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
        "(Lfrida/java/bridge/rs/test/TestSubject;)Lfrida/java/bridge/rs/test/TestSubject;";
    let object_class = java.find_class("java.lang.Object")?;
    let object_array =
        java.new_object_array(&object_class, &[Some(&object), Some(&second_object)])?;
    let second_object_array = java.new_object_array(&object_class, &[Some(&second_object)])?;

    println!("app_process_test: checking overload facade replacements");
    let answer_overload = wrapper.static_method_overload("facadeAnswer", &[])?;
    let mut replacement = unsafe {
        answer_overload.replace(experimental::MethodImplementation::StaticI32(
            replacement_answer,
        ))?
    };
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        1337,
        "facadeAnswer replacement",
    )?;
    match unsafe {
        answer_overload.replace(experimental::MethodImplementation::StaticI32(
            replacement_answer,
        ))
    } {
        Err(error) => assert_eq!(
            error,
            Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "target ArtMethod already has an active replacement".to_owned(),
            }
        ),
        Ok(mut duplicate) => {
            duplicate.revert()?;
            return Err(Error::UnsupportedFeature {
                feature: "method replacement lifecycle",
                reason: "duplicate active static replacement was accepted".to_owned(),
            });
        }
    };
    replacement.revert()?;
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        314,
        "facadeAnswer restored",
    )?;

    let mut closure_replacement =
        unsafe { answer_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Int(4040)))? };
    let Some(summary) = closure_replacement.debug_summary() else {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "closure replacement debug summary was unavailable".to_owned(),
        });
    };
    expect_clone_backend_summary(&summary)?;
    expect_int(
        answer_overload.call_static([])?,
        4040,
        "facadeAnswer closure replacement",
    )?;
    closure_replacement.revert()?;
    expect_int(
        answer_overload.call_static([])?,
        314,
        "facadeAnswer restored after closure replacement",
    )?;

    let mut implementation = unsafe {
        answer_overload.implementation(|invocation| {
            if invocation.kind() != MethodKind::Static
                || invocation.name() != "facadeAnswer"
                || invocation.signature().to_string() != "()I"
                || invocation.class().is_none()
                || invocation.receiver().is_some()
                || !invocation.arguments().is_empty()
            {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: "facadeAnswer implementation received unexpected invocation shape"
                        .to_owned(),
                });
            }
            Ok(experimental::ImplementationReturn::Int(5050))
        })?
    };
    expect_int(
        answer_overload.call_static([])?,
        5050,
        "facadeAnswer implementation replacement",
    )?;
    implementation.revert()?;
    expect_int(
        answer_overload.call_static([])?,
        314,
        "facadeAnswer restored after implementation replacement",
    )?;

    let boolean_overload = wrapper.static_method_overload("staticBoolean", &[])?;
    let mut closure_replacement = unsafe {
        boolean_overload.replace_closure(|invocation| {
            if invocation.kind() != MethodKind::Static
                || invocation.name() != "staticBoolean"
                || invocation.signature().to_string() != "()Z"
                || invocation.class().is_none()
                || invocation.receiver().is_some()
                || !invocation.arguments().is_empty()
            {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "staticBoolean closure received unexpected invocation shape".to_owned(),
                });
            }
            Ok(experimental::RawJavaReturn::Boolean(jni::JNI_FALSE))
        })?
    };
    expect_bool(
        boolean_overload.call_static([])?,
        false,
        "staticBoolean closure replacement",
    )?;
    closure_replacement.revert()?;

    let byte_overload = wrapper.static_method_overload("staticByte", &[])?;
    let mut closure_replacement =
        unsafe { byte_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Byte(-12)))? };
    expect_byte(
        byte_overload.call_static([])?,
        -12,
        "staticByte closure replacement",
    )?;
    closure_replacement.revert()?;

    let char_overload = wrapper.static_method_overload("staticChar", &[])?;
    let mut closure_replacement =
        unsafe { char_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Char(90)))? };
    expect_char(
        char_overload.call_static([])?,
        90,
        "staticChar closure replacement",
    )?;
    closure_replacement.revert()?;

    let short_overload = wrapper.static_method_overload("staticShort", &[])?;
    let mut closure_replacement = unsafe {
        short_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Short(-321)))?
    };
    expect_short(
        short_overload.call_static([])?,
        -321,
        "staticShort closure replacement",
    )?;
    closure_replacement.revert()?;

    let long_overload = wrapper.static_method_overload("staticLong", &[])?;
    let mut closure_replacement = unsafe {
        long_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Long(9_876_543_210)))?
    };
    expect_long(
        long_overload.call_static([])?,
        9_876_543_210,
        "staticLong closure replacement",
    )?;
    closure_replacement.revert()?;

    let float_overload = wrapper.static_method_overload("staticFloat", &[])?;
    let mut closure_replacement = unsafe {
        float_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Float(6.25)))?
    };
    expect_float(
        float_overload.call_static([])?,
        6.25,
        "staticFloat closure replacement",
    )?;
    closure_replacement.revert()?;

    let double_overload = wrapper.static_method_overload("staticDouble", &[])?;
    let mut closure_replacement = unsafe {
        double_overload.replace_closure(|_| Ok(experimental::RawJavaReturn::Double(12.5)))?
    };
    expect_double(
        double_overload.call_static([])?,
        12.5,
        "staticDouble closure replacement",
    )?;
    closure_replacement.revert()?;

    let closure_string = java.new_string_utf("closure-static-string")?;
    let closure_string_ptr = closure_string.as_jobject() as usize;
    let string_overload = wrapper.static_method_overload("staticString", &[])?;
    let mut closure_replacement = unsafe {
        string_overload.replace_closure(move |_| {
            Ok(experimental::RawJavaReturn::Object(
                closure_string_ptr as jni::jobject,
            ))
        })?
    };
    expect_string(
        string_overload.call_static([])?,
        Some("closure-static-string"),
        "staticString closure replacement",
    )?;
    closure_replacement.revert()?;

    let static_add_overload =
        wrapper.static_method_overload_by_name("staticAdd", &["int", "int"])?;
    let mut closure_replacement = unsafe {
        static_add_overload.replace_closure(|invocation| {
            if invocation.arguments() != [JavaValue::Int(2), JavaValue::Int(5)] {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "staticAdd closure received unexpected arguments".to_owned(),
                });
            }
            let original = invocation
                .call_original((2_i32, 5_i32))?
                .into_int("staticAdd closure original")?;
            Ok(experimental::RawJavaReturn::Int(original + 800))
        })?
    };
    expect_int(
        static_add_overload.call_static([JavaValue::Int(2), JavaValue::Int(5)])?,
        807,
        "staticAdd closure replacement calling original",
    )?;
    closure_replacement.revert()?;

    let original_answer = answer_overload.original()?;
    let _ = FACADE_STATIC_ANSWER_ORIGINAL.set(original_answer);
    let mut replacement = unsafe {
        answer_overload.replace(experimental::MethodImplementation::StaticI32(
            replacement_facade_answer_calling_original,
        ))?
    };
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        2314,
        "facadeAnswer replacement calling original",
    )?;
    replacement.revert()?;

    let mut closure_replacement = unsafe {
        answer_overload.replace_closure(|invocation| {
            let original = invocation
                .call_original(())?
                .into_int("facadeAnswer closure original")?;
            Ok(experimental::RawJavaReturn::Int(original + 3000))
        })?
    };
    expect_int(
        answer_overload.call_static([])?,
        3314,
        "facadeAnswer closure calling original",
    )?;
    closure_replacement.revert()?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    let instance_number_overload = wrapper.method_overload("facadeInstanceNumber", &[])?;
    let mut replacement = unsafe {
        instance_number_overload.replace(experimental::MethodImplementation::InstanceI32(
            replacement_instance_number,
        ))?
    };
    expect_int(
        instance_number_overload.call(&object, [])?,
        2026,
        "facadeInstanceNumber replacement",
    )?;
    match unsafe {
        instance_number_overload.replace(experimental::MethodImplementation::InstanceI32(
            replacement_instance_number,
        ))
    } {
        Err(error) => assert_eq!(
            error,
            Error::InvalidReplacementState {
                operation: "ART replacement registration",
                reason: "target ArtMethod already has an active replacement".to_owned(),
            }
        ),
        Ok(mut duplicate) => {
            duplicate.revert()?;
            return Err(Error::UnsupportedFeature {
                feature: "method replacement lifecycle",
                reason: "duplicate active instance replacement was accepted".to_owned(),
            });
        }
    };
    expect_int(
        instance_number_overload.call(&second_object, [])?,
        -2,
        "facade second receiver facadeInstanceNumber replacement",
    )?;
    replacement.revert()?;

    let mut closure_replacement = unsafe {
        instance_number_overload.replace_closure(|invocation| {
            if invocation.receiver().is_none() || !invocation.arguments().is_empty() {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "instance closure received unexpected invocation shape".to_owned(),
                });
            }
            Ok(experimental::RawJavaReturn::Int(3030))
        })?
    };
    expect_int(
        instance_number_overload.call(&object, [])?,
        3030,
        "facadeInstanceNumber closure replacement",
    )?;
    closure_replacement.revert()?;

    let instance_add_overload = wrapper.method_overload_by_name("instanceAdd", &["int", "int"])?;
    let mut closure_replacement = unsafe {
        instance_add_overload.replace_closure(|invocation| {
            if invocation.receiver().is_none()
                || invocation.class().is_some()
                || invocation.arguments() != [JavaValue::Int(2), JavaValue::Int(5)]
            {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "instanceAdd closure received unexpected invocation shape".to_owned(),
                });
            }
            let original = invocation
                .call_original((2_i32, 5_i32))?
                .into_int("instanceAdd closure original")?;
            Ok(experimental::RawJavaReturn::Int(original + 900))
        })?
    };
    expect_int(
        instance_add_overload.call(&object, [JavaValue::Int(2), JavaValue::Int(5)])?,
        938,
        "instanceAdd closure replacement calling original",
    )?;
    closure_replacement.revert()?;

    let mut implementation = unsafe {
        instance_add_overload.implementation(|invocation| {
            if invocation.receiver().is_none()
                || invocation.arguments() != [JavaValue::Int(2), JavaValue::Int(5)]
            {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: "instanceAdd implementation received unexpected invocation shape"
                        .to_owned(),
                });
            }
            let original = invocation
                .call_original((2_i32, 5_i32))?
                .into_int("instanceAdd implementation original")?;
            Ok(experimental::ImplementationReturn::Int(original + 1000))
        })?
    };
    expect_int(
        instance_add_overload.call(&object, [JavaValue::Int(2), JavaValue::Int(5)])?,
        1038,
        "instanceAdd implementation calling original",
    )?;
    implementation.revert()?;

    let facade_output = java.new_string_utf("facade-replacement")?;
    REPLACEMENT_STRING.store(facade_output.as_jobject(), Ordering::SeqCst);
    let overload_string =
        wrapper.method_overload_by_name("facadeOverload", &["java.lang.String"])?;
    let facade_input = java.new_string_utf("facade-input")?;
    EXPECTED_ARGUMENT.store(facade_input.as_jobject(), Ordering::SeqCst);
    let mut replacement = unsafe {
        overload_string.replace(experimental::MethodImplementation::InstanceStringToString(
            replacement_overload,
        ))?
    };
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        Some("facade-replacement"),
        "facade overload(String) replacement",
    )?;
    replacement.revert()?;

    let closure_output = java.new_string_utf("facade-closure-replacement")?;
    let closure_output_ptr = closure_output.as_jobject() as usize;
    let mut closure_replacement = unsafe {
        overload_string.replace_closure(move |invocation| {
            if invocation.arguments().len() != 1 {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "String closure received the wrong argument count".to_owned(),
                });
            }
            Ok(experimental::RawJavaReturn::Object(
                closure_output_ptr as jni::jobject,
            ))
        })?
    };
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        Some("facade-closure-replacement"),
        "facade overload(String) closure replacement",
    )?;
    closure_replacement.revert()?;

    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);
    let static_object_echo =
        wrapper.static_method_overload_by_name("facadeStaticObjectEcho", &["java.lang.Object"])?;
    let mut replacement = unsafe {
        static_object_echo.replace(
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

    let closure_object_output = second_object.as_jobject() as usize;
    let mut closure_replacement = unsafe {
        static_object_echo.replace_closure(move |invocation| {
            if invocation.arguments().len() != 1 {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "static object closure received unexpected argument count".to_owned(),
                });
            }
            if invocation.arguments()[0] == JavaValue::Null {
                Ok(experimental::RawJavaReturn::Object(ptr::null_mut()))
            } else {
                Ok(experimental::RawJavaReturn::Object(
                    closure_object_output as jni::jobject,
                ))
            }
        })?
    };
    expect_object_same(
        &compare_env,
        static_object_echo.call_static([JavaValue::from(&object)])?,
        Some(second_object.as_jobject()),
        "facade staticObjectEcho closure replacement",
    )?;
    expect_object_same(
        &compare_env,
        static_object_echo.call_static([JavaValue::Null])?,
        None,
        "facade staticObjectEcho null closure replacement",
    )?;
    closure_replacement.revert()?;

    let implementation_object_output = second_object.retain()?;
    let mut implementation = unsafe {
        static_object_echo.implementation(move |invocation| {
            if invocation.arguments().len() != 1 {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: "static object implementation received unexpected argument count"
                        .to_owned(),
                });
            }
            if invocation.arguments()[0] == JavaValue::Null {
                Ok(experimental::ImplementationReturn::object::<JavaObject>(
                    None,
                ))
            } else {
                Ok(experimental::ImplementationReturn::object(Some(
                    &implementation_object_output,
                )))
            }
        })?
    };
    expect_object_same(
        &compare_env,
        static_object_echo.call_static([JavaValue::from(&object)])?,
        Some(second_object.as_jobject()),
        "facade staticObjectEcho implementation replacement",
    )?;
    expect_object_same(
        &compare_env,
        static_object_echo.call_static([JavaValue::Null])?,
        None,
        "facade staticObjectEcho null implementation replacement",
    )?;
    implementation.revert()?;

    subject.call_static("resetVoidCounter", "()V", &[])?;
    VOID_REPLACEMENT_COUNTER.store(0, Ordering::SeqCst);
    let static_object_sink =
        wrapper.static_method_overload_by_name("staticObjectSink", &["java.lang.Object"])?;
    let mut closure_replacement = unsafe {
        static_object_sink.replace_closure(|invocation| {
            match invocation.arguments() {
                [JavaValue::Object(_)] => {
                    VOID_REPLACEMENT_COUNTER.fetch_add(10, Ordering::SeqCst);
                }
                [JavaValue::Null] => {
                    VOID_REPLACEMENT_COUNTER.fetch_add(20, Ordering::SeqCst);
                }
                _ => {
                    return Err(Error::UnsupportedFeature {
                        feature: "closure-backed replacement",
                        reason: "staticObjectSink closure received unexpected arguments".to_owned(),
                    });
                }
            }
            Ok(experimental::RawJavaReturn::Void)
        })?
    };
    static_object_sink.call_static([JavaValue::from(&object)])?;
    static_object_sink.call_static([JavaValue::Null])?;
    expect_int(
        subject.call_static("voidCounter", "()I", &[])?,
        0,
        "staticObjectSink Java state during closure replacement",
    )?;
    if VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst) != 30 {
        return replacement_counter_mismatch(
            "staticObjectSink closure replacement counter",
            30,
            VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst),
        );
    }
    closure_replacement.revert()?;

    VOID_REPLACEMENT_COUNTER.store(0, Ordering::SeqCst);
    let instance_object_sink =
        wrapper.method_overload_by_name("objectSink", &["java.lang.Object"])?;
    let mut closure_replacement = unsafe {
        instance_object_sink.replace_closure(|invocation| {
            if invocation.receiver().is_none() {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "objectSink closure did not receive a receiver".to_owned(),
                });
            }
            match invocation.arguments() {
                [JavaValue::Object(_)] => {
                    VOID_REPLACEMENT_COUNTER.fetch_add(10, Ordering::SeqCst);
                }
                [JavaValue::Null] => {
                    VOID_REPLACEMENT_COUNTER.fetch_add(20, Ordering::SeqCst);
                }
                _ => {
                    return Err(Error::UnsupportedFeature {
                        feature: "closure-backed replacement",
                        reason: "objectSink closure received unexpected arguments".to_owned(),
                    });
                }
            }
            Ok(experimental::RawJavaReturn::Void)
        })?
    };
    instance_object_sink.call(&object, [JavaValue::from(&second_object)])?;
    instance_object_sink.call(&object, [JavaValue::Null])?;
    expect_int(
        subject.call_method(&object, "instanceVoidCounter", "()I", &[])?,
        0,
        "objectSink Java state during closure replacement",
    )?;
    if VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst) != 30 {
        return replacement_counter_mismatch(
            "objectSink closure replacement counter",
            30,
            VOID_REPLACEMENT_COUNTER.load(Ordering::SeqCst),
        );
    }
    closure_replacement.revert()?;

    EXPECTED_ARGUMENT.store(object_array.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object_array.as_jobject(), Ordering::SeqCst);
    let static_object_array_echo = wrapper
        .static_method_overload_by_name("facadeStaticObjectArrayEcho", &["java.lang.Object[]"])?;
    let mut replacement = unsafe {
        static_object_array_echo.replace(
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

    let closure_array_output = second_object_array.as_jobject() as usize;
    let mut closure_replacement = unsafe {
        static_object_array_echo.replace_closure(move |invocation| {
            if invocation.class().is_none() || invocation.arguments().len() != 1 {
                return Err(Error::UnsupportedFeature {
                    feature: "closure-backed replacement",
                    reason: "static object-array closure received unexpected invocation shape"
                        .to_owned(),
                });
            }
            Ok(experimental::RawJavaReturn::Object(
                closure_array_output as jni::jobject,
            ))
        })?
    };
    expect_object_same(
        &compare_env,
        static_object_array_echo.call_static([JavaValue::from(&object_array)])?,
        Some(second_object_array.as_jobject()),
        "facade staticObjectArrayEcho closure replacement",
    )?;
    closure_replacement.revert()?;

    let implementation_array_output =
        java.new_object_array(&object_class, &[Some(&second_object)])?;
    let implementation_array_output_ptr = implementation_array_output.as_jobject();
    let mut implementation = unsafe {
        static_object_array_echo.implementation(move |invocation| {
            if invocation.class().is_none() || invocation.arguments().len() != 1 {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason:
                        "static object-array implementation received unexpected invocation shape"
                            .to_owned(),
                });
            }
            Ok(experimental::ImplementationReturn::array(Some(
                &implementation_array_output,
            )))
        })?
    };
    expect_object_same(
        &compare_env,
        static_object_array_echo.call_static([JavaValue::from(&object_array)])?,
        Some(implementation_array_output_ptr),
        "facade staticObjectArrayEcho implementation replacement",
    )?;
    implementation.revert()?;

    let mut closure_replacement = unsafe {
        answer_overload.replace_closure(|_| {
            Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "intentional closure failure".to_owned(),
            })
        })?
    };
    expect_int(
        answer_overload.call_static([])?,
        0,
        "facadeAnswer closure failure default",
    )?;
    let last_error = closure_replacement
        .last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "closure failure did not record an error".to_owned(),
        })?;
    if !last_error.contains("intentional closure failure") {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: format!("unexpected closure failure error: {last_error}"),
        });
    }
    if !closure_replacement
        .take_last_error()
        .is_some_and(|error| error.contains("intentional closure failure"))
    {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "closure failure take_last_error did not return the recorded error".to_owned(),
        });
    }
    if closure_replacement.last_error().is_some() {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "closure failure take_last_error did not clear the recorded error".to_owned(),
        });
    }
    closure_replacement.revert()?;

    let mut closure_replacement = unsafe {
        answer_overload
            .replace_closure(|_| Ok(experimental::RawJavaReturn::Object(ptr::null_mut())))?
    };
    expect_int(
        answer_overload.call_static([])?,
        0,
        "facadeAnswer closure wrong return default",
    )?;
    let last_error = closure_replacement
        .last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "closure wrong return did not record an error".to_owned(),
        })?;
    if !last_error.contains("requires int return") {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: format!("unexpected closure wrong-return error: {last_error}"),
        });
    }
    closure_replacement.revert()?;

    let mut closure_replacement = unsafe {
        answer_overload.replace_closure(|_| -> Result<experimental::RawJavaReturn> {
            panic!("intentional closure panic")
        })?
    };
    let previous_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic_result = answer_overload.call_static([]);
    std::panic::set_hook(previous_panic_hook);
    expect_int(panic_result?, 0, "facadeAnswer closure panic default")?;
    let last_error =
        closure_replacement
            .take_last_error()
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "closure panic did not record an error".to_owned(),
            })?;
    if !last_error.contains("panicked") {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: format!("unexpected closure panic error: {last_error}"),
        });
    }
    closure_replacement.revert()?;

    let mut implementation = unsafe {
        answer_overload.implementation(|_| {
            Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "intentional implementation failure".to_owned(),
            })
        })?
    };
    expect_int(
        answer_overload.call_static([])?,
        0,
        "facadeAnswer implementation failure default",
    )?;
    let last_error = implementation
        .take_last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: "implementation failure did not record an error".to_owned(),
        })?;
    if !last_error.contains("intentional implementation failure") {
        return Err(Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: format!("unexpected implementation failure error: {last_error}"),
        });
    }
    implementation.revert()?;

    let mut implementation = unsafe {
        answer_overload.implementation(|_| -> Result<experimental::ImplementationReturn> {
            panic!("intentional implementation panic")
        })?
    };
    let previous_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic_result = answer_overload.call_static([]);
    std::panic::set_hook(previous_panic_hook);
    expect_int(
        panic_result?,
        0,
        "facadeAnswer implementation panic default",
    )?;
    let last_error = implementation
        .take_last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: "implementation panic did not record an error".to_owned(),
        })?;
    if !last_error.contains("panicked") {
        return Err(Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: format!("unexpected implementation panic error: {last_error}"),
        });
    }
    implementation.revert()?;

    let unsupported_static = wrapper.static_method_overload_by_name(
        "staticObjectPairEcho",
        &["java.lang.Object", "java.lang.Object"],
    )?;
    if let Ok(mut replacement) = unsafe {
        unsupported_static
            .replace_closure(|_| Ok(experimental::RawJavaReturn::Object(ptr::null_mut())))
    } {
        replacement.revert()?;
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "unsupported static multi-reference closure shape was accepted".to_owned(),
        });
    }
    let unsupported_primitive = wrapper.static_method_overload_by_name(
        "staticPrimitiveMix",
        &["boolean", "byte", "char", "short"],
    )?;
    if let Ok(mut replacement) = unsafe {
        unsupported_primitive.replace_closure(|_| Ok(experimental::RawJavaReturn::Int(0)))
    } {
        replacement.revert()?;
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "unsupported primitive closure shape was accepted".to_owned(),
        });
    }

    run_replacement_lifecycle_checks(java, &subject, &wrapper, &object)?;
    check_startup_hook_shape_replacements(java, &subject, &object, &second_object, &compare_env)?;

    println!("app_process_test: checking app-loader static object replacements");
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let _ = STATIC_OBJECT_ARRAY_ECHO_ORIGINAL.set(static_object_array_echo_original.original()?);
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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

    println!("app_process_test: checking app-loader instance object replacements");
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(second_object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(object.as_jobject(), Ordering::SeqCst);
    let object_echo_overload =
        wrapper.method_overload_by_name("objectEcho", &["java.lang.Object"])?;
    let mut replacement = unsafe {
        object_echo_overload.replace_native(
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
    let mut replacement = unsafe {
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
    let _ =
        INSTANCE_OBJECT_ARRAY_ECHO_ORIGINAL.set(instance_object_array_echo_original.original()?);
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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

    println!("app_process_test: checking app-loader overload isolation");
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
        Some("dex-test"),
        "message original",
    )?;
    let mut replacement = unsafe {
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
        Some("dex-test"),
        "message restored",
    )?;

    println!("app_process_test: checking app-loader instance replacement across receivers");
    let mut replacement = unsafe {
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

    println!("app_process_test: checking instance original call from replacement");
    let mut replacement = unsafe {
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

    println!("app_process_test: checking app-loader instance primitive replacements");
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    subject.call_method(&object, "bumpInstanceVoidCounter", "()V", &[])?;
    expect_int(
        subject.call_method(&object, "instanceVoidCounter", "()I", &[])?,
        1,
        "bumpInstanceVoidCounter original",
    )?;
    VOID_REPLACEMENT_COUNTER.store(0, Ordering::SeqCst);
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let _ = INSTANCE_ADD_ORIGINAL.set(instance_add_original.original()?);
    let mut replacement = unsafe {
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

    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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
    let mut replacement = unsafe {
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

    println!("app_process_test: checking private static replacement");
    let hidden_output = java.new_string_utf("app-process-replacement")?;
    REPLACEMENT_STRING.store(hidden_output.as_jobject(), Ordering::SeqCst);
    match unsafe {
        experimental::replace_static_string_method(&subject, "hiddenStatic", replacement_string)
    } {
        Ok(mut replacement) => {
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
            println!("app_process_test: private static replacement lookup unavailable");
        }
        Err(error) => return Err(error),
    }

    REPLACEMENT_STRING.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_RECEIVER.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}

fn check_startup_hook_shape_replacements(
    java: &Java,
    subject: &JavaClass,
    object: &JavaObject,
    second_object: &JavaObject,
    compare_env: &Env<'_>,
) -> Result<()> {
    println!("app_process_test: checking startup-hook replacement ABI shapes");
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);

    let six_signature =
        "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;";
    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    let mut replacement = unsafe {
        experimental::replace_instance_native_method(
            subject,
            "startupLoadedApkSix",
            six_signature,
            replacement_startup_loaded_apk_six as *const () as *mut std::ffi::c_void,
        )?
    };
    expect_object_same(
        compare_env,
        subject.call_method(
            object,
            "startupLoadedApkSix",
            six_signature,
            &[
                JavaValue::from(object),
                JavaValue::from(second_object),
                JavaValue::from(second_object),
                JavaValue::Boolean(true),
                JavaValue::Boolean(false),
                JavaValue::Boolean(true),
            ],
        )?,
        Some(second_object.as_jobject()),
        "startupLoadedApkSix replacement",
    )?;
    replacement.revert()?;

    let seven_signature =
        "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZZ)Ljava/lang/Object;";
    let mut replacement = unsafe {
        experimental::replace_instance_native_method(
            subject,
            "startupLoadedApkSeven",
            seven_signature,
            replacement_startup_loaded_apk_seven as *const () as *mut std::ffi::c_void,
        )?
    };
    expect_object_same(
        compare_env,
        subject.call_method(
            object,
            "startupLoadedApkSeven",
            seven_signature,
            &[
                JavaValue::from(object),
                JavaValue::from(second_object),
                JavaValue::from(second_object),
                JavaValue::Boolean(true),
                JavaValue::Boolean(false),
                JavaValue::Boolean(true),
                JavaValue::Boolean(false),
            ],
        )?,
        Some(second_object.as_jobject()),
        "startupLoadedApkSeven replacement",
    )?;
    replacement.revert()?;

    let three_signature = "(Ljava/lang/Object;Ljava/lang/Object;I)Ljava/lang/Object;";
    let mut replacement = unsafe {
        experimental::replace_instance_native_method(
            subject,
            "startupLoadedApkThree",
            three_signature,
            replacement_startup_loaded_apk_three as *const () as *mut std::ffi::c_void,
        )?
    };
    expect_object_same(
        compare_env,
        subject.call_method(
            object,
            "startupLoadedApkThree",
            three_signature,
            &[
                JavaValue::from(object),
                JavaValue::from(second_object),
                JavaValue::Int(7),
            ],
        )?,
        Some(second_object.as_jobject()),
        "startupLoadedApkThree replacement",
    )?;
    replacement.revert()?;

    let string_signature = "(Ljava/lang/String;Ljava/lang/Object;I)Ljava/lang/Object;";
    let package_name = java.new_string_utf("frida.java.bridge.rs.test")?;
    EXPECTED_ARGUMENT.store(package_name.as_jobject(), Ordering::SeqCst);
    let mut replacement = unsafe {
        experimental::replace_instance_native_method(
            subject,
            "startupLoadedApkString",
            string_signature,
            replacement_startup_loaded_apk_string as *const () as *mut std::ffi::c_void,
        )?
    };
    expect_object_same(
        compare_env,
        subject.call_method(
            object,
            "startupLoadedApkString",
            string_signature,
            &[
                JavaValue::from(&package_name),
                JavaValue::from(second_object),
                JavaValue::Int(9),
            ],
        )?,
        Some(second_object.as_jobject()),
        "startupLoadedApkString replacement",
    )?;
    replacement.revert()?;

    let make_application_signature = "(ZLjava/lang/Object;)Ljava/lang/Object;";
    EXPECTED_ARGUMENT.store(second_object.as_jobject(), Ordering::SeqCst);
    let mut replacement = unsafe {
        experimental::replace_instance_native_method(
            subject,
            "startupMakeApplication",
            make_application_signature,
            replacement_startup_make_application as *const () as *mut std::ffi::c_void,
        )?
    };
    expect_object_same(
        compare_env,
        subject.call_method(
            object,
            "startupMakeApplication",
            make_application_signature,
            &[JavaValue::Boolean(false), JavaValue::from(second_object)],
        )?,
        Some(second_object.as_jobject()),
        "startupMakeApplication replacement",
    )?;
    replacement.revert()?;
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}
