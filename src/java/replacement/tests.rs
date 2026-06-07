use super::*;
use super::{
    closure::{
        ClosureArgumentLocation, ClosureInvocationFrame, ClosureReplacementState,
        ClosureValueLayout, ReplacementInvocation, callback_local_frame_survivor,
        closure_replacement_layout, dispatch_closure_invocation,
        validate_closure_replacement_signature,
    },
    original::{OriginalMethod, RawJavaReturn},
    original_call::prepare_original_call_args,
};
use std::{ptr, sync::Mutex};

use crate::{
    Result,
    env::MethodKind,
    error::Error,
    java::raw,
    jni,
    refs::{ClassKind, GlobalRef},
    signature::{JavaType, MethodSignature},
    value::{JavaValue, RawJavaObject},
    vm::Vm,
};

#[test]
fn original_method_captures_non_constructor_metadata_and_rejects_raw_constructor_parts() {
    let original = OriginalMethod::from_parts(MethodKind::Instance, "answer", "()I")
        .expect("instance original method should be captured");
    assert_eq!(original.kind(), MethodKind::Instance);
    assert_eq!(original.name(), "answer");
    assert_eq!(original.signature(), "()I");

    assert_eq!(
        OriginalMethod::from_parts(MethodKind::Constructor, "<init>", "()V"),
        Err(Error::WrongMethodKind {
            operation: "OriginalMethod::new",
        })
    );
}

#[test]
fn callback_local_frame_only_promotes_object_returns() {
    let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
    assert_eq!(
        callback_local_frame_survivor(RawJavaReturn::Object(object)),
        object
    );
    assert_eq!(
        callback_local_frame_survivor(RawJavaReturn::Object(ptr::null_mut())),
        ptr::null_mut()
    );

    for value in [
        RawJavaReturn::Void,
        RawJavaReturn::Boolean(jni::JNI_TRUE),
        RawJavaReturn::Byte(1),
        RawJavaReturn::Char(2),
        RawJavaReturn::Short(3),
        RawJavaReturn::Int(4),
        RawJavaReturn::Long(5),
        RawJavaReturn::Float(6.0),
        RawJavaReturn::Double(7.0),
    ] {
        assert_eq!(callback_local_frame_survivor(value), ptr::null_mut());
    }
}

fn test_closure_state<F>(signature: &str, callback: F) -> ClosureReplacementState
where
    F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
{
    test_closure_state_with_kind(MethodKind::Static, "answer", signature, callback)
}

fn test_closure_state_with_kind<F>(
    kind: MethodKind,
    name: &str,
    signature: &str,
    callback: F,
) -> ClosureReplacementState
where
    F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
{
    let vm = Vm::dangling_for_tests();
    let target_class =
        raw::Class::from_global(vm.clone(), "com.example.Subject".to_owned(), unsafe {
            GlobalRef::<ClassKind>::from_raw(vm.clone(), ptr::dangling_mut()).unwrap()
        });
    ClosureReplacementState {
        vm,
        target_class,
        kind,
        name: name.to_owned(),
        signature: MethodSignature::parse(signature).expect("test signature should parse"),
        original: (kind != MethodKind::Constructor)
            .then(|| OriginalMethod::from_parts(kind, name, signature))
            .transpose()
            .expect("test original should be captured"),
        callback: Box::new(callback),
        last_error: Mutex::new(None),
        error_handler: Mutex::new(None),
        active_invocations: Default::default(),
    }
}

#[test]
fn accepts_arbitrary_non_constructor_closure_replacement_signatures() {
    for signature in [
        "()V",
        "()Z",
        "()B",
        "()C",
        "()S",
        "()I",
        "()J",
        "()F",
        "()D",
        "()Ljava/lang/Object;",
        "()[Ljava/lang/Object;",
        "(I)I",
        "(Ljava/lang/Object;I)V",
        "(Ljava/lang/String;)Ljava/lang/String;",
        "([Ljava/lang/Object;)[Ljava/lang/Object;",
        "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
        "(Ljava/lang/Object;I)Ljava/lang/Object;",
        "(ZBCSIJFDLjava/lang/Object;[I)D",
        "(IIIIIIIIDDDDDDDDD)D",
    ] {
        validate_closure_replacement_signature(
            MethodKind::Static,
            &MethodSignature::parse(signature).unwrap(),
            "test",
        )
        .unwrap_or_else(|_| panic!("{signature} should be supported"));
    }
}

