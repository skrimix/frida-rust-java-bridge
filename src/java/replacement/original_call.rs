use std::{
    ffi::{CString, c_void},
    ptr,
    ptr::NonNull,
};

use crate::{
    Error, Result,
    art::original_method_call_bypass,
    env::{Env, check_pending_exception_preserve_raw, check_pending_exception_raw},
    java::{IntoJavaCallArgs, PreparedJavaCallArgs, raw},
    jni,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

#[cfg(test)]
use crate::java::IntoJavaArgs;

use super::original::RawJavaReturn;

#[doc(hidden)]
pub(crate) unsafe fn call_original_static_method<A: IntoJavaCallArgs>(
    vm: &Vm,
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
    signature: &str,
    args: A,
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    let attached_env = Env::from_raw(env, vm.clone());
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement class",
        });
    }

    let (parsed, args) = prepare_original_call_args_for_env(&attached_env, signature, args)?;
    let name = CString::new(name)?;
    let signature = CString::new(signature)?;
    let get_static_method =
        unsafe { jni::env_function::<jni::GetStaticMethodId>(env, jni::ENV_GET_STATIC_METHOD_ID) };
    let method =
        unsafe { get_static_method(env.as_ptr(), class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check_pending_exception_raw(env, "JNIEnv::GetStaticMethodID")? };
    if method.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetStaticMethodID",
        });
    }

    unsafe {
        call_original_static_by_return(env, class, method, parsed.return_type(), args.values())
    }
}

#[doc(hidden)]
pub(crate) unsafe fn call_original_instance_method<A: IntoJavaCallArgs>(
    vm: &Vm,
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
    signature: &str,
    args: A,
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    let attached_env = Env::from_raw(env, vm.clone());
    if receiver.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement receiver",
        });
    }

    let get_object_class =
        unsafe { jni::env_function::<jni::GetObjectClass>(env, jni::ENV_GET_OBJECT_CLASS) };
    let class = unsafe { get_object_class(env.as_ptr(), receiver) };
    unsafe { check_pending_exception_raw(env, "JNIEnv::GetObjectClass")? };
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetObjectClass",
        });
    }
    let class = LocalRefGuard::new(env, class);

    unsafe {
        let (parsed, args) = prepare_original_call_args_for_env(&attached_env, signature, args)?;
        let name = CString::new(name)?;
        let signature = CString::new(signature)?;
        let get_method = jni::env_function::<jni::GetMethodId>(env, jni::ENV_GET_METHOD_ID);
        let method = get_method(
            env.as_ptr(),
            class.as_jclass(),
            name.as_ptr(),
            signature.as_ptr(),
        );
        check_pending_exception_raw(env, "JNIEnv::GetMethodID")?;
        if method.is_null() {
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetMethodID",
            });
        }

        call_original_instance_by_return(env, receiver, method, parsed.return_type(), args.values())
    }
}

#[doc(hidden)]
pub(crate) unsafe fn call_original_constructor_method<A: IntoJavaCallArgs>(
    vm: &Vm,
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    declaring_class: &raw::Class,
    signature: &str,
    args: A,
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    let attached_env = Env::from_raw(env, vm.clone());
    if receiver.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement receiver",
        });
    }

    let (parsed, args) = prepare_original_call_args_for_env(&attached_env, signature, args)?;
    if parsed.return_type() != &JavaType::Void {
        return Err(Error::InvalidReturnType {
            operation: "OriginalMethod::call_constructor",
            expected: "void",
            actual: parsed.return_type().to_string(),
        });
    }

    let name = CString::new("<init>")?;
    let signature = CString::new(signature)?;
    let get_method = unsafe { jni::env_function::<jni::GetMethodId>(env, jni::ENV_GET_METHOD_ID) };
    let method = unsafe {
        get_method(
            env.as_ptr(),
            declaring_class.raw_jclass(),
            name.as_ptr(),
            signature.as_ptr(),
        )
    };
    unsafe { check_pending_exception_raw(env, "JNIEnv::GetMethodID(<init>)")? };
    if method.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetMethodID(<init>)",
        });
    }

    let args = jni_args(args.values());
    let thread = unsafe { art_thread_from_env(env)? };
    let _bypass = original_method_call_bypass(method as usize, thread);
    let call = unsafe {
        jni::env_function::<jni::CallNonvirtualVoidMethodA>(
            env,
            jni::ENV_CALL_NONVIRTUAL_VOID_METHOD_A,
        )
    };
    unsafe {
        call(
            env.as_ptr(),
            receiver,
            declaring_class.raw_jclass(),
            method,
            jni_args_ptr(&args),
        )
    };
    unsafe { check_pending_exception_preserve_raw(env, "JNIEnv::CallNonvirtualVoidMethodA")? };
    Ok(RawJavaReturn::Void)
}

