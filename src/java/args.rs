use super::*;

pub(crate) trait JavaCallArg {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg>;

    fn into_java_overload_arg(self) -> JavaOverloadArg;
}

pub(crate) struct PreparedJavaCallArg {
    value: JavaValue,
    local_ref: Option<jni::jobject>,
}

impl PreparedJavaCallArgs {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
            local_refs: Vec::new(),
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

impl IntoJavaArgs for () {
    fn into_java_args(self) -> Vec<JavaValue> {
        Vec::new()
    }
}

impl IntoJavaArgs for JavaArgs {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.values
    }
}

impl IntoJavaArgs for &JavaArgs {
    fn into_java_args(self) -> Vec<JavaValue> {
        self.values.clone()
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

impl<A: Into<JavaValue>> IntoJavaArgs for A {
    fn into_java_args(self) -> Vec<JavaValue> {
        vec![self.into()]
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
    ) -> Result<PreparedJavaCallArgs> {
        let values = PreparedJavaCallArgs::with_capacity(0);
        values.validate_len(expected)?;
        Ok(values)
    }
}

impl IntoJavaOverloadArgs for () {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        Vec::new()
    }
}

impl<A: JavaCallArg> IntoJavaCallArgs for A {
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs> {
        if expected.len() != 1 {
            return Err(Error::InvalidArguments {
                expected: expected.len(),
                actual: 1,
            });
        }

        let mut values = PreparedJavaCallArgs::with_capacity(1);
        values.push(self.into_java_call_arg(env, &expected[0], 0)?);
        Ok(values)
    }
}

impl<A: JavaCallArg> IntoJavaOverloadArgs for A {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        vec![self.into_java_overload_arg()]
    }
}

impl<A: JavaCallArg> IntoJavaCallArgs for Vec<A> {
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs> {
        prepare_call_args(self, env, expected)
    }
}

impl<A: JavaCallArg> IntoJavaOverloadArgs for Vec<A> {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        self.into_iter()
            .map(JavaCallArg::into_java_overload_arg)
            .collect()
    }
}

impl<const N: usize, A: JavaCallArg> IntoJavaCallArgs for [A; N] {
    fn into_java_call_args(
        self,
        env: &Env<'_>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs> {
        prepare_call_args(self, env, expected)
    }
}

impl<const N: usize, A: JavaCallArg> IntoJavaOverloadArgs for [A; N] {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        self.into_iter()
            .map(JavaCallArg::into_java_overload_arg)
            .collect()
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
    ) -> Result<PreparedJavaCallArgs> {
        prepare_call_args(self.iter(), env, expected)
    }
}

impl<'a, A> IntoJavaOverloadArgs for &'a [A]
where
    &'a A: JavaCallArg,
{
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        self.iter()
            .map(JavaCallArg::into_java_overload_arg)
            .collect()
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
    ) -> Result<PreparedJavaCallArgs> {
        self.as_slice().into_java_call_args(env, expected)
    }
}

