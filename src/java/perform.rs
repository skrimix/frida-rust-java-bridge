use super::*;

const APP_LOADER_DEFERRED_INIT: &str = "deferred app-loader initialization";
const APP_LOADER_DEFERRED_INIT_EXPERIMENTAL: &str = "Android app-loader deferral prerequisites are available for experimental startup-hook backed Java::perform queue draining";
const MAKE_APPLICATION_SIGNATURE: &str =
    "(ZLandroid/app/Instrumentation;)Landroid/app/Application;";
const GET_PACKAGE_INFO_AI_7_SIGNATURE: &str = "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZZ)Landroid/app/LoadedApk;";
const GET_PACKAGE_INFO_AI_6_SIGNATURE: &str = "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZ)Landroid/app/LoadedApk;";
const GET_PACKAGE_INFO_AI_3_SIGNATURE: &str = "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;";
const GET_PACKAGE_INFO_STRING_3_SIGNATURE: &str =
    "(Ljava/lang/String;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;";

pub(super) type PerformCallback = Box<dyn FnOnce(Java) -> Result<()> + Send + 'static>;

pub(super) struct PendingPerform {
    pub(super) callback: PerformCallback,
    pub(super) state: Arc<Mutex<PerformStatus>>,
}

pub(super) struct AppPerformState {
    vm: Vm,
    inner: Mutex<AppPerformInner>,
}

struct AppPerformInner {
    default: Option<DefaultAppLoader>,
    pending: VecDeque<PendingPerform>,
    hooks: Option<AppPerformHooks>,
}

#[derive(Clone)]
struct DefaultAppLoader {
    loader: ClassLoaderRef,
    classes: Arc<Mutex<HashMap<String, RawJavaClass>>>,
}

struct AppPerformHooks {
    _make_application: Option<replacement::JavaHookGuard>,
    _get_package_info: Option<replacement::JavaHookGuard>,
}

pub(crate) fn app_loader_deferral_support(
    vm: &Vm,
    method_replacement: &FeatureSupport,
) -> FeatureSupport {
    match method_replacement {
        FeatureSupport::Unsupported { reason } => {
            return FeatureSupport::Unsupported {
                reason: format!("method replacement prerequisites are unavailable: {reason}"),
            };
        }
        FeatureSupport::Supported | FeatureSupport::Experimental { .. } => {}
    }

    match probe_app_loader_deferral(vm) {
        Ok(()) => FeatureSupport::Experimental {
            reason: APP_LOADER_DEFERRED_INIT_EXPERIMENTAL.to_owned(),
        },
        Err(Error::UnsupportedFeature { reason, .. }) => FeatureSupport::Unsupported { reason },
        Err(error) => FeatureSupport::Unsupported {
            reason: error.to_string(),
        },
    }
}

impl PerformHandle {
    pub(super) fn new_pending() -> Self {
        Self {
            state: Arc::new(Mutex::new(PerformStatus::Pending)),
        }
    }

    /// Returns the latest observed state of the registered callback.
    pub fn status(&self) -> PerformStatus {
        self.state
            .lock()
            .expect("perform handle state poisoned")
            .clone()
    }

    pub fn is_pending(&self) -> bool {
        matches!(self.status(), PerformStatus::Pending)
    }
}

fn probe_app_loader_deferral(vm: &Vm) -> Result<()> {
    let java = Java::new(vm.clone());
    let make_application = probe_make_application_hook_shape(&java);
    let get_package_info = java
        .find_class("android.app.ActivityThread")
        .and_then(|activity_thread| probe_get_package_info_hook_shape(&activity_thread));

    if make_application.is_ok() || get_package_info.is_ok() {
        return Ok(());
    }

    let make_application_error = make_application
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "not checked".to_owned());
    let get_package_info_error = get_package_info
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| "not checked".to_owned());
    Err(Error::UnsupportedFeature {
        feature: APP_LOADER_DEFERRED_INIT,
        reason: format!(
            "no supported LoadedApk.makeApplication or ActivityThread.getPackageInfo hook shape was found (LoadedApk: {make_application_error}; ActivityThread: {get_package_info_error})"
        ),
    })
}

fn probe_make_application_hook_shape(java: &Java) -> Result<()> {
    let loaded_apk = java.find_class("android.app.LoadedApk")?;
    match loaded_apk.resolve_instance_method("makeApplicationInner", MAKE_APPLICATION_SIGNATURE) {
        Ok(_) => Ok(()),
        Err(inner_error) => loaded_apk
            .resolve_instance_method("makeApplication", MAKE_APPLICATION_SIGNATURE)
            .map(|_| ())
            .map_err(|make_error| Error::UnsupportedFeature {
                feature: APP_LOADER_DEFERRED_INIT,
                reason: format!(
                    "LoadedApk.makeApplicationInner shape unavailable ({inner_error}); LoadedApk.makeApplication shape unavailable ({make_error})"
                ),
            }),
    }
}

