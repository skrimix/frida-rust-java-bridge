use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    ops::Deref,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[cfg(test)]
use crate::capabilities::FeatureSupport;
use crate::{
    AndroidVersion,
    method_query::{
        MethodQuery, glob_matches, is_platform_class, normalize_case, parse_method_query,
        query_method_name,
    },
};
use crate::{
    capabilities::JavaCapabilities,
    env::{AttachedEnv, Env},
    error::{Error, Result},
    jni,
    loader::{ClassLoaderKind, ClassLoaderRef},
    metadata::{self, JavaMethodMetadata, JavaMethodQueryClass, JavaMethodQueryGroup},
    refs::{AsJObject, ClassKind, GlobalRef},
    signature::JavaType,
    vm::Vm,
};

use super::{
    AppPerformState, Java, JavaArray, JavaChooseControl, JavaClass, JavaObject, JavaScope,
    PendingPerform, PerformHandle, PerformResult, app_class_loader_from_activity_thread,
    app_loader_deferral_support, app_perform_state, array_from_ref_with_class, complete_perform,
    default_app_loader_global, default_java_global, find_class_with_loader,
    main_thread_scheduling_support, normalize_class_lookup_name, perform_callback_with_result, raw,
};

fn remaining_timeout(started: Instant, timeout: Duration) -> Option<Duration> {
    timeout.checked_sub(started.elapsed())
}

fn app_loader_wait_timed_out(timeout: Duration) -> Error {
    Error::AppClassLoaderWaitTimedOut {
        timeout,
        reason: "app loader was not found before the timeout elapsed".to_owned(),
    }
}

