use super::*;

pub(crate) trait JavaCallArg {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg>;
}

pub(crate) struct PreparedJavaArg {
    value: JavaValue,
    local_ref: Option<jni::jobject>,
}

impl PreparedJavaArgValues {
    fn empty() -> Self {
        Self {
            values: Vec::new(),
            local_refs: Vec::new(),
        }
    }

    fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            local_refs: Vec::new(),
        }
    }

    fn push(&mut self, arg: PreparedJavaArg) {
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

impl<'vm> PreparedJavaArgs<'vm> {
    pub(crate) fn new<A: IntoJavaCallArgs>(
        vm: &'vm Vm,
        expected: &[JavaType],
        args: A,
    ) -> Result<Self> {
        let env = vm.attach_current_thread()?;
        let prepared = args.into_java_call_args(&env, expected)?;
        prepared.validate_len(expected)?;
        Ok(Self {
            env,
            values: prepared.values,
            local_refs: prepared.local_refs,
        })
    }

    pub(crate) fn values(&self) -> &[JavaValue] {
        &self.values
    }
}

impl Drop for PreparedJavaArgs<'_> {
    fn drop(&mut self) {
        for local_ref in self.local_refs.drain(..) {
            unsafe { self.env.delete_local_ref_raw(local_ref) };
        }
    }
}

impl IntoJavaArgs for () {
    fn into_java_args(self) -> Vec<JavaValue> {
        Vec::new()
    }
}

impl IntoJavaArgs for Vec<JavaValue> {
    fn into_java_args(self) -> Vec<JavaValue> {
        self
    }
}

impl IntoJavaArgs for &[JavaValue] {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.to_vec()
    }
}

impl<const N: usize> IntoJavaArgs for [JavaValue; N] {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.to_vec()
    }
}

impl<const N: usize> IntoJavaArgs for &[JavaValue; N] {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.to_vec()
    }
}

macro_rules! impl_into_java_args_for_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> IntoJavaArgs for ($($name,)+)
        where
            $($name: Into<JavaValue>),+
        {
            fn into_java_args(self) -> Vec<JavaValue> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                vec![$($name.into()),+]
            }
        }
    };
}

impl_into_java_args_for_tuple!(A);
impl_into_java_args_for_tuple!(A, B);
impl_into_java_args_for_tuple!(A, B, C);
impl_into_java_args_for_tuple!(A, B, C, D);
impl_into_java_args_for_tuple!(A, B, C, D, E);
impl_into_java_args_for_tuple!(A, B, C, D, E, F);
impl_into_java_args_for_tuple!(A, B, C, D, E, F, G);
impl_into_java_args_for_tuple!(A, B, C, D, E, F, G, H);

impl IntoJavaCallArgs for () {
    fn into_java_call_args(
        self,
        _env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaArgValues> {
        let values = PreparedJavaArgValues::empty();
        values.validate_len(expected)?;
        Ok(values)
    }
}

impl<A: JavaCallArg> IntoJavaCallArgs for Vec<A> {
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaArgValues> {
        prepare_call_args(self, env, expected)
    }
}

impl<const N: usize, A: JavaCallArg> IntoJavaCallArgs for [A; N] {
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaArgValues> {
        prepare_call_args(self, env, expected)
    }
}

impl<'a, A> IntoJavaCallArgs for &'a [A]
where
    &'a A: JavaCallArg,
{
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaArgValues> {
        prepare_call_args(self.iter(), env, expected)
    }
}

impl<'a, const N: usize, A> IntoJavaCallArgs for &'a [A; N]
where
    &'a A: JavaCallArg,
{
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaArgValues> {
        self.as_slice().into_java_call_args(env, expected)
    }
}

macro_rules! impl_into_java_call_args_for_tuple {
    ($actual:expr; $(($name:ident, $index:tt)),+ $(,)?) => {
        impl<$($name),+> IntoJavaCallArgs for ($($name,)+)
        where
            $($name: JavaCallArg),+
        {
            fn into_java_call_args(
                self,
                env: &Env<'_>,
                expected: &[JavaType],
            ) -> Result<PreparedJavaArgValues> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                if expected.len() != $actual {
                    return Err(Error::InvalidArguments {
                        expected: expected.len(),
                        actual: $actual,
                    });
                }
                let mut values = PreparedJavaArgValues::with_capacity($actual);
                $(values.push($name.into_java_call_arg(env, &expected[$index], $index)?);)+
                values.validate_len(expected)?;
                Ok(values)
            }
        }
    };
}