fn probe_get_package_info_hook_shape(activity_thread: &RawJavaClass) -> Result<()> {
    let candidates = [
        GET_PACKAGE_INFO_AI_7_SIGNATURE,
        GET_PACKAGE_INFO_AI_6_SIGNATURE,
        GET_PACKAGE_INFO_AI_3_SIGNATURE,
        GET_PACKAGE_INFO_STRING_3_SIGNATURE,
    ];
    let mut errors = Vec::new();
    for signature in candidates {
        match activity_thread.resolve_instance_method("getPackageInfo", signature) {
            Ok(_) => return Ok(()),
            Err(error) => errors.push(format!("{signature}: {error}")),
        }
    }
    Err(Error::UnsupportedFeature {
        feature: APP_LOADER_DEFERRED_INIT,
        reason: format!(
            "no supported ActivityThread.getPackageInfo overload shape was found ({})",
            errors.join("; ")
        ),
    })
}

impl AppPerformState {
    pub(super) fn get(vm: Vm) -> &'static Self {
        APP_PERFORM_STATE.get_or_init(|| Self::new(vm))
    }

    pub(super) fn default_loader_global() -> Option<ClassLoaderRef> {
        APP_PERFORM_STATE.get().and_then(Self::default_loader)
    }

    pub(super) fn default_java_global(vm: &Vm) -> Option<Java> {
        APP_PERFORM_STATE
            .get()
            .and_then(|state| state.default_java(vm))
    }

    pub(super) fn new(vm: Vm) -> Self {
        Self {
            vm,
            inner: Mutex::new(AppPerformInner {
                default: None,
                pending: VecDeque::new(),
                hooks: None,
            }),
        }
    }

    pub(super) fn default_loader(&self) -> Option<ClassLoaderRef> {
        self.inner
            .lock()
            .expect("perform state poisoned")
            .default
            .as_ref()
            .map(|default| default.loader.clone())
    }

    pub(super) fn default_java(&self, vm: &Vm) -> Option<Java> {
        self.inner
            .lock()
            .expect("perform state poisoned")
            .default
            .as_ref()
            .map(|default| Java {
                vm: vm.clone(),
                loader: Some(default.loader.clone()),
                classes: default.classes.clone(),
            })
    }

    pub(super) fn publish_app_loader(&self, loader: &ClassLoaderRef) -> Result<()> {
        let env = self.vm.attach_current_thread()?;
        let mut inner = self.inner.lock().expect("perform state poisoned");

        if let Some(default) = &inner.default
            && env.is_same_object(&default.loader, loader)?
        {
            return Ok(());
        }

        let default = DefaultAppLoader {
            loader: loader.clone(),
            classes: Arc::new(Mutex::new(HashMap::new())),
        };
        inner.default = Some(default);
        Ok(())
    }

    pub(super) fn enqueue(&self, callback: PerformCallback, state: Arc<Mutex<PerformStatus>>) {
        let mut inner = self.inner.lock().expect("perform state poisoned");
        inner.pending.push_back(PendingPerform { callback, state });
    }

    pub(super) fn ensure_hook(&self) -> Result<()> {
        let mut inner = self.inner.lock().expect("perform state poisoned");
        if inner.hooks.is_some() {
            return Ok(());
        }

        let java = Java::new(self.vm.clone());
        let make_application_hook = install_make_application_hook(&java);
        if let Err(error) = &make_application_hook {
            println!(
                "frida-java-bridge-rs: deferred app-loader LoadedApk.makeApplication hook unavailable: {error}"
            );
        }
        let get_package_info_hook = java
            .find_class("android.app.ActivityThread")
            .and_then(|activity_thread| install_get_package_info_hook(&activity_thread));
        if let Err(error) = &get_package_info_hook {
            println!(
                "frida-java-bridge-rs: deferred app-loader ActivityThread.getPackageInfo hook unavailable: {error}"
            );
        }
        if make_application_hook.is_err() && get_package_info_hook.is_err() {
            return Err(Error::UnsupportedFeature {
                feature: APP_LOADER_DEFERRED_INIT,
                reason: "no supported LoadedApk.makeApplication or ActivityThread.getPackageInfo hook could be installed".to_owned(),
            });
        }

        inner.hooks = Some(AppPerformHooks {
            _make_application: make_application_hook.ok(),
            _get_package_info: get_package_info_hook.ok(),
        });
        Ok(())
    }

    pub(super) fn drain_if_ready(&self) {
        let java = Java::new(self.vm.clone());
        let Ok(loader) = java.app_class_loader() else {
            return;
        };
        if self.publish_app_loader(&loader).is_err() {
            return;
        }
        self.drain_with_app_java(Java::new(self.vm.clone()).with_loader(&loader));
    }

    pub(super) fn drain_with_app_java(&self, app_java: Java) {
        let mut pending = VecDeque::new();
        {
            let mut inner = self.inner.lock().expect("perform state poisoned");
            std::mem::swap(&mut pending, &mut inner.pending);
        }

        while let Some(operation) = pending.pop_front() {
            complete_perform(operation, app_java.clone());
        }
    }
}