#[test]
fn lays_out_generic_closure_replacement_signatures() {
    let layout = closure_replacement_layout(
        MethodKind::Static,
        &MethodSignature::parse("(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;")
            .unwrap(),
    )
    .expect("multi-reference static layout should classify");
    assert_eq!(layout.return_value, ClosureValueLayout::Reference);
    assert_eq!(
        layout
            .arguments
            .iter()
            .map(|argument| argument.location)
            .collect::<Vec<_>>(),
        vec![
            ClosureArgumentLocation::GeneralRegister(2),
            ClosureArgumentLocation::GeneralRegister(3),
        ]
    );

    let layout = closure_replacement_layout(
        MethodKind::Instance,
        &MethodSignature::parse("(ZBCS)I").unwrap(),
    )
    .expect("mixed primitive instance layout should classify");
    assert_eq!(layout.return_value, ClosureValueLayout::General32);
    assert_eq!(
        layout
            .arguments
            .iter()
            .map(|argument| (argument.value, argument.location))
            .collect::<Vec<_>>(),
        vec![
            (
                ClosureValueLayout::General32,
                ClosureArgumentLocation::GeneralRegister(2),
            ),
            (
                ClosureValueLayout::General32,
                ClosureArgumentLocation::GeneralRegister(3),
            ),
            (
                ClosureValueLayout::General32,
                ClosureArgumentLocation::GeneralRegister(4),
            ),
            (
                ClosureValueLayout::General32,
                ClosureArgumentLocation::GeneralRegister(5),
            ),
        ]
    );

    let layout = closure_replacement_layout(
        MethodKind::Static,
        &MethodSignature::parse("(JD)J").unwrap(),
    )
    .expect("wide static layout should classify");
    assert_eq!(layout.return_value, ClosureValueLayout::General64);
    assert_eq!(
        layout
            .arguments
            .iter()
            .map(|argument| (argument.value, argument.location))
            .collect::<Vec<_>>(),
        vec![
            (
                ClosureValueLayout::General64,
                ClosureArgumentLocation::GeneralRegister(2),
            ),
            (
                ClosureValueLayout::Float64,
                ClosureArgumentLocation::FloatRegister(0),
            ),
        ]
    );
}

#[test]
fn lays_out_stack_passed_closure_replacement_arguments() {
    let layout = closure_replacement_layout(
        MethodKind::Static,
        &MethodSignature::parse("(IIIIIIIIDDDDDDDDD)D").unwrap(),
    )
    .expect("spilled mixed layout should classify");
    assert_eq!(layout.return_value, ClosureValueLayout::Float64);
    assert_eq!(
        layout
            .arguments
            .iter()
            .map(|argument| argument.location)
            .collect::<Vec<_>>(),
        vec![
            ClosureArgumentLocation::GeneralRegister(2),
            ClosureArgumentLocation::GeneralRegister(3),
            ClosureArgumentLocation::GeneralRegister(4),
            ClosureArgumentLocation::GeneralRegister(5),
            ClosureArgumentLocation::GeneralRegister(6),
            ClosureArgumentLocation::GeneralRegister(7),
            ClosureArgumentLocation::Stack { offset: 0 },
            ClosureArgumentLocation::Stack { offset: 8 },
            ClosureArgumentLocation::FloatRegister(0),
            ClosureArgumentLocation::FloatRegister(1),
            ClosureArgumentLocation::FloatRegister(2),
            ClosureArgumentLocation::FloatRegister(3),
            ClosureArgumentLocation::FloatRegister(4),
            ClosureArgumentLocation::FloatRegister(5),
            ClosureArgumentLocation::FloatRegister(6),
            ClosureArgumentLocation::FloatRegister(7),
            ClosureArgumentLocation::Stack { offset: 16 },
        ]
    );
}

