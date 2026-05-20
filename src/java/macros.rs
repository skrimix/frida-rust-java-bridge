macro_rules! java_return_extractors {
    ($(
        $name:ident, $variant:ident, $return:ty, $expected:literal;
    )+) => {
        $(
            pub fn $name(self, operation: &'static str) -> Result<$return> {
                match self {
                    Self::$variant(value) => Ok(value),
                    other => Err(invalid_return(operation, $expected, other)),
                }
            }
        )+
    };
}

macro_rules! java_new_primitive_arrays {
    ($(
        $name:ident, $element:ty, $env_new:ident, $java_type:expr;
    )+) => {
        $(
            pub fn $name(&self, elements: &[$element]) -> Result<JavaArray> {
                let env = self.vm.attach_current_thread()?;
                let array = env.$env_new(elements)?;
                array_from_ref(&env, &self.vm, &array, $java_type)
            }
        )+
    };
}

macro_rules! java_primitive_array_accessors {
    ($operation_type:literal; $(
        $get_name:ident, $set_name:ident, $element:ty, $java_type:expr,
        $env_get:ident, $env_set:ident;
    )+) => {
        $(
            pub fn $get_name(&self) -> Result<Vec<$element>> {
                ensure_element_type(
                    &self.element_type,
                    &$java_type,
                    concat!($operation_type, "::", stringify!($get_name)),
                )?;
                let env = self.vm.attach_current_thread()?;
                let mut values = vec![Default::default(); self.len()? as usize];
                env.$env_get(self, 0, &mut values)?;
                Ok(values)
            }

            pub fn $set_name(&self, values: &[$element]) -> Result<()> {
                ensure_element_type(
                    &self.element_type,
                    &$java_type,
                    concat!($operation_type, "::", stringify!($set_name)),
                )?;
                let env = self.vm.attach_current_thread()?;
                env.$env_set(self, 0, values)
            }
        )+
    };
}
