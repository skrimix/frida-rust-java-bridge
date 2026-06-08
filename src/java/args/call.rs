use crate::java::conversion::{
    prepare_call_reference, prepare_call_rust_string, prepare_call_value,
};
use crate::{
    env::Env,
    error::{Error, Result},
    refs::AsJObject,
    signature::JavaType,
    value::JavaValue,
};

use super::{
    super::{
        IntoJavaCallArgs, IntoJavaOverloadArgs, JavaArray, JavaLocalArray, JavaLocalObject,
        JavaObject,
    },
    JavaOverloadArg, PreparedJavaCallArg, PreparedJavaCallArgs,
};

pub(crate) trait JavaCallArg {
    fn into_java_call_arg(
        self,
        env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg>;

    fn into_java_overload_arg(self) -> JavaOverloadArg;
}

impl IntoJavaCallArgs for () {
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
        let values = PreparedJavaCallArgs::with_capacity(0, env);
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
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
        if expected.len() != 1 {
            return Err(Error::InvalidArguments {
                expected: expected.len(),
                actual: 1,
            });
        }

        let mut values = PreparedJavaCallArgs::with_capacity(1, env);
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
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
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
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
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
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
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
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
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
            fn into_java_call_args<'env, 'vm>(
                self,
                env: &'env Env<'vm>,
                expected: &[JavaType],
            ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
                #[allow(non_snake_case)]
                let ($($name,)+) = self;
                if expected.len() != $actual {
                    return Err(Error::InvalidArguments {
                        expected: expected.len(),
                        actual: $actual,
                    });
                }
                let mut values = PreparedJavaCallArgs::with_capacity($actual, env);
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

impl From<&JavaObject> for JavaValue {
    fn from(value: &JavaObject) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

impl From<Option<&JavaObject>> for JavaValue {
    fn from(value: Option<&JavaObject>) -> Self {
        value.map_or(Self::NULL, Self::from)
    }
}

impl From<&JavaArray> for JavaValue {
    fn from(value: &JavaArray) -> Self {
        Self::object_ref(value.as_jobject())
    }
}

impl From<Option<&JavaArray>> for JavaValue {
    fn from(value: Option<&JavaArray>) -> Self {
        value.map_or(Self::NULL, Self::from)
    }
}

impl<'local> JavaCallArg for &JavaLocalObject<'local> {
    fn into_java_call_arg(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_call_reference(self.as_jobject(), expected, index).map(PreparedJavaCallArg::from)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::Value(JavaValue::object_ref(self.as_jobject()))
    }
}

impl<'local> JavaCallArg for Option<&JavaLocalObject<'local>> {
    fn into_java_call_arg(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_call_reference(
            self.map_or(std::ptr::null_mut(), |value| value.as_jobject()),
            expected,
            index,
        )
        .map(PreparedJavaCallArg::from)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::Value(self.map_or(JavaValue::NULL, |value| {
            JavaValue::object_ref(value.as_jobject())
        }))
    }
}

impl<'local> JavaCallArg for &JavaLocalArray<'local> {
    fn into_java_call_arg(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_call_reference(self.as_jobject(), expected, index).map(PreparedJavaCallArg::from)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::Value(JavaValue::object_ref(self.as_jobject()))
    }
}

impl<'local> JavaCallArg for Option<&JavaLocalArray<'local>> {
    fn into_java_call_arg(
        self,
        _env: &Env<'_>,
        expected: &JavaType,
        index: usize,
    ) -> Result<PreparedJavaCallArg> {
        prepare_call_reference(
            self.map_or(std::ptr::null_mut(), |value| value.as_jobject()),
            expected,
            index,
        )
        .map(PreparedJavaCallArg::from)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::Value(self.map_or(JavaValue::NULL, |value| {
            JavaValue::object_ref(value.as_jobject())
        }))
    }
}

fn prepare_call_args<'env, 'vm, I, A>(
    args: I,
    env: &'env Env<'vm>,
    expected: &[JavaType],
) -> Result<PreparedJavaCallArgs<'env, 'vm>>
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

    let mut values = PreparedJavaCallArgs::with_capacity(args.len(), env);
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
        prepare_call_value(self.into(), expected, index).map(PreparedJavaCallArg::from)
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
            Self::RustString(value) => prepare_call_rust_string(&value, env, expected, index)
                .map(PreparedJavaCallArg::from),
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
        prepare_call_rust_string(self, env, expected, index).map(PreparedJavaCallArg::from)
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
        prepare_call_rust_string(self, env, expected, index).map(PreparedJavaCallArg::from)
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
        prepare_call_rust_string(&self, env, expected, index).map(PreparedJavaCallArg::from)
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
        prepare_call_rust_string(self, env, expected, index).map(PreparedJavaCallArg::from)
    }

    fn into_java_overload_arg(self) -> JavaOverloadArg {
        JavaOverloadArg::RustString(self.clone())
    }
}
