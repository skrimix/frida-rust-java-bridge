#![allow(dead_code)]

use std::{
    collections::HashSet,
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs,
    ptr::{self, NonNull},
    sync::{Arc, OnceLock},
};

use frida_gum::Module;

use crate::{
    env::MethodKind,
    error::{Error, Result},
    java::{ClassLoaderKind, ClassLoaderRef, JavaClass},
    jni, metadata,
    refs::{AsJClass, AsJObject, ClassKind, GlobalRef},
    runtime::{FeatureSupport, native_pointer_to_fn},
    signature::MethodSignature,
    vm::Vm,
};

mod thread_transition;

const FEATURE_CLASS_LOADER_ENUMERATION: &str = "ART class-loader enumeration";
const FEATURE_LOADED_CLASS_ENUMERATION: &str = "ART loaded-class enumeration";
const FEATURE_METHOD_QUERY: &str = "ART direct method enumeration";
const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const STD_STRING_SIZE: usize = 3 * POINTER_SIZE;
const PROP_VALUE_MAX: usize = 92;
const K_ACC_STATIC: u32 = 0x0008;
const K_ACC_FINAL: u32 = 0x0010;
const K_ACC_NATIVE: u32 = 0x0100;
const K_ACC_CONSTRUCTOR: u32 = 0x00010000;
const K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE: u32 = 0x40000000;
const K_ACC_NTERP_INVOKE_FAST_PATH_FLAG: u32 = 0x00200000;
const K_ACC_PUBLIC_API: u32 = 0x10000000;
const CLASS_LAYOUT_SCAN_LIMIT: usize = 0x100;
const METHOD_LAYOUT_SCAN_LIMIT: usize = 64;
const ART_METHOD_MIN_SIZE: usize = 16;
const ART_METHOD_MAX_SIZE: usize = 256;
const ART_METHOD_ARRAY_MAX_PROBE: usize = 100;
const ADD_GLOBAL_REF_OBJ_PTR: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadENS_6ObjPtrINS_6mirror6ObjectEEE";
const ADD_GLOBAL_REF_POINTER: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadEPNS_6mirror6ObjectE";
const SUSPEND_ALL_WITH_CAUSE: &str = "_ZN3art10ThreadList10SuspendAllEPKcb";
const SUSPEND_ALL_LEGACY: &str = "_ZN3art10ThreadList10SuspendAllEv";
const RESUME_ALL: &str = "_ZN3art10ThreadList9ResumeAllEv";
const VISIT_CLASS_LOADERS: &str =
    "_ZNK3art11ClassLinker17VisitClassLoadersEPNS_18ClassLoaderVisitorE";
const VISIT_CLASSES_VISITOR: &str = "_ZN3art11ClassLinker12VisitClassesEPNS_12ClassVisitorE";
const VISIT_CLASSES_CALLBACK: &str =
    "_ZN3art11ClassLinker12VisitClassesEPFbPNS_6mirror5ClassEPvES4_";
const GET_CLASS_DESCRIPTOR: &str = "_ZN3art6mirror5Class13GetDescriptorEPNSt3__112basic_stringIcNS2_11char_traitsIcEENS2_9allocatorIcEEEE";
const PRETTY_METHOD: &str = "_ZN3art9ArtMethod12PrettyMethodEb";
const PRETTY_METHOD_NULL_SAFE: &str = "_ZN3art12PrettyMethodEPNS_9ArtMethodEb";
const DECODE_METHOD_ID: &str = "_ZN3art3jni12JniIdManager14DecodeMethodIdEP10_jmethodID";
const JNI_EXCEPTION_CLEAR: &str = "_ZN3art3JNIILb1EE14ExceptionClearEP7_JNIEnv";
const JNI_FATAL_ERROR: &str = "_ZN3art3JNIILb1EE10FatalErrorEP7_JNIEnvPKc";

type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
type GetClassDescriptor = unsafe extern "C" fn(*mut c_void, *mut ArtStdString) -> *const c_char;
type PrettyMethod = unsafe extern "C" fn(*mut ArtStdString, *mut c_void, bool);
type DecodeMethodId = unsafe extern "C" fn(*mut c_void, jni::jmethodID) -> *mut c_void;
type SuspendAllWithCause = unsafe extern "C" fn(*mut c_void, *const c_char, bool);
type SuspendAllLegacy = unsafe extern "C" fn(*mut c_void);
type ResumeAll = unsafe extern "C" fn(*mut c_void);
type VisitClassLoaders = unsafe extern "C" fn(*mut c_void, *mut ArtClassLoaderVisitor);
type VisitClasses = unsafe extern "C" fn(*mut c_void, *mut ArtClassVisitor);
type VisitClassesCallback = unsafe extern "C" fn(*mut c_void, ArtClassCallback, *mut c_void);
type ArtClassCallback = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
}

#[derive(Clone)]
pub(crate) struct ArtBackend {
    add_global_ref: Option<AddGlobalRef>,
    suspend_all: Option<SuspendAll>,
    resume_all: Option<ResumeAll>,
    visit_class_loaders: Option<VisitClassLoaders>,
    visit_classes: Option<VisitClassesKind>,
    get_class_descriptor: Option<GetClassDescriptor>,
    pretty_method: Option<PrettyMethodFunction>,
    decode_method_id: Option<DecodeMethodId>,
    exception_clear: Option<*const c_void>,
    fatal_error: Option<*const c_void>,
    thread_transition: Arc<OnceLock<thread_transition::ThreadTransition>>,
}

#[derive(Clone, Copy)]
enum SuspendAll {
    WithCause(SuspendAllWithCause),
    Legacy(SuspendAllLegacy),
}

#[derive(Clone, Copy)]
enum VisitClassesKind {
    Visitor(VisitClasses),
    Callback(VisitClassesCallback),
}

#[repr(C)]
struct ArtClassLoaderVisitor {
    vtable: *const *const c_void,
    vtable_storage: [*const c_void; 3],
    loaders: *mut Vec<*mut c_void>,
}

#[repr(C)]
struct ArtClassVisitor {
    vtable: *const *const c_void,
    vtable_storage: [*const c_void; 3],
    context: *mut c_void,
    visit: ArtRustClassCallback,
}