#[test]
fn accepts_void_constructor_closure_replacement_signatures() {
    for signature in ["()V", "(I)V", "(Ljava/lang/Object;IZ[Ljava/lang/Object;)V"] {
        validate_closure_replacement_signature(
            MethodKind::Constructor,
            &MethodSignature::parse(signature).unwrap(),
            "test",
        )
        .unwrap_or_else(|_| panic!("constructor ABI {signature} should be supported"));
    }

    let layout = closure_replacement_layout(
        MethodKind::Constructor,
        &MethodSignature::parse("(Ljava/lang/Object;I)V").unwrap(),
    )
    .expect("constructor layout should classify");
    assert_eq!(layout.return_value, ClosureValueLayout::Void);
    assert_eq!(
        layout
            .arguments
            .iter()
            .map(|argument| argument.location)
            .collect::<Vec<_>>(),
        vec![
            ClosureArgumentLocation::GeneralRegister(2),
            ClosureArgumentLocation::GeneralRegister(3),
        ]
    );
}

#[test]
fn rejects_non_void_constructor_closure_replacement_signatures() {
    assert_eq!(
        validate_closure_replacement_signature(
            MethodKind::Constructor,
            &MethodSignature::parse("()I").unwrap(),
            "test",
        ),
        Err(Error::InvalidReplacementImplementation {
            operation: "test",
            expected: "constructor replacement descriptor returning void".to_owned(),
            actual: "non-void constructor descriptor",
        })
    );
}

#[test]
fn rejects_oversized_closure_replacement_frames() {
    let signature = format!("({})I", "I".repeat(600));
    let error = validate_closure_replacement_signature(
        MethodKind::Static,
        &MethodSignature::parse(&signature).unwrap(),
        "test",
    )
    .unwrap_err();

    let Error::InvalidReplacementImplementation {
        operation,
        expected,
        actual,
    } = error
    else {
        panic!("unexpected oversized layout error: {error:?}");
    };
    assert_eq!(operation, "test");
    assert!(expected.contains("closure replacement invocation frame"));
    assert_eq!(actual, "descriptor is too large");
}

#[test]
fn closure_state_invokes_callback_and_records_failures() {
    let state = test_closure_state("()I", |_| Ok(RawJavaReturn::Int(77)));
    assert_eq!(
        state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
        RawJavaReturn::Int(77)
    );
    assert_eq!(state.last_error(), None);

    let state = test_closure_state("()I", |_| {
        Err(Error::UnsupportedFeature {
            feature: "test closure",
            reason: "nope".to_owned(),
        })
    });
    assert_eq!(
        state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
        RawJavaReturn::Int(0)
    );
    assert!(
        state
            .last_error()
            .as_ref()
            .is_some_and(|error| error.contains("nope"))
    );
}

#[test]
fn closure_state_defaults_on_wrong_return_type() {
    let state = test_closure_state("()I", |_| Ok(RawJavaReturn::Object(ptr::null_mut())));
    assert_eq!(
        state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
        RawJavaReturn::Int(0)
    );
    assert!(
        state
            .last_error()
            .as_ref()
            .is_some_and(|error| error.contains("requires int return"))
    );
}

#[test]
fn closure_state_defaults_on_panic_and_keeps_error_until_taken() {
    let state = test_closure_state("()I", |_| panic!("intentional closure panic for test"));
    assert_eq!(
        state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
        RawJavaReturn::Int(0)
    );
    assert!(
        state
            .last_error()
            .as_ref()
            .is_some_and(|error| error.contains("panicked"))
    );

    let state = test_closure_state("()I", |_| Ok(RawJavaReturn::Int(77)));
    state.record_error("previous closure failure".to_owned());
    assert_eq!(
        state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
        RawJavaReturn::Int(77)
    );
    assert_eq!(
        state.last_error().as_deref(),
        Some("previous closure failure")
    );
    assert_eq!(
        state.take_last_error().as_deref(),
        Some("previous closure failure")
    );
    assert_eq!(state.last_error(), None);
}

#[test]
fn closure_state_reports_errors_to_handler() {
    let reported = std::sync::Arc::new(Mutex::new(Vec::new()));
    let reported_for_handler = reported.clone();
    let state = test_closure_state("()I", |_| {
        Err(Error::UnsupportedFeature {
            feature: "test closure",
            reason: "reported failure".to_owned(),
        })
    });
    state.set_error_handler(std::sync::Arc::new(move |error| {
        reported_for_handler.lock().unwrap().push(error);
    }));

    assert_eq!(
        state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
        RawJavaReturn::Int(0)
    );

    let reported = reported.lock().unwrap();
    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0].kind(), MethodKind::Static);
    assert_eq!(reported[0].name(), "answer");
    assert_eq!(reported[0].signature().to_string(), "()I");
    assert!(reported[0].message().contains("reported failure"));
    assert!(reported[0].to_string().contains("static method answer()I"));
}

