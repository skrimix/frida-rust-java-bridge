use super::*;

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

impl IntoJavaCallArgs for JavaArgs {
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
        self.values.into_java_call_args(env, expected)
    }
}

impl IntoJavaOverloadArgs for JavaArgs {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        self.values.into_java_overload_args()
    }
}

impl IntoJavaCallArgs for &JavaArgs {
    fn into_java_call_args<'env, 'vm>(
        self,
        env: &'env Env<'vm>,
        expected: &[JavaType],
    ) -> Result<PreparedJavaCallArgs<'env, 'vm>> {
        self.values.as_slice().into_java_call_args(env, expected)
    }
}

impl IntoJavaOverloadArgs for &JavaArgs {
    fn into_java_overload_args(self) -> Vec<JavaOverloadArg> {
        self.values.as_slice().into_java_overload_args()
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
