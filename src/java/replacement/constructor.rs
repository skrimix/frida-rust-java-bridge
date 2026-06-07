use crate::{
    Result,
    env::{Env, MethodKind},
    java::{IntoJavaCallArgs, JavaLocalArray, JavaLocalObject},
    signature::MethodSignature,
};

use super::{
    arguments::{FromJavaHookArgument, JavaHookArgument, JavaHookArguments},
    context::JavaHookContext,
};

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

impl<'state> JavaConstructorHookContext<'state> {
    pub(super) fn from_context(inner: JavaHookContext<'state>) -> Self {
        Self { inner }
    }

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