#[test]
fn closure_state_passes_reference_arguments() {
    let object = ptr::dangling_mut();
    let object_addr = object as usize;
    let state = test_closure_state(
        "(Ljava/lang/Object;)Ljava/lang/Object;",
        move |invocation| {
            let object = object_addr as jni::jobject;
            assert_eq!(invocation.arguments(), &[JavaValue::object_ref(object)]);
            Ok(RawJavaReturn::Object(object))
        },
    );
    assert_eq!(
        state.invoke(
            ptr::null_mut(),
            ptr::null_mut(),
            vec![JavaValue::object_ref(object)]
        ),
        RawJavaReturn::Object(object)
    );
}

#[test]
fn closure_dispatcher_decodes_jvalue_arguments_and_writes_return() {
    let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
    let object_addr = object as usize;
    let state = test_closure_state(
        "(ZBCSIJFDLjava/lang/Object;Ljava/lang/Object;)D",
        move |invocation| {
            assert_eq!(
                invocation.arguments(),
                &[
                    JavaValue::Boolean(true),
                    JavaValue::Byte(-7),
                    JavaValue::Char(65),
                    JavaValue::Short(-9),
                    JavaValue::Int(11),
                    JavaValue::Long(13),
                    JavaValue::Float(1.25),
                    JavaValue::Double(2.5),
                    JavaValue::object_ref(object_addr as jni::jobject),
                    JavaValue::NULL,
                ]
            );
            Ok(RawJavaReturn::Double(42.5))
        },
    );
    let mut args = [
        jni::jvalue { z: jni::JNI_TRUE },
        jni::jvalue { b: -7 },
        jni::jvalue { c: 65 },
        jni::jvalue { s: -9 },
        jni::jvalue { i: 11 },
        jni::jvalue { j: 13 },
        jni::jvalue { f: 1.25 },
        jni::jvalue { d: 2.5 },
        jni::jvalue { l: object },
        jni::jvalue { l: ptr::null_mut() },
    ];
    let mut frame = ClosureInvocationFrame {
        state: &state as *const _ as *mut _,
        env: ptr::null_mut(),
        target: ptr::null_mut(),
        arguments: args.as_mut_ptr(),
        argument_count: args.len(),
        return_value: jni::jvalue { j: 0 },
    };

    unsafe { dispatch_closure_invocation(&mut frame) };

    assert_eq!(unsafe { frame.return_value.d }, 42.5);
}

#[test]
fn closure_invocation_exposes_static_and_instance_metadata() {
    let class = ptr::without_provenance_mut::<jni::_jobject>(0x1230) as jni::jclass;
    let class_addr = class as usize;
    let state = test_closure_state_with_kind(
        MethodKind::Static,
        "staticAdd",
        "(II)I",
        move |invocation| {
            assert_eq!(invocation.kind(), MethodKind::Static);
            assert_eq!(invocation.name(), "staticAdd");
            assert_eq!(invocation.signature().to_string(), "(II)I");
            assert_eq!(invocation.class(), Some(class_addr as jni::jclass));
            assert_eq!(invocation.receiver(), None);
            assert_eq!(invocation.env_raw(), ptr::null_mut());
            assert_eq!(
                invocation.arguments(),
                &[JavaValue::Int(2), JavaValue::Int(5)]
            );
            assert!(invocation.env().is_err());
            Ok(RawJavaReturn::Int(7))
        },
    );
    assert_eq!(
        state.invoke(
            ptr::null_mut(),
            class.cast(),
            vec![JavaValue::Int(2), JavaValue::Int(5)]
        ),
        RawJavaReturn::Int(7)
    );

    let receiver = ptr::without_provenance_mut::<jni::_jobject>(0x4560);
    let receiver_addr = receiver as usize;
    let state = test_closure_state_with_kind(
        MethodKind::Instance,
        "objectEcho",
        "(Ljava/lang/Object;)Ljava/lang/Object;",
        move |invocation| {
            assert_eq!(invocation.kind(), MethodKind::Instance);
            assert_eq!(invocation.name(), "objectEcho");
            assert_eq!(invocation.class(), None);
            assert_eq!(invocation.receiver(), Some(receiver_addr as jni::jobject));
            assert_eq!(invocation.arguments(), &[JavaValue::NULL]);
            Ok(RawJavaReturn::Object(ptr::null_mut()))
        },
    );
    assert_eq!(
        state.invoke(ptr::null_mut(), receiver, vec![JavaValue::NULL]),
        RawJavaReturn::Object(ptr::null_mut())
    );

    let receiver = ptr::without_provenance_mut::<jni::_jobject>(0x7890);
    let receiver_addr = receiver as usize;
    let state = test_closure_state_with_kind(
        MethodKind::Constructor,
        "<init>",
        "(I)V",
        move |invocation| {
            assert_eq!(invocation.kind(), MethodKind::Constructor);
            assert_eq!(invocation.name(), "<init>");
            assert_eq!(invocation.class(), None);
            assert_eq!(invocation.receiver(), Some(receiver_addr as jni::jobject));
            assert_eq!(invocation.arguments(), &[JavaValue::Int(31)]);
            assert_eq!(
                unsafe { invocation.call_original((31_i32,)) },
                Err(Error::WrongMethodKind {
                    operation: "ReplacementInvocation::call_original",
                })
            );
            Ok(RawJavaReturn::Void)
        },
    );
    assert_eq!(
        state.invoke(ptr::null_mut(), receiver, vec![JavaValue::Int(31)]),
        RawJavaReturn::Void
    );
}

