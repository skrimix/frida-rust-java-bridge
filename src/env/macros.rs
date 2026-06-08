// Primitive Env macro entries are the local audit table for JNI slot correctness. Each generated
// method must pass the exact JNI vtable slot and function type to a shared helper; method and field
// helpers also pass the expected Java type there, and the shared helper owns pending-exception
// checks after the raw JNI call.
macro_rules! primitive_jni_table {
    ($consumer:ident) => {
        $consumer! {
            bool, jni::jboolean, JavaType::Boolean,
            |value| value == jni::JNI_TRUE,
            |value| if value { jni::JNI_TRUE } else { jni::JNI_FALSE },
            call_instance_boolean_method, "JNIEnv::CallBooleanMethodA",
            jni::ENV_CALL_BOOLEAN_METHOD_A, jni::CallBooleanMethodA,
            call_static_boolean_method, "JNIEnv::CallStaticBooleanMethodA",
            jni::ENV_CALL_STATIC_BOOLEAN_METHOD_A, jni::CallStaticBooleanMethodA,
            get_instance_boolean_field, set_instance_boolean_field,
            "JNIEnv::GetBooleanField", jni::ENV_GET_BOOLEAN_FIELD, jni::GetBooleanField,
            "JNIEnv::SetBooleanField", jni::ENV_SET_BOOLEAN_FIELD, jni::SetBooleanField,
            get_static_boolean_field, set_static_boolean_field,
            "JNIEnv::GetStaticBooleanField", jni::ENV_GET_STATIC_BOOLEAN_FIELD,
            jni::GetStaticBooleanField,
            "JNIEnv::SetStaticBooleanField", jni::ENV_SET_STATIC_BOOLEAN_FIELD,
            jni::SetStaticBooleanField,
            Boolean;

            jni::jbyte, jni::jbyte, JavaType::Byte,
            |value| value,
            |value| value,
            call_instance_byte_method, "JNIEnv::CallByteMethodA",
            jni::ENV_CALL_BYTE_METHOD_A, jni::CallByteMethodA,
            call_static_byte_method, "JNIEnv::CallStaticByteMethodA",
            jni::ENV_CALL_STATIC_BYTE_METHOD_A, jni::CallStaticByteMethodA,
            get_instance_byte_field, set_instance_byte_field,
            "JNIEnv::GetByteField", jni::ENV_GET_BYTE_FIELD, jni::GetByteField,
            "JNIEnv::SetByteField", jni::ENV_SET_BYTE_FIELD, jni::SetByteField,
            get_static_byte_field, set_static_byte_field,
            "JNIEnv::GetStaticByteField", jni::ENV_GET_STATIC_BYTE_FIELD,
            jni::GetStaticByteField,
            "JNIEnv::SetStaticByteField", jni::ENV_SET_STATIC_BYTE_FIELD,
            jni::SetStaticByteField,
            Byte;

            jni::jchar, jni::jchar, JavaType::Char,
            |value| value,
            |value| value,
            call_instance_char_method, "JNIEnv::CallCharMethodA",
            jni::ENV_CALL_CHAR_METHOD_A, jni::CallCharMethodA,
            call_static_char_method, "JNIEnv::CallStaticCharMethodA",
            jni::ENV_CALL_STATIC_CHAR_METHOD_A, jni::CallStaticCharMethodA,
            get_instance_char_field, set_instance_char_field,
            "JNIEnv::GetCharField", jni::ENV_GET_CHAR_FIELD, jni::GetCharField,
            "JNIEnv::SetCharField", jni::ENV_SET_CHAR_FIELD, jni::SetCharField,
            get_static_char_field, set_static_char_field,
            "JNIEnv::GetStaticCharField", jni::ENV_GET_STATIC_CHAR_FIELD,
            jni::GetStaticCharField,
            "JNIEnv::SetStaticCharField", jni::ENV_SET_STATIC_CHAR_FIELD,
            jni::SetStaticCharField,
            Char;

            jni::jshort, jni::jshort, JavaType::Short,
            |value| value,
            |value| value,
            call_instance_short_method, "JNIEnv::CallShortMethodA",
            jni::ENV_CALL_SHORT_METHOD_A, jni::CallShortMethodA,
            call_static_short_method, "JNIEnv::CallStaticShortMethodA",
            jni::ENV_CALL_STATIC_SHORT_METHOD_A, jni::CallStaticShortMethodA,
            get_instance_short_field, set_instance_short_field,
            "JNIEnv::GetShortField", jni::ENV_GET_SHORT_FIELD, jni::GetShortField,
            "JNIEnv::SetShortField", jni::ENV_SET_SHORT_FIELD, jni::SetShortField,
            get_static_short_field, set_static_short_field,
            "JNIEnv::GetStaticShortField", jni::ENV_GET_STATIC_SHORT_FIELD,
            jni::GetStaticShortField,
            "JNIEnv::SetStaticShortField", jni::ENV_SET_STATIC_SHORT_FIELD,
            jni::SetStaticShortField,
            Short;

            jni::jint, jni::jint, JavaType::Int,
            |value| value,
            |value| value,
            call_instance_int_method, "JNIEnv::CallIntMethodA",
            jni::ENV_CALL_INT_METHOD_A, jni::CallIntMethodA,
            call_static_int_method, "JNIEnv::CallStaticIntMethodA",
            jni::ENV_CALL_STATIC_INT_METHOD_A, jni::CallStaticIntMethodA,
            get_instance_int_field, set_instance_int_field,
            "JNIEnv::GetIntField", jni::ENV_GET_INT_FIELD, jni::GetIntField,
            "JNIEnv::SetIntField", jni::ENV_SET_INT_FIELD, jni::SetIntField,
            get_static_int_field, set_static_int_field,
            "JNIEnv::GetStaticIntField", jni::ENV_GET_STATIC_INT_FIELD,
            jni::GetStaticIntField,
            "JNIEnv::SetStaticIntField", jni::ENV_SET_STATIC_INT_FIELD,
            jni::SetStaticIntField,
            Int;

            jni::jlong, jni::jlong, JavaType::Long,
            |value| value,
            |value| value,
            call_instance_long_method, "JNIEnv::CallLongMethodA",
            jni::ENV_CALL_LONG_METHOD_A, jni::CallLongMethodA,
            call_static_long_method, "JNIEnv::CallStaticLongMethodA",
            jni::ENV_CALL_STATIC_LONG_METHOD_A, jni::CallStaticLongMethodA,
            get_instance_long_field, set_instance_long_field,
            "JNIEnv::GetLongField", jni::ENV_GET_LONG_FIELD, jni::GetLongField,
            "JNIEnv::SetLongField", jni::ENV_SET_LONG_FIELD, jni::SetLongField,
            get_static_long_field, set_static_long_field,
            "JNIEnv::GetStaticLongField", jni::ENV_GET_STATIC_LONG_FIELD,
            jni::GetStaticLongField,
            "JNIEnv::SetStaticLongField", jni::ENV_SET_STATIC_LONG_FIELD,
            jni::SetStaticLongField,
            Long;

            jni::jfloat, jni::jfloat, JavaType::Float,
            |value| value,
            |value| value,
            call_instance_float_method, "JNIEnv::CallFloatMethodA",
            jni::ENV_CALL_FLOAT_METHOD_A, jni::CallFloatMethodA,
            call_static_float_method, "JNIEnv::CallStaticFloatMethodA",
            jni::ENV_CALL_STATIC_FLOAT_METHOD_A, jni::CallStaticFloatMethodA,
            get_instance_float_field, set_instance_float_field,
            "JNIEnv::GetFloatField", jni::ENV_GET_FLOAT_FIELD, jni::GetFloatField,
            "JNIEnv::SetFloatField", jni::ENV_SET_FLOAT_FIELD, jni::SetFloatField,
            get_static_float_field, set_static_float_field,
            "JNIEnv::GetStaticFloatField", jni::ENV_GET_STATIC_FLOAT_FIELD,
            jni::GetStaticFloatField,
            "JNIEnv::SetStaticFloatField", jni::ENV_SET_STATIC_FLOAT_FIELD,
            jni::SetStaticFloatField,
            Float;

            jni::jdouble, jni::jdouble, JavaType::Double,
            |value| value,
            |value| value,
            call_instance_double_method, "JNIEnv::CallDoubleMethodA",
            jni::ENV_CALL_DOUBLE_METHOD_A, jni::CallDoubleMethodA,
            call_static_double_method, "JNIEnv::CallStaticDoubleMethodA",
            jni::ENV_CALL_STATIC_DOUBLE_METHOD_A, jni::CallStaticDoubleMethodA,
            get_instance_double_field, set_instance_double_field,
            "JNIEnv::GetDoubleField", jni::ENV_GET_DOUBLE_FIELD, jni::GetDoubleField,
            "JNIEnv::SetDoubleField", jni::ENV_SET_DOUBLE_FIELD, jni::SetDoubleField,
            get_static_double_field, set_static_double_field,
            "JNIEnv::GetStaticDoubleField", jni::ENV_GET_STATIC_DOUBLE_FIELD,
            jni::GetStaticDoubleField,
            "JNIEnv::SetStaticDoubleField", jni::ENV_SET_STATIC_DOUBLE_FIELD,
            jni::SetStaticDoubleField,
            Double;
        }
    };
}

