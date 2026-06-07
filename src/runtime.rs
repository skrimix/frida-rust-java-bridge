use std::{
    ptr::{self, NonNull},
    sync::Arc,
};

use frida_gum::{Gum, Process};

use crate::{
    art::{ArtBackend, ArtModuleRange},
    error::{Error, Result},
    jni,
    native::{native_pointer_to_fn, process_gum},
};

const JNI_GET_CREATED_JAVA_VMS: &str = "JNI_GetCreatedJavaVMs";

#[derive(Clone)]
pub(crate) struct Runtime {
    inner: Arc<RuntimeInner>,
}

pub(crate) struct RuntimeInner {
    pub(crate) _gum: &'static Gum,
    pub(crate) vm: NonNull<jni::JavaVM>,
    pub(crate) art: ArtBackend,
}

// JavaVM is a process-global JNI handle whose invocation table is immutable after VM creation.
// Env remains thread-affine; only the VM handle is shareable so callers can attach the current
// thread from whichever native thread they are running on.
unsafe impl Send for RuntimeInner {}
unsafe impl Sync for RuntimeInner {}

impl Runtime {
    pub(crate) fn obtain() -> Result<Self> {
        let gum = process_gum();
        let process = Process::obtain(gum);
        let modules = process.enumerate_modules();
        let art = modules
            .iter()
            .find(|module| {
                module.name() == "libart.so" && !module.path().contains("/system/fake-libs")
            })
            .ok_or(Error::ArtRuntimeNotFound)?;
        let android_runtime = modules
            .iter()
            .find(|module| module.name() == "libandroid_runtime.so")
            .map(ArtModuleRange::from_module);

        let get_created_java_vms = resolve_jni_get_created_java_vms(art)?;
        let vm = get_created_java_vm(get_created_java_vms)?;

        let art_backend = ArtBackend::from_module(art, android_runtime);

        Ok(Self {
            inner: Arc::new(RuntimeInner {
                _gum: gum,
                vm,
                art: art_backend,
            }),
        })
    }

    pub(crate) fn into_inner(self) -> Arc<RuntimeInner> {
        self.inner
    }
}

fn resolve_jni_get_created_java_vms(
    module: &frida_gum::Module,
) -> Result<jni::JNIGetCreatedJavaVMs> {
    let pointer = module
        .find_export_by_name(JNI_GET_CREATED_JAVA_VMS)
        .or_else(|| module.find_symbol_by_name(JNI_GET_CREATED_JAVA_VMS))
        .ok_or_else(|| Error::SymbolNotFound {
            module: module.name(),
            symbol: JNI_GET_CREATED_JAVA_VMS,
        })?;

    Ok(native_pointer_to_fn(pointer))
}

fn get_created_java_vm(
    get_created_java_vms: jni::JNIGetCreatedJavaVMs,
) -> Result<NonNull<jni::JavaVM>> {
    let mut vm = ptr::null_mut();
    let mut count = 0;

    // SAFETY: The function pointer was resolved from ART's JNI_GetCreatedJavaVMs export.
    let result = unsafe { get_created_java_vms(&mut vm, 1, &mut count) };
    Error::check_jni_result(JNI_GET_CREATED_JAVA_VMS, result)?;

    if count == 0 {
        return Err(Error::NoCreatedJavaVm);
    }

    NonNull::new(vm).ok_or(Error::NoCreatedJavaVm)
}
