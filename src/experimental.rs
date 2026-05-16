//! High-risk ART method replacement prototypes.
//!
//! In this crate, `experimental` does not mark the only unstable API boundary. The whole project is
//! a private pre-user experiment, and exported APIs may change. This module is specifically for
//! test-facing method replacement scaffolding that is more dangerous, more ART-layout-sensitive, or
//! less ergonomic than the rest of the current bridge surface.

use std::{
    ffi::{CString, c_int, c_void},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    sync::Mutex,
};

use frida_gum::instruction_writer::{Aarch64InstructionWriter, Aarch64Register, InstructionWriter};

use crate::{
    Error, Result,
    art::{ArtMethodReplacementGuard, original_method_call_bypass},
    env::{Env, MethodKind},
    java::{IntoJavaArgs, JavaClass, JavaMethodOverload},
    jni,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

const FEATURE_CLOSURE_REPLACEMENT: &str = "closure-backed method replacement";

unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        length: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        offset: isize,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, length: usize, prot: c_int) -> c_int;
    fn munmap(addr: *mut c_void, length: usize) -> c_int;
}

pub type StaticVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass);
pub type StaticStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jstring;
pub type StaticBooleanReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jboolean;
pub type StaticByteReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jbyte;
pub type StaticCharReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jchar;
pub type StaticShortReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jshort;
pub type StaticI32ReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jint;
pub type StaticI64ReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jlong;
pub type StaticF32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jfloat;
pub type StaticF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass) -> jni::jdouble;
pub type StaticStringToStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jstring) -> jni::jstring;
pub type StaticReferenceToReferenceReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jobject) -> jni::jobject;
pub type StaticI32I32ToI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jint, jni::jint) -> jni::jint;
pub type StaticZBCSToI32ReplacementFn = unsafe extern "C" fn(
    *mut jni::JNIEnv,
    jni::jclass,
    jni::jboolean,
    jni::jbyte,
    jni::jchar,
    jni::jshort,
) -> jni::jint;
pub type StaticI64F64ToI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jlong, jni::jdouble) -> jni::jlong;
pub type StaticF32F64ToF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jclass, jni::jfloat, jni::jdouble) -> jni::jdouble;
pub type InstanceVoidReplacementFn = unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject);
pub type InstanceBooleanReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jboolean;
pub type InstanceByteReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jbyte;
pub type InstanceCharReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jchar;
pub type InstanceShortReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jshort;
pub type InstanceI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jint;
pub type InstanceI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jlong;
pub type InstanceF32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jfloat;
pub type InstanceF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jdouble;
pub type InstanceStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject) -> jni::jstring;
pub type InstanceStringToStringReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jstring) -> jni::jstring;
pub type InstanceReferenceToReferenceReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jobject) -> jni::jobject;
pub type InstanceReferenceToVoidReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jobject);
pub type InstanceI32I32ToI32ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jint, jni::jint) -> jni::jint;
pub type InstanceZBCSToI32ReplacementFn = unsafe extern "C" fn(
    *mut jni::JNIEnv,
    jni::jobject,
    jni::jboolean,
    jni::jbyte,
    jni::jchar,
    jni::jshort,
) -> jni::jint;
pub type InstanceI64F64ToI64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jlong, jni::jdouble) -> jni::jlong;
pub type InstanceF32F64ToF64ReplacementFn =
    unsafe extern "C" fn(*mut jni::JNIEnv, jni::jobject, jni::jfloat, jni::jdouble) -> jni::jdouble;

/// A JNI-native implementation supported by the current experimental overload facade.
///
/// Each variant names the exact method kind and ABI shape accepted by the hidden ART backend. This
/// intentionally keeps unsupported signatures visible instead of weakening type checks.
#[derive(Clone, Copy)]
pub enum MethodImplementation {
    StaticVoid(StaticVoidReplacementFn),
    StaticString(StaticStringReplacementFn),
    StaticBoolean(StaticBooleanReplacementFn),
    StaticByte(StaticByteReplacementFn),
    StaticChar(StaticCharReplacementFn),
    StaticShort(StaticShortReplacementFn),
    StaticI32(StaticI32ReplacementFn),
    StaticI64(StaticI64ReplacementFn),
    StaticF32(StaticF32ReplacementFn),
    StaticF64(StaticF64ReplacementFn),
    StaticStringToString(StaticStringToStringReplacementFn),
    StaticReferenceToReference(StaticReferenceToReferenceReplacementFn),
    StaticI32I32ToI32(StaticI32I32ToI32ReplacementFn),
    StaticZBCSToI32(StaticZBCSToI32ReplacementFn),
    StaticI64F64ToI64(StaticI64F64ToI64ReplacementFn),
    StaticF32F64ToF64(StaticF32F64ToF64ReplacementFn),
    InstanceVoid(InstanceVoidReplacementFn),
    InstanceBoolean(InstanceBooleanReplacementFn),
    InstanceByte(InstanceByteReplacementFn),
    InstanceChar(InstanceCharReplacementFn),
    InstanceShort(InstanceShortReplacementFn),
    InstanceI32(InstanceI32ReplacementFn),
    InstanceI64(InstanceI64ReplacementFn),
    InstanceF32(InstanceF32ReplacementFn),
    InstanceF64(InstanceF64ReplacementFn),
    InstanceString(InstanceStringReplacementFn),
    InstanceStringToString(InstanceStringToStringReplacementFn),
    InstanceReferenceToReference(InstanceReferenceToReferenceReplacementFn),
    InstanceReferenceToVoid(InstanceReferenceToVoidReplacementFn),
    InstanceI32I32ToI32(InstanceI32I32ToI32ReplacementFn),
    InstanceZBCSToI32(InstanceZBCSToI32ReplacementFn),
    InstanceI64F64ToI64(InstanceI64F64ToI64ReplacementFn),
    InstanceF32F64ToF64(InstanceF32F64ToF64ReplacementFn),
}

/// A raw JNI-native implementation for a supported experimental replacement ABI.
///
/// This is the descriptor-driven layer underneath the signature-specific helpers above. It still
/// requires an exact JNI-native callback ABI and only accepts the ABI shapes tested by the current
/// hidden backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeMethodImplementation {
    kind: MethodKind,
    signature: NativeImplementationSignature,
    function: *mut c_void,
    implementation_name: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NativeImplementationSignature {
    Exact(String),
    OneReferenceToReference,
}

impl NativeMethodImplementation {
    /// Creates a raw static-method implementation for a supported replacement signature.
    ///
    /// # Safety
    ///
    /// `function` must point to a valid JNI native function matching `signature` exactly and must
    /// remain valid until the returned replacement guard is reverted or dropped.
    pub unsafe fn static_method(signature: &str, function: *mut c_void) -> Result<Self> {
        Self::new(
            MethodKind::Static,
            signature,
            function,
            "NativeMethodImplementation",
            "NativeMethodImplementation::static_method",
        )
    }

