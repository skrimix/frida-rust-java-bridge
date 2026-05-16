use super::*;

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
}