impl_into_java_call_args_for_tuple!(1; (A, 0));
impl_into_java_call_args_for_tuple!(2; (A, 0), (B, 1));
impl_into_java_call_args_for_tuple!(3; (A, 0), (B, 1), (C, 2));
impl_into_java_call_args_for_tuple!(4; (A, 0), (B, 1), (C, 2), (D, 3));
impl_into_java_call_args_for_tuple!(5; (A, 0), (B, 1), (C, 2), (D, 3), (E, 4));
impl_into_java_call_args_for_tuple!(6; (A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5));
impl_into_java_call_args_for_tuple!(7; (A, 0), (B, 1), (C, 2), (D, 3), (E, 4), (F, 5), (G, 6));
impl_into_java_call_args_for_tuple!(
    8;
    (A, 0),
    (B, 1),
    (C, 2),
    (D, 3),
    (E, 4),
    (F, 5),
    (G, 6),
    (H, 7)
);

impl From<&JavaObject> for JavaValue {
    fn from(value: &JavaObject) -> Self {
        Self::Object(value.as_jobject())
    }
}

impl From<Option<&JavaObject>> for JavaValue {
    fn from(value: Option<&JavaObject>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

impl From<&JavaLocalObject<'_>> for JavaValue {
    fn from(value: &JavaLocalObject<'_>) -> Self {
        Self::Object(value.as_jobject())
    }
}

impl From<Option<&JavaLocalObject<'_>>> for JavaValue {
    fn from(value: Option<&JavaLocalObject<'_>>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

impl From<&JavaArray> for JavaValue {
    fn from(value: &JavaArray) -> Self {
        Self::Object(value.as_jobject())
    }
}

impl From<Option<&JavaArray>> for JavaValue {
    fn from(value: Option<&JavaArray>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

impl From<&JavaLocalArray<'_>> for JavaValue {
    fn from(value: &JavaLocalArray<'_>) -> Self {
        Self::Object(value.as_jobject())
    }
}

impl From<Option<&JavaLocalArray<'_>>> for JavaValue {
    fn from(value: Option<&JavaLocalArray<'_>>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

fn prepare_call_args<I, A>(
    args: I,
    env: &Env<'_>,
    expected: &[JavaType],
) -> Result<PreparedJavaArgValues>
where
    I: IntoIterator<Item = A>,
    A: JavaCallArg,
{
    let args = args.into_iter().collect::<Vec<_>>();
    if args.len() != expected.len() {
        return Err(Error::InvalidArguments {
            expected: expected.len(),
            actual: args.len(),
        });
    }

    let mut values = PreparedJavaArgValues::with_capacity(args.len());
    for (index, (arg, expected)) in args.into_iter().zip(expected).enumerate() {
        values.push(arg.into_java_call_arg(env, expected, index)?);
    }
    Ok(values)
}

impl<T> JavaCallArg for T
where
    T: Into<JavaValue>,
{
    fn into_java_call_arg(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg> {
        let value = self.into();
        if value.matches_type(expected) {
            Ok(PreparedJavaArg {
                value,
                local_ref: None,
            })
        } else {
            Err(Error::InvalidArgumentType {
                index,
                expected: expected.to_string(),
                actual: value.type_name(),
            })
        }
    }
}

impl JavaCallArg for &JavaValue {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg> {
        (*self).into_java_call_arg(env, expected, index)
    }
}

impl JavaCallArg for &str {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg> {
        prepare_rust_string_arg(self, env, expected, index)
    }
}

impl JavaCallArg for &&str {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg> {
        prepare_rust_string_arg(self, env, expected, index)
    }
}

impl JavaCallArg for String {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg> {
        prepare_rust_string_arg(&self, env, expected, index)
    }
}

impl JavaCallArg for &String {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaArg> {
        prepare_rust_string_arg(self, env, expected, index)
    }
}

impl<T> IntoJavaFieldValue for T
where
    T: Into<JavaValue>,
{
    fn into_java_field_value(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        let value = self.into();
        if value.matches_type(expected) {
            Ok(PreparedJavaFieldValue::new(value, None))
        } else {
            Err(Error::InvalidFieldValueType {
                operation,
                expected: expected.to_string(),
                actual: value.type_name(),
            })
        }
    }
}

impl IntoJavaFieldValue for &str {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_rust_string_field_value(self, env, expected, operation)
    }
}

impl IntoJavaFieldValue for String {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_rust_string_field_value(&self, env, expected, operation)
    }
}

impl IntoJavaFieldValue for &String {
    fn into_java_field_value(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        operation: &'static str,
    ) -> Result<PreparedJavaFieldValue> {
        prepare_rust_string_field_value(self, env, expected, operation)
    }
}

fn prepare_rust_string_arg(
    value: &str,
    env: &Env<'_>,
    expected: &JavaType,
    index: usize,
) -> Result<PreparedJavaArg> {
    if !accepts_rust_string(expected) {
        return Err(Error::InvalidArgumentType {
            index,
            expected: expected.to_string(),
            actual: "string",
        });
    }

    let local_ref = env.new_string_utf_raw(value)?;
    Ok(PreparedJavaArg {
        value: JavaValue::Object(local_ref),
        local_ref: Some(local_ref),
    })
}

fn prepare_rust_string_field_value(
    value: &str,
    env: &Env<'_>,
    expected: &JavaType,
    operation: &'static str,
) -> Result<PreparedJavaFieldValue> {
    if !accepts_rust_string(expected) {
        return Err(Error::InvalidFieldValueType {
            operation,
            expected: expected.to_string(),
            actual: "string",
        });
    }

    let local_ref = env.new_string_utf_raw(value)?;
    Ok(PreparedJavaFieldValue::new(
        JavaValue::Object(local_ref),
        Some(local_ref),
    ))
}

fn accepts_rust_string(expected: &JavaType) -> bool {
    matches!(
        expected,
        JavaType::Object(class) if class == "java/lang/String" || class == "java/lang/Object"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_common_java_argument_containers() {
        assert_eq!(().into_java_args(), Vec::<JavaValue>::new());

        let values = [JavaValue::Int(7), JavaValue::Null];
        assert_eq!(
            values.into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );
        assert_eq!(
            (&values).into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );

        let slice: &[JavaValue] = &values;
        assert_eq!(
            slice.into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );

        assert_eq!(
            vec![JavaValue::Boolean(true)].into_java_args(),
            vec![JavaValue::Boolean(true)]
        );
    }

    #[test]
    fn converts_tuple_java_arguments() {
        assert_eq!(
            (7 as jni::jint, true, JavaValue::Null).into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Boolean(true), JavaValue::Null]
        );
    }

    #[test]
    fn converts_optional_java_object_arguments() {
        assert_eq!(JavaValue::from(None::<&JavaObject>), JavaValue::Null);
        assert_eq!(
            (None::<&JavaObject>,).into_java_args(),
            vec![JavaValue::Null]
        );
    }

    #[test]
    fn recognizes_rust_string_argument_targets() {
        assert!(accepts_rust_string(&JavaType::Object(
            "java/lang/String".to_owned()
        )));
        assert!(accepts_rust_string(&JavaType::Object(
            "java/lang/Object".to_owned()
        )));
        assert!(!accepts_rust_string(&JavaType::Object(
            "java/lang/CharSequence".to_owned()
        )));
        assert!(!accepts_rust_string(&JavaType::Int));
        assert!(!accepts_rust_string(&JavaType::Array(Box::new(
            JavaType::Object("java/lang/String".to_owned())
        ))));
    }
}
