use crate::{
    Error, Result,
    java::{JavaLocalArray, JavaLocalObject, display_java_char},
    jni,
    signature::JavaType,
    value::{JavaValue, RawJavaObject},
};

use super::context::JavaHookContext;

/// Untyped callback-argument inspection view.
///
/// Prefer [`JavaHookContext::arg`], [`JavaHookContext::arg_object`],
/// and [`JavaHookContext::arg_array`] in hooks that know the expected argument shape.
pub struct JavaHookArguments<'context, 'state> {
    context: &'context JavaHookContext<'state>,
}

/// Iterator over untyped callback-argument inspection values.
pub struct JavaHookArgumentsIter<'context, 'state> {
    context: &'context JavaHookContext<'state>,
    index: usize,
}

/// Reference payload used by safely inspectable hook arguments.
///
/// Object and array values borrow from the active callback. Call `retain()` on the returned local
/// wrapper when the value must outlive the callback.
#[derive(Debug)]
pub enum JavaHookArgumentRef<'state> {
    Object(JavaLocalObject<'state>),
    Array(JavaLocalArray<'state>),
}

/// One safely inspectable replacement argument.
pub type JavaHookArgument<'state> = JavaValue<JavaHookArgumentRef<'state>>;

mod sealed {
    pub trait FromJavaValueSealed {}
}

/// Converts one primitive replacement argument into a typed Rust value.
///
/// Object-like arguments need callback context so the crate can build lifetime-bound local views;
/// use [`JavaHookContext::arg`], [`JavaHookContext::arg_object`], or
/// [`JavaHookContext::arg_array`] for those values.
pub trait FromJavaValue: sealed::FromJavaValueSealed + Sized {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self>;
}

/// Converts one replacement argument into a typed Rust value with access to the hook context.
///
/// This powers [`JavaHookContext::arg`]. Primitive conversions are provided through
/// [`FromJavaValue`], while context-aware conversions such as `String` can read JNI-backed
/// references safely during the callback.
pub(super) trait FromJavaHookArgument<'state>: Sized {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self>;
}

impl<'state> JavaHookContext<'state> {
    /// Returns an untyped inspection view over the callback arguments.
    ///
    /// Typed hooks should usually prefer [`JavaHookContext::arg`],
    /// [`JavaHookContext::arg_object`], or [`JavaHookContext::arg_array`].
    pub fn args(&self) -> JavaHookArguments<'_, 'state> {
        JavaHookArguments { context: self }
    }

    /// Returns one untyped inspection value.
    ///
    /// Prefer typed helpers when the callback knows the expected argument type.
    pub fn arg_value(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        self.hook_argument(index)
    }

    /// Returns one argument formatted for diagnostic logging.
    ///
    /// Primitive values are formatted directly, null reference lanes are rendered as `null`,
    /// `java.lang.String` values are extracted as Rust strings, and other references use Java's
    /// `toString()` implementation.
    ///
    /// This is a convenience shorthand for `invocation.arg_value(index)?.java_display()`.
    pub fn arg_display(&self, index: usize) -> Result<String> {
        self.arg_value(index)?.java_display()
    }

