#![allow(non_camel_case_types)]

use std::{
    ffi::{c_char, c_void},
    mem,
    ptr::NonNull,
};

pub type jint = i32;
pub type jboolean = u8;
pub type jbyte = i8;
pub type jchar = u16;
pub type jshort = i16;
pub type jlong = i64;
pub type jfloat = f32;
pub type jdouble = f64;
pub type jsize = jint;

pub type jobject = *mut _jobject;
pub type jclass = jobject;
pub type jstring = jobject;
pub type jthrowable = jobject;

#[repr(C)]
pub struct _jobject {
    _private: [u8; 0],
}

#[repr(C)]
pub struct JavaVM {
    functions: *const *const c_void,
}

#[repr(C)]
pub struct JNIEnv {
    functions: *const *const c_void,
}

pub const JNI_OK: jint = 0;
pub const JNI_ERR: jint = -1;
pub const JNI_EDETACHED: jint = -2;
pub const JNI_EVERSION: jint = -3;

pub const JNI_FALSE: jboolean = 0;
pub const JNI_TRUE: jboolean = 1;

pub const JNI_VERSION_1_6: jint = 0x0001_0006;

pub type JNIGetCreatedJavaVMs = unsafe extern "C" fn(*mut *mut JavaVM, jsize, *mut jsize) -> jint;

pub(crate) type AttachCurrentThread =
    unsafe extern "C" fn(*mut JavaVM, *mut *mut JNIEnv, *mut c_void) -> jint;
pub(crate) type DetachCurrentThread = unsafe extern "C" fn(*mut JavaVM) -> jint;
pub(crate) type GetEnv = unsafe extern "C" fn(*mut JavaVM, *mut *mut c_void, jint) -> jint;

pub(crate) type GetVersion = unsafe extern "C" fn(*mut JNIEnv) -> jint;
pub(crate) type FindClass = unsafe extern "C" fn(*mut JNIEnv, *const c_char) -> jclass;
pub(crate) type ExceptionOccurred = unsafe extern "C" fn(*mut JNIEnv) -> jthrowable;
pub(crate) type ExceptionClear = unsafe extern "C" fn(*mut JNIEnv);
pub(crate) type NewGlobalRef = unsafe extern "C" fn(*mut JNIEnv, jobject) -> jobject;
pub(crate) type DeleteGlobalRef = unsafe extern "C" fn(*mut JNIEnv, jobject);
pub(crate) type DeleteLocalRef = unsafe extern "C" fn(*mut JNIEnv, jobject);
pub(crate) type NewStringUtf = unsafe extern "C" fn(*mut JNIEnv, *const c_char) -> jstring;
pub(crate) type GetStringUtfChars =
    unsafe extern "C" fn(*mut JNIEnv, jstring, *mut jboolean) -> *const c_char;
pub(crate) type ReleaseStringUtfChars = unsafe extern "C" fn(*mut JNIEnv, jstring, *const c_char);
pub(crate) type ExceptionCheck = unsafe extern "C" fn(*mut JNIEnv) -> jboolean;

pub(crate) const JVM_ATTACH_CURRENT_THREAD: usize = 4;
pub(crate) const JVM_DETACH_CURRENT_THREAD: usize = 5;
pub(crate) const JVM_GET_ENV: usize = 6;

pub(crate) const ENV_GET_VERSION: usize = 4;
pub(crate) const ENV_FIND_CLASS: usize = 6;
pub(crate) const ENV_EXCEPTION_OCCURRED: usize = 15;
pub(crate) const ENV_EXCEPTION_CLEAR: usize = 17;
pub(crate) const ENV_NEW_GLOBAL_REF: usize = 21;
pub(crate) const ENV_DELETE_GLOBAL_REF: usize = 22;
pub(crate) const ENV_DELETE_LOCAL_REF: usize = 23;
pub(crate) const ENV_NEW_STRING_UTF: usize = 167;
pub(crate) const ENV_GET_STRING_UTF_CHARS: usize = 169;
pub(crate) const ENV_RELEASE_STRING_UTF_CHARS: usize = 170;
pub(crate) const ENV_EXCEPTION_CHECK: usize = 228;

pub(crate) unsafe fn vm_function<T: Copy>(vm: NonNull<JavaVM>, slot: usize) -> T {
    // SAFETY: JavaVM is a JNI handle whose first word is a valid function table pointer.
    let functions = unsafe { (*vm.as_ptr()).functions };
    let pointer = unsafe { *functions.add(slot) };
    debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<*const c_void>());
    unsafe { mem::transmute_copy(&pointer) }
}

pub(crate) unsafe fn env_function<T: Copy>(env: NonNull<JNIEnv>, slot: usize) -> T {
    // SAFETY: JNIEnv is a JNI handle whose first word is a valid function table pointer.
    let functions = unsafe { (*env.as_ptr()).functions };
    let pointer = unsafe { *functions.add(slot) };
    debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<*const c_void>());
    unsafe { mem::transmute_copy(&pointer) }
}