impl Java {
    pub(crate) fn new(vm: Vm) -> Self {
        Self {
            vm,
            loader: None,
            classes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Obtains the main Java bridge handle.
    ///
    /// The returned handle is your starting point for interacting with Java.
    pub fn obtain() -> Result<Self> {
        Ok(Self::new(Vm::from_runtime(
            crate::runtime::Runtime::obtain()?.into_inner(),
        )))
    }

    /// Returns the low-level VM handle backing this Java facade.
    ///
    /// You usually don't need this unless you are directly calling raw JNI functions.
    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    /// Returns the class loader this handle is scoped to, if one was selected.
    ///
    /// If no loader is set, low-level class lookups will use the system bootstrap loader.
    /// High-level methods like [`Java::use_class`] will still try to find the application's main loader.
    pub fn loader(&self) -> Option<&ClassLoaderRef> {
        self.loader.as_ref()
    }

    /// Attaches the current thread to the Java VM and returns a scope guard.
    ///
    /// The thread stays attached as long as you keep the returned [`JavaScope`]. If the thread
    /// was already attached, this only borrows the existing `JNIEnv`; otherwise the thread is
    /// detached when the scope is dropped.
    pub fn attach(&self) -> Result<JavaScope<'_>> {
        Ok(JavaScope {
            java: self,
            env: self.vm.attach_current_thread()?,
            _thread_affine: PhantomData,
        })
    }

    /// Returns the default application class loader if it has already been found.
    ///
    /// This only inspects known state and has no side effects. It does not query
    /// `ActivityThread.currentApplication()`, install startup hooks, or enqueue deferred work.
    pub fn default_app_loader(&self) -> Option<ClassLoaderRef> {
        default_app_loader_global()
    }

    /// Creates a new [`Java`] handle that uses the given class loader for lookups.
    ///
    /// The new handle gets its own empty class cache to avoid mixing up classes from different loaders.
    pub fn with_loader(&self, loader: &ClassLoaderRef) -> Self {
        Self {
            vm: self.vm.clone(),
            loader: Some(loader.clone()),
            classes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Reports which ART-dependent bridge features are supported on this device.
    ///
    /// These checks only describe whether a feature appears available. They do not install hooks or
    /// enqueue work.
    pub fn capabilities(&self) -> JavaCapabilities {
        let method_replacement = self.vm.art().method_replacement_support(&self.vm);
        let vm_handle = unsafe { self.vm.handle() };
        JavaCapabilities {
            class_loader_enumeration: self.vm.art().class_loader_enumeration_support(vm_handle),
            loaded_class_enumeration: self.vm.art().loaded_class_enumeration_support(vm_handle),
            app_loader_deferral: app_loader_deferral_support(&self.vm, &method_replacement),
            main_thread_scheduling: main_thread_scheduling_support(&self.vm),
            heap_enumeration: self.vm.art().heap_enumeration_support(vm_handle),
            deoptimization: self.vm.art().deoptimization_support(&self.vm),
            method_replacement,
        }
    }

    /// Requests full ART deoptimization for the current process.
    ///
    /// This is currently supported only when the API 26+ arm64 ART deoptimization prerequisites are available.
    pub fn deoptimize_everything(&self) -> Result<()> {
        self.vm.art().deoptimize_everything(&self.vm)
    }

    /// Requests ART boot-image deoptimization for the current process.
    ///
    /// This is currently supported only on the API 26+ arm64 runtime.
    pub fn deoptimize_boot_image(&self) -> Result<()> {
        self.vm.art().deoptimize_boot_image(&self.vm)
    }

    /// Returns the Android release string and SDK API level for this process.
    pub fn android_version(&self) -> Result<AndroidVersion> {
        crate::android::android_version()
    }

    /// Returns the Android SDK API level for this process.
    pub fn android_api_level(&self) -> Result<i32> {
        crate::android::android_api_level()
    }

    /// Returns the process system class loader.
    ///
    /// Use [`Java::with_loader`] with the returned reference to create a system-loader-scoped
    /// handle.
    pub fn system_class_loader(&self) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        self.system_class_loader_attached(&env)
    }

    fn system_class_loader_attached(&self, env: &Env<'_>) -> Result<ClassLoaderRef> {
        let class_loader_class = env.find_class("java/lang/ClassLoader")?;
        let get_system_class_loader = env.lookup_static_method(
            &class_loader_class,
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
        )?;
        // SAFETY: `get_system_class_loader` was resolved from `class_loader_class` immediately
        // above.
        let loader = unsafe {
            env.call_static_object_method(&class_loader_class, &get_system_class_loader, &[])?
        }
        .ok_or(Error::NullReturn {
            operation: "ClassLoader.getSystemClassLoader",
        })?;
        ClassLoaderRef::from_object_ref(env, &self.vm, &loader, ClassLoaderKind::System)
    }

    /// Returns the application class loader when `Application` exists.
    ///
    /// This resolves the app loader synchronously. During processes or startup phases where
    /// `ActivityThread.currentApplication()` is still null, it returns
    /// [`Error::AppClassLoaderUnavailable`].
    pub fn app_class_loader(&self) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        app_class_loader_from_activity_thread(&env, &self.vm)
    }

    /// Creates a new [`Java`] handle scoped to the application class loader.
    ///
    /// This selects the loader synchronously. If app startup has not captured an
    /// `Application` yet, it returns [`Error::AppClassLoaderUnavailable`]; use [`Java::perform()`]
    /// to defer until the app loader is known or [`Java::wait_for_app_loader`] for a blocking alternative.
    pub fn with_app_loader(&self) -> Result<Self> {
        let state = app_perform_state(self.vm.clone());
        self.with_app_loader_state(state)
    }

    fn with_app_loader_state(&self, state: &AppPerformState) -> Result<Self> {
        let env = self.vm.attach_current_thread()?;
        self.with_app_loader_attached_state(&env, state)
    }

    fn with_app_loader_attached_state(
        &self,
        env: &Env<'_>,
        state: &AppPerformState,
    ) -> Result<Self> {
        let loader = app_class_loader_from_activity_thread(env, &self.vm)?;
        state.publish_app_loader(&loader)?;
        Ok(self.with_loader(&loader))
    }

    /// Blocks until the application class loader is available.
    ///
    /// This is a synchronous helper for native helper threads and straight-line setup flows that
    /// need an app-loader-scoped [`Java`] handle before continuing. If the loader is already
    /// published, this returns immediately. If `ActivityThread.currentApplication()` is already
    /// available, this remembers that loader and returns immediately. Otherwise it installs the same
    /// deferred startup hooks used by [`Java::perform`] and waits for one of those hooks to capture
    /// the loader.
    ///
    /// Avoid calling this directly from `JNI_OnLoad`, `Agent_OnAttach`, Android main-thread startup
    /// callbacks, method replacement callbacks, or any callback whose return is needed for app
    /// startup to continue. In those contexts prefer [`Java::perform`], or spawn a background Rust
    /// thread and wait there.
    ///
    /// The timeout covers immediate probing, hook installation, and the blocking wait. A zero
    /// timeout performs only the already-known and immediate `currentApplication()` checks; it
    /// does not install deferred startup hooks.
    pub fn wait_for_app_loader(&self, timeout: Duration) -> Result<Self> {
        let state = app_perform_state(self.vm.clone());
        self.wait_for_app_loader_with_state(state, timeout)
    }

    fn wait_for_app_loader_with_state(
        &self,
        state: &AppPerformState,
        timeout: Duration,
    ) -> Result<Self> {
        let started = Instant::now();

        if let Some(app_java) = state.default_java(&self.vm) {
            return Ok(app_java);
        }

        match self.with_app_loader_state(state) {
            Ok(app_java) => return Ok(state.default_java(&self.vm).unwrap_or(app_java)),
            Err(Error::AppClassLoaderUnavailable { .. }) => {}
            Err(error) => return Err(error),
        }

        let Some(remaining) = remaining_timeout(started, timeout) else {
            return Err(app_loader_wait_timed_out(timeout));
        };
        if remaining.is_zero() {
            return Err(app_loader_wait_timed_out(timeout));
        }

        state.ensure_hook()?;
        state.drain_if_ready();

        if let Some(app_java) = state.default_java(&self.vm) {
            return Ok(app_java);
        }

        let Some(remaining) = remaining_timeout(started, timeout) else {
            return Err(app_loader_wait_timed_out(timeout));
        };
        if remaining.is_zero() {
            return Err(app_loader_wait_timed_out(timeout));
        }

        if state.wait_for_default_loader(remaining).is_some()
            && let Some(app_java) = state.default_java(&self.vm)
        {
            return Ok(app_java);
        }

        Err(app_loader_wait_timed_out(timeout))
    }

    /// Runs `callback` inside a Java scope once the app loader is available.
    ///
    /// This is equivalent to Frida's `Java.perform()`. In ordinary app-class code,
    /// use this first and call [`Java::use_class()`] inside the callback:
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::Java;
    ///
    /// let java = Java::obtain().unwrap();
    /// java.perform(|java| {
    ///     let activity = java.use_class("android.app.Activity").unwrap();
    ///     Ok(())
    /// }).unwrap();
    /// ```
    ///
    /// If `ActivityThread.currentApplication()` already exposes an application loader, the callback
    /// runs synchronously before this method returns. Otherwise, it will be queued to run
    /// automatically when `Application` appears. The returned result tracks that queued startup work
    /// and holds the callback's eventual value; callers that only want side effects may ignore it
    /// after `?`.
    pub fn perform<F, T>(&self, callback: F) -> Result<PerformResult<T>>
    where
        F: for<'scope> FnOnce(JavaScope<'scope>) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let state = app_perform_state(self.vm.clone());
        self.perform_with_state(callback, state)
    }

    fn perform_with_state<F, T>(
        &self,
        callback: F,
        state: &AppPerformState,
    ) -> Result<PerformResult<T>>
    where
        F: for<'scope> FnOnce(JavaScope<'scope>) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let handle = PerformHandle::new_pending();
        let value = Arc::new(Mutex::new(None));
        let callback = perform_callback_with_result(callback, value.clone());

        if let Some(loader) = state.default_loader() {
            complete_perform(
                PendingPerform {
                    callback,
                    state: handle.state.clone(),
                },
                self.with_loader(&loader),
            );
            return Ok(PerformResult::new(handle, value));
        }

        match self.with_app_loader_state(state) {
            Ok(app_java) => {
                complete_perform(
                    PendingPerform {
                        callback,
                        state: handle.state.clone(),
                    },
                    app_java,
                );
                Ok(PerformResult::new(handle, value))
            }
            Err(Error::AppClassLoaderUnavailable { .. }) => {
                state.ensure_hook()?;
                state.enqueue(callback, handle.state.clone());
                state.drain_if_ready();
                Ok(PerformResult::new(handle, value))
            }
            Err(error) => Err(error),
        }
    }

    /// Runs `callback` synchronously with the current thread attached to the VM.
    ///
    /// This is equivalent to Frida's `Java.performNow()`. It runs immediately and does not
    /// wait for the app to load. It's useful for working with system classes or when you already
    /// have a handle with the right class loader. Unlike [`Java::perform`], this does not wait for the app class loader,
    /// enqueue work, or install startup hooks. The callback receives a [`JavaScope`] preserving this
    /// handle's current class-loader scope.
    pub fn perform_now<F, T>(&self, callback: F) -> Result<T>
    where
        F: for<'scope> FnOnce(JavaScope<'scope>) -> Result<T>,
    {
        callback(self.attach()?)
    }

    /// Wraps a Java object as a class-loader reference after validating its runtime type.
    pub fn class_loader_from_object(&self, object: &JavaObject) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        self.class_loader_from_object_attached(&env, object)
    }

    fn class_loader_from_object_attached(
        &self,
        env: &Env<'_>,
        object: &JavaObject,
    ) -> Result<ClassLoaderRef> {
        ClassLoaderRef::from_object_ref(env, &self.vm, object, ClassLoaderKind::Object)
    }

    /// Enumerates ART class loaders.
    ///
    /// This feature requires specific Android versions and architectures (currently API 26+ arm64).
    /// If the device isn't supported, this will return [`Error::UnsupportedFeature`].
    pub fn enumerate_class_loaders(&self) -> Result<Vec<ClassLoaderRef>> {
        let env = self.vm.attach_current_thread()?;
        let handles = self.vm.art().enumerate_class_loader_handles(&self.vm)?;
        self.class_loader_refs_from_art_handles(&env, handles)
    }

    /// Enumerates loaded Java classes when the ART backend supports it.
    pub fn enumerate_loaded_classes(&self) -> Result<Vec<JavaClass>> {
        Ok(self
            .enumerate_loaded_raw_classes()?
            .into_iter()
            .map(JavaClass::from_raw)
            .collect())
    }

    /// Enumerates methods matching a Frida-style `class!method` query.
    ///
    /// You can use wildcards like `com.example.*` for classes.
    /// Constructors are named `$init`. Add `/i` for case-insensitive matching, `/s` to include
    /// JNI descriptor signatures such as `$init(I)V`, and `/u` to skip bootstrap/platform classes.
    pub fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        let query = parse_method_query(query)?;

        match self.vm.art().enumerate_methods(&self.vm, &query) {
            Ok(groups) => self.method_query_groups_from_art_groups(groups),
            Err(Error::UnsupportedFeature {
                feature: "ART direct method enumeration",
                ..
            }) => {
                let classes = self.enumerate_loaded_raw_classes()?;
                self.enumerate_methods_with_reflection(&classes, &query)
            }
            Err(error) => Err(error),
        }
    }

    fn enumerate_methods_with_reflection(
        &self,
        classes: &[raw::Class],
        query: &MethodQuery,
    ) -> Result<Vec<JavaMethodQueryGroup>> {
        let env = self.vm.attach_current_thread()?;
        let mut groups: Vec<JavaMethodQueryGroup> = Vec::new();

        for class in classes {
            let class_name = class.name();
            if query.skip_system_classes && is_platform_class(class_name) {
                continue;
            }

            let class_match_name = normalize_case(class_name, query.ignore_case);
            if !glob_matches(&query.class_pattern, &class_match_name) {
                continue;
            }

            let mut loader = None;
            if query.skip_system_classes {
                loader = metadata::class_loader(&env, &self.vm, class)?;
                if loader.is_none() {
                    continue;
                }
            }

            let mut seen = HashSet::new();
            let mut methods = Vec::new();
            for method in metadata::declared_methods(&env, class)? {
                if method.name == "<clinit>" {
                    continue;
                }
                let display_name = query_method_name(
                    method.kind,
                    &method.name,
                    &method.signature,
                    query.include_signature,
                );
                if !query.include_signature && !seen.insert(display_name.clone()) {
                    continue;
                }

                let method_match_name = normalize_case(&display_name, query.ignore_case);
                if glob_matches(&query.method_pattern, &method_match_name) {
                    methods.push(method);
                }
            }

            if methods.is_empty() {
                continue;
            }

            if loader.is_none() {
                loader = metadata::class_loader(&env, &self.vm, class)?;
            }

            let group_index = find_method_query_group(&groups, loader.as_ref());
            let class_group = JavaMethodQueryClass {
                name: class_name.to_owned(),
                methods,
            };
            if let Some(index) = group_index {
                groups[index].classes.push(class_group);
            } else {
                groups.push(JavaMethodQueryGroup {
                    loader,
                    classes: vec![class_group],
                });
            }
        }

        Ok(groups)
    }

    fn method_query_groups_from_art_groups(
        &self,
        groups: Vec<crate::art::ArtMethodQueryGroup>,
    ) -> Result<Vec<JavaMethodQueryGroup>> {
        let env = self.vm.attach_current_thread()?;
        let mut public_groups = Vec::with_capacity(groups.len());
        let mut remaining_loaders = groups
            .iter()
            .filter_map(|group| group.loader)
            .collect::<Vec<_>>();

        for group in groups {
            if let Some(raw) = group.loader
                && let Some(index) = remaining_loaders.iter().position(|loader| *loader == raw)
            {
                remaining_loaders.remove(index);
            }

            let loader = match group.loader {
                Some(raw) => match unsafe {
                    ClassLoaderRef::from_global_raw_attached(
                        &env,
                        self.vm.clone(),
                        raw,
                        ClassLoaderKind::Enumerated,
                    )
                } {
                    Ok(loader) => Some(loader),
                    Err(error) => {
                        for raw in remaining_loaders {
                            unsafe { env.delete_global_ref_raw(raw) };
                        }
                        return Err(error);
                    }
                },
                None => None,
            };
            let classes = group
                .classes
                .into_iter()
                .map(|class| JavaMethodQueryClass {
                    name: class.name,
                    methods: class
                        .methods
                        .into_iter()
                        .map(|method| JavaMethodMetadata {
                            name: method.name,
                            kind: method.kind,
                            signature: method.signature,
                            modifiers: method.modifiers,
                            id: method.id,
                        })
                        .collect(),
                })
                .collect();
            public_groups.push(JavaMethodQueryGroup { loader, classes });
        }

        Ok(public_groups)
    }

    fn enumerate_loaded_raw_classes(&self) -> Result<Vec<raw::Class>> {
        let env = self.vm.attach_current_thread()?;
        let handles = self.vm.art().enumerate_loaded_class_handles(&self.vm)?;
        self.raw_classes_from_art_handles(&env, handles)
    }

    fn class_loader_refs_from_art_handles(
        &self,
        env: &Env<'_>,
        handles: Vec<crate::art::ArtClassLoaderHandle>,
    ) -> Result<Vec<ClassLoaderRef>> {
        let mut raw_handles = handles
            .into_iter()
            .map(|handle| handle.raw)
            .collect::<Vec<_>>();
        let mut loaders = Vec::with_capacity(raw_handles.len());
        raw_handles.reverse();

        while let Some(raw) = raw_handles.pop() {
            match unsafe {
                ClassLoaderRef::from_global_raw_attached(
                    env,
                    self.vm.clone(),
                    raw,
                    ClassLoaderKind::Enumerated,
                )
            } {
                Ok(loader) => loaders.push(loader),
                Err(error) => {
                    for remaining in raw_handles {
                        unsafe { env.delete_global_ref_raw(remaining) };
                    }
                    return Err(error);
                }
            }
        }

        Ok(loaders)
    }

    fn raw_classes_from_art_handles(
        &self,
        env: &Env<'_>,
        handles: Vec<crate::art::ArtLoadedClassHandle>,
    ) -> Result<Vec<raw::Class>> {
        let mut handles = handles;
        let mut classes = Vec::with_capacity(handles.len());
        handles.reverse();

        while let Some(handle) = handles.pop() {
            match unsafe { GlobalRef::<ClassKind>::from_raw(self.vm.clone(), handle.raw) } {
                Ok(global) => {
                    classes.push(raw::Class::from_global(handle.name, global));
                }
                Err(error) => {
                    for remaining in handles {
                        unsafe { env.delete_global_ref_raw(remaining.raw) };
                    }
                    return Err(error);
                }
            }
        }

        Ok(classes)
    }

    /// Finds all live objects whose runtime class exactly matches `class_name`.
    ///
    /// `class_name` is resolved in this handle's class-loader scope. The callback receives each
    /// matching object as a temporary global reference; call [`.retain()`](`JavaObject::retain()`) inside the callback if you
    /// want to keep the object.
    pub fn choose_instances<F>(&self, class_name: &str, mut callback: F) -> Result<()>
    where
        F: FnMut(&JavaObject) -> Result<JavaChooseControl>,
    {
        let class = self.find_class(class_name)?;
        let env = self.vm.attach_current_thread()?;
        let handles = self
            .vm
            .art()
            .enumerate_heap_instance_handles(&self.vm, class.as_jobject())?;
        deliver_heap_instance_handles(&env, JavaClass::from_raw(class), handles, &mut callback)
    }

    /// Finds a class in this handle's class-loader scope.
    ///
    /// This is the low-level lookup primitive. It returns a raw class handle and uses exactly this
    /// handle's loader scope: a loader-scoped handle searches that loader, while a bare handle uses
    /// JNI `FindClass` from the current native context.
    ///
    /// Accepted names include dotted binary names (`java.lang.String`), JNI internal names
    /// (`java/lang/String`), object descriptors (`Ljava/lang/String;`), and array descriptors
    /// (`[I`, `[Ljava/lang/String;`). Bootstrap lookups use JNI internal names with
    /// `FindClass`; loader-backed lookups use binary names through `ClassLoader.loadClass()` and
    /// array descriptors through `Class.forName(name, false, loader)`.
    pub fn find_class(&self, name: &str) -> Result<raw::Class> {
        let env = self.vm.attach_current_thread()?;
        self.find_class_attached(&env, name)
    }

    pub(crate) fn find_class_attached(&self, env: &Env<'_>, name: &str) -> Result<raw::Class> {
        let lookup = normalize_class_lookup_name(name);

        if let Some(class) = self
            .classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .get(&lookup.find_class_name)
            .cloned()
        {
            return Ok(class);
        }

        let local = match &self.loader {
            Some(loader) => find_class_with_loader(env, loader, &lookup)?,
            None => env.find_class(&lookup.find_class_name)?,
        };
        let class = env.new_global_ref(&local)?;

        let class = raw::Class::from_global(lookup.loader_name.clone(), class);

        self.classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .insert(lookup.find_class_name, class.clone());

        Ok(class)
    }

    /// Looks up a class by its fully qualified name and returns a high-level wrapper.
    ///
    /// This is equivalent to Frida's `Java.use()`. The returned [`JavaClass`] wrapper lets you
    /// call static methods, create instances, access instance members, and inspect class metadata.
    /// Use this for normal Java work; use [`Java::find_class`] when you need the raw class handle.
    ///
    /// ### Class Loader Context
    ///
    /// - If this `Java` handle is scoped to a specific class loader (e.g., created via [`.with_loader()`](Java::with_loader)),
    ///   it will search only within that loader's scope.
    /// - If this is a bare bootstrap handle, it will look up the class in the known application class loader
    ///   (once initialized by [`Java::perform`] or [`Java::with_app_loader`]).
    pub fn use_class(&self, name: &str) -> Result<JavaClass> {
        let java = self.wrapper_lookup_java();
        Ok(JavaClass::from_raw(java.find_class(name)?))
    }

    fn wrapper_lookup_java(&self) -> Java {
        if self.loader.is_none() {
            default_java_global(&self.vm).unwrap_or_else(|| self.clone())
        } else {
            self.clone()
        }
    }

    /// Creates a Java `java.lang.String` object from UTF-8 Rust text.
    pub fn new_string_utf(&self, text: &str) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        self.new_string_utf_attached(&env, text)
    }

    pub(crate) fn new_string_utf_attached(&self, env: &Env<'_>, text: &str) -> Result<JavaObject> {
        let string = env.new_string_utf(text)?;
        let string_class = env.find_class("java/lang/String")?;
        let string_class = env.new_global_ref(&string_class)?;
        let string = unsafe { env.new_global_ref_raw(string.as_jobject())? };
        let string = unsafe { GlobalRef::from_raw(self.vm.clone(), string)? };
        Ok(JavaObject::from_global_ref(
            JavaClass::from_raw(raw::Class::from_global(
                "java.lang.String".to_owned(),
                string_class,
            )),
            string,
        ))
    }

    /// Creates a Java object array with nullable initial elements.
    ///
    /// `element_class` selects the declared array element type. Each non-null element must be
    /// assignable to that type according to JNI.
    pub fn new_object_array(
        &self,
        element_class: &raw::Class,
        elements: &[Option<&JavaObject>],
    ) -> Result<JavaArray> {
        let env = self.vm.attach_current_thread()?;
        self.new_object_array_attached(&env, element_class, elements)
    }

    pub(crate) fn new_object_array_attached(
        &self,
        env: &Env<'_>,
        element_class: &raw::Class,
        elements: &[Option<&JavaObject>],
    ) -> Result<JavaArray> {
        let array = env.new_object_array(
            elements.len() as jni::jsize,
            element_class,
            None::<&JavaObject>,
        )?;
        for (index, element) in elements.iter().enumerate() {
            env.set_object_array_element(&array, index as jni::jsize, *element)?;
        }
        let element_type = JavaType::Object(element_class.name().replace('.', "/"));
        let array_type = JavaType::Array(Box::new(element_type.clone()));
        let array_class = env.get_object_class(&array)?;
        let array_class = env.new_global_ref(&array_class)?;
        array_from_ref_with_class(
            env,
            JavaClass::from_raw(raw::Class::from_global(array_type.to_string(), array_class)),
            &array,
            element_type,
        )
    }

    /// Creates a Java `boolean[]` initialized from Rust values.
    pub fn new_boolean_array(&self, elements: &[bool]) -> Result<JavaArray> {
        let values = bools_to_jboolean(elements);
        let env = self.vm.attach_current_thread()?;
        self.new_boolean_array_attached(&env, &values)
    }

    pub(crate) fn new_boolean_array_attached(
        &self,
        env: &Env<'_>,
        values: &[jni::jboolean],
    ) -> Result<JavaArray> {
        let array = env.new_boolean_array(values)?;
        let array_type = JavaType::Array(Box::new(JavaType::Boolean));
        let array_class = env.get_object_class(&array)?;
        let array_class = env.new_global_ref(&array_class)?;
        array_from_ref_with_class(
            env,
            JavaClass::from_raw(raw::Class::from_global(array_type.to_string(), array_class)),
            &array,
            JavaType::Boolean,
        )
    }

    java_new_primitive_arrays! {
        new_byte_array, jni::jbyte, new_byte_array, JavaType::Byte;
        new_char_array, jni::jchar, new_char_array, JavaType::Char;
        new_short_array, jni::jshort, new_short_array, JavaType::Short;
        new_int_array, jni::jint, new_int_array, JavaType::Int;
        new_long_array, jni::jlong, new_long_array, JavaType::Long;
        new_float_array, jni::jfloat, new_float_array, JavaType::Float;
        new_double_array, jni::jdouble, new_double_array, JavaType::Double;
    }
}