    /// Creates a raw instance-method implementation for a supported replacement signature.
    ///
    /// # Safety
    ///
    /// `function` must point to a valid JNI native function matching `signature` exactly and must
    /// remain valid until the returned replacement guard is reverted or dropped.
    pub unsafe fn instance_method(signature: &str, function: *mut c_void) -> Result<Self> {
        Self::new(
            MethodKind::Instance,
            signature,
            function,
            "NativeMethodImplementation",
            "NativeMethodImplementation::instance_method",
        )
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn signature(&self) -> &str {
        match &self.signature {
            NativeImplementationSignature::Exact(signature) => signature,
            NativeImplementationSignature::OneReferenceToReference => {
                "one-reference-argument/reference-return"
            }
        }
    }

    fn typed(
        kind: MethodKind,
        signature: &'static str,
        function: *mut c_void,
        implementation_name: &'static str,
    ) -> Result<Self> {
        Self::new(
            kind,
            signature,
            function,
            implementation_name,
            "experimental::replace_method",
        )
    }

    fn new(
        kind: MethodKind,
        signature: &str,
        function: *mut c_void,
        implementation_name: &'static str,
        operation: &'static str,
    ) -> Result<Self> {
        let signature = supported_replacement_signature(kind, signature, operation)?;
        Ok(Self {
            kind,
            signature: NativeImplementationSignature::Exact(signature),
            function,
            implementation_name,
        })
    }

    fn typed_reference_to_reference(
        kind: MethodKind,
        function: *mut c_void,
        implementation_name: &'static str,
    ) -> Self {
        Self {
            kind,
            signature: NativeImplementationSignature::OneReferenceToReference,
            function,
            implementation_name,
        }
    }
}

impl MethodImplementation {
    fn into_native(self) -> Result<NativeMethodImplementation> {
        match self {
            Self::StaticVoid(function) => typed_native(
                MethodKind::Static,
                "()V",
                function as *const () as *mut c_void,
                "StaticVoid",
            ),
            Self::StaticString(function) => typed_native(
                MethodKind::Static,
                "()Ljava/lang/String;",
                function as *const () as *mut c_void,
                "StaticString",
            ),
            Self::StaticBoolean(function) => typed_native(
                MethodKind::Static,
                "()Z",
                function as *const () as *mut c_void,
                "StaticBoolean",
            ),
            Self::StaticByte(function) => typed_native(
                MethodKind::Static,
                "()B",
                function as *const () as *mut c_void,
                "StaticByte",
            ),
            Self::StaticChar(function) => typed_native(
                MethodKind::Static,
                "()C",
                function as *const () as *mut c_void,
                "StaticChar",
            ),
            Self::StaticShort(function) => typed_native(
                MethodKind::Static,
                "()S",
                function as *const () as *mut c_void,
                "StaticShort",
            ),
            Self::StaticI32(function) => typed_native(
                MethodKind::Static,
                "()I",
                function as *const () as *mut c_void,
                "StaticI32",
            ),
            Self::StaticI64(function) => typed_native(
                MethodKind::Static,
                "()J",
                function as *const () as *mut c_void,
                "StaticI64",
            ),
            Self::StaticF32(function) => typed_native(
                MethodKind::Static,
                "()F",
                function as *const () as *mut c_void,
                "StaticF32",
            ),
            Self::StaticF64(function) => typed_native(
                MethodKind::Static,
                "()D",
                function as *const () as *mut c_void,
                "StaticF64",
            ),
            Self::StaticStringToString(function) => typed_native(
                MethodKind::Static,
                "(Ljava/lang/String;)Ljava/lang/String;",
                function as *const () as *mut c_void,
                "StaticStringToString",
            ),
            Self::StaticReferenceToReference(function) => Ok(typed_reference_native(
                MethodKind::Static,
                function as *const () as *mut c_void,
                "StaticReferenceToReference",
            )),
            Self::StaticI32I32ToI32(function) => typed_native(
                MethodKind::Static,
                "(II)I",
                function as *const () as *mut c_void,
                "StaticI32I32ToI32",
            ),
            Self::StaticZBCSToI32(function) => typed_native(
                MethodKind::Static,
                "(ZBCS)I",
                function as *const () as *mut c_void,
                "StaticZBCSToI32",
            ),
            Self::StaticI64F64ToI64(function) => typed_native(
                MethodKind::Static,
                "(JD)J",
                function as *const () as *mut c_void,
                "StaticI64F64ToI64",
            ),
            Self::StaticF32F64ToF64(function) => typed_native(
                MethodKind::Static,
                "(FD)D",
                function as *const () as *mut c_void,
                "StaticF32F64ToF64",
            ),
            Self::InstanceVoid(function) => typed_native(
                MethodKind::Instance,
                "()V",
                function as *const () as *mut c_void,
                "InstanceVoid",
            ),
            Self::InstanceBoolean(function) => typed_native(
                MethodKind::Instance,
                "()Z",
                function as *const () as *mut c_void,
                "InstanceBoolean",
            ),
            Self::InstanceByte(function) => typed_native(
                MethodKind::Instance,
                "()B",
                function as *const () as *mut c_void,
                "InstanceByte",
            ),
            Self::InstanceChar(function) => typed_native(
                MethodKind::Instance,
                "()C",
                function as *const () as *mut c_void,
                "InstanceChar",
            ),
            Self::InstanceShort(function) => typed_native(
                MethodKind::Instance,
                "()S",
                function as *const () as *mut c_void,
                "InstanceShort",
            ),
            Self::InstanceI32(function) => typed_native(
                MethodKind::Instance,
                "()I",
                function as *const () as *mut c_void,
                "InstanceI32",
            ),
            Self::InstanceI64(function) => typed_native(
                MethodKind::Instance,
                "()J",
                function as *const () as *mut c_void,
                "InstanceI64",
            ),
            Self::InstanceF32(function) => typed_native(
                MethodKind::Instance,
                "()F",
                function as *const () as *mut c_void,
                "InstanceF32",
            ),
            Self::InstanceF64(function) => typed_native(
                MethodKind::Instance,
                "()D",
                function as *const () as *mut c_void,
                "InstanceF64",
            ),
            Self::InstanceString(function) => typed_native(
                MethodKind::Instance,
                "()Ljava/lang/String;",
                function as *const () as *mut c_void,
                "InstanceString",
            ),
            Self::InstanceStringToString(function) => typed_native(
                MethodKind::Instance,
                "(Ljava/lang/String;)Ljava/lang/String;",
                function as *const () as *mut c_void,
                "InstanceStringToString",
            ),
            Self::InstanceReferenceToReference(function) => Ok(typed_reference_native(
                MethodKind::Instance,
                function as *const () as *mut c_void,
                "InstanceReferenceToReference",
            )),
            Self::InstanceReferenceToVoid(function) => typed_native(
                MethodKind::Instance,
                "(Ljava/lang/Object;)V",
                function as *const () as *mut c_void,
                "InstanceReferenceToVoid",
            ),
            Self::InstanceI32I32ToI32(function) => typed_native(
                MethodKind::Instance,
                "(II)I",
                function as *const () as *mut c_void,
                "InstanceI32I32ToI32",
            ),
            Self::InstanceZBCSToI32(function) => typed_native(
                MethodKind::Instance,
                "(ZBCS)I",
                function as *const () as *mut c_void,
                "InstanceZBCSToI32",
            ),
            Self::InstanceI64F64ToI64(function) => typed_native(
                MethodKind::Instance,
                "(JD)J",
                function as *const () as *mut c_void,
                "InstanceI64F64ToI64",
            ),
            Self::InstanceF32F64ToF64(function) => typed_native(
                MethodKind::Instance,
                "(FD)D",
                function as *const () as *mut c_void,
                "InstanceF32F64ToF64",
            ),
        }
    }
}

fn typed_native(
    kind: MethodKind,
    signature: &'static str,
    function: *mut c_void,
    implementation_name: &'static str,
) -> Result<NativeMethodImplementation> {
    NativeMethodImplementation::typed(kind, signature, function, implementation_name)
}

fn typed_reference_native(
    kind: MethodKind,
    function: *mut c_void,
    implementation_name: &'static str,
) -> NativeMethodImplementation {
    NativeMethodImplementation::typed_reference_to_reference(kind, function, implementation_name)
}

macro_rules! static_replacement {
    (
        $(#[$meta:meta])*
        $function:ident,
        $replacement_type:ty,
        $signature:literal,
        $guard_type:ty
    ) => {
        $(#[$meta])*
        #[doc(hidden)]
        pub unsafe fn $function(
            class: &JavaClass,
            name: &str,
            replacement: $replacement_type,
        ) -> Result<$guard_type> {
            unsafe {
                replace_static_native_method(
                    class,
                    name,
                    $signature,
                    replacement as *const () as *mut c_void,
                )
            }
        }
    };
}

macro_rules! instance_replacement {
    (
        $(#[$meta:meta])*
        $function:ident,
        $replacement_type:ty,
        $signature:literal,
        $guard_type:ty
    ) => {
        $(#[$meta])*
        #[doc(hidden)]
        pub unsafe fn $function(
            class: &JavaClass,
            name: &str,
            replacement: $replacement_type,
        ) -> Result<$guard_type> {
            unsafe {
                replace_instance_native_method(
                    class,
                    name,
                    $signature,
                    replacement as *const () as *mut c_void,
                )
            }
        }
    };
}

pub struct MethodReplacement {
    inner: Option<ArtMethodReplacementGuard>,
}

pub type ReplacementClosure =
    dyn for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static;

pub struct ClosureMethodReplacement {
    replacement: Option<MethodReplacement>,
    _thunk: ClosureReplacementThunk,
    state: Box<ClosureReplacementState>,
}

pub struct ReplacementInvocation<'state> {
    state: &'state ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
    arguments: Vec<JavaValue>,
}

struct ClosureReplacementState {
    vm: Vm,
    kind: MethodKind,
    name: String,
    signature: MethodSignature,
    original: OriginalMethod,
    callback: Box<ReplacementClosure>,
    last_error: Mutex<Option<String>>,
}

struct ClosureReplacementThunk {
    pointer: NonNull<c_void>,
    length: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosureReplacementAbi {
    NoArgsVoid,
    NoArgsBoolean,
    NoArgsByte,
    NoArgsChar,
    NoArgsShort,
    NoArgsInt,
    NoArgsLong,
    NoArgsFloat,
    NoArgsDouble,
    NoArgsObject,
    OneReferenceToReference,
    OneReferenceToVoid,
    I32I32ToI32,
}

impl MethodReplacement {
    pub fn revert(mut self) -> Result<()> {
        if let Some(mut inner) = self.inner.take() {
            inner.revert()?;
        }
        Ok(())
    }

    pub fn debug_summary(&self) -> Option<String> {
        self.inner.as_ref().map(|inner| inner.debug_summary())
    }
}

impl ClosureMethodReplacement {
    pub fn revert(mut self) -> Result<()> {
        if let Some(replacement) = self.replacement.take() {
            replacement.revert()?;
        }
        Ok(())
    }

    pub fn debug_summary(&self) -> Option<String> {
        self.replacement
            .as_ref()
            .and_then(MethodReplacement::debug_summary)
    }

    pub fn last_error(&self) -> Option<String> {
        self.state.last_error()
    }

    pub fn take_last_error(&self) -> Option<String> {
        self.state.take_last_error()
    }
}

impl Drop for ClosureMethodReplacement {
    fn drop(&mut self) {
        if let Some(replacement) = self.replacement.take() {
            let _ = replacement.revert();
        }
    }
}

impl<'state> ReplacementInvocation<'state> {
    pub fn env_raw(&self) -> *mut jni::JNIEnv {
        self.env
    }

    pub fn env(&self) -> Result<Env<'state>> {
        let env = NonNull::new(self.env).ok_or(Error::NullReturn {
            operation: "closure replacement JNIEnv",
        })?;
        Ok(Env::from_raw(env, &self.state.vm))
    }

    pub fn kind(&self) -> MethodKind {
        self.state.kind
    }

    pub fn name(&self) -> &str {
        &self.state.name
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.state.signature
    }

    pub fn class(&self) -> Option<jni::jclass> {
        (self.state.kind == MethodKind::Static).then_some(self.target.cast())
    }

    pub fn receiver(&self) -> Option<jni::jobject> {
        (self.state.kind == MethodKind::Instance).then_some(self.target)
    }

    pub fn arguments(&self) -> &[JavaValue] {
        &self.arguments
    }

    pub fn original(&self) -> &OriginalMethod {
        &self.state.original
    }

    /// Calls the replaced method's original implementation from this closure.
    ///
    /// # Safety
    ///
    /// The raw JNI target received by this invocation must still be valid, and this must only be
    /// called while the current thread is inside this replacement callback.
    pub unsafe fn call_original<A: IntoJavaArgs>(&self, args: A) -> Result<RawJavaReturn> {
        match self.state.kind {
            MethodKind::Static => unsafe {
                self.state
                    .original
                    .call_static(self.env, self.target.cast(), args)
            },
            MethodKind::Instance => unsafe {
                self.state
                    .original
                    .call_instance(self.env, self.target, args)
            },
            MethodKind::Constructor => Err(Error::WrongMethodKind {
                operation: "ReplacementInvocation::call_original",
            }),
        }
    }
}

impl ClosureReplacementState {
    fn new<F>(overload: &JavaMethodOverload, callback: F) -> Result<Self>
    where
        F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
    {
        Ok(Self {
            vm: overload.class().vm().clone(),
            kind: overload.kind(),
            name: overload.name().to_owned(),
            signature: overload.signature().clone(),
            original: OriginalMethod::new(overload)?,
            callback: Box::new(callback),
            last_error: Mutex::new(None),
        })
    }

    fn invoke(
        &self,
        env: *mut jni::JNIEnv,
        target: jni::jobject,
        arguments: Vec<JavaValue>,
    ) -> RawJavaReturn {
        let invocation = ReplacementInvocation {
            state: self,
            env,
            target,
            arguments,
        };
        let result = catch_unwind(AssertUnwindSafe(|| (self.callback)(invocation)));
        match result {
            Ok(Ok(value)) => match self.validate_return(value) {
                Ok(value) => value,
                Err(error) => {
                    self.record_error(error.to_string());
                    self.default_return()
                }
            },
            Ok(Err(error)) => {
                self.record_error(error.to_string());
                self.default_return()
            }
            Err(_) => {
                self.record_error("closure replacement callback panicked".to_owned());
                self.default_return()
            }
        }
    }

    fn validate_return(&self, value: RawJavaReturn) -> Result<RawJavaReturn> {
        let valid = matches!(
            (self.signature.return_type(), value),
            (JavaType::Void, RawJavaReturn::Void)
                | (JavaType::Boolean, RawJavaReturn::Boolean(_))
                | (JavaType::Byte, RawJavaReturn::Byte(_))
                | (JavaType::Char, RawJavaReturn::Char(_))
                | (JavaType::Short, RawJavaReturn::Short(_))
                | (JavaType::Int, RawJavaReturn::Int(_))
                | (JavaType::Long, RawJavaReturn::Long(_))
                | (JavaType::Float, RawJavaReturn::Float(_))
                | (JavaType::Double, RawJavaReturn::Double(_))
                | (JavaType::Object(_), RawJavaReturn::Object(_))
                | (JavaType::Array(_), RawJavaReturn::Object(_))
        );
        if valid {
            Ok(value)
        } else {
            Err(invalid_raw_return(
                "closure replacement return",
                self.signature.return_type().jni_return_name(),
                value,
            ))
        }
    }

    fn default_return(&self) -> RawJavaReturn {
        match self.signature.return_type() {
            JavaType::Void => RawJavaReturn::Void,
            JavaType::Boolean => RawJavaReturn::Boolean(jni::JNI_FALSE),
            JavaType::Byte => RawJavaReturn::Byte(0),
            JavaType::Char => RawJavaReturn::Char(0),
            JavaType::Short => RawJavaReturn::Short(0),
            JavaType::Int => RawJavaReturn::Int(0),
            JavaType::Long => RawJavaReturn::Long(0),
            JavaType::Float => RawJavaReturn::Float(0.0),
            JavaType::Double => RawJavaReturn::Double(0.0),
            JavaType::Object(_) | JavaType::Array(_) => RawJavaReturn::Object(ptr::null_mut()),
        }
    }

    fn record_error(&self, error: String) {
        *self
            .last_error
            .lock()
            .expect("closure replacement error mutex poisoned") = Some(error);
    }

    fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .expect("closure replacement error mutex poisoned")
            .clone()
    }

    fn take_last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .expect("closure replacement error mutex poisoned")
            .take()
    }
}

impl ClosureReplacementThunk {
    fn new(abi: ClosureReplacementAbi, state: *mut ClosureReplacementState) -> Result<Self> {
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;
        const LENGTH: usize = 4096;

        if !cfg!(target_arch = "aarch64") {
            return Err(Error::UnsupportedFeature {
                feature: FEATURE_CLOSURE_REPLACEMENT,
                reason: "closure replacement trampolines are currently arm64-only".to_owned(),
            });
        }
        if state.is_null() {
            return Err(Error::NullReturn {
                operation: "closure replacement state",
            });
        }

        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                LENGTH,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return Err(Error::UnsupportedFeature {
                feature: FEATURE_CLOSURE_REPLACEMENT,
                reason: "unable to allocate closure replacement trampoline".to_owned(),
            });
        }

        if let Err(error) = write_closure_trampoline(pointer, state, abi) {
            unsafe { munmap(pointer, LENGTH) };
            return Err(error);
        }
        unsafe {
            frida_gum_sys::gum_clear_cache(pointer, LENGTH as u64);
            if mprotect(pointer, LENGTH, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, LENGTH);
                return Err(Error::UnsupportedFeature {
                    feature: FEATURE_CLOSURE_REPLACEMENT,
                    reason: "unable to protect closure replacement trampoline".to_owned(),
                });
            }
        }

        let Some(pointer) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, LENGTH) };
            return Err(Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            pointer,
            length: LENGTH,
        })
    }

    fn as_ptr(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }
}