impl Drop for AppPerformState {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.inner.lock() {
            for operation in inner.pending.drain(..) {
                set_perform_status(
                    &operation.state,
                    PerformStatus::Failed(Error::UnsupportedFeature {
                        feature: APP_LOADER_DEFERRED_INIT,
                        reason:
                            "perform queue was dropped before the app class loader became available"
                                .to_owned(),
                    }),
                );
            }
            if let Some(hooks) = inner.hooks.take() {
                // If a non-static owner ever drops this state after installing ART method
                // replacements, keep the hooks alive instead of restoring methods while ART may
                // already be shutting down. The process-global OnceLock path is never dropped.
                std::mem::forget(hooks);
            }
        }
    }
}

fn install_make_application_hook(java: &Java) -> Result<replacement::JavaHookGuard> {
    let loaded_apk = java.find_class("android.app.LoadedApk")?;
    match install_make_application_method_hook(&loaded_apk, "makeApplicationInner") {
        Ok(hook) => Ok(hook),
        Err(inner_error) => install_make_application_method_hook(&loaded_apk, "makeApplication")
            .map_err(|make_error| Error::UnsupportedFeature {
            feature: APP_LOADER_DEFERRED_INIT,
            reason: format!(
                "LoadedApk.makeApplicationInner hook failed ({inner_error}); LoadedApk.makeApplication hook failed ({make_error})"
            ),
        }),
    }
}

fn install_make_application_method_hook(
    loaded_apk: &RawJavaClass,
    name: &'static str,
) -> Result<replacement::JavaHookGuard> {
    let method = JavaMethod::from_raw_exact(
        loaded_apk,
        MethodKind::Instance,
        name,
        MAKE_APPLICATION_SIGNATURE,
    )?;
    unsafe {
        method.replace(move |invocation| {
            let application: Option<jni::jobject> =
                invocation.call_original(invocation.arguments().to_vec())?;
            if let Some(application) = application {
                drain_from_application_raw(invocation.env_raw(), application);
            }
            Ok(application)
        })
    }
}

fn install_get_package_info_hook(
    activity_thread: &RawJavaClass,
) -> Result<replacement::JavaHookGuard> {
    let candidates = [
        GET_PACKAGE_INFO_AI_7_SIGNATURE,
        GET_PACKAGE_INFO_AI_6_SIGNATURE,
        GET_PACKAGE_INFO_AI_3_SIGNATURE,
        GET_PACKAGE_INFO_STRING_3_SIGNATURE,
    ];
    let mut errors = Vec::new();
    for signature in candidates {
        match install_get_package_info_method_hook(activity_thread, signature) {
            Ok(hook) => return Ok(hook),
            Err(error) => errors.push(format!("{signature}: {error}")),
        }
    }
    Err(Error::UnsupportedFeature {
        feature: APP_LOADER_DEFERRED_INIT,
        reason: format!(
            "no supported ActivityThread.getPackageInfo overload could be hooked ({})",
            errors.join("; ")
        ),
    })
}

fn install_get_package_info_method_hook(
    activity_thread: &RawJavaClass,
    signature: &'static str,
) -> Result<replacement::JavaHookGuard> {
    let method = JavaMethod::from_raw_exact(
        activity_thread,
        MethodKind::Instance,
        "getPackageInfo",
        signature,
    )?;
    unsafe {
        method.replace(move |invocation| {
            let loaded_apk: Option<jni::jobject> =
                invocation.call_original(invocation.arguments().to_vec())?;
            if let Some(loaded_apk) = loaded_apk {
                drain_from_loaded_apk_raw(invocation.env_raw(), loaded_apk);
            }
            Ok(loaded_apk)
        })
    }
}

pub(super) fn class_loader_from_get_class_loader<T: AsJObject>(
    env: &Env<'_>,
    vm: &Vm,
    object: &T,
    operation: &'static str,
) -> Result<ClassLoaderRef> {
    let object_class = env.get_object_class(object)?;
    let get_class_loader =
        env.lookup_instance_method(&object_class, "getClassLoader", "()Ljava/lang/ClassLoader;")?;
    let loader = env
        .call_instance_object_method(object, &get_class_loader, &[])?
        .ok_or(Error::NullReturn { operation })?;

    ClassLoaderRef::from_object_ref(env, vm, &loader, ClassLoaderKind::App)
}

