use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    slice,
    sync::Mutex,
};

use crate::{
    Error, Result,
    env::{Env, MethodKind},
    java::{IntoJavaArgs, JavaMethodOverload},
    jni,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

use super::{
    native::{
        MethodReplacement, replace_instance_closure_trampoline_method,
        replace_static_closure_trampoline_method, replacement_kind_name,
    },
    original::{OriginalMethod, RawJavaReturn, invalid_raw_return},
    trampoline::ClosureReplacementThunk,
};

type ReplacementClosure =
    dyn for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static;

pub(crate) struct ClosureMethodReplacement {
    replacement: Option<MethodReplacement>,
    thunk: Option<ClosureReplacementThunk>,
    state: Option<Box<ClosureReplacementState>>,
}

pub(crate) struct ReplacementInvocation<'state> {
    pub(crate) state: &'state ClosureReplacementState,
    pub(crate) env: *mut jni::JNIEnv,
    pub(crate) target: jni::jobject,
    pub(crate) arguments: Vec<JavaValue>,
}

pub(crate) struct ClosureReplacementState {
    pub(crate) vm: Vm,
    pub(crate) kind: MethodKind,
    pub(crate) name: String,
    pub(crate) signature: MethodSignature,
    pub(crate) original: OriginalMethod,
    pub(crate) callback: Box<ReplacementClosure>,
    pub(crate) last_error: Mutex<Option<String>>,
}