struct BorrowedObject(jni::jobject);

impl crate::refs::sealed::JavaObjectRefSealed for BorrowedObject {
    fn as_jobject(&self) -> jni::jobject {
        self.0
    }
}

impl crate::refs::JavaObjectRef for BorrowedObject {}

#[test]
fn hook_return_converts_to_raw_values() {
    let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
    let borrowed = BorrowedObject(object);

    assert_eq!(JavaHookReturn::void().into_raw(), RawJavaReturn::Void);
    assert_eq!(
        JavaHookReturn::boolean(true).into_raw(),
        RawJavaReturn::Boolean(jni::JNI_TRUE)
    );
    assert_eq!(
        JavaHookReturn::boolean(false).into_raw(),
        RawJavaReturn::Boolean(jni::JNI_FALSE)
    );
    assert_eq!(JavaHookReturn::byte(-7).into_raw(), RawJavaReturn::Byte(-7));
    assert_eq!(JavaHookReturn::char(65).into_raw(), RawJavaReturn::Char(65));
    assert_eq!(
        JavaHookReturn::short(-9).into_raw(),
        RawJavaReturn::Short(-9)
    );
    assert_eq!(JavaHookReturn::int(11).into_raw(), RawJavaReturn::Int(11));
    assert_eq!(JavaHookReturn::long(13).into_raw(), RawJavaReturn::Long(13));
    assert_eq!(
        JavaHookReturn::float(1.25).into_raw(),
        RawJavaReturn::Float(1.25)
    );
    assert_eq!(
        JavaHookReturn::double(2.5).into_raw(),
        RawJavaReturn::Double(2.5)
    );
    assert_eq!(
        unsafe { JavaHookReturn::object(Some(&borrowed)) }.into_raw(),
        RawJavaReturn::Object(object)
    );
    assert_eq!(
        unsafe { JavaHookReturn::array(Some(&borrowed)) }.into_raw(),
        RawJavaReturn::Object(object)
    );
    assert_eq!(
        JavaHookReturn::null_object().into_raw(),
        RawJavaReturn::Object(ptr::null_mut())
    );
    assert_eq!(
        JavaHookReturn::null_array().into_raw(),
        RawJavaReturn::Object(ptr::null_mut())
    );

    assert_eq!(
        JavaHookReturn::from_raw_for_type(
            RawJavaReturn::Object(object),
            &JavaType::Array(Box::new(JavaType::Int)),
        ),
        unsafe { JavaHookReturn::raw_array(object) }
    );
}

