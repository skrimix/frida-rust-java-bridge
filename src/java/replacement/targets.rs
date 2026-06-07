use crate::{
    Result,
    java::{
        JavaBoundMethodGroup, JavaBoundMethodOverload, JavaClass, JavaConstructor, JavaMethod,
        JavaMethodGroup,
    },
};

use super::{
    api::JavaHookGuard,
    constructor::{JavaConstructorHookContext, JavaConstructorInitialized},
    context::JavaHookContext,
    install::{install_constructor_hook, install_constructor_hook_unchecked, install_method_hook},
    returns::JavaHookReturn,
};

impl JavaClass {
    pub fn replace<F>(&self, name: &str, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.method(name)?.replace(callback)
    }

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
    /// The callback must call the selected original constructor through the supplied constructor
    /// context and return the resulting initialization token.
    pub fn replace_constructor<'types, F>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaConstructorHookContext<'a>) -> Result<JavaConstructorInitialized<'a>>
            + Send
            + Sync
            + 'static,
    {
        let constructor = self.constructor(arguments)?;
        constructor.replace(callback)
    }

    /// Replaces the selected constructor overload without enforcing original-constructor
    /// initialization.
    ///
    /// # Safety
    ///
    /// Constructor callbacks must initialize the receiver consistently enough for Java code that
    /// observes the object, and must return void.
    pub unsafe fn replace_constructor_unchecked<'types, F>(
        &self,
        arguments: impl AsRef<[&'types str]>,
        callback: F,
    ) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        let constructor = self.constructor(arguments)?;
        unsafe { constructor.replace_unchecked(callback) }
    }
}

impl JavaMethodGroup {
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.unambiguous()?.replace(callback)
    }

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
    /// The callback receives
    /// [`JavaConstructorHookContext`](crate::java::replacement::JavaConstructorHookContext)
    /// with `kind()` set to [`MethodKind::Constructor`](crate::env::MethodKind::Constructor),
    /// `name()` set to `"<init>"`, and `this_object()` pointing at the object being initialized.
    /// The callback must call the original constructor through `call_original()` or
    /// `call_original_current()` and return the resulting initialization token. Keep the returned
    /// guard alive while the replacement should remain active; reverting or dropping it restores the
    /// original constructor.
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaConstructorHookContext<'a>) -> Result<JavaConstructorInitialized<'a>>
            + Send
            + Sync
            + 'static,
    {
        unsafe { install_constructor_hook(self, callback) }
    }

    /// Replaces this selected constructor overload without enforcing original-constructor
    /// initialization.
    ///
    /// # Safety
    ///
    /// This is backed by ART method replacement. Constructor callbacks must initialize the receiver
    /// consistently enough for Java code that observes the object, and should return through
    /// [`JavaHookContext::ret`](crate::java::replacement::JavaHookContext::ret).
    pub unsafe fn replace_unchecked<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        unsafe { install_constructor_hook_unchecked(self, callback) }
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
    pub fn replace<F>(&self, callback: F) -> Result<JavaHookGuard>
    where
        F: for<'a> Fn(JavaHookContext<'a>) -> Result<JavaHookReturn<'a>> + Send + Sync + 'static,
    {
        self.unambiguous()?.overload().replace(callback)
    }

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
