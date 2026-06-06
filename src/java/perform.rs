use std::collections::VecDeque;

use super::*;

const APP_LOADER_DEFERRED_INIT: &str = "deferred app-loader initialization";
const MAKE_APPLICATION_SIGNATURE: &str =
    "(ZLandroid/app/Instrumentation;)Landroid/app/Application;";
const HANDLE_BIND_APPLICATION_SIGNATURE: &str = "(Landroid/app/ActivityThread$AppBindData;)V";
const APP_BIND_DATA_INSTRUMENTATION_FIELD: &str = "Landroid/content/ComponentName;";
const GET_PACKAGE_INFO_AI_7_SIGNATURE: &str = "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZZ)Landroid/app/LoadedApk;";
const GET_PACKAGE_INFO_AI_6_SIGNATURE: &str = "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;Ljava/lang/ClassLoader;ZZZ)Landroid/app/LoadedApk;";
const GET_PACKAGE_INFO_AI_3_SIGNATURE: &str = "(Landroid/content/pm/ApplicationInfo;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;";
const GET_PACKAGE_INFO_STRING_3_SIGNATURE: &str =
    "(Ljava/lang/String;Landroid/content/res/CompatibilityInfo;I)Landroid/app/LoadedApk;";

pub(super) type PerformCallback =
    Box<dyn for<'scope> FnOnce(JavaScope<'scope>) -> Result<()> + Send + 'static>;

/// Current state of a deferred app-loader operation registered through `Java::perform`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PerformStatus {
    Pending,
    Completed,
    Failed(Error),
}

/// A handle to a `Java::perform` callback.
#[derive(Clone)]
pub struct PerformHandle {
    pub(super) state: Arc<Mutex<PerformStatus>>,
}

/// A handle to a `Java::perform` callback and its eventual value.
#[derive(Clone)]
pub struct PerformResult<T> {
    handle: PerformHandle,
    value: Arc<Mutex<Option<Result<T>>>>,
}

pub(super) fn perform_callback_with_result<F, T>(
    callback: F,
    value: Arc<Mutex<Option<Result<T>>>>,
) -> PerformCallback
where
    F: for<'scope> FnOnce(JavaScope<'scope>) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    Box::new(move |java| {
        let result = callback(java);
        let status = result.as_ref().map(|_| ()).map_err(Clone::clone);
        *value.lock().expect("perform result value state poisoned") = Some(result);
        status
    })
}

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
    startup_drain: AppStartupDrain,
}

#[derive(Clone)]
struct DefaultAppLoader {
    loader: ClassLoaderRef,
    classes: Arc<Mutex<HashMap<String, raw::Class>>>,
}