struct RawClass(jni::jclass);

type ArtRustClassCallback = unsafe fn(*mut c_void, *mut c_void) -> bool;

#[repr(C)]
struct ArtStdString {
    storage: [usize; 3],
}

struct RawLoadedClass {
    name: String,
    raw: jni::jclass,
}

struct ArtClassProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    get_class_descriptor: GetClassDescriptor,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    seen: HashSet<usize>,
    classes: &'callback mut Vec<RawLoadedClass>,
    error: Option<Error>,
}

#[derive(Clone)]
struct PrettyMethodFunction {
    function: PrettyMethod,
    _thunk: Option<Arc<ExecutableMemory>>,
}

struct ExecutableMemory {
    pointer: NonNull<c_void>,
    length: usize,
}

unsafe impl Send for ExecutableMemory {}
unsafe impl Sync for ExecutableMemory {}

struct FindArtClassProcessor {
    get_class_descriptor: GetClassDescriptor,
    descriptor: &'static str,
    class: Option<*mut c_void>,
    error: Option<Error>,
}

struct RawMethodQueryGroup {
    loader_key: u32,
    loader: Option<jni::jobject>,
    classes: Vec<metadata::JavaMethodQueryClass>,
}

struct ArtMethodQueryProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    get_class_descriptor: GetClassDescriptor,
    pretty_method: PrettyMethodFunction,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    query: &'callback metadata::MethodQuery,
    layout: ArtMethodQueryLayout,
    memory: &'callback MemoryRanges,
    seen_classes: HashSet<usize>,
    groups: &'callback mut Vec<RawMethodQueryGroup>,
    error: Option<Error>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtRuntimeLayout {
    runtime: *mut c_void,
    thread_list: *mut c_void,
    class_linker: *mut c_void,
    jni_id_manager: *mut c_void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodQueryLayout {
    class_methods_offset: usize,
    class_copied_methods_offset: usize,
    method_size: usize,
    method_access_flags_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtArray {
    data: *mut c_void,
    length: usize,
}

#[derive(Debug, Default)]
struct MemoryRanges {
    ranges: Vec<MemoryRange>,
}

#[derive(Debug)]
struct MemoryRange {
    start: usize,
    end: usize,
    executable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodRuntimeLayout {
    method_size: usize,
    access_flags_offset: usize,
}

impl ArtBackend {
    pub(crate) fn from_module(module: &Module) -> Self {
        Self {
            add_global_ref: resolve_any(module, &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER]),
            suspend_all: resolve_suspend_all(module),
            resume_all: resolve(module, RESUME_ALL),
            visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
            visit_classes: resolve_visit_classes(module),
            get_class_descriptor: resolve(module, GET_CLASS_DESCRIPTOR),
            pretty_method: resolve_pretty_method(module),
            decode_method_id: resolve(module, DECODE_METHOD_ID),
            exception_clear: resolve_pointer(module, JNI_EXCEPTION_CLEAR),
            fatal_error: resolve_pointer(module, JNI_FATAL_ERROR),
            thread_transition: Arc::new(OnceLock::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            add_global_ref: None,
            suspend_all: None,
            resume_all: None,
            visit_class_loaders: None,
            visit_classes: None,
            get_class_descriptor: None,
            pretty_method: None,
            decode_method_id: None,
            exception_clear: None,
            fatal_error: None,
            thread_transition: Arc::new(OnceLock::new()),
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
        let thread_get_name = env.get_method(&thread_class, "getName", "()Ljava/lang/String;")?;
        let thread_is_alive = env.get_method(&thread_class, "isAlive", "()Z")?;
        let thread_current_thread =
            env.get_static_method(&thread_class, "currentThread", "()Ljava/lang/Thread;")?;
        let system_class = env.find_class("java/lang/System")?;
        let system_current_time_millis =
            env.get_static_method(&system_class, "currentTimeMillis", "()J")?;

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

    fn art_method_from_jni_id(
        &self,
        layout: &ArtRuntimeLayout,
        method_id: jni::jmethodID,
    ) -> Vec<*mut c_void> {
        let mut candidates = vec![method_id.cast::<c_void>()];
        if let Some(decode_method_id) = self.decode_method_id
            && !layout.jni_id_manager.is_null()
        {
            let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
            if !decoded.is_null() && !candidates.contains(&decoded) {
                candidates.push(decoded);
            }
        }
        candidates
    }

    fn with_runnable_art_thread(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let transition = self.thread_transition(env, feature)?;
        transition.run(feature, env, f)
    }

    fn thread_transition(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
    ) -> Result<&thread_transition::ThreadTransition> {
        if let Some(transition) = self.thread_transition.get() {
            return Ok(transition);
        }

        let transition =
            thread_transition::build(feature, env, self.exception_clear, self.fatal_error)?;
        let _ = self.thread_transition.set(transition);
        Ok(self
            .thread_transition
            .get()
            .expect("thread transition was just initialized"))
    }
}

impl ArtClassLoaderVisitor {
    fn new(loaders: &mut Vec<*mut c_void>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            loaders,
        }
    }

    fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class_loader as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }

    fn take_loaders(&mut self) -> Vec<*mut c_void> {
        let loaders = unsafe { &mut *self.loaders };
        std::mem::take(loaders)
    }
}

impl ArtClassVisitor {
    fn new_loaded(processor: &mut ArtClassProcessor<'_>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut ArtClassProcessor<'_>).cast(),
            visit: visit_loaded_class,
        }
    }

    fn new_finder(processor: &mut FindArtClassProcessor) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut FindArtClassProcessor).cast(),
            visit: visit_find_art_class,
        }
    }

    fn new_method_query(processor: &mut ArtMethodQueryProcessor<'_>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut ArtMethodQueryProcessor<'_>).cast(),
            visit: visit_method_query_class,
        }
    }

    fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }
}

impl<'callback> ArtClassProcessor<'callback> {
    fn new(
        add_global_ref: AddGlobalRef,
        get_class_descriptor: GetClassDescriptor,
        vm: &'callback Vm,
        thread: *mut c_void,
        classes: &'callback mut Vec<RawLoadedClass>,
    ) -> Self {
        Self {
            add_global_ref,
            get_class_descriptor,
            vm_handle: vm.handle().as_ptr(),
            thread,
            seen: HashSet::new(),
            classes,
            error: None,
        }
    }