impl AsRef<Java> for Java {
    fn as_ref(&self) -> &Java {
        self
    }
}

impl<'java> JavaScope<'java> {
    /// Returns the underlying Java handle for this attached scope.
    pub fn java(&self) -> &'java Java {
        self.java
    }

    /// Returns the attached raw JNI environment for this scope.
    ///
    /// Prefer the high-level wrapper APIs unless code really needs direct JNI-style calls or
    /// references.
    pub fn env(&self) -> &AttachedEnv<'java> {
        &self.env
    }

    /// Returns the process system class loader using this scope's existing attachment.
    pub fn system_class_loader(&self) -> Result<ClassLoaderRef> {
        self.java.system_class_loader_attached(&self.env)
    }

    /// Returns the current application class loader using this scope's attachment.
    pub fn app_class_loader(&self) -> Result<ClassLoaderRef> {
        app_class_loader_from_activity_thread(&self.env, &self.java.vm)
    }

    /// Creates a new `Java` handle scoped to the application class loader.
    pub fn with_app_loader(&self) -> Result<Java> {
        let state = app_perform_state(self.java.vm.clone());
        self.java.with_app_loader_attached_state(&self.env, state)
    }

    /// Wraps a Java object as a class-loader reference using this scope's attachment.
    pub fn class_loader_from_object(&self, object: &JavaObject) -> Result<ClassLoaderRef> {
        self.java
            .class_loader_from_object_attached(&self.env, object)
    }

    /// Finds a class in this handle's class-loader scope.
    ///
    /// This is the low-level lookup primitive. It returns a raw class handle and uses exactly this
    /// handle's loader scope: a loader-scoped handle searches that loader, while a bare handle uses
    /// JNI `FindClass` from the current native context.
    ///
    /// Accepted names include dotted binary names (`java.lang.String`), JNI internal names
    /// (`java/lang/String`), object descriptors (`Ljava/lang/String;`), and array descriptors
    /// (`[I`, `[Ljava/lang/String;`). Bootstrap lookups use JNI internal names with
    /// `FindClass`; loader-backed lookups use binary names through `ClassLoader.loadClass()` and
    /// array descriptors through `Class.forName(name, false, loader)`.
    pub fn find_class(&self, name: &str) -> Result<raw::Class> {
        self.java.find_class_attached(&self.env, name)
    }

    /// Looks up a class by its fully qualified name and returns a high-level wrapper.
    ///
    /// This is equivalent to Frida's `Java.use()`. The returned [`JavaClass`] wrapper lets you
    /// call static methods, create instances, access instance members, and inspect class metadata.
    /// Use this for normal Java work; use [`Java::find_class`] when you need the raw class handle.
    ///
    /// ### Class Loader Context
    ///
    /// - If this `Java` handle is scoped to a specific class loader (e.g., created via [`.with_loader()`](Java::with_loader)),
    ///   it will search only within that loader's scope.
    /// - If this is a bare bootstrap handle, it will look up the class in the known application class loader
    ///   (once initialized by [`Java::perform`] or [`Java::with_app_loader`]).
    pub fn use_class(&self, name: &str) -> Result<JavaClass> {
        let java = self.java.wrapper_lookup_java();
        Ok(JavaClass::from_raw(
            java.find_class_attached(&self.env, name)?,
        ))
    }

    /// Creates a Java `java.lang.String` from Rust text.
    pub fn new_string_utf(&self, text: &str) -> Result<JavaObject> {
        self.java.new_string_utf_attached(&self.env, text)
    }

    /// Creates a Java object array with nullable initial elements.
    ///
    /// `element_class` selects the declared array element type. Each non-null element must be
    /// assignable to that type according to JNI.
    pub fn new_object_array(
        &self,
        element_class: &raw::Class,
        elements: &[Option<&JavaObject>],
    ) -> Result<JavaArray> {
        self.java
            .new_object_array_attached(&self.env, element_class, elements)
    }

    /// Creates a Java `boolean[]` array initialized from Rust values.
    pub fn new_boolean_array(&self, elements: &[bool]) -> Result<JavaArray> {
        let values = bools_to_jboolean(elements);
        self.java.new_boolean_array_attached(&self.env, &values)
    }

    attached_java_new_primitive_arrays! {
        new_byte_array, jni::jbyte, new_byte_array, JavaType::Byte;
        new_char_array, jni::jchar, new_char_array, JavaType::Char;
        new_short_array, jni::jshort, new_short_array, JavaType::Short;
        new_int_array, jni::jint, new_int_array, JavaType::Int;
        new_long_array, jni::jlong, new_long_array, JavaType::Long;
        new_float_array, jni::jfloat, new_float_array, JavaType::Float;
        new_double_array, jni::jdouble, new_double_array, JavaType::Double;
    }
}

