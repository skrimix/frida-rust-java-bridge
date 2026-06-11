use std::{
    cell::RefCell,
    ffi::c_void,
    panic::{self, AssertUnwindSafe},
    ptr::NonNull,
};

use crate::{
    env::Env,
    error::{Error, Result},
    jni,
};

use super::layout::find_art_thread_jni_env_offset;

#[cfg(target_arch = "aarch64")]
mod arm64;

const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const TRANSITION_CODE_SIZE: usize = 65536;
const JNIENV_EXT_SELF_OFFSET: u64 = POINTER_SIZE as u64;

type RunnableThreadTransitionPerform = unsafe extern "C" fn(*mut jni::JNIEnv);
type RunnableCallback<'a> = dyn FnMut(*mut c_void) + 'a;

thread_local! {
    static RUNNABLE_CALLBACK: RefCell<RunnableCallbackSlot> =
        RefCell::new(RunnableCallbackSlot::default());
}

#[derive(Default)]
struct RunnableCallbackSlot {
    active: bool,
    callback: Option<*mut RunnableCallback<'static>>,
}

#[derive(Debug)]
struct RunnableCallbackGuard;

impl Drop for RunnableCallbackGuard {
    fn drop(&mut self) {
        RUNNABLE_CALLBACK.with(|slot| {
            *slot.borrow_mut() = RunnableCallbackSlot::default();
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtThreadSpec {
    exception_offset: usize,
}

pub(super) struct RunnableThreadTransition {
    perform: RunnableThreadTransitionPerform,
    code: NonNull<c_void>,
}

unsafe impl Send for RunnableThreadTransition {}
unsafe impl Sync for RunnableThreadTransition {}

impl RunnableThreadTransition {
    pub(super) fn run(
        &self,
        feature: &'static str,
        env: &Env<'_>,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let mut result = None;
        let mut f = Some(f);
        let mut callback = |thread| {
            if let Some(f) = f.take() {
                result = Some(f(thread));
            }
        };

        let callback: *mut RunnableCallback<'_> = &mut callback;
        let callback = unsafe {
            std::mem::transmute::<*mut RunnableCallback<'_>, *mut RunnableCallback<'static>>(
                callback,
            )
        };
        let _guard = install_runnable_callback(feature, callback)?;

        let unwind = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.perform)(env.handle().as_ptr());
        }));

        if unwind.is_err() {
            return unsupported(feature, "runnable thread transition callback panicked");
        }

        result
            .unwrap_or_else(|| unsupported(feature, "unable to perform runnable thread transition"))
    }
}

fn install_runnable_callback(
    feature: &'static str,
    callback: *mut RunnableCallback<'static>,
) -> Result<RunnableCallbackGuard> {
    RUNNABLE_CALLBACK.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.active || slot.callback.is_some() {
            return unsupported(
                feature,
                "runnable thread transition is already active on this thread",
            );
        }

        slot.active = true;
        slot.callback = Some(callback);
        Ok(RunnableCallbackGuard)
    })
}

impl Drop for RunnableThreadTransition {
    fn drop(&mut self) {
        unsafe { frida_gum_sys::gum_free_pages(self.code.as_ptr()) };
    }
}

pub(super) fn build(
    feature: &'static str,
    env: &Env<'_>,
    exception_clear: Option<*const c_void>,
    fatal_error: Option<*const c_void>,
) -> Result<RunnableThreadTransition> {
    if !cfg!(target_arch = "aarch64") {
        return unsupported(
            feature,
            "runnable thread transition recompilation only supports arm64-v8a",
        );
    }

    let thread_spec = detect_thread_spec(feature, env)?;
    let exception_clear = exception_clear.unwrap_or_else(|| unsafe {
        let function: jni::ExceptionClear =
            jni::env_function(env.handle(), jni::ENV_EXCEPTION_CLEAR);
        function as *const c_void
    });
    let fatal_error = fatal_error.unwrap_or_else(|| unsafe {
        let function: jni::FatalError = jni::env_function(env.handle(), jni::ENV_FATAL_ERROR);
        function as *const c_void
    });

    #[cfg(target_arch = "aarch64")]
    {
        arm64::build_thread_transition(feature, exception_clear, fatal_error, thread_spec)
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        let _ = (exception_clear, fatal_error, thread_spec);
        unsupported(
            feature,
            "runnable thread transition recompilation only supports arm64-v8a",
        )
    }
}