#[cfg(test)]
pub(crate) fn prepare_original_call_args<A: IntoJavaArgs>(
    signature: &str,
    args: A,
) -> Result<(MethodSignature, Vec<JavaValue>)> {
    let parsed = MethodSignature::parse(signature)?;
    let args = args.into_java_args();
    parsed.validate_arguments(&args)?;
    Ok((parsed, args))
}

pub(crate) fn prepare_original_call_args_for_env<'env, 'vm, A: IntoJavaCallArgs>(
    env: &'env Env<'vm>,
    signature: &str,
    args: A,
) -> Result<(MethodSignature, PreparedJavaCallArgs<'env, 'vm>)> {
    let parsed = MethodSignature::parse(signature)?;
    let args = args.into_java_call_args(env, parsed.arguments())?;
    Ok((parsed, args))
}

fn non_null_env(env: *mut jni::JNIEnv) -> Result<NonNull<jni::JNIEnv>> {
    NonNull::new(env).ok_or(crate::Error::NullReturn {
        operation: "replacement JNIEnv",
    })
}

struct LocalRefGuard {
    env: NonNull<jni::JNIEnv>,
    object: jni::jobject,
}

impl LocalRefGuard {
    fn new(env: NonNull<jni::JNIEnv>, object: jni::jobject) -> Self {
        Self { env, object }
    }

    fn as_jclass(&self) -> jni::jclass {
        self.object
    }
}

impl Drop for LocalRefGuard {
    fn drop(&mut self) {
        let delete_local_ref = unsafe {
            jni::env_function::<jni::DeleteLocalRef>(self.env, jni::ENV_DELETE_LOCAL_REF)
        };
        unsafe { delete_local_ref(self.env.as_ptr(), self.object) };
    }
}

macro_rules! original_static_primitive_return_from_entries {
    ($(
        $return:ty, $raw:ty, $java_type:path, $from_raw:expr, $to_raw:expr,
        $instance_call_name:ident, $instance_call_operation:literal,
        $instance_call_slot:expr, $instance_call_function:ty,
        $static_call_name:ident, $static_call_operation:literal,
        $static_call_slot:expr, $static_call_function:ty,
        $instance_get_name:ident, $instance_set_name:ident,
        $instance_get_operation:literal, $instance_get_slot:expr, $instance_get_function:ty,
        $instance_set_operation:literal, $instance_set_slot:expr, $instance_set_function:ty,
        $static_get_name:ident, $static_set_name:ident,
        $static_get_operation:literal, $static_get_slot:expr, $static_get_function:ty,
        $static_set_operation:literal, $static_set_slot:expr, $static_set_function:ty,
        $raw_return:ident;
    )+) => {
        unsafe fn call_original_static_primitive_by_return(
            env: NonNull<jni::JNIEnv>,
            class: jni::jclass,
            method: jni::jmethodID,
            return_type: &JavaType,
            args: *const jni::jvalue,
        ) -> Option<RawJavaReturn> {
            match return_type {
                $(
                    $java_type => {
                        let call = unsafe {
                            jni::env_function::<$static_call_function>(env, $static_call_slot)
                        };
                        Some(RawJavaReturn::$raw_return(unsafe {
                            call(env.as_ptr(), class, method, args)
                        }))
                    }
                )+
                _ => None,
            }
        }
    };
}

crate::env::macros::primitive_jni_table!(original_static_primitive_return_from_entries);

macro_rules! original_instance_primitive_return_from_entries {
    ($(
        $return:ty, $raw:ty, $java_type:path, $from_raw:expr, $to_raw:expr,
        $instance_call_name:ident, $instance_call_operation:literal,
        $instance_call_slot:expr, $instance_call_function:ty,
        $static_call_name:ident, $static_call_operation:literal,
        $static_call_slot:expr, $static_call_function:ty,
        $instance_get_name:ident, $instance_set_name:ident,
        $instance_get_operation:literal, $instance_get_slot:expr, $instance_get_function:ty,
        $instance_set_operation:literal, $instance_set_slot:expr, $instance_set_function:ty,
        $static_get_name:ident, $static_set_name:ident,
        $static_get_operation:literal, $static_get_slot:expr, $static_get_function:ty,
        $static_set_operation:literal, $static_set_slot:expr, $static_set_function:ty,
        $raw_return:ident;
    )+) => {
        unsafe fn call_original_instance_primitive_by_return(
            env: NonNull<jni::JNIEnv>,
            receiver: jni::jobject,
            method: jni::jmethodID,
            return_type: &JavaType,
            args: *const jni::jvalue,
        ) -> Option<RawJavaReturn> {
            match return_type {
                $(
                    $java_type => {
                        let call = unsafe {
                            jni::env_function::<$instance_call_function>(env, $instance_call_slot)
                        };
                        Some(RawJavaReturn::$raw_return(unsafe {
                            call(env.as_ptr(), receiver, method, args)
                        }))
                    }
                )+
                _ => None,
            }
        }
    };
}