impl Drop for ClosureReplacementThunk {
    fn drop(&mut self) {
        unsafe {
            munmap(self.pointer.as_ptr(), self.length);
        }
    }
}

impl Drop for MethodReplacement {
    fn drop(&mut self) {
        if let Some(inner) = &mut self.inner {
            let _ = inner.revert();
        }
    }
}

#[doc(hidden)]
pub type StaticMethodReplacement = MethodReplacement;
#[doc(hidden)]
pub type StaticNoArgReplacement = MethodReplacement;
#[doc(hidden)]
pub type StaticI32Replacement = MethodReplacement;
#[doc(hidden)]
pub type InstanceMethodReplacement = MethodReplacement;
#[doc(hidden)]
pub type InstanceI32Replacement = MethodReplacement;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RawJavaReturn {
    Void,
    Boolean(jni::jboolean),
    Byte(jni::jbyte),
    Char(jni::jchar),
    Short(jni::jshort),
    Int(jni::jint),
    Long(jni::jlong),
    Float(jni::jfloat),
    Double(jni::jdouble),
    Object(jni::jobject),
}

/// Captures the metadata needed to call a replaced method's original implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalMethod {
    kind: MethodKind,
    name: String,
    signature: String,
}

impl OriginalMethod {
    pub fn new(overload: &JavaMethodOverload) -> Result<Self> {
        Self::from_parts(
            overload.kind(),
            overload.name(),
            &overload.signature().to_string(),
        )
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn signature(&self) -> &str {
        &self.signature
    }

    /// Calls this static method's original implementation from a replacement callback.
    ///
    /// # Safety
    ///
    /// `env` and `class` must be the valid JNI environment and declaring class received by the
    /// active replacement callback, and this must only be called while the current thread is inside
    /// a replacement for this method.
    pub unsafe fn call_static<A: IntoJavaArgs>(
        &self,
        env: *mut jni::JNIEnv,
        class: jni::jclass,
        args: A,
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Static {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_static",
            });
        }
        unsafe { call_original_static_method(env, class, &self.name, &self.signature, args) }
    }

    /// Calls this instance method's original implementation from a replacement callback.
    ///
    /// # Safety
    ///
    /// `env` and `receiver` must be the valid JNI environment and receiver received by the active
    /// replacement callback, and this must only be called while the current thread is inside a
    /// replacement for this method.
    pub unsafe fn call_instance<A: IntoJavaArgs>(
        &self,
        env: *mut jni::JNIEnv,
        receiver: jni::jobject,
        args: A,
    ) -> Result<RawJavaReturn> {
        if self.kind != MethodKind::Instance {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::call_instance",
            });
        }
        unsafe { call_original_instance_method(env, receiver, &self.name, &self.signature, args) }
    }

    fn from_parts(kind: MethodKind, name: &str, signature: &str) -> Result<Self> {
        if kind == MethodKind::Constructor {
            return Err(Error::WrongMethodKind {
                operation: "OriginalMethod::new",
            });
        }
        Ok(Self {
            kind,
            name: name.to_owned(),
            signature: MethodSignature::parse(signature)?.to_string(),
        })
    }
}

impl RawJavaReturn {
    pub fn into_void(self, operation: &'static str) -> Result<()> {
        match self {
            Self::Void => Ok(()),
            other => Err(invalid_raw_return(operation, "void", other)),
        }
    }

    pub fn into_boolean(self, operation: &'static str) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(value == jni::JNI_TRUE),
            other => Err(invalid_raw_return(operation, "boolean", other)),
        }
    }

    pub fn into_byte(self, operation: &'static str) -> Result<jni::jbyte> {
        match self {
            Self::Byte(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "byte", other)),
        }
    }

    pub fn into_char(self, operation: &'static str) -> Result<jni::jchar> {
        match self {
            Self::Char(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "char", other)),
        }
    }

    pub fn into_short(self, operation: &'static str) -> Result<jni::jshort> {
        match self {
            Self::Short(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "short", other)),
        }
    }

    pub fn into_int(self, operation: &'static str) -> Result<jni::jint> {
        match self {
            Self::Int(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "int", other)),
        }
    }

    pub fn into_long(self, operation: &'static str) -> Result<jni::jlong> {
        match self {
            Self::Long(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "long", other)),
        }
    }

    pub fn into_float(self, operation: &'static str) -> Result<jni::jfloat> {
        match self {
            Self::Float(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "float", other)),
        }
    }

    pub fn into_double(self, operation: &'static str) -> Result<jni::jdouble> {
        match self {
            Self::Double(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "double", other)),
        }
    }

    pub fn into_object(self, operation: &'static str) -> Result<jni::jobject> {
        match self {
            Self::Object(value) => Ok(value),
            other => Err(invalid_raw_return(operation, "object", other)),
        }
    }
}