fn detect_thread_spec(feature: &'static str, env: &Env<'_>) -> Result<ArtThreadSpec> {
    let thread = art_thread_from_env(env);
    if thread.is_null() {
        return unsupported(feature, "ART Thread pointer is null");
    }

    // SAFETY: `env` is borrowed on the current attached thread while probing ART thread fields.
    detect_thread_exception_offset(feature, thread, unsafe { env.handle() }.as_ptr().cast())
        .map(|exception_offset| ArtThreadSpec { exception_offset })
}

fn detect_thread_exception_offset(
    feature: &'static str,
    thread: *mut c_void,
    env: *mut c_void,
) -> Result<usize> {
    if let Some(offset) = find_art_thread_jni_env_offset(thread, env, None) {
        return Ok(offset - (6 * POINTER_SIZE));
    }

    unsupported(feature, "unable to determine ArtThread field offsets")
}

fn art_thread_from_env(env: &Env<'_>) -> *mut c_void {
    unsafe { env.handle().as_ptr().cast::<*mut c_void>().add(1).read() }
}

unsafe extern "C" fn on_runnable_thread_transition_complete(thread: *mut c_void) {
    let callback = RUNNABLE_CALLBACK.with(|slot| slot.borrow_mut().callback.take());
    let Some(callback) = callback else {
        return;
    };

    let _ = panic::catch_unwind(AssertUnwindSafe(|| {
        let callback = unsafe { &mut *callback };
        callback(thread);
    }));
}

#[cfg(target_arch = "aarch64")]
pub(super) fn detect_jni_ids_indirection_offset(
    feature: &'static str,
    set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    arm64::detect_jni_ids_indirection_offset(feature, set_jni_id_type)
}

fn unsupported<T>(feature: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn derives_thread_exception_offset_from_jni_env_field() {
        let mut thread = [0usize; 40];
        let env = 0x1234usize as *mut c_void;
        let jni_env_offset = 160;
        thread[jni_env_offset / POINTER_SIZE] = env as usize;

        let exception_offset =
            detect_thread_exception_offset("test feature", thread.as_mut_ptr().cast(), env)
                .unwrap();

        assert_eq!(exception_offset, jni_env_offset - (6 * POINTER_SIZE));
    }

    #[test]
    fn runnable_callback_slot_rejects_same_thread_reentry() {
        let mut callback = |_thread| {};
        let callback: *mut RunnableCallback<'_> = &mut callback;
        let callback = unsafe {
            std::mem::transmute::<*mut RunnableCallback<'_>, *mut RunnableCallback<'static>>(
                callback,
            )
        };

        let guard = install_runnable_callback("test feature", callback).unwrap();
        let error = install_runnable_callback("test feature", callback).unwrap_err();

        assert!(matches!(
            error,
            Error::UnsupportedFeature { feature, reason }
                if feature == "test feature"
                    && reason == "runnable thread transition is already active on this thread"
        ));

        drop(guard);
        install_runnable_callback("test feature", callback).unwrap();
    }

    #[test]
    fn runnable_callback_slot_stays_active_after_callback_is_consumed() {
        let called = Cell::new(false);
        let mut callback = |_thread| {
            called.set(true);
        };
        let callback: *mut RunnableCallback<'_> = &mut callback;
        let callback = unsafe {
            std::mem::transmute::<*mut RunnableCallback<'_>, *mut RunnableCallback<'static>>(
                callback,
            )
        };

        let guard = install_runnable_callback("test feature", callback).unwrap();

        unsafe { on_runnable_thread_transition_complete(std::ptr::null_mut()) };
        let error = install_runnable_callback("test feature", callback).unwrap_err();

        assert!(called.get());
        assert!(matches!(
            error,
            Error::UnsupportedFeature { feature, reason }
                if feature == "test feature"
                    && reason == "runnable thread transition is already active on this thread"
        ));

        drop(guard);
        install_runnable_callback("test feature", callback).unwrap();
    }
}
