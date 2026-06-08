use std::{
    collections::HashMap,
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
    ptr::{self, NonNull},
    slice,
    sync::{Arc, Condvar, Mutex},
    thread::ThreadId,
};

use crate::{
    Error, Result,
    art::ArtMethodReplacementGuard,
    env::{Env, MethodKind, PendingJavaException},
    java::{IntoJavaCallArgs, JavaConstructor, JavaMethod, raw},
    jni,
    signature::{JavaType, MethodSignature},
    value::JavaValue,
    vm::Vm,
};

use super::{
    api::JavaHookError,
    original::{
        RawJavaReturn, call_original_constructor_method, call_original_instance_method,
        call_original_static_method, invalid_raw_return,
    },
    trampoline::{ClosureReplacementThunk, validate_closure_trampoline_layout},
};

type ReplacementClosure =
    dyn for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static;
type ReplacementErrorHandler = dyn Fn(JavaHookError) + Send + Sync + 'static;

const CALLBACK_LOCAL_FRAME_CAPACITY: jni::jint = 8;

pub(crate) struct ClosureMethodReplacement {
    replacement: Option<ArtMethodReplacementGuard>,
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
    pub(crate) target_class: raw::Class,
    pub(crate) kind: MethodKind,
    pub(crate) name: String,
    pub(crate) signature: MethodSignature,
    pub(crate) callback: Box<ReplacementClosure>,
    pub(crate) last_error: Mutex<Option<String>>,
    pub(crate) error_handler: Mutex<Option<Arc<ReplacementErrorHandler>>>,
    pub(crate) active_invocations: ActiveInvocationState,
}

#[derive(Default)]
pub(crate) struct ActiveInvocationState {
    inner: Mutex<ActiveInvocationCounts>,
    drained: Condvar,
}

#[derive(Default)]
struct ActiveInvocationCounts {
    closing: bool,
    total: usize,
    threads: HashMap<ThreadId, usize>,
}

struct ActiveInvocationGuard<'state> {
    state: &'state ActiveInvocationState,
    thread: ThreadId,
    closing: bool,
}

struct CallbackLocalFrame<'scope> {
    env: Env<'scope>,
    active: bool,
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
        if let Some(state) = &self.state
            && !state.close_and_wait_until_inactive()
        {
            return Err(Error::InvalidReplacementState {
                operation: "closure replacement revert",
                reason: "replacement is active on the current thread".to_owned(),
            });
        }
        if let Some(mut replacement) = self.replacement.take()
            && let Err(error) = replacement.revert()
        {
            self.replacement = Some(replacement);
            return Err(error);
        }
        if let Some(state) = &self.state {
            state.wait_until_inactive();
        }
        Ok(())
    }

    pub(crate) fn last_error(&self) -> Option<String> {
        self.state.as_ref().and_then(|state| state.last_error())
    }

    pub(crate) fn take_last_error(&self) -> Option<String> {
        self.state
            .as_ref()
            .and_then(|state| state.take_last_error())
    }

    pub(crate) fn set_error_handler<F>(&self, handler: F)
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        if let Some(state) = &self.state {
            state.set_error_handler(Arc::new(handler));
        }
    }

    pub(crate) fn clear_error_handler(&self) {
        if let Some(state) = &self.state {
            state.clear_error_handler();
        }
    }

    fn record_drop_error(&self, error: String) {
        if let Some(state) = &self.state {
            state.record_error(error);
        }
    }

    fn leak_state_and_thunk(&mut self) {
        if let Some(mut thunk) = self.thunk.take() {
            thunk.leak();
        }
        if let Some(state) = self.state.take() {
            std::mem::forget(state);
        }
    }
}