crate::env::macros::primitive_jni_table!(original_instance_primitive_return_from_entries);

fn jni_args(args: &[JavaValue]) -> Vec<jni::jvalue> {
    args.iter().map(|value| value.to_jvalue()).collect()
}

fn jni_args_ptr(args: &[jni::jvalue]) -> *const jni::jvalue {
    if args.is_empty() {
        ptr::null()
    } else {
        args.as_ptr()
    }
}

unsafe fn art_thread_from_env(env: NonNull<jni::JNIEnv>) -> Result<usize> {
    let thread = unsafe { env.as_ptr().cast::<*mut c_void>().add(1).read() as usize };
    if thread == 0 {
        Err(Error::NullReturn {
            operation: "replacement ART thread",
        })
    } else {
        Ok(thread)
    }
}

unsafe fn call_original_static_by_return(
    env: NonNull<jni::JNIEnv>,
    class: jni::jclass,
    method: jni::jmethodID,
    return_type: &JavaType,
    args: &[JavaValue],
) -> Result<RawJavaReturn> {
    let args = jni_args(args);
    let args = jni_args_ptr(&args);
    let thread = unsafe { art_thread_from_env(env)? };
    let _bypass = original_method_call_bypass(method as usize, thread);
    let result = if let Some(result) =
        unsafe { call_original_static_primitive_by_return(env, class, method, return_type, args) }
    {
        result
    } else {
        match return_type {
            JavaType::Void => {
                let call = unsafe {
                    jni::env_function::<jni::CallStaticVoidMethodA>(
                        env,
                        jni::ENV_CALL_STATIC_VOID_METHOD_A,
                    )
                };
                unsafe { call(env.as_ptr(), class, method, args) };
                RawJavaReturn::Void
            }
            JavaType::Object(_) | JavaType::Array(_) => {
                let call = unsafe {
                    jni::env_function::<jni::CallStaticObjectMethodA>(
                        env,
                        jni::ENV_CALL_STATIC_OBJECT_METHOD_A,
                    )
                };
                RawJavaReturn::Object(unsafe { call(env.as_ptr(), class, method, args) })
            }
            JavaType::Boolean
            | JavaType::Byte
            | JavaType::Char
            | JavaType::Short
            | JavaType::Int
            | JavaType::Long
            | JavaType::Float
            | JavaType::Double => unreachable!("primitive return handled by JNI primitive table"),
        }
    };
    unsafe { check_pending_exception_preserve_raw(env, "JNIEnv::CallStaticMethodA")? };
    Ok(result)
}

unsafe fn call_original_instance_by_return(
    env: NonNull<jni::JNIEnv>,
    receiver: jni::jobject,
    method: jni::jmethodID,
    return_type: &JavaType,
    args: &[JavaValue],
) -> Result<RawJavaReturn> {
    let args = jni_args(args);
    let args = jni_args_ptr(&args);
    let thread = unsafe { art_thread_from_env(env)? };
    let _bypass = original_method_call_bypass(method as usize, thread);
    let result = if let Some(result) = unsafe {
        call_original_instance_primitive_by_return(env, receiver, method, return_type, args)
    } {
        result
    } else {
        match return_type {
            JavaType::Void => {
                let call = unsafe {
                    jni::env_function::<jni::CallVoidMethodA>(env, jni::ENV_CALL_VOID_METHOD_A)
                };
                unsafe { call(env.as_ptr(), receiver, method, args) };
                RawJavaReturn::Void
            }
            JavaType::Object(_) | JavaType::Array(_) => {
                let call = unsafe {
                    jni::env_function::<jni::CallObjectMethodA>(env, jni::ENV_CALL_OBJECT_METHOD_A)
                };
                RawJavaReturn::Object(unsafe { call(env.as_ptr(), receiver, method, args) })
            }
            JavaType::Boolean
            | JavaType::Byte
            | JavaType::Char
            | JavaType::Short
            | JavaType::Int
            | JavaType::Long
            | JavaType::Float
            | JavaType::Double => unreachable!("primitive return handled by JNI primitive table"),
        }
    };
    unsafe { check_pending_exception_preserve_raw(env, "JNIEnv::CallMethodA")? };
    Ok(result)
}
