use super::*;
use crate::{error::JavaThrowable, vm::Vm};

const UNKNOWN_JAVA_EXCEPTION: &str = "unknown Java exception";
const EXCEPTION_DETAIL_UNAVAILABLE: &str = "exception detail unavailable";
const EXCEPTION_DETAIL_RAISED: &str =
    "exception detail unavailable because Throwable.toString() raised another Java exception";

pub(crate) struct PendingJavaException {
    env: NonNull<jni::JNIEnv>,
    throwable: jni::jthrowable,
}

impl Env<'_> {
    /// Returns whether a Java exception is currently pending on this thread.
    pub fn exception_check(&self) -> bool {
        let exception_check = self.function::<jni::ExceptionCheck>(jni::ENV_EXCEPTION_CHECK);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_check(self.handle.as_ptr()) == jni::JNI_TRUE }
    }

    /// Clears any Java exception currently pending on this thread.
    pub fn exception_clear(&self) {
        let exception_clear = self.function::<jni::ExceptionClear>(jni::ENV_EXCEPTION_CLEAR);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_clear(self.handle.as_ptr()) };
    }

    /// Returns the pending Java exception as a local reference, if one exists.
    pub fn exception_occurred(&self) -> Option<ThrowableRef<'_>> {
        let throwable = unsafe { self.exception_occurred_raw() };
        unsafe { LocalRef::from_nullable(self.local_ref_scope(), throwable) }
    }

    /// Returns the pending exception local reference, if any.
    ///
    /// # Safety
    ///
    /// The returned local reference follows JNI local reference rules and must only be used on
    /// the current attached thread.
    pub unsafe fn exception_occurred_raw(&self) -> jni::jthrowable {
        let exception_occurred =
            self.function::<jni::ExceptionOccurred>(jni::ENV_EXCEPTION_OCCURRED);
        // SAFETY: The function pointer is read from this JNIEnv's JNI table.
        unsafe { exception_occurred(self.handle.as_ptr()) }
    }
}

pub(crate) unsafe fn take_pending_exception_summary(env: NonNull<jni::JNIEnv>) -> String {
    unsafe { take_pending_exception(env, None) }.0
}

unsafe fn take_pending_exception(
    env: NonNull<jni::JNIEnv>,
    vm: Option<&Vm>,
) -> (String, Option<JavaThrowable>) {
    let exception_occurred =
        unsafe { jni::env_function::<jni::ExceptionOccurred>(env, jni::ENV_EXCEPTION_OCCURRED) };
    let exception = unsafe { exception_occurred(env.as_ptr()) };
    unsafe { clear_pending_exception_raw(env) };

    if exception.is_null() {
        return (UNKNOWN_JAVA_EXCEPTION.to_owned(), None);
    }

    let throwable = unsafe { global_throwable_from_local(env, vm, exception) };
    let summary = unsafe { throwable_to_string(env, exception) }
        .unwrap_or_else(|| EXCEPTION_DETAIL_UNAVAILABLE.to_owned());

    let delete_local_ref =
        unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
    unsafe { delete_local_ref(env.as_ptr(), exception) };

    (summary, throwable)
}

pub(crate) unsafe fn check_pending_exception(
    env: NonNull<jni::JNIEnv>,
    vm: &Vm,
    operation: &'static str,
) -> Result<()> {
    if unsafe { exception_check_raw(env) } {
        let (exception, throwable) = unsafe { take_pending_exception(env, Some(vm)) };
        Err(Error::JavaException {
            operation,
            exception,
            throwable,
        })
    } else {
        Ok(())
    }
}

pub(crate) unsafe fn check_pending_exception_raw(
    env: NonNull<jni::JNIEnv>,
    operation: &'static str,
) -> Result<()> {
    if unsafe { exception_check_raw(env) } {
        let exception = unsafe { take_pending_exception_summary(env) };
        Err(Error::JavaException {
            operation,
            exception,
            throwable: None,
        })
    } else {
        Ok(())
    }
}