impl Deref for JavaScope<'_> {
    type Target = Java;

    fn deref(&self) -> &Self::Target {
        self.java
    }
}

impl AsRef<Java> for JavaScope<'_> {
    fn as_ref(&self) -> &Java {
        self.java
    }
}

pub(crate) fn deliver_heap_instance_handles(
    env: &Env<'_>,
    class: JavaClass,
    mut handles: Vec<crate::art::ArtHeapInstanceHandle>,
    callback: &mut dyn FnMut(&JavaObject) -> Result<JavaChooseControl>,
) -> Result<()> {
    handles.reverse();
    while let Some(handle) = handles.pop() {
        let object = match unsafe { GlobalRef::from_raw(class.class.vm().clone(), handle.raw) } {
            Ok(reference) => JavaObject::from_global_ref(class.clone(), reference),
            Err(error) => {
                delete_heap_instance_handles(env, handles);
                return Err(error);
            }
        };

        let control = callback(&object);
        drop(object);
        match control {
            Ok(JavaChooseControl::Continue) => {}
            Ok(JavaChooseControl::Stop) => {
                delete_heap_instance_handles(env, handles);
                return Ok(());
            }
            Err(error) => {
                delete_heap_instance_handles(env, handles);
                return Err(error);
            }
        }
    }

    Ok(())
}

