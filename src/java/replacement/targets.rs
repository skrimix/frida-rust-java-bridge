use crate::{
    Error, Result,
    env::MethodKind,
    java::{
        JavaBoundMethodGroup, JavaBoundMethodOverload, JavaClass, JavaConstructor, JavaMethod,
        JavaMethodGroup,
    },
    signature::MethodSignature,
};

use super::{
    api::{JavaHookGuard, hook_kind_name},
    closure::{
        replace_closure_method, replace_constructor_closure, validate_closure_replacement_signature,
    },
    context::JavaHookContext,
    returns::{JavaHookReturn, resolve_reference_return_class, validate_reference_return},
};

const METHOD_HOOK_OPERATION: &str = "JavaMethod::replace";
const CONSTRUCTOR_HOOK_OPERATION: &str = "JavaConstructor::replace";

impl JavaClass {
    /// Replaces a method selected by name with a guarded Rust closure hook.
    ///
    /// The method name must resolve to a single visible overload. Use [`JavaClass::replace_with`]
    /// when a class has multiple overloads with the same name. Keep the returned
    /// [`JavaHookGuard`] alive while the replacement should remain active.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaHookGuard, Result};
    ///
    /// fn hook_to_string(java: &Java) -> Result<JavaHookGuard> {
    ///     let builder = java.use_class("java.lang.StringBuilder")?;
    ///     builder.replace("toString", |ctx| {
    ///         let result: String = ctx.call_original(())?;
    ///         println!("StringBuilder.toString() => {result}");
    ///         ctx.ret(result)
    ///     })
    /// }
    /// ```
    pub fn replace<F>(&self, name: &str, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.method(name)?.replace(callback)
    }

    /// Replaces the method overload with the given argument type names.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaHookGuard, Result};
    ///
    /// fn hook_load_url(java: &Java) -> Result<JavaHookGuard> {
    ///     let webview = java.use_class("android.webkit.WebView")?;
    ///     webview.replace_with("loadUrl", ["java.lang.String"], |ctx| {
    ///         let url: String = ctx.arg(0)?;
    ///         println!("loadUrl({url})");
    ///         ctx.call_original::<()>(ctx.args())?;
    ///         ctx.ret(())
    ///     })
    /// }
    /// ```
    pub fn replace_with<'types, F>(
        &self,
        name: &str,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.method(name)?.overload(arguments)?.replace(callback)
    }

    /// Replaces the selected constructor overload with a guarded Rust closure hook.
    ///
    /// The callback receives [`JavaHookContext`] with `kind()` set to [`MethodKind::Constructor`],
    /// `name()` set to `"<init>"`, and `this_object()` pointing at the object being initialized.
    /// Call [`JavaHookContext::call_original`] to forward to the selected original constructor.
    /// If the callback skips the original constructor, it must initialize the receiver enough for
    /// later Java code. Constructor hooks return void, usually with `ctx.ret(())`.
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaHookGuard, Result};
    ///
    /// fn hook_builder_constructor(java: &Java) -> Result<JavaHookGuard> {
    ///     let builder = java.use_class("java.lang.StringBuilder")?;
    ///     builder.replace_constructor(["java.lang.String"], |ctx| {
    ///         let argument: String = ctx.arg(0)?;
    ///         println!("new StringBuilder({argument:?})");
    ///         ctx.call_original::<()>(ctx.args())?;
    ///         ctx.ret(())
    ///     })
    /// }
    /// ```
    pub fn replace_constructor<'types, F>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        let constructor = self.constructor(arguments)?;
        constructor.replace(callback)
    }
}

impl JavaMethodGroup {
    /// Replaces this method when the group contains exactly one overload.
    ///
    /// Use [`JavaMethodGroup::replace_with`] when the overload should be selected explicitly.
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.unambiguous()?.replace(callback)
    }

    /// Replaces the overload with the given argument type names.
    pub fn replace_with<'types, F>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.overload(arguments)?.replace(callback)
    }
}

impl JavaConstructor {
    /// Replaces this selected constructor overload with a guarded Rust closure hook.
    ///
    /// The callback receives [`JavaHookContext`](crate::java::replacement::JavaHookContext) with
    /// `kind()` set to [`MethodKind::Constructor`](crate::env::MethodKind::Constructor),
    /// `name()` set to `"<init>"`, and `this_object()` pointing at the object being initialized.
    /// Call [`JavaHookContext::call_original`](crate::java::replacement::JavaHookContext::call_original)
    /// to forward to the selected original constructor. If the callback skips the original
    /// constructor, it must initialize the receiver enough for later Java code. Constructor hooks
    /// return void, usually with `ctx.ret(())`. Keep the returned guard alive while the replacement
    /// should remain active; reverting or dropping it restores the original constructor.
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        unsafe { install_constructor_hook(self, callback) }
    }
}

impl JavaMethod {
    /// Replaces this selected method overload with a guarded Rust closure hook.
    ///
    /// The callback receives [`JavaHookContext`](crate::java::replacement::JavaHookContext), can
    /// call the original implementation, and returns a
    /// [`JavaHookReturn`](crate::java::replacement::JavaHookReturn), usually by calling
    /// [`JavaHookContext::ret`](crate::java::replacement::JavaHookContext::ret). Keep the returned
    /// guard alive while the replacement should remain active; reverting or dropping it restores the
    /// original method.
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        unsafe { install_method_hook(self, callback) }
    }
}

impl<'object> JavaBoundMethodGroup<'object> {
    /// Replaces this bound method's selected method when the group contains exactly one overload.
    ///
    /// The replacement applies to the Java method, not only to this receiver object.
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.unambiguous()?.overload().replace(callback)
    }

    /// Replaces the overload with the given argument type names.
    ///
    /// The replacement applies to the Java method, not only to this receiver object.
    pub fn replace_with<'types, F>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.overload(arguments)?.overload().replace(callback)
    }

    pub(crate) fn unambiguous(&self) -> Result<JavaBoundMethodOverload<'object>> {
        Ok(JavaBoundMethodOverload {
            object: self.object,
            overload: self.group.unambiguous()?,
        })
    }
}

unsafe fn install_method_hook<F>(overload: &JavaMethod, callback: F) -> Result<JavaHookGuard>
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

unsafe fn install_constructor_hook<F>(
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

fn validate_hook_abi(kind: MethodKind, name: &str, signature: &MethodSignature) -> Result<()> {
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

fn validate_constructor_hook_abi(signature: &MethodSignature) -> Result<()> {
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