    fn visit(&mut self, class: *mut c_void) -> bool {
        if !self.seen.insert(class as usize) {
            return true;
        }

        match self.promote(class) {
            Ok(class) => {
                self.classes.push(class);
                true
            }
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    fn take_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn promote(&self, class: *mut c_void) -> Result<RawLoadedClass> {
        let descriptor = class_descriptor_from_art(class, self.get_class_descriptor)?;
        let raw = unsafe { (self.add_global_ref)(self.vm_handle, self.thread, class) };
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JavaVMExt::AddGlobalRef",
            });
        }

        Ok(RawLoadedClass {
            name: class_name_from_descriptor(&descriptor),
            raw,
        })
    }
}

impl PrettyMethodFunction {
    fn call(&self, method: *mut c_void, with_signature: bool) -> Result<String> {
        let mut storage = ArtStdString { storage: [0; 3] };
        unsafe { (self.function)(&mut storage, method, with_signature) };
        let result = storage.to_string();
        storage.destroy();
        result
    }
}

impl FindArtClassProcessor {
    fn new(get_class_descriptor: GetClassDescriptor, descriptor: &'static str) -> Self {
        Self {
            get_class_descriptor,
            descriptor,
            class: None,
            error: None,
        }
    }

    fn visit(&mut self, class: *mut c_void) -> bool {
        let descriptor = match class_descriptor_from_art(class, self.get_class_descriptor) {
            Ok(descriptor) => descriptor,
            Err(error) => {
                self.error = Some(error);
                return false;
            }
        };
        if descriptor == self.descriptor {
            self.class = Some(class);
            false
        } else {
            true
        }
    }

    fn take_result(&mut self) -> Result<*mut c_void> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.class.ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_QUERY,
            reason: format!(
                "{} was not found by ClassLinker::VisitClasses",
                self.descriptor
            ),
        })
    }
}

impl<'callback> ArtMethodQueryProcessor<'callback> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        add_global_ref: AddGlobalRef,
        get_class_descriptor: GetClassDescriptor,
        pretty_method: PrettyMethodFunction,
        vm: &'callback Vm,
        thread: *mut c_void,
        query: &'callback metadata::MethodQuery,
        layout: ArtMethodQueryLayout,
        memory: &'callback MemoryRanges,
        groups: &'callback mut Vec<RawMethodQueryGroup>,
    ) -> Self {
        Self {
            add_global_ref,
            get_class_descriptor,
            pretty_method,
            vm_handle: vm.handle().as_ptr(),
            thread,
            query,
            layout,
            memory,
            seen_classes: HashSet::new(),
            groups,
            error: None,
        }
    }

    fn visit(&mut self, class: *mut c_void) -> bool {
        if !self.seen_classes.insert(class as usize) {
            return true;
        }

        match self.collect_class(class) {
            Ok(()) => true,
            Err(error) => {
                self.error = Some(error);
                false
            }
        }
    }

    fn take_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn collect_class(&mut self, class: *mut c_void) -> Result<()> {
        let loader_key = class_loader_key(class);
        if self.query.skip_system_classes && loader_key == 0 {
            return Ok(());
        }

        let descriptor = class_descriptor_from_art(class, self.get_class_descriptor)?;
        if !descriptor.starts_with('L') {
            return Ok(());
        }
        let class_name = class_name_from_descriptor(&descriptor);
        if self.query.skip_system_classes && metadata::is_platform_class(&class_name) {
            return Ok(());
        }

        let class_match_name = metadata::normalize_case(&class_name, self.query.ignore_case);
        if !metadata::glob_matches(&self.query.class_pattern, &class_match_name) {
            return Ok(());
        }

        let Some(methods_array) = read_art_array(
            class,
            self.layout.class_methods_offset,
            POINTER_SIZE,
            self.memory,
        ) else {
            return Ok(());
        };
        let copied_methods = read_u16(
            (class as usize + self.layout.class_copied_methods_offset) as *const c_void,
            self.memory,
        )
        .unwrap_or(0) as usize;
        let method_count = copied_methods.min(methods_array.length);
        if method_count == 0 || method_count > 10_000 {
            return Ok(());
        }

        let mut seen = HashSet::new();
        let mut methods = Vec::new();
        for index in 0..method_count {
            let method = unsafe { methods_array.data.byte_add(index * self.layout.method_size) };
            let access_flags = read_u32(
                (method as usize + self.layout.method_access_flags_offset) as *const c_void,
                self.memory,
            )
            .unwrap_or(0);

            let Some(metadata) =
                art_method_metadata(&class_name, method, access_flags, &self.pretty_method)?
            else {
                continue;
            };

            let display_name = metadata::query_method_name(&metadata, self.query.include_signature);
            if !self.query.include_signature && !seen.insert(display_name.clone()) {
                continue;
            }

            let method_match_name = metadata::normalize_case(&display_name, self.query.ignore_case);
            if metadata::glob_matches(&self.query.method_pattern, &method_match_name) {
                methods.push(metadata);
            }
        }

        if methods.is_empty() {
            return Ok(());
        }

        let group_index = self.find_or_add_group(loader_key)?;
        self.groups[group_index]
            .classes
            .push(metadata::JavaMethodQueryClass {
                name: class_name,
                methods,
            });
        Ok(())
    }

    fn find_or_add_group(&mut self, loader_key: u32) -> Result<usize> {
        if let Some(index) = self
            .groups
            .iter()
            .position(|group| group.loader_key == loader_key)
        {
            return Ok(index);
        }

        let loader = if loader_key == 0 {
            None
        } else {
            let raw = unsafe {
                (self.add_global_ref)(
                    self.vm_handle,
                    self.thread,
                    loader_key as usize as *mut c_void,
                )
            };
            if raw.is_null() {
                return Err(Error::NullReturn {
                    operation: "JavaVMExt::AddGlobalRef",
                });
            }
            Some(raw)
        };

        self.groups.push(RawMethodQueryGroup {
            loader_key,
            loader,
            classes: Vec::new(),
        });
        Ok(self.groups.len() - 1)
    }
}