    /// Returns whether one object or array argument is Java `null`.
    pub fn arg_is_null(&self, index: usize) -> Result<bool> {
        match self.signature().arguments().get(index) {
            Some(JavaType::Object(_)) | Some(JavaType::Array(_)) => {}
            Some(actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "reference".to_owned(),
                    actual: actual.jni_return_name(),
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: self.inner.arguments().len(),
                });
            }
        }

        match self.argument_value(index)? {
            JavaValue::Object(value) => Ok(value.is_none()),
            JavaValue::Void => Err(invalid_java_value(index, "reference", JavaValue::Void)),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }

    /// Returns the raw callback arguments.
    ///
    /// # Safety
    ///
    /// Object references in the returned values are valid only while this replacement callback is
    /// executing. Use [`JavaHookContext::args`] for safe iterable argument views.
    pub unsafe fn raw_arguments(&self) -> &[JavaValue] {
        self.inner.arguments()
    }

    /// Returns a raw object-like argument.
    ///
    /// # Safety
    ///
    /// The returned raw reference is valid only while this replacement callback is executing.
    pub unsafe fn raw_arg_object(&self, index: usize) -> Result<Option<RawJavaObject>> {
        match self.argument_value(index)? {
            JavaValue::Object(None) => Ok(None),
            JavaValue::Object(Some(value)) => Ok(Some(value)),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }

    /// Extracts one argument through a typed conversion.
    pub fn arg<T: FromJavaHookArgument<'state>>(&self, index: usize) -> Result<T> {
        let value = self
            .inner
            .arguments()
            .get(index)
            .copied()
            .ok_or(Error::InvalidArguments {
                expected: index + 1,
                actual: self.inner.arguments().len(),
            })?;
        T::from_hook_argument(self, value, index)
    }

    /// Returns one object-like argument as a callback-local object view.
    pub fn arg_object(&self, index: usize) -> Result<Option<JavaLocalObject<'state>>> {
        match self.argument_value(index)? {
            JavaValue::Object(None) => Ok(None),
            JavaValue::Object(Some(value)) => self
                .local_object_for_argument(index, value.as_jobject(), "JavaHookContext::arg_object")
                .map(Some),
            other => Err(invalid_java_value(index, "reference", other)),
        }
    }

    /// Returns one array argument as a callback-local array view.
    pub fn arg_array(&self, index: usize) -> Result<Option<JavaLocalArray<'state>>> {
        let element_type = match self.signature().arguments().get(index) {
            Some(JavaType::Array(element)) => (**element).clone(),
            Some(_actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "array".to_owned(),
                    actual: "non-array",
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: self.inner.arguments().len(),
                });
            }
        };

        match self.argument_value(index)? {
            JavaValue::Object(None) => Ok(None),
            JavaValue::Object(Some(value)) => self
                .local_array(
                    value.as_jobject(),
                    element_type,
                    "JavaHookContext::arg_array",
                )
                .map(Some),
            other => Err(invalid_java_value(index, "array", other)),
        }
    }

    fn argument_value(&self, index: usize) -> Result<JavaValue> {
        self.inner
            .arguments()
            .get(index)
            .copied()
            .ok_or(Error::InvalidArguments {
                expected: index + 1,
                actual: self.inner.arguments().len(),
            })
    }

    fn hook_argument(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        let value = self.argument_value(index)?;
        Ok(match value {
            JavaValue::Void => {
                return Err(invalid_java_value(index, "argument", JavaValue::Void));
            }
            JavaValue::Boolean(value) => JavaHookArgument::Boolean(value),
            JavaValue::Byte(value) => JavaHookArgument::Byte(value),
            JavaValue::Char(value) => JavaHookArgument::Char(value),
            JavaValue::Short(value) => JavaHookArgument::Short(value),
            JavaValue::Int(value) => JavaHookArgument::Int(value),
            JavaValue::Long(value) => JavaHookArgument::Long(value),
            JavaValue::Float(value) => JavaHookArgument::Float(value),
            JavaValue::Double(value) => JavaHookArgument::Double(value),
            JavaValue::Object(None) => self.null_reference_argument(index)?,
            JavaValue::Object(Some(value)) => match self.signature().arguments().get(index) {
                Some(JavaType::Array(element)) => {
                    JavaHookArgument::Object(Some(JavaHookArgumentRef::Array(self.local_array(
                        value.as_jobject(),
                        (**element).clone(),
                        "JavaHookContext::arg_value",
                    )?)))
                }
                Some(JavaType::Object(_)) => JavaHookArgument::Object(Some(
                    JavaHookArgumentRef::Object(self.local_object_for_argument(
                        index,
                        value.as_jobject(),
                        "JavaHookContext::arg_value",
                    )?),
                )),
                Some(other) => {
                    return Err(Error::InvalidArgumentType {
                        index,
                        expected: other.to_string(),
                        actual: "object",
                    });
                }
                None => {
                    return Err(Error::InvalidArguments {
                        expected: index + 1,
                        actual: self.inner.arguments().len(),
                    });
                }
            },
        })
    }

    fn null_reference_argument(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        match self.signature().arguments().get(index) {
            Some(JavaType::Array(_)) => Ok(JavaHookArgument::Object(None)),
            Some(JavaType::Object(_)) => Ok(JavaHookArgument::Object(None)),
            Some(other) => Err(Error::InvalidArgumentType {
                index,
                expected: other.to_string(),
                actual: "null",
            }),
            None => Err(Error::InvalidArguments {
                expected: index + 1,
                actual: self.inner.arguments().len(),
            }),
        }
    }

    fn local_object_for_argument(
        &self,
        index: usize,
        value: jni::jobject,
        operation: &'static str,
    ) -> Result<JavaLocalObject<'state>> {
        let name = match self.signature().arguments().get(index) {
            Some(JavaType::Object(name)) => name,
            Some(JavaType::Array(_)) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "object".to_owned(),
                    actual: "array",
                });
            }
            Some(actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "object".to_owned(),
                    actual: actual.jni_return_name(),
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: self.inner.arguments().len(),
                });
            }
        };
        let class = self.class_for_declared_object(name)?;
        self.local_object_with_class(value, class, operation)
    }
}