fn delete_heap_instance_handles(env: &Env<'_>, handles: Vec<crate::art::ArtHeapInstanceHandle>) {
    for handle in handles {
        unsafe { env.delete_global_ref_raw(handle.raw) };
    }
}

fn find_method_query_group(
    groups: &[JavaMethodQueryGroup],
    loader: Option<&ClassLoaderRef>,
) -> Option<usize> {
    groups
        .iter()
        .position(|group| match (&group.loader, loader) {
            (None, None) => true,
            (Some(a), Some(b)) => a.as_jobject() == b.as_jobject(),
            _ => false,
        })
}

fn bools_to_jboolean(elements: &[bool]) -> Vec<jni::jboolean> {
    elements
        .iter()
        .map(|value| {
            if *value {
                jni::JNI_TRUE
            } else {
                jni::JNI_FALSE
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::java::PerformStatus;

    #[test]
    fn caches_are_isolated_per_java_instance() {
        let bootstrap = Java::new(Vm::dangling_for_tests());
        let other = Java::new(Vm::dangling_for_tests());
        assert!(!Arc::ptr_eq(&bootstrap.classes, &other.classes));
        assert!(bootstrap.loader().is_none());
        assert!(other.loader().is_none());
    }

    #[test]
    fn perform_core_accepts_local_app_state() {
        let vm = Vm::dangling_for_tests();
        let java = Java::new(vm.clone());
        let state = AppPerformState::new(vm.clone());
        let loader = unsafe { ClassLoaderRef::dangling_for_tests(vm, ClassLoaderKind::App) };
        state.set_default_loader_for_tests(loader);

        let result: PerformResult<()> = java
            .perform_with_state(
                |_| {
                    panic!("dangling test VM must fail before entering the callback");
                },
                &state,
            )
            .unwrap();

        assert!(matches!(result.status(), PerformStatus::Failed(_)));
        assert_eq!(result.take_result(), None);
        assert_eq!(state.pending_len_for_tests(), 0);
    }

    #[test]
    fn wait_for_app_loader_core_uses_published_local_state() {
        let vm = Vm::dangling_for_tests();
        let java = Java::new(vm.clone());
        let state = AppPerformState::new(vm.clone());
        let loader =
            unsafe { ClassLoaderRef::dangling_for_tests(vm.clone(), ClassLoaderKind::App) };
        state.set_default_loader_for_tests(loader);

        let app_java = java
            .wait_for_app_loader_with_state(&state, Duration::ZERO)
            .unwrap();
        let default_java = state
            .default_java(&vm)
            .expect("test state should have a default Java handle");

        assert_eq!(app_java.loader().unwrap().kind(), ClassLoaderKind::App);
        assert!(Arc::ptr_eq(&app_java.classes, &default_java.classes));
    }

    #[test]
    fn unsupported_capability_reasons_name_deferred_features() {
        let capabilities = Java::new(Vm::dangling_for_tests()).capabilities();

        assert_eq!(
            capabilities.heap_enumeration.unsupported_reason(),
            Some("Heap::VisitObjects and Heap::GetInstances are unavailable")
        );
        assert_eq!(
            capabilities.deoptimization.unsupported_reason(),
            Some("Runtime::DeoptimizeBootImage is unavailable")
        );
        assert_eq!(
            capabilities.method_replacement.unsupported_reason(),
            Some(
                "ART interpreter DoCall entrypoint is unavailable for cloned replacement dispatch"
            )
        );
        assert_eq!(
            capabilities.app_loader_deferral.unsupported_reason(),
            Some(
                "method replacement prerequisites are unavailable: ART interpreter DoCall entrypoint is unavailable for cloned replacement dispatch"
            )
        );
        assert_eq!(
            capabilities.main_thread_scheduling.unsupported_reason(),
            Some("Java VM handle is unavailable in unit tests")
        );
    }

    #[test]
    fn supported_capability_has_no_reason() {
        let support = FeatureSupport::Supported;

        assert!(support.is_supported());
        assert_eq!(support.unsupported_reason(), None);
    }
}
