#![allow(dead_code)]

use std::{
    collections::HashSet,
    ffi::{CStr, CString, c_char, c_void},
    ptr::NonNull,
    sync::{Arc, OnceLock},
};

use frida_gum::Module;

use crate::{
    error::{Error, Result},
    java::{ClassLoaderRef, JavaClass},
    jni, metadata,
    refs::{AsJClass, AsJObject, GlobalRef},
    runtime::native_pointer_to_fn,
    vm::Vm,
};

mod thread_transition;

const FEATURE_CLASS_LOADER_ENUMERATION: &str = "ART class-loader enumeration";
const FEATURE_LOADED_CLASS_ENUMERATION: &str = "ART loaded-class enumeration";
const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const STD_STRING_SIZE: usize = 3 * POINTER_SIZE;
const PROP_VALUE_MAX: usize = 92;
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
const JNI_EXCEPTION_CLEAR: &str = "_ZN3art3JNIILb1EE14ExceptionClearEP7_JNIEnv";
const JNI_FATAL_ERROR: &str = "_ZN3art3JNIILb1EE10FatalErrorEP7_JNIEnvPKc";

type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
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
    classes: *mut Vec<*mut c_void>,
}

struct RawClass(jni::jclass);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtRuntimeLayout {
    thread_list: *mut c_void,
    class_linker: *mut c_void,
}

