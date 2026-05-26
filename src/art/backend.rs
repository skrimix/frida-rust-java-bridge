use std::{
    ffi::{c_char, c_void},
    ptr::NonNull,
    sync::{Arc, OnceLock},
};

use frida_gum::Module;

use super::{
    enumeration::*,
    features::*,
    layout::*,
    memory::MemoryRanges,
    replacement::{
        ArtMethodDispatchThunk, ArtMethodReplacementGuard, ArtReplacementController,
        ArtReplacementSynchronization,
    },
    resolution::*,
    runnable_thread,
    runtime_layout::*,
    strings::ArtStdString,
    symbols::*,
    threads::SuspendedAllThreads,
};
use crate::{
    env::{Env, MethodKind},
    error::{Error, Result},
    java::{ClassLoaderRef, JavaChooseControl, JavaObject, raw},
    jni, metadata,
    refs::AsJObject,
    runtime::FeatureSupport,
    vm::Vm,
};

pub(super) type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
pub(super) type GetClassDescriptor =
    unsafe extern "C" fn(*mut c_void, *mut ArtStdString) -> *const c_char;
pub(super) type PrettyMethod = unsafe extern "C" fn(*mut ArtStdString, *mut c_void, bool);
pub(super) type DecodeMethodId = unsafe extern "C" fn(*mut c_void, jni::jmethodID) -> *mut c_void;
pub(super) type IsQuickEntrypoint = unsafe extern "C" fn(*mut c_void, *const c_void) -> bool;
pub(super) type SuspendAllWithCause = unsafe extern "C" fn(*mut c_void, *const c_char, bool);
pub(super) type SuspendAllLegacy = unsafe extern "C" fn(*mut c_void);
pub(super) type ResumeAll = unsafe extern "C" fn(*mut c_void);
pub(super) type VisitClassLoaders = unsafe extern "C" fn(*mut c_void, *mut ArtClassLoaderVisitor);
pub(super) type VisitClasses = unsafe extern "C" fn(*mut c_void, *mut ArtClassVisitor);
pub(super) type VisitClassesCallback =
    unsafe extern "C" fn(*mut c_void, ArtClassCallback, *mut c_void);
pub(super) type ArtClassCallback = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
pub(super) type VisitObjects = unsafe extern "C" fn(*mut c_void, HeapObjectCallback, *mut c_void);
pub(super) type HeapObjectCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub(super) type GetInstances =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i32, *mut c_void);
pub(super) type GetInstancesAssignable =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, bool, i32, *mut c_void);
pub(super) type DecodeGlobalNoThread =
    unsafe extern "C" fn(*mut jni::JavaVM, jni::jobject) -> usize;
pub(super) type DecodeGlobalWithThread =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, jni::jobject) -> usize;
pub(super) type ThreadDecodeGlobalJObject =
    unsafe extern "C" fn(*mut c_void, jni::jobject) -> usize;
pub(super) type GetOatQuickMethodHeader = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
pub(super) type DbgSetJdwpAllowed = unsafe extern "C" fn(bool);
pub(super) type DbgConfigureJdwp = unsafe extern "C" fn(*const c_void);
pub(super) type StartDebugger = unsafe extern "C" fn(*mut c_void);
pub(super) type DbgStartJdwp = unsafe extern "C" fn();
pub(super) type DbgGoActive = unsafe extern "C" fn();
pub(super) type DbgRequestDeoptimization = unsafe extern "C" fn(*const c_void);
pub(super) type DbgManageDeoptimization = unsafe extern "C" fn();
pub(super) type InstrumentationEnableDeoptimization = unsafe extern "C" fn(*mut c_void);
pub(super) type InstrumentationDeoptimizeEverything =
    unsafe extern "C" fn(*mut c_void, *const c_char);
pub(super) type InstrumentationDeoptimize = unsafe extern "C" fn(*mut c_void, *mut c_void);
pub(super) type RuntimeDeoptimizeBootImage = unsafe extern "C" fn(*mut c_void);

