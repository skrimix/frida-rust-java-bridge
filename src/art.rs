#![allow(dead_code)]

use std::{
    ffi::{c_char, c_void},
    ptr::NonNull,
};

use frida_gum::Module;

use crate::{
    error::{Error, Result},
    java::ClassLoaderRef,
    jni,
    runtime::native_pointer_to_fn,
    vm::Vm,
};

const FEATURE_CLASS_LOADER_ENUMERATION: &str = "ART class-loader enumeration";
const ADD_GLOBAL_REF_OBJ_PTR: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadENS_6ObjPtrINS_6mirror6ObjectEEE";
const ADD_GLOBAL_REF_POINTER: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadEPNS_6mirror6ObjectE";
const SUSPEND_ALL_WITH_CAUSE: &str = "_ZN3art10ThreadList10SuspendAllEPKcb";
const SUSPEND_ALL_LEGACY: &str = "_ZN3art10ThreadList10SuspendAllEv";
const RESUME_ALL: &str = "_ZN3art10ThreadList9ResumeAllEv";
const VISIT_CLASS_LOADERS: &str =
    "_ZNK3art11ClassLinker17VisitClassLoadersEPNS_18ClassLoaderVisitorE";

type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
type SuspendAllWithCause = unsafe extern "C" fn(*mut c_void, *const c_char, bool);
type SuspendAllLegacy = unsafe extern "C" fn(*mut c_void);
type ResumeAll = unsafe extern "C" fn(*mut c_void);
type VisitClassLoaders = unsafe extern "C" fn(*mut c_void, *mut ArtClassLoaderVisitor);

#[derive(Clone)]
pub(crate) struct ArtBackend {
    add_global_ref: Option<AddGlobalRef>,
    suspend_all: Option<SuspendAll>,
    resume_all: Option<ResumeAll>,
    visit_class_loaders: Option<VisitClassLoaders>,
}

#[derive(Clone, Copy)]
enum SuspendAll {
    WithCause(SuspendAllWithCause),
    Legacy(SuspendAllLegacy),
}

#[repr(C)]
struct ArtClassLoaderVisitor {
    vtable: *const *const c_void,
    on_visit: Option<unsafe extern "C" fn(*mut ArtClassLoaderVisitor, *mut c_void)>,
    loaders: *mut Vec<*mut c_void>,
}

impl ArtBackend {
    pub(crate) fn from_module(module: &Module) -> Self {
        Self {
            add_global_ref: resolve_any(module, &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER]),
            suspend_all: resolve_suspend_all(module),
            resume_all: resolve(module, RESUME_ALL),
            visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            add_global_ref: None,
            suspend_all: None,
            resume_all: None,
            visit_class_loaders: None,
        }
    }

    pub(crate) fn enumerate_class_loaders(&self, vm: &Vm) -> Result<Vec<ClassLoaderRef>> {
        self.ensure_symbols()?;
        let _ = vm;

        Err(Error::UnsupportedFeature {
            feature: FEATURE_CLASS_LOADER_ENUMERATION,
            reason: "safe ART thread-state transition support is not implemented yet".to_owned(),
        })
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
}

fn unsupported<T>(reason: impl Into<String>) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature: FEATURE_CLASS_LOADER_ENUMERATION,
        reason: reason.into(),
    })
}

fn resolve<T: Copy>(module: &Module, symbol: &'static str) -> Option<T> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .and_then(|pointer| native_pointer_to_fn(pointer).ok())
}

fn resolve_any<T: Copy>(module: &Module, symbols: &[&'static str]) -> Option<T> {
    symbols.iter().find_map(|symbol| resolve(module, symbol))
}

fn resolve_suspend_all(module: &Module) -> Option<SuspendAll> {
    resolve(module, SUSPEND_ALL_WITH_CAUSE)
        .map(SuspendAll::WithCause)
        .or_else(|| resolve(module, SUSPEND_ALL_LEGACY).map(SuspendAll::Legacy))
}

#[allow(dead_code)]
fn art_runtime_from_vm(vm: NonNull<jni::JavaVM>) -> *mut c_void {
    unsafe { vm.as_ptr().cast::<*mut c_void>().add(1).read() }
}
