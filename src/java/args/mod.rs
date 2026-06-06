use super::*;

mod call;
mod containers;
mod conversion;
mod field;

#[cfg(test)]
mod tests;

pub(crate) use conversion::can_coerce_java_value;

// Argument conversion precedence:
// - Rust strings become temporary jstrings for java.lang.String/Object/CharSequence targets.
// - Local Java wrappers pass their raw object references without becoming general JavaValue impls.
// - JavaValue-compatible primitives/references use descriptor-directed coercion.
// - Containers and tuples prepare each argument after overload selection supplies parameter types.
pub(crate) struct PreparedJavaCallArg {
    value: JavaValue,
    local_ref: Option<jni::jobject>,
}

impl<'env, 'vm> PreparedJavaCallArgs<'env, 'vm> {
    fn with_capacity(capacity: usize, cleanup_env: &'env Env<'vm>) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            local_refs: Vec::new(),
            cleanup_env,
        }
    }

    fn push(&mut self, arg: PreparedJavaCallArg) {
        self.values.push(arg.value);
        if let Some(local_ref) = arg.local_ref {
            self.local_refs.push(local_ref);
        }
    }

    fn validate_len(&self, expected: &[JavaType]) -> Result<()> {
        if self.values.len() == expected.len() {
            Ok(())
        } else {
            Err(Error::InvalidArguments {
                expected: expected.len(),
                actual: self.values.len(),
            })
        }
    }

    pub(crate) fn values(&self) -> &[JavaValue] {
        &self.values
    }

    pub(crate) fn into_parts(mut self) -> (Vec<JavaValue>, Vec<jni::jobject>) {
        (
            std::mem::take(&mut self.values),
            std::mem::take(&mut self.local_refs),
        )
    }
}

impl Drop for PreparedJavaCallArgs<'_, '_> {
    fn drop(&mut self) {
        for local_ref in self.local_refs.drain(..) {
            unsafe { self.cleanup_env.delete_local_ref_raw(local_ref) };
        }
    }
}

impl PreparedJavaFieldValue {
    fn new(value: JavaValue, local_ref: Option<jni::jobject>) -> Self {
        Self { value, local_ref }
    }

    pub(crate) fn value(&self) -> JavaValue {
        self.value
    }

    pub(crate) fn delete_local_ref(self, env: &Env<'_>) {
        if let Some(local_ref) = self.local_ref {
            unsafe { env.delete_local_ref_raw(local_ref) };
        }
    }
}

impl<'vm> AttachedJavaCallArgs<'vm> {
    pub(crate) fn new<A: IntoJavaCallArgs>(
        vm: &'vm Vm,
        expected: &[JavaType],
        args: A,
    ) -> Result<Self> {
        let env = vm.attach_current_thread()?;
        let prepared = args.into_java_call_args(&env, expected)?;
        prepared.validate_len(expected)?;
        let (values, local_refs) = prepared.into_parts();
        Ok(Self {
            env,
            values,
            local_refs,
        })
    }

    pub(crate) fn values(&self) -> &[JavaValue] {
        &self.values
    }
}

impl JavaOverloadArg {
    pub(crate) fn type_name(&self) -> &'static str {
        match self {
            Self::Value(value) => value.type_name(),
            Self::RustString(_) => "string",
        }
    }
}

impl Drop for AttachedJavaCallArgs<'_> {
    fn drop(&mut self) {
        for local_ref in self.local_refs.drain(..) {
            unsafe { self.env.delete_local_ref_raw(local_ref) };
        }
    }
}

impl JavaArgs {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
        }
    }

    pub fn push<V>(&mut self, value: V)
    where
        V: Into<JavaValue>,
    {
        self.values.push(value.into());
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn as_slice(&self) -> &[JavaValue] {
        &self.values
    }

    pub fn into_vec(self) -> Vec<JavaValue> {
        self.values
    }
}

impl From<Vec<JavaValue>> for JavaArgs {
    fn from(values: Vec<JavaValue>) -> Self {
        Self { values }
    }
}

impl<const N: usize> From<[JavaValue; N]> for JavaArgs {
    fn from(values: [JavaValue; N]) -> Self {
        Self {
            values: values.to_vec(),
        }
    }
}
