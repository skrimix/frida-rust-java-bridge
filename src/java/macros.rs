macro_rules! java_new_primitive_arrays {
    ($(
        $name:ident, $element:ty, $env_new:ident, $java_type:expr;
    )+) => {
        $(
            /// Creates a primitive Java array initialized from Rust values.
            ///
            /// ```no_run
            /// use frida_rust_java_bridge::{Java, Result};
            ///
            /// fn pass_bytes(java: &Java) -> Result<()> {
            ///     let bytes = java.new_byte_array(&[1, 2, 3])?;
            ///     let receiver = java.use_class("com.example.app.Receiver")?;
            ///     receiver.call::<()>("acceptBytes", &bytes)?;
            ///     Ok(())
            /// }
            /// ```
            pub fn $name(&self, elements: &[$element]) -> Result<JavaArray> {
                let env = self.vm().attach_current_thread()?;
                let array = env.$env_new(elements)?;
                let array_type = JavaType::Array(Box::new($java_type));
                let array_class = env.get_object_class(&array)?;
                let array_class = env.new_global_ref(&array_class)?;
                array_from_ref_with_class(
                    &env,
                    JavaClass::from_raw(raw::Class::from_global(
                        array_type.descriptor(),
                        array_class,
                    )),
                    &array,
                    $java_type,
                )
            }
        )+
    };
}

macro_rules! java_primitive_array_accessors {
    ($storage:ty; $(
        $get_name:ident, $set_name:ident, $element:ty, $java_type:expr,
        $env_get:ident, $env_set:ident;
    )+) => {
        $(
            /// Copies all elements out of this primitive Java array.
            pub fn $get_name(&self) -> Result<Vec<$element>> {
                ensure_element_type(
                    &self.element_type,
                    &$java_type,
                    operation_name::<$storage>(stringify!($get_name)),
                )?;
                let env = self.vm().attach_current_thread()?;
                let mut values = vec![Default::default(); self.len()? as usize];
                env.$env_get(self, 0, &mut values)?;
                Ok(values)
            }

            /// Copies `values` into this primitive Java array starting at index 0.
            ///
            /// The JNI call fails if `values` is longer than the Java array.
            pub fn $set_name(&self, values: &[$element]) -> Result<()> {
                ensure_element_type(
                    &self.element_type,
                    &$java_type,
                    operation_name::<$storage>(stringify!($set_name)),
                )?;
                let env = self.vm().attach_current_thread()?;
                env.$env_set(self, 0, values)
            }
        )+
    };

    ($operation_type:literal; $(
        $get_name:ident, $set_name:ident, $element:ty, $java_type:expr,
        $env_get:ident, $env_set:ident;
    )+) => {
        $(
            /// Copies all elements out of this primitive Java array.
            pub fn $get_name(&self) -> Result<Vec<$element>> {
                ensure_element_type(
                    &self.element_type,
                    &$java_type,
                    concat!($operation_type, "::", stringify!($get_name)),
                )?;
                let env = self.vm().attach_current_thread()?;
                let mut values = vec![Default::default(); self.len()? as usize];
                env.$env_get(self, 0, &mut values)?;
                Ok(values)
            }

            /// Copies `values` into this primitive Java array starting at index 0.
            ///
            /// The JNI call fails if `values` is longer than the Java array.
            pub fn $set_name(&self, values: &[$element]) -> Result<()> {
                ensure_element_type(
                    &self.element_type,
                    &$java_type,
                    concat!($operation_type, "::", stringify!($set_name)),
                )?;
                let env = self.vm().attach_current_thread()?;
                env.$env_set(self, 0, values)
            }
        )+
    };
}