/// Replaces a selected overload using the current experimental ART backend.
///
/// This is an overload-first facade over the lower-level signature-specific helpers. It keeps the
/// replacement callback ABI explicit while letting callers reuse `JavaClassWrapper` overload
/// selection.
///
/// # Safety
///
/// The selected `implementation` function must be a valid JNI native function for `overload` and
/// must remain valid until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_method(
    overload: &JavaMethodOverload,
    implementation: MethodImplementation,
) -> Result<MethodReplacement> {
    unsafe { replace_native_method(overload, implementation.into_native()?) }
}

/// Replaces a selected overload using a descriptor-driven raw JNI-native implementation.
///
/// # Safety
///
/// The selected `implementation` function must be a valid JNI native function for `overload` and
/// must remain valid until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_native_method(
    overload: &JavaMethodOverload,
    implementation: NativeMethodImplementation,
) -> Result<MethodReplacement> {
    if overload.kind() == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "experimental::replace_native_method",
        });
    }

    let signature = overload.signature().to_string();
    let replacement = native_replacement_pointer_for(overload.kind(), &signature, implementation)?;
    match overload.kind() {
        MethodKind::Static => unsafe {
            replace_static_native_method(overload.class(), overload.name(), &signature, replacement)
        },
        MethodKind::Instance => unsafe {
            replace_instance_native_method(
                overload.class(),
                overload.name(),
                &signature,
                replacement,
            )
        },
        MethodKind::Constructor => Err(Error::WrongMethodKind {
            operation: "experimental::replace_native_method",
        }),
    }
}

/// Replaces a selected overload with a Rust closure using the current experimental ART backend.
///
/// The closure receives a raw invocation object and must return a `RawJavaReturn` matching the
/// overload return type. If the closure returns an error, panics, or returns the wrong value kind,
/// the error is recorded on the returned guard and the JNI default value is returned to Java.
///
/// # Safety
///
/// The closure must only return raw JNI object references that are valid in the callback's JNI
/// environment. This API is backed by the same hidden ART method-replacement prototype as
/// `replace_native_method`.
#[doc(hidden)]
pub unsafe fn replace_closure_method<F>(
    overload: &JavaMethodOverload,
    callback: F,
) -> Result<ClosureMethodReplacement>
where
    F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
{
    if overload.kind() == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "experimental::replace_closure_method",
        });
    }

    let abi = closure_replacement_abi(overload.kind(), overload.signature())?;
    let mut state = Box::new(ClosureReplacementState::new(overload, callback)?);
    let thunk = ClosureReplacementThunk::new(abi, state.as_mut() as *mut _)?;
    let signature = overload.signature().to_string();
    let replacement = match overload.kind() {
        MethodKind::Static => unsafe {
            replace_static_native_method(
                overload.class(),
                overload.name(),
                &signature,
                thunk.as_ptr(),
            )?
        },
        MethodKind::Instance => unsafe {
            replace_instance_native_method(
                overload.class(),
                overload.name(),
                &signature,
                thunk.as_ptr(),
            )?
        },
        MethodKind::Constructor => {
            return Err(Error::WrongMethodKind {
                operation: "experimental::replace_closure_method",
            });
        }
    };

    Ok(ClosureMethodReplacement {
        replacement: Some(replacement),
        _thunk: thunk,
        state,
    })
}

#[doc(hidden)]
pub unsafe fn call_original_static_i32_method(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
) -> Result<jni::jint> {
    match unsafe { call_original_static_method(env, class, name, "()I", [])? } {
        RawJavaReturn::Int(value) => Ok(value),
        other => Err(invalid_raw_return(
            "call_original_static_i32_method",
            "int",
            other,
        )),
    }
}

#[doc(hidden)]
pub unsafe fn call_original_instance_i32_method(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
) -> Result<jni::jint> {
    match unsafe { call_original_instance_method(env, receiver, name, "()I", [])? } {
        RawJavaReturn::Int(value) => Ok(value),
        other => Err(invalid_raw_return(
            "call_original_instance_i32_method",
            "int",
            other,
        )),
    }
}

#[doc(hidden)]
pub unsafe fn call_original_static_method<A: IntoJavaArgs>(
    env: *mut jni::JNIEnv,
    class: jni::jclass,
    name: &str,
    signature: &str,
    args: A,
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement class",
        });
    }

    let (parsed, args) = prepare_original_call_args(signature, args)?;
    let name = CString::new(name)?;
    let signature = CString::new(signature)?;
    let get_static_method =
        unsafe { jni::env_function::<jni::GetStaticMethodId>(env, jni::ENV_GET_STATIC_METHOD_ID) };
    let method =
        unsafe { get_static_method(env.as_ptr(), class, name.as_ptr(), signature.as_ptr()) };
    unsafe { check_pending_exception(env, "JNIEnv::GetStaticMethodID")? };
    if method.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetStaticMethodID",
        });
    }

    unsafe { call_original_static_by_return(env, class, method, parsed.return_type(), &args) }
}

#[doc(hidden)]
pub unsafe fn call_original_instance_method<A: IntoJavaArgs>(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
    signature: &str,
    args: A,
) -> Result<RawJavaReturn> {
    let env = non_null_env(env)?;
    if receiver.is_null() {
        return Err(Error::NullReturn {
            operation: "replacement receiver",
        });
    }

    let get_object_class =
        unsafe { jni::env_function::<jni::GetObjectClass>(env, jni::ENV_GET_OBJECT_CLASS) };
    let class = unsafe { get_object_class(env.as_ptr(), receiver) };
    unsafe { check_pending_exception(env, "JNIEnv::GetObjectClass")? };
    if class.is_null() {
        return Err(Error::NullReturn {
            operation: "JNIEnv::GetObjectClass",
        });
    }

    let result = unsafe {
        let (parsed, args) = prepare_original_call_args(signature, args)?;
        let name = CString::new(name)?;
        let signature = CString::new(signature)?;
        let get_method = jni::env_function::<jni::GetMethodId>(env, jni::ENV_GET_METHOD_ID);
        let method = get_method(env.as_ptr(), class, name.as_ptr(), signature.as_ptr());
        check_pending_exception(env, "JNIEnv::GetMethodID")?;
        if method.is_null() {
            return Err(Error::NullReturn {
                operation: "JNIEnv::GetMethodID",
            });
        }

        call_original_instance_by_return(env, receiver, method, parsed.return_type(), &args)
    };

    let delete_local_ref =
        unsafe { jni::env_function::<jni::DeleteLocalRef>(env, jni::ENV_DELETE_LOCAL_REF) };
    unsafe { delete_local_ref(env.as_ptr(), class) };
    result
}

static_replacement!(
    /// Replaces a static Java method with signature `()V` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_void_method,
    StaticVoidReplacementFn,
    "()V",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_static_string_method,
    StaticStringReplacementFn,
    "()Ljava/lang/String;",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()Z` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_boolean_method,
    StaticBooleanReplacementFn,
    "()Z",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()B` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_byte_method,
    StaticByteReplacementFn,
    "()B",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()C` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_char_method,
    StaticCharReplacementFn,
    "()C",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()S` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_short_method,
    StaticShortReplacementFn,
    "()S",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i32_method,
    StaticI32ReplacementFn,
    "()I",
    StaticI32Replacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i64_method,
    StaticI64ReplacementFn,
    "()J",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()F` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_f32_method,
    StaticF32ReplacementFn,
    "()F",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `()D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_f64_method,
    StaticF64ReplacementFn,
    "()D",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(Ljava/lang/String;)Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_static_string_to_string_method,
    StaticStringToStringReplacementFn,
    "(Ljava/lang/String;)Ljava/lang/String;",
    StaticMethodReplacement
);

/// Replaces a static Java method with one reference argument and a reference return value.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped. Any returned object must be valid in the
/// calling JNI environment, for example a local reference created in the callback or a global
/// reference retained for the callback lifetime.
#[doc(hidden)]
pub unsafe fn replace_static_reference_to_reference_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: StaticReferenceToReferenceReplacementFn,
) -> Result<StaticMethodReplacement> {
    unsafe {
        replace_static_native_method(
            class,
            name,
            signature,
            replacement as *const () as *mut c_void,
        )
    }
}

/// Replaces a static Java method with a raw JNI-native callback for a supported signature.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for `signature` and must remain valid until
/// the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_static_native_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<StaticMethodReplacement> {
    let signature = supported_replacement_signature(
        MethodKind::Static,
        signature,
        "replace_static_native_method",
    )?;
    replace_static_method(class, name, &signature, replacement)
}

static_replacement!(
    /// Replaces a static Java method with signature `(II)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i32_i32_to_i32_method,
    StaticI32I32ToI32ReplacementFn,
    "(II)I",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(ZBCS)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_z_b_c_s_to_i32_method,
    StaticZBCSToI32ReplacementFn,
    "(ZBCS)I",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(JD)J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_i64_f64_to_i64_method,
    StaticI64F64ToI64ReplacementFn,
    "(JD)J",
    StaticMethodReplacement
);

