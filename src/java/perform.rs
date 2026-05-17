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

fn probe_get_package_info_hook_shape(activity_thread: &JavaClass) -> Result<()> {
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
    pub(super) fn new(vm: Vm) -> Self {
        Self {
            vm,
            inner: Mutex::new(AppPerformInner {
                pending: VecDeque::new(),
                hooks: None,
            }),
        }
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
        let Ok(app_java) = java.with_app_loader() else {
            return;
        };
        self.drain_with_app_java(app_java);
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
                std::mem::forget(hooks);
            }
        }
    }
}

fn install_make_application_hook(java: &Java) -> Result<experimental::MethodReplacement> {
    let loaded_apk = java.find_class("android.app.LoadedApk")?;
    match unsafe {
        experimental::replace_instance_native_method(
            &loaded_apk,
            "makeApplicationInner",
            MAKE_APPLICATION_SIGNATURE,
            perform_make_application_inner as *const () as *mut std::ffi::c_void,
        )
    } {
        Ok(hook) => Ok(hook),
        Err(inner_error) => unsafe {
            experimental::replace_instance_native_method(
                &loaded_apk,
                "makeApplication",
                MAKE_APPLICATION_SIGNATURE,
                perform_make_application as *const () as *mut std::ffi::c_void,
            )
        }
        .map_err(|make_error| Error::UnsupportedFeature {
            feature: APP_LOADER_DEFERRED_INIT,
            reason: format!(
                "LoadedApk.makeApplicationInner hook failed ({inner_error}); LoadedApk.makeApplication hook failed ({make_error})"
            ),
        }),
    }
}

