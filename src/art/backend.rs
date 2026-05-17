use super::*;
use super::{enumeration::*, layout::*, support::*};

impl ArtBackend {
    pub(crate) fn from_module(module: &Module, android_runtime: Option<ArtModuleRange>) -> Self {
        Self {
            android_runtime,
            add_global_ref: resolve_any(module, &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER]),
            suspend_all: resolve_suspend_all(module),
            resume_all: resolve(module, RESUME_ALL),
            visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
            visit_classes: resolve_visit_classes(module),
            get_class_descriptor: resolve(module, GET_CLASS_DESCRIPTOR),
            pretty_method: resolve_pretty_method(module),
            decode_method_id: resolve(module, DECODE_METHOD_ID),
            set_jni_id_type: resolve_pointer(module, SET_JNI_ID_TYPE),
            is_quick_resolution_stub: resolve(module, IS_QUICK_RESOLUTION_STUB),
            is_quick_to_interpreter_bridge: resolve(module, IS_QUICK_TO_INTERPRETER_BRIDGE),
            is_quick_generic_jni_stub: resolve(module, IS_QUICK_GENERIC_JNI_STUB),
            exception_clear: resolve_pointer(module, JNI_EXCEPTION_CLEAR),
            fatal_error: resolve_pointer(module, JNI_FATAL_ERROR),
            runnable_thread: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::new(module)),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            android_runtime: None,
            add_global_ref: None,
            suspend_all: None,
            resume_all: None,
            visit_class_loaders: None,
            visit_classes: None,
            get_class_descriptor: None,
            pretty_method: None,
            decode_method_id: None,
            set_jni_id_type: None,
            is_quick_resolution_stub: None,
            is_quick_to_interpreter_bridge: None,
            is_quick_generic_jni_stub: None,
            exception_clear: None,
            fatal_error: None,
            runnable_thread: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::empty_for_tests()),
        }
    }

    pub(crate) fn enumerate_class_loaders(&self, vm: &Vm) -> Result<Vec<ClassLoaderRef>> {
        self.ensure_class_loader_enumeration_supported(vm.handle())?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm.handle(), FEATURE_CLASS_LOADER_ENUMERATION)
            .expect("runtime layout support checked before class-loader enumeration");
        let mut loader_globals = Vec::new();

        self.with_runnable_art_thread(&env, FEATURE_CLASS_LOADER_ENUMERATION, |thread| {
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before enumeration");
            let visit_class_loaders = self
                .visit_class_loaders
                .expect("visit_class_loaders symbol checked before enumeration");
            let mut loader_objects = Vec::new();
            let mut visitor = ArtClassLoaderVisitor::new(&mut loader_objects);
            visitor.initialize_vtable();

            let _suspended = SuspendedAllThreads::new(
                self.suspend_all
                    .expect("suspend_all symbol checked before enumeration"),
                self.resume_all
                    .expect("resume_all symbol checked before enumeration"),
                layout.thread_list,
            );

            // SAFETY: All pointers were resolved from ART, the current thread is in runnable
            // state for ART internal object access, and all ART threads are suspended while the
            // class-linker visitor walks loader objects.
            unsafe {
                visit_class_loaders(layout.class_linker, &mut visitor);
            }

            let vm_handle = vm.handle().as_ptr();
            for loader in visitor.take_loaders() {
                // SAFETY: `loader` is an ART mirror::ClassLoader object delivered by
                // VisitClassLoaders for this VM. AddGlobalRef turns it into a JNI global handle.
                let global = unsafe { add_global_ref(vm_handle, thread, loader) };
                if global.is_null() {
                    return Err(Error::NullReturn {
                        operation: "JavaVMExt::AddGlobalRef",
                    });
                }

                loader_globals.push(global);
            }

            Ok(())
        })?;

        loader_globals
            .into_iter()
            .map(|loader| unsafe {
                ClassLoaderRef::from_global_raw(
                    vm.clone(),
                    loader,
                    crate::java::ClassLoaderKind::Enumerated,
                )
            })
            .collect()
    }

    pub(crate) fn enumerate_loaded_classes(&self, vm: &Vm) -> Result<Vec<JavaClass>> {
        self.ensure_loaded_class_enumeration_supported(vm.handle())?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm.handle(), FEATURE_LOADED_CLASS_ENUMERATION)
            .expect("runtime layout support checked before loaded-class enumeration");
        let mut class_globals = Vec::new();

        self.with_runnable_art_thread(&env, FEATURE_LOADED_CLASS_ENUMERATION, |thread| {
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before class enumeration");
            let visit_classes = self
                .visit_classes
                .expect("visit_classes symbol checked before class enumeration");
            let get_class_descriptor = self
                .get_class_descriptor
                .expect("get_class_descriptor symbol checked before class enumeration");
            let mut processor = ArtClassProcessor::new(
                add_global_ref,
                get_class_descriptor,
                vm,
                thread,
                &mut class_globals,
            );

            match visit_classes {
                VisitClassesKind::Visitor(visit_classes) => {
                    let mut visitor = ArtClassVisitor::new_loaded(&mut processor);
                    visitor.initialize_vtable();
                    unsafe { visit_classes(layout.class_linker, &mut visitor) };
                    processor.take_error()?;
                }
                VisitClassesKind::Callback(visit_classes) => unsafe {
                    visit_classes(
                        layout.class_linker,
                        on_visit_class_callback,
                        (&mut processor as *mut ArtClassProcessor<'_>).cast(),
                    );
                    processor.take_error()?;
                },
            }

            Ok(())
        })?;

        let mut classes = Vec::with_capacity(class_globals.len());
        while let Some(raw_class) = class_globals.pop() {
            let raw = raw_class.raw;
            match java_class_from_loaded(vm, raw_class) {
                Ok(class) => classes.push(class),
                Err(error) => {
                    unsafe { env.delete_global_ref_raw(raw) };
                    for remaining in class_globals {
                        unsafe { env.delete_global_ref_raw(remaining.raw) };
                    }
                    return Err(error);
                }
            }
        }
        classes.reverse();
        Ok(classes)
    }

    pub(crate) fn enumerate_methods(
        &self,
        vm: &Vm,
        query: &str,
    ) -> Result<Vec<metadata::JavaMethodQueryGroup>> {
        let query = metadata::parse_method_query(query)?;
        self.ensure_method_query_supported(vm.handle())?;

        let env = vm.attach_current_thread()?;
        let runtime_layout = detect_runtime_layout(vm.handle(), FEATURE_METHOD_QUERY)
            .expect("runtime layout support checked before ART method query");
        let memory = MemoryRanges::current()?;

        let thread_class = env.find_class("java/lang/Thread")?;
        let thread_get_name =
            env.lookup_instance_method(&thread_class, "getName", "()Ljava/lang/String;")?;
        let thread_is_alive = env.lookup_instance_method(&thread_class, "isAlive", "()Z")?;
        let thread_current_thread =
            env.lookup_static_method(&thread_class, "currentThread", "()Ljava/lang/Thread;")?;
        let system_class = env.find_class("java/lang/System")?;
        let system_current_time_millis =
            env.lookup_static_method(&system_class, "currentTimeMillis", "()J")?;

        let mut raw_groups = Vec::new();
        let query_result = self.with_runnable_art_thread(&env, FEATURE_METHOD_QUERY, |thread| {
            let visit_classes = self
                .visit_classes
                .expect("visit_classes symbol checked before method query");
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before method query");
            let get_class_descriptor = self
                .get_class_descriptor
                .expect("get_class_descriptor symbol checked before method query");
            let pretty_method = self
                .pretty_method
                .clone()
                .expect("pretty_method symbol checked before method query");

            let thread_method = self.art_method_from_jni_id(&runtime_layout, thread_get_name.raw());
            let thread_is_alive_method =
                self.art_method_from_jni_id(&runtime_layout, thread_is_alive.raw());
            let thread_current_thread_method =
                self.art_method_from_jni_id(&runtime_layout, thread_current_thread.raw());
            let process_method =
                self.art_method_from_jni_id(&runtime_layout, system_current_time_millis.raw());
            let method_layout = detect_method_query_layout(
                visit_classes,
                runtime_layout.class_linker,
                get_class_descriptor,
                &[
                    thread_method,
                    thread_is_alive_method,
                    thread_current_thread_method,
                ],
                process_method,
                &memory,
            )?;

            let mut processor = ArtMethodQueryProcessor::new(
                add_global_ref,
                get_class_descriptor,
                pretty_method,
                vm,
                thread,
                &query,
                method_layout,
                &memory,
                &mut raw_groups,
            );

            match visit_classes {
                VisitClassesKind::Visitor(visit_classes) => {
                    let mut visitor = ArtClassVisitor::new_method_query(&mut processor);
                    visitor.initialize_vtable();
                    unsafe { visit_classes(runtime_layout.class_linker, &mut visitor) };
                    processor.take_error()?;
                }
                VisitClassesKind::Callback(visit_classes) => unsafe {
                    visit_classes(
                        runtime_layout.class_linker,
                        on_visit_method_query_callback,
                        (&mut processor as *mut ArtMethodQueryProcessor<'_>).cast(),
                    );
                    processor.take_error()?;
                },
            }

            Ok(())
        });
        if let Err(error) = query_result {
            for raw in raw_groups.iter().filter_map(|group| group.loader) {
                unsafe { env.delete_global_ref_raw(raw) };
            }
            return Err(error);
        }

        raw_method_groups_to_public(vm, raw_groups)
    }

    pub(crate) fn class_loader_enumeration_support(
        &self,
        vm: NonNull<jni::JavaVM>,
    ) -> FeatureSupport {
        if self.visit_class_loaders.is_none() {
            return unsupported_support("VisitClassLoaders is unavailable");
        }
        if self.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.suspend_all.is_none() {
            return unsupported_support("ThreadList::SuspendAll is unavailable");
        }
        if self.resume_all.is_none() {
            return unsupported_support("ThreadList::ResumeAll is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_CLASS_LOADER_ENUMERATION)
    }

    pub(crate) fn loaded_class_enumeration_support(
        &self,
        vm: NonNull<jni::JavaVM>,
    ) -> FeatureSupport {
        if self.visit_classes.is_none() {
            return unsupported_support("ClassLinker::VisitClasses is unavailable");
        }
        if self.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.get_class_descriptor.is_none() {
            return unsupported_support("mirror::Class::GetDescriptor is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_LOADED_CLASS_ENUMERATION)
    }

    pub(crate) fn method_query_support(&self, vm: NonNull<jni::JavaVM>) -> FeatureSupport {
        if self.visit_classes.is_none() {
            return unsupported_support("ClassLinker::VisitClasses is unavailable");
        }
        if self.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.get_class_descriptor.is_none() {
            return unsupported_support("mirror::Class::GetDescriptor is unavailable");
        }
        if self.pretty_method.is_none() {
            return unsupported_support("ArtMethod::PrettyMethod is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_METHOD_QUERY)
    }

    pub(crate) fn method_replacement_support(&self, vm: &Vm) -> FeatureSupport {
        match self.detect_method_replacement_prerequisites(vm) {
            Ok(_) => FeatureSupport::Experimental {
                reason: "ART method replacement prerequisites are available for experimental descriptor-driven static and instance clone-active replacement; exported overload replacement APIs are experimental".to_owned(),
            },
            Err(Error::UnsupportedFeature { reason, .. }) => unsupported_support(reason),
            Err(error) => unsupported_support(error.to_string()),
        }
    }

    pub(crate) fn replace_method(
        &self,
        vm: &Vm,
        kind: MethodKind,
        method_id: jni::jmethodID,
        replacement: *mut c_void,
    ) -> Result<ArtMethodReplacementGuard> {
        if replacement.is_null() {
            return Err(Error::NullReturn {
                operation: "ART replacement function",
            });
        }

        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        validate_replacement_function(replacement, &memory)?;
        let api_level = android_api_level(FEATURE_METHOD_REPLACEMENT)?;
        let layout = self.detect_method_replacement_prerequisites(vm)?;
        self.replacement_controller.ensure_hooks()?;
        self.replacement_controller
            .ensure_quick_entrypoint_hooks(&layout.trampolines)?;
        let mut guard = None;

        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |_thread| {
            let candidates = self.art_method_from_jni_id(&layout.runtime, method_id);
            let compile_dont_bother = compile_dont_bother_flag(api_level);
            let mut saw_readable_candidate = false;
            let mut saw_wrong_kind_candidate = false;
            for method in candidates {
                let Ok(original) = snapshot_art_method(method, &layout.method, &memory) else {
                    continue;
                };
                saw_readable_candidate = true;
                if !art_method_kind_matches(original, kind) {
                    saw_wrong_kind_candidate = true;
                    continue;
                }
                let clone_patched = patched_replacement_method(
                    original,
                    replacement,
                    layout.trampolines.quick_generic_jni_trampoline,
                    compile_dont_bother,
                );
                let cloned_method = clone_replacement_art_method(
                    method,
                    &layout.method,
                    original,
                    clone_patched,
                    &memory,
                )?;
                let dispatch_thunk = ArtMethodDispatchThunk::new(
                    cloned_method.as_ptr(),
                    layout.trampolines.quick_to_interpreter_bridge_trampoline,
                    layout.method.quick_code_offset,
                    layout.thread_managed_stack_offset,
                )?;
                let original_patched = patched_original_method_for_clone_dispatch(
                    original,
                    dispatch_thunk.as_ptr(),
                    compile_dont_bother,
                );
                let _suspended = self.suspend_all_threads(&layout.runtime)?;
                self.replacement_controller.register(
                    method,
                    cloned_method.as_ptr(),
                    dispatch_thunk.as_ptr(),
                    dispatch_thunk.len(),
                    ArtReplacementSynchronization {
                        quick_code_offset: layout.method.quick_code_offset,
                        thread_managed_stack_offset: layout.thread_managed_stack_offset,
                        nterp_entrypoint: None,
                        quick_to_interpreter_bridge: layout
                            .trampolines
                            .quick_to_interpreter_bridge_trampoline
                            as usize,
                    },
                )?;
                self.replacement_controller
                    .register_jni_id(method_id, method);
                if let Err(error) = patch_art_method_verified(
                    method,
                    &layout.method,
                    original,
                    original_patched,
                    &memory,
                ) {
                    self.replacement_controller.unregister(method);
                    return Err(error);
                }
                self.replacement_controller
                    .synchronize_replacement_methods();
                guard = Some(ArtMethodReplacementGuard {
                    backend: self.clone(),
                    vm: vm.clone(),
                    method,
                    cloned_method,
                    dispatch_thunk,
                    layout,
                    original,
                    original_patched,
                    clone_patched,
                    reverted: false,
                });
                return Ok(());
            }

            if saw_wrong_kind_candidate {
                let reason = match kind {
                    MethodKind::Static => "resolved target ArtMethod is not static",
                    MethodKind::Instance => "resolved target ArtMethod is static",
                    MethodKind::Constructor => "resolved target ArtMethod is a constructor",
                };
                return unsupported_feature(FEATURE_METHOD_REPLACEMENT, reason);
            }
            if saw_readable_candidate {
                let reason = match kind {
                    MethodKind::Static => {
                        "unable to resolve a static target ArtMethod from JNI method ID"
                    }
                    MethodKind::Instance => {
                        "unable to resolve an instance target ArtMethod from JNI method ID"
                    }
                    MethodKind::Constructor => {
                        "unable to resolve a constructor target ArtMethod from JNI method ID"
                    }
                };
                return unsupported_feature(FEATURE_METHOD_REPLACEMENT, reason);
            }
            unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to resolve target ArtMethod from JNI method ID: no readable candidates",
            )
        })?;

        guard.ok_or(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "method replacement did not produce a guard".to_owned(),
        })
    }

    pub(super) fn restore_method(
        &self,
        vm: &Vm,
        method: *mut c_void,
        layout: &ArtMethodReplacementLayout,
        original: ArtMethodSnapshot,
    ) -> Result<()> {
        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |_thread| {
            let _suspended = self.suspend_all_threads(&layout.runtime)?;
            restore_art_method_verified(method, &layout.method, original, &memory)
        })
    }

    fn ensure_class_loader_enumeration_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(
            FEATURE_CLASS_LOADER_ENUMERATION,
            self.class_loader_enumeration_support(vm),
        )
    }

    fn ensure_loaded_class_enumeration_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(
            FEATURE_LOADED_CLASS_ENUMERATION,
            self.loaded_class_enumeration_support(vm),
        )
    }

    fn ensure_method_query_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(FEATURE_METHOD_QUERY, self.method_query_support(vm))
    }

    fn detect_method_replacement_prerequisites(
        &self,
        vm: &Vm,
    ) -> Result<ArtMethodReplacementLayout> {
        self.replacement_controller.ensure_dispatch_supported()?;
        if self.pretty_method.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod::PrettyMethod is unavailable",
            );
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "only arm64-v8a is supported in this milestone",
            );
        }
        if self.suspend_all.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ThreadList::SuspendAll is unavailable for safe method patching",
            );
        }
        if self.resume_all.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ThreadList::ResumeAll is unavailable for safe method patching",
            );
        }
        let android_runtime = self
            .android_runtime
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "libandroid_runtime.so is unavailable".to_owned(),
            })?;

        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        let api_level = android_api_level(FEATURE_METHOD_REPLACEMENT)?;
        let (runtime_layout, trampolines) = detect_runtime_layout_for_method_replacement(
            vm.handle(),
            api_level,
            self.set_jni_id_type,
            self.class_linker_entrypoint_predicates(),
            &memory,
            FEATURE_METHOD_REPLACEMENT,
        )?;
        validate_replacement_trampoline(&trampolines, &memory)?;
        if runtime_layout.uses_indirect_jni_ids() && self.decode_method_id.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "JniIdManager::DecodeMethodId is unavailable for indirect JNI method IDs",
            );
        }
        let layout_method = find_method_replacement_layout_probe(&env)?;

        let mut layout = None;
        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |thread| {
            let process_method = self.art_method_from_jni_id(&runtime_layout, layout_method);
            let method_layout = detect_art_method_replacement_layout(
                &process_method,
                android_runtime,
                api_level,
                &memory,
                true,
                FEATURE_METHOD_REPLACEMENT,
            )?;
            layout = Some(ArtMethodReplacementLayout {
                api_level,
                runtime: runtime_layout,
                method: method_layout,
                trampolines,
                thread_managed_stack_offset: detect_art_thread_managed_stack_offset(
                    FEATURE_METHOD_REPLACEMENT,
                    thread,
                    env.handle().as_ptr().cast(),
                )?,
            });
            Ok(())
        })?;

        layout.ok_or(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "method replacement prerequisites were not probed".to_owned(),
        })
    }

    pub(super) fn art_method_from_jni_id(
        &self,
        layout: &ArtRuntimeLayout,
        method_id: jni::jmethodID,
    ) -> Vec<*mut c_void> {
        if layout.uses_indirect_jni_ids() {
            return self
                .decode_method_id
                .and_then(|decode_method_id| {
                    let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
                    (!decoded.is_null()).then_some(decoded)
                })
                .into_iter()
                .collect();
        }

        let mut candidates = vec![method_id.cast::<c_void>()];
        if layout.jni_ids_indirection.is_none()
            && let Some(decode_method_id) = self.decode_method_id
            && !layout.jni_id_manager.is_null()
        {
            let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
            if !decoded.is_null() && !candidates.contains(&decoded) {
                candidates.push(decoded);
            }
        }

        candidates
    }

    fn class_linker_entrypoint_predicates(&self) -> Option<ArtClassLinkerEntrypointPredicates> {
        Some(ArtClassLinkerEntrypointPredicates {
            is_quick_resolution_stub: self.is_quick_resolution_stub?,
            is_quick_to_interpreter_bridge: self.is_quick_to_interpreter_bridge?,
            is_quick_generic_jni_stub: self.is_quick_generic_jni_stub?,
        })
    }

    fn suspend_all_threads(&self, layout: &ArtRuntimeLayout) -> Result<SuspendedAllThreads> {
        let suspend_all = self.suspend_all.ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "ThreadList::SuspendAll is unavailable for safe method patching".to_owned(),
        })?;
        let resume_all = self.resume_all.ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "ThreadList::ResumeAll is unavailable for safe method patching".to_owned(),
        })?;
        Ok(SuspendedAllThreads::new(
            suspend_all,
            resume_all,
            layout.thread_list,
        ))
    }

    fn with_runnable_art_thread(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let transition = self.runnable_thread(env, feature)?;
        transition.run(feature, env, f)
    }

    fn runnable_thread(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
    ) -> Result<&runnable_thread::RunnableThreadTransition> {
        if let Some(transition) = self.runnable_thread.get() {
            return Ok(transition);
        }

        let transition =
            runnable_thread::build(feature, env, self.exception_clear, self.fatal_error)?;
        let _ = self.runnable_thread.set(transition);
        Ok(self
            .runnable_thread
            .get()
            .expect("runnable thread transition was just initialized"))
    }
}

fn find_method_replacement_layout_probe(env: &crate::env::Env<'_>) -> Result<jni::jmethodID> {
    let method = env
        .find_class("android/os/Process")
        .and_then(|class| env.lookup_static_method(&class, "getElapsedCpuTime", "()J"))
        .or_else(|_| {
            let system = env.find_class("java/lang/System")?;
            env.lookup_static_method(&system, "currentTimeMillis", "()J")
        })?;
    Ok(method.raw())
}