/// Resolved ART interface for the current process.
///
/// `ArtBackend` is this crate's view of available ART symbols, inferred layout support, and guarded
/// operations over a live VM. It is not the ART runtime object itself; runtime pointers stay inside
/// the layout structs returned by probing.
#[derive(Clone)]
pub(crate) struct ArtBackend {
    pub(super) android_runtime: Option<ArtModuleRange>,
    pub(super) add_global_ref: Option<AddGlobalRef>,
    pub(super) suspend_all: Option<SuspendAll>,
    pub(super) resume_all: Option<ResumeAll>,
    pub(super) visit_class_loaders: Option<VisitClassLoaders>,
    pub(super) visit_classes: Option<VisitClassesKind>,
    pub(super) visit_objects: Option<VisitObjects>,
    pub(super) get_instances: Option<GetInstancesKind>,
    pub(super) decode_global: Option<DecodeGlobalKind>,
    pub(super) get_class_descriptor: Option<GetClassDescriptor>,
    pub(super) pretty_method: Option<PrettyMethodFunction>,
    pub(super) decode_method_id: Option<DecodeMethodId>,
    pub(super) set_jni_id_type: Option<*const c_void>,
    pub(super) is_quick_resolution_stub: Option<IsQuickEntrypoint>,
    pub(super) is_quick_to_interpreter_bridge: Option<IsQuickEntrypoint>,
    pub(super) is_quick_generic_jni_stub: Option<IsQuickEntrypoint>,
    pub(super) exception_clear: Option<*const c_void>,
    pub(super) fatal_error: Option<*const c_void>,
    pub(super) dbg_set_jdwp_allowed: Option<DbgSetJdwpAllowed>,
    pub(super) dbg_configure_jdwp: Option<DbgConfigureJdwp>,
    pub(super) internal_start_debugger: Option<StartDebugger>,
    pub(super) dbg_start_jdwp: Option<DbgStartJdwp>,
    pub(super) dbg_go_active: Option<DbgGoActive>,
    pub(super) dbg_request_deoptimization: Option<DbgRequestDeoptimization>,
    pub(super) dbg_manage_deoptimization: Option<DbgManageDeoptimization>,
    pub(super) dbg_registry: Option<*const c_void>,
    pub(super) dbg_debugger_active: Option<*const c_void>,
    pub(super) instrumentation_enable_deoptimization: Option<InstrumentationEnableDeoptimization>,
    pub(super) instrumentation_deoptimize_everything: Option<InstrumentationDeoptimizeEverything>,
    pub(super) instrumentation_deoptimize: Option<InstrumentationDeoptimize>,
    pub(super) runtime_deoptimize_boot_image: Option<RuntimeDeoptimizeBootImage>,
    pub(super) jdwp_adb_state_accept: Option<*const c_void>,
    pub(super) jdwp_adb_state_receive_client_fd: Option<*const c_void>,
    pub(super) runnable_thread: Arc<OnceLock<runnable_thread::RunnableThreadTransition>>,
    pub(super) replacement_controller: Arc<ArtReplacementController>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtModuleRange {
    pub(super) start: usize,
    pub(super) end: usize,
}

#[derive(Clone, Copy)]
pub(super) enum SuspendAll {
    WithCause(SuspendAllWithCause),
    Legacy(SuspendAllLegacy),
}

#[derive(Clone, Copy)]
pub(super) enum VisitClassesKind {
    Visitor(VisitClasses),
    Callback(VisitClassesCallback),
}

#[derive(Clone, Copy)]
pub(super) enum GetInstancesKind {
    Exact(GetInstances),
    WithAssignable(GetInstancesAssignable),
}

#[derive(Clone, Copy)]
pub(super) enum DecodeGlobalKind {
    NoThread(DecodeGlobalNoThread),
    WithThread(DecodeGlobalWithThread),
    Thread(ThreadDecodeGlobalJObject),
}

impl ArtBackend {
    pub(crate) fn from_module(module: &Module, android_runtime: Option<ArtModuleRange>) -> Self {
        Self {
            android_runtime,
            add_global_ref: resolve_any(module, &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER]),
            suspend_all: resolve_suspend_all(module),
            resume_all: resolve(module, RESUME_ALL),
            visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
            visit_classes: resolve_visit_classes(module),
            visit_objects: resolve(module, VISIT_OBJECTS),
            get_instances: resolve_get_instances(module),
            decode_global: resolve_decode_global(module),
            get_class_descriptor: resolve(module, GET_CLASS_DESCRIPTOR),
            pretty_method: resolve_pretty_method(module),
            decode_method_id: resolve(module, DECODE_METHOD_ID),
            set_jni_id_type: resolve_pointer(module, SET_JNI_ID_TYPE),
            is_quick_resolution_stub: resolve(module, IS_QUICK_RESOLUTION_STUB),
            is_quick_to_interpreter_bridge: resolve(module, IS_QUICK_TO_INTERPRETER_BRIDGE),
            is_quick_generic_jni_stub: resolve(module, IS_QUICK_GENERIC_JNI_STUB),
            exception_clear: resolve_pointer(module, JNI_EXCEPTION_CLEAR),
            fatal_error: resolve_pointer(module, JNI_FATAL_ERROR),
            dbg_set_jdwp_allowed: resolve(module, DBG_SET_JDWP_ALLOWED),
            dbg_configure_jdwp: resolve(module, DBG_CONFIGURE_JDWP),
            internal_start_debugger: resolve(module, INTERNAL_DEBUGGER_CONTROL_START_DEBUGGER),
            dbg_start_jdwp: resolve(module, DBG_START_JDWP),
            dbg_go_active: resolve(module, DBG_GO_ACTIVE),
            dbg_request_deoptimization: resolve(module, DBG_REQUEST_DEOPTIMIZATION),
            dbg_manage_deoptimization: resolve(module, DBG_MANAGE_DEOPTIMIZATION),
            dbg_registry: resolve_pointer(module, DBG_REGISTRY),
            dbg_debugger_active: resolve_pointer(module, DBG_DEBUGGER_ACTIVE),
            instrumentation_enable_deoptimization: resolve(
                module,
                INSTRUMENTATION_ENABLE_DEOPTIMIZATION,
            ),
            instrumentation_deoptimize_everything: resolve(
                module,
                INSTRUMENTATION_DEOPTIMIZE_EVERYTHING,
            ),
            instrumentation_deoptimize: resolve(module, INSTRUMENTATION_DEOPTIMIZE),
            runtime_deoptimize_boot_image: resolve(module, RUNTIME_DEOPTIMIZE_BOOT_IMAGE),
            jdwp_adb_state_accept: resolve_pointer(module, JDWP_ADB_STATE_ACCEPT),
            jdwp_adb_state_receive_client_fd: resolve_pointer(
                module,
                JDWP_ADB_STATE_RECEIVE_CLIENT_FD,
            ),
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
            visit_objects: None,
            get_instances: None,
            decode_global: None,
            get_class_descriptor: None,
            pretty_method: None,
            decode_method_id: None,
            set_jni_id_type: None,
            is_quick_resolution_stub: None,
            is_quick_to_interpreter_bridge: None,
            is_quick_generic_jni_stub: None,
            exception_clear: None,
            fatal_error: None,
            dbg_set_jdwp_allowed: None,
            dbg_configure_jdwp: None,
            internal_start_debugger: None,
            dbg_start_jdwp: None,
            dbg_go_active: None,
            dbg_request_deoptimization: None,
            dbg_manage_deoptimization: None,
            dbg_registry: None,
            dbg_debugger_active: None,
            instrumentation_enable_deoptimization: None,
            instrumentation_deoptimize_everything: None,
            instrumentation_deoptimize: None,
            runtime_deoptimize_boot_image: None,
            jdwp_adb_state_accept: None,
            jdwp_adb_state_receive_client_fd: None,
            runnable_thread: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::empty_for_tests()),
        }
    }

    pub(crate) fn enumerate_class_loaders(&self, vm: &Vm) -> Result<Vec<ClassLoaderRef>> {
        // SAFETY: ART enumeration needs the process JavaVM pointer for layout probing and global
        // reference creation. `vm` is the live runtime handle owned by this backend call.
        let vm_handle = unsafe { vm.handle() };
        self.ensure_class_loader_enumeration_supported(vm_handle)?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm_handle, FEATURE_CLASS_LOADER_ENUMERATION)
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

            let vm_handle = vm_handle.as_ptr();
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

    pub(crate) fn enumerate_loaded_classes(&self, vm: &Vm) -> Result<Vec<raw::Class>> {
        // SAFETY: ART class enumeration uses this live VM pointer for support checks and runtime
        // layout probing only.
        let vm_handle = unsafe { vm.handle() };
        self.ensure_loaded_class_enumeration_supported(vm_handle)?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm_handle, FEATURE_LOADED_CLASS_ENUMERATION)
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
        // SAFETY: Method query support/layout probing operates on the live process JavaVM.
        let vm_handle = unsafe { vm.handle() };
        self.ensure_method_query_supported(vm_handle)?;

        let env = vm.attach_current_thread()?;
        let runtime_layout = detect_runtime_layout(vm_handle, FEATURE_METHOD_QUERY)
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

            let thread_method =
                self.art_method_from_jni_id(&runtime_layout, unsafe { thread_get_name.raw() });
            let thread_is_alive_method =
                self.art_method_from_jni_id(&runtime_layout, unsafe { thread_is_alive.raw() });
            let thread_current_thread_method = self
                .art_method_from_jni_id(&runtime_layout, unsafe { thread_current_thread.raw() });
            let process_method = self.art_method_from_jni_id(&runtime_layout, unsafe {
                system_current_time_millis.raw()
            });
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

    pub(crate) fn choose_instances(
        &self,
        vm: &Vm,
        class: &raw::Class,
        callback: &mut dyn FnMut(&JavaObject) -> Result<JavaChooseControl>,
    ) -> Result<()> {
        ensure_feature_supported(
            FEATURE_HEAP_ENUMERATION,
            // SAFETY: Heap enumeration support probing operates on the live process JavaVM.
            self.heap_enumeration_support(unsafe { vm.handle() }),
        )?;
        let env = vm.attach_current_thread()?;
        // SAFETY: Heap enumeration layout probing operates on the live process JavaVM.
        let layout = detect_runtime_layout(unsafe { vm.handle() }, FEATURE_HEAP_ENUMERATION)
            .expect("runtime layout support checked before heap enumeration");
        let mut raw_instances = Vec::new();

        let query_result =
            self.with_runnable_art_thread(&env, FEATURE_HEAP_ENUMERATION, |thread| {
                let needle = self.decode_global_object_reference(vm, thread, class.as_jobject())?;
                match self.visit_objects {
                    Some(visit_objects) => self.choose_instances_with_visit_objects(
                        vm,
                        thread,
                        &layout,
                        needle,
                        visit_objects,
                        &mut raw_instances,
                    ),
                    None => {
                        let get_instances =
                            self.get_instances
                                .ok_or_else(|| Error::UnsupportedFeature {
                                    feature: FEATURE_HEAP_ENUMERATION,
                                    reason:
                                        "Heap::VisitObjects and Heap::GetInstances are unavailable"
                                            .to_owned(),
                                })?;
                        self.choose_instances_with_get_instances(
                            vm,
                            &env,
                            thread,
                            &layout,
                            needle,
                            get_instances,
                            &mut raw_instances,
                        )
                    }
                }
            });
        if let Err(error) = query_result {
            for raw in raw_instances {
                unsafe { env.delete_global_ref_raw(raw.0) };
            }
            return Err(error);
        }

        deliver_heap_instances(vm, &env, raw_instances, callback)
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

    pub(crate) fn heap_enumeration_support(&self, vm: NonNull<jni::JavaVM>) -> FeatureSupport {
        if self.visit_objects.is_none() && self.get_instances.is_none() {
            return unsupported_support(
                "Heap::VisitObjects and Heap::GetInstances are unavailable",
            );
        }
        if self.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.decode_global.is_none() {
            return unsupported_support("JavaVMExt::DecodeGlobal is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_HEAP_ENUMERATION)
    }

    fn choose_instances_with_visit_objects(
        &self,
        vm: &Vm,
        thread: *mut c_void,
        layout: &ArtRuntimeLayout,
        needle_class_reference: u32,
        visit_objects: VisitObjects,
        instances: &mut Vec<RawHeapInstance>,
    ) -> Result<()> {
        let add_global_ref = self
            .add_global_ref
            .expect("add_global_ref symbol checked before heap enumeration");
        let mut processor = ArtHeapInstanceProcessor::new(
            add_global_ref,
            vm,
            thread,
            needle_class_reference,
            instances,
        );

        unsafe {
            visit_objects(
                layout.heap,
                on_visit_heap_object,
                (&mut processor as *mut ArtHeapInstanceProcessor<'_>).cast(),
            );
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn choose_instances_with_get_instances(
        &self,
        vm: &Vm,
        env: &Env<'_>,
        thread: *mut c_void,
        layout: &ArtRuntimeLayout,
        needle_class_reference: u32,
        get_instances: GetInstancesKind,
        instances: &mut Vec<RawHeapInstance>,
    ) -> Result<()> {
        // SAFETY: This scope is created while `env` is borrowed on the current attached thread.
        let env_handle = unsafe { env.handle() };
        let memory = MemoryRanges::current_for_feature(FEATURE_HEAP_ENUMERATION)?;
        let mut scope =
            FakeVariableSizedHandleScope::new(thread, env_handle.as_ptr().cast(), &memory)?;
        let class_handle = scope.new_handle(needle_class_reference)?;
        let mut vector = ArtHandleVector::default();

        match get_instances {
            GetInstancesKind::Exact(get_instances) => unsafe {
                get_instances(
                    layout.heap,
                    scope.as_mut_ptr(),
                    class_handle,
                    0,
                    vector.as_mut_ptr(),
                );
            },
            GetInstancesKind::WithAssignable(get_instances) => unsafe {
                get_instances(
                    layout.heap,
                    scope.as_mut_ptr(),
                    class_handle,
                    false,
                    0,
                    vector.as_mut_ptr(),
                );
            },
        }

        let env = vm.attach_current_thread()?;
        for handle in vector.handles() {
            let raw = unsafe { env.new_global_ref_raw(handle.cast())? };
            instances.push(RawHeapInstance(raw));
        }
        vector.dispose();
        scope.dispose(thread);
        Ok(())
    }

    fn decode_global_object_reference(
        &self,
        vm: &Vm,
        thread: *mut c_void,
        object: jni::jobject,
    ) -> Result<u32> {
        let decode_global = self
            .decode_global
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_HEAP_ENUMERATION,
                reason: "JavaVMExt::DecodeGlobal is unavailable".to_owned(),
            })?;
        let decoded = match decode_global {
            DecodeGlobalKind::NoThread(decode_global) => unsafe {
                decode_global(vm.handle().as_ptr(), object)
            },
            DecodeGlobalKind::WithThread(decode_global) => unsafe {
                decode_global(vm.handle().as_ptr(), thread, object)
            },
            DecodeGlobalKind::Thread(decode_global) => unsafe { decode_global(thread, object) },
        };
        if decoded == 0 {
            return Err(Error::NullReturn {
                operation: "JavaVMExt::DecodeGlobal",
            });
        }
        Ok(decoded as u32)
    }

    pub(crate) fn method_replacement_support(&self, vm: &Vm) -> FeatureSupport {
        match self.detect_method_replacement_prerequisites(vm) {
            Ok(_) => FeatureSupport::Supported,
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
            // SAFETY: Replacement layout probing operates on the live process JavaVM.
            unsafe { vm.handle() },
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
                    // SAFETY: `env` is borrowed on the current attached thread for this probe.
                    unsafe { env.handle() }.as_ptr().cast(),
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

    pub(super) fn with_runnable_art_thread(
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
    Ok(unsafe { method.raw() })
}