impl ArtBackend {
    pub(crate) fn from_module(module: &Module) -> Self {
        Self {
            add_global_ref: resolve_any(module, &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER]),
            suspend_all: resolve_suspend_all(module),
            resume_all: resolve(module, RESUME_ALL),
            visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
            visit_classes: resolve_visit_classes(module),
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
            exception_clear: None,
            fatal_error: None,
            thread_transition: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn enumerate_class_loaders(&self, vm: &Vm) -> Result<Vec<ClassLoaderRef>> {
        self.ensure_symbols()?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm.handle(), FEATURE_CLASS_LOADER_ENUMERATION)?;
        let mut loader_globals = Vec::new();

        self.with_runnable_art_thread(&env, |thread| {
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
        self.ensure_loaded_class_symbols()?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm.handle(), FEATURE_LOADED_CLASS_ENUMERATION)?;
        let mut class_globals = Vec::new();

        self.with_runnable_art_thread(&env, |thread| {
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before class enumeration");
            let visit_classes = self
                .visit_classes
                .expect("visit_classes symbol checked before class enumeration");
            let mut class_objects = Vec::new();

            match visit_classes {
                VisitClassesKind::Visitor(visit_classes) => {
                    let mut visitor = ArtClassVisitor::new(&mut class_objects);
                    visitor.initialize_vtable();
                    unsafe { visit_classes(layout.class_linker, &mut visitor) };
                    class_objects = visitor.take_classes();
                }
                VisitClassesKind::Callback(visit_classes) => unsafe {
                    visit_classes(
                        layout.class_linker,
                        on_visit_class_callback,
                        (&mut class_objects as *mut Vec<*mut c_void>).cast(),
                    );
                },
            }

            let vm_handle = vm.handle().as_ptr();
            let mut seen = HashSet::new();
            for class in class_objects {
                if !seen.insert(class as usize) {
                    continue;
                }
                let global = unsafe { add_global_ref(vm_handle, thread, class) };
                if global.is_null() {
                    return Err(Error::NullReturn {
                        operation: "JavaVMExt::AddGlobalRef",
                    });
                }
                class_globals.push(global);
            }

            Ok(())
        })?;

        class_globals
            .into_iter()
            .map(|raw| {
                let raw_class = RawClass(raw);
                let descriptor = metadata::class_descriptor(&env, &raw_class)?;
                let name = class_name_from_descriptor(&descriptor);
                let global = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
                Ok(JavaClass::from_global(vm.clone(), name, global))
            })
            .collect()
    }

    fn ensure_symbols(&self) -> Result<()> {
        if self.visit_class_loaders.is_none() {
            return unsupported("VisitClassLoaders is unavailable");
        }
        if self.add_global_ref.is_none() {
            return unsupported("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.suspend_all.is_none() {
            return unsupported("ThreadList::SuspendAll is unavailable");
        }
        if self.resume_all.is_none() {
            return unsupported("ThreadList::ResumeAll is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported("only arm64-v8a is supported in this milestone");
        }
        Ok(())
    }

    fn ensure_loaded_class_symbols(&self) -> Result<()> {
        if self.visit_classes.is_none() {
            return unsupported_feature(
                FEATURE_LOADED_CLASS_ENUMERATION,
                "ClassLinker::VisitClasses is unavailable",
            );
        }
        if self.add_global_ref.is_none() {
            return unsupported_feature(
                FEATURE_LOADED_CLASS_ENUMERATION,
                "JavaVMExt::AddGlobalRef is unavailable",
            );
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_feature(
                FEATURE_LOADED_CLASS_ENUMERATION,
                "only arm64-v8a is supported in this milestone",
            );
        }
        Ok(())
    }

    fn with_runnable_art_thread(
        &self,
        env: &crate::env::Env<'_>,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let transition = self.thread_transition(env)?;
        transition.run(env, f)
    }

    fn thread_transition(
        &self,
        env: &crate::env::Env<'_>,
    ) -> Result<&thread_transition::ThreadTransition> {
        if let Some(transition) = self.thread_transition.get() {
            return Ok(transition);
        }

        let transition = thread_transition::build(env, self.exception_clear, self.fatal_error)?;
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
    fn new(classes: &mut Vec<*mut c_void>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            classes,
        }
    }

    fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }

    fn take_classes(&mut self) -> Vec<*mut c_void> {
        let classes = unsafe { &mut *self.classes };
        std::mem::take(classes)
    }
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
    let classes = unsafe { &mut *visitor.classes };
    classes.push(class);
    true
}

unsafe extern "C" fn on_visit_class_callback(class: *mut c_void, context: *mut c_void) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    let classes = unsafe { &mut *context.cast::<Vec<*mut c_void>>() };
    classes.push(class);
    true
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

fn detect_runtime_layout(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let api_level = android_api_level()?;
    if api_level < 26 {
        return unsupported_feature(
            feature,
            format!("Android API level {api_level} is below the API 26+ arm64 milestone"),
        );
    }

    let runtime = art_runtime_from_vm(vm);
    if runtime.is_null() {
        return unsupported_feature(feature, "ART Runtime pointer is null");
    }

    let runtime = runtime.cast::<usize>();
    let vm_value = vm.as_ptr() as usize;
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

            if thread_list.is_null() || class_linker.is_null() {
                continue;
            }

            return Ok(ArtRuntimeLayout {
                thread_list,
                class_linker,
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

fn android_api_level() -> Result<i32> {
    let name = CString::new("ro.build.version.sdk").expect("property name has no interior NUL");
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return unsupported("unable to read ro.build.version.sdk");
    }

    let value = unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .map_err(|_| Error::UnsupportedFeature {
            feature: FEATURE_CLASS_LOADER_ENUMERATION,
            reason: "ro.build.version.sdk is not valid UTF-8".to_owned(),
        })?;

    value.parse().map_err(|_| Error::UnsupportedFeature {
        feature: FEATURE_CLASS_LOADER_ENUMERATION,
        reason: format!("ro.build.version.sdk is not an integer: {value:?}"),
    })
}

fn unsupported<T>(reason: impl Into<String>) -> Result<T> {
    unsupported_feature(FEATURE_CLASS_LOADER_ENUMERATION, reason)
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

fn resolve_any<T: Copy>(module: &Module, symbols: &[&'static str]) -> Option<T> {
    symbols.iter().find_map(|symbol| resolve(module, symbol))
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
        descriptor[1..descriptor.len() - 1].to_owned()
    } else {
        descriptor.to_owned()
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
            backend.ensure_symbols(),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "VisitClassLoaders is unavailable".to_owned(),
            })
        );
    }

    #[test]
    fn reports_missing_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);

        assert_eq!(
            backend.ensure_symbols(),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            })
        );
    }

    #[test]
    fn reports_missing_suspend_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);

        assert_eq!(
            backend.ensure_symbols(),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "ThreadList::SuspendAll is unavailable".to_owned(),
            })
        );
    }

    #[test]
    fn reports_missing_resume_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));

        assert_eq!(
            backend.ensure_symbols(),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "ThreadList::ResumeAll is unavailable".to_owned(),
            })
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn accepts_complete_arm64_class_loader_symbol_set() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);

        assert_eq!(backend.ensure_symbols(), Ok(()));
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn reports_non_arm64_architecture_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);

        assert_eq!(
            backend.ensure_symbols(),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            })
        );
    }
}
