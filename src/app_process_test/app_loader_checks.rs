use super::assertions::*;
use super::*;

pub(super) fn check_app_loader_surface(java: &Java, app_java: &Java) -> Result<()> {
    println!("app_process_test: checking app-loader class and wrapper surface");
    let capabilities = java.capabilities();
    let heap_enumeration_available = capabilities.heap_enumeration.is_supported();
    if app_java.loader().is_none() {
        return test_error("app-loader Java unexpectedly lost its loader");
    }

    let subject = check_class_loader_lookup_and_raw_wrapper_basics(java, app_java)?;
    let test_wrapper = app_java.use_class(TEST_SUBJECT)?;
    let wrapper_surface =
        check_constructor_overload_field_binding_and_cast_surface(java, &test_wrapper)?;
    check_field_reads_writes_and_coercions(
        &test_wrapper,
        &wrapper_surface.numbered_object,
        &wrapper_surface.number_field,
    )?;
    check_heap_instance_enumeration(
        app_java,
        &test_wrapper,
        &wrapper_surface.int_constructor,
        &wrapper_surface.number_field,
        heap_enumeration_available,
    )?;
    let method_surface = check_method_dispatch_overloads_and_argument_errors(
        java,
        app_java,
        &test_wrapper,
        &wrapper_surface.test_object,
    )?;
    check_java_array_ergonomics(
        app_java,
        &subject,
        &test_wrapper,
        &wrapper_surface.test_object,
        &method_surface,
    )?;
    Ok(())
}

struct WrapperSurface {
    test_object: JavaObject,
    numbered_object: JavaObject,
    int_constructor: JavaConstructor,
    number_field: JavaField,
}

struct MethodSurface {
    static_object_echo: JavaMethod,
}

