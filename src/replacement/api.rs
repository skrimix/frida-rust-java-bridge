use std::{fmt, ptr::NonNull};

use crate::{
    Error, Result,
    env::{Env, MethodKind, throw_new_illegal_state_exception_if_clear_raw},
    java::{
        IntoJavaCallArgs, JavaClass, JavaConstructor, JavaLocalArray, JavaLocalObject, JavaMethod,
    },
    jni, metadata,
    refs::AsJClass,
    signature::{JavaType, MethodSignature},
};

use super::{
    arguments::{FromJavaHookArgument, JavaHookArgument, JavaHookArguments},
    closure::{
        ClosureMethodReplacement, ReplacementInvocation, replace_closure_method,
        replace_constructor_closure, validate_closure_replacement_signature,
    },
    original::RawJavaReturn,
    returns::{
        FromJavaHookReturn, IntoJavaHookReturn, JavaHookReturn, invalid_hook_return,
        resolve_reference_return_class, validate_reference_return,
    },
};

const METHOD_HOOK_OPERATION: &str = "JavaMethod::replace";
const CONSTRUCTOR_HOOK_OPERATION: &str = "JavaConstructor::replace";

/// Owns one installed Java method or constructor replacement.
///
/// Keep the guard alive for as long as the replacement should remain installed. Dropping the guard
/// attempts to restore the original implementation, but explicit [`JavaHookGuard::revert`] is the
/// way to observe restore errors.
pub struct JavaHookGuard {
    inner: ClosureMethodReplacement,
}

/// Error or panic reported by an installed Java replacement callback.
///
/// Replacement callbacks run later, when Java calls the hooked method. Callback failures still
/// cause Java callers to receive the JNI default value for the method return type, except for Java
/// exceptions from original-call helpers or Java wrapper calls, which are rethrown to the Java
/// caller when the callback returns the Java-backed error.
/// Callers can attach an [`JavaHookGuard::on_error`] reporter to observe failures as they happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaHookError {
    kind: MethodKind,
    name: String,
    signature: MethodSignature,
    message: String,
}

/// Invocation details passed to an installed method replacement.
///
/// A `JavaHookContext` value is valid only while Java is executing the replacement callback. Use it
/// to inspect arguments, get `this`, call the original implementation, create callback-local return
/// values, or access the raw JNI layer when needed.
pub struct JavaHookContext<'state> {
    pub(crate) inner: ReplacementInvocation<'state>,
}

/// Invocation details passed to safe constructor replacement callbacks.
///
/// Safe constructor hooks must call the selected original constructor through
/// [`JavaConstructorHookContext::call_original`] or
/// [`JavaConstructorHookContext::call_original_current`] and return the resulting
/// [`JavaConstructorInitialized`] token. Use the unchecked constructor APIs for hooks that
/// intentionally initialize the receiver some other way.
pub struct JavaConstructorHookContext<'state> {
    inner: JavaHookContext<'state>,
}

/// Proof that a safe constructor replacement has initialized its receiver.
///
/// This token is only produced by safe original-constructor calls made from
/// [`JavaConstructorHookContext`]. It is intentionally neither `Clone` nor `Copy`.
pub struct JavaConstructorInitialized<'state> {
    context: JavaHookContext<'state>,
    _sealed: sealed::ConstructorInitialized,
}

mod sealed {
    pub(super) struct ConstructorInitialized;
}

/// A safe target that can be replaced with a guarded method callback.
pub trait JavaHookTarget {
    /// Replaces this hook target with a guarded Rust closure.
    fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static;
}

/// Owns several hook guards and can revert them together.
///
/// This is useful when a `perform()` callback installs a group of related replacements and wants to
/// keep one returned value alive for all of them.
#[derive(Default)]
pub struct JavaHookSet {
    guards: Vec<JavaHookGuard>,
}