impl Drop for ClosureMethodReplacement {
    fn drop(&mut self) {
        if let Some(state) = self.state.as_ref()
            && !state.close_and_wait_until_inactive()
        {
            self.record_drop_error(
                "closure replacement drop leaked hook state: replacement is active on the current thread"
                    .to_owned(),
            );
            self.leak_state_and_thunk();
            if let Some(replacement) = self.replacement.take() {
                std::mem::forget(replacement);
            }
            return;
        }

        if let Some(mut replacement) = self.replacement.take()
            && let Err(error) = replacement.revert()
        {
            self.record_drop_error(format!(
                "closure replacement drop leaked hook state after restore failure: {error}"
            ));
            self.leak_state_and_thunk();
            std::mem::forget(replacement);
        }
        if let Some(state) = &self.state {
            state.wait_until_inactive();
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
        Ok(Env::from_raw(env, self.state.vm.clone()))
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
        matches!(
            self.state.kind,
            MethodKind::Instance | MethodKind::Constructor
        )
        .then_some(self.target)
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
    pub(crate) unsafe fn call_original<A: IntoJavaCallArgs>(
        &self,
        args: A,
    ) -> Result<RawJavaReturn> {
        let signature = self.state.signature.to_string();
        match self.state.kind {
            MethodKind::Static => unsafe {
                call_original_static_method(
                    &self.state.vm,
                    self.env,
                    self.target.cast(),
                    &self.state.name,
                    &signature,
                    args,
                )
            },
            MethodKind::Instance => unsafe {
                call_original_instance_method(
                    &self.state.vm,
                    self.env,
                    self.target,
                    &self.state.name,
                    &signature,
                    args,
                )
            },
            MethodKind::Constructor => unsafe {
                call_original_constructor_method(
                    &self.state.vm,
                    self.env,
                    self.target,
                    &self.state.target_class,
                    &signature,
                    args,
                )
            },
        }
    }
}

impl ClosureReplacementState {
    fn new<F>(overload: &JavaMethod, callback: F) -> Result<Self>
    where
        F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
    {
        Ok(Self {
            vm: overload.class().vm().clone(),
            target_class: overload.class().clone(),
            kind: overload.kind(),
            name: overload.name().to_owned(),
            signature: overload.signature().clone(),
            callback: Box::new(callback),
            last_error: Mutex::new(None),
            error_handler: Mutex::new(None),
            active_invocations: ActiveInvocationState::default(),
        })
    }

    fn new_constructor<F>(overload: &JavaConstructor, callback: F) -> Result<Self>
    where
        F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
    {
        Ok(Self {
            vm: overload.class().vm().clone(),
            target_class: overload.class().clone(),
            kind: MethodKind::Constructor,
            name: "<init>".to_owned(),
            signature: overload.signature().clone(),
            callback: Box::new(callback),
            last_error: Mutex::new(None),
            error_handler: Mutex::new(None),
            active_invocations: ActiveInvocationState::default(),
        })
    }

    pub(crate) fn invoke(
        &self,
        env: *mut jni::JNIEnv,
        target: jni::jobject,
        arguments: Vec<JavaValue>,
    ) -> RawJavaReturn {
        let active = self.enter_invocation();
        if active.is_closing() {
            return self.default_return();
        }
        let frame = match CallbackLocalFrame::push(env, &self.vm) {
            Ok(frame) => frame,
            Err(error) => {
                self.record_error_preserving_java_exception(env, error);
                return self.default_return();
            }
        };
        let invocation = ReplacementInvocation {
            state: self,
            env,
            target,
            arguments,
        };
        let result = catch_unwind(AssertUnwindSafe(|| (self.callback)(invocation)));
        match result {
            Ok(Ok(value)) => match self.validate_return(value) {
                Ok(value) => self.finish_invocation_return(frame, value),
                Err(error) => {
                    self.record_error_preserving_java_exception(env, error);
                    self.default_return()
                }
            },
            Ok(Err(error)) => {
                self.record_error_preserving_java_exception(env, error);
                self.default_return()
            }
            Err(_) => {
                self.record_message_preserving_pending_exception(
                    env,
                    "closure replacement callback panicked".to_owned(),
                );
                self.default_return()
            }
        }
    }

    fn finish_invocation_return(
        &self,
        frame: Option<CallbackLocalFrame<'_>>,
        value: RawJavaReturn,
    ) -> RawJavaReturn {
        match frame {
            Some(frame) => frame.pop_return(value),
            None => value,
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
            .expect("closure replacement error mutex poisoned") = Some(error.clone());

        let handler = self
            .error_handler
            .lock()
            .expect("closure replacement error-handler mutex poisoned")
            .clone();
        if let Some(handler) = handler {
            let reported_error = error.clone();
            let result = catch_unwind(AssertUnwindSafe(|| {
                handler(JavaHookError::new(
                    self.kind,
                    self.name.clone(),
                    self.signature.clone(),
                    reported_error,
                ));
            }));
            if result.is_err() {
                *self
                    .last_error
                    .lock()
                    .expect("closure replacement error mutex poisoned") =
                    Some(format!("{error}; replacement error handler panicked"));
            }
        }
    }

    fn record_error_preserving_java_exception(&self, env: *mut jni::JNIEnv, error: Error) {
        let message = error.to_string();
        if let Some(throwable) = error.java_throwable().cloned() {
            self.record_error(message);
            let Some(env) = NonNull::new(env) else {
                self.record_error("failed to restore Java exception: null JNIEnv".to_owned());
                return;
            };
            if let Err(error) = unsafe { throwable.throw(env) } {
                self.record_error(format!("failed to restore Java exception: {error}"));
            }
            return;
        }

        self.record_message_preserving_pending_exception(env, message);
    }

    fn record_message_preserving_pending_exception(&self, env: *mut jni::JNIEnv, error: String) {
        let pending_exception =
            NonNull::new(env).and_then(|env| unsafe { PendingJavaException::take(env) });
        self.record_error(error);
        if let Some(exception) = pending_exception
            && let Err(error) = unsafe { exception.throw() }
        {
            self.record_error(format!("failed to restore pending Java exception: {error}"));
        }
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

    pub(crate) fn set_error_handler(&self, handler: Arc<ReplacementErrorHandler>) {
        *self
            .error_handler
            .lock()
            .expect("closure replacement error-handler mutex poisoned") = Some(handler);
    }

    pub(crate) fn clear_error_handler(&self) {
        *self
            .error_handler
            .lock()
            .expect("closure replacement error-handler mutex poisoned") = None;
    }

    fn enter_invocation(&self) -> ActiveInvocationGuard<'_> {
        self.active_invocations.enter()
    }

    fn wait_until_inactive(&self) -> bool {
        self.active_invocations.wait_until_inactive()
    }

    fn close_and_wait_until_inactive(&self) -> bool {
        self.active_invocations.close_and_wait_until_inactive()
    }
}

impl<'scope> CallbackLocalFrame<'scope> {
    fn push(env: *mut jni::JNIEnv, vm: &'scope Vm) -> Result<Option<Self>> {
        let Some(env) = NonNull::new(env) else {
            return Ok(None);
        };
        let env = Env::from_raw(env, vm.clone());
        env.push_local_frame_raw(CALLBACK_LOCAL_FRAME_CAPACITY)?;
        Ok(Some(Self { env, active: true }))
    }

    fn pop_return(mut self, value: RawJavaReturn) -> RawJavaReturn {
        let survivor = callback_local_frame_survivor(value);
        match value {
            RawJavaReturn::Object(object) => {
                debug_assert_eq!(survivor, object);
                RawJavaReturn::Object(unsafe { self.pop_with_survivor(survivor) })
            }
            other => {
                unsafe { self.pop_with_survivor(survivor) };
                other
            }
        }
    }

    unsafe fn pop_with_survivor(&mut self, survivor: jni::jobject) -> jni::jobject {
        if !self.active {
            return survivor;
        }
        self.active = false;
        unsafe { self.env.pop_local_frame_raw(survivor) }
    }
}

pub(crate) fn callback_local_frame_survivor(value: RawJavaReturn) -> jni::jobject {
    match value {
        RawJavaReturn::Object(object) => object,
        _ => ptr::null_mut(),
    }
}

impl Drop for CallbackLocalFrame<'_> {
    fn drop(&mut self) {
        if self.active {
            unsafe { self.pop_with_survivor(ptr::null_mut()) };
        }
    }
}

impl ActiveInvocationState {
    fn enter(&self) -> ActiveInvocationGuard<'_> {
        let thread = std::thread::current().id();
        let mut counts = self
            .inner
            .lock()
            .expect("closure replacement active-invocation mutex poisoned");
        counts.total += 1;
        *counts.threads.entry(thread).or_insert(0) += 1;
        ActiveInvocationGuard {
            state: self,
            thread,
            closing: counts.closing,
        }
    }

    fn wait_until_inactive(&self) -> bool {
        let thread = std::thread::current().id();
        let mut counts = self
            .inner
            .lock()
            .expect("closure replacement active-invocation mutex poisoned");
        if counts.threads.get(&thread).copied().unwrap_or(0) != 0 {
            return false;
        }
        while counts.total != 0 {
            counts = self
                .drained
                .wait(counts)
                .expect("closure replacement active-invocation mutex poisoned");
        }
        true
    }

    fn close_and_wait_until_inactive(&self) -> bool {
        let thread = std::thread::current().id();
        let mut counts = self
            .inner
            .lock()
            .expect("closure replacement active-invocation mutex poisoned");
        if counts.threads.get(&thread).copied().unwrap_or(0) != 0 {
            return false;
        }
        counts.closing = true;
        while counts.total != 0 {
            counts = self
                .drained
                .wait(counts)
                .expect("closure replacement active-invocation mutex poisoned");
        }
        true
    }
}

impl ActiveInvocationGuard<'_> {
    fn is_closing(&self) -> bool {
        self.closing
    }
}

