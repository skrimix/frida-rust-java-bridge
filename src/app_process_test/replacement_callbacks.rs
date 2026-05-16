use super::*;

pub(super) unsafe extern "C" fn replacement_answer(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jint {
    1337
}

pub(super) unsafe extern "C" fn replacement_lifecycle_static_a(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jint {
    1700
}

pub(super) unsafe extern "C" fn replacement_lifecycle_static_b(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jint {
    2700
}

pub(super) unsafe extern "C" fn replacement_answer_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
) -> jni::jint {
    match unsafe { experimental::call_original_static_i32_method(env, class, "answer") } {
        Ok(value) => value + 1000,
        Err(error) => {
            println!("app_process_test: static original call failed: {error}");
            -1000
        }
    }
}

pub(super) unsafe extern "C" fn replacement_facade_answer_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
) -> jni::jint {
    let Some(original) = FACADE_STATIC_ANSWER_ORIGINAL.get() else {
        return -2000;
    };
    match unsafe { original.call_static(env, class, ()) }
        .and_then(|value| value.into_int("facade static original call"))
    {
        Ok(value) => value + 2000,
        Err(error) => {
            println!("app_process_test: facade static original call failed: {error}");
            -2001
        }
    }
}

pub(super) unsafe extern "C" fn replacement_void(_env: *mut jni::JNIEnv, _class: jni::jclass) {
    VOID_REPLACEMENT_COUNTER.fetch_add(1, Ordering::SeqCst);
}

pub(super) unsafe extern "C" fn replacement_string(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jstring {
    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

pub(super) unsafe extern "C" fn replacement_boolean(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jboolean {
    jni::JNI_FALSE
}

pub(super) unsafe extern "C" fn replacement_byte(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jbyte {
    -8
}

pub(super) unsafe extern "C" fn replacement_char(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jchar {
    b'Z' as jni::jchar
}

pub(super) unsafe extern "C" fn replacement_short(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jshort {
    -1234
}

pub(super) unsafe extern "C" fn replacement_long(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jlong {
    -9876543210
}

pub(super) unsafe extern "C" fn replacement_float(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jfloat {
    -2.5
}

pub(super) unsafe extern "C" fn replacement_double(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
) -> jni::jdouble {
    -6.25
}

pub(super) unsafe extern "C" fn replacement_static_echo(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    argument: jni::jstring,
) -> jni::jstring {
    let expected_argument = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
    if env.is_null()
        || expected_argument.is_null()
        || argument.is_null()
        || !unsafe { raw_is_same_object(env, argument, expected_argument) }
    {
        return ptr::null_mut();
    }

    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

pub(super) unsafe extern "C" fn replacement_static_echo_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    argument: jni::jstring,
) -> jni::jstring {
    let arg = if argument.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(argument)
    };
    match unsafe {
        experimental::call_original_static_method(
            env,
            class,
            "staticEcho",
            "(Ljava/lang/String;)Ljava/lang/String;",
            [arg],
        )
    } {
        Ok(experimental::RawJavaReturn::Object(value)) => {
            let expected_argument = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
            if argument.is_null() {
                if value.is_null() {
                    REPLACEMENT_STRING.load(Ordering::SeqCst)
                } else {
                    ptr::null_mut()
                }
            } else if !expected_argument.is_null()
                && !value.is_null()
                && unsafe { raw_is_same_object(env, value, expected_argument) }
            {
                REPLACEMENT_STRING.load(Ordering::SeqCst)
            } else {
                ptr::null_mut()
            }
        }
        Ok(other) => {
            println!("app_process_test: staticEcho original returned {other:?}");
            ptr::null_mut()
        }
        Err(error) => {
            println!("app_process_test: staticEcho original call failed: {error}");
            ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn replacement_static_object_echo(
    env: *mut jni::JNIEnv,
    _class: jni::jclass,
    argument: jni::jobject,
) -> jni::jobject {
    if unsafe { replacement_argument_matches(env, argument) } {
        REPLACEMENT_OBJECT.load(Ordering::SeqCst)
    } else {
        ptr::null_mut()
    }
}

pub(super) unsafe extern "C" fn replacement_static_object_array_echo(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    argument: jni::jobject,
) -> jni::jobject {
    unsafe { replacement_static_object_echo(env, class, argument) }
}

pub(super) unsafe extern "C" fn replacement_static_object_echo_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    argument: jni::jobject,
) -> jni::jobject {
    let arg = if argument.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(argument)
    };
    match unsafe {
        experimental::call_original_static_method(
            env,
            class,
            "staticObjectEcho",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            [arg],
        )
    } {
        Ok(experimental::RawJavaReturn::Object(value))
            if unsafe { replacement_argument_matches(env, value) } =>
        {
            REPLACEMENT_OBJECT.load(Ordering::SeqCst)
        }
        Ok(other) => {
            println!("app_process_test: staticObjectEcho original returned {other:?}");
            ptr::null_mut()
        }
        Err(error) => {
            println!("app_process_test: staticObjectEcho original call failed: {error}");
            ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn replacement_static_object_array_echo_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    argument: jni::jobject,
) -> jni::jobject {
    let Some(original) = STATIC_OBJECT_ARRAY_ECHO_ORIGINAL.get() else {
        return ptr::null_mut();
    };
    let arg = if argument.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(argument)
    };
    match unsafe { original.call_static(env, class, (arg,)) }
        .and_then(|value| value.into_object("staticObjectArrayEcho original call"))
    {
        Ok(value) if unsafe { replacement_argument_matches(env, value) } => {
            REPLACEMENT_OBJECT.load(Ordering::SeqCst)
        }
        Ok(_) => {
            println!(
                "app_process_test: staticObjectArrayEcho original returned unexpected object"
            );
            ptr::null_mut()
        }
        Err(error) => {
            println!("app_process_test: staticObjectArrayEcho original call failed: {error}");
            ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn replacement_static_add(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    left + right + 45
}

pub(super) unsafe extern "C" fn replacement_static_add_calling_original(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    match unsafe {
        experimental::call_original_static_method(env, class, "staticAdd", "(II)I", (left, right))
    }
    .and_then(|value| value.into_int("staticAdd original call"))
    {
        Ok(value) => value + 1000,
        Err(error) => {
            println!("app_process_test: staticAdd original call failed: {error}");
            -1000
        }
    }
}

pub(super) unsafe extern "C" fn replacement_static_primitive_mix(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    flag: jni::jboolean,
    value: jni::jbyte,
    letter: jni::jchar,
    extra: jni::jshort,
) -> jni::jint {
    if flag == jni::JNI_TRUE && value == 2 && letter == b'C' as jni::jchar && extra == 5 {
        4242
    } else {
        -4242
    }
}

pub(super) unsafe extern "C" fn replacement_static_wide(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    value: jni::jlong,
    extra: jni::jdouble,
) -> jni::jlong {
    if value == 40 && (extra - 2.0).abs() < 0.0001 {
        9001
    } else {
        -9001
    }
}

pub(super) unsafe extern "C" fn replacement_static_float_mix(
    _env: *mut jni::JNIEnv,
    _class: jni::jclass,
    value: jni::jfloat,
    extra: jni::jdouble,
) -> jni::jdouble {
    if (value - 1.5).abs() < 0.0001 && (extra - 2.25).abs() < 0.0001 {
        8.5
    } else {
        -8.5
    }
}

pub(super) unsafe extern "C" fn replacement_instance_number(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jint {
    let expected = EXPECTED_RECEIVER.load(Ordering::SeqCst);
    if expected.is_null() || env.is_null() {
        return -1;
    }
    if unsafe { raw_is_same_object(env, receiver, expected) } {
        2026
    } else {
        -2
    }
}

pub(super) unsafe extern "C" fn replacement_lifecycle_instance_a(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jint {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        1701
    } else {
        -1701
    }
}

pub(super) unsafe extern "C" fn replacement_lifecycle_instance_b(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jint {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        2701
    } else {
        -2701
    }
}

pub(super) unsafe extern "C" fn replacement_instance_number_calling_original(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jint {
    match unsafe {
        experimental::call_original_instance_i32_method(env, receiver, "instanceNumber")
    } {
        Ok(value) => value + 100,
        Err(error) => {
            println!("app_process_test: instance original call failed: {error}");
            -100
        }
    }
}

pub(super) unsafe extern "C" fn replacement_instance_void(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        VOID_REPLACEMENT_COUNTER.fetch_add(1, Ordering::SeqCst);
    } else {
        VOID_REPLACEMENT_COUNTER.store(-100, Ordering::SeqCst);
    }
}

pub(super) unsafe extern "C" fn replacement_instance_boolean(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jboolean {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        jni::JNI_FALSE
    } else {
        jni::JNI_TRUE
    }
}

pub(super) unsafe extern "C" fn replacement_instance_byte(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jbyte {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        -8
    } else {
        8
    }
}

pub(super) unsafe extern "C" fn replacement_instance_char(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jchar {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        b'Z' as jni::jchar
    } else {
        b'Y' as jni::jchar
    }
}

pub(super) unsafe extern "C" fn replacement_instance_short(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jshort {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        -1234
    } else {
        1234
    }
}

pub(super) unsafe extern "C" fn replacement_instance_long(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jlong {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        -9876543210
    } else {
        9876543210
    }
}

pub(super) unsafe extern "C" fn replacement_instance_float(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jfloat {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        -2.5
    } else {
        2.5
    }
}

pub(super) unsafe extern "C" fn replacement_instance_double(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
) -> jni::jdouble {
    if unsafe { replacement_receiver_matches(env, receiver) } {
        -6.25
    } else {
        6.25
    }
}

pub(super) unsafe extern "C" fn replacement_instance_add(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    if unsafe { replacement_receiver_matches(env, receiver) } && left == 2 && right == 5 {
        left + right + 45
    } else {
        -4242
    }
}

pub(super) unsafe extern "C" fn replacement_instance_add_calling_original(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    let Some(original) = INSTANCE_ADD_ORIGINAL.get() else {
        return -1000;
    };
    match unsafe { original.call_instance(env, receiver, (left, right)) }
        .and_then(|value| value.into_int("instanceAdd original call"))
    {
        Ok(value) => value + 1000,
        Err(error) => {
            println!("app_process_test: instanceAdd original call failed: {error}");
            -1000
        }
    }
}

pub(super) unsafe extern "C" fn replacement_instance_primitive_mix(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    flag: jni::jboolean,
    value: jni::jbyte,
    letter: jni::jchar,
    extra: jni::jshort,
) -> jni::jint {
    if unsafe { replacement_receiver_matches(env, receiver) }
        && flag == jni::JNI_TRUE
        && value == 2
        && letter == b'C' as jni::jchar
        && extra == 5
    {
        4242
    } else {
        -4242
    }
}

pub(super) unsafe extern "C" fn replacement_instance_wide(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    value: jni::jlong,
    extra: jni::jdouble,
) -> jni::jlong {
    if unsafe { replacement_receiver_matches(env, receiver) }
        && value == 40
        && (extra - 2.0).abs() < 0.0001
    {
        9001
    } else {
        -9001
    }
}

pub(super) unsafe extern "C" fn replacement_instance_float_mix(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    value: jni::jfloat,
    extra: jni::jdouble,
) -> jni::jdouble {
    if unsafe { replacement_receiver_matches(env, receiver) }
        && (value - 1.5).abs() < 0.0001
        && (extra - 2.25).abs() < 0.0001
    {
        8.5
    } else {
        -8.5
    }
}

pub(super) unsafe extern "C" fn replacement_instance_string(
    _env: *mut jni::JNIEnv,
    _receiver: jni::jobject,
) -> jni::jstring {
    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

pub(super) unsafe extern "C" fn replacement_overload(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jstring,
) -> jni::jstring {
    let expected_receiver = EXPECTED_RECEIVER.load(Ordering::SeqCst);
    if expected_receiver.is_null()
        || env.is_null()
        || !unsafe { raw_is_same_object(env, receiver, expected_receiver) }
    {
        return ptr::null_mut();
    }

    if argument.is_null() {
        return REPLACEMENT_STRING.load(Ordering::SeqCst);
    }

    let expected_argument = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
    if expected_argument.is_null()
        || !unsafe { raw_is_same_object(env, argument, expected_argument) }
    {
        return ptr::null_mut();
    }

    REPLACEMENT_STRING.load(Ordering::SeqCst)
}

pub(super) unsafe extern "C" fn replacement_overload_calling_original(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jstring,
) -> jni::jstring {
    let expected_receiver = EXPECTED_RECEIVER.load(Ordering::SeqCst);
    if expected_receiver.is_null()
        || env.is_null()
        || !unsafe { raw_is_same_object(env, receiver, expected_receiver) }
    {
        return ptr::null_mut();
    }

    let arg = if argument.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(argument)
    };
    match unsafe {
        experimental::call_original_instance_method(
            env,
            receiver,
            "overload",
            "(Ljava/lang/String;)Ljava/lang/String;",
            [arg],
        )
    } {
        Ok(experimental::RawJavaReturn::Object(value)) => {
            let expected_argument = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
            if argument.is_null() {
                if value.is_null() {
                    REPLACEMENT_STRING.load(Ordering::SeqCst)
                } else {
                    ptr::null_mut()
                }
            } else if !expected_argument.is_null()
                && !value.is_null()
                && unsafe { raw_is_same_object(env, value, expected_argument) }
            {
                REPLACEMENT_STRING.load(Ordering::SeqCst)
            } else {
                ptr::null_mut()
            }
        }
        Ok(other) => {
            println!("app_process_test: overload original returned {other:?}");
            ptr::null_mut()
        }
        Err(error) => {
            println!("app_process_test: overload original call failed: {error}");
            ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn replacement_instance_object_echo(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jobject,
) -> jni::jobject {
    if unsafe { replacement_receiver_matches(env, receiver) }
        && unsafe { replacement_argument_matches(env, argument) }
    {
        REPLACEMENT_OBJECT.load(Ordering::SeqCst)
    } else {
        ptr::null_mut()
    }
}

pub(super) unsafe extern "C" fn replacement_instance_object_array_echo(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jobject,
) -> jni::jobject {
    unsafe { replacement_instance_object_echo(env, receiver, argument) }
}

pub(super) unsafe extern "C" fn replacement_instance_subject_echo_calling_original(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jobject,
) -> jni::jobject {
    if !unsafe { replacement_receiver_matches(env, receiver) } {
        return ptr::null_mut();
    }

    let arg = if argument.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(argument)
    };
    match unsafe {
        experimental::call_original_instance_method(
            env,
            receiver,
            "subjectEcho",
            "(Lfrida/java/bridge/rs/test/TestSubject;)Lfrida/java/bridge/rs/test/TestSubject;",
            [arg],
        )
    } {
        Ok(experimental::RawJavaReturn::Object(value))
            if unsafe { replacement_argument_matches(env, value) } =>
        {
            REPLACEMENT_OBJECT.load(Ordering::SeqCst)
        }
        Ok(other) => {
            println!("app_process_test: subjectEcho original returned {other:?}");
            ptr::null_mut()
        }
        Err(error) => {
            println!("app_process_test: subjectEcho original call failed: {error}");
            ptr::null_mut()
        }
    }
}

pub(super) unsafe extern "C" fn replacement_instance_object_array_echo_calling_original(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    argument: jni::jobject,
) -> jni::jobject {
    if !unsafe { replacement_receiver_matches(env, receiver) } {
        return ptr::null_mut();
    }
    let Some(original) = INSTANCE_OBJECT_ARRAY_ECHO_ORIGINAL.get() else {
        return ptr::null_mut();
    };

    let arg = if argument.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(argument)
    };
    match unsafe { original.call_instance(env, receiver, (arg,)) }
        .and_then(|value| value.into_object("objectArrayEcho original call"))
    {
        Ok(value) if unsafe { replacement_argument_matches(env, value) } => {
            REPLACEMENT_OBJECT.load(Ordering::SeqCst)
        }
        Ok(_) => {
            println!("app_process_test: objectArrayEcho original returned unexpected object");
            ptr::null_mut()
        }
        Err(error) => {
            println!("app_process_test: objectArrayEcho original call failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe fn replacement_argument_matches(env: *mut jni::JNIEnv, argument: jni::jobject) -> bool {
    let expected = EXPECTED_ARGUMENT.load(Ordering::SeqCst);
    if env.is_null() {
        return false;
    }
    if expected.is_null() {
        argument.is_null()
    } else {
        !argument.is_null() && unsafe { raw_is_same_object(env, argument, expected) }
    }
}

unsafe fn replacement_receiver_matches(env: *mut jni::JNIEnv, receiver: jni::jobject) -> bool {
    let expected = EXPECTED_RECEIVER.load(Ordering::SeqCst);
    !env.is_null()
        && !receiver.is_null()
        && !expected.is_null()
        && unsafe { raw_is_same_object(env, receiver, expected) }
}

unsafe fn raw_is_same_object(
    env: *mut jni::JNIEnv,
    left: jni::jobject,
    right: jni::jobject,
) -> bool {
    let env = unsafe { NonNull::new_unchecked(env) };
    let is_same_object =
        unsafe { jni::env_function::<jni::IsSameObject>(env, jni::ENV_IS_SAME_OBJECT) };
    unsafe { is_same_object(env.as_ptr(), left, right) == jni::JNI_TRUE }
}