impl JavaHookGuard {
    /// Restores the original method now.
    ///
    /// This is safe to call more than once; after a successful restore, later calls are no-ops. If
    /// restore reports an error, the replacement stays active. Dropping a guard that has not been
    /// successfully restored also attempts a restore, but drop cannot return teardown errors. Use
    /// explicit `revert()` when restore failure must be observed as a `Result`.
    pub fn revert(&mut self) -> Result<()> {
        self.inner.revert()
    }

    /// Installs an error reporter and returns this guard for call-site chaining.
    ///
    /// The reporter is called on the Java thread that encountered the callback failure, after the
    /// same error has been recorded for [`JavaHookGuard::last_error`].
    pub fn on_error<F>(self, handler: F) -> Self
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        self.set_error_handler(handler);
        self
    }

    /// Installs or replaces the error reporter for this guard.
    ///
    /// Use this when a guard has already been stored somewhere. For builder-style installation at
    /// the replacement call site, prefer [`JavaHookGuard::on_error`].
    pub fn set_error_handler<F>(&self, handler: F)
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        self.inner.set_error_handler(handler);
    }

    /// Clears the installed error reporter, if any.
    pub fn clear_error_handler(&self) {
        self.inner.clear_error_handler();
    }

    /// Returns the most recent callback, panic, or best-effort teardown error recorded by the
    /// replacement.
    ///
    /// Callback failures cause Java callers to receive the JNI default value for the method's
    /// return type unless the callback failure preserved a Java exception for rethrow, and the
    /// error is kept here for explicit inspection while the guard is still alive.
    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error()
    }

    /// Returns and clears the most recent callback, panic, or best-effort teardown error recorded
    /// by the replacement.
    pub fn take_last_error(&self) -> Option<String> {
        self.inner.take_last_error()
    }
}

impl JavaHookError {
    pub(crate) fn new(
        kind: MethodKind,
        name: String,
        signature: MethodSignature,
        message: String,
    ) -> Self {
        Self {
            kind,
            name,
            signature,
            message,
        }
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.signature
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for JavaHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}{}: {}",
            hook_kind_name(self.kind),
            self.name,
            self.signature,
            self.message
        )
    }
}

impl JavaHookSet {
    /// Creates an empty hook set.
    pub fn new() -> Self {
        Self { guards: Vec::new() }
    }

    /// Returns the number of guards stored in this set.
    pub fn len(&self) -> usize {
        self.guards.len()
    }

    /// Returns whether this set contains no guards.
    pub fn is_empty(&self) -> bool {
        self.guards.is_empty()
    }

    /// Adds an already-created guard to this set.
    pub fn push(&mut self, guard: JavaHookGuard) {
        self.guards.push(guard);
    }

    /// Replaces `target` and stores the returned guard in this set.
    pub fn replace<T, F>(&mut self, target: T, callback: F) -> Result<&mut JavaHookGuard>
    where
        T: JavaHookTarget,
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        let guard = target.replace(callback)?;
        self.guards.push(guard);
        Ok(self
            .guards
            .last_mut()
            .expect("guard was just pushed into JavaHookSet"))
    }

    /// Restores every guard in reverse installation order.
    ///
    /// All guards are asked to restore even if an earlier restore fails. The first restore error
    /// encountered in reverse order is returned after the remaining guards have been attempted.
    pub fn revert_all(&mut self) -> Result<()> {
        revert_all_in_reverse(&mut self.guards, JavaHookGuard::revert)
    }

    /// Returns the most recent recorded error from each guard that has one.
    pub fn last_errors(&self) -> Vec<String> {
        self.guards
            .iter()
            .filter_map(JavaHookGuard::last_error)
            .collect()
    }
}

fn revert_all_in_reverse<T>(
    guards: &mut [T],
    mut revert: impl FnMut(&mut T) -> Result<()>,
) -> Result<()> {
    let mut first_error = None;
    for guard in guards.iter_mut().rev() {
        if let Err(error) = revert(guard)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

impl JavaHookTarget for JavaMethod {
    fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        JavaMethod::replace(self, callback)
    }
}

impl JavaHookTarget for &JavaMethod {
    fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        JavaMethod::replace(self, callback)
    }
}