fn java_class_from_loaded(vm: &Vm, class: RawLoadedClass) -> Result<JavaClass> {
    let global = unsafe { GlobalRef::<ClassKind>::from_raw(vm.clone(), class.raw)? };
    Ok(JavaClass::from_global(vm.clone(), class.name, global))
}

fn class_descriptor_from_art(
    class: *mut c_void,
    get_class_descriptor: GetClassDescriptor,
) -> Result<String> {
    let mut storage = ArtStdString { storage: [0; 3] };
    let descriptor = unsafe { get_class_descriptor(class, &mut storage) };
    if descriptor.is_null() {
        return Err(Error::NullReturn {
            operation: "art::mirror::Class::GetDescriptor",
        });
    }

    let descriptor = unsafe { CStr::from_ptr(descriptor) }
        .to_str()
        .map(str::to_owned)
        .map_err(Error::from);
    storage.destroy();
    descriptor
}

fn art_method_metadata(
    class_name: &str,
    method: *mut c_void,
    access_flags: u32,
    pretty_method: &PrettyMethodFunction,
) -> Result<Option<metadata::JavaMethodMetadata>> {
    let pretty = pretty_method
        .call(method, true)
        .map_err(|error| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_QUERY,
            reason: format!("ArtMethod::PrettyMethod failed: {error}"),
        })?;
    let Some((return_type, rest)) = pretty.split_once(' ') else {
        return unsupported_method_query(format!("unexpected PrettyMethod output: {pretty:?}"));
    };
    let prefix = format!("{class_name}.");
    let Some(name_and_arguments) = rest.strip_prefix(&prefix) else {
        return unsupported_method_query(format!(
            "PrettyMethod output {pretty:?} does not start with class {class_name:?}"
        ));
    };
    let Some(open_paren) = name_and_arguments.find('(') else {
        return unsupported_method_query(format!(
            "PrettyMethod output has no arguments: {pretty:?}"
        ));
    };
    let Some(arguments) = name_and_arguments.strip_suffix(')') else {
        return unsupported_method_query(format!(
            "PrettyMethod output has no closing ')': {pretty:?}"
        ));
    };
    let name = &name_and_arguments[..open_paren];
    let arguments = &arguments[open_paren + 1..];
    if name == "<clinit>" {
        return Ok(None);
    }

    let kind = if access_flags & K_ACC_CONSTRUCTOR != 0 {
        MethodKind::Constructor
    } else if access_flags & K_ACC_STATIC != 0 {
        MethodKind::Static
    } else {
        MethodKind::Instance
    };
    let name = if kind == MethodKind::Constructor {
        "<init>"
    } else {
        name
    };
    let signature =
        MethodSignature::from_pretty_types(return_type, arguments).map_err(|error| {
            Error::UnsupportedFeature {
                feature: FEATURE_METHOD_QUERY,
                reason: format!("unable to parse PrettyMethod signature {pretty:?}: {error}"),
            }
        })?;

    Ok(Some(metadata::JavaMethodMetadata {
        name: name.to_owned(),
        kind,
        signature,
        modifiers: (access_flags & 0xffff) as jni::jint,
        id: method.cast(),
    }))
}

fn detect_method_query_layout(
    visit_classes: VisitClassesKind,
    class_linker: *mut c_void,
    get_class_descriptor: GetClassDescriptor,
    thread_method_candidates: &[Vec<*mut c_void>],
    process_method_candidates: Vec<*mut c_void>,
    memory: &MemoryRanges,
) -> Result<ArtMethodQueryLayout> {
    let method_layout = detect_art_method_runtime_layout(&process_method_candidates, memory)?;
    let thread_class = find_art_class_by_descriptor(
        visit_classes,
        class_linker,
        get_class_descriptor,
        "Ljava/lang/Thread;",
    )?;
    let class_layout = detect_thread_class_method_layout(
        thread_class,
        thread_method_candidates,
        method_layout.method_size,
        memory,
    )?;
    Ok(ArtMethodQueryLayout {
        class_methods_offset: class_layout.class_methods_offset,
        class_copied_methods_offset: class_layout.class_copied_methods_offset,
        method_size: class_layout.method_size,
        method_access_flags_offset: method_layout.access_flags_offset,
    })
}

fn find_art_class_by_descriptor(
    visit_classes: VisitClassesKind,
    class_linker: *mut c_void,
    get_class_descriptor: GetClassDescriptor,
    descriptor: &'static str,
) -> Result<*mut c_void> {
    let mut processor = FindArtClassProcessor::new(get_class_descriptor, descriptor);
    match visit_classes {
        VisitClassesKind::Visitor(visit_classes) => {
            let mut visitor = ArtClassVisitor::new_finder(&mut processor);
            visitor.initialize_vtable();
            unsafe { visit_classes(class_linker, &mut visitor) };
        }
        VisitClassesKind::Callback(visit_classes) => unsafe {
            visit_classes(
                class_linker,
                on_visit_find_art_class_callback,
                (&mut processor as *mut FindArtClassProcessor).cast(),
            );
        },
    }
    processor.take_result()
}

unsafe extern "C" fn on_visit_find_art_class_callback(
    class: *mut c_void,
    context: *mut c_void,
) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_find_art_class(context, class) }
}

fn detect_thread_class_method_layout(
    thread_class: *mut c_void,
    method_candidates: &[Vec<*mut c_void>],
    method_size: usize,
    memory: &MemoryRanges,
) -> Result<ArtMethodQueryLayout> {
    for methods_offset in (0..CLASS_LAYOUT_SCAN_LIMIT).step_by(4) {
        let Some(array) = read_art_array(thread_class, methods_offset, POINTER_SIZE, memory) else {
            continue;
        };
        if array.length == 0 || array.length > ART_METHOD_ARRAY_MAX_PROBE {
            continue;
        }

        let Some(array_bytes) = array.length.checked_mul(method_size) else {
            continue;
        };
        if !memory.contains(array.data as usize, array_bytes) {
            continue;
        }
        if !art_method_array_contains_all(array, method_size, method_candidates) {
            continue;
        }

        let copied_methods_offset =
            detect_copied_methods_offset(thread_class, methods_offset, array.length, memory)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_QUERY,
                    reason: "unable to determine mirror::Class copied-method count offset"
                        .to_owned(),
                })?;
        return Ok(ArtMethodQueryLayout {
            class_methods_offset: methods_offset,
            class_copied_methods_offset: copied_methods_offset,
            method_size,
            method_access_flags_offset: 0,
        });
    }

    unsupported_method_query("unable to determine mirror::Class methods layout")
}