#[repr(C)]
pub(super) struct ClosureInvocationFrame {
    pub(super) state: *mut ClosureReplacementState,
    pub(super) env: *mut jni::JNIEnv,
    pub(super) target: jni::jobject,
    pub(super) arguments: *const jni::jvalue,
    pub(super) argument_count: usize,
    pub(super) return_value: jni::jvalue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosureReplacementAbi {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClosureReplacementLayout {
    pub(crate) arguments: Vec<ClosureArgumentLayout>,
    pub(crate) return_value: ClosureValueLayout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClosureArgumentLayout {
    pub(crate) ty: JavaType,
    pub(crate) value: ClosureValueLayout,
    pub(crate) location: ClosureArgumentLocation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosureArgumentLocation {
    GeneralRegister(u8),
    FloatRegister(u8),
    Stack { offset: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClosureValueLayout {
    Void,
    General32,
    General64,
    Float32,
    Float64,
    Reference,
}

impl ClosureMethodReplacement {
    pub(crate) fn revert(&mut self) -> Result<()> {
        if let Some(mut replacement) = self.replacement.take()
            && let Err(error) = replacement.revert()
        {
            self.replacement = Some(replacement);
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn debug_summary(&self) -> Option<String> {
        self.replacement
            .as_ref()
            .and_then(MethodReplacement::debug_summary)
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        self.state.as_ref().and_then(|state| state.last_error())
    }

    pub(crate) fn take_last_error(&self) -> Option<String> {
        self.state
            .as_ref()
            .and_then(|state| state.take_last_error())
    }
}

impl Drop for ClosureMethodReplacement {
    fn drop(&mut self) {
        if let Some(mut replacement) = self.replacement.take()
            && replacement.revert().is_err()
        {
            if let Some(mut thunk) = self.thunk.take() {
                thunk.leak();
            }
            if let Some(state) = self.state.take() {
                std::mem::forget(state);
            }
            std::mem::forget(replacement);
        }
    }
}

impl<'state> ReplacementInvocation<'state> {
    pub(crate) fn env_raw(&self) -> *mut jni::JNIEnv {
        self.env
    }

    pub(crate) fn env(&self) -> Result<Env<'state>> {
        let env = NonNull::new(self.env).ok_or(Error::NullReturn {
            operation: "closure replacement JNIEnv",
        })?;
        Ok(Env::from_raw(env, &self.state.vm))
    }

    pub(crate) fn kind(&self) -> MethodKind {
        self.state.kind
    }

    pub(crate) fn name(&self) -> &str {
        &self.state.name
    }

    pub(crate) fn signature(&self) -> &MethodSignature {
        &self.state.signature
    }

    pub(crate) fn class(&self) -> Option<jni::jclass> {
        (self.state.kind == MethodKind::Static).then_some(self.target.cast())
    }

    pub(crate) fn receiver(&self) -> Option<jni::jobject> {
        (self.state.kind == MethodKind::Instance).then_some(self.target)
    }

    pub(crate) fn arguments(&self) -> &[JavaValue] {
        &self.arguments
    }

    /// Calls the replaced method's original implementation from this closure.
    ///
    /// # Safety
    ///
    /// The raw JNI target received by this invocation must still be valid, and this must only be
    /// called while the current thread is inside this replacement callback.
    pub(crate) unsafe fn call_original<A: IntoJavaArgs>(&self, args: A) -> Result<RawJavaReturn> {
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

    pub(crate) fn invoke(
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

    pub(crate) fn record_error(&self, error: String) {
        *self
            .last_error
            .lock()
            .expect("closure replacement error mutex poisoned") = Some(error);
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .expect("closure replacement error mutex poisoned")
            .clone()
    }

    pub(crate) fn take_last_error(&self) -> Option<String> {
        self.last_error
            .lock()
            .expect("closure replacement error mutex poisoned")
            .take()
    }
}

pub(crate) unsafe fn replace_closure_method<F>(
    overload: &JavaMethodOverload,
    callback: F,
) -> Result<ClosureMethodReplacement>
where
    F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
{
    if overload.kind() == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "replacement::replace_closure_method",
        });
    }

    let layout = closure_replacement_layout(overload.kind(), overload.signature())?;
    let mut state = Box::new(ClosureReplacementState::new(overload, callback)?);
    let thunk = ClosureReplacementThunk::new(&layout, state.as_mut() as *mut _)?;
    let signature = overload.signature().to_string();
    let replacement = match overload.kind() {
        MethodKind::Static => unsafe {
            replace_static_closure_trampoline_method(
                overload.class(),
                overload.name(),
                &signature,
                thunk.as_ptr(),
            )?
        },
        MethodKind::Instance => unsafe {
            replace_instance_closure_trampoline_method(
                overload.class(),
                overload.name(),
                &signature,
                thunk.as_ptr(),
            )?
        },
        MethodKind::Constructor => {
            return Err(Error::WrongMethodKind {
                operation: "replacement::replace_closure_method",
            });
        }
    };

    Ok(ClosureMethodReplacement {
        replacement: Some(replacement),
        thunk: Some(thunk),
        state: Some(state),
    })
}

pub(super) unsafe extern "C" fn dispatch_closure_invocation(frame: *mut ClosureInvocationFrame) {
    let Some(frame) = (unsafe { frame.as_mut() }) else {
        return;
    };
    frame.return_value = zero_jvalue();

    let Some(state) = (unsafe { frame.state.as_ref() }) else {
        return;
    };
    let args = if frame.arguments.is_null() && frame.argument_count != 0 {
        Err(Error::NullReturn {
            operation: "closure replacement arguments",
        })
    } else {
        let values = unsafe { slice::from_raw_parts(frame.arguments, frame.argument_count) };
        closure_arguments_from_jvalues(state.signature.arguments(), values)
    };

    let result = match args {
        Ok(args) => state.invoke(frame.env, frame.target, args),
        Err(error) => {
            state.record_error(error.to_string());
            state.default_return()
        }
    };
    frame.return_value = raw_return_to_jvalue(result);
}

pub(crate) fn closure_replacement_abi(
    kind: MethodKind,
    signature: &MethodSignature,
) -> Result<ClosureReplacementAbi> {
    if kind == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "replacement::replace_closure_method",
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
            operation: "replacement::replace_closure_method",
            expected: format!(
                "supported {} closure replacement ABI",
                replacement_kind_name(kind)
            ),
            actual: "closure",
        });
    };
    Ok(abi)
}

pub(crate) fn closure_replacement_layout(
    kind: MethodKind,
    signature: &MethodSignature,
) -> Result<ClosureReplacementLayout> {
    if kind == MethodKind::Constructor {
        return Err(Error::WrongMethodKind {
            operation: "replacement::replace_closure_method",
        });
    }

    let mut next_general_register = 2;
    let mut next_float_register = 0;
    let mut next_stack_offset = 0;
    let mut arguments = Vec::with_capacity(signature.arguments().len());
    for ty in signature.arguments() {
        let value = ClosureValueLayout::for_type(ty);
        let location = match value.register_class() {
            Some(ClosureRegisterClass::General) if next_general_register < 8 => {
                let register = next_general_register;
                next_general_register += 1;
                ClosureArgumentLocation::GeneralRegister(register)
            }
            Some(ClosureRegisterClass::Float) if next_float_register < 8 => {
                let register = next_float_register;
                next_float_register += 1;
                ClosureArgumentLocation::FloatRegister(register)
            }
            Some(_) => {
                let offset = next_stack_offset;
                next_stack_offset += 8;
                ClosureArgumentLocation::Stack { offset }
            }
            None => unreachable!("Java method arguments cannot be void"),
        };

        arguments.push(ClosureArgumentLayout {
            ty: ty.clone(),
            value,
            location,
        });
    }

    Ok(ClosureReplacementLayout {
        arguments,
        return_value: ClosureValueLayout::for_type(signature.return_type()),
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClosureRegisterClass {
    General,
    Float,
}

impl ClosureValueLayout {
    fn for_type(ty: &JavaType) -> Self {
        match ty {
            JavaType::Void => Self::Void,
            JavaType::Boolean
            | JavaType::Byte
            | JavaType::Char
            | JavaType::Short
            | JavaType::Int => Self::General32,
            JavaType::Long => Self::General64,
            JavaType::Float => Self::Float32,
            JavaType::Double => Self::Float64,
            JavaType::Object(_) | JavaType::Array(_) => Self::Reference,
        }
    }

    fn register_class(self) -> Option<ClosureRegisterClass> {
        match self {
            Self::Void => None,
            Self::General32 | Self::General64 | Self::Reference => {
                Some(ClosureRegisterClass::General)
            }
            Self::Float32 | Self::Float64 => Some(ClosureRegisterClass::Float),
        }
    }
}

fn closure_arguments_from_jvalues(
    signature_arguments: &[JavaType],
    values: &[jni::jvalue],
) -> Result<Vec<JavaValue>> {
    if signature_arguments.len() != values.len() {
        return Err(Error::InvalidArguments {
            expected: signature_arguments.len(),
            actual: values.len(),
        });
    }

    signature_arguments
        .iter()
        .zip(values)
        .map(|(ty, value)| unsafe { java_value_from_jvalue(ty, *value) })
        .collect()
}

unsafe fn java_value_from_jvalue(ty: &JavaType, value: jni::jvalue) -> Result<JavaValue> {
    Ok(match ty {
        JavaType::Void => {
            return Err(Error::InvalidArgumentType {
                index: 0,
                expected: "non-void argument".to_owned(),
                actual: "void",
            });
        }
        JavaType::Boolean => JavaValue::Boolean(unsafe { value.z } != jni::JNI_FALSE),
        JavaType::Byte => JavaValue::Byte(unsafe { value.b }),
        JavaType::Char => JavaValue::Char(unsafe { value.c }),
        JavaType::Short => JavaValue::Short(unsafe { value.s }),
        JavaType::Int => JavaValue::Int(unsafe { value.i }),
        JavaType::Long => JavaValue::Long(unsafe { value.j }),
        JavaType::Float => JavaValue::Float(unsafe { value.f }),
        JavaType::Double => JavaValue::Double(unsafe { value.d }),
        JavaType::Object(_) | JavaType::Array(_) => reference_argument(unsafe { value.l }),
    })
}

fn raw_return_to_jvalue(value: RawJavaReturn) -> jni::jvalue {
    match value {
        RawJavaReturn::Void => zero_jvalue(),
        RawJavaReturn::Boolean(value) => jni::jvalue { z: value },
        RawJavaReturn::Byte(value) => jni::jvalue { b: value },
        RawJavaReturn::Char(value) => jni::jvalue { c: value },
        RawJavaReturn::Short(value) => jni::jvalue { s: value },
        RawJavaReturn::Int(value) => jni::jvalue { i: value },
        RawJavaReturn::Long(value) => jni::jvalue { j: value },
        RawJavaReturn::Float(value) => jni::jvalue { f: value },
        RawJavaReturn::Double(value) => jni::jvalue { d: value },
        RawJavaReturn::Object(value) => jni::jvalue { l: value },
    }
}

fn zero_jvalue() -> jni::jvalue {
    jni::jvalue { j: 0 }
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

pub(super) unsafe extern "C" fn closure_no_args_void(
    state: *mut ClosureReplacementState,
    env: *mut jni::JNIEnv,
    target: jni::jobject,
) {
    let Some(state) = (unsafe { closure_state(state) }) else {
        return;
    };
    let _ = state.invoke(env, target, Vec::new());
}

pub(super) unsafe extern "C" fn closure_no_args_boolean(
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

pub(super) unsafe extern "C" fn closure_no_args_byte(
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

pub(super) unsafe extern "C" fn closure_no_args_char(
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

pub(super) unsafe extern "C" fn closure_no_args_short(
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

pub(super) unsafe extern "C" fn closure_no_args_int(
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

pub(super) unsafe extern "C" fn closure_no_args_long(
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

pub(super) unsafe extern "C" fn closure_no_args_float(
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

pub(super) unsafe extern "C" fn closure_no_args_double(
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

pub(super) unsafe extern "C" fn closure_no_args_object(
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

pub(super) unsafe extern "C" fn closure_one_reference_to_reference(
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

pub(super) unsafe extern "C" fn closure_one_reference_to_void(
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

pub(super) unsafe extern "C" fn closure_i32_i32_to_i32(
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