fn app_perform_env<'vm>(vm: &'vm Vm, env: *mut jni::JNIEnv) -> Result<Env<'vm>> {
    let env = NonNull::new(env).ok_or(Error::NullReturn {
        operation: "perform callback JNIEnv",
    })?;
    Ok(Env::from_raw(env, vm))
}

fn drain_from_application_raw(env: *mut jni::JNIEnv, application: jni::jobject) {
    if application.is_null() {
        return;
    }
    let Some(state) = APP_PERFORM_STATE.get() else {
        return;
    };
    let result: Result<()> = (|| {
        let env = app_perform_env(&state.vm, env)?;
        let application = RawObject(application);
        let loader = class_loader_from_get_class_loader(
            &env,
            &state.vm,
            &application,
            "Application.getClassLoader",
        )?;
        state.publish_app_loader(&loader)?;
        state.drain_with_app_java(Java::new(state.vm.clone()).with_loader(&loader));
        Ok(())
    })();
    if let Err(error) = result {
        println!("frida-java-bridge-rs: deferred app-loader Application drain failed: {error}");
    }
}

fn drain_from_loaded_apk_raw(env: *mut jni::JNIEnv, loaded_apk: jni::jobject) {
    if loaded_apk.is_null() {
        return;
    }
    let Some(state) = APP_PERFORM_STATE.get() else {
        return;
    };
    let result: Result<()> = (|| {
        let env = app_perform_env(&state.vm, env)?;
        let loaded_apk = RawObject(loaded_apk);
        let loader = class_loader_from_get_class_loader(
            &env,
            &state.vm,
            &loaded_apk,
            "LoadedApk.getClassLoader",
        )?;
        state.publish_app_loader(&loader)?;
        state.drain_with_app_java(Java::new(state.vm.clone()).with_loader(&loader));
        Ok(())
    })();
    if let Err(error) = result {
        println!("frida-java-bridge-rs: deferred app-loader LoadedApk drain failed: {error}");
    }
}

pub(super) fn complete_perform(operation: PendingPerform, app_java: Java) {
    let status = match (operation.callback)(app_java) {
        Ok(()) => PerformStatus::Completed,
        Err(error) => PerformStatus::Failed(error),
    };
    set_perform_status(&operation.state, status);
}

pub(super) fn set_perform_status(state: &Arc<Mutex<PerformStatus>>, status: PerformStatus) {
    *state.lock().expect("perform handle state poisoned") = status;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc as StdArc;

    #[test]
    fn perform_handle_starts_pending_and_reports_completion() {
        let handle = PerformHandle::new_pending();
        assert_eq!(handle.status(), PerformStatus::Pending);
        assert!(handle.is_pending());

        set_perform_status(&handle.state, PerformStatus::Completed);
        assert_eq!(handle.status(), PerformStatus::Completed);
        assert!(!handle.is_pending());
    }

    #[test]
    fn app_perform_state_drains_callbacks_fifo() {
        let state = AppPerformState::new(Vm::dangling_for_tests());
        let order = StdArc::new(Mutex::new(Vec::new()));
        let first = PerformHandle::new_pending();
        let second = PerformHandle::new_pending();

        let first_order = order.clone();
        state.enqueue(
            Box::new(move |_| {
                first_order.lock().unwrap().push(1);
                Ok(())
            }),
            first.state.clone(),
        );

        let second_order = order.clone();
        state.enqueue(
            Box::new(move |_| {
                second_order.lock().unwrap().push(2);
                Ok(())
            }),
            second.state.clone(),
        );

        state.drain_with_app_java(Java::new(Vm::dangling_for_tests()));

        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
        assert_eq!(first.status(), PerformStatus::Completed);
        assert_eq!(second.status(), PerformStatus::Completed);
    }

    #[test]
    fn app_perform_state_records_callback_errors() {
        let state = AppPerformState::new(Vm::dangling_for_tests());
        let handle = PerformHandle::new_pending();
        state.enqueue(
            Box::new(|_| {
                Err(Error::UnsupportedFeature {
                    feature: "test perform",
                    reason: "callback failed".to_owned(),
                })
            }),
            handle.state.clone(),
        );

        state.drain_with_app_java(Java::new(Vm::dangling_for_tests()));

        assert_eq!(
            handle.status(),
            PerformStatus::Failed(Error::UnsupportedFeature {
                feature: "test perform",
                reason: "callback failed".to_owned(),
            })
        );
    }
}
