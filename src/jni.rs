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
pub type jarray = jobject;
pub type jobjectArray = jarray;
pub type jbooleanArray = jarray;
pub type jbyteArray = jarray;
pub type jcharArray = jarray;
pub type jshortArray = jarray;
pub type jintArray = jarray;
pub type jlongArray = jarray;
pub type jfloatArray = jarray;
pub type jdoubleArray = jarray;
pub type jweak = jobject;
pub type jmethodID = *mut _jmethodID;
pub type jfieldID = *mut _jfieldID;

#[repr(C)]
pub struct _jobject {
    _private: [u8; 0],
}

#[repr(C)]
pub struct _jmethodID {
    _private: [u8; 0],
}

#[repr(C)]
pub struct _jfieldID {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union jvalue {
    pub z: jboolean,
    pub b: jbyte,
    pub c: jchar,
    pub s: jshort,
    pub i: jint,
    pub j: jlong,
    pub f: jfloat,
    pub d: jdouble,
    pub l: jobject,
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

#[repr(C)]
pub struct JavaVMOption {
    pub option_string: *mut c_char,
    pub extra_info: *mut c_void,
}

#[repr(C)]
pub struct JavaVMInitArgs {
    pub version: jint,
    pub n_options: jint,
    pub options: *mut JavaVMOption,
    pub ignore_unrecognized: jboolean,
}

pub type JNICreateJavaVM =
    unsafe extern "C" fn(*mut *mut JavaVM, *mut *mut c_void, *mut JavaVMInitArgs) -> jint;

pub type JNIGetCreatedJavaVMs = unsafe extern "C" fn(*mut *mut JavaVM, jsize, *mut jsize) -> jint;

pub(crate) type AttachCurrentThread =
    unsafe extern "C" fn(*mut JavaVM, *mut *mut JNIEnv, *mut c_void) -> jint;
pub(crate) type DetachCurrentThread = unsafe extern "C" fn(*mut JavaVM) -> jint;
pub(crate) type GetEnv = unsafe extern "C" fn(*mut JavaVM, *mut *mut c_void, jint) -> jint;

pub(crate) type GetVersion = unsafe extern "C" fn(*mut JNIEnv) -> jint;
pub(crate) type FindClass = unsafe extern "C" fn(*mut JNIEnv, *const c_char) -> jclass;
pub(crate) type FromReflectedMethod = unsafe extern "C" fn(*mut JNIEnv, jobject) -> jmethodID;
pub(crate) type FromReflectedField = unsafe extern "C" fn(*mut JNIEnv, jobject) -> jfieldID;
pub(crate) type ToReflectedMethod =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, jboolean) -> jobject;
pub(crate) type ToReflectedField =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jboolean) -> jobject;
pub(crate) type GetMethodId =
    unsafe extern "C" fn(*mut JNIEnv, jclass, *const c_char, *const c_char) -> jmethodID;
pub(crate) type GetObjectClass = unsafe extern "C" fn(*mut JNIEnv, jobject) -> jclass;
pub(crate) type IsInstanceOf = unsafe extern "C" fn(*mut JNIEnv, jobject, jclass) -> jboolean;
pub(crate) type IsSameObject = unsafe extern "C" fn(*mut JNIEnv, jobject, jobject) -> jboolean;
pub(crate) type GetStaticMethodId =
    unsafe extern "C" fn(*mut JNIEnv, jclass, *const c_char, *const c_char) -> jmethodID;
pub(crate) type GetFieldId =
    unsafe extern "C" fn(*mut JNIEnv, jclass, *const c_char, *const c_char) -> jfieldID;
pub(crate) type GetStaticFieldId =
    unsafe extern "C" fn(*mut JNIEnv, jclass, *const c_char, *const c_char) -> jfieldID;