#[test]
fn hook_return_converts_from_rust_values() {
    let vm = Vm::dangling_for_tests();
    assert_eq!(
        ().into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Void, "test"),
        Ok(JavaHookReturn::void())
    );
    assert_eq!(
        true.into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Boolean, "test"),
        Ok(JavaHookReturn::boolean(true))
    );
    assert_eq!(
        (11 as jni::jint).into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Int, "test"),
        Ok(JavaHookReturn::int(11))
    );
    assert_eq!(
        (13 as jni::jlong).into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Long, "test"),
        Ok(JavaHookReturn::long(13))
    );
    assert_eq!(
        (1.25 as jni::jfloat).into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Float, "test"),
        Ok(JavaHookReturn::float(1.25))
    );
    assert_eq!(
        (2.5 as jni::jdouble).into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Double, "test"),
        Ok(JavaHookReturn::double(2.5))
    );
    assert_eq!(
        JavaHookReturn::int(11).into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Int, "test"),
        Ok(JavaHookReturn::int(11))
    );
    assert_eq!(
        JavaHookReturn::null_object().into_raw(),
        RawJavaReturn::Object(ptr::null_mut())
    );
    assert_eq!(
        JavaHookReturn::null_array().into_raw(),
        RawJavaReturn::Object(ptr::null_mut())
    );
}

#[test]
fn hook_return_adapts_numeric_literals_to_java_return_type() {
    let vm = Vm::dangling_for_tests();
    assert_eq!(
        8080.into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Long, "test"),
        Ok(JavaHookReturn::long(8080))
    );
    assert_eq!(
        90.into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Char, "test"),
        Ok(JavaHookReturn::char(90))
    );
    assert_eq!(
        6.25.into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Float, "test"),
        Ok(JavaHookReturn::float(6.25))
    );
    assert_eq!(
        (1.5_f32).into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Double, "test"),
        Ok(JavaHookReturn::double(1.5))
    );
}

#[test]
fn explicit_hook_returns_stay_strictly_typed() {
    let vm = Vm::dangling_for_tests();
    assert_eq!(
        JavaHookReturn::int(8080).into_hook_return_for(
            ptr::null_mut(),
            &vm,
            &JavaType::Long,
            "test"
        ),
        Err(Error::InvalidReturnType {
            operation: "test",
            expected: "long",
            actual: "int".to_owned(),
        })
    );
    assert_eq!(
        JavaHookReturn::double(6.25).into_hook_return_for(
            ptr::null_mut(),
            &vm,
            &JavaType::Float,
            "test"
        ),
        Err(Error::InvalidReturnType {
            operation: "test",
            expected: "float",
            actual: "double".to_owned(),
        })
    );
}

#[test]
fn hook_return_rejects_out_of_range_numeric_adaptation() {
    let vm = Vm::dangling_for_tests();
    assert_eq!(
        300.into_hook_return_for(ptr::null_mut(), &vm, &JavaType::Byte, "test"),
        Err(Error::InvalidReturnType {
            operation: "test",
            expected: "byte",
            actual: "int 300 outside byte range".to_owned(),
        })
    );
    assert_eq!(
        (f64::from(f32::MAX) * 2.0).into_hook_return_for(
            ptr::null_mut(),
            &vm,
            &JavaType::Float,
            "test"
        ),
        Err(Error::InvalidReturnType {
            operation: "test",
            expected: "float",
            actual: format!("double {} outside float range", f64::from(f32::MAX) * 2.0),
        })
    );
}

#[test]
fn hook_return_extracts_to_rust_values() {
    let state = test_closure_state("()V", |_| Ok(RawJavaReturn::Void));
    let invocation = JavaHookContext {
        inner: ReplacementInvocation {
            state: &state,
            env: ptr::null_mut(),
            target: ptr::null_mut(),
            arguments: vec![],
        },
    };

    assert_eq!(
        <()>::from_hook_return(JavaHookReturn::void(), &invocation, "test"),
        Ok(())
    );
    assert_eq!(
        bool::from_hook_return(JavaHookReturn::boolean(true), &invocation, "test"),
        Ok(true)
    );
    assert_eq!(
        jni::jint::from_hook_return(JavaHookReturn::int(11), &invocation, "test"),
        Ok(11)
    );
    assert_eq!(
        bool::from_hook_return(JavaHookReturn::int(11), &invocation, "test"),
        Err(Error::InvalidReturnType {
            operation: "test",
            expected: "boolean",
            actual: "int".to_owned(),
        })
    );
}

