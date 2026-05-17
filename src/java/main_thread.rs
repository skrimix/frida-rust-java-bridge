use super::*;

const MAIN_THREAD_SCHEDULING: &str = "main-thread scheduling";
const EPOLL_WAIT: &str = "epoll_wait";
const MAIN_THREAD_SCHEDULING_EXPERIMENTAL: &str = "Android main-thread scheduling prerequisites are available for experimental queued callback dispatch through the main looper";

pub(crate) fn main_thread_scheduling_support(vm: &Vm) -> FeatureSupport {
    #[cfg(test)]
    if vm.handle() == NonNull::dangling() {
        return FeatureSupport::Unsupported {
            reason: "Java VM handle is unavailable in unit tests".to_owned(),
        };
    }

    match probe_main_thread_scheduling(vm) {
        Ok(()) => FeatureSupport::Experimental {
            reason: MAIN_THREAD_SCHEDULING_EXPERIMENTAL.to_owned(),
        },
        Err(Error::UnsupportedFeature { reason, .. }) => FeatureSupport::Unsupported { reason },
        Err(error) => FeatureSupport::Unsupported {
            reason: error.to_string(),
        },
    }
}

impl MainThreadTaskHandle {
    pub(super) fn new_pending() -> Self {
        Self {
            state: Arc::new(Mutex::new(MainThreadTaskStatus::Pending)),
        }
    }

    /// Returns the latest observed state of the scheduled callback.
    pub fn status(&self) -> MainThreadTaskStatus {
        self.state
            .lock()
            .expect("main-thread task handle state poisoned")
            .clone()
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.status(), MainThreadTaskStatus::Pending)
    }
}

impl Java {
    /// Returns whether the current thread is Android's main Java thread.
    pub fn is_main_thread(&self) -> Result<bool> {
        let looper = self.find_class("android.os.Looper")?;
        let main_looper = looper
            .call_static("getMainLooper", "()Landroid/os/Looper;", &[])?
            .into_object("Looper.getMainLooper")?;
        let current_looper = looper
            .call_static("myLooper", "()Landroid/os/Looper;", &[])?
            .into_object("Looper.myLooper")?;

        match (main_looper, current_looper) {
            (Some(main), Some(current)) => {
                let env = self.vm.attach_current_thread()?;
                env.is_same_object(&main, &current)
            }
            _ => Ok(false),
        }
    }

    /// Queues `callback` to run from Android's main thread.
    ///
    /// Scheduling always queues and wakes the main looper, matching upstream's scheduling behavior
    /// instead of running callbacks inline when the caller already happens to be on the main thread.
    /// The callback receives a clone of this `Java` handle, preserving its class-loader scope.
    pub fn schedule_on_main_thread<F>(&self, callback: F) -> Result<MainThreadTaskHandle>
    where
        F: FnOnce(Java) -> Result<()> + Send + 'static,
    {
        let handle = MainThreadTaskHandle::new_pending();
        let state = main_thread_state(self.vm.clone());
        state.ensure_hook()?;
        state.enqueue(self.clone(), Box::new(callback), handle.state.clone());

        if let Err(error) = wake_main_thread(self) {
            set_main_thread_task_status(&handle.state, MainThreadTaskStatus::Failed(error.clone()));
            return Err(error);
        }

        Ok(handle)
    }
}

fn probe_main_thread_scheduling(vm: &Vm) -> Result<()> {
    if frida_gum::Module::find_global_export_by_name(EPOLL_WAIT).is_none() {
        return Err(Error::UnsupportedFeature {
            feature: MAIN_THREAD_SCHEDULING,
            reason: "libc epoll_wait export was not found".to_owned(),
        });
    }

    let java = Java::new(vm.clone());
    let looper = java.find_class("android.os.Looper")?;
    let main_looper = looper
        .call_static("getMainLooper", "()Landroid/os/Looper;", &[])?
        .into_object("Looper.getMainLooper")?
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: MAIN_THREAD_SCHEDULING,
            reason: "Looper.getMainLooper() returned null".to_owned(),
        })?;

    let handler = java.find_class("android.os.Handler")?;
    handler.resolve_constructor("(Landroid/os/Looper;)V")?;
    handler.resolve_instance_method("sendEmptyMessage", "(I)Z")?;

    drop(main_looper);
    Ok(())
}

