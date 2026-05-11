use std::{
    mem,
    ptr::{self, NonNull},
    sync::Arc,
};

use frida_gum::{Gum, NativePointer, Process};

use crate::{
    error::{Error, Result},
    java::Java,
    jni,
    vm::Vm,
};

const JNI_GET_CREATED_JAVA_VMS: &str = "JNI_GetCreatedJavaVMs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFlavor {
    Art,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

pub(crate) struct RuntimeInner {
    pub(crate) _gum: Gum,
    pub(crate) vm: NonNull<jni::JavaVM>,
    pub(crate) flavor: RuntimeFlavor,
}

// JavaVM is a process-global JNI handle whose invocation table is immutable after VM creation.
// Env remains thread-affine; only the VM handle is shareable so callers can attach the current
// thread from whichever native thread they are running on.
unsafe impl Send for RuntimeInner {}
unsafe impl Sync for RuntimeInner {}

impl Runtime {
    pub fn obtain() -> Result<Self> {
        let gum = Gum::obtain();
        let process = Process::obtain(&gum);
        let art = process
            .enumerate_modules()
            .into_iter()
            .find(|module| {
                module.name() == "libart.so" && !module.path().contains("/system/fake-libs")
            })
            .ok_or(Error::ArtRuntimeNotFound)?;

        let get_created_java_vms = resolve_jni_get_created_java_vms(&art)?;
        let vm = get_created_java_vm(get_created_java_vms)?;

        Ok(Self {
            inner: Arc::new(RuntimeInner {
                _gum: gum,
                vm,
                flavor: RuntimeFlavor::Art,
            }),
        })
    }

    pub fn flavor(&self) -> RuntimeFlavor {
        self.inner.flavor
    }

    pub fn vm(&self) -> Vm {
        Vm::from_runtime(self.inner.clone())
    }

    pub fn java(&self) -> Java {
        Java::new(self.vm())
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

    native_pointer_to_fn(pointer)
}

fn get_created_java_vm(
    get_created_java_vms: jni::JNIGetCreatedJavaVMs,
) -> Result<NonNull<jni::JavaVM>> {
    let mut vm = ptr::null_mut();
    let mut count = 0;

    // SAFETY: The function pointer was resolved from ART's JNI_GetCreatedJavaVMs export.
    let result = unsafe { get_created_java_vms(&mut vm, 1, &mut count) };
    Error::jni_result(JNI_GET_CREATED_JAVA_VMS, result)?;

    if count == 0 {
        return Err(Error::NoCreatedJavaVm);
    }

    NonNull::new(vm).ok_or(Error::NoCreatedJavaVm)
}

fn native_pointer_to_fn<T: Copy>(pointer: NativePointer) -> Result<T> {
    debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<*mut std::ffi::c_void>());
    Ok(unsafe { mem::transmute_copy(&pointer.0) })
}