fn check_class_loader_lookup_and_raw_wrapper_basics(
    java: &Java,
    app_java: &Java,
) -> Result<raw::Class> {
    let subject = app_java.find_class(TEST_SUBJECT)?;
    let cached_subject = app_java.find_class(TEST_SUBJECT)?;
    if cached_subject.name() != TEST_SUBJECT {
        return test_error(format!(
            "cached TestSubject class name mismatch: {}",
            cached_subject.name()
        ));
    }
    let misleading_loader_class = app_java.find_class(MISLEADING_CLASS_LOADER)?;
    let misleading_loader_object = misleading_loader_class.new_object("()V", &[])?;
    let misleading_loader = java.class_loader_from_object(&misleading_loader_object)?;
    let misleading_java = java.with_loader(&misleading_loader);
    match misleading_java.find_class(TEST_SUBJECT) {
        Err(Error::ClassLookupMismatch { requested, actual })
            if requested == TEST_SUBJECT && actual == "java.lang.String" => {}
        Err(error) => return Err(error),
        Ok(class) => {
            return test_error(format!(
                "misleading ClassLoader cached {} for requested {TEST_SUBJECT}",
                class.name()
            ));
        }
    }
    let answer_return = subject.call_static("answer", "()I", &[])?;
    if answer_return.java_display()? != "42" {
        return test_error(format!(
            "JavaReturn TestSubject.answer display mismatch: {}",
            answer_return.java_display()?
        ));
    }
    let answer = read_int(answer_return, "TestSubject.answer")?;
    if answer != 42 {
        return test_error(format!("TestSubject.answer mismatch: {answer}"));
    }
    let test_object = subject.new_object("()V", &[])?;
    let object_display = test_object.java_display()?;
    if !object_display.contains("frida.rust.java.bridge.test.TestSubject@") {
        return test_error(format!("JavaObject display mismatch: {object_display}"));
    }
    let message_return =
        subject.call_method(&test_object, "message", "()Ljava/lang/String;", &[])?;
    if message_return.java_display()? != "dex-test" {
        return test_error(format!(
            "JavaReturn TestSubject.message display mismatch: {}",
            message_return.java_display()?
        ));
    }
    let message = read_object(message_return, "TestSubject.message")?
        .ok_or_else(|| test_failure("TestSubject.message unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "dex-test" {
        return test_error(format!("TestSubject.message mismatch: {message:?}"));
    }

    Ok(subject)
}

fn check_constructor_overload_field_binding_and_cast_surface(
    java: &Java,
    test_wrapper: &JavaClass,
) -> Result<WrapperSurface> {
    if test_wrapper.java_display() != "<class: frida.rust.java.bridge.test.TestSubject>" {
        return test_error(format!(
            "JavaClass display mismatch: {}",
            test_wrapper.java_display()
        ));
    }
    if !test_wrapper
        .constructors()?
        .iter()
        .any(|method| method.signature.to_string() == "()V")
    {
        return test_error("JavaClass TestSubject default constructor was not found");
    }
    let answer = test_wrapper.call::<jni::jint>("answer", ())?;
    if answer != 42 {
        return test_error(format!("JavaClass TestSubject.answer mismatch: {answer}"));
    }
    let test_object = test_wrapper.constructor([])?.new_object(())?;
    let message_overload = test_wrapper.method("message")?.overload([] as [&str; 0])?;
    let message = read_object(
        message_overload.call_raw(&test_object, ())?,
        "JavaClass TestSubject.message",
    )?
    .ok_or_else(|| test_failure("JavaClass TestSubject.message unexpectedly returned null"))?;
    let message = message.get_string()?;
    if message != "dex-test" {
        return test_error(format!(
            "JavaClass TestSubject.message mismatch: {message:?}"
        ));
    }

    let wrapper_methods = test_wrapper.declared_methods()?;
    require_method(
        &wrapper_methods,
        "message",
        MethodKind::Instance,
        "()Ljava/lang/String;",
        "JavaClass declared TestSubject.message",
    )?;
    let wrapper_fields = test_wrapper.declared_fields()?;
    require_field(
        &wrapper_fields,
        "number",
        FieldKind::Instance,
        &JavaType::Int,
        "JavaClass declared TestSubject.number",
    )?;
    if !test_wrapper.is_instance(&test_object)? {
        return test_error("JavaClass TestSubject did not recognize its instance");
    }
    let object_wrapper = java.use_class("java.lang.Object")?;
    if !object_wrapper.is_instance(&test_object)? {
        return test_error("JavaClass Object did not recognize TestSubject instance");
    }
    let plain_object = object_wrapper.new(())?;
    if !object_wrapper.is_instance(&plain_object)? {
        return test_error("JavaClass::new Object instance mismatch");
    }
    let dispatch_object = test_wrapper.new(())?;
    let dispatch_message = read_object(
        message_overload.call_raw(&dispatch_object, ())?,
        "JavaClass dispatch constructor TestSubject.message",
    )?
    .ok_or_else(|| test_failure("JavaClass dispatch constructor message null"))?
    .get_string()?;
    if dispatch_message != "dex-test" {
        return test_error(format!(
            "JavaClass dispatch constructor message mismatch: {dispatch_message:?}"
        ));
    }
    let retained_object = test_object.cast(&object_wrapper)?;
    if retained_object.class().name() != "java.lang.Object" {
        return test_error(format!(
            "JavaObject::cast wrapper class mismatch: {}",
            retained_object.class().name()
        ));
    }
    let _ = retained_object.call::<jni::jint>("hashCode", ())?;

    println!("app_process_test: checking app-loader overload handles");
    let default_constructor = test_wrapper.constructor_by_types(&[])?;
    if default_constructor.signature().to_string() != "()V" {
        return test_error(format!(
            "JavaConstructor default signature mismatch: {}",
            default_constructor.signature()
        ));
    }
    if default_constructor.java_display()
        != "function frida.rust.java.bridge.test.TestSubject.<init>()V"
    {
        return test_error(format!(
            "JavaConstructor display mismatch: {}",
            default_constructor.java_display()
        ));
    }
    let test_object = default_constructor.new_object(())?;
    if test_object.class().name() != TEST_SUBJECT {
        return test_error(format!(
            "JavaConstructor wrapper class mismatch: {}",
            test_object.class().name()
        ));
    }
    let constructor_alias_object = test_wrapper.constructor([])?.new_object(())?;
    let alias_message = read_object(
        message_overload.call_raw(&constructor_alias_object, ())?,
        "JavaClass constructor alias TestSubject.message",
    )?
    .ok_or_else(|| test_failure("JavaClass constructor alias message null"))?;
    let alias_message = alias_message.get_string()?;
    if alias_message != "dex-test" {
        return test_error(format!(
            "JavaClass constructor alias message mismatch: {alias_message:?}"
        ));
    }
    let int_constructor = test_wrapper.constructor(["int"])?;
    let numbered_object = int_constructor.new_object((31 as jni::jint,))?;
    let alias_numbered_object = test_wrapper.new_with(["int"], (31 as jni::jint,))?;
    let dispatch_numbered_object = test_wrapper.new(31 as jni::jint)?;
    let number_field = test_wrapper.field("number")?;
    if number_field.java_display() != "field frida.rust.java.bridge.test.TestSubject.number: I" {
        return test_error(format!(
            "JavaField display mismatch: {}",
            number_field.java_display()
        ));
    }
    let number = number_field.get_int(&numbered_object)?;
    if number != 31 {
        return test_error(format!("JavaField TestSubject.number mismatch: {number}"));
    }
    let typed_number = number_field.get::<jni::jint>(&numbered_object)?;
    if typed_number != 31 {
        return test_error(format!(
            "typed JavaField TestSubject.number mismatch: {typed_number}"
        ));
    }
    let alias_number = number_field.get_int(&alias_numbered_object)?;
    if alias_number != 31 {
        return test_error(format!(
            "JavaClass new_with TestSubject.number mismatch: {alias_number}"
        ));
    }
    let dispatch_number = number_field.get_int(&dispatch_numbered_object)?;
    if dispatch_number != 31 {
        return test_error(format!(
            "JavaClass dispatch constructor TestSubject.number mismatch: {dispatch_number}"
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
    Ok(WrapperSurface {
        test_object,
        numbered_object,
        int_constructor,
        number_field,
    })
}

fn check_field_reads_writes_and_coercions(
    test_wrapper: &JavaClass,
    numbered_object: &JavaObject,
    number_field: &JavaField,
) -> Result<()> {
    number_field.set_int(numbered_object, 37)?;
    let number = number_field.get_int(numbered_object)?;
    if number != 37 {
        return test_error(format!(
            "JavaField TestSubject.number after set mismatch: {number}"
        ));
    }
    number_field.set(numbered_object, 38 as jni::jint)?;
    let number = number_field.get::<jni::jint>(numbered_object)?;
    if number != 38 {
        return test_error(format!(
            "generic JavaField TestSubject.number after set mismatch: {number}"
        ));
    }
    numbered_object.field("number")?.set(39 as jni::jint)?;
    let number = numbered_object.field("number")?.get::<jni::jint>()?;
    if number != 39 {
        return test_error(format!(
            "JavaBoundFieldHandle TestSubject.number mismatch: {number}"
        ));
    }
    let flag_field = test_wrapper.field("flag")?;
    if !flag_field.get_boolean(numbered_object)? {
        return test_error("JavaField TestSubject.flag mismatch");
    }
    flag_field.set_boolean(numbered_object, false)?;
    if flag_field.get_boolean(numbered_object)? {
        return test_error("JavaField TestSubject.flag after set mismatch");
    }
    let small_field = test_wrapper.field("small")?;
    if small_field.get_byte(numbered_object)? != 2 {
        return test_error("JavaField TestSubject.small mismatch");
    }
    small_field.set_byte(numbered_object, 3)?;
    if small_field.get_byte(numbered_object)? != 3 {
        return test_error("JavaField TestSubject.small after set mismatch");
    }
    let letter_field = test_wrapper.field("letter")?;
    if letter_field.get_char(numbered_object)? != 'C' as jni::jchar {
        return test_error("JavaField TestSubject.letter mismatch");
    }
    letter_field.set_char(numbered_object, 'D' as jni::jchar)?;
    if letter_field.get_char(numbered_object)? != 'D' as jni::jchar {
        return test_error("JavaField TestSubject.letter after set mismatch");
    }
    let short_field = test_wrapper.field("shortNumber")?;
    if short_field.get_short(numbered_object)? != 123 {
        return test_error("JavaField TestSubject.shortNumber mismatch");
    }
    short_field.set_short(numbered_object, 124)?;
    if short_field.get_short(numbered_object)? != 124 {
        return test_error("JavaField TestSubject.shortNumber after set mismatch");
    }
    let wide_field = test_wrapper.field("wideNumber")?;
    if wide_field.get_long(numbered_object)? != 1000 {
        return test_error("JavaField TestSubject.wideNumber mismatch");
    }
    wide_field.set_long(numbered_object, 1001)?;
    if wide_field.get_long(numbered_object)? != 1001 {
        return test_error("JavaField TestSubject.wideNumber after set mismatch");
    }
    let ratio_field = test_wrapper.field("ratio")?;
    if (ratio_field.get_float(numbered_object)? - 1.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.ratio mismatch");
    }
    ratio_field.set_float(numbered_object, 2.5)?;
    if (ratio_field.get_float(numbered_object)? - 2.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.ratio after set mismatch");
    }
    let precise_field = test_wrapper.field("precise")?;
    if (precise_field.get_double(numbered_object)? - 2.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.precise mismatch");
    }
    precise_field.set_double(numbered_object, 3.5)?;
    if (precise_field.get_double(numbered_object)? - 3.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.precise after set mismatch");
    }

    let static_flag_field = test_wrapper.field("staticFlag")?;
    if !static_flag_field.get::<bool>(())? {
        return test_error("JavaField TestSubject.staticFlag mismatch");
    }
    static_flag_field.set((), false)?;
    if test_wrapper.get_field::<bool>("staticFlag")? {
        return test_error("JavaField TestSubject.staticFlag after set mismatch");
    }
    numbered_object.field("staticFlag")?.set(true)?;
    if !numbered_object.field("staticFlag")?.get::<bool>()? {
        return test_error("JavaObject field TestSubject.staticFlag after set mismatch");
    }
    let shadowed_gear_field = test_wrapper.field("shadowedNumber")?;
    if shadowed_gear_field.kind() != FieldKind::Static {
        return test_error(format!(
            "JavaField TestSubject.shadowedNumber selected {:?}, expected static",
            shadowed_gear_field.kind()
        ));
    }
    if shadowed_gear_field.get::<jni::jint>(())? != 29 {
        return test_error("JavaField TestSubject.shadowedNumber static value mismatch");
    }
    numbered_object
        .field("shadowedNumber")?
        .set(30 as jni::jint)?;
    if shadowed_gear_field.get::<jni::jint>(())? != 30 {
        return test_error("JavaObject field TestSubject.shadowedNumber static set mismatch");
    }
    let shadowed_static_field = test_wrapper.field("shadowedStaticField")?;
    if shadowed_static_field.kind() != FieldKind::Instance {
        return test_error(format!(
            "JavaField TestSubject.shadowedStaticField selected {:?}, expected instance",
            shadowed_static_field.kind()
        ));
    }
    if shadowed_static_field.get::<jni::jint>(numbered_object)? != 73 {
        return test_error("JavaField TestSubject.shadowedStaticField instance value mismatch");
    }
    let inherited_static_number = test_wrapper.field("inheritedStaticNumber")?;
    if inherited_static_number.kind() != FieldKind::Static {
        return test_error(format!(
            "JavaField TestSubject.inheritedStaticNumber selected {:?}, expected static",
            inherited_static_number.kind()
        ));
    }
    if inherited_static_number.get::<jni::jint>(())? != 61 {
        return test_error("JavaField TestSubject.inheritedStaticNumber value mismatch");
    }
    inherited_static_number.set((), 62 as jni::jint)?;
    if inherited_static_number.get::<jni::jint>(())? != 62 {
        return test_error("JavaField TestSubject.inheritedStaticNumber after set mismatch");
    }
    if test_wrapper.get_field::<jni::jbyte>("staticSmall")? != 2 {
        return test_error("JavaField TestSubject.staticSmall mismatch");
    }
    test_wrapper.set_field("staticSmall", 3 as jni::jbyte)?;
    if test_wrapper.get_field::<jni::jbyte>("staticSmall")? != 3 {
        return test_error("JavaField TestSubject.staticSmall after set mismatch");
    }
    test_wrapper.set_field("staticSmall", 4)?;
    if test_wrapper.get_field::<jni::jbyte>("staticSmall")? != 4 {
        return test_error("JavaField TestSubject.staticSmall after int coercion mismatch");
    }
    match test_wrapper.set_field("staticSmall", 128) {
        Err(Error::InvalidFieldValue {
            operation: "JavaField::set",
            expected,
            actual,
        }) if expected == "B" && actual == "int 128 outside byte range" => {}
        Err(error) => return Err(error),
        Ok(_) => return test_error("JavaField TestSubject.staticSmall accepted out-of-range int"),
    }
    if test_wrapper.get_field::<jni::jchar>("staticLetter")? != 'C' as jni::jchar {
        return test_error("JavaField TestSubject.staticLetter mismatch");
    }
    test_wrapper.set_field("staticLetter", 'D' as jni::jchar)?;
    if test_wrapper.get_field::<jni::jchar>("staticLetter")? != 'D' as jni::jchar {
        return test_error("JavaField TestSubject.staticLetter after set mismatch");
    }
    test_wrapper.set_field("staticLetter", 69)?;
    if test_wrapper.get_field::<jni::jchar>("staticLetter")? != 'E' as jni::jchar {
        return test_error("JavaField TestSubject.staticLetter after int coercion mismatch");
    }
    if test_wrapper.get_field::<jni::jshort>("staticShortNumber")? != 123 {
        return test_error("JavaField TestSubject.staticShortNumber mismatch");
    }
    test_wrapper.set_field("staticShortNumber", 124 as jni::jshort)?;
    if test_wrapper.get_field::<jni::jshort>("staticShortNumber")? != 124 {
        return test_error("JavaField TestSubject.staticShortNumber after set mismatch");
    }
    test_wrapper.set_field("staticShortNumber", 125)?;
    if test_wrapper.get_field::<jni::jshort>("staticShortNumber")? != 125 {
        return test_error("JavaField TestSubject.staticShortNumber after int coercion mismatch");
    }
    if test_wrapper.get_field::<jni::jlong>("staticWideNumber")? != 1000 {
        return test_error("JavaField TestSubject.staticWideNumber mismatch");
    }
    test_wrapper.set_field("staticWideNumber", 1001 as jni::jlong)?;
    if test_wrapper.get_field::<jni::jlong>("staticWideNumber")? != 1001 {
        return test_error("JavaField TestSubject.staticWideNumber after set mismatch");
    }
    test_wrapper.set_field("staticWideNumber", 1002)?;
    if test_wrapper.get_field::<jni::jlong>("staticWideNumber")? != 1002 {
        return test_error("JavaField TestSubject.staticWideNumber after int coercion mismatch");
    }
    if (test_wrapper.get_field::<jni::jfloat>("staticRatio")? - 1.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.staticRatio mismatch");
    }
    test_wrapper.set_field("staticRatio", 2.5_f32)?;
    if (test_wrapper.get_field::<jni::jfloat>("staticRatio")? - 2.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.staticRatio after set mismatch");
    }
    test_wrapper.set_field("staticRatio", 2.75_f64)?;
    if (test_wrapper.get_field::<jni::jfloat>("staticRatio")? - 2.75).abs() > 0.0001 {
        return test_error("JavaField TestSubject.staticRatio after double coercion mismatch");
    }
    if (test_wrapper.get_field::<jni::jdouble>("staticPrecise")? - 2.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.staticPrecise mismatch");
    }
    test_wrapper.set_field("staticPrecise", 3.5)?;
    if (test_wrapper.get_field::<jni::jdouble>("staticPrecise")? - 3.5).abs() > 0.0001 {
        return test_error("JavaField TestSubject.staticPrecise after set mismatch");
    }
    test_wrapper.set_field("staticPrecise", 3.75_f32)?;
    if (test_wrapper.get_field::<jni::jdouble>("staticPrecise")? - 3.75).abs() > 0.0001 {
        return test_error("JavaField TestSubject.staticPrecise after float coercion mismatch");
    }

    Ok(())
}

fn check_heap_instance_enumeration(
    app_java: &Java,
    test_wrapper: &JavaClass,
    int_constructor: &JavaConstructor,
    number_field: &JavaField,
    heap_enumeration_available: bool,
) -> Result<()> {
    println!("app_process_test: checking heap instance enumeration capability");
    let heap_subject_a = int_constructor.new_object((8101 as jni::jint,))?;
    let heap_subject_b = int_constructor.new_object((8102 as jni::jint,))?;
    if heap_enumeration_available {
        let mut numbers = Vec::new();
        app_java.choose_instances(TEST_SUBJECT, |object| {
            if object.class().name() != TEST_SUBJECT {
                return test_error(format!(
                    "heap enumeration selected class mismatch: {}",
                    object.class().name()
                ));
            }
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

    Ok(())
}

fn check_method_dispatch_overloads_and_argument_errors(
    java: &Java,
    app_java: &Java,
    test_wrapper: &JavaClass,
    test_object: &JavaObject,
) -> Result<MethodSurface> {
    let answer_overload = test_wrapper.method("answer")?.overload([] as [&str; 0])?;
    if answer_overload.java_display()
        != "function frida.rust.java.bridge.test.TestSubject.answer()I"
    {
        return test_error(format!(
            "JavaMethod display mismatch: {}",
            answer_overload.java_display()
        ));
    }
    let answer = answer_overload.call::<jni::jint>((), ())?;
    if answer != 42 {
        return test_error(format!("JavaMethod TestSubject.answer mismatch: {answer}"));
    }
    let typed_answer = answer_overload.call::<jni::jint>((), ())?;
    if typed_answer != 42 {
        return test_error(format!(
            "typed JavaMethod TestSubject.answer mismatch: {typed_answer}"
        ));
    }
    let raw_answer = answer_overload
        .call_raw((), ())?
        .into_int("JavaMethod::call_raw answer")?;
    if raw_answer != 42 {
        return test_error(format!(
            "raw JavaMethod TestSubject.answer mismatch: {raw_answer}"
        ));
    }
    let selected_answer = test_wrapper.call::<jni::jint>("answer", ())?;
    if selected_answer != 42 {
        return test_error(format!(
            "selector JavaMethod TestSubject.answer mismatch: {selected_answer}"
        ));
    }
    let message_overload = test_wrapper.method("message")?.overload([] as [&str; 0])?;
    let message = message_overload
        .call_string(test_object, ())?
        .ok_or_else(|| test_failure("JavaMethod TestSubject.message unexpectedly null"))?;
    if message != "dex-test" {
        return test_error(format!(
            "JavaMethod TestSubject.message mismatch: {message:?}"
        ));
    }
    let typed_message = message_overload.call::<String>(test_object, ())?;
    if typed_message != "dex-test" {
        return test_error(format!(
            "typed JavaMethod TestSubject.message mismatch: {typed_message:?}"
        ));
    }
    let answer_method = test_wrapper.method("answer")?.overload([] as [&str; 0])?;
    if answer_method.kind() != MethodKind::Static
        || answer_method.name() != "answer"
        || answer_method.signature().to_string() != "()I"
    {
        return test_error("JavaMethod TestSubject.answer metadata mismatch");
    }
    let answer = answer_method.call::<jni::jint>((), ())?;
    if answer != 42 {
        return test_error(format!("JavaMethod TestSubject.answer mismatch: {answer}"));
    }
    let answer = answer_method.call::<jni::jint>((), ())?;
    if answer != 42 {
        return test_error(format!(
            "typed JavaMethod TestSubject.answer mismatch: {answer}"
        ));
    }
    test_wrapper.call::<()>("resetVoidCounter", ())?;
    test_wrapper.call::<()>("bumpVoidCounter", ())?;
    let void_counter = test_wrapper.call::<jni::jint>("voidCounter", ())?;
    if void_counter != 1 {
        return test_error(format!(
            "typed void JavaMethod counter mismatch: {void_counter}"
        ));
    }
    let message_method = test_wrapper.method("message")?.overload([] as [&str; 0])?;
    if message_method.kind() != MethodKind::Instance
        || message_method.name() != "message"
        || message_method.signature().to_string() != "()Ljava/lang/String;"
    {
        return test_error("JavaMethod TestSubject.message metadata mismatch");
    }
    let message = message_method
        .call_string(test_object, ())?
        .ok_or_else(|| test_failure("JavaMethod TestSubject.message unexpectedly null"))?;
    if message != "dex-test" {
        return test_error(format!(
            "JavaMethod TestSubject.message mismatch: {message:?}"
        ));
    }
    let message = message_method.call::<String>(test_object, ())?;
    if message != "dex-test" {
        return test_error(format!(
            "typed JavaMethod TestSubject.message mismatch: {message:?}"
        ));
    }
    let cast_subject = test_object.cast(test_wrapper)?;
    let bound_message = cast_subject.call::<String>("message", ())?;
    if bound_message != "dex-test" {
        return test_error(format!(
            "JavaObject cast TestSubject.message mismatch: {bound_message:?}"
        ));
    }
    let runtime_class = test_object.runtime_class()?;
    if runtime_class.name() != TEST_SUBJECT {
        return test_error(format!(
            "JavaObject runtime_class name mismatch: {}",
            runtime_class.name()
        ));
    }
    let direct_message = test_object.call::<String>("message", ())?;
    if direct_message != "dex-test" {
        return test_error(format!(
            "JavaObject direct TestSubject.message mismatch: {direct_message:?}"
        ));
    }
    let inherited_message = test_object.call::<String>("inheritedMessage", ())?;
    if inherited_message != "base-message" {
        return test_error(format!(
            "JavaObject inherited TestSubjectBase.inheritedMessage mismatch: {inherited_message:?}"
        ));
    }
    let inherited_message_by_arity =
        test_object.call_with::<String>("inheritedMessage", [] as [&str; 0], ())?;
    if inherited_message_by_arity != "base-message" {
        return test_error(format!(
            "JavaClass arity-selected inherited TestSubjectBase.inheritedMessage mismatch: {inherited_message_by_arity:?}"
        ));
    }
    let inherited_static_answer = test_wrapper.call::<jni::jint>("inheritedStaticAnswer", ())?;
    if inherited_static_answer != 515 {
        return test_error(format!(
            "JavaClass inherited static TestSubjectBase.inheritedStaticAnswer mismatch: {inherited_static_answer}"
        ));
    }
    let shadowed_message = test_object.call::<String>("shadowedMessage", ())?;
    if shadowed_message != "child-shadowed" {
        return test_error(format!(
            "JavaObject shadowed TestSubject.shadowedMessage mismatch: {shadowed_message:?}"
        ));
    }
    match test_object.call_with::<String>("shadowedMessage", ["int"], 7) {
        Err(Error::OverloadNotFound {
            class,
            kind: "method",
            name,
            arguments,
        }) if class == TEST_SUBJECT && name == "shadowedMessage" && arguments == "(I)" => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "shadowed superclass overload unexpectedly resolved: {value:?}"
            ));
        }
    }
    test_object.set_field("inheritedNumber", 22 as jni::jint)?;
    let inherited_number = test_object.get_field::<jni::jint>("inheritedNumber")?;
    if inherited_number != 22 {
        return test_error(format!(
            "JavaObject inherited TestSubjectBase.inheritedNumber mismatch: {inherited_number}"
        ));
    }
    let not_subject = app_java.new_string_utf("not-subject")?;
    match not_subject.cast(test_wrapper) {
        Err(Error::InvalidObjectType {
            operation: "JavaClass::cast",
            ..
        }) => {}
        Err(error) => return Err(error),
        Ok(_) => return test_error("JavaObject::cast accepted a non-TestSubject object"),
    }
    let instance_number = test_wrapper
        .method("instanceNumber")?
        .overload([] as [&str; 0])?;
    match instance_number.call_int(&not_subject, ()) {
        Err(Error::InvalidObjectType {
            operation: "JavaMethod::call receiver",
            ..
        }) => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "JavaMethod TestSubject.instanceNumber accepted non-TestSubject receiver: {value}"
            ));
        }
    }
    let number_field = test_wrapper.field("number")?;
    match number_field.get_int(&not_subject) {
        Err(Error::InvalidObjectType {
            operation: "JavaField::get receiver",
            ..
        }) => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "JavaField TestSubject.number get accepted non-TestSubject receiver: {value}"
            ));
        }
    }
    match number_field.set_int(&not_subject, 23) {
        Err(Error::InvalidObjectType {
            operation: "JavaField::set receiver",
            ..
        }) => {}
        Err(error) => return Err(error),
        Ok(()) => {
            return test_error(
                "JavaField TestSubject.number set accepted non-TestSubject receiver",
            );
        }
    }
    let static_echo_string = test_wrapper
        .method("staticEcho")?
        .overload(["java.lang.String"])?;
    match static_echo_string.call::<String>((), (test_object,)) {
        Err(Error::InvalidArgumentType {
            index: 0,
            expected,
            actual: "object",
        }) if expected == "Ljava/lang/String;" => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "JavaMethod TestSubject.staticEcho(String) accepted TestSubject argument: {value:?}"
            ));
        }
    }
    let runtime_exception_wrapper = java.use_class("java.lang.RuntimeException")?;
    let runtime_exception_string_ctor =
        runtime_exception_wrapper.constructor(["java.lang.String"])?;
    match runtime_exception_string_ctor.new_object((test_object,)) {
        Err(Error::InvalidArgumentType {
            index: 0,
            expected,
            actual: "object",
        }) if expected == "Ljava/lang/String;" => {}
        Err(error) => return Err(error),
        Ok(_) => {
            return test_error(
                "RuntimeException(String) accepted TestSubject constructor argument",
            );
        }
    }
    let subject_value_field = test_wrapper.field("subjectValue")?;
    subject_value_field.set_object(test_object, Some(test_object))?;
    match subject_value_field.set_object(test_object, Some(&not_subject)) {
        Err(Error::InvalidFieldValueType {
            operation: "JavaField::set",
            expected,
            actual: "object",
        }) if expected == format!("L{};", TEST_SUBJECT.replace('.', "/")) => {}
        Err(error) => return Err(error),
        Ok(()) => {
            return test_error("JavaField TestSubject.subjectValue accepted String field value");
        }
    }
    let value = test_object.call::<String>("overload", ())?;
    if value != "no-args" {
        return test_error(format!(
            "dispatch JavaMethod TestSubject.overload() mismatch: {value:?}"
        ));
    }
    let value = test_object.call_with::<String>("overload", ["java.lang.String"], "typed")?;
    if value != "typed" {
        return test_error(format!(
            "JavaMethod TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let overload_string = test_wrapper
        .method("overload")?
        .overload(["java.lang.String"])?;
    let value = overload_string
        .call_string(test_object, ["typed"])?
        .ok_or_else(|| test_failure("JavaMethod TestSubject.overload(String) null"))?;
    if value != "typed" {
        return test_error(format!(
            "JavaMethod TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let overload_string_from_selector = test_wrapper
        .method("overload")?
        .overload(["java.lang.String"])?;
    let value = overload_string_from_selector.call::<String>(test_object, ["typed-selector"])?;
    if value != "typed-selector" {
        return test_error(format!(
            "selector TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let value = overload_string_from_selector.call::<String>(test_object, "typed-single")?;
    if value != "typed-single" {
        return test_error(format!(
            "single-argument TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let value =
        overload_string_from_selector.call::<String>(test_object, String::from("typed-owned"))?;
    if value != "typed-owned" {
        return test_error(format!(
            "owned String TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let borrowed_string = String::from("typed-borrowed");
    let value = overload_string_from_selector.call::<String>(test_object, &borrowed_string)?;
    if value != "typed-borrowed" {
        return test_error(format!(
            "borrowed String TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let value = test_object.call_with::<jni::jint>(
        "instanceAdd",
        ["int", "int"],
        (2 as jni::jint, 3 as jni::jint),
    )?;
    if value != 12 {
        return test_error(format!(
            "arity selector TestSubject.instanceAdd mismatch: {value}"
        ));
    }
    let value = test_object.call::<String>("overload", "typed-dispatch")?;
    if value != "typed-dispatch" {
        return test_error(format!(
            "dispatch JavaMethod TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let value = test_object.call_with::<String>(
        "overload",
        ["java.lang.Object"],
        "typed-object-dispatch",
    )?;
    if value != "typed-object-dispatch" {
        return test_error(format!(
            "explicit JavaMethod TestSubject.overload(Object) mismatch: {value:?}"
        ));
    }
    let value =
        cast_subject.call_with::<String>("overload", ["java.lang.String"], ["typed-bound"])?;
    if value != "typed-bound" {
        return test_error(format!(
            "bound selector TestSubject.overload(String) mismatch: {value:?}"
        ));
    }
    let input = app_java.new_string_utf("typed-object")?;
    let value = overload_string
        .call_string(test_object, (&input,))?
        .ok_or_else(|| test_failure("JavaMethod TestSubject.overload(String object arg) null"))?;
    if value != "typed-object" {
        return test_error(format!(
            "JavaMethod TestSubject.overload(String object arg) mismatch: {value:?}"
        ));
    }
    let value =
        test_wrapper.call_with::<String>("staticEcho", ["java.lang.String"], ["typed-static"])?;
    if value != "typed-static" {
        return test_error(format!(
            "JavaMethod TestSubject.staticEcho(String) mismatch: {value:?}"
        ));
    }
    let static_object_echo = test_wrapper
        .method("staticObjectEcho")?
        .overload(["java.lang.Object"])?;
    let value = test_wrapper.call_with::<String>(
        "staticObjectEcho",
        ["java.lang.Object"],
        ["typed-object-param"],
    )?;
    if value != "typed-object-param" {
        return test_error(format!(
            "JavaMethod TestSubject.staticObjectEcho(Object) mismatch: {value:?}"
        ));
    }
    let static_object_int_sink = test_wrapper
        .method("staticObjectIntSink")?
        .overload(["java.lang.Object", "int"])?;
    test_wrapper.call::<()>("resetVoidCounter", ())?;
    static_object_int_sink.call::<()>((), ("typed-object-tuple", 7 as jni::jint))?;
    let void_counter = test_wrapper.call::<jni::jint>("voidCounter", ())?;
    if void_counter != 17 {
        return test_error(format!(
            "JavaMethod TestSubject.staticObjectIntSink Rust string tuple mismatch: {void_counter}"
        ));
    }
    for _ in 0..700 {
        match static_object_int_sink.call::<()>((), ("temporary-string", "wrong")) {
            Err(Error::InvalidArgumentType {
                index: 1,
                expected,
                actual: "string",
            }) if expected == "I" => {}
            Err(error) => return Err(error),
            Ok(()) => {
                return test_error(
                    "JavaMethod TestSubject.staticObjectIntSink accepted bad trailing string",
                );
            }
        }
    }
    static_object_int_sink.call::<()>((), ("post-error-string", 3 as jni::jint))?;
    let value = test_wrapper.call_with::<String>(
        "staticCharSequenceEcho",
        ["java.lang.CharSequence"],
        "typed-char-sequence",
    )?;
    if value != "typed-char-sequence" {
        return test_error(format!(
            "JavaMethod TestSubject.staticCharSequenceEcho(CharSequence) mismatch: {value:?}"
        ));
    }
    let instance_add = test_wrapper
        .method("instanceAdd")?
        .overload(["int", "int"])?;
    match instance_add.call_int(test_object, ["typed", "wrong"]) {
        Err(Error::InvalidArgumentType {
            index: 0,
            expected,
            actual: "string",
        }) if expected == "I" => {}
        Err(error) => return Err(error),
        Ok(value) => {
            return test_error(format!(
                "JavaMethod TestSubject.instanceAdd unexpectedly accepted string args: {value}"
            ));
        }
    }
    let static_byte_from_byte = test_wrapper
        .method("staticByteFromByte")?
        .overload(["byte"])?;
    if static_byte_from_byte.call::<jni::jbyte>((), 5)? != 6 {
        return test_error("JavaMethod staticByteFromByte int coercion mismatch");
    }
    match static_byte_from_byte.call::<jni::jbyte>((), 128) {
        Err(Error::InvalidArgumentValue {
            index: 0,
            expected,
            actual,
        }) if expected == "B" && actual == "int 128 outside byte range" => {}
        Err(error) => return Err(error),
        Ok(_) => return test_error("JavaMethod staticByteFromByte accepted out-of-range int"),
    }
    let static_char_from_char = test_wrapper
        .method("staticCharFromChar")?
        .overload(["char"])?;
    if static_char_from_char.call::<jni::jchar>((), 65)? != 'B' as jni::jchar {
        return test_error("JavaMethod staticCharFromChar int coercion mismatch");
    }
    let static_short_from_short = test_wrapper
        .method("staticShortFromShort")?
        .overload(["short"])?;
    if static_short_from_short.call::<jni::jshort>((), 32000)? != 32001 {
        return test_error("JavaMethod staticShortFromShort int coercion mismatch");
    }
    let static_wide = test_wrapper
        .method("staticWide")?
        .overload(["long", "double"])?;
    if static_wide.call::<jni::jlong>((), (40, 2.0_f64))? != 42 {
        return test_error("JavaMethod staticWide int-to-long coercion mismatch");
    }
    let static_float_from_float = test_wrapper
        .method("staticFloatFromFloat")?
        .overload(["float"])?;
    if (static_float_from_float.call::<jni::jfloat>((), 1.25_f64)? - 2.75).abs() > 0.0001 {
        return test_error("JavaMethod staticFloatFromFloat double coercion mismatch");
    }
    let static_float_mix = test_wrapper
        .method("staticFloatMix")?
        .overload(["float", "double"])?;
    if (static_float_mix.call::<jni::jdouble>((), (1.5_f64, 2.5_f32))? - 4.0).abs() > 0.0001 {
        return test_error("JavaMethod staticFloatMix float/double coercion mismatch");
    }

    Ok(MethodSurface { static_object_echo })
}

fn check_java_array_ergonomics(
    app_java: &Java,
    subject: &raw::Class,
    test_wrapper: &JavaClass,
    test_object: &JavaObject,
    method_surface: &MethodSurface,
) -> Result<()> {
    let static_object_echo = &method_surface.static_object_echo;
    println!("app_process_test: checking JavaArray ergonomics");
    let object_class = app_java.find_class("java.lang.Object")?;
    let object_array = app_java.new_object_array(&object_class, &[Some(test_object), None])?;
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
    if first.class().name() != "java.lang.Object" {
        return test_error(format!(
            "JavaArray object selected element class mismatch: {}",
            first.class().name()
        ));
    }
    let first_runtime_class = first.runtime_class()?;
    if first_runtime_class.name() != TEST_SUBJECT {
        return test_error(format!(
            "JavaArray object runtime element class mismatch: {}",
            first_runtime_class.name()
        ));
    }
    let env = app_java.vm().attach_current_thread()?;
    if !env.is_same_object(&first, test_object)? {
        return test_error("JavaArray object first element mismatch");
    }
    if object_array.get_object(1)?.is_some() {
        return test_error("JavaArray object null element unexpectedly present");
    }
    object_array.set_object(1, Some(test_object))?;
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
        .method("staticObjectArrayEcho")?
        .overload(["java.lang.Object[]"])?;
    let echoed = object_array_overload
        .call::<Option<JavaArray>>((), (&object_array,))?
        .ok_or_else(|| test_failure("JavaMethod staticObjectArrayEcho null"))?;
    if !env.is_same_object(&echoed, &object_array)? {
        return test_error("JavaMethod staticObjectArrayEcho mismatch");
    }
    let echoed = object_array_overload.call::<JavaArray>((), (&object_array,))?;
    if !env.is_same_object(&echoed, &object_array)? {
        return test_error("typed JavaMethod staticObjectArrayEcho mismatch");
    }
    let echoed = static_object_echo.call::<JavaObject>((), (test_object,))?;
    if !env.is_same_object(&echoed, test_object)? {
        return test_error("typed JavaMethod staticObjectEcho mismatch");
    }
    if echoed.class().name() != "java.lang.Object" {
        return test_error(format!(
            "JavaMethod object return wrapper class mismatch: {}",
            echoed.class().name()
        ));
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
    let echoed_return =
        subject.call_static("staticIntArrayEcho", "([I)[I", &[JavaValue::from(&ints)])?;
    let echoed_display = echoed_return.java_display()?;
    if !echoed_display.starts_with("[I@") || echoed_display == "[4, 5, 6]" {
        return test_error(format!("JavaArray display mismatch: {echoed_display}"));
    }
    let echoed = echoed_return
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
            test_object,
            "sumIntArray",
            "([I)I",
            &[JavaValue::from(&ints)],
        )?,
        "TestSubject.sumIntArray",
    )?;
    if sum != 15 {
        return test_error(format!("TestSubject.sumIntArray mismatch: {sum}"));
    }
    let int_array_overload = test_wrapper.method("intArrayEcho")?.overload(["int[]"])?;
    let echoed = int_array_overload
        .call_array(test_object, (&ints,))?
        .ok_or_else(|| test_failure("JavaMethod intArrayEcho null"))?;
    if echoed.get_ints()? != [4, 5, 6] {
        return test_error(format!(
            "JavaMethod intArrayEcho values mismatch: {:?}",
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
        .method("staticBooleanArrayEcho")?
        .overload(["boolean[]"])?
        .call::<Option<JavaArray>>((), (&booleans,))?
        .ok_or_else(|| test_failure("JavaMethod staticBooleanArrayEcho null"))?;
    if echoed.get_booleans()? != [false, true, true] {
        return test_error(format!(
            "JavaMethod staticBooleanArrayEcho mismatch: {:?}",
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