struct AppPerformHooks {
    _handle_bind_application: Option<replacement::JavaHookGuard>,
    _make_application: Option<replacement::JavaHookGuard>,
    _get_package_info: Option<replacement::JavaHookGuard>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppStartupDrain {
    Waiting {
        hookpoint: AppStartupHookPoint,
    },
    Draining {
        hookpoint: AppStartupHookPoint,
        source: AppStartupDrainSource,
    },
    Initialized {
        source: AppStartupDrainSource,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppStartupHookPoint {
    Early,
    Late,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AppStartupDrainSource {
    CurrentApplication,
    Application,
    LoadedApk,
}

impl AppStartupDrain {
    fn new() -> Self {
        Self::Waiting {
            hookpoint: AppStartupHookPoint::Early,
        }
    }

    fn require_late_hookpoint(&mut self) {
        if matches!(
            self,
            Self::Waiting {
                hookpoint: AppStartupHookPoint::Early,
            }
        ) {
            *self = Self::Waiting {
                hookpoint: AppStartupHookPoint::Late,
            };
        }
    }

    fn begin(&mut self, source: AppStartupDrainSource) -> bool {
        let Self::Waiting { hookpoint } = *self else {
            return false;
        };

        if source == AppStartupDrainSource::LoadedApk && hookpoint == AppStartupHookPoint::Late {
            return false;
        }

        *self = Self::Draining { hookpoint, source };
        true
    }

    fn finish(&mut self, success: bool) {
        let Self::Draining { hookpoint, source } = *self else {
            return;
        };

        *self = if success {
            Self::Initialized { source }
        } else {
            Self::Waiting { hookpoint }
        };
    }
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
        FeatureSupport::Supported => {}
    }

    match probe_app_loader_deferral(vm) {
        Ok(()) => FeatureSupport::Supported,
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

impl<T> PerformResult<T> {
    pub(super) fn new(handle: PerformHandle, value: Arc<Mutex<Option<Result<T>>>>) -> Self {
        Self { handle, value }
    }

    /// Returns the status handle for the registered callback.
    pub fn handle(&self) -> &PerformHandle {
        &self.handle
    }

    /// Returns the latest observed state of the registered callback.
    pub fn status(&self) -> PerformStatus {
        self.handle.status()
    }

    pub fn is_pending(&self) -> bool {
        self.handle.is_pending()
    }

    /// Runs `callback` with the completed value if the callback has succeeded.
    ///
    /// This keeps owned values such as hook guards inside the perform result while allowing callers
    /// to inspect or operate on them after deferred setup completes.
    pub fn with_value<R>(&self, callback: impl FnOnce(&T) -> R) -> Option<R> {
        self.value
            .lock()
            .expect("perform result value state poisoned")
            .as_ref()
            .and_then(|result| result.as_ref().ok().map(callback))
    }

    /// Takes the callback result if the callback has run.
    ///
    /// If setup failed before the callback was entered, `status()` reports the failure while this
    /// remains empty.
    pub fn take_result(&self) -> Option<Result<T>> {
        self.value
            .lock()
            .expect("perform result value state poisoned")
            .take()
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

fn probe_get_package_info_hook_shape(activity_thread: &raw::Class) -> Result<()> {
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
                startup_drain: AppStartupDrain::new(),
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

    fn begin_startup_drain(&self, source: AppStartupDrainSource) -> bool {
        self.inner
            .lock()
            .expect("perform state poisoned")
            .startup_drain
            .begin(source)
    }

    fn finish_startup_drain(&self, success: bool) {
        self.inner
            .lock()
            .expect("perform state poisoned")
            .startup_drain
            .finish(success);
    }

    fn require_late_startup_drain(&self) {
        self.inner
            .lock()
            .expect("perform state poisoned")
            .startup_drain
            .require_late_hookpoint();
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
        let handle_bind_application_hook = java
            .find_class("android.app.ActivityThread")
            .and_then(|activity_thread| install_handle_bind_application_hook(&activity_thread));
        if let Err(error) = &handle_bind_application_hook {
            println!(
                "frida-rust-java-bridge: deferred app-loader ActivityThread.handleBindApplication hook unavailable: {error}"
            );
        }
        let make_application_hook = install_make_application_hook(&java);
        if let Err(error) = &make_application_hook {
            println!(
                "frida-rust-java-bridge: deferred app-loader LoadedApk.makeApplication hook unavailable: {error}"
            );
        }
        let get_package_info_hook = java
            .find_class("android.app.ActivityThread")
            .and_then(|activity_thread| install_get_package_info_hook(&activity_thread));
        if let Err(error) = &get_package_info_hook {
            println!(
                "frida-rust-java-bridge: deferred app-loader ActivityThread.getPackageInfo hook unavailable: {error}"
            );
        }
        if make_application_hook.is_err() && get_package_info_hook.is_err() {
            return Err(Error::UnsupportedFeature {
                feature: APP_LOADER_DEFERRED_INIT,
                reason: "no supported LoadedApk.makeApplication or ActivityThread.getPackageInfo hook could be installed".to_owned(),
            });
        }

        inner.hooks = Some(AppPerformHooks {
            _handle_bind_application: handle_bind_application_hook.ok(),
            _make_application: make_application_hook.ok(),
            _get_package_info: get_package_info_hook.ok(),
        });
        Ok(())
    }

    pub(super) fn drain_if_ready(&self) {
        if !self.begin_startup_drain(AppStartupDrainSource::CurrentApplication) {
            return;
        }
        let java = Java::new(self.vm.clone());
        let Ok(loader) = java.app_class_loader() else {
            self.finish_startup_drain(false);
            return;
        };
        if self.publish_app_loader(&loader).is_err() {
            self.finish_startup_drain(false);
            return;
        }
        self.drain_with_app_java(Java::new(self.vm.clone()).with_loader(&loader));
        self.finish_startup_drain(true);
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

fn install_handle_bind_application_hook(
    activity_thread: &raw::Class,
) -> Result<replacement::JavaHookGuard> {
    let method = JavaMethod::from_raw_exact(
        activity_thread,
        MethodKind::Instance,
        "handleBindApplication",
        HANDLE_BIND_APPLICATION_SIGNATURE,
    )?;
    method.replace(move |invocation| {
        if let Err(error) = mark_late_startup_drain_if_instrumented(&invocation) {
            println!(
                "frida-rust-java-bridge: deferred app-loader ActivityThread.handleBindApplication inspection failed: {error}"
            );
        }
        invocation.call_original_current::<()>()?;
        Ok(replacement::JavaHookReturn::void())
    })
}

fn mark_late_startup_drain_if_instrumented(
    invocation: &replacement::JavaHookContext<'_>,
) -> Result<()> {
    let Some(state) = APP_PERFORM_STATE.get() else {
        return Ok(());
    };
    let Some(data) = (unsafe { invocation.raw_arg_object(0)? }) else {
        return Ok(());
    };

    let env = invocation.env()?;
    let data = RawObject(data.as_jobject());
    let data_class = env.get_object_class(&data)?;
    let instrumentation_name = env.lookup_instance_field(
        &data_class,
        "instrumentationName",
        APP_BIND_DATA_INSTRUMENTATION_FIELD,
    )?;
    // SAFETY: `instrumentation_name` was resolved from `data_class`, the runtime class of `data`.
    if unsafe { env.get_instance_object_field(&data, &instrumentation_name)? }.is_some() {
        state.require_late_startup_drain();
    }

    Ok(())
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
    loaded_apk: &raw::Class,
    name: &'static str,
) -> Result<replacement::JavaHookGuard> {
    let method = JavaMethod::from_raw_exact(
        loaded_apk,
        MethodKind::Instance,
        name,
        MAKE_APPLICATION_SIGNATURE,
    )?;
    method.replace(move |invocation| {
        let application = unsafe {
            invocation
                .call_original_raw(invocation.raw_arguments().to_vec())?
                .into_raw_object("LoadedApk.makeApplication hook")?
        };
        if !application.is_null() {
            drain_from_application_raw(unsafe { invocation.raw_env() }, application);
            unsafe { Ok(replacement::JavaHookReturn::raw_object(application)) }
        } else {
            Ok(replacement::JavaHookReturn::null_object())
        }
    })
}

fn install_get_package_info_hook(
    activity_thread: &raw::Class,
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
    activity_thread: &raw::Class,
    signature: &'static str,
) -> Result<replacement::JavaHookGuard> {
    let method = JavaMethod::from_raw_exact(
        activity_thread,
        MethodKind::Instance,
        "getPackageInfo",
        signature,
    )?;
    method.replace(move |invocation| {
        let loaded_apk = unsafe {
            invocation
                .call_original_raw(invocation.raw_arguments().to_vec())?
                .into_raw_object("ActivityThread.getPackageInfo hook")?
        };
        if !loaded_apk.is_null() {
            drain_from_loaded_apk_raw(unsafe { invocation.raw_env() }, loaded_apk);
            unsafe { Ok(replacement::JavaHookReturn::raw_object(loaded_apk)) }
        } else {
            Ok(replacement::JavaHookReturn::null_object())
        }
    })
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
    // SAFETY: `get_class_loader` was resolved from `object`'s runtime class immediately above.
    let loader = unsafe { env.call_instance_object_method(object, &get_class_loader, &[])? }
        .ok_or(Error::NullReturn { operation })?;

    ClassLoaderRef::from_object_ref(env, vm, &loader, ClassLoaderKind::App)
}

fn app_perform_env<'vm>(vm: &'vm Vm, env: *mut jni::JNIEnv) -> Result<Env<'vm>> {
    let env = NonNull::new(env).ok_or(Error::NullReturn {
        operation: "perform callback JNIEnv",
    })?;
    Ok(Env::from_raw(env, vm.clone()))
}

fn drain_from_application_raw(env: *mut jni::JNIEnv, application: jni::jobject) {
    if application.is_null() {
        return;
    }
    let Some(state) = APP_PERFORM_STATE.get() else {
        return;
    };
    if !state.begin_startup_drain(AppStartupDrainSource::Application) {
        return;
    }
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
    match result {
        Ok(()) => state.finish_startup_drain(true),
        Err(error) => {
            state.finish_startup_drain(false);
            println!(
                "frida-rust-java-bridge: deferred app-loader Application drain failed: {error}"
            );
        }
    }
}

fn drain_from_loaded_apk_raw(env: *mut jni::JNIEnv, loaded_apk: jni::jobject) {
    if loaded_apk.is_null() {
        return;
    }
    let Some(state) = APP_PERFORM_STATE.get() else {
        return;
    };
    if !state.begin_startup_drain(AppStartupDrainSource::LoadedApk) {
        return;
    }
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
    match result {
        Ok(()) => state.finish_startup_drain(true),
        Err(error) => {
            state.finish_startup_drain(false);
            println!("frida-rust-java-bridge: deferred app-loader LoadedApk drain failed: {error}");
        }
    }
}

pub(super) fn complete_perform(operation: PendingPerform, app_java: Java) {
    let status = match app_java.attach() {
        Ok(attached) => perform_callback_status(|| (operation.callback)(attached)),
        Err(error) => PerformStatus::Failed(error),
    };
    set_perform_status(&operation.state, status);
}

fn perform_callback_status(callback: impl FnOnce() -> Result<()>) -> PerformStatus {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback)) {
        Ok(Ok(())) => PerformStatus::Completed,
        Ok(Err(error)) => PerformStatus::Failed(error),
        Err(_) => PerformStatus::Failed(Error::UnsupportedFeature {
            feature: "Java::perform callback",
            reason: "callback panicked".to_owned(),
        }),
    }
}

pub(super) fn set_perform_status(state: &Arc<Mutex<PerformStatus>>, status: PerformStatus) {
    *state.lock().expect("perform handle state poisoned") = status;
}

#[cfg(test)]
mod tests {
    use super::*;

    static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

    fn with_suppressed_panic_hook<R>(callback: impl FnOnce() -> R) -> R {
        let _guard = PANIC_HOOK_LOCK.lock().unwrap();
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(callback));
        std::panic::set_hook(previous_hook);
        match result {
            Ok(value) => value,
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

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
    fn perform_holds_and_takes_completed_value() {
        let handle = PerformHandle::new_pending();
        let value = Arc::new(Mutex::new(Some(Ok(7))));
        let result = PerformResult::new(handle.clone(), value);

        assert_eq!(result.status(), PerformStatus::Pending);
        assert_eq!(result.with_value(|value| *value + 1), Some(8));
        assert_eq!(result.take_result(), Some(Ok(7)));
        assert_eq!(result.with_value(|value| *value), None);

        set_perform_status(&handle.state, PerformStatus::Completed);
        assert_eq!(result.status(), PerformStatus::Completed);
    }

    #[test]
    fn app_perform_state_enqueues_callbacks_fifo() {
        let state = AppPerformState::new(Vm::dangling_for_tests());
        let first = PerformHandle::new_pending();
        let second = PerformHandle::new_pending();

        state.enqueue(Box::new(|_| Ok(())), first.state.clone());
        state.enqueue(Box::new(|_| Ok(())), second.state.clone());

        let inner = state.inner.lock().unwrap();
        assert_eq!(inner.pending.len(), 2);

        assert_eq!(first.status(), PerformStatus::Pending);
        assert_eq!(second.status(), PerformStatus::Pending);
    }

    #[test]
    fn startup_drain_allows_one_early_loaded_apk_path() {
        let mut drain = AppStartupDrain::new();

        assert!(drain.begin(AppStartupDrainSource::LoadedApk));
        drain.finish(true);

        assert_eq!(
            drain,
            AppStartupDrain::Initialized {
                source: AppStartupDrainSource::LoadedApk,
            }
        );
        assert!(!drain.begin(AppStartupDrainSource::Application));
        assert!(!drain.begin(AppStartupDrainSource::LoadedApk));
    }

    #[test]
    fn startup_drain_retries_after_failed_attempt() {
        let mut drain = AppStartupDrain::new();

        assert!(drain.begin(AppStartupDrainSource::LoadedApk));
        drain.finish(false);

        assert!(drain.begin(AppStartupDrainSource::Application));
        drain.finish(true);
        assert_eq!(
            drain,
            AppStartupDrain::Initialized {
                source: AppStartupDrainSource::Application,
            }
        );
    }

    #[test]
    fn startup_drain_late_hookpoint_rejects_loaded_apk_and_waits_for_application() {
        let mut drain = AppStartupDrain::new();
        drain.require_late_hookpoint();

        assert!(!drain.begin(AppStartupDrainSource::LoadedApk));
        assert!(drain.begin(AppStartupDrainSource::Application));
        drain.finish(true);
        assert_eq!(
            drain,
            AppStartupDrain::Initialized {
                source: AppStartupDrainSource::Application,
            }
        );
    }

    #[test]
    fn startup_drain_ignores_late_signal_after_initialization_or_active_drain() {
        let mut initialized = AppStartupDrain::new();
        assert!(initialized.begin(AppStartupDrainSource::LoadedApk));
        initialized.finish(true);
        initialized.require_late_hookpoint();
        assert_eq!(
            initialized,
            AppStartupDrain::Initialized {
                source: AppStartupDrainSource::LoadedApk,
            }
        );

        let mut active = AppStartupDrain::new();
        assert!(active.begin(AppStartupDrainSource::LoadedApk));
        active.require_late_hookpoint();
        active.finish(false);
        assert!(active.begin(AppStartupDrainSource::LoadedApk));
    }

    #[test]
    fn complete_perform_records_attachment_errors() {
        let handle = PerformHandle::new_pending();
        complete_perform(
            PendingPerform {
                callback: Box::new(|_| Ok(())),
                state: handle.state.clone(),
            },
            Java::new(Vm::dangling_for_tests()),
        );

        assert!(matches!(handle.status(), PerformStatus::Failed(_)));
    }

    #[test]
    fn perform_callback_status_records_panics() {
        let status = with_suppressed_panic_hook(|| {
            perform_callback_status(|| {
                panic!("intentional perform callback panic");
            })
        });

        assert_eq!(
            status,
            PerformStatus::Failed(Error::UnsupportedFeature {
                feature: "Java::perform callback",
                reason: "callback panicked".to_owned(),
            })
        );
    }
}