pub(crate) use primitive_jni_table;

macro_rules! primitive_instance_method_calls_from_entries {
    ($(
        $return:ty, $raw:ty, $java_type:path, $from_raw:expr, $to_raw:expr,
        $name:ident, $operation:literal, $slot:expr, $function:ty,
        $static_name:ident, $static_operation:literal, $static_slot:expr, $static_function:ty,
        $instance_get_name:ident, $instance_set_name:ident,
        $instance_get_operation:literal, $instance_get_slot:expr, $instance_get_function:ty,
        $instance_set_operation:literal, $instance_set_slot:expr, $instance_set_function:ty,
        $static_get_name:ident, $static_set_name:ident,
        $static_get_operation:literal, $static_get_slot:expr, $static_get_function:ty,
        $static_set_operation:literal, $static_set_slot:expr, $static_set_function:ty,
        $raw_return:ident;
    )+) => {
        $(
            /// Calls an instance primitive method with a detached method ID.
            ///
            /// # Safety
            ///
            /// `method` must have been resolved from `object`'s class or one of its supertypes in
            /// this process ART runtime, and every object reference in `args` must be valid for this attached
            /// thread until the JNI call completes.
            pub unsafe fn $name(
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
                        ($from_raw)(call(env, object, method, args))
                    },
                )
            }
        )+
    };
}