fn art_method_array_contains_all(
    array: ArtArray,
    method_size: usize,
    method_candidates: &[Vec<*mut c_void>],
) -> bool {
    method_candidates.iter().all(|candidates| {
        (0..array.length).any(|index| {
            let method = unsafe { array.data.byte_add(index * method_size) };
            candidates.contains(&method)
        })
    })
}

fn detect_copied_methods_offset(
    class: *mut c_void,
    methods_offset: usize,
    method_count: usize,
    memory: &MemoryRanges,
) -> Option<usize> {
    if method_count > u16::MAX as usize {
        return None;
    }
    for offset in (methods_offset..CLASS_LAYOUT_SCAN_LIMIT).step_by(4) {
        let value = read_u16((class as usize + offset) as *const c_void, memory)?;
        if value as usize == method_count {
            return Some(offset);
        }
    }
    None
}

fn detect_art_method_runtime_layout(
    method_candidates: &[*mut c_void],
    memory: &MemoryRanges,
) -> Result<ArtMethodRuntimeLayout> {
    let expected_native = 0x0001 | K_ACC_STATIC | K_ACC_NATIVE;
    let expected_final_native = expected_native | K_ACC_FINAL;
    let mask = !(K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_PUBLIC_API
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG);
    for &method in method_candidates {
        if method.is_null() || !memory.contains(method as usize, METHOD_LAYOUT_SCAN_LIMIT) {
            continue;
        }
        let mut access_flags_offset = None;
        for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(4) {
            let Some(flags) = read_u32((method as usize + offset) as *const c_void, memory) else {
                continue;
            };
            let relevant_flags = flags & mask;
            if relevant_flags == expected_native || relevant_flags == expected_final_native {
                access_flags_offset = Some(offset);
                break;
            }
        }

        let Some(access_flags_offset) = access_flags_offset else {
            continue;
        };
        let Some(method_size) = detect_art_method_size(method, memory) else {
            continue;
        };
        return Ok(ArtMethodRuntimeLayout {
            method_size,
            access_flags_offset,
        });
    }

    unsupported_method_query("unable to determine ArtMethod runtime layout")
}

fn detect_art_method_size(method: *mut c_void, memory: &MemoryRanges) -> Option<usize> {
    let mut previous_executable_pointer_offset = None;
    for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(POINTER_SIZE) {
        let value = read_usize((method as usize + offset) as *const c_void, memory)?;
        if !memory.contains_executable(value, 1) {
            continue;
        }

        if let Some(previous) = previous_executable_pointer_offset
            && offset == previous + POINTER_SIZE
        {
            let size = offset + POINTER_SIZE;
            if (ART_METHOD_MIN_SIZE..=ART_METHOD_MAX_SIZE).contains(&size) {
                return Some(size);
            }
        }
        previous_executable_pointer_offset = Some(offset);
    }

    None
}

fn read_art_array(
    object: *mut c_void,
    offset: usize,
    length_size: usize,
    memory: &MemoryRanges,
) -> Option<ArtArray> {
    let header = read_usize((object as usize + offset) as *const c_void, memory)? as *mut c_void;
    if header.is_null() || !memory.contains(header as usize, length_size) {
        return None;
    }

    let length = if length_size == 4 {
        read_u32(header.cast(), memory)? as usize
    } else {
        read_usize(header.cast(), memory)?
    };
    if length == 0 {
        return None;
    }

    let data = unsafe { header.byte_add(length_size) };
    Some(ArtArray { data, length })
}

fn read_usize(pointer: *const c_void, memory: &MemoryRanges) -> Option<usize> {
    if !memory.contains(pointer as usize, POINTER_SIZE) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(pointer.cast::<usize>()) })
}

fn read_u32(pointer: *const c_void, memory: &MemoryRanges) -> Option<u32> {
    if !memory.contains(pointer as usize, std::mem::size_of::<u32>()) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(pointer.cast::<u32>()) })
}

fn read_u16(pointer: *const c_void, memory: &MemoryRanges) -> Option<u16> {
    if !memory.contains(pointer as usize, std::mem::size_of::<u16>()) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(pointer.cast::<u16>()) })
}

fn class_loader_key(class: *mut c_void) -> u32 {
    unsafe { ptr::read_unaligned((class as usize + (2 * 4)) as *const u32) }
}

fn raw_method_groups_to_public(
    vm: &Vm,
    raw_groups: Vec<RawMethodQueryGroup>,
) -> Result<Vec<metadata::JavaMethodQueryGroup>> {
    let env = vm.attach_current_thread()?;
    let mut groups = Vec::with_capacity(raw_groups.len());
    let mut remaining_loaders = raw_groups
        .iter()
        .filter_map(|group| group.loader)
        .collect::<Vec<_>>();

    for group in raw_groups {
        if let Some(raw) = group.loader
            && let Some(index) = remaining_loaders.iter().position(|loader| *loader == raw)
        {
            remaining_loaders.remove(index);
        }

        let loader = match group.loader {
            Some(raw) => match unsafe {
                ClassLoaderRef::from_global_raw(vm.clone(), raw, ClassLoaderKind::Enumerated)
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
        groups.push(metadata::JavaMethodQueryGroup {
            loader,
            classes: group.classes,
        });
    }

    Ok(groups)
}

fn unsupported_method_query<T>(reason: impl Into<String>) -> Result<T> {
    unsupported_feature(FEATURE_METHOD_QUERY, reason)
}

impl ArtStdString {
    fn to_string(&self) -> Result<String> {
        let data = self.data();
        if data.is_null() {
            return Err(Error::NullReturn {
                operation: "std::string::c_str",
            });
        }
        unsafe { CStr::from_ptr(data) }
            .to_str()
            .map(str::to_owned)
            .map_err(Error::from)
    }

    fn data(&self) -> *const c_char {
        if self.storage[0] & 1 != 0 {
            self.storage[2] as *const c_char
        } else {
            (self as *const Self).cast::<u8>().wrapping_add(1).cast()
        }
    }

    fn destroy(&mut self) {
        if self.storage[0] & 1 != 0 {
            unsafe { free(self.storage[2] as *mut c_void) };
        }
    }
}

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

unsafe extern "C" fn on_visit_class_loader(
    visitor: *mut ArtClassLoaderVisitor,
    loader: *mut c_void,
) {
    if visitor.is_null() || loader.is_null() {
        return;
    }

    let visitor = unsafe { &mut *visitor };
    let loaders = unsafe { &mut *visitor.loaders };
    loaders.push(loader);
}

unsafe extern "C" fn on_visit_class(visitor: *mut ArtClassVisitor, class: *mut c_void) -> bool {
    if visitor.is_null() || class.is_null() {
        return true;
    }

    let visitor = unsafe { &mut *visitor };
    unsafe { (visitor.visit)(visitor.context, class) }
}

unsafe extern "C" fn on_visit_class_callback(class: *mut c_void, context: *mut c_void) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_loaded_class(context, class) }
}

