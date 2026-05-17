//! ART method replacement facade and internal scaffolding.
//!
//! The intended user-facing replacement path is
//! [`JavaMethodOverload::implementation`](crate::JavaMethodOverload::implementation). It installs a
//! guarded Rust closure, passes [`ImplementationInvocation`] to the callback, and accepts values
//! convertible into [`ImplementationReturn`]. Lower-level raw JNI/native replacement helpers remain
//! crate-internal scaffolding for app startup hooks and live-runtime harnesses.

#![allow(dead_code)]

mod api;
mod closure;
mod native;
mod original;
mod trampoline;

const FEATURE_CLOSURE_REPLACEMENT: &str = "closure-backed method replacement";

pub(crate) use api::implementation_method;
pub use api::{
    FromImplementationReturn, FromJavaValue, ImplementationGuard, ImplementationInvocation,
    ImplementationReturn, IntoImplementationReturn,
};
pub(crate) use closure::{ClosureMethodReplacement, ReplacementInvocation, replace_closure_method};
#[cfg(test)]
use closure::{ClosureReplacementAbi, ClosureReplacementState, closure_replacement_abi};
#[allow(unused_imports)]
pub(crate) use native::{
    MethodImplementation, MethodReplacement, NativeMethodImplementation,
    call_original_instance_i32_method, call_original_instance_method,
    call_original_static_i32_method, call_original_static_method, replace_instance_boolean_method,
    replace_instance_byte_method, replace_instance_char_method,
    replace_instance_f32_f64_to_f64_method, replace_instance_f32_method,
    replace_instance_f64_method, replace_instance_i32_i32_to_i32_method,
    replace_instance_i32_method, replace_instance_i64_f64_to_i64_method,
    replace_instance_i64_method, replace_instance_native_method,
    replace_instance_reference_to_reference_method, replace_instance_short_method,
    replace_instance_string_method, replace_instance_string_to_string_method,
    replace_instance_void_method, replace_instance_z_b_c_s_to_i32_method, replace_method,
    replace_native_method, replace_static_boolean_method, replace_static_byte_method,
    replace_static_char_method, replace_static_f32_f64_to_f64_method, replace_static_f32_method,
    replace_static_f64_method, replace_static_i32_i32_to_i32_method, replace_static_i32_method,
    replace_static_i64_f64_to_i64_method, replace_static_i64_method, replace_static_native_method,
    replace_static_reference_to_reference_method, replace_static_short_method,
    replace_static_string_method, replace_static_string_to_string_method,
    replace_static_void_method, replace_static_z_b_c_s_to_i32_method,
};
#[cfg(test)]
use native::{
    native_replacement_pointer_for, prepare_original_call_args, replacement_pointer_for,
    validate_reference_to_reference_signature,
};
pub(crate) use original::{OriginalMethod, RawJavaReturn};

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::c_void, ptr, sync::Mutex};

    use crate::{
        Result,
        env::MethodKind,
        error::Error,
        jni,
        signature::{JavaType, MethodSignature},
        value::JavaValue,
        vm::Vm,
    };

    unsafe extern "C" fn static_i32(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jint {
        1
    }

    unsafe extern "C" fn static_object_echo(
        _env: *mut jni::JNIEnv,
        _class: jni::jclass,
        value: jni::jobject,
    ) -> jni::jobject {
        value
    }

    unsafe extern "C" fn instance_i32(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
    ) -> jni::jint {
        1
    }

    unsafe extern "C" fn instance_string_to_string(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        value: jni::jstring,
    ) -> jni::jstring {
        value
    }

    unsafe extern "C" fn instance_object_echo(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        value: jni::jobject,
    ) -> jni::jobject {
        value
    }

    unsafe extern "C" fn instance_object_void(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        _value: jni::jobject,
    ) {
    }

    fn dummy_replacement_ptr() -> *mut c_void {
        static_i32 as *const () as *mut c_void
    }

    #[test]
    fn accepts_supported_native_replacement_abi_shapes() {
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
            "()Ljava/lang/String;",
            "(Ljava/lang/String;)Ljava/lang/String;",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            "(Ljava/lang/Object;)V",
            "([Ljava/lang/Object;)[Ljava/lang/Object;",
            "([Ljava/lang/Object;)V",
            "([I)[Ljava/lang/Object;",
            "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZZ)Landroid/app/LoadedApk;",
            "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZ)Landroid/app/LoadedApk;",
            "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;",
            "(Ljava/lang/String;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;",
            "(ZLandroid/app/Instrumentation;)Landroid/app/Application;",
            "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZZ)Ljava/lang/Object;",
            "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;",
            "(Ljava/lang/Object;Ljava/lang/Object;I)Ljava/lang/Object;",
            "(Ljava/lang/String;Ljava/lang/Object;I)Ljava/lang/Object;",
            "(ZLjava/lang/Object;)Ljava/lang/Object;",
            "(II)I",
            "(ZBCS)I",
            "(JD)J",
            "(FD)D",
        ] {
            unsafe {
                NativeMethodImplementation::static_method(signature, dummy_replacement_ptr())
                    .unwrap_or_else(|_| panic!("static ABI {signature} should be supported"));
                NativeMethodImplementation::instance_method(signature, dummy_replacement_ptr())
                    .unwrap_or_else(|_| panic!("instance ABI {signature} should be supported"));
            }
        }
    }

    #[test]
    fn rejects_unsupported_native_replacement_abi_shapes() {
        assert_eq!(
            unsafe {
                NativeMethodImplementation::static_method(
                    "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                    dummy_replacement_ptr(),
                )
            },
            Err(Error::InvalidReplacementImplementation {
                operation: "NativeMethodImplementation::static_method",
                expected: "supported static method replacement ABI".to_owned(),
                actual: "NativeMethodImplementation",
            })
        );

        assert_eq!(
            unsafe {
                NativeMethodImplementation::instance_method(
                    "()Ljava/lang/Object;",
                    dummy_replacement_ptr(),
                )
            },
            Err(Error::InvalidReplacementImplementation {
                operation: "NativeMethodImplementation::instance_method",
                expected: "supported instance method replacement ABI".to_owned(),
                actual: "NativeMethodImplementation",
            })
        );

        assert!(matches!(
            unsafe { NativeMethodImplementation::static_method("(I", dummy_replacement_ptr()) },
            Err(Error::InvalidSignature { .. })
        ));
    }

    #[test]
    fn accepts_matching_replacement_implementations() {
        replacement_pointer_for(
            MethodKind::Static,
            "()I",
            MethodImplementation::StaticI32(static_i32),
        )
        .expect("static int implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "(Ljava/lang/String;)Ljava/lang/String;",
            MethodImplementation::InstanceStringToString(instance_string_to_string),
        )
        .expect("instance string implementation should match");

        replacement_pointer_for(
            MethodKind::Static,
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            MethodImplementation::StaticReferenceToReference(static_object_echo),
        )
        .expect("static reference implementation should match");

        replacement_pointer_for(
            MethodKind::Static,
            "([Ljava/lang/Object;)[Ljava/lang/Object;",
            MethodImplementation::StaticReferenceToReference(static_object_echo),
        )
        .expect("static object array implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "([I)[Ljava/lang/Object;",
            MethodImplementation::InstanceReferenceToReference(instance_object_echo),
        )
        .expect("instance primitive array implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "(Ljava/lang/Object;)V",
            MethodImplementation::InstanceReferenceToVoid(instance_object_void),
        )
        .expect("instance reference-to-void implementation should match");
    }

    #[test]
    fn rejects_mismatched_replacement_implementations() {
        assert_eq!(
            replacement_pointer_for(
                MethodKind::Instance,
                "()I",
                MethodImplementation::StaticI32(static_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "StaticI32",
            })
        );

        assert_eq!(
            replacement_pointer_for(
                MethodKind::Static,
                "()I",
                MethodImplementation::InstanceI32(instance_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_method",
                expected: "instance method ()I".to_owned(),
                actual: "InstanceI32",
            })
        );
    }

    #[test]
    fn rejects_unsupported_facade_signatures() {
        assert_eq!(
            replacement_pointer_for(
                MethodKind::Static,
                "(I)I",
                MethodImplementation::StaticI32(static_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "StaticI32",
            })
        );
    }

    #[test]
    fn rejects_mismatched_native_replacement_implementations() {
        let implementation =
            unsafe { NativeMethodImplementation::static_method("()I", dummy_replacement_ptr()) }
                .expect("static int native implementation should be accepted");
        assert_eq!(
            native_replacement_pointer_for(MethodKind::Instance, "()I", implementation),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "NativeMethodImplementation",
            })
        );
    }

    #[test]
    fn original_method_captures_metadata_and_rejects_constructors() {
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
        ClosureReplacementState {
            vm: Vm::dangling_for_tests(),
            kind,
            name: name.to_owned(),
            signature: MethodSignature::parse(signature).expect("test signature should parse"),
            original: OriginalMethod::from_parts(kind, name, signature)
                .expect("test original should be captured"),
            callback: Box::new(callback),
            last_error: Mutex::new(None),
        }
    }

    #[test]
    fn classifies_closure_replacement_signatures() {
        for (signature, abi) in [
            ("()V", ClosureReplacementAbi::NoArgsVoid),
            ("()Z", ClosureReplacementAbi::NoArgsBoolean),
            ("()B", ClosureReplacementAbi::NoArgsByte),
            ("()C", ClosureReplacementAbi::NoArgsChar),
            ("()S", ClosureReplacementAbi::NoArgsShort),
            ("()I", ClosureReplacementAbi::NoArgsInt),
            ("()J", ClosureReplacementAbi::NoArgsLong),
            ("()F", ClosureReplacementAbi::NoArgsFloat),
            ("()D", ClosureReplacementAbi::NoArgsDouble),
            ("()Ljava/lang/Object;", ClosureReplacementAbi::NoArgsObject),
            ("()[Ljava/lang/Object;", ClosureReplacementAbi::NoArgsObject),
        ] {
            assert_eq!(
                closure_replacement_abi(
                    MethodKind::Static,
                    &MethodSignature::parse(signature).unwrap()
                ),
                Ok(abi),
                "{signature}"
            );
        }
        assert_eq!(
            closure_replacement_abi(MethodKind::Static, &MethodSignature::parse("()I").unwrap()),
            Ok(ClosureReplacementAbi::NoArgsInt)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse("(Ljava/lang/String;)Ljava/lang/String;").unwrap()
            ),
            Ok(ClosureReplacementAbi::OneReferenceToReference)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Static,
                &MethodSignature::parse("([Ljava/lang/Object;)[Ljava/lang/Object;").unwrap()
            ),
            Ok(ClosureReplacementAbi::OneReferenceToReference)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse("(Ljava/lang/Object;)V").unwrap()
            ),
            Ok(ClosureReplacementAbi::OneReferenceToVoid)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Static,
                &MethodSignature::parse("(II)I").unwrap()
            ),
            Ok(ClosureReplacementAbi::I32I32ToI32)
        );
    }

    #[test]
    fn rejects_unsupported_closure_replacement_signatures() {
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse(
                    "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;"
                )
                .unwrap()
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_closure_method",
                expected: "supported instance closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(MethodKind::Static, &MethodSignature::parse("(I)I").unwrap()),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_closure_method",
                expected: "supported static closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Static,
                &MethodSignature::parse("(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;")
                    .unwrap()
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_closure_method",
                expected: "supported static closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse("(Ljava/lang/Object;I)Ljava/lang/Object;").unwrap()
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "replacement::replace_closure_method",
                expected: "supported instance closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Constructor,
                &MethodSignature::parse("()V").unwrap()
            ),
            Err(Error::WrongMethodKind {
                operation: "replacement::replace_closure_method",
            })
        );
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
    fn closure_state_passes_reference_arguments() {
        let object = ptr::dangling_mut();
        let object_addr = object as usize;
        let state = test_closure_state(
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            move |invocation| {
                let object = object_addr as jni::jobject;
                assert_eq!(invocation.arguments(), &[JavaValue::Object(object)]);
                Ok(RawJavaReturn::Object(object))
            },
        );
        assert_eq!(
            state.invoke(
                ptr::null_mut(),
                ptr::null_mut(),
                vec![JavaValue::Object(object)]
            ),
            RawJavaReturn::Object(object)
        );
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
                assert_eq!(invocation.arguments(), &[JavaValue::Null]);
                Ok(RawJavaReturn::Object(ptr::null_mut()))
            },
        );
        assert_eq!(
            state.invoke(ptr::null_mut(), receiver, vec![JavaValue::Null]),
            RawJavaReturn::Object(ptr::null_mut())
        );
    }

    struct BorrowedObject(jni::jobject);

    impl crate::refs::AsJObject for BorrowedObject {
        fn as_jobject(&self) -> jni::jobject {
            self.0
        }
    }

    #[test]
    fn implementation_return_converts_to_raw_values() {
        let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
        let borrowed = BorrowedObject(object);

        assert_eq!(ImplementationReturn::Void.into_raw(), RawJavaReturn::Void);
        assert_eq!(
            ImplementationReturn::Boolean(true).into_raw(),
            RawJavaReturn::Boolean(jni::JNI_TRUE)
        );
        assert_eq!(
            ImplementationReturn::Boolean(false).into_raw(),
            RawJavaReturn::Boolean(jni::JNI_FALSE)
        );
        assert_eq!(
            ImplementationReturn::Byte(-7).into_raw(),
            RawJavaReturn::Byte(-7)
        );
        assert_eq!(
            ImplementationReturn::Char(65).into_raw(),
            RawJavaReturn::Char(65)
        );
        assert_eq!(
            ImplementationReturn::Short(-9).into_raw(),
            RawJavaReturn::Short(-9)
        );
        assert_eq!(
            ImplementationReturn::Int(11).into_raw(),
            RawJavaReturn::Int(11)
        );
        assert_eq!(
            ImplementationReturn::Long(13).into_raw(),
            RawJavaReturn::Long(13)
        );
        assert_eq!(
            ImplementationReturn::Float(1.25).into_raw(),
            RawJavaReturn::Float(1.25)
        );
        assert_eq!(
            ImplementationReturn::Double(2.5).into_raw(),
            RawJavaReturn::Double(2.5)
        );
        assert_eq!(
            ImplementationReturn::object(Some(&borrowed)).into_raw(),
            RawJavaReturn::Object(object)
        );
        assert_eq!(
            ImplementationReturn::array(Some(&borrowed)).into_raw(),
            RawJavaReturn::Object(object)
        );
        assert_eq!(
            ImplementationReturn::object::<BorrowedObject>(None).into_raw(),
            RawJavaReturn::Object(ptr::null_mut())
        );
        assert_eq!(
            ImplementationReturn::array::<BorrowedObject>(None).into_raw(),
            RawJavaReturn::Object(ptr::null_mut())
        );

        assert_eq!(
            ImplementationReturn::from_raw_for_type(
                RawJavaReturn::Object(object),
                &JavaType::Array(Box::new(JavaType::Int)),
            ),
            ImplementationReturn::Array(Some(object))
        );
    }

    #[test]
    fn implementation_return_converts_from_rust_values() {
        assert_eq!(().into_implementation_return(), ImplementationReturn::Void);
        assert_eq!(
            true.into_implementation_return(),
            ImplementationReturn::Boolean(true)
        );
        assert_eq!(
            (11 as jni::jint).into_implementation_return(),
            ImplementationReturn::Int(11)
        );
        assert_eq!(
            (13 as jni::jlong).into_implementation_return(),
            ImplementationReturn::Long(13)
        );
        assert_eq!(
            (1.25 as jni::jfloat).into_implementation_return(),
            ImplementationReturn::Float(1.25)
        );
        assert_eq!(
            (2.5 as jni::jdouble).into_implementation_return(),
            ImplementationReturn::Double(2.5)
        );
        let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
        assert_eq!(
            object.into_implementation_return(),
            ImplementationReturn::Object(Some(object))
        );
        assert_eq!(
            Some(object).into_implementation_return(),
            ImplementationReturn::Object(Some(object))
        );
        assert_eq!(
            None::<jni::jobject>.into_implementation_return(),
            ImplementationReturn::Object(None)
        );
        assert_eq!(
            ImplementationReturn::null_object().into_raw(),
            RawJavaReturn::Object(ptr::null_mut())
        );
        assert_eq!(
            ImplementationReturn::null_array().into_raw(),
            RawJavaReturn::Object(ptr::null_mut())
        );
    }

    #[test]
    fn implementation_return_extracts_to_rust_values() {
        assert_eq!(
            <()>::from_implementation_return(ImplementationReturn::Void, "test"),
            Ok(())
        );
        assert_eq!(
            bool::from_implementation_return(ImplementationReturn::Boolean(true), "test"),
            Ok(true)
        );
        assert_eq!(
            jni::jint::from_implementation_return(ImplementationReturn::Int(11), "test"),
            Ok(11)
        );

        let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
        assert_eq!(
            jni::jobject::from_implementation_return(
                ImplementationReturn::Object(Some(object)),
                "test"
            ),
            Ok(object)
        );
        assert_eq!(
            Option::<jni::jobject>::from_implementation_return(
                ImplementationReturn::Array(None),
                "test"
            ),
            Ok(None)
        );
        assert_eq!(
            bool::from_implementation_return(ImplementationReturn::Int(11), "test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "boolean",
                actual: "int".to_owned(),
            })
        );
    }

    #[test]
    fn implementation_invocation_exposes_metadata() {
        let class = ptr::without_provenance_mut::<jni::_jobject>(0x1230) as jni::jclass;
        let state = test_closure_state_with_kind(MethodKind::Static, "staticAdd", "(II)I", |_| {
            Ok(RawJavaReturn::Int(0))
        });
        let invocation = ImplementationInvocation {
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
        assert_eq!(invocation.class(), Some(class));
        assert_eq!(invocation.receiver(), None);
        assert_eq!(
            invocation.arguments(),
            &[JavaValue::Int(2), JavaValue::Int(5)]
        );
    }

    #[test]
    fn implementation_invocation_extracts_typed_arguments() {
        let object = ptr::without_provenance_mut::<jni::_jobject>(0x1230);
        let state = test_closure_state_with_kind(
            MethodKind::Static,
            "staticMixed",
            "(IZLjava/lang/Object;)I",
            |_| Ok(RawJavaReturn::Int(0)),
        );
        let invocation = ImplementationInvocation {
            inner: ReplacementInvocation {
                state: &state,
                env: ptr::null_mut(),
                target: ptr::null_mut(),
                arguments: vec![
                    JavaValue::Int(2),
                    JavaValue::Boolean(true),
                    JavaValue::Object(object),
                    JavaValue::Null,
                ],
            },
        };

        assert_eq!(invocation.arg::<jni::jint>(0), Ok(2));
        assert_eq!(invocation.arg::<bool>(1), Ok(true));
        assert_eq!(invocation.arg::<jni::jobject>(2), Ok(object));
        assert_eq!(invocation.arg::<Option<jni::jobject>>(2), Ok(Some(object)));
        assert_eq!(invocation.arg::<Option<jni::jobject>>(3), Ok(None));
        assert_eq!(invocation.args(), invocation.arguments());
        assert_eq!(
            invocation.arg::<jni::jlong>(0),
            Err(Error::InvalidArgumentType {
                index: 0,
                expected: "long".to_owned(),
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
    }

    #[test]
    fn prepares_original_call_arguments_from_generic_containers() {
        let (signature, args) =
            prepare_original_call_args("(IZLjava/lang/Object;)I", (1_i32, true, JavaValue::Null))
                .expect("tuple arguments should validate");
        assert_eq!(signature.to_string(), "(IZLjava/lang/Object;)I");
        assert_eq!(
            args,
            vec![JavaValue::Int(1), JavaValue::Boolean(true), JavaValue::Null]
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
        assert_eq!(RawJavaReturn::Void.into_void("test"), Ok(()));
        assert_eq!(
            RawJavaReturn::Boolean(jni::JNI_TRUE).into_boolean("test"),
            Ok(true)
        );
        assert_eq!(
            RawJavaReturn::Boolean(jni::JNI_FALSE).into_boolean("test"),
            Ok(false)
        );
        assert_eq!(RawJavaReturn::Byte(-7).into_byte("test"), Ok(-7));
        assert_eq!(RawJavaReturn::Char(65).into_char("test"), Ok(65));
        assert_eq!(RawJavaReturn::Short(-9).into_short("test"), Ok(-9));
        assert_eq!(RawJavaReturn::Int(11).into_int("test"), Ok(11));
        assert_eq!(RawJavaReturn::Long(13).into_long("test"), Ok(13));
        assert_eq!(RawJavaReturn::Float(1.25).into_float("test"), Ok(1.25));
        assert_eq!(RawJavaReturn::Double(2.5).into_double("test"), Ok(2.5));

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

    #[test]
    fn validates_reference_to_reference_signatures() {
        validate_reference_to_reference_signature("(Ljava/lang/Object;)Ljava/lang/Object;", "test")
            .expect("object signature should be accepted");
        validate_reference_to_reference_signature(
            "(Lfrida/java/bridge/rs/test/TestSubject;)Lfrida/java/bridge/rs/test/TestSubject;",
            "test",
        )
        .expect("custom object signature should be accepted");
        validate_reference_to_reference_signature("([I)[Ljava/lang/Object;", "test")
            .expect("array signature should be accepted");
    }

    #[test]
    fn rejects_non_reference_replacement_signatures() {
        assert_eq!(
            validate_reference_to_reference_signature("(I)Ljava/lang/Object;", "test"),
            Err(Error::InvalidArgumentType {
                index: 0,
                expected: "reference".to_owned(),
                actual: "int",
            })
        );
        assert_eq!(
            validate_reference_to_reference_signature("(Ljava/lang/Object;)I", "test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "reference",
                actual: "I".to_owned(),
            })
        );
        assert_eq!(
            validate_reference_to_reference_signature(
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                "test",
            ),
            Err(Error::InvalidArguments {
                expected: 1,
                actual: 2,
            })
        );
    }
}