static_replacement!(
    /// Replaces a static Java method with signature `(FD)D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_static_f32_f64_to_f64_method,
    StaticF32F64ToF64ReplacementFn,
    "(FD)D",
    StaticMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()V` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_void_method,
    InstanceVoidReplacementFn,
    "()V",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()Z` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_boolean_method,
    InstanceBooleanReplacementFn,
    "()Z",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()B` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_byte_method,
    InstanceByteReplacementFn,
    "()B",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()C` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_char_method,
    InstanceCharReplacementFn,
    "()C",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()S` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_short_method,
    InstanceShortReplacementFn,
    "()S",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i32_method,
    InstanceI32ReplacementFn,
    "()I",
    InstanceI32Replacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i64_method,
    InstanceI64ReplacementFn,
    "()J",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()F` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_f32_method,
    InstanceF32ReplacementFn,
    "()F",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_f64_method,
    InstanceF64ReplacementFn,
    "()D",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `()Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_instance_string_method,
    InstanceStringReplacementFn,
    "()Ljava/lang/String;",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(II)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i32_i32_to_i32_method,
    InstanceI32I32ToI32ReplacementFn,
    "(II)I",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(ZBCS)I` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_z_b_c_s_to_i32_method,
    InstanceZBCSToI32ReplacementFn,
    "(ZBCS)I",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(JD)J` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_i64_f64_to_i64_method,
    InstanceI64F64ToI64ReplacementFn,
    "(JD)J",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(FD)D` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped.
    replace_instance_f32_f64_to_f64_method,
    InstanceF32F64ToF64ReplacementFn,
    "(FD)D",
    InstanceMethodReplacement
);

instance_replacement!(
    /// Replaces an instance Java method with signature `(Ljava/lang/String;)Ljava/lang/String;` using the current experimental ART backend.
    ///
    /// # Safety
    ///
    /// `replacement` must be a valid JNI native function for the target method and must remain valid
    /// until the returned guard is reverted or dropped. Any returned object must be valid in the
    /// calling JNI environment, for example a local reference created in the callback or a global
    /// reference retained for the callback lifetime.
    replace_instance_string_to_string_method,
    InstanceStringToStringReplacementFn,
    "(Ljava/lang/String;)Ljava/lang/String;",
    InstanceMethodReplacement
);

/// Replaces an instance Java method with one reference argument and a reference return value.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped. Any returned object must be valid in the
/// calling JNI environment, for example a local reference created in the callback or a global
/// reference retained for the callback lifetime.
#[doc(hidden)]
pub unsafe fn replace_instance_reference_to_reference_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: InstanceReferenceToReferenceReplacementFn,
) -> Result<InstanceMethodReplacement> {
    unsafe {
        replace_instance_native_method(
            class,
            name,
            signature,
            replacement as *const () as *mut c_void,
        )
    }
}

/// Replaces an instance Java method with one reference argument and a `void` return value.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for the target method and must remain valid
/// until the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_instance_reference_to_void_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: InstanceReferenceToVoidReplacementFn,
) -> Result<InstanceMethodReplacement> {
    unsafe {
        replace_instance_native_method(
            class,
            name,
            signature,
            replacement as *const () as *mut c_void,
        )
    }
}

/// Replaces an instance Java method with a raw JNI-native callback for a supported signature.
///
/// # Safety
///
/// `replacement` must be a valid JNI native function for `signature` and must remain valid until
/// the returned guard is reverted or dropped.
#[doc(hidden)]
pub unsafe fn replace_instance_native_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<InstanceMethodReplacement> {
    let signature = supported_replacement_signature(
        MethodKind::Instance,
        signature,
        "replace_instance_native_method",
    )?;
    replace_instance_method(class, name, &signature, replacement)
}

fn replace_static_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<StaticMethodReplacement> {
    let method = class.resolve_static_method(name, signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}

fn replace_instance_method(
    class: &JavaClass,
    name: &str,
    signature: &str,
    replacement: *mut c_void,
) -> Result<InstanceMethodReplacement> {
    let method = class.resolve_instance_method(name, signature)?;
    let inner = class.vm().replace_method(&method, replacement)?;
    Ok(MethodReplacement { inner: Some(inner) })
}

#[cfg(test)]
fn replacement_pointer_for(
    kind: MethodKind,
    signature: &str,
    implementation: MethodImplementation,
) -> Result<*mut c_void> {
    native_replacement_pointer_for(kind, signature, implementation.into_native()?)
}

fn native_replacement_pointer_for(
    actual_kind: MethodKind,
    actual_signature: &str,
    implementation: NativeMethodImplementation,
) -> Result<*mut c_void> {
    let actual_signature = MethodSignature::parse(actual_signature)?.to_string();
    if actual_kind != implementation.kind {
        return Err(replacement_mismatch(
            implementation.kind,
            implementation.signature(),
            implementation.implementation_name,
        ));
    }

    match &implementation.signature {
        NativeImplementationSignature::Exact(expected_signature)
            if actual_signature == expected_signature.as_str() =>
        {
            supported_replacement_signature(
                actual_kind,
                &actual_signature,
                "experimental::replace_method",
            )?;
            Ok(implementation.function)
        }
        NativeImplementationSignature::Exact(expected_signature) => Err(replacement_mismatch(
            implementation.kind,
            expected_signature,
            implementation.implementation_name,
        )),
        NativeImplementationSignature::OneReferenceToReference => {
            validate_reference_to_reference_signature(
                &actual_signature,
                "experimental::replace_method",
            )?;
            Ok(implementation.function)
        }
    }
}

fn supported_replacement_signature(
    kind: MethodKind,
    signature: &str,
    operation: &'static str,
) -> Result<String> {
    let parsed = MethodSignature::parse(signature)?;
    if replacement_abi_is_supported(&parsed) {
        Ok(parsed.to_string())
    } else {
        Err(Error::InvalidReplacementImplementation {
            operation,
            expected: format!(
                "supported {} method replacement ABI",
                replacement_kind_name(kind)
            ),
            actual: "NativeMethodImplementation",
        })
    }
}

fn replacement_abi_is_supported(signature: &MethodSignature) -> bool {
    let args = signature.arguments();
    let return_type = signature.return_type();

    if startup_hook_abi_is_supported(signature) {
        return true;
    }

    if args.is_empty() {
        return matches!(
            return_type,
            JavaType::Void
                | JavaType::Boolean
                | JavaType::Byte
                | JavaType::Char
                | JavaType::Short
                | JavaType::Int
                | JavaType::Long
                | JavaType::Float
                | JavaType::Double
        ) || is_java_lang_string(return_type);
    }

    if args.len() == 1 && args[0].is_reference() && return_type.is_reference() {
        return true;
    }

    if args.len() == 1 && args[0].is_reference() && return_type == &JavaType::Void {
        return true;
    }

    matches!(
        signature.to_string().as_str(),
        "(II)I" | "(ZBCS)I" | "(JD)J" | "(FD)D"
    )
}

fn closure_replacement_abi(
    kind: MethodKind,
    signature: &MethodSignature,
) -> Result<ClosureReplacementAbi> {
    if kind == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "experimental::replace_closure_method",
        });
    }

    let args = signature.arguments();
    let return_type = signature.return_type();
    let abi = if args.is_empty() {
        match return_type {
            JavaType::Void => ClosureReplacementAbi::NoArgsVoid,
            JavaType::Boolean => ClosureReplacementAbi::NoArgsBoolean,
            JavaType::Byte => ClosureReplacementAbi::NoArgsByte,
            JavaType::Char => ClosureReplacementAbi::NoArgsChar,
            JavaType::Short => ClosureReplacementAbi::NoArgsShort,
            JavaType::Int => ClosureReplacementAbi::NoArgsInt,
            JavaType::Long => ClosureReplacementAbi::NoArgsLong,
            JavaType::Float => ClosureReplacementAbi::NoArgsFloat,
            JavaType::Double => ClosureReplacementAbi::NoArgsDouble,
            JavaType::Object(_) | JavaType::Array(_) => ClosureReplacementAbi::NoArgsObject,
        }
    } else if args.len() == 1 && args[0].is_reference() && return_type.is_reference() {
        ClosureReplacementAbi::OneReferenceToReference
    } else if args.len() == 1 && args[0].is_reference() && return_type == &JavaType::Void {
        ClosureReplacementAbi::OneReferenceToVoid
    } else if signature.to_string() == "(II)I" {
        ClosureReplacementAbi::I32I32ToI32
    } else {
        return Err(Error::InvalidReplacementImplementation {
            operation: "experimental::replace_closure_method",
            expected: format!(
                "supported {} closure replacement ABI",
                replacement_kind_name(kind)
            ),
            actual: "closure",
        });
    };
    Ok(abi)
}

fn startup_hook_abi_is_supported(signature: &MethodSignature) -> bool {
    matches!(
        signature.to_string().as_str(),
        "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZZ)Landroid/app/LoadedApk;"
            | "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZ)Landroid/app/LoadedApk;"
            | "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;"
            | "(Ljava/lang/String;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;"
            | "(ZLandroid/app/Instrumentation;)Landroid/app/Application;"
            | "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZZ)Ljava/lang/Object;"
            | "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;"
            | "(Ljava/lang/Object;Ljava/lang/Object;I)Ljava/lang/Object;"
            | "(Ljava/lang/String;Ljava/lang/Object;I)Ljava/lang/Object;"
            | "(ZLjava/lang/Object;)Ljava/lang/Object;"
    )
}

fn is_java_lang_string(ty: &JavaType) -> bool {
    matches!(ty, JavaType::Object(name) if name == "java/lang/String")
}

fn replacement_mismatch(
    expected_kind: MethodKind,
    expected_signature: &str,
    actual_implementation: &'static str,
) -> Error {
    Error::InvalidReplacementImplementation {
        operation: "experimental::replace_method",
        expected: format!(
            "{} method {expected_signature}",
            replacement_kind_name(expected_kind)
        ),
        actual: actual_implementation,
    }
}

fn replacement_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Constructor => "constructor",
        MethodKind::Instance => "instance",
        MethodKind::Static => "static",
    }
}

fn validate_reference_to_reference_signature(
    signature: &str,
    operation: &'static str,
) -> Result<()> {
    let parsed = MethodSignature::parse(signature)?;
    if parsed.arguments().len() != 1 {
        return Err(Error::InvalidArguments {
            expected: 1,
            actual: parsed.arguments().len(),
        });
    }
    if !parsed.arguments()[0].is_reference() {
        return Err(Error::InvalidArgumentType {
            index: 0,
            expected: "reference".to_owned(),
            actual: parsed.arguments()[0].jni_return_name(),
        });
    }
    if !parsed.return_type().is_reference() {
        return Err(Error::InvalidReturnType {
            operation,
            expected: "reference",
            actual: parsed.return_type().to_string(),
        });
    }
    Ok(())
}

