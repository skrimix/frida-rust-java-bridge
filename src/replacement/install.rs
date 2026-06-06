use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::NonNull,
};

use crate::{
    Error, Result,
    env::{MethodKind, throw_new_illegal_state_exception_if_clear_raw},
    java::{JavaConstructor, JavaMethod},
    jni,
    signature::MethodSignature,
};

use super::{
    api::{
        JavaConstructorHookContext, JavaConstructorInitialized, JavaHookContext, JavaHookGuard,
        hook_kind_name,
    },
    closure::{
        replace_closure_method, replace_constructor_closure, validate_closure_replacement_signature,
    },
    original::RawJavaReturn,
    returns::{JavaHookReturn, resolve_reference_return_class, validate_reference_return},
};

const METHOD_HOOK_OPERATION: &str = "JavaMethod::replace";
const CONSTRUCTOR_HOOK_OPERATION: &str = "JavaConstructor::replace";

pub(crate) unsafe fn install_method_hook<F>(
    overload: &JavaMethod,
    callback: F,
) -> Result<JavaHookGuard>
where
    F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
{
    validate_hook_abi(overload.kind(), overload.name(), overload.signature())?;
    let return_type = overload.signature().return_type().clone();
    let return_class = resolve_reference_return_class(overload.class(), &return_type)?;
    let inner = unsafe {
        replace_closure_method(overload, move |invocation| {
            let env = invocation.env_raw();
            callback(JavaHookContext::from_invocation(invocation)).and_then(|value| {
                let hook_return =
                    validate_reference_return(env, &return_class, &return_type, value)?;
                Ok(hook_return.into_raw())
            })
        })
    }?;
    Ok(JavaHookGuard::from_replacement(inner))
}

pub(crate) unsafe fn install_constructor_hook<F>(
    overload: &JavaConstructor,
    callback: F,
) -> Result<JavaHookGuard>
where
    F: for<'a> Fn(JavaConstructorHookContext<'a>) -> Result<JavaConstructorInitialized<'a>>
        + Send
        + Sync
        + 'static,
{
    validate_constructor_hook_abi(overload.signature())?;
    let inner = unsafe {
        replace_constructor_closure(overload, move |invocation| {
            let env = invocation.env_raw();
            let context = JavaConstructorHookContext::from_context(
                JavaHookContext::from_invocation(invocation),
            );
            let result = catch_unwind(AssertUnwindSafe(|| callback(context)));
            match result {
                Ok(Ok(_initialized)) => Ok(RawJavaReturn::Void),
                Ok(Err(error)) => {
                    ensure_safe_constructor_failure_exception(env, &error)?;
                    Err(error)
                }
                Err(_) => {
                    let error = Error::InvalidReplacementState {
                        operation: CONSTRUCTOR_HOOK_OPERATION,
                        reason:
                            "safe constructor replacement callback panicked before initialization"
                                .to_owned(),
                    };
                    ensure_safe_constructor_failure_exception(env, &error)?;
                    Err(error)
                }
            }
        })
    }?;
    Ok(JavaHookGuard::from_replacement(inner))
}

pub(crate) unsafe fn install_constructor_hook_unchecked<F>(
    overload: &JavaConstructor,
    callback: F,
) -> Result<JavaHookGuard>
where
    F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
{
    validate_constructor_hook_abi(overload.signature())?;
    let return_type = overload.signature().return_type().clone();
    let inner = unsafe {
        replace_constructor_closure(overload, move |invocation| {
            callback(JavaHookContext::from_invocation(invocation)).and_then(|value| {
                value
                    .coerce_for_return_type(&return_type, "closure replacement return")
                    .map(JavaHookReturn::into_raw)
            })
        })
    }?;
    Ok(JavaHookGuard::from_replacement(inner))
}

fn ensure_safe_constructor_failure_exception(env: *mut jni::JNIEnv, error: &Error) -> Result<()> {
    if error.java_throwable().is_some() {
        return Ok(());
    }

    let env = NonNull::new(env).ok_or(Error::NullReturn {
        operation: "closure replacement JNIEnv",
    })?;
    unsafe { throw_new_illegal_state_exception_if_clear_raw(env, &error.to_string()) }
}

pub(crate) fn validate_hook_abi(
    kind: MethodKind,
    name: &str,
    signature: &MethodSignature,
) -> Result<()> {
    if kind == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: METHOD_HOOK_OPERATION,
        });
    }
    hook_signature_supported(kind, signature, METHOD_HOOK_OPERATION).map_err(|error| match error {
        Error::WrongMethodKind { .. } => Error::WrongMethodKind {
            operation: METHOD_HOOK_OPERATION,
        },
        Error::InvalidReplacementImplementation { actual, .. } => {
            Error::UnsupportedReplacementImplementation {
                operation: METHOD_HOOK_OPERATION,
                method: format!("{} {name}", hook_kind_name(kind)),
                reason: hook_unsupported_reason(actual),
            }
        }
        other => other,
    })
}

pub(crate) fn validate_constructor_hook_abi(signature: &MethodSignature) -> Result<()> {
    hook_signature_supported(
        MethodKind::Constructor,
        signature,
        CONSTRUCTOR_HOOK_OPERATION,
    )
    .map_err(|error| match error {
        Error::WrongMethodKind { .. } => Error::WrongMethodKind {
            operation: CONSTRUCTOR_HOOK_OPERATION,
        },
        Error::InvalidReplacementImplementation { actual, .. } => {
            Error::UnsupportedReplacementImplementation {
                operation: CONSTRUCTOR_HOOK_OPERATION,
                method: "constructor <init>".to_owned(),
                reason: hook_unsupported_reason(actual),
            }
        }
        other => other,
    })
}