#[test]
fn hook_context_exposes_metadata() {
    let class = ptr::without_provenance_mut::<jni::_jobject>(0x1230) as jni::jclass;
    let state = test_closure_state_with_kind(MethodKind::Static, "staticAdd", "(II)I", |_| {
        Ok(RawJavaReturn::Int(0))
    });
    let invocation = JavaHookContext {
        inner: ReplacementInvocation {
            state: &state,
            env: ptr::null_mut(),
            target: class.cast(),
            arguments: vec![JavaValue::Int(2), JavaValue::Int(5)],
        },
    };

    assert_eq!(invocation.kind(), MethodKind::Static);
    assert_eq!(invocation.name(), "staticAdd");
    assert_eq!(invocation.signature().to_string(), "(II)I");
    assert_eq!(unsafe { invocation.raw_class() }, Some(class));
    assert_eq!(unsafe { invocation.raw_receiver() }, None);
    assert!(matches!(invocation.maybe_this_object(), Ok(None)));
    assert!(matches!(
        invocation.this_object(),
        Err(Error::WrongMethodKind {
            operation: "JavaHookContext::this_object"
        })
    ));
    assert_eq!(
        unsafe { invocation.raw_arguments() },
        &[JavaValue::Int(2), JavaValue::Int(5)]
    );
    let values = invocation
        .args()
        .iter()
        .map(|argument| {
            argument.map(|argument| match argument {
                JavaHookArgument::Int(value) => value,
                other => panic!("unexpected argument: {other:?}"),
            })
        })
        .collect::<Result<Vec<_>>>()
        .unwrap();
    assert_eq!(values, vec![2, 5]);
    assert_eq!(invocation.args().len(), 2);
}

#[test]
fn hook_context_extracts_typed_arguments() {
    let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
    let state = test_closure_state_with_kind(
        MethodKind::Static,
        "staticMixed",
        "(IZLjava/lang/Object;Ljava/lang/Object;)I",
        |_| Ok(RawJavaReturn::Int(0)),
    );
    let invocation = JavaHookContext {
        inner: ReplacementInvocation {
            state: &state,
            env: ptr::null_mut(),
            target: ptr::null_mut(),
            arguments: vec![
                JavaValue::Int(2),
                JavaValue::Boolean(true),
                JavaValue::object_ref(object),
                JavaValue::NULL,
            ],
        },
    };

    assert_eq!(invocation.arg::<jni::jint>(0), Ok(2));
    assert_eq!(invocation.arg::<bool>(1), Ok(true));
    assert_eq!(
        unsafe { invocation.raw_arg_object(2) },
        Ok(Some(RawJavaObject::from_raw(object)))
    );
    assert_eq!(unsafe { invocation.raw_arg_object(3) }, Ok(None));
    assert_eq!(invocation.arg_is_null(2), Ok(false));
    assert_eq!(invocation.arg_is_null(3), Ok(true));
    assert!(matches!(
        invocation.arg_value(0).unwrap(),
        JavaHookArgument::Int(2)
    ));
    assert!(matches!(
        invocation.arg_value(1).unwrap(),
        JavaHookArgument::Boolean(true)
    ));
    assert!(matches!(
        invocation.arg_value(3).unwrap(),
        JavaHookArgument::Object(None)
    ));
    assert_eq!(invocation.args().len(), 4);
    assert_eq!(
        invocation.arg::<jni::jlong>(0),
        Err(Error::InvalidArgumentType {
            index: 0,
            expected: "long".to_owned(),
            actual: "int",
        })
    );
    assert_eq!(
        invocation.arg_is_null(0),
        Err(Error::InvalidArgumentType {
            index: 0,
            expected: "reference".to_owned(),
            actual: "int",
        })
    );
    assert_eq!(
        invocation.arg::<jni::jint>(4),
        Err(Error::InvalidArguments {
            expected: 5,
            actual: 4,
        })
    );
    assert_eq!(
        invocation.arg_is_null(4),
        Err(Error::InvalidArguments {
            expected: 5,
            actual: 4,
        })
    );
}