fn non_null_env(env: *mut jni::JNIEnv) -> Result<NonNull<jni::JNIEnv>> {
    NonNull::new(env).ok_or(crate::Error::NullReturn {
        operation: "replacement JNIEnv",
    })
}

fn prepare_original_call_args<A: IntoJavaArgs>(
    signature: &str,
    args: A,
) -> Result<(MethodSignature, Vec<JavaValue>)> {
    let parsed = MethodSignature::parse(signature)?;
    let args = args.into_java_args();
    parsed.validate_arguments(&args)?;
    Ok((parsed, args))
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

unsafe fn art_thread_from_env(env: NonNull<jni::JNIEnv>) -> usize {
    unsafe { env.as_ptr().cast::<*mut c_void>().add(1).read() as usize }
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
    let thread = unsafe { art_thread_from_env(env) };
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
    unsafe { check_pending_exception(env, "JNIEnv::CallStaticMethodA")? };
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
    let thread = unsafe { art_thread_from_env(env) };
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
    unsafe { check_pending_exception(env, "JNIEnv::CallMethodA")? };
    Ok(result)
}

fn invalid_raw_return(
    operation: &'static str,
    expected: &'static str,
    actual: RawJavaReturn,
) -> Error {
    Error::InvalidReturnType {
        operation,
        expected,
        actual: raw_return_type_name(actual).to_owned(),
    }
}

fn raw_return_type_name(value: RawJavaReturn) -> &'static str {
    match value {
        RawJavaReturn::Void => "void",
        RawJavaReturn::Boolean(_) => "boolean",
        RawJavaReturn::Byte(_) => "byte",
        RawJavaReturn::Char(_) => "char",
        RawJavaReturn::Short(_) => "short",
        RawJavaReturn::Int(_) => "int",
        RawJavaReturn::Long(_) => "long",
        RawJavaReturn::Float(_) => "float",
        RawJavaReturn::Double(_) => "double",
        RawJavaReturn::Object(_) => "object",
    }
}

fn write_closure_trampoline(
    code: *mut c_void,
    state: *mut ClosureReplacementState,
    abi: ClosureReplacementAbi,
) -> Result<()> {
    let writer = Aarch64InstructionWriter::new(code as u64);
    match closure_trampoline_extra_arg_count(abi) {
        0 => {
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X2, Aarch64Register::X1),
                "move JNI target",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X1, Aarch64Register::X0),
                "move JNI env",
            )?;
        }
        1 => {
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X3, Aarch64Register::X2),
                "move first JNI argument",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X2, Aarch64Register::X1),
                "move JNI target",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X1, Aarch64Register::X0),
                "move JNI env",
            )?;
        }
        2 => {
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X4, Aarch64Register::X3),
                "move second JNI argument",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X3, Aarch64Register::X2),
                "move first JNI argument",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X2, Aarch64Register::X1),
                "move JNI target",
            )?;
            ensure_closure_writer(
                writer.put_mov_reg_reg(Aarch64Register::X1, Aarch64Register::X0),
                "move JNI env",
            )?;
        }
        _ => unreachable!("closure replacement supports at most two Java arguments"),
    }
    ensure_closure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X0, state as u64),
        "load closure replacement state",
    )?;
    ensure_closure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X16, closure_handler_for_abi(abi) as u64),
        "load closure replacement handler",
    )?;
    ensure_closure_writer(
        writer.put_br_reg(Aarch64Register::X16),
        "branch to closure replacement handler",
    )?;
    ensure_closure_writer(writer.flush(), "flush closure replacement trampoline")
}

fn closure_trampoline_extra_arg_count(abi: ClosureReplacementAbi) -> usize {
    match abi {
        ClosureReplacementAbi::NoArgsVoid
        | ClosureReplacementAbi::NoArgsBoolean
        | ClosureReplacementAbi::NoArgsByte
        | ClosureReplacementAbi::NoArgsChar
        | ClosureReplacementAbi::NoArgsShort
        | ClosureReplacementAbi::NoArgsInt
        | ClosureReplacementAbi::NoArgsLong
        | ClosureReplacementAbi::NoArgsFloat
        | ClosureReplacementAbi::NoArgsDouble
        | ClosureReplacementAbi::NoArgsObject => 0,
        ClosureReplacementAbi::OneReferenceToReference
        | ClosureReplacementAbi::OneReferenceToVoid => 1,
        ClosureReplacementAbi::I32I32ToI32 => 2,
    }
}

fn closure_handler_for_abi(abi: ClosureReplacementAbi) -> *const c_void {
    match abi {
        ClosureReplacementAbi::NoArgsVoid => closure_no_args_void as *const c_void,
        ClosureReplacementAbi::NoArgsBoolean => closure_no_args_boolean as *const c_void,
        ClosureReplacementAbi::NoArgsByte => closure_no_args_byte as *const c_void,
        ClosureReplacementAbi::NoArgsChar => closure_no_args_char as *const c_void,
        ClosureReplacementAbi::NoArgsShort => closure_no_args_short as *const c_void,
        ClosureReplacementAbi::NoArgsInt => closure_no_args_int as *const c_void,
        ClosureReplacementAbi::NoArgsLong => closure_no_args_long as *const c_void,
        ClosureReplacementAbi::NoArgsFloat => closure_no_args_float as *const c_void,
        ClosureReplacementAbi::NoArgsDouble => closure_no_args_double as *const c_void,
        ClosureReplacementAbi::NoArgsObject => closure_no_args_object as *const c_void,
        ClosureReplacementAbi::OneReferenceToReference => {
            closure_one_reference_to_reference as *const c_void
        }
        ClosureReplacementAbi::OneReferenceToVoid => closure_one_reference_to_void as *const c_void,
        ClosureReplacementAbi::I32I32ToI32 => closure_i32_i32_to_i32 as *const c_void,
    }
}

fn ensure_closure_writer(ok: bool, operation: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::UnsupportedFeature {
            feature: FEATURE_CLOSURE_REPLACEMENT,
            reason: format!("{operation} failed while generating closure trampoline"),
        })
    }
}

unsafe fn closure_state<'state>(
    state: *mut ClosureReplacementState,
) -> Option<&'state ClosureReplacementState> {
    unsafe { state.as_ref() }
}

fn reference_argument(value: jni::jobject) -> JavaValue {
    if value.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(value)
    }
}

unsafe extern "C" fn closure_no_args_void(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return;
    };
    let _ = state.invoke(env, target, Vec::new());
}

unsafe extern "C" fn closure_no_args_boolean(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jboolean {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return jni::JNI_FALSE;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Boolean(value) => value,
        other => {
            state.record_error(
                invalid_raw_return("closure boolean return", "boolean", other).to_string(),
            );
            jni::JNI_FALSE
        }
    }
}

unsafe extern "C" fn closure_no_args_byte(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jbyte {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Byte(value) => value,
        other => {
            state
                .record_error(invalid_raw_return("closure byte return", "byte", other).to_string());
            0
        }
    }
}

unsafe extern "C" fn closure_no_args_char(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jchar {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Char(value) => value,
        other => {
            state
                .record_error(invalid_raw_return("closure char return", "char", other).to_string());
            0
        }
    }
}

unsafe extern "C" fn closure_no_args_short(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jshort {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Short(value) => value,
        other => {
            state.record_error(
                invalid_raw_return("closure short return", "short", other).to_string(),
            );
            0
        }
    }
}

unsafe extern "C" fn closure_no_args_int(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jint {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Int(value) => value,
        other => {
            state.record_error(invalid_raw_return("closure int return", "int", other).to_string());
            0
        }
    }
}

unsafe extern "C" fn closure_no_args_long(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jlong {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Long(value) => value,
        other => {
            state
                .record_error(invalid_raw_return("closure long return", "long", other).to_string());
            0
        }
    }
}

unsafe extern "C" fn closure_no_args_float(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jfloat {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0.0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Float(value) => value,
        other => {
            state.record_error(
                invalid_raw_return("closure float return", "float", other).to_string(),
            );
            0.0
        }
    }
}

unsafe extern "C" fn closure_no_args_double(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jdouble {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0.0;
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Double(value) => value,
        other => {
            state.record_error(
                invalid_raw_return("closure double return", "double", other).to_string(),
            );
            0.0
        }
    }
}

unsafe extern "C" fn closure_no_args_object(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) -> jni::jobject {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return ptr::null_mut();
    };
    match state.invoke(env, target, Vec::new()) {
        RawJavaReturn::Object(value) => value,
        other => {
            state.record_error(
                invalid_raw_return("closure object return", "object", other).to_string(),
            );
            ptr::null_mut()
        }
    }
}

unsafe extern "C" fn closure_one_reference_to_reference(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
    argument: jni::jobject,
) -> jni::jobject {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return ptr::null_mut();
    };
    match state.invoke(env, target, vec![reference_argument(argument)]) {
        RawJavaReturn::Object(value) => value,
        other => {
            state.record_error(
                invalid_raw_return("closure reference return", "object", other).to_string(),
            );
            ptr::null_mut()
        }
    }
}

unsafe extern "C" fn closure_one_reference_to_void(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
    argument: jni::jobject,
) {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return;
    };
    let _ = state.invoke(env, target, vec![reference_argument(argument)]);
}

unsafe extern "C" fn closure_i32_i32_to_i32(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
    left: jni::jint,
    right: jni::jint,
) -> jni::jint {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return 0;
    };
    match state.invoke(
        env,
        target,
        vec![JavaValue::Int(left), JavaValue::Int(right)],
    ) {
        RawJavaReturn::Int(value) => value,
        other => {
            state.record_error(invalid_raw_return("closure int return", "int", other).to_string());
            0
        }
    }
}

