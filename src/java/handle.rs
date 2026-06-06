use super::*;

impl Java {
    pub(crate) fn new(vm: Vm) -> Self {
        Self {
            vm,
            loader: None,
            classes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Obtains the current Android ART Java bridge.
    ///
    /// The returned handle is ready for JS-style `perform()` work. Low-level `find_class()` calls
    /// on it are bootstrap-scoped; high-level `use_class()` calls inside `perform()` use the app
    /// loader once it is available.
    pub fn obtain() -> Result<Self> {
        Ok(Self::new(crate::runtime::Runtime::obtain()?.vm()))
    }

    /// Returns the low-level VM handle backing this Java facade.
    ///
    /// Most callers do not need this. Use it when code deliberately works at the attachment or raw
    /// JNI boundary.
    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    /// Returns the explicit loader scope for this handle, if one was selected.
    ///
    /// `None` means low-level [`Java::find_class`] uses bootstrap lookup. High-level
    /// [`Java::use_class`] may still prefer a published default app loader on a bare handle.
    pub fn loader(&self) -> Option<&ClassLoaderRef> {
        self.loader.as_ref()
    }

    /// Enters a synchronous Java scope by attaching the current thread to this handle's VM.
    ///
    /// This is the guard-shaped form of scoped Java work. It returns a [`JavaScope`] that keeps the
    /// thread attached until the guard is dropped. If the thread is already attached this only
    /// borrows the existing `JNIEnv`; otherwise the thread is detached at the end of the scope.
    ///
    /// Most callers should use `perform()` for app-loader-scoped work or `perform_now()` when they
    /// want the same immediate scope expressed as a closure. Reach for `attach()` when code needs
    /// to hold the guard explicitly, usually because it needs direct `env()` access.
    pub fn attach(&self) -> Result<JavaScope<'_>> {
        Ok(JavaScope {
            java: self,
            env: self.vm.attach_current_thread()?,
            _thread_affine: PhantomData,
        })
    }

    /// Returns the process default app loader if it has already been published.
    ///
    /// This is a side-effect-free inspection helper. It does not query
    /// `ActivityThread.currentApplication()`, install startup hooks, or enqueue deferred work.
    pub fn default_app_loader(&self) -> Option<ClassLoaderRef> {
        AppPerformState::default_loader_global()
    }

    /// Returns a new `Java` handle that resolves classes through `loader`.
    ///
    /// The returned handle starts with an empty class cache. This keeps bootstrap, system-loader,
    /// DexClassLoader, and enumerated-loader lookups isolated even when the same binary class name
    /// is requested.
    pub fn with_loader(&self, loader: &ClassLoaderRef) -> Self {
        Self {
            vm: self.vm.clone(),
            loader: Some(loader.clone()),
            classes: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Reports runtime support for ART-dependent bridge features.
    ///
    /// Capability checks are side-effect-light: they describe whether a feature appears available
    /// without installing hooks or enqueueing work.
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
    /// This mirrors upstream `Java.deoptimizeEverything()` for Android ART. It is currently
    /// supported only when the API 26+ arm64 ART deoptimization prerequisites are available.
    pub fn deoptimize_everything(&self) -> Result<()> {
        self.vm.art().deoptimize_everything(&self.vm)
    }

    /// Requests ART boot-image deoptimization for the current process.
    ///
    /// This mirrors upstream `Java.deoptimizeBootImage()` for Android ART. It is currently
    /// supported only on the crate's API 26+ arm64 runtime milestone.
    pub fn deoptimize_boot_image(&self) -> Result<()> {
        self.vm.art().deoptimize_boot_image(&self.vm)
    }

    /// Returns the Android release string and SDK API level for this process.
    pub fn android_version(&self) -> Result<crate::AndroidVersion> {
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

    /// Returns the current Android application's class loader when an app `Application` exists.
    ///
    /// This is a synchronous app-loader selection helper. Processes or startup phases where
    /// `ActivityThread.currentApplication()` is still null return `Error::AppClassLoaderUnavailable`
    /// instead of falling back to an unrelated loader.
    pub fn app_class_loader(&self) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        app_class_loader_from_activity_thread(&env, &self.vm)
    }

    /// Returns a new `Java` handle scoped to the current Android application's class loader.
    ///
    /// This is the synchronous loader-selection primitive behind the immediate `perform()` path.
    /// If app startup has not published an `Application` yet, this returns
    /// `Error::AppClassLoaderUnavailable`; use `perform()` for JS-style deferral.
    pub fn with_app_loader(&self) -> Result<Self> {
        let loader = self.app_class_loader()?;
        AppPerformState::get(self.vm.clone()).publish_app_loader(&loader)?;
        Ok(self.with_loader(&loader))
    }

    /// Runs `callback` inside a Java scope once the app loader is available.
    ///
    /// This is the Rust equivalent of upstream `Java.perform()`. It is the only scope-entering
    /// helper here that may defer until the Android app loader exists. In ordinary app-class code,
    /// use this first and call `use_class()` inside the callback:
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
    /// runs synchronously before this method returns. Otherwise the callback is queued and
    /// ART startup hooks are installed to drain pending callbacks from Android app
    /// binding and `LoadedApk` creation paths. The returned result observes queued startup work
    /// and owns the callback's eventual value; JS-like side-effect callers may ignore it after `?`.
    ///
    /// Synchronous app-loader helpers keep returning `Error::AppClassLoaderUnavailable` while no
    /// application is available.
    pub fn perform<F, T>(&self, callback: F) -> Result<PerformResult<T>>
    where
        F: for<'scope> FnOnce(JavaScope<'scope>) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let handle = PerformHandle::new_pending();
        let value = Arc::new(Mutex::new(None));
        let callback = perform_callback_with_result(callback, value.clone());

        if let Some(loader) = self.default_app_loader() {
            complete_perform(
                PendingPerform {
                    callback,
                    state: handle.state.clone(),
                },
                self.with_loader(&loader),
            );
            return Ok(PerformResult::new(handle, value));
        }

        match self.with_app_loader() {
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
                let state = AppPerformState::get(self.vm.clone());
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
    /// This is the Rust equivalent of upstream `Java.performNow()`: the closure-shaped form of
    /// `attach()`. It is useful for bootstrap or system classes, or for code already running on an
    /// explicit loader-backed handle. Unlike `perform()`, this helper does not wait for the app
    /// class loader, enqueue work, or install startup hooks. The callback receives a `JavaScope`
    /// preserving this handle's current class-loader scope.
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

    /// Enumerates ART class loaders when the current runtime layout is supported.
    ///
    /// This is currently an Android ART API 26+ arm64 milestone feature. Unsupported layouts,
    /// missing ART symbols, and unsupported architectures return `Error::UnsupportedFeature`
    /// instead of silently falling back.
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

    /// Enumerates methods matching an upstream-inspired `class!method` query.
    ///
    /// Class patterns use Java binary names such as `java.lang.String` and
    /// `com.example.*`. Constructor methods are exposed as `$init`.
    /// Supported modifiers are `/i` for case-insensitive matching, `/s` for signature-aware
    /// matching, and `/u` for skipping bootstrap/platform classes. Signatures included by `/s`
    /// remain JNI descriptors, for example `$init(I)V`.
    pub fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        match self.vm.art().enumerate_methods(&self.vm, query) {
            Ok(groups) => Ok(groups),
            Err(Error::UnsupportedFeature {
                feature: "ART direct method enumeration",
                ..
            }) => {
                let classes = self.enumerate_loaded_raw_classes()?;
                metadata::enumerate_methods(self, &classes, query)
            }
            Err(error) => Err(error),
        }
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
                    classes.push(raw::Class::from_global(
                        self.vm.clone(),
                        handle.name,
                        global,
                    ));
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

    /// Enumerates live heap instances whose runtime class exactly matches `class_name`.
    ///
    /// `class_name` is resolved in this handle's class-loader scope. The callback receives each
    /// matching object as a temporary global reference; call `retain()` inside the callback if the
    /// object should outlive that callback invocation.
    pub fn choose_instances<F>(&self, class_name: &str, mut callback: F) -> Result<()>
    where
        F: FnMut(&JavaObject) -> Result<JavaChooseControl>,
    {
        let class = self.find_class(class_name)?;
        self.vm
            .art()
            .choose_instances(&self.vm, &class, &mut callback)
    }

    /// Finds a class in this handle's class-loader scope.
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

        let class = raw::Class::from_global(self.vm.clone(), lookup.loader_name.clone(), class);

        self.classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .insert(lookup.find_class_name, class.clone());

        Ok(class)
    }

    /// Looks up a class by its fully qualified name and returns a high-level wrapper.
    ///
    /// This is equivalent to Frida's `Java.use()`. The returned [`JavaClass`] wrapper provides access to
    /// static methods, constructors, instance members, and class metadata.
    ///
    /// ### Class Loader Context
    ///
    /// - If this `Java` handle is scoped to a specific class loader (e.g., created via [`.with_loader()`](Java::with_loader)),
    ///   it will search only within that loader's scope.
    /// - If this is a bare bootstrap handle, it will look up the class in the published application class loader
    ///   (once initialized by [`Java::perform`] or [`Java::with_app_loader`]). This matches the familiar default behavior of
    ///   upstream Frida wrappers.
    pub fn use_class(&self, name: &str) -> Result<JavaClass> {
        let java = self.wrapper_lookup_java();
        Ok(JavaClass::from_raw(java.find_class(name)?))
    }

    fn wrapper_lookup_java(&self) -> Java {
        if self.loader.is_none() {
            AppPerformState::default_java_global(&self.vm).unwrap_or_else(|| self.clone())
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
        object_from_ref(env, &self.vm, &string)
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
        array_from_ref(
            env,
            &self.vm,
            &array,
            JavaType::Object(element_class.name().replace('.', "/")),
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
        array_from_ref(env, &self.vm, &array, JavaType::Boolean)
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

    /// Returns the attached raw JNI environment for this lexical scope.
    ///
    /// Use high-level wrapper APIs unless code really needs direct JNI-style calls or references.
    pub fn env(&self) -> &AttachedEnv<'java> {
        &self.env
    }

    /// Returns the process system class loader using this scope's existing attachment.
    pub fn system_class_loader(&self) -> Result<ClassLoaderRef> {
        self.java.system_class_loader_attached(&self.env)
    }

    /// Returns the current Android app class loader using this scope's existing attachment.
    pub fn app_class_loader(&self) -> Result<ClassLoaderRef> {
        app_class_loader_from_activity_thread(&self.env, &self.java.vm)
    }

    /// Publishes and returns a Java handle scoped to the current Android app class loader.
    pub fn with_app_loader(&self) -> Result<Java> {
        let loader = self.app_class_loader()?;
        AppPerformState::get(self.java.vm.clone()).publish_app_loader(&loader)?;
        Ok(self.java.with_loader(&loader))
    }

    /// Wraps a Java object as a class-loader reference using this scope's attachment.
    pub fn class_loader_from_object(&self, object: &JavaObject) -> Result<ClassLoaderRef> {
        self.java
            .class_loader_from_object_attached(&self.env, object)
    }

    /// Finds a class in this scope's loader boundary.
    pub fn find_class(&self, name: &str) -> Result<raw::Class> {
        self.java.find_class_attached(&self.env, name)
    }

    /// Builds a high-level class wrapper in this scope's loader boundary.
    pub fn use_class(&self, name: &str) -> Result<JavaClass> {
        let java = self.java.wrapper_lookup_java();
        Ok(JavaClass::from_raw(
            java.find_class_attached(&self.env, name)?,
        ))
    }

    /// Creates a Java `java.lang.String` using this scope's attachment.
    pub fn new_string_utf(&self, text: &str) -> Result<JavaObject> {
        self.java.new_string_utf_attached(&self.env, text)
    }

    /// Creates a Java object array using this scope's attachment.
    pub fn new_object_array(
        &self,
        element_class: &raw::Class,
        elements: &[Option<&JavaObject>],
    ) -> Result<JavaArray> {
        self.java
            .new_object_array_attached(&self.env, element_class, elements)
    }

    /// Creates a Java `boolean[]` using this scope's attachment.
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

    #[test]
    fn caches_are_isolated_per_java_instance() {
        let bootstrap = Java::new(Vm::dangling_for_tests());
        let other = Java::new(Vm::dangling_for_tests());
        assert!(!Arc::ptr_eq(&bootstrap.classes, &other.classes));
        assert!(bootstrap.loader().is_none());
        assert!(other.loader().is_none());
    }
}