impl MainThreadState {
    pub(super) fn new(vm: Vm) -> Self {
        let main_thread_id = {
            let process = frida_gum::Process::obtain(vm.gum());
            process.id()
        };
        Self {
            vm,
            main_thread_id,
            inner: Mutex::new(MainThreadInner {
                pending: VecDeque::new(),
                hooks: None,
            }),
        }
    }

    pub(super) fn enqueue(
        &self,
        java: Java,
        callback: MainThreadCallback,
        state: Arc<Mutex<MainThreadTaskStatus>>,
    ) {
        let mut inner = self.inner.lock().expect("main-thread state poisoned");
        inner.pending.push_back(PendingMainThreadTask {
            java,
            callback,
            state,
        });
    }

    pub(super) fn ensure_hook(&self) -> Result<()> {
        let mut inner = self.inner.lock().expect("main-thread state poisoned");
        if inner.hooks.is_some() {
            return Ok(());
        }

        let epoll_wait =
            frida_gum::Module::find_global_export_by_name(EPOLL_WAIT).ok_or_else(|| {
                Error::UnsupportedFeature {
                    feature: MAIN_THREAD_SCHEDULING,
                    reason: "libc epoll_wait export was not found".to_owned(),
                }
            })?;

        let mut interceptor = frida_gum::interceptor::Interceptor::obtain(self.vm.gum());
        let mut listener = Box::new(MainThreadPollListener);
        let listener_handle =
            interceptor
                .attach(epoll_wait, listener.as_mut())
                .map_err(|error| Error::UnsupportedFeature {
                    feature: MAIN_THREAD_SCHEDULING,
                    reason: format!("unable to hook epoll_wait: {error}"),
                })?;
        inner.hooks = Some(MainThreadHooks {
            _interceptor: interceptor,
            _listener_handle: listener_handle,
            _listener: listener,
        });
        Ok(())
    }

    pub(super) fn drain_if_main_thread(&self, thread_id: u32) {
        if thread_id != self.main_thread_id {
            return;
        }

        let mut pending = VecDeque::new();
        {
            let mut inner = self.inner.lock().expect("main-thread state poisoned");
            std::mem::swap(&mut pending, &mut inner.pending);
        }

        while let Some(task) = pending.pop_front() {
            if !matches!(
                task.state
                    .lock()
                    .expect("main-thread task state poisoned")
                    .clone(),
                MainThreadTaskStatus::Pending
            ) {
                continue;
            }

            let status = match (task.callback)(task.java) {
                Ok(()) => MainThreadTaskStatus::Completed,
                Err(error) => MainThreadTaskStatus::Failed(error),
            };
            set_main_thread_task_status(&task.state, status);
        }
    }
}

impl frida_gum::interceptor::InvocationListener for MainThreadPollListener {
    fn on_enter(&mut self, context: frida_gum::interceptor::InvocationContext<'_>) {
        if let Some(state) = MAIN_THREAD_STATE.get() {
            state.drain_if_main_thread(context.thread_id());
        }
    }

    fn on_leave(&mut self, _context: frida_gum::interceptor::InvocationContext<'_>) {}
}

// Gum hook handles are process-global native objects kept behind the scheduler mutex. They are not
// moved after installation, and the listener only reaches back into the process-global queue.
unsafe impl Send for MainThreadHooks {}

pub(super) fn main_thread_state(vm: Vm) -> &'static MainThreadState {
    MAIN_THREAD_STATE.get_or_init(|| MainThreadState::new(vm))
}

pub(super) fn set_main_thread_task_status(
    state: &Arc<Mutex<MainThreadTaskStatus>>,
    status: MainThreadTaskStatus,
) {
    *state.lock().expect("main-thread task state poisoned") = status;
}