unsafe fn check_pending_exception(
    env: NonNull<jni::JNIEnv>,
    operation: &'static str,
) -> Result<()> {
    let exception_check =
        unsafe { jni::env_function::<jni::ExceptionCheck>(env, jni::ENV_EXCEPTION_CHECK) };
    if unsafe { exception_check(env.as_ptr()) } == jni::JNI_TRUE {
        let exception_clear =
            unsafe { jni::env_function::<jni::ExceptionClear>(env, jni::ENV_EXCEPTION_CLEAR) };
        unsafe { exception_clear(env.as_ptr()) };
        Err(Error::JavaException { operation })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn static_i32(_env: *mut jni::JNIEnv, _class: jni::jclass) -> jni::jint {
        1
    }

    unsafe extern "C" fn static_object_echo(
        _env: *mut jni::JNIEnv,
        _class: jni::jclass,
        value: jni::jobject,
    ) -> jni::jobject {
        value
    }

    unsafe extern "C" fn instance_i32(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
    ) -> jni::jint {
        1
    }

    unsafe extern "C" fn instance_string_to_string(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        value: jni::jstring,
    ) -> jni::jstring {
        value
    }

    unsafe extern "C" fn instance_object_echo(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        value: jni::jobject,
    ) -> jni::jobject {
        value
    }

    unsafe extern "C" fn instance_object_void(
        _env: *mut jni::JNIEnv,
        _receiver: jni::jobject,
        _value: jni::jobject,
    ) {
    }

    fn dummy_replacement_ptr() -> *mut c_void {
        static_i32 as *const () as *mut c_void
    }

    #[test]
    fn accepts_supported_native_replacement_abi_shapes() {
        for signature in [
            "()V",
            "()Z",
            "()B",
            "()C",
            "()S",
            "()I",
            "()J",
            "()F",
            "()D",
            "()Ljava/lang/String;",
            "(Ljava/lang/String;)Ljava/lang/String;",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            "(Ljava/lang/Object;)V",
            "([Ljava/lang/Object;)[Ljava/lang/Object;",
            "([Ljava/lang/Object;)V",
            "([I)[Ljava/lang/Object;",
            "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZZ)Landroid/app/LoadedApk;",
            "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZ)Landroid/app/LoadedApk;",
            "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;",
            "(Ljava/lang/String;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;",
            "(ZLandroid/app/Instrumentation;)Landroid/app/Application;",
            "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZZ)Ljava/lang/Object;",
            "(Ljava/lang/Object;Ljava/lang/Object;Ljava/lang/Object;ZZZ)Ljava/lang/Object;",
            "(Ljava/lang/Object;Ljava/lang/Object;I)Ljava/lang/Object;",
            "(Ljava/lang/String;Ljava/lang/Object;I)Ljava/lang/Object;",
            "(ZLjava/lang/Object;)Ljava/lang/Object;",
            "(II)I",
            "(ZBCS)I",
            "(JD)J",
            "(FD)D",
        ] {
            unsafe {
                NativeMethodImplementation::static_method(signature, dummy_replacement_ptr())
                    .unwrap_or_else(|_| panic!("static ABI {signature} should be supported"));
                NativeMethodImplementation::instance_method(signature, dummy_replacement_ptr())
                    .unwrap_or_else(|_| panic!("instance ABI {signature} should be supported"));
            }
        }
    }

    #[test]
    fn rejects_unsupported_native_replacement_abi_shapes() {
        assert_eq!(
            unsafe {
                NativeMethodImplementation::static_method(
                    "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                    dummy_replacement_ptr(),
                )
            },
            Err(Error::InvalidReplacementImplementation {
                operation: "NativeMethodImplementation::static_method",
                expected: "supported static method replacement ABI".to_owned(),
                actual: "NativeMethodImplementation",
            })
        );

        assert_eq!(
            unsafe {
                NativeMethodImplementation::instance_method(
                    "()Ljava/lang/Object;",
                    dummy_replacement_ptr(),
                )
            },
            Err(Error::InvalidReplacementImplementation {
                operation: "NativeMethodImplementation::instance_method",
                expected: "supported instance method replacement ABI".to_owned(),
                actual: "NativeMethodImplementation",
            })
        );

        assert!(matches!(
            unsafe { NativeMethodImplementation::static_method("(I", dummy_replacement_ptr()) },
            Err(Error::InvalidSignature { .. })
        ));
    }

    #[test]
    fn accepts_matching_replacement_implementations() {
        replacement_pointer_for(
            MethodKind::Static,
            "()I",
            MethodImplementation::StaticI32(static_i32),
        )
        .expect("static int implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "(Ljava/lang/String;)Ljava/lang/String;",
            MethodImplementation::InstanceStringToString(instance_string_to_string),
        )
        .expect("instance string implementation should match");

        replacement_pointer_for(
            MethodKind::Static,
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            MethodImplementation::StaticReferenceToReference(static_object_echo),
        )
        .expect("static reference implementation should match");

        replacement_pointer_for(
            MethodKind::Static,
            "([Ljava/lang/Object;)[Ljava/lang/Object;",
            MethodImplementation::StaticReferenceToReference(static_object_echo),
        )
        .expect("static object array implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "([I)[Ljava/lang/Object;",
            MethodImplementation::InstanceReferenceToReference(instance_object_echo),
        )
        .expect("instance primitive array implementation should match");

        replacement_pointer_for(
            MethodKind::Instance,
            "(Ljava/lang/Object;)V",
            MethodImplementation::InstanceReferenceToVoid(instance_object_void),
        )
        .expect("instance reference-to-void implementation should match");
    }

    #[test]
    fn rejects_mismatched_replacement_implementations() {
        assert_eq!(
            replacement_pointer_for(
                MethodKind::Instance,
                "()I",
                MethodImplementation::StaticI32(static_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "StaticI32",
            })
        );

        assert_eq!(
            replacement_pointer_for(
                MethodKind::Static,
                "()I",
                MethodImplementation::InstanceI32(instance_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "instance method ()I".to_owned(),
                actual: "InstanceI32",
            })
        );
    }

    #[test]
    fn rejects_unsupported_facade_signatures() {
        assert_eq!(
            replacement_pointer_for(
                MethodKind::Static,
                "(I)I",
                MethodImplementation::StaticI32(static_i32),
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "StaticI32",
            })
        );
    }

    #[test]
    fn rejects_mismatched_native_replacement_implementations() {
        let implementation =
            unsafe { NativeMethodImplementation::static_method("()I", dummy_replacement_ptr()) }
                .expect("static int native implementation should be accepted");
        assert_eq!(
            native_replacement_pointer_for(MethodKind::Instance, "()I", implementation),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_method",
                expected: "static method ()I".to_owned(),
                actual: "NativeMethodImplementation",
            })
        );
    }

    #[test]
    fn original_method_captures_metadata_and_rejects_constructors() {
        let original = OriginalMethod::from_parts(MethodKind::Instance, "answer", "()I")
            .expect("instance original method should be captured");
        assert_eq!(original.kind(), MethodKind::Instance);
        assert_eq!(original.name(), "answer");
        assert_eq!(original.signature(), "()I");

        assert_eq!(
            OriginalMethod::from_parts(MethodKind::Constructor, "<init>", "()V"),
            Err(Error::WrongMethodKind {
                operation: "OriginalMethod::new",
            })
        );
    }

    fn test_closure_state<F>(signature: &str, callback: F) -> ClosureReplacementState
    where
        F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
    {
        test_closure_state_with_kind(MethodKind::Static, "answer", signature, callback)
    }

    fn test_closure_state_with_kind<F>(
        kind: MethodKind,
        name: &str,
        signature: &str,
        callback: F,
    ) -> ClosureReplacementState
    where
        F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
    {
        ClosureReplacementState {
            vm: Vm::dangling_for_tests(),
            kind,
            name: name.to_owned(),
            signature: MethodSignature::parse(signature).expect("test signature should parse"),
            original: OriginalMethod::from_parts(kind, name, signature)
                .expect("test original should be captured"),
            callback: Box::new(callback),
            last_error: Mutex::new(None),
        }
    }

    #[test]
    fn classifies_closure_replacement_signatures() {
        for (signature, abi) in [
            ("()V", ClosureReplacementAbi::NoArgsVoid),
            ("()Z", ClosureReplacementAbi::NoArgsBoolean),
            ("()B", ClosureReplacementAbi::NoArgsByte),
            ("()C", ClosureReplacementAbi::NoArgsChar),
            ("()S", ClosureReplacementAbi::NoArgsShort),
            ("()I", ClosureReplacementAbi::NoArgsInt),
            ("()J", ClosureReplacementAbi::NoArgsLong),
            ("()F", ClosureReplacementAbi::NoArgsFloat),
            ("()D", ClosureReplacementAbi::NoArgsDouble),
            ("()Ljava/lang/Object;", ClosureReplacementAbi::NoArgsObject),
            ("()[Ljava/lang/Object;", ClosureReplacementAbi::NoArgsObject),
        ] {
            assert_eq!(
                closure_replacement_abi(
                    MethodKind::Static,
                    &MethodSignature::parse(signature).unwrap()
                ),
                Ok(abi),
                "{signature}"
            );
        }
        assert_eq!(
            closure_replacement_abi(MethodKind::Static, &MethodSignature::parse("()I").unwrap()),
            Ok(ClosureReplacementAbi::NoArgsInt)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse("(Ljava/lang/String;)Ljava/lang/String;").unwrap()
            ),
            Ok(ClosureReplacementAbi::OneReferenceToReference)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Static,
                &MethodSignature::parse("([Ljava/lang/Object;)[Ljava/lang/Object;").unwrap()
            ),
            Ok(ClosureReplacementAbi::OneReferenceToReference)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse("(Ljava/lang/Object;)V").unwrap()
            ),
            Ok(ClosureReplacementAbi::OneReferenceToVoid)
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Static,
                &MethodSignature::parse("(II)I").unwrap()
            ),
            Ok(ClosureReplacementAbi::I32I32ToI32)
        );
    }

    #[test]
    fn rejects_unsupported_closure_replacement_signatures() {
        assert_eq!(
            closure_replacement_abi(MethodKind::Static, &MethodSignature::parse("(I)I").unwrap()),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_closure_method",
                expected: "supported static closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Static,
                &MethodSignature::parse("(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;")
                    .unwrap()
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_closure_method",
                expected: "supported static closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Instance,
                &MethodSignature::parse("(Ljava/lang/Object;I)Ljava/lang/Object;").unwrap()
            ),
            Err(Error::InvalidReplacementImplementation {
                operation: "experimental::replace_closure_method",
                expected: "supported instance closure replacement ABI".to_owned(),
                actual: "closure",
            })
        );
        assert_eq!(
            closure_replacement_abi(
                MethodKind::Constructor,
                &MethodSignature::parse("()V").unwrap()
            ),
            Err(Error::WrongMethodKind {
                operation: "experimental::replace_closure_method",
            })
        );
    }

    #[test]
    fn closure_state_invokes_callback_and_records_failures() {
        let state = test_closure_state("()I", |_| Ok(RawJavaReturn::Int(77)));
        assert_eq!(
            state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
            RawJavaReturn::Int(77)
        );
        assert_eq!(state.last_error(), None);

        let state = test_closure_state("()I", |_| {
            Err(Error::UnsupportedFeature {
                feature: "test closure",
                reason: "nope".to_owned(),
            })
        });
        assert_eq!(
            state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
            RawJavaReturn::Int(0)
        );
        assert!(
            state
                .last_error()
                .as_ref()
                .is_some_and(|error| error.contains("nope"))
        );
    }

    #[test]
    fn closure_state_defaults_on_wrong_return_type() {
        let state = test_closure_state("()I", |_| Ok(RawJavaReturn::Object(ptr::null_mut())));
        assert_eq!(
            state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
            RawJavaReturn::Int(0)
        );
        assert!(
            state
                .last_error()
                .as_ref()
                .is_some_and(|error| error.contains("requires int return"))
        );
    }

    #[test]
    fn closure_state_defaults_on_panic_and_keeps_error_until_taken() {
        let state = test_closure_state("()I", |_| panic!("intentional closure panic for test"));
        assert_eq!(
            state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
            RawJavaReturn::Int(0)
        );
        assert!(
            state
                .last_error()
                .as_ref()
                .is_some_and(|error| error.contains("panicked"))
        );

        let state = test_closure_state("()I", |_| Ok(RawJavaReturn::Int(77)));
        state.record_error("previous closure failure".to_owned());
        assert_eq!(
            state.invoke(ptr::null_mut(), ptr::null_mut(), Vec::new()),
            RawJavaReturn::Int(77)
        );
        assert_eq!(
            state.last_error().as_deref(),
            Some("previous closure failure")
        );
        assert_eq!(
            state.take_last_error().as_deref(),
            Some("previous closure failure")
        );
        assert_eq!(state.last_error(), None);
    }

    #[test]
    fn closure_state_passes_reference_arguments() {
        let object = ptr::dangling_mut();
        let object_addr = object as usize;
        let state = test_closure_state(
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            move |invocation| {
                let object = object_addr as jni::jobject;
                assert_eq!(invocation.arguments(), &[JavaValue::Object(object)]);
                Ok(RawJavaReturn::Object(object))
            },
        );
        assert_eq!(
            state.invoke(
                ptr::null_mut(),
                ptr::null_mut(),
                vec![JavaValue::Object(object)]
            ),
            RawJavaReturn::Object(object)
        );
    }

    #[test]
    fn closure_invocation_exposes_static_and_instance_metadata() {
        let class = ptr::without_provenance_mut::<jni::_jobject>(0x1230) as jni::jclass;
        let class_addr = class as usize;
        let state = test_closure_state_with_kind(
            MethodKind::Static,
            "staticAdd",
            "(II)I",
            move |invocation| {
                assert_eq!(invocation.kind(), MethodKind::Static);
                assert_eq!(invocation.name(), "staticAdd");
                assert_eq!(invocation.signature().to_string(), "(II)I");
                assert_eq!(invocation.class(), Some(class_addr as jni::jclass));
                assert_eq!(invocation.receiver(), None);
                assert_eq!(invocation.env_raw(), ptr::null_mut());
                assert_eq!(
                    invocation.arguments(),
                    &[JavaValue::Int(2), JavaValue::Int(5)]
                );
                assert!(invocation.env().is_err());
                assert_eq!(invocation.original().name(), "staticAdd");
                Ok(RawJavaReturn::Int(7))
            },
        );
        assert_eq!(
            state.invoke(
                ptr::null_mut(),
                class.cast(),
                vec![JavaValue::Int(2), JavaValue::Int(5)]
            ),
            RawJavaReturn::Int(7)
        );

        let receiver = ptr::without_provenance_mut::<jni::_jobject>(0x4560);
        let receiver_addr = receiver as usize;
        let state = test_closure_state_with_kind(
            MethodKind::Instance,
            "objectEcho",
            "(Ljava/lang/Object;)Ljava/lang/Object;",
            move |invocation| {
                assert_eq!(invocation.kind(), MethodKind::Instance);
                assert_eq!(invocation.name(), "objectEcho");
                assert_eq!(invocation.class(), None);
                assert_eq!(invocation.receiver(), Some(receiver_addr as jni::jobject));
                assert_eq!(invocation.arguments(), &[JavaValue::Null]);
                Ok(RawJavaReturn::Object(ptr::null_mut()))
            },
        );
        assert_eq!(
            state.invoke(ptr::null_mut(), receiver, vec![JavaValue::Null]),
            RawJavaReturn::Object(ptr::null_mut())
        );
    }

    #[test]
    fn prepares_original_call_arguments_from_generic_containers() {
        let (signature, args) =
            prepare_original_call_args("(IZLjava/lang/Object;)I", (1_i32, true, JavaValue::Null))
                .expect("tuple arguments should validate");
        assert_eq!(signature.to_string(), "(IZLjava/lang/Object;)I");
        assert_eq!(
            args,
            vec![JavaValue::Int(1), JavaValue::Boolean(true), JavaValue::Null]
        );

        let args = [JavaValue::Int(1), JavaValue::Long(2)];
        assert_eq!(
            prepare_original_call_args("(IJ)V", &args)
                .expect("array reference arguments should validate")
                .1,
            args
        );
    }

    #[test]
    fn rejects_invalid_generic_original_call_arguments() {
        assert_eq!(
            prepare_original_call_args("(I)V", (JavaValue::Long(1),)),
            Err(Error::InvalidArgumentType {
                index: 0,
                expected: "I".to_owned(),
                actual: "long",
            })
        );

        assert_eq!(
            prepare_original_call_args("(II)V", (1_i32,)),
            Err(Error::InvalidArguments {
                expected: 2,
                actual: 1,
            })
        );
    }

    #[test]
    fn extracts_typed_raw_return_values() {
        assert_eq!(RawJavaReturn::Void.into_void("test"), Ok(()));
        assert_eq!(
            RawJavaReturn::Boolean(jni::JNI_TRUE).into_boolean("test"),
            Ok(true)
        );
        assert_eq!(
            RawJavaReturn::Boolean(jni::JNI_FALSE).into_boolean("test"),
            Ok(false)
        );
        assert_eq!(RawJavaReturn::Byte(-7).into_byte("test"), Ok(-7));
        assert_eq!(RawJavaReturn::Char(65).into_char("test"), Ok(65));
        assert_eq!(RawJavaReturn::Short(-9).into_short("test"), Ok(-9));
        assert_eq!(RawJavaReturn::Int(11).into_int("test"), Ok(11));
        assert_eq!(RawJavaReturn::Long(13).into_long("test"), Ok(13));
        assert_eq!(RawJavaReturn::Float(1.25).into_float("test"), Ok(1.25));
        assert_eq!(RawJavaReturn::Double(2.5).into_double("test"), Ok(2.5));

        let object = ptr::dangling_mut();
        assert_eq!(
            RawJavaReturn::Object(object).into_object("test"),
            Ok(object)
        );
    }

    #[test]
    fn rejects_wrong_raw_return_extraction() {
        assert_eq!(
            RawJavaReturn::Int(1).into_object("test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "object",
                actual: "int".to_owned(),
            })
        );
    }

    #[test]
    fn validates_reference_to_reference_signatures() {
        validate_reference_to_reference_signature("(Ljava/lang/Object;)Ljava/lang/Object;", "test")
            .expect("object signature should be accepted");
        validate_reference_to_reference_signature(
            "(Lfrida/java/bridge/rs/test/TestSubject;)Lfrida/java/bridge/rs/test/TestSubject;",
            "test",
        )
        .expect("custom object signature should be accepted");
        validate_reference_to_reference_signature("([I)[Ljava/lang/Object;", "test")
            .expect("array signature should be accepted");
    }

    #[test]
    fn rejects_non_reference_replacement_signatures() {
        assert_eq!(
            validate_reference_to_reference_signature("(I)Ljava/lang/Object;", "test"),
            Err(Error::InvalidArgumentType {
                index: 0,
                expected: "reference".to_owned(),
                actual: "int",
            })
        );
        assert_eq!(
            validate_reference_to_reference_signature("(Ljava/lang/Object;)I", "test"),
            Err(Error::InvalidReturnType {
                operation: "test",
                expected: "reference",
                actual: "I".to_owned(),
            })
        );
        assert_eq!(
            validate_reference_to_reference_signature(
                "(Ljava/lang/Object;Ljava/lang/Object;)Ljava/lang/Object;",
                "test",
            ),
            Err(Error::InvalidArguments {
                expected: 1,
                actual: 2,
            })
        );
    }
}