macro_rules! primitive_instance_method_calls {
    () => {
        primitive_jni_table!(primitive_instance_method_calls_from_entries);
    };
}

macro_rules! primitive_static_method_calls_from_entries {
    ($(
        $return:ty, $raw:ty, $java_type:path, $from_raw:expr, $to_raw:expr,
        $instance_name:ident, $instance_operation:literal, $instance_slot:expr, $instance_function:ty,
        $name:ident, $operation:literal, $slot:expr, $function:ty,
        $instance_get_name:ident, $instance_set_name:ident,
        $instance_get_operation:literal, $instance_get_slot:expr, $instance_get_function:ty,
        $instance_set_operation:literal, $instance_set_slot:expr, $instance_set_function:ty,
        $static_get_name:ident, $static_set_name:ident,
        $static_get_operation:literal, $static_get_slot:expr, $static_get_function:ty,
        $static_set_operation:literal, $static_set_slot:expr, $static_set_function:ty,
        $raw_return:ident;
    )+) => {
        $(
            /// Calls a static primitive method with a detached method ID.
            ///
            /// # Safety
            ///
            /// `method` must have been resolved from `class` in this process ART runtime, and every object
            /// reference in `args` must be valid for this attached thread until the JNI call
            /// completes.
            pub unsafe fn $name(
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
                        ($from_raw)(call(env, class, method, args))
                    },
                )
            }
        )+
    };
}

macro_rules! primitive_static_method_calls {
    () => {
        primitive_jni_table!(primitive_static_method_calls_from_entries);
    };
}