pub(crate) type NewObjectA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jobject;
pub(crate) type CallObjectMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jobject;
pub(crate) type CallBooleanMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jboolean;
pub(crate) type CallByteMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jbyte;
pub(crate) type CallCharMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jchar;
pub(crate) type CallShortMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jshort;
pub(crate) type CallIntMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jint;
pub(crate) type CallLongMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jlong;
pub(crate) type CallFloatMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jfloat;
pub(crate) type CallDoubleMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue) -> jdouble;
pub(crate) type CallVoidMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jmethodID, *const jvalue);
pub(crate) type CallNonvirtualVoidMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jobject, jclass, jmethodID, *const jvalue);
pub(crate) type CallStaticObjectMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jobject;
pub(crate) type CallStaticBooleanMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jboolean;
pub(crate) type CallStaticByteMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jbyte;
pub(crate) type CallStaticCharMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jchar;
pub(crate) type CallStaticShortMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jshort;
pub(crate) type CallStaticIntMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jint;
pub(crate) type CallStaticLongMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jlong;
pub(crate) type CallStaticFloatMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jfloat;
pub(crate) type CallStaticDoubleMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue) -> jdouble;
pub(crate) type CallStaticVoidMethodA =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jmethodID, *const jvalue);
pub(crate) type GetObjectField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jobject;
pub(crate) type GetBooleanField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jboolean;
pub(crate) type GetByteField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jbyte;
pub(crate) type GetCharField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jchar;
pub(crate) type GetShortField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jshort;
pub(crate) type GetIntField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jint;
pub(crate) type GetLongField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jlong;
pub(crate) type GetFloatField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jfloat;
pub(crate) type GetDoubleField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID) -> jdouble;
pub(crate) type SetObjectField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jobject);
pub(crate) type SetBooleanField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jboolean);
pub(crate) type SetByteField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jbyte);
pub(crate) type SetCharField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jchar);
pub(crate) type SetShortField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jshort);
pub(crate) type SetIntField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jint);
pub(crate) type SetLongField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jlong);
pub(crate) type SetFloatField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jfloat);
pub(crate) type SetDoubleField = unsafe extern "C" fn(*mut JNIEnv, jobject, jfieldID, jdouble);
pub(crate) type GetStaticObjectField =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jobject;
pub(crate) type GetStaticBooleanField =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jboolean;
pub(crate) type GetStaticByteField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jbyte;
pub(crate) type GetStaticCharField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jchar;
pub(crate) type GetStaticShortField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jshort;
pub(crate) type GetStaticIntField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jint;
pub(crate) type GetStaticLongField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jlong;
pub(crate) type GetStaticFloatField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jfloat;
pub(crate) type GetStaticDoubleField =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID) -> jdouble;
pub(crate) type SetStaticObjectField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jobject);
pub(crate) type SetStaticBooleanField =
    unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jboolean);
pub(crate) type SetStaticByteField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jbyte);
pub(crate) type SetStaticCharField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jchar);
pub(crate) type SetStaticShortField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jshort);
pub(crate) type SetStaticIntField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jint);
pub(crate) type SetStaticLongField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jlong);
pub(crate) type SetStaticFloatField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jfloat);
pub(crate) type SetStaticDoubleField = unsafe extern "C" fn(*mut JNIEnv, jclass, jfieldID, jdouble);
pub(crate) type ExceptionOccurred = unsafe extern "C" fn(*mut JNIEnv) -> jthrowable;
pub(crate) type ExceptionClear = unsafe extern "C" fn(*mut JNIEnv);
pub(crate) type NewGlobalRef = unsafe extern "C" fn(*mut JNIEnv, jobject) -> jobject;
pub(crate) type DeleteGlobalRef = unsafe extern "C" fn(*mut JNIEnv, jobject);
pub(crate) type DeleteLocalRef = unsafe extern "C" fn(*mut JNIEnv, jobject);
pub(crate) type GetStringLength = unsafe extern "C" fn(*mut JNIEnv, jstring) -> jsize;
pub(crate) type GetStringChars =
    unsafe extern "C" fn(*mut JNIEnv, jstring, *mut jboolean) -> *const jchar;
pub(crate) type ReleaseStringChars = unsafe extern "C" fn(*mut JNIEnv, jstring, *const jchar);
pub(crate) type NewStringUtf = unsafe extern "C" fn(*mut JNIEnv, *const c_char) -> jstring;
pub(crate) type GetStringUtfChars =
    unsafe extern "C" fn(*mut JNIEnv, jstring, *mut jboolean) -> *const c_char;
pub(crate) type ReleaseStringUtfChars = unsafe extern "C" fn(*mut JNIEnv, jstring, *const c_char);
pub(crate) type GetArrayLength = unsafe extern "C" fn(*mut JNIEnv, jarray) -> jsize;
pub(crate) type NewObjectArray =
    unsafe extern "C" fn(*mut JNIEnv, jsize, jclass, jobject) -> jobjectArray;
pub(crate) type GetObjectArrayElement =
    unsafe extern "C" fn(*mut JNIEnv, jobjectArray, jsize) -> jobject;
pub(crate) type SetObjectArrayElement =
    unsafe extern "C" fn(*mut JNIEnv, jobjectArray, jsize, jobject);
pub(crate) type ExceptionCheck = unsafe extern "C" fn(*mut JNIEnv) -> jboolean;

pub(crate) const JVM_ATTACH_CURRENT_THREAD: usize = 4;
pub(crate) const JVM_DETACH_CURRENT_THREAD: usize = 5;
pub(crate) const JVM_GET_ENV: usize = 6;

