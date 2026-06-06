use crate::{
    Error, Result,
    env::{Env, MethodKind},
    java::{IntoJavaCallArgs, Java, JavaClass, JavaLocalArray, JavaLocalObject},
    jni, metadata,
    refs::AsJClass,
    signature::{JavaType, MethodSignature},
};

use super::{
    closure::ReplacementInvocation,
    returns::{FromJavaHookReturn, IntoJavaHookReturn, JavaHookReturn, invalid_hook_return},
};

/// Invocation details passed to an installed method replacement.
///
/// A `JavaHookContext` value is valid only while Java is executing the replacement callback. Use it
/// to inspect arguments, get `this`, call the original implementation, create callback-local return
/// values, or access the raw JNI layer when needed.
pub struct JavaHookContext<'state> {
    pub(crate) inner: ReplacementInvocation<'state>,
}

impl<'state> JavaHookContext<'state> {
    pub(super) fn from_invocation(inner: ReplacementInvocation<'state>) -> Self {
        Self { inner }
    }

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
        let java = Java::new(self.inner.state.vm.clone());
        let scoped_java = match metadata::class_loader(
            &env,
            &self.inner.state.vm,
            &self.inner.state.target_class,
        )? {
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