pub(crate) unsafe fn check_pending_exception_preserve_raw(
    env: NonNull<jni::JNIEnv>,
    operation: &'static str,
) -> Result<()> {
    if !unsafe { exception_check_raw(env) } {
        return Ok(());
    }

    let exception = unsafe { PendingJavaException::take(env) };
    let summary = exception
        .as_ref()
        .and_then(|exception| unsafe { throwable_to_string(env, exception.throwable) })
        .unwrap_or_else(|| EXCEPTION_DETAIL_UNAVAILABLE.to_owned());

    if let Some(exception) = exception
        && let Err(error) = unsafe { exception.throw() }
    {
        return Err(Error::JavaException {
            operation,
            exception: format!("{summary}; failed to restore pending Java exception: {error}"),
            throwable: None,
        });
    }

    Err(Error::JavaException {
        operation,
        exception: summary,
        throwable: None,
    })
}

pub(crate) unsafe fn throw_new_illegal_state_exception_if_clear_raw(
    env: NonNull<jni::JNIEnv>,
    message: &str,
) -> Result<()> {
    if unsafe { exception_check_raw(env) } {
        return Ok(());
    }

    let class_name = CString::new("java/lang/IllegalStateException")
        .expect("bootstrap exception class name has no interior NUL");
    let find_class = unsafe { jni::env_function::<jni::FindClass>(env, jni::ENV_FIND_CLASS) };
    let exception_class = unsafe { find_class(env.as_ptr(), class_name.as_ptr()) };
    if unsafe { exception_check_raw(env) } {
        return unsafe {
            check_pending_exception_preserve_raw(
                env,
                "JNIEnv::FindClass(java/lang/IllegalStateException)",
            )
        };
    }
    if exception_class.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::FindClass(java/lang/IllegalStateException)",
        });
    }

    let sanitized = message.replace('\0', "\\0");
    let message = CString::new(sanitized)?;
    let throw_new = unsafe { jni::env_function::<jni::ThrowNew>(env, jni::ENV_THROW_NEW) };
    let result = unsafe { throw_new(env.as_ptr(), exception_class, message.as_ptr()) };

    let delete_local_ref =
        unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
    unsafe { delete_local_ref(env.as_ptr(), exception_class) };

    Error::check_jni_result("JNIEnv::ThrowNew", result)
}

unsafe fn global_throwable_from_local(
    env: NonNull<jni::JNIEnv>,
    vm: Option<&Vm>,
    exception: jni::jthrowable,
) -> Option<JavaThrowable> {
    let vm = vm?;
    let new_global_ref =
        unsafe { jni::env_function::<jni::NewGlobalRef>(env, jni::ENV_NEW_GLOBAL_REF) };
    let global = unsafe { new_global_ref(env.as_ptr(), exception) };
    if unsafe { exception_check_raw(env) } {
        unsafe { clear_pending_exception_raw(env) };
        return None;
    }
    if global.is_null() {
        None
    } else {
        Some(unsafe { JavaThrowable::from_global_raw(vm.clone(), global.cast()) })
    }
}

impl PendingJavaException {
    pub(crate) unsafe fn take(env: NonNull<jni::JNIEnv>) -> Option<Self> {
        if !unsafe { exception_check_raw(env) } {
            return None;
        }

        let exception_occurred = unsafe {
            jni::env_function::<jni::ExceptionOccurred>(env, jni::ENV_EXCEPTION_OCCURRED)
        };
        let throwable = unsafe { exception_occurred(env.as_ptr()) };
        unsafe { clear_pending_exception_raw(env) };
        (!throwable.is_null()).then_some(Self { env, throwable })
    }

    pub(crate) unsafe fn throw(self) -> Result<()> {
        let throw = unsafe { jni::env_function::<jni::Throw>(self.env, jni::ENV_THROW) };
        let result = unsafe { throw(self.env.as_ptr(), self.throwable) };
        let delete_local_ref = unsafe {
            jni::env_function::<jni::DeleteLocalRef>(self.env, jni::ENV_DELETE_LOCAL_REF)
        };
        unsafe { delete_local_ref(self.env.as_ptr(), self.throwable) };
        Error::check_jni_result("JNIEnv::Throw", result)
    }
}

