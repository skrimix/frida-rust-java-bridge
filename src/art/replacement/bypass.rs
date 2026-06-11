use std::sync::{
    Mutex, MutexGuard,
    atomic::{AtomicUsize, Ordering},
};

use super::controller::global_replacement_controller;

pub(in crate::art) static ORIGINAL_CALL_BYPASS_METHOD: AtomicUsize = AtomicUsize::new(0);
pub(in crate::art) static ORIGINAL_CALL_BYPASS_THREAD: AtomicUsize = AtomicUsize::new(0);
pub(in crate::art) static ORIGINAL_CALL_BYPASS_OWNER_THREAD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CALL_BYPASS_LOCK: Mutex<()> = Mutex::new(());

pub(crate) struct OriginalMethodCallBypass {
    _lock: Option<MutexGuard<'static, ()>>,
    previous: usize,
    previous_thread: usize,
}

pub(crate) fn original_method_call_bypass(
    method: usize,
    thread: usize,
) -> OriginalMethodCallBypass {
    let method = global_replacement_controller().map_or(method, |controller| {
        controller.art_method_for_jni_id(method)
    });
    let lock = if thread != 0 && ORIGINAL_CALL_BYPASS_OWNER_THREAD.load(Ordering::SeqCst) == thread
    {
        None
    } else {
        let lock = ORIGINAL_CALL_BYPASS_LOCK
            .lock()
            .expect("ART original-call bypass mutex poisoned");
        ORIGINAL_CALL_BYPASS_OWNER_THREAD.store(thread, Ordering::SeqCst);
        Some(lock)
    };
    let previous = ORIGINAL_CALL_BYPASS_METHOD.swap(method, Ordering::SeqCst);
    let previous_thread = ORIGINAL_CALL_BYPASS_THREAD.swap(thread, Ordering::SeqCst);
    OriginalMethodCallBypass {
        _lock: lock,
        previous,
        previous_thread,
    }
}

impl Drop for OriginalMethodCallBypass {
    fn drop(&mut self) {
        ORIGINAL_CALL_BYPASS_METHOD.store(self.previous, Ordering::SeqCst);
        ORIGINAL_CALL_BYPASS_THREAD.store(self.previous_thread, Ordering::SeqCst);
        if self._lock.is_some() {
            ORIGINAL_CALL_BYPASS_OWNER_THREAD.store(0, Ordering::SeqCst);
        }
    }
}