/// A constructor-like target that can be replaced only with caller-provided safety guarantees.
pub trait UnsafeJavaHookTarget {
    /// Replaces this constructor-like hook target with a guarded Rust closure.
    ///
    /// # Safety
    ///
    /// Constructor callbacks must initialize the receiver consistently enough for Java code that
    /// observes the object, and must return a void hook return.
    unsafe fn replace_unchecked<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static;
}

impl UnsafeJavaHookTarget for JavaConstructor {
    unsafe fn replace_unchecked<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        unsafe { JavaConstructor::replace_unchecked(self, callback) }
    }
}

impl UnsafeJavaHookTarget for &JavaConstructor {
    unsafe fn replace_unchecked<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        unsafe { JavaConstructor::replace_unchecked(self, callback) }
    }
}

impl<'state> JavaConstructorHookContext<'state> {
    /// Returns whether this replacement is a constructor, static method, or instance method.
    pub fn kind(&self) -> MethodKind {
        self.inner.kind()
    }

    /// Returns the Java member name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Returns the selected constructor signature.
    pub fn signature(&self) -> &MethodSignature {
        self.inner.signature()
    }

    /// Returns a raw JNI environment bound to the active callback.
    pub fn env(&self) -> Result<Env<'state>> {
        self.inner.env()
    }

    /// Returns the constructor receiver being initialized.
    pub fn this_object(&self) -> Result<JavaLocalObject<'state>> {
        self.inner.this_object()
    }

    pub fn args(&self) -> JavaHookArguments<'_, 'state> {
        self.inner.args()
    }

    pub fn arg_value(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        self.inner.arg_value(index)
    }

    pub fn arg_display(&self, index: usize) -> Result<String> {
        self.inner.arg_display(index)
    }

    pub fn arg_is_null(&self, index: usize) -> Result<bool> {
        self.inner.arg_is_null(index)
    }

    pub fn arg<T: FromJavaHookArgument<'state>>(&self, index: usize) -> Result<T> {
        self.inner.arg(index)
    }

    pub fn arg_object(&self, index: usize) -> Result<Option<JavaLocalObject<'state>>> {
        self.inner.arg_object(index)
    }

    pub fn arg_array(&self, index: usize) -> Result<Option<JavaLocalArray<'state>>> {
        self.inner.arg_array(index)
    }

    /// Calls the selected original constructor and returns the initialization proof token.
    pub fn call_original<A: IntoJavaCallArgs>(
        self,
        args: A,
    ) -> Result<JavaConstructorInitialized<'state>> {
        let context = self.inner;
        context.call_original_void(args)?;
        Ok(JavaConstructorInitialized {
            context,
            _sealed: sealed::ConstructorInitialized,
        })
    }

    /// Calls the selected original constructor with the callback's current arguments.
    pub fn call_original_current(self) -> Result<JavaConstructorInitialized<'state>> {
        let args = self.inner.inner.arguments().to_vec();
        self.call_original(args)
    }
}

impl<'state> JavaConstructorInitialized<'state> {
    /// Returns the method-hook context that produced this initialization token.
    pub fn context(&self) -> &JavaHookContext<'state> {
        &self.context
    }

    pub fn kind(&self) -> MethodKind {
        self.context.kind()
    }

    pub fn name(&self) -> &str {
        self.context.name()
    }

    pub fn signature(&self) -> &MethodSignature {
        self.context.signature()
    }

    /// Returns a raw JNI environment bound to the active callback.
    pub fn env(&self) -> Result<Env<'state>> {
        self.context.env()
    }

    /// Returns the initialized constructor receiver.
    pub fn this_object(&self) -> Result<JavaLocalObject<'state>> {
        self.context.this_object()
    }

    pub fn args(&self) -> JavaHookArguments<'_, 'state> {
        self.context.args()
    }

    pub fn arg_value(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        self.context.arg_value(index)
    }

    pub fn arg_display(&self, index: usize) -> Result<String> {
        self.context.arg_display(index)
    }

    pub fn arg_is_null(&self, index: usize) -> Result<bool> {
        self.context.arg_is_null(index)
    }

    pub fn arg<T: FromJavaHookArgument<'state>>(&self, index: usize) -> Result<T> {
        self.context.arg(index)
    }

    pub fn arg_object(&self, index: usize) -> Result<Option<JavaLocalObject<'state>>> {
        self.context.arg_object(index)
    }

    pub fn arg_array(&self, index: usize) -> Result<Option<JavaLocalArray<'state>>> {
        self.context.arg_array(index)
    }
}