unsafe extern "C" fn on_visit_method_query_callback(
    class: *mut c_void,
    context: *mut c_void,
) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_method_query_class(context, class) }
}

unsafe fn visit_loaded_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<ArtClassProcessor<'_>>() };
    processor.visit(class)
}

unsafe fn visit_find_art_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<FindArtClassProcessor>() };
    processor.visit(class)
}

unsafe fn visit_method_query_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<ArtMethodQueryProcessor<'_>>() };
    processor.visit(class)
}

struct SuspendedAllThreads {
    resume_all: ResumeAll,
    thread_list: *mut c_void,
}

impl SuspendedAllThreads {
    fn new(suspend_all: SuspendAll, resume_all: ResumeAll, thread_list: *mut c_void) -> Self {
        match suspend_all {
            SuspendAll::WithCause(suspend_all) => {
                static CAUSE: &CStr = c"frida";
                unsafe { suspend_all(thread_list, CAUSE.as_ptr(), false) };
            }
            SuspendAll::Legacy(suspend_all) => unsafe { suspend_all(thread_list) },
        }

        Self {
            resume_all,
            thread_list,
        }
    }
}

impl Drop for SuspendedAllThreads {
    fn drop(&mut self) {
        unsafe { (self.resume_all)(self.thread_list) };
    }
}

impl ExecutableMemory {
    #[cfg(target_arch = "aarch64")]
    fn aarch64_pretty_method_thunk(target: *const c_void) -> Result<Self> {
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;

        let length = 32;
        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                length,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return unsupported_method_query("unable to allocate PrettyMethod ABI thunk");
        }

        let mut code = [0u8; 32];
        write_u32_le(&mut code, 0, 0xaa0003e8); // mov x8, x0
        write_u32_le(&mut code, 4, 0xaa0103e0); // mov x0, x1
        write_u32_le(&mut code, 8, 0xaa0203e1); // mov x1, x2
        write_u32_le(&mut code, 12, 0x58000047); // ldr x7, #8
        write_u32_le(&mut code, 16, 0xd61f00e0); // br x7
        write_u64_le(&mut code, 20, target as usize as u64);

        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), pointer.cast::<u8>(), code.len());
            if mprotect(pointer, length, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, length);
                return unsupported_method_query("unable to protect PrettyMethod ABI thunk");
            }
        }

        let pointer = NonNull::new(pointer).ok_or(Error::NullReturn { operation: "mmap" })?;
        Ok(Self { pointer, length })
    }
}

impl Drop for ExecutableMemory {
    fn drop(&mut self) {
        unsafe {
            munmap(self.pointer.as_ptr(), self.length);
        }
    }
}

impl MemoryRanges {
    fn current() -> Result<Self> {
        let maps =
            fs::read_to_string("/proc/self/maps").map_err(|error| Error::UnsupportedFeature {
                feature: FEATURE_METHOD_QUERY,
                reason: format!("unable to read /proc/self/maps: {error}"),
            })?;
        let mut ranges = Vec::new();
        for line in maps.lines() {
            let mut columns = line.split_whitespace();
            let Some(addresses) = columns.next() else {
                continue;
            };
            let Some(perms) = columns.next() else {
                continue;
            };
            if !perms.starts_with('r') {
                continue;
            }
            let Some((start, end)) = addresses.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(end)) = (
                usize::from_str_radix(start, 16),
                usize::from_str_radix(end, 16),
            ) else {
                continue;
            };
            ranges.push(MemoryRange {
                start,
                end,
                executable: perms.as_bytes().get(2) == Some(&b'x'),
            });
        }
        Ok(Self { ranges })
    }

    fn contains(&self, address: usize, length: usize) -> bool {
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges
            .iter()
            .any(|range| address >= range.start && end <= range.end)
    }

    fn contains_executable(&self, address: usize, length: usize) -> bool {
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges
            .iter()
            .any(|range| range.executable && address >= range.start && end <= range.end)
    }
}

fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64_le(buffer: &mut [u8], offset: usize, value: u64) {
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

unsafe extern "C" {
    fn mmap(
        address: *mut c_void,
        length: usize,
        protection: c_int,
        flags: c_int,
        file_descriptor: c_int,
        offset: isize,
    ) -> *mut c_void;
    fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
    fn munmap(address: *mut c_void, length: usize) -> c_int;
}

fn detect_runtime_layout(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let api_level = android_api_level(feature)?;
    let runtime = art_runtime_from_vm(vm);
    detect_runtime_layout_from_runtime(api_level, runtime, vm.as_ptr() as usize, feature)
}

fn detect_runtime_layout_from_runtime(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    if api_level < 26 {
        return unsupported_feature(
            feature,
            format!("Android API level {api_level} is below the API 26+ arm64 milestone"),
        );
    }
    if runtime.is_null() {
        return unsupported_feature(feature, "ART Runtime pointer is null");
    }

    let runtime = runtime.cast::<usize>();
    for offset in (384..(384 + (100 * POINTER_SIZE))).step_by(POINTER_SIZE) {
        let value = unsafe { runtime.byte_add(offset).read() };
        if value != vm_value {
            continue;
        }

        for class_linker_offset in class_linker_offsets_for_api(api_level, offset) {
            if class_linker_offset < (2 * POINTER_SIZE) {
                continue;
            }

            let intern_table_offset = class_linker_offset - POINTER_SIZE;
            let thread_list_offset = intern_table_offset - POINTER_SIZE;
            let thread_list = unsafe { runtime.byte_add(thread_list_offset).read() as *mut c_void };
            let class_linker =
                unsafe { runtime.byte_add(class_linker_offset).read() as *mut c_void };
            let jni_id_manager = if api_level >= 30 {
                unsafe { runtime.byte_add(offset - POINTER_SIZE).read() as *mut c_void }
            } else {
                std::ptr::null_mut()
            };

            if thread_list.is_null() || class_linker.is_null() {
                continue;
            }

            return Ok(ArtRuntimeLayout {
                runtime: runtime.cast(),
                thread_list,
                class_linker,
                jni_id_manager,
            });
        }
    }

    unsupported_feature(feature, "unable to determine ART Runtime field offsets")
}

fn class_linker_offsets_for_api(api_level: i32, vm_offset: usize) -> Vec<usize> {
    if api_level >= 33 {
        vec![vm_offset - (4 * POINTER_SIZE)]
    } else if api_level >= 30 {
        vec![
            vm_offset - (3 * POINTER_SIZE),
            vm_offset - (4 * POINTER_SIZE),
        ]
    } else if api_level >= 29 {
        vec![vm_offset - (2 * POINTER_SIZE)]
    } else if api_level >= 27 {
        vec![vm_offset - STD_STRING_SIZE - (3 * POINTER_SIZE)]
    } else {
        vec![vm_offset - STD_STRING_SIZE - (2 * POINTER_SIZE)]
    }
}

fn android_api_level(feature: &'static str) -> Result<i32> {
    let name = CString::new("ro.build.version.sdk").expect("property name has no interior NUL");
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return unsupported_feature(feature, "unable to read ro.build.version.sdk");
    }

    let value = unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .map_err(|_| Error::UnsupportedFeature {
            feature,
            reason: "ro.build.version.sdk is not valid UTF-8".to_owned(),
        })?;

    value.parse().map_err(|_| Error::UnsupportedFeature {
        feature,
        reason: format!("ro.build.version.sdk is not an integer: {value:?}"),
    })
}

fn runtime_layout_support(vm: NonNull<jni::JavaVM>, feature: &'static str) -> FeatureSupport {
    match detect_runtime_layout(vm, feature) {
        Ok(_) => FeatureSupport::Supported,
        Err(Error::UnsupportedFeature { reason, .. }) => FeatureSupport::Unsupported { reason },
        Err(error) => FeatureSupport::Unsupported {
            reason: error.to_string(),
        },
    }
}

fn ensure_feature_supported(feature: &'static str, support: FeatureSupport) -> Result<()> {
    match support {
        FeatureSupport::Supported => Ok(()),
        FeatureSupport::Unsupported { reason } => unsupported_feature(feature, reason),
    }
}

fn unsupported_support(reason: impl Into<String>) -> FeatureSupport {
    FeatureSupport::Unsupported {
        reason: reason.into(),
    }
}

fn unsupported_feature<T>(feature: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}

fn resolve<T: Copy>(module: &Module, symbol: &'static str) -> Option<T> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .and_then(|pointer| native_pointer_to_fn(pointer).ok())
}

fn resolve_pointer(module: &Module, symbol: &'static str) -> Option<*const c_void> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .map(|pointer| pointer.0 as *const c_void)
}

