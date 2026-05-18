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
    /// The returned handle is bootstrap-scoped. Use `with_app_loader()`, `with_loader()`, or
    /// `perform()` for app-loader-scoped class lookup.
    pub fn obtain() -> Result<Self> {
        Ok(Self::new(crate::runtime::Runtime::obtain()?.vm()))
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn loader(&self) -> Option<&ClassLoaderRef> {
        self.loader.as_ref()
    }

    /// Returns the process default app loader if it has already been published.
    ///
    /// This is a side-effect-free inspection helper. It does not query
    /// `ActivityThread.currentApplication()`, install startup hooks, or enqueue deferred work.
    pub fn default_app_loader(&self) -> Option<ClassLoaderRef> {
        default_app_loader()
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

    pub fn capabilities(&self) -> JavaCapabilities {
        self.vm.capabilities()
    }

    pub fn android_version(&self) -> Result<crate::AndroidVersion> {
        crate::android::android_version()
    }

    pub fn android_api_level(&self) -> Result<jni::jint> {
        crate::android::android_api_level()
    }

    pub fn system_class_loader(&self) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        let class_loader_class = env.find_class("java/lang/ClassLoader")?;
        let get_system_class_loader = env.lookup_static_method(
            &class_loader_class,
            "getSystemClassLoader",
            "()Ljava/lang/ClassLoader;",
        )?;
        let loader = env
            .call_static_object_method(&class_loader_class, &get_system_class_loader, &[])?
            .ok_or(Error::NullReturn {
                operation: "ClassLoader.getSystemClassLoader",
            })?;
        ClassLoaderRef::from_object_ref(&env, &self.vm, &loader, ClassLoaderKind::System)
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
    pub fn with_app_loader(&self) -> Result<Self> {
        let loader = self.app_class_loader()?;
        app_perform_state(self.vm.clone()).publish_app_loader(&loader)?;
        Ok(self.with_loader(&loader))
    }

    /// Runs `callback` with an app-loader-scoped `Java` handle once the app loader is available.
    ///
    /// If `ActivityThread.currentApplication()` already exposes an application loader, the callback
    /// runs synchronously before this method returns. Otherwise the callback is queued and
    /// experimental ART startup hooks are installed to drain pending callbacks from Android app
    /// binding and `LoadedApk` creation paths. Synchronous app-loader helpers keep returning
    /// `Error::AppClassLoaderUnavailable` while no application is available.
    pub fn perform<F>(&self, callback: F) -> Result<PerformHandle>
    where
        F: FnOnce(Java) -> Result<()> + Send + 'static,
    {
        let handle = PerformHandle::new_pending();
        if let Some(loader) = self.default_app_loader() {
            complete_perform(
                PendingPerform {
                    callback: Box::new(callback),
                    state: handle.state.clone(),
                },
                self.with_loader(&loader),
            );
            return Ok(handle);
        }

        match self.with_app_loader() {
            Ok(app_java) => {
                complete_perform(
                    PendingPerform {
                        callback: Box::new(callback),
                        state: handle.state.clone(),
                    },
                    app_java,
                );
                Ok(handle)
            }
            Err(Error::AppClassLoaderUnavailable { .. }) => {
                let state = app_perform_state(self.vm.clone());
                state.ensure_hook()?;
                state.enqueue(Box::new(callback), handle.state.clone());
                state.drain_if_ready();
                Ok(handle)
            }
            Err(error) => Err(error),
        }
    }

    /// Runs `callback` synchronously with the current thread attached to the VM.
    ///
    /// Unlike `perform()`, this helper does not wait for the app class loader, enqueue work, or
    /// install startup hooks. The callback receives a clone of this `Java` handle, preserving its
    /// current class-loader scope.
    pub fn perform_now<F, T>(&self, callback: F) -> Result<T>
    where
        F: FnOnce(Java) -> Result<T>,
    {
        let _env = self.vm.attach_current_thread()?;
        callback(self.clone())
    }

    /// Wraps a Java object as a class-loader reference after validating its runtime type.
    pub fn class_loader_from_object(&self, object: &JavaObject) -> Result<ClassLoaderRef> {
        let env = self.vm.attach_current_thread()?;
        ClassLoaderRef::from_java_object(&env, &self.vm, object, ClassLoaderKind::Object)
    }

    /// Enumerates ART class loaders when the current runtime layout is supported.
    ///
    /// This is currently an Android ART API 26+ arm64 milestone feature. Unsupported layouts,
    /// missing ART symbols, and unsupported architectures return `Error::UnsupportedFeature`
    /// instead of silently falling back.
    pub fn enumerate_class_loaders(&self) -> Result<Vec<ClassLoaderRef>> {
        self.vm.enumerate_class_loaders()
    }

    /// Enumerates loaded Java classes when the ART backend supports it.
    pub fn enumerate_loaded_classes(&self) -> Result<Vec<JavaClass>> {
        self.vm.enumerate_loaded_classes()
    }

    /// Enumerates methods matching an upstream-inspired `class!method` query.
    ///
    /// Class patterns use Java binary names such as `java.lang.String` and
    /// `com.example.*`. Constructor methods are exposed as `$init`.
    /// Supported modifiers are `/i` for case-insensitive matching, `/s` for signature-aware
    /// matching, and `/u` for skipping bootstrap/platform classes. Signatures included by `/s`
    /// remain JNI descriptors, for example `$init(I)V`.
    pub fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        match self.vm.enumerate_methods(query) {
            Ok(groups) => Ok(groups),
            Err(Error::UnsupportedFeature {
                feature: "ART direct method enumeration",
                ..
            }) => {
                let classes = self.enumerate_loaded_classes()?;
                metadata::enumerate_methods(self, &classes, query)
            }
            Err(error) => Err(error),
        }
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
        self.vm.choose_instances(&class, &mut callback)
    }

    /// Finds a class in this handle's class-loader scope.
    ///
    /// Accepted names include dotted binary names (`java.lang.String`), JNI internal names
    /// (`java/lang/String`), object descriptors (`Ljava/lang/String;`), and array descriptors
    /// (`[I`, `[Ljava/lang/String;`). Bootstrap lookups use JNI internal names with
    /// `FindClass`; loader-backed lookups use binary names through `ClassLoader.loadClass()` and
    /// array descriptors through `Class.forName(name, false, loader)`.
    pub fn find_class(&self, name: &str) -> Result<JavaClass> {
        let env = self.vm.attach_current_thread()?;
        let lookup = normalize_class_lookup_name(name);

        if let Some(class) = self
            .classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .get(&lookup.cache_key)
            .cloned()
        {
            return Ok(class);
        }

        let local = match &self.loader {
            Some(loader) => find_class_with_loader(&env, loader, &lookup)?,
            None => env.find_class(&lookup.find_class_name)?,
        };
        let class = env.new_global_ref(&local)?;

        let class = JavaClass {
            inner: Arc::new(JavaClassInner {
                vm: self.vm.clone(),
                name: lookup.public_name,
                class,
                methods: Mutex::new(HashMap::new()),
                fields: Mutex::new(HashMap::new()),
            }),
        };

        self.classes
            .lock()
            .expect("Java class cache mutex poisoned")
            .insert(lookup.cache_key, class.clone());

        Ok(class)
    }

    /// Builds a Java.use-style class wrapper in this handle's class-loader scope.
    ///
    /// The wrapper exposes reflection-backed member metadata and explicit overload invocation on
    /// top of `JavaClass`. Explicit loader-backed handles preserve their loader boundary. A bare
    /// bootstrap handle prefers the published default app loader once `Java::perform()` or
    /// `Java::with_app_loader()` has initialized it, matching upstream's wrapper default while
    /// leaving `find_class()` as the low-level bootstrap lookup primitive.
    pub fn use_class(&self, name: &str) -> Result<JavaClassWrapper> {
        let java = if self.loader.is_none() {
            default_app_java(&self.vm).unwrap_or_else(|| self.clone())
        } else {
            self.clone()
        };
        Ok(JavaClassWrapper::new(java.find_class(name)?))
    }

    pub fn new_string_utf(&self, text: &str) -> Result<JavaObject> {
        let env = self.vm.attach_current_thread()?;
        let string = env.new_string_utf(text)?;
        object_from_ref(&env, &self.vm, &string)
    }

    pub fn new_object_array(
        &self,
        element_class: &JavaClass,
        elements: &[Option<&JavaObject>],
    ) -> Result<JavaArray> {
        let env = self.vm.attach_current_thread()?;
        let array = env.new_object_array(
            elements.len() as jni::jsize,
            element_class,
            None::<&JavaObject>,
        )?;
        for (index, element) in elements.iter().enumerate() {
            env.set_object_array_element(&array, index as jni::jsize, *element)?;
        }
        array_from_ref(
            &env,
            &self.vm,
            &array,
            JavaType::Object(element_class.name().replace('.', "/")),
        )
    }

    pub fn new_boolean_array(&self, elements: &[bool]) -> Result<JavaArray> {
        let values = elements
            .iter()
            .map(|value| {
                if *value {
                    jni::JNI_TRUE
                } else {
                    jni::JNI_FALSE
                }
            })
            .collect::<Vec<_>>();
        let env = self.vm.attach_current_thread()?;
        let array = env.new_boolean_array(&values)?;
        array_from_ref(&env, &self.vm, &array, JavaType::Boolean)
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