impl Drop for ActiveInvocationGuard<'_> {
    fn drop(&mut self) {
        let mut counts = self
            .state
            .inner
            .lock()
            .expect("closure replacement active-invocation mutex poisoned");
        counts.total = counts.total.saturating_sub(1);
        if let Some(thread_count) = counts.threads.get_mut(&self.thread) {
            *thread_count = thread_count.saturating_sub(1);
            if *thread_count == 0 {
                counts.threads.remove(&self.thread);
            }
        }
        if counts.total == 0 {
            self.state.drained.notify_all();
        }
    }
}

pub(crate) unsafe fn replace_closure_method<F>(
    overload: &JavaMethod,
    callback: F,
) -> Result<ClosureMethodReplacement>
where
    F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
{
    let layout = validate_closure_signature_for_kind(
        overload.kind(),
        overload.signature(),
        "java::replacement::replace_closure_method",
    )?;
    validate_closure_trampoline_layout(&layout, "java::replacement::replace_closure_method")?;
    let mut state = Box::new(ClosureReplacementState::new(overload, callback)?);
    let thunk = ClosureReplacementThunk::new(&layout, state.as_mut() as *mut _)?;
    let replacement = match overload.kind() {
        MethodKind::Static | MethodKind::Instance => unsafe {
            replace_closure_trampoline_method(
                overload.class(),
                overload.kind(),
                Some(overload.name()),
                overload.signature(),
                thunk.as_ptr(),
            )?
        },
        MethodKind::Constructor => {
            return Err(Error::WrongMethodKind {
                operation: "java::replacement::replace_closure_method",
            });
        }
    };

    Ok(ClosureMethodReplacement {
        replacement: Some(replacement),
        thunk: Some(thunk),
        state: Some(state),
    })
}