fn wake_main_thread(java: &Java) -> Result<()> {
    let looper = java.find_class("android.os.Looper")?;
    let main_looper = looper
        .call_static("getMainLooper", "()Landroid/os/Looper;", &[])?
        .into_object("Looper.getMainLooper")?
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: MAIN_THREAD_SCHEDULING,
            reason: "Looper.getMainLooper() returned null".to_owned(),
        })?;

    let handler_class = java.find_class("android.os.Handler")?;
    let handler =
        handler_class.new_object("(Landroid/os/Looper;)V", &[JavaValue::from(&main_looper)])?;
    let delivered = handler_class
        .call_method(&handler, "sendEmptyMessage", "(I)Z", &[JavaValue::Int(1)])?
        .into_boolean("Handler.sendEmptyMessage")?;
    if delivered {
        Ok(())
    } else {
        Err(Error::UnsupportedFeature {
            feature: MAIN_THREAD_SCHEDULING,
            reason: "Handler.sendEmptyMessage(1) returned false".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_state(main_thread_id: u32) -> MainThreadState {
        MainThreadState {
            vm: Vm::dangling_for_tests(),
            main_thread_id,
            inner: Mutex::new(MainThreadInner {
                pending: VecDeque::new(),
                hooks: None,
            }),
        }
    }

    #[test]
    fn main_thread_task_handle_starts_pending_and_reports_completion() {
        let handle = MainThreadTaskHandle::new_pending();
        assert_eq!(handle.status(), MainThreadTaskStatus::Pending);
        assert!(handle.is_pending());

        set_main_thread_task_status(&handle.state, MainThreadTaskStatus::Completed);
        assert_eq!(handle.status(), MainThreadTaskStatus::Completed);
        assert!(!handle.is_pending());
    }

    #[test]
    fn main_thread_state_drains_callbacks_fifo() {
        let state = test_state(7);
        let order = Arc::new(Mutex::new(Vec::new()));
        let first = MainThreadTaskHandle::new_pending();
        let second = MainThreadTaskHandle::new_pending();

        let first_order = order.clone();
        state.enqueue(
            Java::new(Vm::dangling_for_tests()),
            Box::new(move |java| {
                assert!(java.loader().is_none());
                first_order.lock().unwrap().push(1);
                Ok(())
            }),
            first.state.clone(),
        );

        let second_order = order.clone();
        state.enqueue(
            Java::new(Vm::dangling_for_tests()),
            Box::new(move |java| {
                assert!(java.loader().is_none());
                second_order.lock().unwrap().push(2);
                Ok(())
            }),
            second.state.clone(),
        );

        state.drain_if_main_thread(7);

        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
        assert_eq!(first.status(), MainThreadTaskStatus::Completed);
        assert_eq!(second.status(), MainThreadTaskStatus::Completed);
    }

    #[test]
    fn main_thread_state_records_callback_errors() {
        let state = test_state(7);
        let handle = MainThreadTaskHandle::new_pending();

        state.enqueue(
            Java::new(Vm::dangling_for_tests()),
            Box::new(|_| {
                Err(Error::UnsupportedFeature {
                    feature: "test main-thread scheduling",
                    reason: "callback failed".to_owned(),
                })
            }),
            handle.state.clone(),
        );

        state.drain_if_main_thread(7);

        assert_eq!(
            handle.status(),
            MainThreadTaskStatus::Failed(Error::UnsupportedFeature {
                feature: "test main-thread scheduling",
                reason: "callback failed".to_owned(),
            })
        );
    }

    #[test]
    fn main_thread_state_does_not_drain_other_threads() {
        let state = test_state(7);
        let handle = MainThreadTaskHandle::new_pending();
        let ran = Arc::new(Mutex::new(false));
        let ran_for_callback = ran.clone();

        state.enqueue(
            Java::new(Vm::dangling_for_tests()),
            Box::new(move |_| {
                *ran_for_callback.lock().unwrap() = true;
                Ok(())
            }),
            handle.state.clone(),
        );

        state.drain_if_main_thread(8);

        assert_eq!(handle.status(), MainThreadTaskStatus::Pending);
        assert!(!*ran.lock().unwrap());
    }
}