macro_rules! primitive_instance_fields_from_entries {
    ($(
        $return:ty, $raw:ty, $java_type:path, $from_raw:expr, $to_raw:expr,
        $instance_call_name:ident, $instance_call_operation:literal,
        $instance_call_slot:expr, $instance_call_function:ty,
        $static_call_name:ident, $static_call_operation:literal,
        $static_call_slot:expr, $static_call_function:ty,
        $get_name:ident, $set_name:ident,
        $get_operation:literal, $get_slot:expr, $get_function:ty,
        $set_operation:literal, $set_slot:expr, $set_function:ty,
        $static_get_name:ident, $static_set_name:ident,
        $static_get_operation:literal, $static_get_slot:expr, $static_get_function:ty,
        $static_set_operation:literal, $static_set_slot:expr, $static_set_function:ty,
        $raw_return:ident;
    )+) => {
        $(
            /// Gets an instance primitive field with a detached field ID.
            ///
            /// # Safety
            ///
            /// `field` must have been resolved from `object`'s class or one of its supertypes in
            /// this process ART runtime.
            pub unsafe fn $get_name(
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
                        ($from_raw)(get(env, object, field))
                    },
                )
            }

            /// Sets an instance primitive field with a detached field ID.
            ///
            /// # Safety
            ///
            /// `field` must have been resolved from `object`'s class or one of its supertypes in
            /// this process ART runtime.
            pub unsafe fn $set_name(
                &self,
                object: &(impl AsJObject + ?Sized),
                field: &FieldId,
                value: $return,
            ) -> Result<()> {
                let value: $raw = ($to_raw)(value);
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

macro_rules! primitive_instance_fields {
    () => {
        primitive_jni_table!(primitive_instance_fields_from_entries);
    };
}

macro_rules! primitive_static_fields_from_entries {
    ($(
        $return:ty, $raw:ty, $java_type:path, $from_raw:expr, $to_raw:expr,
        $instance_call_name:ident, $instance_call_operation:literal,
        $instance_call_slot:expr, $instance_call_function:ty,
        $static_call_name:ident, $static_call_operation:literal,
        $static_call_slot:expr, $static_call_function:ty,
        $instance_get_name:ident, $instance_set_name:ident,
        $instance_get_operation:literal, $instance_get_slot:expr, $instance_get_function:ty,
        $instance_set_operation:literal, $instance_set_slot:expr, $instance_set_function:ty,
        $get_name:ident, $set_name:ident,
        $get_operation:literal, $get_slot:expr, $get_function:ty,
        $set_operation:literal, $set_slot:expr, $set_function:ty,
        $raw_return:ident;
    )+) => {
        $(
            /// Gets a static primitive field with a detached field ID.
            ///
            /// # Safety
            ///
            /// `field` must have been resolved from `class` in this process ART runtime.
            pub unsafe fn $get_name(
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
                        ($from_raw)(get(env, class, field))
                    },
                )
            }

            /// Sets a static primitive field with a detached field ID.
            ///
            /// # Safety
            ///
            /// `field` must have been resolved from `class` in this process ART runtime.
            pub unsafe fn $set_name(
                &self,
                class: &impl AsJClass,
                field: &FieldId,
                value: $return,
            ) -> Result<()> {
                let value: $raw = ($to_raw)(value);
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

macro_rules! primitive_static_fields {
    () => {
        primitive_jni_table!(primitive_static_fields_from_entries);
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
            /// Creates a primitive Java array initialized from the provided elements.
            pub fn $new_name(&self, elements: &[$element]) -> Result<ArrayRef<'_>> {
                let array = self.new_primitive_array(elements.len(), $new_slot, $new_operation)?;
                self.$set_name(&array, 0, elements)?;
                Ok(array)
            }

            /// Copies a region from a primitive Java array into `output`.
            pub fn $get_name(
                &self,
                array: &(impl AsJObject + ?Sized),
                start: jni::jsize,
                output: &mut [$element],
            ) -> Result<()> {
                self.get_primitive_array_region(array, start, output, $get_slot, $get_operation)
            }

            /// Copies `input` into a region of a primitive Java array.
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