impl<'context, 'state> JavaHookArguments<'context, 'state> {
    pub fn len(&self) -> usize {
        self.context.inner.arguments().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Result<JavaHookArgument<'state>> {
        self.context.arg_value(index)
    }

    pub fn iter(&self) -> JavaHookArgumentsIter<'context, 'state> {
        JavaHookArgumentsIter {
            context: self.context,
            index: 0,
        }
    }
}

impl<'context, 'state> IntoIterator for JavaHookArguments<'context, 'state> {
    type Item = Result<JavaHookArgument<'state>>;
    type IntoIter = JavaHookArgumentsIter<'context, 'state>;

    fn into_iter(self) -> Self::IntoIter {
        JavaHookArgumentsIter {
            context: self.context,
            index: 0,
        }
    }
}

impl<'context, 'state> Iterator for JavaHookArgumentsIter<'context, 'state> {
    type Item = Result<JavaHookArgument<'state>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.index >= self.context.inner.arguments().len() {
            return None;
        }
        let index = self.index;
        self.index += 1;
        Some(self.context.arg_value(index))
    }
}

impl JavaValue<JavaHookArgumentRef<'_>> {
    pub fn java_display(&self) -> Result<String> {
        Ok(match self {
            Self::Void => "void".to_owned(),
            Self::Boolean(value) => value.to_string(),
            Self::Byte(value) => value.to_string(),
            Self::Char(value) => display_java_char(*value),
            Self::Short(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Long(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Double(value) => value.to_string(),
            Self::Object(Some(JavaHookArgumentRef::Object(value))) => value.java_display()?,
            Self::Object(Some(JavaHookArgumentRef::Array(value))) => value.java_display()?,
            Self::Object(None) => "null".to_owned(),
        })
    }
}

macro_rules! impl_java_value_conversion {
    ($type:ty, $value_variant:ident, $name:literal) => {
        impl sealed::FromJavaValueSealed for $type {}

        impl FromJavaValue for $type {
            fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
                match value {
                    JavaValue::$value_variant(value) => Ok(value),
                    other => Err(invalid_java_value(index, $name, other)),
                }
            }
        }
    };
}

impl_java_value_conversion!(jni::jbyte, Byte, "byte");
impl_java_value_conversion!(jni::jchar, Char, "char");
impl_java_value_conversion!(jni::jshort, Short, "short");
impl_java_value_conversion!(jni::jint, Int, "int");
impl_java_value_conversion!(jni::jlong, Long, "long");
impl_java_value_conversion!(jni::jfloat, Float, "float");
impl_java_value_conversion!(jni::jdouble, Double, "double");