impl<'a, const N: usize, A> IntoJavaOverloadArgs for &'a [A; N]
where
    &'a A: JavaCallArg,
{
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        self.as_slice().into_java_overload_args()
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
            ) -> Result<PreparedJavaCallArgs> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                if expected.len() != $actual {
                    return Err(Error::InvalidArguments {
                        expected: expected.len(),
                        actual: $actual,
                    });
                }
                let mut values = PreparedJavaCallArgs::with_capacity($actual);
                $(values.push($name.into_java_call_arg(env, &expected[$index], $index)?);)+
                values.validate_len(expected)?;
                Ok(values)
            }
        }

        impl<$($name),+> IntoJavaOverloadArgs for ($($name,)+)
        where
            $($name: JavaCallArg),+
        {
            fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                vec![$($name.into_java_overload_arg()),+]
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

impl<R> From<&JavaObject<R>> for JavaValue
where
    R: JavaObjectRef,
{
    fn from(value: &JavaObject<R>) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

impl<R> From<Option<&JavaObject<R>>> for JavaValue
where
    R: JavaObjectRef,
{
    fn from(value: Option<&JavaObject<R>>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

impl<R> From<&JavaRef<R>> for JavaValue
where
    R: JavaObjectRef,
{
    fn from(value: &JavaRef<R>) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

impl<R> From<Option<&JavaRef<R>>> for JavaValue
where
    R: JavaObjectRef,
{
    fn from(value: Option<&JavaRef<R>>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

impl<R> From<&JavaArray<R>> for JavaValue
where
    R: JavaObjectRef,
{
    fn from(value: &JavaArray<R>) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

impl<R> From<Option<&JavaArray<R>>> for JavaValue
where
    R: JavaObjectRef,
{
    fn from(value: Option<&JavaArray<R>>) -> Self {
        value.map_or(Self::Null, Self::from)
    }
}

fn prepare_call_args<I, A>(
    args: I,
    env: &Env<'_>,
    expected: &[JavaType],
) -> Result<PreparedJavaCallArgs>
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

    let mut values = PreparedJavaCallArgs::with_capacity(args.len());
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
    ) -> Result<PreparedJavaCallArg> {
        let value = self.into();
        let value = coerce_java_call_value(value, expected, index)?;
        Ok(PreparedJavaCallArg {
            value,
            local_ref: None,
        })
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::Value(self.into())
    }
}

impl JavaCallArg for &JavaValue {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        (*self).into_java_call_arg(env, expected, index)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::Value(*self)
    }
}

impl JavaCallArg for JavaOverloadArg {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        match self {
            Self::Value(value) => value.into_java_call_arg(env, expected, index),
            Self::RustString(value) => prepare_rust_string_arg(&value, env, expected, index),
        }
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        self
    }
}

impl JavaCallArg for &str {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_rust_string_arg(self, env, expected, index)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::RustString(self.to_owned())
    }
}

impl JavaCallArg for &&str {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_rust_string_arg(self, env, expected, index)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::RustString((*self).to_owned())
    }
}

impl JavaCallArg for String {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_rust_string_arg(&self, env, expected, index)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::RustString(self)
    }
}

impl JavaCallArg for &String {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_rust_string_arg(self, env, expected, index)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::RustString(self.clone())
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
        let value = coerce_java_field_value(value, expected, operation)?;
        Ok(PreparedJavaFieldValue::new(value, None))
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

impl<T> sealed::IntoJavaFieldValueSealed for T where T: Into<JavaValue> {}
impl sealed::IntoJavaFieldValueSealed for &str {}
impl sealed::IntoJavaFieldValueSealed for String {}
impl sealed::IntoJavaFieldValueSealed for &String {}

fn prepare_rust_string_arg(
    value: &str,
    env: &Env<'_>,
    expected: &JavaType,
    index: usize,
) -> Result<PreparedJavaCallArg> {
    if !accepts_rust_string(expected) {
        return Err(Error::InvalidArgumentType {
            index,
            expected: expected.to_string(),
            actual: "string",
        });
    }

    // SAFETY: The local string reference is stored in `PreparedJavaCallArg` and deleted after the JNI
    // call consuming it completes.
    let local_ref = unsafe { env.new_string_utf_raw(value)? };
    Ok(PreparedJavaCallArg {
        value: JavaValue::object_ref(local_ref),
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

    // SAFETY: The local string reference is stored in `PreparedJavaFieldValue` and deleted after
    // the JNI field operation consuming it completes.
    let local_ref = unsafe { env.new_string_utf_raw(value)? };
    Ok(PreparedJavaFieldValue::new(
        JavaValue::object_ref(local_ref),
        Some(local_ref),
    ))
}

fn accepts_rust_string(expected: &JavaType) -> bool {
    matches!(
        expected,
        JavaType::Object(class) if class == "java/lang/String" || class == "java/lang/Object"
            || class == "java/lang/CharSequence"
    )
}

fn coerce_java_call_value(
    value: JavaValue,
    expected: &JavaType,
    index: usize,
) -> Result<JavaValue> {
    coerce_java_value(value, expected).map_err(|error| match error {
        JavaValueCoercionError::Type { actual } => Error::InvalidArgumentType {
            index,
            expected: expected.to_string(),
            actual,
        },
        JavaValueCoercionError::Value { actual } => Error::InvalidArgumentValue {
            index,
            expected: expected.to_string(),
            actual,
        },
    })
}

fn coerce_java_field_value(
    value: JavaValue,
    expected: &JavaType,
    operation: &'static str,
) -> Result<JavaValue> {
    coerce_java_value(value, expected).map_err(|error| match error {
        JavaValueCoercionError::Type { actual } => Error::InvalidFieldValueType {
            operation,
            expected: expected.to_string(),
            actual,
        },
        JavaValueCoercionError::Value { actual } => Error::InvalidFieldValue {
            operation,
            expected: expected.to_string(),
            actual,
        },
    })
}

enum JavaValueCoercionError {
    Type { actual: &'static str },
    Value { actual: String },
}

fn coerce_java_value(
    value: JavaValue,
    expected: &JavaType,
) -> std::result::Result<JavaValue, JavaValueCoercionError> {
    if value.matches_type(expected) {
        return Ok(value);
    }

    match (value, expected) {
        (JavaValue::Int(value), JavaType::Byte) => {
            narrow_int_value(value, i8::MIN as i32, i8::MAX as i32, "byte")
                .map(|value| JavaValue::Byte(value as jni::jbyte))
        }
        (JavaValue::Int(value), JavaType::Char) => {
            narrow_int_value(value, 0, u16::MAX as i32, "char")
                .map(|value| JavaValue::Char(value as jni::jchar))
        }
        (JavaValue::Int(value), JavaType::Short) => {
            narrow_int_value(value, i16::MIN as i32, i16::MAX as i32, "short")
                .map(|value| JavaValue::Short(value as jni::jshort))
        }
        (JavaValue::Int(value), JavaType::Long) => Ok(JavaValue::Long(value as jni::jlong)),
        (JavaValue::Float(value), JavaType::Double) => Ok(JavaValue::Double(value as jni::jdouble)),
        (JavaValue::Double(value), JavaType::Float) => {
            double_to_float_value(value).map(JavaValue::Float)
        }
        (value, _) => Err(JavaValueCoercionError::Type {
            actual: value.type_name(),
        }),
    }
}

pub(crate) fn can_coerce_java_value(value: JavaValue, expected: &JavaType) -> bool {
    coerce_java_value(value, expected).is_ok()
}

fn narrow_int_value(
    value: jni::jint,
    min: jni::jint,
    max: jni::jint,
    expected: &'static str,
) -> std::result::Result<jni::jint, JavaValueCoercionError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(JavaValueCoercionError::Value {
            actual: format!("int {value} outside {expected} range"),
        })
    }
}

fn double_to_float_value(
    value: jni::jdouble,
) -> std::result::Result<jni::jfloat, JavaValueCoercionError> {
    if value.is_finite() && value.abs() <= f32::MAX as f64 {
        Ok(value as jni::jfloat)
    } else {
        Err(JavaValueCoercionError::Value {
            actual: format!("double {value} is not finite or outside float range"),
        })
    }
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
    fn converts_explicit_java_args_container() {
        let mut args = JavaArgs::with_capacity(2);
        args.push(7 as jni::jint);
        args.push(JavaValue::Null);

        assert_eq!(args.len(), 2);
        assert!(!args.is_empty());
        assert_eq!(args.as_slice(), &[JavaValue::Int(7), JavaValue::Null]);
        assert_eq!(
            (&args).into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );
        assert_eq!(
            args.into_java_args(),
            vec![JavaValue::Int(7), JavaValue::Null]
        );
    }

    #[test]
    fn java_args_macro_builds_long_mixed_lists() {
        let args = crate::java_args![
            1 as jni::jint,
            2 as jni::jint,
            3 as jni::jint,
            4 as jni::jint,
            5 as jni::jint,
            6 as jni::jint,
            7 as jni::jint,
            8 as jni::jint,
            9 as jni::jint,
            true,
            JavaValue::Null,
        ];

        assert_eq!(args.len(), 11);
        assert_eq!(
            args.into_java_args(),
            vec![
                JavaValue::Int(1),
                JavaValue::Int(2),
                JavaValue::Int(3),
                JavaValue::Int(4),
                JavaValue::Int(5),
                JavaValue::Int(6),
                JavaValue::Int(7),
                JavaValue::Int(8),
                JavaValue::Int(9),
                JavaValue::Boolean(true),
                JavaValue::Null,
            ]
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
    fn converts_bare_single_java_argument() {
        assert_eq!((7 as jni::jint).into_java_args(), vec![JavaValue::Int(7)]);
        assert_eq!(JavaValue::Null.into_java_args(), vec![JavaValue::Null]);
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
        assert!(accepts_rust_string(&JavaType::Object(
            "java/lang/CharSequence".to_owned()
        )));
        assert!(!accepts_rust_string(&JavaType::Object(
            "java/lang/StringBuilder".to_owned()
        )));
        assert!(!accepts_rust_string(&JavaType::Int));
        assert!(!accepts_rust_string(&JavaType::Array(Box::new(
            JavaType::Object("java/lang/String".to_owned())
        ))));
    }

    #[test]
    fn coerces_descriptor_selected_numeric_arguments_conservatively() {
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(7), &JavaType::Byte, 0).unwrap(),
            JavaValue::Byte(7)
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(65), &JavaType::Char, 0).unwrap(),
            JavaValue::Char(65)
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(-300), &JavaType::Short, 0).unwrap(),
            JavaValue::Short(-300)
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(7), &JavaType::Long, 0).unwrap(),
            JavaValue::Long(7)
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Float(1.5), &JavaType::Double, 0).unwrap(),
            JavaValue::Double(1.5)
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Double(2.5), &JavaType::Float, 0).unwrap(),
            JavaValue::Float(2.5)
        );
    }

    #[test]
    fn rejects_out_of_range_numeric_argument_coercions() {
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(128), &JavaType::Byte, 1).unwrap_err(),
            Error::InvalidArgumentValue {
                index: 1,
                expected: "B".to_owned(),
                actual: "int 128 outside byte range".to_owned(),
            }
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(-1), &JavaType::Char, 2).unwrap_err(),
            Error::InvalidArgumentValue {
                index: 2,
                expected: "C".to_owned(),
                actual: "int -1 outside char range".to_owned(),
            }
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Double(f64::MAX), &JavaType::Float, 3).unwrap_err(),
            Error::InvalidArgumentValue {
                index: 3,
                expected: "F".to_owned(),
                actual: format!("double {} is not finite or outside float range", f64::MAX),
            }
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Double(f64::INFINITY), &JavaType::Float, 4)
                .unwrap_err(),
            Error::InvalidArgumentValue {
                index: 4,
                expected: "F".to_owned(),
                actual: "double inf is not finite or outside float range".to_owned(),
            }
        );
    }

    #[test]
    fn preserves_exact_type_rejection_for_unsupported_coercions() {
        assert_eq!(
            coerce_java_call_value(JavaValue::Long(7), &JavaType::Int, 0).unwrap_err(),
            Error::InvalidArgumentType {
                index: 0,
                expected: "I".to_owned(),
                actual: "long",
            }
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Boolean(true), &JavaType::Int, 0).unwrap_err(),
            Error::InvalidArgumentType {
                index: 0,
                expected: "I".to_owned(),
                actual: "boolean",
            }
        );
        assert_eq!(
            coerce_java_call_value(JavaValue::Int(7), &JavaType::Float, 0).unwrap_err(),
            Error::InvalidArgumentType {
                index: 0,
                expected: "F".to_owned(),
                actual: "int",
            }
        );
    }

    #[test]
    fn reports_out_of_range_numeric_field_values() {
        assert_eq!(
            coerce_java_field_value(JavaValue::Int(32768), &JavaType::Short, "field").unwrap_err(),
            Error::InvalidFieldValue {
                operation: "field",
                expected: "S".to_owned(),
                actual: "int 32768 outside short range".to_owned(),
            }
        );
    }
}