fn install_get_package_info_hook(
    activity_thread: &JavaClass,
) -> Result<experimental::MethodReplacement> {
    let candidates = [
        (
            GET_PACKAGE_INFO_AI_7_SIGNATURE,
            perform_get_package_info_ai_7 as *const () as *mut std::ffi::c_void,
        ),
        (
            GET_PACKAGE_INFO_AI_6_SIGNATURE,
            perform_get_package_info_ai_6 as *const () as *mut std::ffi::c_void,
        ),
        (
            GET_PACKAGE_INFO_AI_3_SIGNATURE,
            perform_get_package_info_ai_3 as *const () as *mut std::ffi::c_void,
        ),
        (
            GET_PACKAGE_INFO_STRING_3_SIGNATURE,
            perform_get_package_info_string_3 as *const () as *mut std::ffi::c_void,
        ),
    ];
    let mut errors = Vec::new();
    for (signature, callback) in candidates {
        match unsafe {
            experimental::replace_instance_native_method(
                activity_thread,
                "getPackageInfo",
                signature,
                callback,
            )
        } {
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

fn java_value_from_raw_object(object: jni::jobject) -> JavaValue {
    if object.is_null() {
        JavaValue::Null
    } else {
        JavaValue::Object(object)
    }
}

fn java_value_from_jboolean(value: jni::jboolean) -> JavaValue {
    JavaValue::Boolean(value != jni::JNI_FALSE)
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

unsafe extern "C" fn perform_make_application_inner(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    force_default_app_class: jni::jboolean,
    instrumentation: jni::jobject,
) -> jni::jobject {
    unsafe {
        perform_make_application_by_name(
            env,
            receiver,
            "makeApplicationInner",
            force_default_app_class,
            instrumentation,
        )
    }
}

unsafe extern "C" fn perform_make_application(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    force_default_app_class: jni::jboolean,
    instrumentation: jni::jobject,
) -> jni::jobject {
    unsafe {
        perform_make_application_by_name(
            env,
            receiver,
            "makeApplication",
            force_default_app_class,
            instrumentation,
        )
    }
}

unsafe fn perform_make_application_by_name(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    name: &str,
    force_default_app_class: jni::jboolean,
    instrumentation: jni::jobject,
) -> jni::jobject {
    let original = unsafe {
        experimental::call_original_instance_method(
            env,
            receiver,
            name,
            MAKE_APPLICATION_SIGNATURE,
            [
                java_value_from_jboolean(force_default_app_class),
                java_value_from_raw_object(instrumentation),
            ],
        )
    };
    match original.and_then(|value| value.into_object("LoadedApk.makeApplication original")) {
        Ok(application) => {
            drain_from_application_raw(env, application);
            application
        }
        Err(error) => {
            println!("frida-java-bridge-rs: deferred app-loader {name} original failed: {error}");
            ptr::null_mut()
        }
    }
}

unsafe extern "C" fn perform_get_package_info_ai_7(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    app_info: jni::jobject,
    compat_info: jni::jobject,
    base_loader: jni::jobject,
    security_violation: jni::jboolean,
    include_code: jni::jboolean,
    register_package: jni::jboolean,
    is_sdk_sandbox: jni::jboolean,
) -> jni::jobject {
    unsafe {
        perform_get_package_info(
            env,
            receiver,
            GET_PACKAGE_INFO_AI_7_SIGNATURE,
            [
                java_value_from_raw_object(app_info),
                java_value_from_raw_object(compat_info),
                java_value_from_raw_object(base_loader),
                java_value_from_jboolean(security_violation),
                java_value_from_jboolean(include_code),
                java_value_from_jboolean(register_package),
                java_value_from_jboolean(is_sdk_sandbox),
            ],
        )
    }
}

unsafe extern "C" fn perform_get_package_info_ai_6(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    app_info: jni::jobject,
    compat_info: jni::jobject,
    base_loader: jni::jobject,
    security_violation: jni::jboolean,
    include_code: jni::jboolean,
    register_package: jni::jboolean,
) -> jni::jobject {
    unsafe {
        perform_get_package_info(
            env,
            receiver,
            GET_PACKAGE_INFO_AI_6_SIGNATURE,
            [
                java_value_from_raw_object(app_info),
                java_value_from_raw_object(compat_info),
                java_value_from_raw_object(base_loader),
                java_value_from_jboolean(security_violation),
                java_value_from_jboolean(include_code),
                java_value_from_jboolean(register_package),
            ],
        )
    }
}

unsafe extern "C" fn perform_get_package_info_ai_3(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    app_info: jni::jobject,
    compat_info: jni::jobject,
    flags: jni::jint,
) -> jni::jobject {
    unsafe {
        perform_get_package_info(
            env,
            receiver,
            GET_PACKAGE_INFO_AI_3_SIGNATURE,
            [
                java_value_from_raw_object(app_info),
                java_value_from_raw_object(compat_info),
                JavaValue::Int(flags),
            ],
        )
    }
}

unsafe extern "C" fn perform_get_package_info_string_3(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    package_name: jni::jstring,
    compat_info: jni::jobject,
    flags: jni::jint,
) -> jni::jobject {
    unsafe {
        perform_get_package_info(
            env,
            receiver,
            GET_PACKAGE_INFO_STRING_3_SIGNATURE,
            [
                java_value_from_raw_object(package_name),
                java_value_from_raw_object(compat_info),
                JavaValue::Int(flags),
            ],
        )
    }
}

unsafe fn perform_get_package_info<const N: usize>(
    env: *mut jni::JNIEnv,
    receiver: jni::jobject,
    signature: &str,
    args: [JavaValue; N],
) -> jni::jobject {
    let original = unsafe {
        experimental::call_original_instance_method(
            env,
            receiver,
            "getPackageInfo",
            signature,
            args,
        )
    };
    match original.and_then(|value| value.into_object("ActivityThread.getPackageInfo original")) {
        Ok(loaded_apk) => {
            drain_from_loaded_apk_raw(env, loaded_apk);
            loaded_apk
        }
        Err(error) => {
            println!(
                "frida-java-bridge-rs: deferred app-loader getPackageInfo {signature} original failed: {error}"
            );
            ptr::null_mut()
        }
    }
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