impl<'state> JavaHookContext<'state> {
    /// Returns the raw callback `JNIEnv`.
    ///
    /// # Safety
    ///
    /// The returned pointer is valid only while this replacement callback is executing.
    pub unsafe fn raw_env(&self) -> *mut jni::JNIEnv {
        self.inner.env_raw()
    }

    /// Returns a raw JNI environment bound to the active callback.
    ///
    /// Prefer wrapper helpers and typed hook APIs unless code needs direct JNI operations.
    pub fn env(&self) -> Result<Env<'state>> {
        self.inner.env()
    }

    /// Returns whether this replacement is a constructor, static method, or instance method.
    pub fn kind(&self) -> MethodKind {
        self.inner.kind()
    }

    /// Returns the Java member name.
    pub fn name(&self) -> &str {
        self.inner.name()
    }

    /// Returns the selected method or constructor signature.
    pub fn signature(&self) -> &MethodSignature {
        self.inner.signature()
    }

    /// Converts a Rust hook return value while this callback invocation is still alive.
    ///
    /// Use this helper to turn local object/array views, nullable local views, owned wrappers,
    /// primitives, strings, or explicit [`JavaHookReturn`] values into a lifetime-bound callback
    /// return safely.
    pub fn ret<R: IntoJavaHookReturn<'state>>(&self, value: R) -> Result<JavaHookReturn<'state>> {
        value.into_hook_return_for(
            self.inner.env_raw(),
            &self.inner.state.vm,
            self.signature().return_type(),
            "JavaHookContext::ret",
        )
    }

    /// Returns the raw class argument for a static-method hook.
    ///
    /// # Safety
    ///
    /// The returned local reference is valid only while this replacement callback is executing.
    pub unsafe fn raw_class(&self) -> Option<jni::jclass> {
        self.inner.class()
    }

    /// Returns the raw receiver argument for an instance-method or constructor hook.
    ///
    /// # Safety
    ///
    /// The returned local reference is valid only while this replacement callback is executing.
    pub unsafe fn raw_receiver(&self) -> Option<jni::jobject> {
        self.inner.receiver()
    }

    /// Returns the current Java `this` object for an instance-method or constructor hook.
    ///
    /// Static method hooks do not have a `this` object; use
    /// [`JavaHookContext::maybe_this_object`] when handling static and instance hooks through the
    /// same callback path.
    pub fn this_object(&self) -> Result<JavaLocalObject<'state>> {
        self.receiver_object("JavaHookContext::this_object")?
            .ok_or(Error::WrongMethodKind {
                operation: "JavaHookContext::this_object",
            })
    }

    /// Returns the current Java `this` object when this hook has one.
    ///
    /// This returns `Ok(None)` for static-method hooks.
    pub fn maybe_this_object(&self) -> Result<Option<JavaLocalObject<'state>>> {
        self.receiver_object("JavaHookContext::maybe_this_object")
    }

    fn receiver_object(&self, operation: &'static str) -> Result<Option<JavaLocalObject<'state>>> {
        self.inner
            .receiver()
            .map(|receiver| {
                self.local_object_with_class(
                    receiver,
                    JavaClass::from_raw(self.inner.state.target_class.clone()),
                    operation,
                )
            })
            .transpose()
    }

    /// Calls the replaced method's original implementation and returns the raw hook return lane.
    ///
    /// # Safety
    ///
    /// Object references in the returned value are valid only while this replacement callback is
    /// executing. Prefer the typed original-call helpers for safe object and array views.
    pub unsafe fn call_original_raw<A: IntoJavaCallArgs>(
        &self,
        args: A,
    ) -> Result<JavaHookReturn<'state>> {
        let original = unsafe { self.inner.call_original(args)? };
        Ok(JavaHookReturn::from_raw_for_type(
            original,
            self.signature().return_type(),
        ))
    }

    /// Forwards this invocation to the original implementation and returns the raw hook lane.
    ///
    /// This is the raw pass-through hook shorthand for callbacks that only observe the call or
    /// perform side effects before returning the original result. Object references in the
    /// returned value are callback-local JNI references, but the returned token is bound to this
    /// callback lifetime and raw reference extraction remains explicit through unsafe
    /// [`JavaHookReturn`] methods.
    pub fn proceed(&self) -> Result<JavaHookReturn<'state>> {
        unsafe { self.call_original_raw(self.inner.arguments()) }
    }

    /// Calls the replaced method's original implementation with the callback's current arguments.
    ///
    /// The return value is extracted through [`FromJavaHookReturn`], so object and array returns
    /// borrow from this callback instead of escaping as raw JNI references. Use
    /// [`JavaHookContext::proceed`] when the callback wants to return the raw original result
    /// unchanged, or [`JavaHookContext::call_original_raw`] when raw callback-local handles are
    /// required with explicit replacement arguments.
    pub fn call_original_current<T>(&self) -> Result<T>
    where
        T: FromJavaHookReturn<'state>,
    {
        self.call_original(self.inner.arguments())
    }

    /// Calls the replaced method's original implementation and extracts a typed return value.
    ///
    /// This is a readable alias for [`JavaHookContext::call_original`] when the callback wants to
    /// forward or adjust a returned value. Use [`JavaHookContext::call_original_raw`] for explicit
    /// raw JNI return handling.
    pub fn call_original_return<T>(&self, args: impl IntoJavaCallArgs) -> Result<T>
    where
        T: FromJavaHookReturn<'state>,
    {
        self.call_original(args)
    }

    pub fn call_original<T>(&self, args: impl IntoJavaCallArgs) -> Result<T>
    where
        T: FromJavaHookReturn<'state>,
    {
        T::from_hook_return(
            unsafe { self.call_original_raw(args)? },
            self,
            "JavaHookContext::call_original",
        )
    }

    pub fn call_original_void<A: IntoJavaCallArgs>(&self, args: A) -> Result<()> {
        unsafe { self.call_original_raw(args)? }.into_void("JavaHookContext::call_original_void")
    }

    pub fn call_original_object<A: IntoJavaCallArgs>(
        &self,
        args: A,
    ) -> Result<Option<JavaLocalObject<'state>>> {
        match unsafe { self.call_original_raw(args)? } {
            JavaHookReturn::Object(value) => value
                .map(|object| {
                    self.local_object_for_return(
                        object.as_jobject(),
                        "JavaHookContext::call_original_object",
                    )
                })
                .transpose(),
            other => Err(invalid_hook_return(
                "JavaHookContext::call_original_object",
                "object",
                other,
            )),
        }
    }

    pub fn call_original_array<A: IntoJavaCallArgs>(
        &self,
        args: A,
    ) -> Result<Option<JavaLocalArray<'state>>> {
        let element_type = match self.signature().return_type() {
            JavaType::Array(element) => (**element).clone(),
            actual => {
                return Err(Error::InvalidReturnType {
                    operation: "JavaHookContext::call_original_array",
                    expected: "array",
                    actual: actual.to_string(),
                });
            }
        };

        match unsafe { self.call_original_raw(args)? } {
            JavaHookReturn::Object(value) => value
                .map(|array| {
                    self.local_array(
                        array.as_jobject(),
                        element_type,
                        "JavaHookContext::call_original_array",
                    )
                })
                .transpose(),
            other => Err(invalid_hook_return(
                "JavaHookContext::call_original_array",
                "array",
                other,
            )),
        }
    }

    pub(super) fn local_object_for_return(
        &self,
        value: jni::jobject,
        operation: &'static str,
    ) -> Result<JavaLocalObject<'state>> {
        self.local_object_for_type(value, self.signature().return_type(), operation)
    }

    fn local_object_for_type(
        &self,
        value: jni::jobject,
        ty: &JavaType,
        operation: &'static str,
    ) -> Result<JavaLocalObject<'state>> {
        let JavaType::Object(name) = ty else {
            return Err(Error::InvalidReturnType {
                operation,
                expected: "object",
                actual: ty.to_string(),
            });
        };
        let class = self.class_for_declared_object(name)?;
        self.local_object_with_class(value, class, operation)
    }

    pub(super) fn local_object_with_class(
        &self,
        value: jni::jobject,
        class: JavaClass,
        operation: &'static str,
    ) -> Result<JavaLocalObject<'state>> {
        if value.is_null() {
            return Err(Error::NullReturn { operation });
        }
        let object = unsafe { JavaLocalObject::from_raw_with_class(class.clone(), value)? };
        if class.is_instance(&object)? {
            Ok(object)
        } else {
            let env = self.env()?;
            let actual = env.get_object_class(&object)?;
            Err(Error::InvalidObjectType {
                operation,
                expected: "declared object type",
                actual: format!("{:p} is not {}", actual.as_jclass(), class.name()),
            })
        }
    }

    pub(super) fn class_for_declared_object(&self, name: &str) -> Result<JavaClass> {
        let env = self.env()?;
        let java = self.inner.state.vm.java();
        let scoped_java = match metadata::class_loader(&env, &java, &self.inner.state.target_class)?
        {
            Some(loader) => java.with_loader(&loader),
            None => java,
        };
        Ok(JavaClass::from_raw(
            scoped_java.find_class(&name.replace('/', "."))?,
        ))
    }

    pub(super) fn local_array(
        &self,
        value: jni::jobject,
        element_type: JavaType,
        operation: &'static str,
    ) -> Result<JavaLocalArray<'state>> {
        if value.is_null() {
            return Err(Error::NullReturn { operation });
        }
        let env = self.env()?;
        unsafe { JavaLocalArray::from_raw(env.vm().clone(), value, element_type) }
    }
}

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
            callback(JavaHookContext { inner: invocation }).and_then(|value| {
                let hook_return =
                    validate_reference_return(env, &return_class, &return_type, value)?;
                Ok(hook_return.into_raw())
            })
        })
    }?;
    Ok(JavaHookGuard { inner })
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
            let context = JavaConstructorHookContext {
                inner: JavaHookContext { inner: invocation },
            };
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(context)));
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
    Ok(JavaHookGuard { inner })
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
            callback(JavaHookContext { inner: invocation }).and_then(|value| {
                value
                    .coerce_for_return_type(&return_type, "closure replacement return")
                    .map(JavaHookReturn::into_raw)
            })
        })
    }?;
    Ok(JavaHookGuard { inner })
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

fn hook_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Static => "static method",
        MethodKind::Instance => "instance method",
        MethodKind::Constructor => "constructor",
    }
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
    fn hook_set_revert_loop_attempts_every_guard_after_failure() {
        #[derive(Debug)]
        struct FakeGuard {
            id: usize,
            result: Result<()>,
        }

        let first_error = Error::UnsupportedFeature {
            feature: "test hook revert",
            reason: "middle guard failed".to_owned(),
        };
        let later_error = Error::UnsupportedFeature {
            feature: "test hook revert",
            reason: "newest guard failed".to_owned(),
        };
        let mut guards = [
            FakeGuard {
                id: 0,
                result: Ok(()),
            },
            FakeGuard {
                id: 1,
                result: Err(first_error.clone()),
            },
            FakeGuard {
                id: 2,
                result: Err(later_error.clone()),
            },
        ];
        let mut attempted = Vec::new();

        let result = revert_all_in_reverse(&mut guards, |guard| {
            attempted.push(guard.id);
            guard.result.clone()
        });

        assert_eq!(attempted, [2, 1, 0]);
        assert_eq!(result, Err(later_error));
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