pub(crate) unsafe fn replace_constructor_closure<F>(
    overload: &JavaConstructor,
    callback: F,
) -> Result<ClosureMethodReplacement>
where
    F: for<'a> Fn(ReplacementInvocation<'a>) -> Result<RawJavaReturn> + Send + Sync + 'static,
{
    let layout = validate_closure_signature_for_kind(
        MethodKind::Constructor,
        overload.signature(),
        "java::replacement::replace_constructor_closure",
    )?;
    let mut state = Box::new(ClosureReplacementState::new_constructor(
        overload, callback,
    )?);
    let thunk = ClosureReplacementThunk::new(&layout, state.as_mut() as *mut _)?;
    let replacement = unsafe {
        replace_closure_trampoline_method(
            overload.class(),
            MethodKind::Constructor,
            None,
            overload.signature(),
            thunk.as_ptr(),
        )?
    };

    Ok(ClosureMethodReplacement {
        replacement: Some(replacement),
        thunk: Some(thunk),
        state: Some(state),
    })
}

unsafe fn replace_closure_trampoline_method(
    class: &raw::Class,
    kind: MethodKind,
    name: Option<&str>,
    signature: &MethodSignature,
    replacement: *mut c_void,
) -> Result<ArtMethodReplacementGuard> {
    let signature = signature.to_string();
    let method = match kind {
        MethodKind::Static => class.resolve_static_method(
            name.ok_or(Error::WrongMethodKind {
                operation: "java::replacement::replace_closure_method",
            })?,
            &signature,
        )?,
        MethodKind::Instance => class.resolve_instance_method(
            name.ok_or(Error::WrongMethodKind {
                operation: "java::replacement::replace_closure_method",
            })?,
            &signature,
        )?,
        MethodKind::Constructor => class.resolve_constructor(&signature)?,
    };

    class.vm().art().replace_method(
        class.vm().clone().into(),
        method.kind(),
        unsafe { method.raw() },
        replacement,
    )
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

pub(crate) fn validate_closure_replacement_signature(
    kind: MethodKind,
    signature: &MethodSignature,
    operation: &'static str,
) -> Result<()> {
    let layout = validate_closure_signature_for_kind(kind, signature, operation)?;
    validate_closure_trampoline_layout(&layout, operation)
}

fn validate_closure_signature_for_kind(
    kind: MethodKind,
    signature: &MethodSignature,
    operation: &'static str,
) -> Result<ClosureReplacementLayout> {
    if kind == MethodKind::Constructor && signature.return_type() != &JavaType::Void {
        return Err(Error::InvalidReplacementImplementation {
            operation,
            expected: "constructor replacement descriptor returning void".to_owned(),
            actual: "non-void constructor descriptor",
        });
    }

    closure_replacement_layout(kind, signature)
}

pub(crate) fn closure_replacement_layout(
    kind: MethodKind,
    signature: &MethodSignature,
) -> Result<ClosureReplacementLayout> {
    if kind == MethodKind::Constructor && signature.return_type() != &JavaType::Void {
        return Err(Error::InvalidReplacementImplementation {
            operation: "java::replacement::replace_closure_method",
            expected: "constructor replacement descriptor returning void".to_owned(),
            actual: "non-void constructor descriptor",
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

fn reference_argument(value: jni::jobject) -> JavaValue {
    if value.is_null() {
        JavaValue::NULL
    } else {
        JavaValue::object_ref(value)
    }
}
