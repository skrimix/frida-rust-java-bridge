use super::assertions::*;
use super::replacement_lifecycle::run_replacement_lifecycle_checks;
use super::*;

pub(super) fn run_replacement_checks(java: &Java, app_java: &Java) -> Result<()> {
    let capabilities = java.capabilities();
    if !capabilities.method_replacement.is_supported() {
        if let Some(reason) = capabilities.method_replacement.unsupported_reason() {
            println!("app_process_test: skipping replacement checks: {reason}");
            return Ok(());
        }
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: "method replacement capability reported an unknown state".to_owned(),
        });
    }

    let subject = app_java.find_class(TEST_SUBJECT)?;
    let cached_subject = app_java.find_class(TEST_SUBJECT)?;
    let wrapper = app_java.use_class(TEST_SUBJECT)?;

    println!("app_process_test: checking public constructor implementation replacement");
    let int_constructor = wrapper.constructor(["int"])?;
    let number_field = wrapper.field("number")?;
    let baseline_object = int_constructor.new_object((31 as jni::jint,))?;
    if number_field.get_int(&baseline_object)? != 31 {
        return test_error("TestSubject(int) baseline constructor did not set number");
    }
    let mut constructor_replacement = int_constructor.replace(|invocation| {
        let receiver = invocation.this_object()?;
        if invocation.kind() != MethodKind::Constructor
            || invocation.name() != "<init>"
            || invocation.args().len() != 1
        {
            return Err(Error::UnsupportedFeature {
                feature: "constructor replacement",
                reason: "constructor closure received unexpected invocation shape".to_owned(),
            });
        }
        let number = invocation.arg::<jni::jint>(0)?;
        let initialized = invocation.call_original((number + 1000,))?;
        if receiver.as_jobject().is_null() {
            return Err(Error::NullReturn {
                operation: "constructor replacement initialized receiver",
            });
        }
        Ok(initialized)
    })?;
    let Some(summary) = constructor_replacement.debug_summary() else {
        return Err(Error::UnsupportedFeature {
            feature: "ART method replacement",
            reason: "constructor replacement debug summary was unavailable".to_owned(),
        });
    };
    expect_clone_backend_summary(&summary)?;
    match int_constructor.replace(|invocation| invocation.call_original_current()) {
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
                feature: "constructor replacement",
                reason: "duplicate active constructor replacement was accepted".to_owned(),
            });
        }
    };
    let replacement_object = subject.new_object("(I)V", &[JavaValue::Int(41)])?;
    if number_field.get_int(&replacement_object)? != 1041 {
        return test_error("TestSubject(int) constructor replacement did not set number");
    }
    let cached_replacement_object = cached_subject.new_object("(I)V", &[JavaValue::Int(42)])?;
    if number_field.get_int(&cached_replacement_object)? != 1042 {
        return test_error("cached TestSubject(int) constructor replacement did not set number");
    }
    let wrapper_replacement_object = wrapper
        .constructor(["int"])?
        .new_object((43 as jni::jint,))?;
    if number_field.get_int(&wrapper_replacement_object)? != 1043 {
        return test_error("wrapper TestSubject(int) constructor replacement did not set number");
    }
    java.find_class("java.lang.System")?
        .call_static("gc", "()V", &[])?;
    let post_gc_object = int_constructor.new_object((44 as jni::jint,))?;
    if number_field.get_int(&post_gc_object)? != 1044 {
        return test_error("TestSubject(int) constructor replacement failed after System.gc");
    }
    constructor_replacement.revert()?;

    let mut wrong_return_constructor =
        unsafe { int_constructor.replace_unchecked(|_| Ok(replacement::JavaHookReturn::int(7)))? };
    let _ = int_constructor.new_object((45 as jni::jint,))?;
    let last_error =
        wrong_return_constructor
            .take_last_error()
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: "constructor replacement",
                reason: "constructor wrong return did not record an error".to_owned(),
            })?;
    if !last_error.contains("requires void return") {
        return Err(Error::UnsupportedFeature {
            feature: "constructor replacement",
            reason: format!("unexpected constructor wrong-return error: {last_error}"),
        });
    }
    wrong_return_constructor.revert()?;

    let mut failing_constructor = int_constructor.replace(|_| {
        Err(Error::UnsupportedFeature {
            feature: "constructor replacement",
            reason: "intentional safe constructor failure".to_owned(),
        })
    })?;
    match int_constructor.new_object((47 as jni::jint,)) {
        Err(Error::JavaException { exception, .. })
            if exception.contains("java.lang.IllegalStateException") => {}
        Err(error) => {
            failing_constructor.revert()?;
            return Err(Error::UnsupportedFeature {
                feature: "constructor replacement",
                reason: format!("safe constructor failure raised unexpected error: {error}"),
            });
        }
        Ok(_) => {
            failing_constructor.revert()?;
            return Err(Error::UnsupportedFeature {
                feature: "constructor replacement",
                reason: "safe constructor failure completed object creation".to_owned(),
            });
        }
    }
    let last_error =
        failing_constructor
            .take_last_error()
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: "constructor replacement",
                reason: "safe constructor failure did not record an error".to_owned(),
            })?;
    if !last_error.contains("intentional safe constructor failure") {
        failing_constructor.revert()?;
        return Err(Error::UnsupportedFeature {
            feature: "constructor replacement",
            reason: format!("unexpected safe constructor failure error: {last_error}"),
        });
    }
    failing_constructor.revert()?;

    let restored_object = int_constructor.new_object((46 as jni::jint,))?;
    if number_field.get_int(&restored_object)? != 46 {
        return test_error("TestSubject(int) constructor replacement did not restore original");
    }

    let object = subject.new_object("(I)V", &[JavaValue::Int(31)])?;
    let second_object = subject.new_object("(I)V", &[JavaValue::Int(32)])?;
    let compare_env = java.vm().attach_current_thread()?;
    let object_class = java.find_class("java.lang.Object")?;
    let object_array =
        java.new_object_array(&object_class, &[Some(&object), Some(&second_object)])?;
    let second_object_array = java.new_object_array(&object_class, &[Some(&second_object)])?;
    let int_array = app_java.new_int_array(&[1, 2, 3])?;

    println!("app_process_test: checking overload facade replacements");
    let mut direct_answer_replacement = wrapper.replace("facadeAnswer", |_| Ok(1441))?;
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        1441,
        "facadeAnswer direct replacement",
    )?;
    direct_answer_replacement.revert()?;

    let answer_overload = wrapper.method("facadeAnswer")?.overload([] as [&str; 0])?;
    let mut replacement = answer_overload.replace(|_| Ok(1337))?;
    expect_int(
        subject.call_static("facadeAnswer", "()I", &[])?,
        1337,
        "facadeAnswer replacement",
    )?;
    match answer_overload.replace(|_| Ok(1337)) {
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

    println!("app_process_test: checking replacement frame stack visitor compatibility");
    let throwable_class = java.find_class("java.lang.Throwable")?;
    let stackvisitor_message = java.new_string_utf("stackvisitor-active-replacement")?;
    let mut stackvisitor_replacement = answer_overload.replace(move |_| {
        let throwable = throwable_class.new_object(
            "(Ljava/lang/String;)V",
            &[JavaValue::from(&stackvisitor_message)],
        )?;
        let stack_trace = throwable_class
            .call_method(
                &throwable,
                "getStackTrace",
                "()[Ljava/lang/StackTraceElement;",
                &[],
            )?
            .into_array("Throwable.getStackTrace during replacement")?
            .ok_or(Error::NullReturn {
                operation: "Throwable.getStackTrace during replacement",
            })?;
        if stack_trace.is_empty()? {
            return Err(Error::UnsupportedFeature {
                feature: "ART method replacement stack visitor",
                reason: "Throwable.getStackTrace returned an empty stack".to_owned(),
            });
        }
        Ok(6061)
    })?;
    expect_int(
        answer_overload.call((), ())?,
        6061,
        "facadeAnswer stack visitor replacement",
    )?;
    stackvisitor_replacement.revert()?;

    let mut closure_replacement = answer_overload.replace(|_| Ok(4040))?;
    let Some(summary) = closure_replacement.debug_summary() else {
        return Err(Error::UnsupportedFeature {
            feature: "closure-backed replacement",
            reason: "closure replacement debug summary was unavailable".to_owned(),
        });
    };
    expect_clone_backend_summary(&summary)?;
    expect_int(
        answer_overload.call((), ())?,
        4040,
        "facadeAnswer closure replacement",
    )?;
    closure_replacement.revert()?;
    expect_int(
        answer_overload.call((), ())?,
        314,
        "facadeAnswer restored after closure replacement",
    )?;

    let mut implementation = answer_overload.replace(|invocation| {
        if invocation.kind() != MethodKind::Static
            || invocation.name() != "facadeAnswer"
            || invocation.signature().to_string() != "()I"
            || invocation.maybe_this_object()?.is_some()
            || !invocation.args().is_empty()
        {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "facadeAnswer implementation received unexpected invocation shape"
                    .to_owned(),
            });
        }
        Ok(5050)
    })?;
    expect_int(
        answer_overload.call((), ())?,
        5050,
        "facadeAnswer implementation replacement",
    )?;
    match answer_overload.replace(|_| Ok(6060)) {
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
                feature: "implementation replacement",
                reason: "duplicate active implementation replacement was accepted".to_owned(),
            });
        }
    };
    implementation.revert()?;
    implementation.revert()?;
    expect_int(
        answer_overload.call((), ())?,
        314,
        "facadeAnswer restored after implementation replacement",
    )?;

    let answer_handle = wrapper.method("facadeAnswer")?.overload([] as [&str; 0])?;
    let mut implementation = answer_handle.replace(|invocation| {
        if invocation.kind() != MethodKind::Static
            || invocation.name() != "facadeAnswer"
            || invocation.signature().to_string() != "()I"
            || invocation.maybe_this_object()?.is_some()
            || !invocation.args().is_empty()
        {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "facadeAnswer method selector received unexpected invocation shape"
                    .to_owned(),
            });
        }
        Ok(5151)
    })?;
    expect_int(
        answer_handle.call((), ())?,
        5151,
        "facadeAnswer method selector implementation replacement",
    )?;
    implementation.revert()?;
    expect_int(
        answer_handle.call((), ())?,
        314,
        "facadeAnswer restored after method selector implementation replacement",
    )?;

    let boolean_overload = wrapper.method("staticBoolean")?.overload([] as [&str; 0])?;
    let mut closure_replacement = boolean_overload.replace(|invocation| {
        if invocation.kind() != MethodKind::Static
            || invocation.name() != "staticBoolean"
            || invocation.signature().to_string() != "()Z"
            || invocation.maybe_this_object()?.is_some()
            || !invocation.args().is_empty()
        {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticBoolean closure received unexpected invocation shape".to_owned(),
            });
        }
        Ok(false)
    })?;
    expect_bool(
        boolean_overload.call((), ())?,
        false,
        "staticBoolean closure replacement",
    )?;
    closure_replacement.revert()?;

    let byte_overload = wrapper.method("staticByte")?.overload([] as [&str; 0])?;
    let mut closure_replacement = byte_overload.replace(|_| Ok(-12 as jni::jbyte))?;
    expect_byte(
        byte_overload.call((), ())?,
        -12,
        "staticByte closure replacement",
    )?;
    closure_replacement.revert()?;

    let char_overload = wrapper.method("staticChar")?.overload([] as [&str; 0])?;
    let mut closure_replacement = char_overload.replace(|_| Ok(90 as jni::jchar))?;
    expect_char(
        char_overload.call((), ())?,
        90,
        "staticChar closure replacement",
    )?;
    closure_replacement.revert()?;

    let short_overload = wrapper.method("staticShort")?.overload([] as [&str; 0])?;
    let mut closure_replacement = short_overload.replace(|_| Ok(-321 as jni::jshort))?;
    expect_short(
        short_overload.call((), ())?,
        -321,
        "staticShort closure replacement",
    )?;
    closure_replacement.revert()?;

    let long_overload = wrapper.method("staticLong")?.overload([] as [&str; 0])?;
    let mut closure_replacement = long_overload.replace(|_| Ok(9_876_543_210_i64))?;
    expect_long(
        long_overload.call((), ())?,
        9_876_543_210,
        "staticLong closure replacement",
    )?;
    closure_replacement.revert()?;

    let float_overload = wrapper.method("staticFloat")?.overload([] as [&str; 0])?;
    let mut closure_replacement = float_overload.replace(|_| Ok(6.25_f32))?;
    expect_float(
        float_overload.call((), ())?,
        6.25,
        "staticFloat closure replacement",
    )?;
    closure_replacement.revert()?;

    let double_overload = wrapper.method("staticDouble")?.overload([] as [&str; 0])?;
    let mut closure_replacement = double_overload.replace(|_| Ok(12.5))?;
    expect_double(
        double_overload.call((), ())?,
        12.5,
        "staticDouble closure replacement",
    )?;
    closure_replacement.revert()?;

    let closure_string = java.new_string_utf("closure-static-string")?;
    let string_overload = wrapper.method("staticString")?.overload([] as [&str; 0])?;
    let mut closure_replacement =
        string_overload.replace(move |_| Ok(closure_string.as_hook_return()))?;
    expect_string(
        string_overload.call((), ())?,
        Some("closure-static-string"),
        "staticString closure replacement",
    )?;
    closure_replacement.revert()?;

    let mut direct_add_replacement =
        wrapper.replace_with("staticAdd", ["int", "int"], |_| Ok(9001))?;
    expect_int(
        wrapper.call_with::<JavaReturn>("staticAdd", ["int", "int"], (2, 5))?,
        9001,
        "staticAdd direct overload replacement",
    )?;
    direct_add_replacement.revert()?;

    let static_add_overload = wrapper.method("staticAdd")?.overload(["int", "int"])?;
    let mut closure_replacement = static_add_overload.replace(|invocation| {
        let args = invocation
            .args()
            .iter()
            .map(|argument| {
                argument.and_then(|argument| match argument {
                    replacement::JavaHookArgument::Int(value) => Ok(value),
                    other => Err(Error::UnsupportedFeature {
                        feature: "closure-backed replacement",
                        reason: format!("staticAdd unexpected argument view: {other:?}"),
                    }),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if args != [2, 5] {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticAdd closure received unexpected arguments".to_owned(),
            });
        }
        let original: i32 = invocation.call_original((2_i32, 5_i32))?;
        Ok(original + 800)
    })?;
    expect_int(
        static_add_overload.call((), [JavaValue::Int(2), JavaValue::Int(5)])?,
        807,
        "staticAdd closure replacement calling original",
    )?;
    closure_replacement.revert()?;

    let static_identity_overload = wrapper.method("staticIdentity")?.overload(["int"])?;
    let mut implementation = static_identity_overload.replace(|invocation| {
        let value: i32 = invocation.arg(0)?;
        let original: i32 = invocation.call_original(value)?;
        Ok(original + 1000)
    })?;
    expect_int(
        static_identity_overload.call((), [JavaValue::Int(41)])?,
        1041,
        "staticIdentity arbitrary implementation calling original",
    )?;
    implementation.revert()?;

    let static_boolean_arg = wrapper.method("staticBooleanFromInt")?.overload(["int"])?;
    let mut implementation = static_boolean_arg.replace(|invocation| {
        let value: i32 = invocation.arg(0)?;
        Ok(value == 0)
    })?;
    expect_bool(
        static_boolean_arg.call((), [JavaValue::Int(5)])?,
        false,
        "staticBooleanFromInt arbitrary implementation",
    )?;
    implementation.revert()?;

    let static_byte_arg = wrapper.method("staticByteFromByte")?.overload(["byte"])?;
    let mut implementation = static_byte_arg.replace(|invocation| {
        let value: jni::jbyte = invocation.arg(0)?;
        Ok(value + 10_i8)
    })?;
    expect_byte(
        static_byte_arg.call((), [JavaValue::Byte(2)])?,
        12,
        "staticByteFromByte arbitrary implementation",
    )?;
    implementation.revert()?;

    let static_char_arg = wrapper.method("staticCharFromChar")?.overload(["char"])?;
    let mut implementation = static_char_arg.replace(|invocation| {
        let value: jni::jchar = invocation.arg(0)?;
        Ok(value + 10_u16)
    })?;
    expect_char(
        static_char_arg.call((), [JavaValue::Char(b'A' as jni::jchar)])?,
        b'K' as jni::jchar,
        "staticCharFromChar arbitrary implementation",
    )?;
    implementation.revert()?;

    let static_short_arg = wrapper
        .method("staticShortFromShort")?
        .overload(["short"])?;
    let mut implementation = static_short_arg.replace(|invocation| {
        let value: jni::jshort = invocation.arg(0)?;
        Ok(value + 10_i16)
    })?;
    expect_short(
        static_short_arg.call((), [JavaValue::Short(2)])?,
        12,
        "staticShortFromShort arbitrary implementation",
    )?;
    implementation.revert()?;

    let static_float_arg = wrapper
        .method("staticFloatFromFloat")?
        .overload(["float"])?;
    let mut implementation = static_float_arg.replace(|invocation| {
        let value: f32 = invocation.arg(0)?;
        Ok(value + 10.0)
    })?;
    expect_float(
        static_float_arg.call((), [JavaValue::Float(2.5)])?,
        12.5,
        "staticFloatFromFloat arbitrary implementation",
    )?;
    implementation.revert()?;

    subject.call_static("resetVoidCounter", "()V", &[])?;
    let static_object_int_sink = wrapper
        .method("staticObjectIntSink")?
        .overload(["java.lang.Object", "int"])?;
    let mut implementation = static_object_int_sink.replace(|invocation| {
        let value: Option<JavaLocalObject> = invocation.arg(0)?;
        let extra: i32 = invocation.arg(1)?;
        if value.is_some() && extra == 7 {
            Ok(())
        } else {
            Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "staticObjectIntSink arbitrary implementation received unexpected args"
                    .to_owned(),
            })
        }
    })?;
    static_object_int_sink.call::<()>((), [JavaValue::from(&object), JavaValue::Int(7)])?;
    expect_int(
        subject.call_static("voidCounter", "()I", &[])?,
        0,
        "staticObjectIntSink arbitrary implementation skipped Java state",
    )?;
    implementation.revert()?;

    let mixed_reference_overload = wrapper
        .method("staticReferencePrimitiveArrayMix")?
        .overload(["java.lang.Object", "int", "java.lang.Object[]", "boolean"])?;
    let mixed_reference_output_ptr = second_object_array.as_jobject();
    let mixed_reference_output = second_object_array.retain()?;
    let mut implementation = mixed_reference_overload.replace(move |invocation| {
        if invocation.args().len() != 4 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "staticReferencePrimitiveArrayMix argument count mismatch".to_owned(),
            });
        }
        let first = invocation.arg_object(0)?;
        let value: i32 = invocation.arg(1)?;
        let second = invocation.arg_array(2)?;
        let choose_array: bool = invocation.arg(3)?;
        let original = invocation.call_original_object((
            first.as_ref(),
            value,
            second.as_ref(),
            choose_array,
        ))?;
        if original.is_none() {
            Ok(replacement::JavaHookReturn::null_object())
        } else {
            Ok(mixed_reference_output.as_hook_return())
        }
    })?;
    expect_object_same(
        &compare_env,
        mixed_reference_overload.call(
            (),
            [
                JavaValue::from(&object),
                JavaValue::Int(1),
                JavaValue::from(&object_array),
                JavaValue::Boolean(true),
            ],
        )?,
        Some(mixed_reference_output_ptr),
        "staticReferencePrimitiveArrayMix arbitrary implementation",
    )?;
    expect_object_same(
        &compare_env,
        mixed_reference_overload.call(
            (),
            [
                JavaValue::Null,
                JavaValue::Int(0),
                JavaValue::from(&object_array),
                JavaValue::Boolean(false),
            ],
        )?,
        None,
        "staticReferencePrimitiveArrayMix arbitrary implementation null original",
    )?;
    implementation.revert()?;

    let static_pair_overload = wrapper
        .method("staticObjectPairEcho")?
        .overload(["java.lang.Object", "java.lang.Object"])?;
    let static_pair_closure_output = second_object.retain()?;
    let mut closure_replacement = static_pair_overload.replace(move |invocation| {
        if invocation.arg_is_null(0)? || invocation.arg_is_null(1)? {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticObjectPairEcho closure received unexpected arguments".to_owned(),
            });
        }
        Ok(static_pair_closure_output.as_hook_return())
    })?;
    expect_object_same(
        &compare_env,
        static_pair_overload.call(
            (),
            [JavaValue::from(&object), JavaValue::from(&second_object)],
        )?,
        Some(second_object.as_jobject()),
        "staticObjectPairEcho multi-reference closure replacement",
    )?;
    closure_replacement.revert()?;

    let static_pair_output = second_object.retain()?;
    let mut implementation = static_pair_overload.replace(move |invocation| {
        if invocation.kind() != MethodKind::Static || invocation.args().len() != 2 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "staticObjectPairEcho implementation received unexpected invocation shape"
                    .to_owned(),
            });
        }
        let first = invocation.arg_object(0)?;
        let second = invocation.arg_object(1)?;
        if first.is_none() && second.is_none() {
            Ok(replacement::JavaHookReturn::null_object())
        } else {
            Ok(static_pair_output.as_hook_return())
        }
    })?;
    expect_object_same(
        &compare_env,
        static_pair_overload.call(
            (),
            [JavaValue::from(&object), JavaValue::from(&second_object)],
        )?,
        Some(second_object.as_jobject()),
        "staticObjectPairEcho implementation replacement",
    )?;
    expect_object_same(
        &compare_env,
        static_pair_overload.call((), [JavaValue::Null, JavaValue::Null])?,
        None,
        "staticObjectPairEcho null implementation replacement",
    )?;
    implementation.revert()?;

    let primitive_mix_overload = wrapper
        .method("staticPrimitiveMix")?
        .overload(["boolean", "byte", "char", "short"])?;
    let mut closure_replacement = primitive_mix_overload.replace(|invocation| {
        let flag: bool = invocation.arg(0)?;
        let value: jni::jbyte = invocation.arg(1)?;
        let letter: jni::jchar = invocation.arg(2)?;
        let extra: jni::jshort = invocation.arg(3)?;
        if (flag, value, letter, extra) != (true, 2, b'C' as jni::jchar, 5) {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticPrimitiveMix closure received unexpected arguments".to_owned(),
            });
        }
        Ok(5151)
    })?;
    expect_int(
        primitive_mix_overload.call(
            (),
            [
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        5151,
        "staticPrimitiveMix generic closure replacement",
    )?;
    closure_replacement.revert()?;

    let mut implementation = primitive_mix_overload.replace(|invocation| {
        let flag: bool = invocation.arg(0)?;
        let value: jni::jbyte = invocation.arg(1)?;
        let letter: jni::jchar = invocation.arg(2)?;
        let extra: jni::jshort = invocation.arg(3)?;
        if (flag, value, letter, extra) != (true, 2, b'C' as jni::jchar, 5) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "staticPrimitiveMix implementation received unexpected arguments"
                    .to_owned(),
            });
        }
        let original: i32 = invocation.call_original((flag, value, letter, extra))?;
        Ok(original + 5000)
    })?;
    expect_int(
        primitive_mix_overload.call(
            (),
            [
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        5074,
        "staticPrimitiveMix implementation calling original",
    )?;
    implementation.revert()?;

    let static_wide_overload = wrapper.method("staticWide")?.overload(["long", "double"])?;
    let mut closure_replacement = static_wide_overload.replace(|invocation| {
        let value: i64 = invocation.arg(0)?;
        let extra: f64 = invocation.arg(1)?;
        if (value, extra) != (40, 2.0) {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticWide closure received unexpected arguments".to_owned(),
            });
        }
        Ok(8080)
    })?;
    expect_long(
        static_wide_overload.call((), [JavaValue::Long(40), JavaValue::Double(2.0)])?,
        8080,
        "staticWide generic closure replacement",
    )?;
    closure_replacement.revert()?;

    let mut implementation = static_wide_overload.replace(|invocation| {
        let value: i64 = invocation.arg(0)?;
        let extra: f64 = invocation.arg(1)?;
        if (value, extra) != (40, 2.0) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "staticWide implementation received unexpected arguments".to_owned(),
            });
        }
        Ok(8181_i64)
    })?;
    expect_long(
        static_wide_overload.call((), [JavaValue::Long(40), JavaValue::Double(2.0)])?,
        8181,
        "staticWide implementation replacement",
    )?;
    implementation.revert()?;

    let static_float_mix_overload = wrapper
        .method("staticFloatMix")?
        .overload(["float", "double"])?;
    let mut closure_replacement = static_float_mix_overload.replace(|invocation| {
        let value: f32 = invocation.arg(0)?;
        let extra: f64 = invocation.arg(1)?;
        if (value, extra) != (1.5, 2.25) {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticFloatMix closure received unexpected arguments".to_owned(),
            });
        }
        Ok(9090.5)
    })?;
    expect_double(
        static_float_mix_overload.call((), [JavaValue::Float(1.5), JavaValue::Double(2.25)])?,
        9090.5,
        "staticFloatMix generic closure replacement",
    )?;
    closure_replacement.revert()?;

    let mut implementation = static_float_mix_overload.replace(|invocation| {
        let value: f32 = invocation.arg(0)?;
        let extra: f64 = invocation.arg(1)?;
        if (value, extra) != (1.5, 2.25) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "staticFloatMix implementation received unexpected arguments".to_owned(),
            });
        }
        Ok(9191.5_f64)
    })?;
    expect_double(
        static_float_mix_overload.call((), [JavaValue::Float(1.5), JavaValue::Double(2.25)])?,
        9191.5,
        "staticFloatMix implementation replacement",
    )?;
    implementation.revert()?;

    let stack_arg_types = [
        "int", "int", "int", "int", "int", "int", "int", "int", "double", "double", "double",
        "double", "double", "double", "double", "double", "double",
    ];
    let stack_args = [
        JavaValue::Int(1),
        JavaValue::Int(2),
        JavaValue::Int(3),
        JavaValue::Int(4),
        JavaValue::Int(5),
        JavaValue::Int(6),
        JavaValue::Int(7),
        JavaValue::Int(8),
        JavaValue::Double(0.5),
        JavaValue::Double(1.5),
        JavaValue::Double(2.5),
        JavaValue::Double(3.5),
        JavaValue::Double(4.5),
        JavaValue::Double(5.5),
        JavaValue::Double(6.5),
        JavaValue::Double(7.5),
        JavaValue::Double(8.5),
    ];
    let stack_spill_overload = wrapper
        .method("staticStackSpill")?
        .overload(stack_arg_types.as_slice())?;
    expect_double(
        stack_spill_overload.call((), stack_args)?,
        76.5,
        "staticStackSpill original",
    )?;
    let mut closure_replacement = stack_spill_overload.replace(|invocation| {
        expect_stack_spill_arguments(
            &invocation,
            "closure-backed replacement",
            "staticStackSpill closure received unexpected arguments",
        )?;
        Ok(7070.5)
    })?;
    expect_double(
        stack_spill_overload.call((), stack_args)?,
        7070.5,
        "staticStackSpill stack-passed closure replacement",
    )?;
    closure_replacement.revert()?;

    let mut implementation = stack_spill_overload.replace(|invocation| {
        expect_stack_spill_arguments(
            &invocation,
            "implementation replacement",
            "staticStackSpill implementation received unexpected arguments",
        )?;
        let original: f64 = invocation.call_original(crate::java_args![
            1 as jni::jint,
            2 as jni::jint,
            3 as jni::jint,
            4 as jni::jint,
            5 as jni::jint,
            6 as jni::jint,
            7 as jni::jint,
            8 as jni::jint,
            0.5 as jni::jdouble,
            1.5 as jni::jdouble,
            2.5 as jni::jdouble,
            3.5 as jni::jdouble,
            4.5 as jni::jdouble,
            5.5 as jni::jdouble,
            6.5 as jni::jdouble,
            7.5 as jni::jdouble,
            8.5 as jni::jdouble,
        ])?;
        Ok(original + 1000.0)
    })?;
    expect_double(
        stack_spill_overload.call((), stack_args)?,
        1076.5,
        "staticStackSpill implementation calling original",
    )?;
    implementation.revert()?;

    let mut closure_replacement = answer_overload.replace(|invocation| {
        let original: i32 = invocation.call_original(())?;
        Ok(original + 3000)
    })?;
    expect_int(
        answer_overload.call((), ())?,
        3314,
        "facadeAnswer closure calling original",
    )?;
    closure_replacement.revert()?;

    let mut implementation = answer_overload.replace(|invocation| {
        let original: i32 = invocation.call_original(())?;
        Ok(original + 4000)
    })?;
    expect_int(
        answer_overload.call((), ())?,
        4314,
        "facadeAnswer implementation calling original",
    )?;
    implementation.revert()?;

    let throwing_answer_overload = wrapper
        .method("facadeThrowingAnswer")?
        .overload([] as [&str; 0])?;
    let mut throwing_replacement =
        throwing_answer_overload.replace(|invocation| invocation.call_original_current())?;
    match throwing_answer_overload.call::<jni::jint>((), ()) {
        Err(Error::JavaException { exception, .. }) if exception.contains("facade-boom") => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return Err(Error::UnsupportedFeature {
                feature: "replacement original-call Java exception rethrow",
                reason: format!("throwing replacement returned default/value {value}"),
            });
        }
    }
    let last_error =
        throwing_replacement
            .take_last_error()
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: "replacement original-call Java exception rethrow",
                reason: "throwing replacement did not record an error".to_owned(),
            })?;
    if !last_error.contains("facade-boom") {
        return Err(Error::UnsupportedFeature {
            feature: "replacement original-call Java exception rethrow",
            reason: format!("unexpected throwing replacement error: {last_error}"),
        });
    }
    throwing_replacement.revert()?;

    let wrapper_throwing_answer = throwing_answer_overload.clone();
    let mut wrapper_throwing_replacement =
        answer_overload.replace(move |_| -> Result<replacement::JavaHookReturn> {
            let _value: jni::jint = wrapper_throwing_answer.call((), ())?;
            Ok(replacement::JavaHookReturn::Int(1234))
        })?;
    match answer_overload.call::<jni::jint>((), ()) {
        Err(Error::JavaException { exception, .. }) if exception.contains("facade-boom") => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return Err(Error::UnsupportedFeature {
                feature: "replacement wrapper-call Java exception rethrow",
                reason: format!("wrapper-call replacement returned default/value {value}"),
            });
        }
    }
    let last_error = wrapper_throwing_replacement
        .take_last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "replacement wrapper-call Java exception rethrow",
            reason: "wrapper-call replacement did not record an error".to_owned(),
        })?;
    if !last_error.contains("facade-boom") {
        return Err(Error::UnsupportedFeature {
            feature: "replacement wrapper-call Java exception rethrow",
            reason: format!("unexpected wrapper-call replacement error: {last_error}"),
        });
    }
    wrapper_throwing_replacement.revert()?;

    let wrapper_throwing_answer = throwing_answer_overload.clone();
    let mut wrapper_error_conversion =
        answer_overload.replace(move |_| -> Result<replacement::JavaHookReturn> {
            match wrapper_throwing_answer.call::<jni::jint>((), ()) {
                Err(Error::JavaException { exception, .. })
                    if exception.contains("facade-boom") =>
                {
                    Err(Error::UnsupportedFeature {
                        feature: "replacement wrapper-call Java exception conversion",
                        reason: "converted Java exception into Rust error".to_owned(),
                    })
                }
                Err(error) => Err(error),
                Ok(value) => Err(Error::UnsupportedFeature {
                    feature: "replacement wrapper-call Java exception conversion",
                    reason: format!("throwing wrapper unexpectedly returned {value}"),
                }),
            }
        })?;
    expect_int(
        answer_overload.call((), ())?,
        0,
        "facadeAnswer converted wrapper exception default",
    )?;
    let last_error =
        wrapper_error_conversion
            .take_last_error()
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: "replacement wrapper-call Java exception conversion",
                reason: "converted wrapper-call replacement did not record an error".to_owned(),
            })?;
    if !last_error.contains("converted Java exception") {
        return Err(Error::UnsupportedFeature {
            feature: "replacement wrapper-call Java exception conversion",
            reason: format!("unexpected converted wrapper-call error: {last_error}"),
        });
    }
    wrapper_error_conversion.revert()?;

    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    let instance_number_overload = wrapper
        .method("facadeInstanceNumber")?
        .overload([] as [&str; 0])?;
    let mut replacement = instance_number_overload.replace(|_| Ok(2026))?;
    expect_int(
        instance_number_overload.call(&object, ())?,
        2026,
        "facadeInstanceNumber replacement",
    )?;
    match instance_number_overload.replace(|_| Ok(2026)) {
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
    replacement.revert()?;

    let mut closure_replacement = instance_number_overload.replace(|invocation| {
        if invocation.maybe_this_object()?.is_none() || !invocation.args().is_empty() {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "instance closure received unexpected invocation shape".to_owned(),
            });
        }
        Ok(3030)
    })?;
    expect_int(
        instance_number_overload.call(&object, ())?,
        3030,
        "facadeInstanceNumber closure replacement",
    )?;
    closure_replacement.revert()?;

    let receiver_number_field = wrapper.field("number")?;
    let this_object = subject.new_object("(I)V", &[JavaValue::Int(31)])?;
    let subject_for_receiver_callback = subject.clone();
    let mut implementation = instance_number_overload.replace(move |invocation| {
        let receiver = invocation.this_object()?;
        subject_for_receiver_callback.set_field(&receiver, "number", "I", JavaValue::Int(41))?;
        let original: i32 = invocation.call_original(())?;
        Ok(original)
    })?;
    let receiver_result = instance_number_overload.call(&this_object, ())?;
    if !matches!(receiver_result, JavaReturn::Int(141)) {
        return Err(Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: format!(
                "facadeInstanceNumber implementation using this_object mismatch: expected int 141, got {receiver_result:?}, last error {:?}",
                implementation.last_error()
            ),
        });
    }
    implementation.revert()?;
    let receiver_number = receiver_number_field.get_int(&this_object)?;
    if receiver_number != 41 {
        return Err(Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: format!("this_object field write mismatch: {receiver_number}"),
        });
    }

    let instance_number_handle = wrapper
        .method("facadeInstanceNumber")?
        .overload([] as [&str; 0])?;
    let mut implementation = instance_number_handle.replace(|invocation| {
        if invocation.kind() != MethodKind::Instance
            || invocation.name() != "facadeInstanceNumber"
            || invocation.signature().to_string() != "()I"
            || invocation.maybe_this_object()?.is_none()
            || !invocation.args().is_empty()
        {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "facadeInstanceNumber method selector received unexpected invocation shape"
                    .to_owned(),
            });
        }
        Ok(6161)
    })?;
    expect_int(
        instance_number_handle.call(&object, ())?,
        6161,
        "facadeInstanceNumber method selector implementation replacement",
    )?;
    implementation.revert()?;

    let instance_add_overload = wrapper.method("instanceAdd")?.overload(["int", "int"])?;
    let mut closure_replacement = instance_add_overload.replace(|invocation| {
        let a: i32 = invocation.arg(0)?;
        let b: i32 = invocation.arg(1)?;
        if invocation.maybe_this_object()?.is_none() || (a, b) != (2, 5) {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "instanceAdd closure received unexpected invocation shape".to_owned(),
            });
        }
        let original: i32 = invocation.call_original((a, b))?;
        Ok(original + 900)
    })?;
    expect_int(
        instance_add_overload.call(&object, [JavaValue::Int(2), JavaValue::Int(5)])?,
        938,
        "instanceAdd closure replacement calling original",
    )?;
    closure_replacement.revert()?;

    let mut implementation = instance_add_overload.replace(|invocation| {
        if invocation.maybe_this_object()?.is_none() {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instanceAdd implementation did not receive a receiver".to_owned(),
            });
        }
        let a: i32 = invocation.arg(0)?;
        let b: i32 = invocation.arg(1)?;
        if (a, b) != (2, 5) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instanceAdd implementation received unexpected arguments".to_owned(),
            });
        }
        let original: i32 = invocation.call_original((a, b))?;
        Ok(original + 1000)
    })?;
    expect_int(
        instance_add_overload.call(&object, [JavaValue::Int(2), JavaValue::Int(5)])?,
        1038,
        "instanceAdd implementation calling original",
    )?;
    implementation.revert()?;

    let instance_pair_overload = wrapper
        .method("objectPairEcho")?
        .overload(["java.lang.Object", "java.lang.Object"])?;
    let mut implementation = instance_pair_overload.replace(|invocation| {
        let receiver = invocation.this_object()?;
        if invocation.args().len() != 2 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "objectPairEcho implementation received unexpected invocation shape"
                    .to_owned(),
            });
        }
        let receiver_string = receiver.java_to_string()?;
        if !receiver_string.contains("frida.java.bridge.rs.test.TestSubject@") {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: format!("unexpected receiver toString: {receiver_string}"),
            });
        }
        let first: Option<JavaLocalObject> = invocation.arg(0)?;
        if first.is_some() {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "objectPairEcho expected a null first argument".to_owned(),
            });
        }
        match invocation.arg::<JavaLocalObject>(0) {
            Err(Error::NullReturn {
                operation: "JavaHookContext::arg",
            }) => {}
            Err(error) => return Err(error),
            Ok(_) => {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: "objectPairEcho non-null typed extraction accepted null".to_owned(),
                });
            }
        }
        let argument: Option<JavaLocalObject> = invocation.arg(1)?;
        if let Some(argument) = &argument {
            let argument_string = argument.java_to_string()?;
            if !argument_string.contains("frida.java.bridge.rs.test.TestSubject@") {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: format!("unexpected argument toString: {argument_string}"),
                });
            }
            let argument_display = invocation.arg_display(1)?;
            if !argument_display.contains("frida.java.bridge.rs.test.TestSubject@") {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: format!("unexpected argument display: {argument_display}"),
                });
            }
            let argument_value_display = invocation.arg_value(1)?.java_display()?;
            if argument_value_display != argument_display {
                return Err(Error::UnsupportedFeature {
                    feature: "implementation replacement",
                    reason: format!("argument value display mismatch: {argument_value_display}"),
                });
            }
        } else if invocation.arg_display(1)? != "null" {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "null argument display mismatch".to_owned(),
            });
        } else if invocation.arg_value(1)?.java_display()? != "null" {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "null argument value display mismatch".to_owned(),
            });
        }
        invocation.call_original_return((first.as_ref(), argument.as_ref()))
    })?;
    expect_object_same(
        &compare_env,
        instance_pair_overload.call(&object, [JavaValue::Null, JavaValue::from(&second_object)])?,
        Some(second_object.as_jobject()),
        "objectPairEcho implementation calling original",
    )?;
    expect_object_same(
        &compare_env,
        instance_pair_overload.call(&object, [JavaValue::Null, JavaValue::Null])?,
        None,
        "objectPairEcho null implementation calling original",
    )?;
    implementation.revert()?;

    let instance_primitive_mix_overload = wrapper
        .method("instancePrimitiveMix")?
        .overload(["boolean", "byte", "char", "short"])?;
    let mut implementation = instance_primitive_mix_overload.replace(|invocation| {
        if invocation.maybe_this_object()?.is_none() {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instancePrimitiveMix implementation did not receive a receiver".to_owned(),
            });
        }
        let flag: bool = invocation.arg(0)?;
        let value: jni::jbyte = invocation.arg(1)?;
        let letter: jni::jchar = invocation.arg(2)?;
        let extra: jni::jshort = invocation.arg(3)?;
        if (flag, value, letter, extra) != (true, 2, b'C' as jni::jchar, 5) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instancePrimitiveMix implementation received unexpected arguments"
                    .to_owned(),
            });
        }
        Ok(5252_i32)
    })?;
    expect_int(
        instance_primitive_mix_overload.call(
            &object,
            [
                JavaValue::Boolean(true),
                JavaValue::Byte(2),
                JavaValue::Char(b'C' as jni::jchar),
                JavaValue::Short(5),
            ],
        )?,
        5252,
        "instancePrimitiveMix implementation replacement",
    )?;
    implementation.revert()?;

    let instance_wide_overload = wrapper
        .method("instanceWide")?
        .overload(["long", "double"])?;
    let mut implementation = instance_wide_overload.replace(|invocation| {
        let value: i64 = invocation.arg(0)?;
        let extra: f64 = invocation.arg(1)?;
        if (value, extra) != (40, 2.0) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instanceWide implementation received unexpected arguments".to_owned(),
            });
        }
        Ok(8282_i64)
    })?;
    expect_long(
        instance_wide_overload.call(&object, [JavaValue::Long(40), JavaValue::Double(2.0)])?,
        8282,
        "instanceWide implementation replacement",
    )?;
    implementation.revert()?;

    let instance_float_mix_overload = wrapper
        .method("instanceFloatMix")?
        .overload(["float", "double"])?;
    let mut implementation = instance_float_mix_overload.replace(|invocation| {
        let value: f32 = invocation.arg(0)?;
        let extra: f64 = invocation.arg(1)?;
        if (value, extra) != (1.5, 2.25) {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instanceFloatMix implementation received unexpected arguments".to_owned(),
            });
        }
        Ok(9292.5_f64)
    })?;
    expect_double(
        instance_float_mix_overload
            .call(&object, [JavaValue::Float(1.5), JavaValue::Double(2.25)])?,
        9292.5,
        "instanceFloatMix implementation replacement",
    )?;
    implementation.revert()?;

    let instance_stack_spill_overload = wrapper
        .method("instanceStackSpill")?
        .overload(stack_arg_types.as_slice())?;
    expect_double(
        instance_stack_spill_overload.call(&object, stack_args)?,
        107.5,
        "instanceStackSpill original",
    )?;
    let mut implementation = instance_stack_spill_overload.replace(|invocation| {
        if invocation.maybe_this_object()?.is_none() {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "instanceStackSpill implementation received unexpected invocation shape"
                    .to_owned(),
            });
        }
        expect_stack_spill_arguments(
            &invocation,
            "implementation replacement",
            "instanceStackSpill implementation received unexpected invocation shape",
        )?;
        let original: f64 = invocation.call_original(crate::java_args![
            1 as jni::jint,
            2 as jni::jint,
            3 as jni::jint,
            4 as jni::jint,
            5 as jni::jint,
            6 as jni::jint,
            7 as jni::jint,
            8 as jni::jint,
            0.5 as jni::jdouble,
            1.5 as jni::jdouble,
            2.5 as jni::jdouble,
            3.5 as jni::jdouble,
            4.5 as jni::jdouble,
            5.5 as jni::jdouble,
            6.5 as jni::jdouble,
            7.5 as jni::jdouble,
            8.5 as jni::jdouble,
        ])?;
        Ok(original + 2000.0)
    })?;
    expect_double(
        instance_stack_spill_overload.call(&object, stack_args)?,
        2107.5,
        "instanceStackSpill implementation calling original",
    )?;
    implementation.revert()?;

    let facade_output = java.new_string_utf("facade-replacement")?;
    REPLACEMENT_STRING.store(facade_output.as_jobject(), Ordering::SeqCst);
    let overload_string = wrapper
        .method("facadeOverload")?
        .overload(["java.lang.String"])?;
    let facade_input = java.new_string_utf("facade-input")?;
    EXPECTED_ARGUMENT.store(facade_input.as_jobject(), Ordering::SeqCst);
    let mut replacement = overload_string.replace(move |invocation| {
        let argument = invocation.arg::<Option<String>>(0)?;
        if argument.as_deref() != Some("facade-input") {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "facadeOverload received unexpected String argument".to_owned(),
            });
        }
        Ok(facade_output.as_hook_return())
    })?;
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        Some("facade-replacement"),
        "facade overload(String) replacement",
    )?;
    replacement.revert()?;

    let closure_output = java.new_string_utf("facade-closure-replacement")?;
    let mut closure_replacement = overload_string.replace(move |invocation| {
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "String closure received the wrong argument count".to_owned(),
            });
        }
        Ok(closure_output.as_hook_return())
    })?;
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        Some("facade-closure-replacement"),
        "facade overload(String) closure replacement",
    )?;
    closure_replacement.revert()?;

    let mut implementation = overload_string.replace(|invocation| {
        let argument = invocation.arg::<String>(0)?;
        if argument != "facade-input" {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: format!("unexpected String argument: {argument:?}"),
            });
        }
        let input = invocation.arg_object(0)?;
        let original = invocation.call_original::<String>(input.as_ref())?;
        if original != "facade-input" {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: format!("unexpected original String return: {original:?}"),
            });
        }
        Ok(replacement::JavaHookReturn::from(input.as_ref()))
    })?;
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        Some("facade-input"),
        "facade overload(String) implementation using string conversions",
    )?;
    implementation.revert()?;

    let invalid_string_return = object.as_jobject() as usize;
    let mut invalid_return_replacement = overload_string.replace(move |_| unsafe {
        Ok(replacement::JavaHookReturn::raw_object(
            invalid_string_return as jni::jobject,
        ))
    })?;
    expect_string(
        overload_string.call(&object, [JavaValue::from(&facade_input)])?,
        None,
        "facade overload(String) invalid object return default",
    )?;
    let last_error = invalid_return_replacement
        .take_last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: "invalid object return did not record an error".to_owned(),
        })?;
    if !last_error.contains("closure replacement return expected object") {
        return Err(Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: format!("unexpected invalid object return error: {last_error}"),
        });
    }
    invalid_return_replacement.revert()?;

    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);
    let static_object_echo = wrapper
        .method("facadeStaticObjectEcho")?
        .overload(["java.lang.Object"])?;
    let static_object_output = second_object.retain()?;
    let mut replacement = static_object_echo.replace(move |invocation| {
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "static object replacement received unexpected arguments".to_owned(),
            });
        }
        Ok(static_object_output.as_hook_return())
    })?;
    expect_object_same(
        &compare_env,
        static_object_echo.call((), [JavaValue::from(&object)])?,
        Some(second_object.as_jobject()),
        "facade staticObjectEcho replacement",
    )?;
    replacement.revert()?;

    let closure_object_output = second_object.retain()?;
    let mut closure_replacement = static_object_echo.replace(move |invocation| {
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "static object closure received unexpected argument count".to_owned(),
            });
        }
        if invocation.arg_is_null(0)? {
            Ok(replacement::JavaHookReturn::null_object())
        } else {
            Ok(closure_object_output.as_hook_return())
        }
    })?;
    expect_object_same(
        &compare_env,
        static_object_echo.call((), [JavaValue::from(&object)])?,
        Some(second_object.as_jobject()),
        "facade staticObjectEcho closure replacement",
    )?;
    expect_object_same(
        &compare_env,
        static_object_echo.call((), [JavaValue::Null])?,
        None,
        "facade staticObjectEcho null closure replacement",
    )?;
    closure_replacement.revert()?;

    let implementation_object_output = second_object.retain()?;
    let mut implementation = static_object_echo.replace(move |invocation| {
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "static object implementation received unexpected argument count"
                    .to_owned(),
            });
        }
        if invocation.arg_is_null(0)? {
            Ok(replacement::JavaHookReturn::null_object())
        } else {
            Ok(implementation_object_output.as_hook_return())
        }
    })?;
    expect_object_same(
        &compare_env,
        static_object_echo.call((), [JavaValue::from(&object)])?,
        Some(second_object.as_jobject()),
        "facade staticObjectEcho implementation replacement",
    )?;
    expect_object_same(
        &compare_env,
        static_object_echo.call((), [JavaValue::Null])?,
        None,
        "facade staticObjectEcho null implementation replacement",
    )?;
    implementation.revert()?;

    subject.call_static("resetVoidCounter", "()V", &[])?;
    VOID_REPLACEMENT_COUNTER.store(0, Ordering::SeqCst);
    let static_object_sink = wrapper
        .method("staticObjectSink")?
        .overload(["java.lang.Object"])?;
    let mut closure_replacement = static_object_sink.replace(|invocation| {
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "staticObjectSink closure received unexpected arguments".to_owned(),
            });
        }
        if invocation.arg_is_null(0)? {
            VOID_REPLACEMENT_COUNTER.fetch_add(20, Ordering::SeqCst);
        } else {
            VOID_REPLACEMENT_COUNTER.fetch_add(10, Ordering::SeqCst);
        }
        Ok(())
    })?;
    static_object_sink.call::<()>((), [JavaValue::from(&object)])?;
    static_object_sink.call::<()>((), [JavaValue::Null])?;
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
    let instance_object_sink = wrapper
        .method("objectSink")?
        .overload(["java.lang.Object"])?;
    let mut closure_replacement = instance_object_sink.replace(|invocation| {
        if invocation.maybe_this_object()?.is_none() {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "objectSink closure did not receive a receiver".to_owned(),
            });
        }
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "objectSink closure received unexpected arguments".to_owned(),
            });
        }
        if invocation.arg_is_null(0)? {
            VOID_REPLACEMENT_COUNTER.fetch_add(20, Ordering::SeqCst);
        } else {
            VOID_REPLACEMENT_COUNTER.fetch_add(10, Ordering::SeqCst);
        }
        Ok(())
    })?;
    instance_object_sink.call::<()>(&object, [JavaValue::from(&second_object)])?;
    instance_object_sink.call::<()>(&object, [JavaValue::Null])?;
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
        .method("facadeStaticObjectArrayEcho")?
        .overload(["java.lang.Object[]"])?;
    let static_object_array_output = second_object_array.retain()?;
    let mut replacement = static_object_array_echo.replace(move |invocation| {
        if invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "static object-array replacement received unexpected arguments".to_owned(),
            });
        }
        Ok(static_object_array_output.as_hook_return())
    })?;
    expect_object_same(
        &compare_env,
        static_object_array_echo.call((), [JavaValue::from(&object_array)])?,
        Some(second_object_array.as_jobject()),
        "facade staticObjectArrayEcho replacement",
    )?;
    replacement.revert()?;

    let closure_array_output = second_object_array.retain()?;
    let mut closure_replacement = static_object_array_echo.replace(move |invocation| {
        if invocation.kind() != MethodKind::Static || invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "static object-array closure received unexpected invocation shape"
                    .to_owned(),
            });
        }
        Ok(closure_array_output.as_hook_return())
    })?;
    expect_object_same(
        &compare_env,
        static_object_array_echo.call((), [JavaValue::from(&object_array)])?,
        Some(second_object_array.as_jobject()),
        "facade staticObjectArrayEcho closure replacement",
    )?;
    closure_replacement.revert()?;

    let implementation_array_output =
        java.new_object_array(&object_class, &[Some(&second_object)])?;
    let implementation_array_output_ptr = implementation_array_output.as_jobject();
    let mut implementation = static_object_array_echo.replace(move |invocation| {
        if invocation.kind() != MethodKind::Static || invocation.args().len() != 1 {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "static object-array implementation received unexpected invocation shape"
                    .to_owned(),
            });
        }
        Ok(implementation_array_output.as_hook_return())
    })?;
    expect_object_same(
        &compare_env,
        static_object_array_echo.call((), [JavaValue::from(&object_array)])?,
        Some(implementation_array_output_ptr),
        "facade staticObjectArrayEcho implementation replacement",
    )?;
    implementation.revert()?;

    let mut closure_replacement =
        answer_overload.replace(|_| -> Result<replacement::JavaHookReturn> {
            Err(Error::UnsupportedFeature {
                feature: "closure-backed replacement",
                reason: "intentional closure failure".to_owned(),
            })
        })?;
    expect_int(
        answer_overload.call((), ())?,
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

    let mut closure_replacement =
        answer_overload.replace(|_| Ok(replacement::JavaHookReturn::null_object()))?;
    expect_int(
        answer_overload.call((), ())?,
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

    let mut closure_replacement =
        answer_overload.replace(|_| -> Result<replacement::JavaHookReturn> {
            panic!("intentional closure panic")
        })?;
    let previous_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic_result = answer_overload.call((), ());
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

    let mut implementation =
        answer_overload.replace(|_| -> Result<replacement::JavaHookReturn> {
            Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "intentional implementation failure".to_owned(),
            })
        })?;
    expect_int(
        answer_overload.call((), ())?,
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

    let mut implementation =
        answer_overload.replace(|_| Ok(replacement::JavaHookReturn::null_object()))?;
    expect_int(
        answer_overload.call((), ())?,
        0,
        "facadeAnswer implementation wrong return default",
    )?;
    let last_error = implementation
        .take_last_error()
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: "implementation wrong return did not record an error".to_owned(),
        })?;
    if !last_error.contains("requires int return") {
        return Err(Error::UnsupportedFeature {
            feature: "implementation replacement",
            reason: format!("unexpected implementation wrong-return error: {last_error}"),
        });
    }
    implementation.revert()?;

    let mut implementation =
        answer_overload.replace(|_| -> Result<replacement::JavaHookReturn> {
            panic!("intentional implementation panic")
        })?;
    let previous_panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic_result = answer_overload.call((), ());
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

    let array_to_int = wrapper.method("sumIntArray")?.overload(["int[]"])?;
    let mut implementation = array_to_int.replace(|invocation| {
        let array: JavaLocalArray = invocation.arg(0)?;
        let nullable_array: Option<JavaLocalArray> = invocation.arg(0)?;
        if nullable_array.is_none() {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: "typed int[] argument unexpectedly returned null".to_owned(),
            });
        }
        let values = array.get_ints()?;
        if values != [1, 2, 3] {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: format!("unexpected int[] argument values: {values:?}"),
            });
        }
        let argument_display = invocation.arg_display(0)?;
        let argument_value_display = invocation.arg_value(0)?.java_display()?;
        if argument_display != argument_value_display || !argument_display.starts_with("[I@") {
            return Err(Error::UnsupportedFeature {
                feature: "implementation replacement",
                reason: format!("unexpected int[] argument display: {argument_value_display}"),
            });
        }
        let original = invocation
            .call_original_return(Some(&array))?
            .into_int("sumIntArray typed-array original return")?;
        Ok(original + 100)
    })?;
    expect_int(
        array_to_int.call(&object, [JavaValue::from(&int_array)])?,
        106,
        "sumIntArray arbitrary implementation calling original",
    )?;
    implementation.revert()?;

    run_replacement_lifecycle_checks(java, &subject, &wrapper, &object)?;
    check_startup_hook_shape_replacements(java, &subject, &object, &second_object, &compare_env)?;

    REPLACEMENT_STRING.store(ptr::null_mut(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_RECEIVER.store(ptr::null_mut(), Ordering::SeqCst);
    EXPECTED_ARGUMENT.store(ptr::null_mut(), Ordering::SeqCst);
    Ok(())
}

fn expect_stack_spill_arguments(
    invocation: &replacement::JavaHookContext<'_>,
    feature: &'static str,
    reason: &'static str,
) -> Result<()> {
    for (index, expected) in [1, 2, 3, 4, 5, 6, 7, 8].into_iter().enumerate() {
        let actual: i32 = invocation.arg(index)?;
        if actual != expected {
            return Err(Error::UnsupportedFeature {
                feature,
                reason: reason.to_owned(),
            });
        }
    }
    for (offset, expected) in [0.5, 1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5]
        .into_iter()
        .enumerate()
    {
        let actual: f64 = invocation.arg(offset + 8)?;
        if (actual - expected).abs() >= 0.0001 {
            return Err(Error::UnsupportedFeature {
                feature,
                reason: reason.to_owned(),
            });
        }
    }
    Ok(())
}

fn check_startup_hook_shape_replacements(
    java: &Java,
    subject: &RawJavaClass,
    object: &JavaObject,
    second_object: &JavaObject,
    compare_env: &Env<'_>,
) -> Result<()> {
    println!("app_process_test: checking startup-hook replacement descriptor shapes");
    EXPECTED_RECEIVER.store(object.as_jobject(), Ordering::SeqCst);
    REPLACEMENT_OBJECT.store(second_object.as_jobject(), Ordering::SeqCst);

    let six_signature =
        "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;";
    EXPECTED_ARGUMENT.store(object.as_jobject(), Ordering::SeqCst);
    let mut replacement = replace_startup_shape(
        subject,
        "startupLoadedApkSix",
        six_signature,
        6,
        second_object,
    )?;
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
    let mut replacement = replace_startup_shape(
        subject,
        "startupLoadedApkSeven",
        seven_signature,
        7,
        second_object,
    )?;
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
    let mut replacement = replace_startup_shape(
        subject,
        "startupLoadedApkThree",
        three_signature,
        3,
        second_object,
    )?;
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
    let mut replacement = replace_startup_shape(
        subject,
        "startupLoadedApkString",
        string_signature,
        3,
        second_object,
    )?;
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
    let mut replacement = replace_startup_shape(
        subject,
        "startupMakeApplication",
        make_application_signature,
        2,
        second_object,
    )?;
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

fn replace_startup_shape(
    subject: &RawJavaClass,
    name: &'static str,
    signature: &str,
    expected_argument_count: usize,
    output: &JavaObject,
) -> Result<replacement::JavaHookGuard> {
    let method = JavaMethod::from_raw_exact(subject, MethodKind::Instance, name, signature)?;
    let output = output.retain()?;
    method.replace(move |invocation| {
        if invocation.args().len() != expected_argument_count {
            return Err(Error::UnsupportedFeature {
                feature: "startup-hook descriptor replacement",
                reason: format!(
                    "{name} received {} arguments, expected {expected_argument_count}",
                    invocation.args().len()
                ),
            });
        }
        Ok(output.as_hook_return())
    })
}
