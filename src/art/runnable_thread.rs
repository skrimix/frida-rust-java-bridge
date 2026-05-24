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

#[cfg(target_arch = "aarch64")]
mod arm64;

const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const TRANSITION_CODE_SIZE: usize = 65536;
const JNIENV_EXT_SELF_OFFSET: u64 = POINTER_SIZE as u64;

type RunnableThreadTransitionPerform = unsafe extern "C" fn(*mut jni::JNIEnv);
type RunnableCallback<'a> = dyn FnMut(*mut c_void) + 'a;

thread_local! {
    static RUNNABLE_CALLBACK: RefCell<Option<*mut RunnableCallback<'static>>> =
        RefCell::new(None);
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
        RUNNABLE_CALLBACK.with(|slot| {
            *slot.borrow_mut() = Some(callback);
        });

        let unwind = panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            (self.perform)(env.handle().as_ptr());
        }));

        RUNNABLE_CALLBACK.with(|slot| {
            *slot.borrow_mut() = None;
        });

        if unwind.is_err() {
            return unsupported(feature, "runnable thread transition callback panicked");
        }

        result
            .unwrap_or_else(|| unsupported(feature, "unable to perform runnable thread transition"))
    }
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
    let thread = thread.cast::<usize>();
    let env_value = env as usize;
    for offset in (144..256).step_by(POINTER_SIZE) {
        let value = unsafe { thread.byte_add(offset).read() };
        if value == env_value {
            return Ok(offset - (6 * POINTER_SIZE));
        }
    }

    unsupported(feature, "unable to determine ArtThread field offsets")
}

fn art_thread_from_env(env: &Env<'_>) -> *mut c_void {
    unsafe { env.handle().as_ptr().cast::<*mut c_void>().add(1).read() }
}

unsafe extern "C" fn on_runnable_thread_transition_complete(thread: *mut c_void) {
    let callback = RUNNABLE_CALLBACK.with(|slot| slot.borrow_mut().take());
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
}