pub(crate) const ENV_GET_VERSION: usize = 4;
pub(crate) const ENV_FIND_CLASS: usize = 6;
pub(crate) const ENV_FROM_REFLECTED_METHOD: usize = 7;
pub(crate) const ENV_FROM_REFLECTED_FIELD: usize = 8;
pub(crate) const ENV_TO_REFLECTED_METHOD: usize = 9;
pub(crate) const ENV_TO_REFLECTED_FIELD: usize = 12;
pub(crate) const ENV_EXCEPTION_OCCURRED: usize = 15;
pub(crate) const ENV_EXCEPTION_CLEAR: usize = 17;
pub(crate) const ENV_FATAL_ERROR: usize = 18;
pub(crate) const ENV_NEW_GLOBAL_REF: usize = 21;
pub(crate) const ENV_DELETE_GLOBAL_REF: usize = 22;
pub(crate) const ENV_DELETE_LOCAL_REF: usize = 23;
pub(crate) const ENV_IS_SAME_OBJECT: usize = 24;
pub(crate) const ENV_NEW_OBJECT_A: usize = 30;
pub(crate) const ENV_GET_OBJECT_CLASS: usize = 31;
pub(crate) const ENV_IS_INSTANCE_OF: usize = 32;
pub(crate) const ENV_GET_METHOD_ID: usize = 33;
pub(crate) const ENV_CALL_OBJECT_METHOD_A: usize = 36;
pub(crate) const ENV_CALL_BOOLEAN_METHOD_A: usize = 39;
pub(crate) const ENV_CALL_BYTE_METHOD_A: usize = 42;
pub(crate) const ENV_CALL_CHAR_METHOD_A: usize = 45;
pub(crate) const ENV_CALL_SHORT_METHOD_A: usize = 48;
pub(crate) const ENV_CALL_INT_METHOD_A: usize = 51;
pub(crate) const ENV_CALL_LONG_METHOD_A: usize = 54;
pub(crate) const ENV_CALL_FLOAT_METHOD_A: usize = 57;
pub(crate) const ENV_CALL_DOUBLE_METHOD_A: usize = 60;
pub(crate) const ENV_CALL_VOID_METHOD_A: usize = 63;
pub(crate) const ENV_CALL_NONVIRTUAL_VOID_METHOD_A: usize = 93;
pub(crate) const ENV_GET_FIELD_ID: usize = 94;
pub(crate) const ENV_GET_OBJECT_FIELD: usize = 95;
pub(crate) const ENV_GET_BOOLEAN_FIELD: usize = 96;
pub(crate) const ENV_GET_BYTE_FIELD: usize = 97;
pub(crate) const ENV_GET_CHAR_FIELD: usize = 98;
pub(crate) const ENV_GET_SHORT_FIELD: usize = 99;
pub(crate) const ENV_GET_INT_FIELD: usize = 100;
pub(crate) const ENV_GET_LONG_FIELD: usize = 101;
pub(crate) const ENV_GET_FLOAT_FIELD: usize = 102;
pub(crate) const ENV_GET_DOUBLE_FIELD: usize = 103;
pub(crate) const ENV_SET_OBJECT_FIELD: usize = 104;
pub(crate) const ENV_SET_BOOLEAN_FIELD: usize = 105;
pub(crate) const ENV_SET_BYTE_FIELD: usize = 106;
pub(crate) const ENV_SET_CHAR_FIELD: usize = 107;
pub(crate) const ENV_SET_SHORT_FIELD: usize = 108;
pub(crate) const ENV_SET_INT_FIELD: usize = 109;
pub(crate) const ENV_SET_LONG_FIELD: usize = 110;
pub(crate) const ENV_SET_FLOAT_FIELD: usize = 111;
pub(crate) const ENV_SET_DOUBLE_FIELD: usize = 112;
pub(crate) const ENV_GET_STATIC_METHOD_ID: usize = 113;
pub(crate) const ENV_CALL_STATIC_OBJECT_METHOD_A: usize = 116;
pub(crate) const ENV_CALL_STATIC_BOOLEAN_METHOD_A: usize = 119;
pub(crate) const ENV_CALL_STATIC_BYTE_METHOD_A: usize = 122;
pub(crate) const ENV_CALL_STATIC_CHAR_METHOD_A: usize = 125;
pub(crate) const ENV_CALL_STATIC_SHORT_METHOD_A: usize = 128;
pub(crate) const ENV_CALL_STATIC_INT_METHOD_A: usize = 131;
pub(crate) const ENV_CALL_STATIC_LONG_METHOD_A: usize = 134;
pub(crate) const ENV_CALL_STATIC_FLOAT_METHOD_A: usize = 137;
pub(crate) const ENV_CALL_STATIC_DOUBLE_METHOD_A: usize = 140;
pub(crate) const ENV_CALL_STATIC_VOID_METHOD_A: usize = 143;
pub(crate) const ENV_GET_STATIC_FIELD_ID: usize = 144;
pub(crate) const ENV_GET_STATIC_OBJECT_FIELD: usize = 145;
pub(crate) const ENV_GET_STATIC_BOOLEAN_FIELD: usize = 146;
pub(crate) const ENV_GET_STATIC_BYTE_FIELD: usize = 147;
pub(crate) const ENV_GET_STATIC_CHAR_FIELD: usize = 148;
pub(crate) const ENV_GET_STATIC_SHORT_FIELD: usize = 149;
pub(crate) const ENV_GET_STATIC_INT_FIELD: usize = 150;
pub(crate) const ENV_GET_STATIC_LONG_FIELD: usize = 151;
pub(crate) const ENV_GET_STATIC_FLOAT_FIELD: usize = 152;
pub(crate) const ENV_GET_STATIC_DOUBLE_FIELD: usize = 153;
pub(crate) const ENV_SET_STATIC_OBJECT_FIELD: usize = 154;
pub(crate) const ENV_SET_STATIC_BOOLEAN_FIELD: usize = 155;
pub(crate) const ENV_SET_STATIC_BYTE_FIELD: usize = 156;
pub(crate) const ENV_SET_STATIC_CHAR_FIELD: usize = 157;
pub(crate) const ENV_SET_STATIC_SHORT_FIELD: usize = 158;
pub(crate) const ENV_SET_STATIC_INT_FIELD: usize = 159;
pub(crate) const ENV_SET_STATIC_LONG_FIELD: usize = 160;
pub(crate) const ENV_SET_STATIC_FLOAT_FIELD: usize = 161;
pub(crate) const ENV_SET_STATIC_DOUBLE_FIELD: usize = 162;
pub(crate) const ENV_GET_STRING_LENGTH: usize = 164;
pub(crate) const ENV_GET_STRING_CHARS: usize = 165;
pub(crate) const ENV_RELEASE_STRING_CHARS: usize = 166;
pub(crate) const ENV_NEW_STRING_UTF: usize = 167;
pub(crate) const ENV_GET_STRING_UTF_CHARS: usize = 169;
pub(crate) const ENV_RELEASE_STRING_UTF_CHARS: usize = 170;
pub(crate) const ENV_GET_ARRAY_LENGTH: usize = 171;
pub(crate) const ENV_NEW_OBJECT_ARRAY: usize = 172;
pub(crate) const ENV_GET_OBJECT_ARRAY_ELEMENT: usize = 173;
pub(crate) const ENV_SET_OBJECT_ARRAY_ELEMENT: usize = 174;
pub(crate) const ENV_NEW_BOOLEAN_ARRAY: usize = 175;
pub(crate) const ENV_NEW_BYTE_ARRAY: usize = 176;
pub(crate) const ENV_NEW_CHAR_ARRAY: usize = 177;
pub(crate) const ENV_NEW_SHORT_ARRAY: usize = 178;
pub(crate) const ENV_NEW_INT_ARRAY: usize = 179;
pub(crate) const ENV_NEW_LONG_ARRAY: usize = 180;
pub(crate) const ENV_NEW_FLOAT_ARRAY: usize = 181;
pub(crate) const ENV_NEW_DOUBLE_ARRAY: usize = 182;
pub(crate) const ENV_GET_BOOLEAN_ARRAY_REGION: usize = 199;
pub(crate) const ENV_GET_BYTE_ARRAY_REGION: usize = 200;
pub(crate) const ENV_GET_CHAR_ARRAY_REGION: usize = 201;
pub(crate) const ENV_GET_SHORT_ARRAY_REGION: usize = 202;
pub(crate) const ENV_GET_INT_ARRAY_REGION: usize = 203;
pub(crate) const ENV_GET_LONG_ARRAY_REGION: usize = 204;
pub(crate) const ENV_GET_FLOAT_ARRAY_REGION: usize = 205;
pub(crate) const ENV_GET_DOUBLE_ARRAY_REGION: usize = 206;
pub(crate) const ENV_SET_BOOLEAN_ARRAY_REGION: usize = 207;
pub(crate) const ENV_SET_BYTE_ARRAY_REGION: usize = 208;
pub(crate) const ENV_SET_CHAR_ARRAY_REGION: usize = 209;
pub(crate) const ENV_SET_SHORT_ARRAY_REGION: usize = 210;
pub(crate) const ENV_SET_INT_ARRAY_REGION: usize = 211;
pub(crate) const ENV_SET_LONG_ARRAY_REGION: usize = 212;
pub(crate) const ENV_SET_FLOAT_ARRAY_REGION: usize = 213;
pub(crate) const ENV_SET_DOUBLE_ARRAY_REGION: usize = 214;
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