fn hook_signature_supported(
    kind: MethodKind,
    signature: &MethodSignature,
    operation: &'static str,
) -> Result<()> {
    if kind == MethodKind::Constructor {
        return validate_closure_replacement_signature(kind, signature, operation);
    }
    validate_closure_replacement_signature(kind, signature, operation)
}

fn hook_unsupported_reason(actual: &'static str) -> &'static str {
    match actual {
        "descriptor is too large" | "descriptor overflows closure invocation frame sizing" => {
            "descriptor has too many arguments"
        }
        _ => "descriptor is unsupported",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signature(value: &str) -> MethodSignature {
        MethodSignature::parse(value).unwrap()
    }

    #[test]
    fn hook_admission_accepts_current_facade_lanes() {
        for (kind, name, descriptor) in [
            (MethodKind::Static, "staticAnswer", "()I"),
            (MethodKind::Static, "staticString", "()Ljava/lang/String;"),
            (MethodKind::Static, "staticArray", "()[Ljava/lang/Object;"),
            (MethodKind::Static, "staticIdentity", "(I)I"),
            (
                MethodKind::Static,
                "staticObjectEcho",
                "(Ljava/lang/Object;)Ljava/lang/Object;",
            ),
            (MethodKind::Instance, "objectSink", "(Ljava/lang/Object;)V"),
            (
                MethodKind::Static,
                "staticObjectIntVoid",
                "(Ljava/lang/Object;I)V",
            ),
            (MethodKind::Instance, "instanceAdd", "(II)I"),
            (
                MethodKind::Static,
                "staticObjectPairEcho",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            ),
            (
                MethodKind::Instance,
                "instanceObjectPairEcho",
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
            ),
            (MethodKind::Static, "staticPrimitiveMix", "(ZBCS)I"),
            (MethodKind::Instance, "instancePrimitiveMix", "(ZBCS)I"),
            (MethodKind::Static, "staticWide", "(JD)J"),
            (MethodKind::Instance, "instanceWide", "(JD)J"),
            (MethodKind::Static, "staticFloatMix", "(FD)D"),
            (MethodKind::Instance, "instanceFloatMix", "(FD)D"),
            (
                MethodKind::Static,
                "staticStackSpill",
                "(IIIIIIIIDDDDDDDDD)D",
            ),
            (
                MethodKind::Instance,
                "instanceStackSpill",
                "(IIIIIIIIDDDDDDDDD)D",
            ),
            (
                MethodKind::Static,
                "staticMixedReferences",
                "(Ljava/lang/Object;I[Ljava/lang/Object;Z)Ljava/lang/Object;",
            ),
            (MethodKind::Instance, "sumIntArray", "([I)I"),
        ] {
            validate_hook_abi(kind, name, &signature(descriptor)).unwrap();
        }
    }

    #[test]
    fn hook_admission_error_names_facade_and_reason() {
        let many_int_args = format!("({})I", "I".repeat(600));
        let error = validate_hook_abi(MethodKind::Static, "tooLarge", &signature(&many_int_args))
            .unwrap_err();

        let Error::UnsupportedReplacementImplementation {
            operation,
            method,
            reason,
        } = error
        else {
            panic!("unexpected admission error: {error:?}");
        };

        assert_eq!(operation, METHOD_HOOK_OPERATION);
        assert!(method.starts_with("static method tooLarge"));
        assert_eq!(reason, "descriptor has too many arguments");
    }

    #[test]
    fn hook_admission_rejects_constructors_as_facade_operation() {
        assert_eq!(
            validate_hook_abi(MethodKind::Constructor, "$init", &signature("()V")).unwrap_err(),
            Error::WrongMethodKind {
                operation: METHOD_HOOK_OPERATION,
            }
        );
    }

    #[test]
    fn constructor_hook_admission_accepts_void_constructor_lanes() {
        for descriptor in ["()V", "(I)V", "(Ljava/lang/Object;IZ[Ljava/lang/Object;)V"] {
            validate_constructor_hook_abi(&signature(descriptor))
                .unwrap_or_else(|_| panic!("constructor facade should accept {descriptor}"));
        }
    }

    #[test]
    fn constructor_hook_admission_error_names_facade_and_reason() {
        let many_int_args = format!("({})V", "I".repeat(600));
        let error = validate_constructor_hook_abi(&signature(&many_int_args)).unwrap_err();

        let Error::UnsupportedReplacementImplementation {
            operation,
            method,
            reason,
        } = error
        else {
            panic!("unexpected admission error: {error:?}");
        };

        assert_eq!(operation, CONSTRUCTOR_HOOK_OPERATION);
        assert_eq!(method, "constructor <init>");
        assert_eq!(reason, "descriptor has too many arguments");
    }

    #[test]
    fn constructor_hook_admission_rejects_non_void_descriptors() {
        assert_eq!(
            validate_constructor_hook_abi(&signature("()I")).unwrap_err(),
            Error::UnsupportedReplacementImplementation {
                operation: CONSTRUCTOR_HOOK_OPERATION,
                method: "constructor <init>".to_owned(),
                reason: "descriptor is unsupported",
            }
        );
    }
}
