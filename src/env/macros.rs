// Primitive Env macro entries are the local audit table for JNI slot correctness. Each generated
// method must pass the exact JNI vtable slot and function type to a shared helper; method and field
// helpers also pass the expected Java type there, and the shared helper owns pending-exception
// checks after the raw JNI call.
macro_rules! primitive_instance_method_calls {
    ($(
        $name:ident, $return:ty, $java_type:expr, $operation:literal, $slot:expr, $function:ty, $convert:expr;
    )+) => {
        $(
            pub fn $name(
                &self,
                object: &(impl AsJObject + ?Sized),
                method: &MethodId,
                args: &[JavaValue],
            ) -> Result<$return> {
                self.call_instance_primitive(
                    InstancePrimitiveCall {
                        object: object.as_jobject(),
                        method,
                        args,
                        expected_return: $java_type,
                        operation: $operation,
                        slot: $slot,
                    },
                    |call: $function, env, object, method, args| unsafe {
                        ($convert)(call(env, object, method, args))
                    },
                )
            }
        )+
    };
}

macro_rules! primitive_static_method_calls {
    ($(
        $name:ident, $return:ty, $java_type:expr, $operation:literal, $slot:expr, $function:ty, $convert:expr;
    )+) => {
        $(
            pub fn $name(
                &self,
                class: &impl AsJClass,
                method: &MethodId,
                args: &[JavaValue],
            ) -> Result<$return> {
                self.call_static_primitive(
                    StaticPrimitiveCall {
                        class,
                        method,
                        args,
                        expected_return: $java_type,
                        operation: $operation,
                        slot: $slot,
                    },
                    |call: $function, env, class, method, args| unsafe {
                        ($convert)(call(env, class, method, args))
                    },
                )
            }
        )+
    };
}

macro_rules! primitive_instance_fields {
    ($(
        $get_name:ident, $set_name:ident, $return:ty, $raw:ty, $java_type:expr,
        $get_operation:literal, $get_slot:expr, $get_function:ty, $get_convert:expr,
        $set_operation:literal, $set_slot:expr, $set_function:ty, $set_convert:expr;
    )+) => {
        $(
            pub fn $get_name(
                &self,
                object: &(impl AsJObject + ?Sized),
                field: &FieldId,
            ) -> Result<$return> {
                self.get_instance_primitive_field(
                    InstancePrimitiveField {
                        object: object.as_jobject(),
                        field,
                        expected_type: $java_type,
                        operation: $get_operation,
                        slot: $get_slot,
                    },
                    |get: $get_function, env, object, field| unsafe {
                        ($get_convert)(get(env, object, field))
                    },
                )
            }

            pub fn $set_name(
                &self,
                object: &(impl AsJObject + ?Sized),
                field: &FieldId,
                value: $return,
            ) -> Result<()> {
                let value: $raw = ($set_convert)(value);
                self.set_instance_primitive_field(
                    InstancePrimitiveField {
                        object: object.as_jobject(),
                        field,
                        expected_type: $java_type,
                        operation: $set_operation,
                        slot: $set_slot,
                    },
                    |set: $set_function, env, object, field| unsafe {
                        set(env, object, field, value)
                    },
                )
            }
        )+
    };
}

macro_rules! primitive_static_fields {
    ($(
        $get_name:ident, $set_name:ident, $return:ty, $raw:ty, $java_type:expr,
        $get_operation:literal, $get_slot:expr, $get_function:ty, $get_convert:expr,
        $set_operation:literal, $set_slot:expr, $set_function:ty, $set_convert:expr;
    )+) => {
        $(
            pub fn $get_name(
                &self,
                class: &impl AsJClass,
                field: &FieldId,
            ) -> Result<$return> {
                self.get_static_primitive_field(
                    StaticPrimitiveField {
                        class,
                        field,
                        expected_type: $java_type,
                        operation: $get_operation,
                        slot: $get_slot,
                    },
                    |get: $get_function, env, class, field| unsafe {
                        ($get_convert)(get(env, class, field))
                    },
                )
            }

            pub fn $set_name(
                &self,
                class: &impl AsJClass,
                field: &FieldId,
                value: $return,
            ) -> Result<()> {
                let value: $raw = ($set_convert)(value);
                self.set_static_primitive_field(
                    StaticPrimitiveField {
                        class,
                        field,
                        expected_type: $java_type,
                        operation: $set_operation,
                        slot: $set_slot,
                    },
                    |set: $set_function, env, class, field| unsafe {
                        set(env, class, field, value)
                    },
                )
            }
        )+
    };
}

macro_rules! primitive_arrays {
    ($(
        $new_name:ident, $get_name:ident, $set_name:ident, $element:ty,
        $new_operation:literal, $new_slot:expr,
        $get_operation:literal, $get_slot:expr,
        $set_operation:literal, $set_slot:expr;
    )+) => {
        $(
            pub fn $new_name(&self, elements: &[$element]) -> Result<ArrayRef<'_>> {
                let array = self.new_primitive_array(elements.len(), $new_slot, $new_operation)?;
                self.$set_name(&array, 0, elements)?;
                Ok(array)
            }

            pub fn $get_name(
                &self,
                array: &(impl AsJObject + ?Sized),
                start: jni::jsize,
                output: &mut [$element],
            ) -> Result<()> {
                self.get_primitive_array_region(array, start, output, $get_slot, $get_operation)
            }

            pub fn $set_name(
                &self,
                array: &(impl AsJObject + ?Sized),
                start: jni::jsize,
                input: &[$element],
            ) -> Result<()> {
                self.set_primitive_array_region(array, start, input, $set_slot, $set_operation)
            }
        )+
    };
}
