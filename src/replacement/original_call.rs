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
    let attached_env = Env::from_raw(env, vm);
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
    let attached_env = Env::from_raw(env, vm);
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
    let attached_env = Env::from_raw(env, vm);
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
    let result = match return_type {
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
        JavaType::Boolean => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticBooleanMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_BOOLEAN_METHOD_A,
                )
            };
            RawJavaReturn::Boolean(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Byte => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticByteMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_BYTE_METHOD_A,
                )
            };
            RawJavaReturn::Byte(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Char => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticCharMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_CHAR_METHOD_A,
                )
            };
            RawJavaReturn::Char(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Short => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticShortMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_SHORT_METHOD_A,
                )
            };
            RawJavaReturn::Short(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Int => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticIntMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_INT_METHOD_A,
                )
            };
            RawJavaReturn::Int(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Long => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticLongMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_LONG_METHOD_A,
                )
            };
            RawJavaReturn::Long(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Float => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticFloatMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_FLOAT_METHOD_A,
                )
            };
            RawJavaReturn::Float(unsafe { call(env.as_ptr(), class, method, args) })
        }
        JavaType::Double => {
            let call = unsafe {
                jni::env_function::<jni::CallStaticDoubleMethodA>(
                    env,
                    jni::ENV_CALL_STATIC_DOUBLE_METHOD_A,
                )
            };
            RawJavaReturn::Double(unsafe { call(env.as_ptr(), class, method, args) })
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
    let result = match return_type {
        JavaType::Void => {
            let call = unsafe {
                jni::env_function::<jni::CallVoidMethodA>(env, jni::ENV_CALL_VOID_METHOD_A)
            };
            unsafe { call(env.as_ptr(), receiver, method, args) };
            RawJavaReturn::Void
        }
        JavaType::Boolean => {
            let call = unsafe {
                jni::env_function::<jni::CallBooleanMethodA>(env, jni::ENV_CALL_BOOLEAN_METHOD_A)
            };
            RawJavaReturn::Boolean(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Byte => {
            let call = unsafe {
                jni::env_function::<jni::CallByteMethodA>(env, jni::ENV_CALL_BYTE_METHOD_A)
            };
            RawJavaReturn::Byte(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Char => {
            let call = unsafe {
                jni::env_function::<jni::CallCharMethodA>(env, jni::ENV_CALL_CHAR_METHOD_A)
            };
            RawJavaReturn::Char(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Short => {
            let call = unsafe {
                jni::env_function::<jni::CallShortMethodA>(env, jni::ENV_CALL_SHORT_METHOD_A)
            };
            RawJavaReturn::Short(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Int => {
            let call = unsafe {
                jni::env_function::<jni::CallIntMethodA>(env, jni::ENV_CALL_INT_METHOD_A)
            };
            RawJavaReturn::Int(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Long => {
            let call = unsafe {
                jni::env_function::<jni::CallLongMethodA>(env, jni::ENV_CALL_LONG_METHOD_A)
            };
            RawJavaReturn::Long(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Float => {
            let call = unsafe {
                jni::env_function::<jni::CallFloatMethodA>(env, jni::ENV_CALL_FLOAT_METHOD_A)
            };
            RawJavaReturn::Float(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Double => {
            let call = unsafe {
                jni::env_function::<jni::CallDoubleMethodA>(env, jni::ENV_CALL_DOUBLE_METHOD_A)
            };
            RawJavaReturn::Double(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
        JavaType::Object(_) | JavaType::Array(_) => {
            let call = unsafe {
                jni::env_function::<jni::CallObjectMethodA>(env, jni::ENV_CALL_OBJECT_METHOD_A)
            };
            RawJavaReturn::Object(unsafe { call(env.as_ptr(), receiver, method, args) })
        }
    };
    unsafe { check_pending_exception_preserve_raw(env, "JNIEnv::CallMethodA")? };
    Ok(result)
}