impl sealed::FromJavaValueSealed for bool {}

impl FromJavaValue for bool {
    fn from_java_value(value: JavaValue, index: usize) -> Result<Self> {
        match value {
            JavaValue::Boolean(value) => Ok(value),
            other => Err(invalid_java_value(index, "boolean", other)),
        }
    }
}

impl<'state, T> FromJavaHookArgument<'state> for T
where
    T: FromJavaValue,
{
    fn from_hook_argument(
        _context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        T::from_java_value(value, index)
    }
}

impl<'state> FromJavaHookArgument<'state> for Option<JavaLocalObject<'state>> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        match context.signature().arguments().get(index) {
            Some(JavaType::Object(_)) => {}
            Some(JavaType::Array(_)) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "object".to_owned(),
                    actual: "array",
                });
            }
            Some(actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "object".to_owned(),
                    actual: actual.jni_return_name(),
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: context.inner.arguments().len(),
                });
            }
        }

        match value {
            JavaValue::Object(None) => Ok(None),
            JavaValue::Object(Some(value)) => context
                .local_object_for_argument(index, value.as_jobject(), "JavaHookContext::arg")
                .map(Some),
            other => Err(invalid_java_value(index, "object", other)),
        }
    }
}

impl<'state> FromJavaHookArgument<'state> for JavaLocalObject<'state> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        Option::<JavaLocalObject<'state>>::from_hook_argument(context, value, index)?.ok_or(
            Error::NullReturn {
                operation: "JavaHookContext::arg",
            },
        )
    }
}

impl<'state> FromJavaHookArgument<'state> for Option<JavaLocalArray<'state>> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        let element_type = match context.signature().arguments().get(index) {
            Some(JavaType::Array(element)) => (**element).clone(),
            Some(JavaType::Object(_)) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "array".to_owned(),
                    actual: "object",
                });
            }
            Some(actual) => {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: "array".to_owned(),
                    actual: actual.jni_return_name(),
                });
            }
            None => {
                return Err(Error::InvalidArguments {
                    expected: index + 1,
                    actual: context.inner.arguments().len(),
                });
            }
        };

        match value {
            JavaValue::Object(None) => Ok(None),
            JavaValue::Object(Some(value)) => context
                .local_array(value.as_jobject(), element_type, "JavaHookContext::arg")
                .map(Some),
            other => Err(invalid_java_value(index, "array", other)),
        }
    }
}

impl<'state> FromJavaHookArgument<'state> for JavaLocalArray<'state> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        Option::<JavaLocalArray<'state>>::from_hook_argument(context, value, index)?.ok_or(
            Error::NullReturn {
                operation: "JavaHookContext::arg",
            },
        )
    }
}

impl<'state> FromJavaHookArgument<'state> for Option<String> {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        match value {
            JavaValue::Object(None) => Ok(None),
            JavaValue::Object(Some(value)) => context
                .local_object_for_argument(index, value.as_jobject(), "JavaHookContext::arg")?
                .get_string()
                .map(Some),
            other => Err(invalid_java_value(index, "java.lang.String", other)),
        }
    }
}

impl<'state> FromJavaHookArgument<'state> for String {
    fn from_hook_argument(
        context: &JavaHookContext<'state>,
        value: JavaValue,
        index: usize,
    ) -> Result<Self> {
        Option::<String>::from_hook_argument(context, value, index)?.ok_or(Error::NullReturn {
            operation: "JavaHookContext::arg",
        })
    }
}

fn invalid_java_value(index: usize, expected: &'static str, actual: JavaValue) -> Error {
    Error::InvalidArgumentType {
        index,
        expected: expected.to_owned(),
        actual: actual.type_name(),
    }
}