unsafe fn throwable_to_string(
    env: NonNull<jni::JNIEnv>,
    exception: jni::jthrowable,
) -> Option<String> {
    let get_object_class =
        unsafe { jni::env_function::<jni::GetObjectClass>(env, jni::ENV_GET_OBJECT_CLASS) };
    let exception_class = unsafe { get_object_class(env.as_ptr(), exception) };
    if let Some(summary) = unsafe { take_detail_exception(env) } {
        return Some(summary);
    }
    if exception_class.is_null() {
        return None;
    }

    let get_method_id =
        unsafe { jni::env_function::<jni::GetMethodId>(env, jni::ENV_GET_METHOD_ID) };
    let name = CString::new("toString").expect("method name has no interior NUL");
    let signature =
        CString::new("()Ljava/lang/String;").expect("method signature has no interior NUL");
    let to_string = unsafe {
        get_method_id(
            env.as_ptr(),
            exception_class,
            name.as_ptr(),
            signature.as_ptr(),
        )
    };

    let delete_local_ref =
        unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
    unsafe { delete_local_ref(env.as_ptr(), exception_class) };

    if let Some(summary) = unsafe { take_detail_exception(env) } {
        return Some(summary);
    }
    if to_string.is_null() {
        return None;
    }

    let call_object_method =
        unsafe { jni::env_function::<jni::CallObjectMethodA>(env, jni::ENV_CALL_OBJECT_METHOD_A) };
    let string = unsafe { call_object_method(env.as_ptr(), exception, to_string, ptr::null()) };
    if let Some(summary) = unsafe { take_detail_exception(env) } {
        return Some(summary);
    }
    if string.is_null() {
        return None;
    }

    let summary = unsafe { java_string_to_lossy_string(env, string) };
    unsafe { delete_local_ref(env.as_ptr(), string) };
    summary
}

unsafe fn java_string_to_lossy_string(
    env: NonNull<jni::JNIEnv>,
    string: jni::jstring,
) -> Option<String> {
    let get_string_length =
        unsafe { jni::env_function::<jni::GetStringLength>(env, jni::ENV_GET_STRING_LENGTH) };
    let get_string_chars =
        unsafe { jni::env_function::<jni::GetStringChars>(env, jni::ENV_GET_STRING_CHARS) };
    let release_string_chars =
        unsafe { jni::env_function::<jni::ReleaseStringChars>(env, jni::ENV_RELEASE_STRING_CHARS) };
    let mut is_copy = jni::JNI_FALSE;

    let length = unsafe { get_string_length(env.as_ptr(), string) };
    if let Some(_summary) = unsafe { take_detail_exception(env) } {
        return None;
    }
    let chars = unsafe { get_string_chars(env.as_ptr(), string, &mut is_copy) };
    if let Some(_summary) = unsafe { take_detail_exception(env) } {
        return None;
    }
    if chars.is_null() {
        return None;
    }

    let chars = unsafe { std::slice::from_raw_parts(chars, length as usize) };
    let summary = std::char::decode_utf16(chars.iter().copied())
        .map(|item| item.unwrap_or(std::char::REPLACEMENT_CHARACTER))
        .collect();

    unsafe { release_string_chars(env.as_ptr(), string, chars.as_ptr()) };

    Some(summary)
}

unsafe fn take_detail_exception(env: NonNull<jni::JNIEnv>) -> Option<String> {
    if unsafe { exception_check_raw(env) } {
        unsafe { clear_pending_exception_raw(env) };
        Some(EXCEPTION_DETAIL_RAISED.to_owned())
    } else {
        None
    }
}

unsafe fn exception_check_raw(env: NonNull<jni::JNIEnv>) -> bool {
    let exception_check =
        unsafe { jni::env_function::<jni::ExceptionCheck>(env, jni::ENV_EXCEPTION_CHECK) };
    unsafe { exception_check(env.as_ptr()) == jni::JNI_TRUE }
}

unsafe fn clear_pending_exception_raw(env: NonNull<jni::JNIEnv>) {
    let exception_clear =
        unsafe { jni::env_function::<jni::ExceptionClear>(env, jni::ENV_EXCEPTION_CLEAR) };
    unsafe { exception_clear(env.as_ptr()) };
}