fn resolve_pointer_any(module: &Module, symbols: &[&'static str]) -> Option<*const c_void> {
    symbols
        .iter()
        .find_map(|symbol| resolve_pointer(module, symbol))
}

fn resolve_any<T: Copy>(module: &Module, symbols: &[&'static str]) -> Option<T> {
    symbols.iter().find_map(|symbol| resolve(module, symbol))
}

fn resolve_pretty_method(module: &Module) -> Option<PrettyMethodFunction> {
    let pointer = resolve_pointer_any(module, &[PRETTY_METHOD, PRETTY_METHOD_NULL_SAFE])?;
    #[cfg(target_arch = "aarch64")]
    {
        let thunk = Arc::new(ExecutableMemory::aarch64_pretty_method_thunk(pointer).ok()?);
        let function = unsafe {
            std::mem::transmute_copy::<*mut c_void, PrettyMethod>(&thunk.pointer.as_ptr())
        };
        Some(PrettyMethodFunction {
            function,
            _thunk: Some(thunk),
        })
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let function = native_pointer_to_fn(frida_gum::NativePointer(pointer as usize)).ok()?;
        Some(PrettyMethodFunction {
            function,
            _thunk: None,
        })
    }
}

fn resolve_suspend_all(module: &Module) -> Option<SuspendAll> {
    resolve(module, SUSPEND_ALL_WITH_CAUSE)
        .map(SuspendAll::WithCause)
        .or_else(|| resolve(module, SUSPEND_ALL_LEGACY).map(SuspendAll::Legacy))
}

fn resolve_visit_classes(module: &Module) -> Option<VisitClassesKind> {
    resolve(module, VISIT_CLASSES_VISITOR)
        .map(VisitClassesKind::Visitor)
        .or_else(|| resolve(module, VISIT_CLASSES_CALLBACK).map(VisitClassesKind::Callback))
}

fn class_name_from_descriptor(descriptor: &str) -> String {
    if descriptor.starts_with('L') && descriptor.ends_with(';') {
        descriptor[1..descriptor.len() - 1].replace('/', ".")
    } else {
        descriptor.replace('/', ".")
    }
}

impl AsJObject for RawClass {
    fn as_jobject(&self) -> jni::jobject {
        self.0
    }
}

impl AsJClass for RawClass {
    fn as_jclass(&self) -> jni::jclass {
        self.0
    }
}

#[allow(dead_code)]
fn art_runtime_from_vm(vm: NonNull<jni::JavaVM>) -> *mut c_void {
    unsafe { vm.as_ptr().cast::<*mut c_void>().add(1).read() }
}

#[cfg(test)]
mod tests {
    use super::*;

    unsafe extern "C" fn dummy_add_global_ref(
        _vm: *mut jni::JavaVM,
        _thread: *mut c_void,
        _object: *mut c_void,
    ) -> jni::jobject {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn dummy_suspend_all(_thread_list: *mut c_void) {}

    unsafe extern "C" fn dummy_resume_all(_thread_list: *mut c_void) {}

    unsafe extern "C" fn dummy_visit_class_loaders(
        _class_linker: *mut c_void,
        _visitor: *mut ArtClassLoaderVisitor,
    ) {
    }

    unsafe extern "C" fn dummy_visit_classes(
        _class_linker: *mut c_void,
        _visitor: *mut ArtClassVisitor,
    ) {
    }

    #[test]
    fn derives_api_26_runtime_offsets() {
        let vm_offset = 512;
        assert_eq!(
            class_linker_offsets_for_api(26, vm_offset),
            vec![vm_offset - STD_STRING_SIZE - (2 * POINTER_SIZE)]
        );
    }

    #[test]
    fn derives_api_30_runtime_offset_candidates() {
        let vm_offset = 512;
        assert_eq!(
            class_linker_offsets_for_api(30, vm_offset),
            vec![
                vm_offset - (3 * POINTER_SIZE),
                vm_offset - (4 * POINTER_SIZE)
            ]
        );
    }

    #[test]
    fn detects_runtime_layout_from_supported_offsets() {
        let vm_offset = 512;
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];
        let vm_value = 0x1234usize;
        let thread_list = 0x2000usize as *mut c_void;
        let class_linker = 0x3000usize as *mut c_void;
        let jni_id_manager = 0x4000usize as *mut c_void;

        runtime[vm_offset / POINTER_SIZE] = vm_value;
        runtime[(vm_offset - POINTER_SIZE) / POINTER_SIZE] = jni_id_manager as usize;
        runtime[(vm_offset - (5 * POINTER_SIZE)) / POINTER_SIZE] = thread_list as usize;
        runtime[(vm_offset - (3 * POINTER_SIZE)) / POINTER_SIZE] = class_linker as usize;

        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                runtime.as_mut_ptr().cast(),
                vm_value,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Ok(ArtRuntimeLayout {
                runtime: runtime.as_mut_ptr().cast(),
                thread_list,
                class_linker,
                jni_id_manager,
            })
        );
    }

    #[test]
    fn detects_thread_class_method_array_layout_from_known_method() {
        let method_size = 24;
        let method_count = 3usize;
        let mut class_object = vec![0u8; CLASS_LAYOUT_SCAN_LIMIT];
        let mut methods = vec![0u8; POINTER_SIZE + (method_count * method_size)];
        let methods_offset = 32;
        let copied_methods_offset = 44;
        let methods_header = methods.as_mut_ptr() as usize;
        let known_method = unsafe {
            methods
                .as_mut_ptr()
                .byte_add(POINTER_SIZE + method_size)
                .cast::<c_void>()
        };
        class_object[methods_offset..methods_offset + POINTER_SIZE]
            .copy_from_slice(&methods_header.to_ne_bytes());
        methods[..POINTER_SIZE].copy_from_slice(&method_count.to_ne_bytes());
        class_object[copied_methods_offset..copied_methods_offset + 2]
            .copy_from_slice(&(method_count as u16).to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_object.as_ptr() as usize,
                    end: class_object.as_ptr() as usize + class_object.len(),
                    executable: false,
                },
                MemoryRange {
                    start: methods.as_ptr() as usize,
                    end: methods.as_ptr() as usize + methods.len(),
                    executable: false,
                },
            ],
        };

        let layout = detect_thread_class_method_layout(
            class_object.as_mut_ptr().cast(),
            &[vec![known_method]],
            method_size,
            &memory,
        )
        .unwrap();

        assert_eq!(layout.class_methods_offset, methods_offset);
        assert_eq!(layout.class_copied_methods_offset, copied_methods_offset);
        assert_eq!(layout.method_size, method_size);
    }

    #[test]
    fn rejects_pre_api_26_runtime_layout() {
        assert_eq!(
            detect_runtime_layout_from_runtime(
                25,
                std::ptr::dangling_mut::<usize>().cast(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "Android API level 25 is below the API 26+ arm64 milestone".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_null_runtime_layout() {
        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                std::ptr::null_mut(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "ART Runtime pointer is null".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_unknown_runtime_layout() {
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];

        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                runtime.as_mut_ptr().cast(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "unable to determine ART Runtime field offsets".to_owned(),
            })
        );
    }

    #[test]
    fn maps_unsupported_support_to_matching_feature_error() {
        assert_eq!(
            ensure_feature_supported(
                FEATURE_CLASS_LOADER_ENUMERATION,
                FeatureSupport::Unsupported {
                    reason: "test reason".to_owned(),
                },
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "test reason".to_owned(),
            })
        );
    }

    #[test]
    fn initializes_class_loader_visitor_vtable_after_placement() {
        let mut loaders = Vec::new();
        let mut visitor = ArtClassLoaderVisitor::new(&mut loaders);
        assert!(visitor.vtable.is_null());

        visitor.initialize_vtable();

        assert_eq!(visitor.vtable, visitor.vtable_storage.as_ptr());
        assert_eq!(
            visitor.vtable_storage[2],
            on_visit_class_loader as *const c_void
        );
    }

    #[test]
    fn reports_missing_visit_class_loaders_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "VisitClassLoaders is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_suspend_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ThreadList::SuspendAll is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_resume_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ThreadList::ResumeAll is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_visit_classes_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ClassLinker::VisitClasses is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_loaded_class_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_classes = Some(VisitClassesKind::Visitor(dummy_visit_classes));

        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn reports_non_arm64_architecture_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.visit_classes = Some(VisitClassesKind::Visitor(dummy_visit_classes));
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            }
        );
        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            }
        );
    }
}