#[test]
fn hook_context_formats_simple_arguments_for_display() {
    let state = test_closure_state_with_kind(
        MethodKind::Static,
        "staticMixed",
        "(ZBCSIJFDLjava/lang/Object;)I",
        |_| Ok(RawJavaReturn::Int(0)),
    );
    let invocation = JavaHookContext {
        inner: ReplacementInvocation {
            state: &state,
            env: ptr::null_mut(),
            target: ptr::null_mut(),
            arguments: vec![
                JavaValue::Boolean(true),
                JavaValue::Byte(-2),
                JavaValue::Char('A' as jni::jchar),
                JavaValue::Short(-3),
                JavaValue::Int(4),
                JavaValue::Long(5),
                JavaValue::Float(1.25),
                JavaValue::Double(2.5),
                JavaValue::NULL,
            ],
        },
    };

    assert_eq!(invocation.arg_display(0), Ok("true".to_owned()));
    assert_eq!(
        invocation.arg_value(0).unwrap().java_display(),
        Ok("true".to_owned())
    );
    assert_eq!(invocation.arg_display(1), Ok("-2".to_owned()));
    assert_eq!(invocation.arg_display(2), Ok("A".to_owned()));
    assert_eq!(invocation.arg_display(3), Ok("-3".to_owned()));
    assert_eq!(invocation.arg_display(4), Ok("4".to_owned()));
    assert_eq!(invocation.arg_display(5), Ok("5".to_owned()));
    assert_eq!(invocation.arg_display(6), Ok("1.25".to_owned()));
    assert_eq!(invocation.arg_display(7), Ok("2.5".to_owned()));
    assert_eq!(invocation.arg_display(8), Ok("null".to_owned()));
    assert_eq!(
        invocation.arg_value(8).unwrap().java_display(),
        Ok("null".to_owned())
    );
    assert_eq!(
        invocation.arg_display(9),
        Err(Error::InvalidArguments {
            expected: 10,
            actual: 9,
        })
    );
}

#[test]
fn hook_context_display_reports_invalid_char_and_null_lanes() {
    let state = test_closure_state_with_kind(MethodKind::Static, "staticMixed", "(CI)I", |_| {
        Ok(RawJavaReturn::Int(0))
    });
    let invocation = JavaHookContext {
        inner: ReplacementInvocation {
            state: &state,
            env: ptr::null_mut(),
            target: ptr::null_mut(),
            arguments: vec![JavaValue::Char(0xD800), JavaValue::NULL],
        },
    };

    assert_eq!(invocation.arg_display(0), Ok("\\uD800".to_owned()));
    assert_eq!(
        invocation.arg_value(0).unwrap().java_display(),
        Ok("\\uD800".to_owned())
    );
    assert_eq!(
        invocation.arg_display(1),
        Err(Error::InvalidArgumentType {
            index: 1,
            expected: "I".to_owned(),
            actual: "null",
        })
    );
}

#[test]
fn prepares_original_call_arguments_from_generic_containers() {
    let (signature, args) =
        prepare_original_call_args("(IZLjava/lang/Object;)I", (1_i32, true, JavaValue::NULL))
            .expect("tuple arguments should validate");
    assert_eq!(signature.to_string(), "(IZLjava/lang/Object;)I");
    assert_eq!(
        args,
        vec![JavaValue::Int(1), JavaValue::Boolean(true), JavaValue::NULL]
    );

    let args = [JavaValue::Int(1), JavaValue::Long(2)];
    assert_eq!(
        prepare_original_call_args("(IJ)V", &args)
            .expect("array reference arguments should validate")
            .1,
        args
    );
}

#[test]
fn rejects_invalid_generic_original_call_arguments() {
    assert_eq!(
        prepare_original_call_args("(I)V", (JavaValue::Long(1),)),
        Err(Error::InvalidArgumentType {
            index: 0,
            expected: "I".to_owned(),
            actual: "long",
        })
    );

    assert_eq!(
        prepare_original_call_args("(II)V", (1_i32,)),
        Err(Error::InvalidArguments {
            expected: 2,
            actual: 1,
        })
    );
}

#[test]
fn extracts_typed_raw_return_values() {
    assert_eq!(RawJavaReturn::Int(11).into_int("test"), Ok(11));

    let object = ptr::dangling_mut();
    assert_eq!(
        RawJavaReturn::Object(object).into_object("test"),
        Ok(object)
    );
}

#[test]
fn rejects_wrong_raw_return_extraction() {
    assert_eq!(
        RawJavaReturn::Int(1).into_object("test"),
        Err(Error::InvalidReturnType {
            operation: "test",
            expected: "object",
            actual: "int".to_owned(),
        })
    );
}
