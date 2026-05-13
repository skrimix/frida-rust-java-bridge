use std::{
    mem,
    ptr::{self, NonNull},
    sync::Arc,
};

use frida_gum::{Gum, NativePointer, Process};

use crate::{
    art::{ArtBackend, ArtModuleRange},
    error::{Error, Result},
    java::{ClassLoaderRef, Java, JavaClass},
    jni,
    metadata::JavaMethodQueryGroup,
    vm::Vm,
};

const JNI_GET_CREATED_JAVA_VMS: &str = "JNI_GetCreatedJavaVMs";
const HEAP_ENUMERATION_UNSUPPORTED: &str =
    "heap enumeration is outside loader/metadata V1 and is not implemented yet";
const DEOPTIMIZATION_UNSUPPORTED: &str =
    "deoptimization is outside loader/metadata V1 and is not implemented yet";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFlavor {
    Art,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeCapabilities {
    pub flavor: RuntimeFlavor,
    pub class_loader_enumeration: FeatureSupport,
    pub loaded_class_enumeration: FeatureSupport,
    pub heap_enumeration: FeatureSupport,
    pub deoptimization: FeatureSupport,
    pub method_replacement: FeatureSupport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureSupport {
    Supported,
    Unsupported { reason: String },
}

impl FeatureSupport {
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    pub fn unsupported_reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

pub(crate) struct RuntimeInner {
    pub(crate) _gum: Gum,
    pub(crate) vm: NonNull<jni::JavaVM>,
    pub(crate) flavor: RuntimeFlavor,
    pub(crate) art: ArtBackend,
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
                flavor: RuntimeFlavor::Art,
                art: art_backend,
            }),
        })
    }

    pub fn flavor(&self) -> RuntimeFlavor {
        self.inner.flavor
    }

    pub fn capabilities(&self) -> RuntimeCapabilities {
        self.inner.capabilities(&self.vm())
    }

    pub fn vm(&self) -> Vm {
        Vm::from_runtime(self.inner.clone())
    }

    pub fn java(&self) -> Java {
        Java::new(self.vm())
    }

    pub fn enumerate_class_loaders(&self) -> Result<Vec<ClassLoaderRef>> {
        self.inner.enumerate_class_loaders(&self.vm())
    }

    pub fn enumerate_loaded_classes(&self) -> Result<Vec<JavaClass>> {
        self.inner.enumerate_loaded_classes(&self.vm())
    }

    pub fn enumerate_methods(&self, query: &str) -> Result<Vec<JavaMethodQueryGroup>> {
        self.inner.enumerate_methods(&self.vm(), query)
    }
}

impl RuntimeInner {
    pub(crate) fn capabilities(&self, vm: &Vm) -> RuntimeCapabilities {
        match self.flavor {
            RuntimeFlavor::Art => RuntimeCapabilities {
                flavor: RuntimeFlavor::Art,
                class_loader_enumeration: self.art.class_loader_enumeration_support(self.vm),
                loaded_class_enumeration: self.art.loaded_class_enumeration_support(self.vm),
                heap_enumeration: FeatureSupport::Unsupported {
                    reason: HEAP_ENUMERATION_UNSUPPORTED.to_owned(),
                },
                deoptimization: FeatureSupport::Unsupported {
                    reason: DEOPTIMIZATION_UNSUPPORTED.to_owned(),
                },
                method_replacement: self.art.method_replacement_support(vm),
            },
        }
    }

    pub(crate) fn enumerate_class_loaders(&self, vm: &Vm) -> Result<Vec<ClassLoaderRef>> {
        match self.flavor {
            RuntimeFlavor::Art => self.art.enumerate_class_loaders(vm),
        }
    }

    pub(crate) fn enumerate_loaded_classes(&self, vm: &Vm) -> Result<Vec<JavaClass>> {
        match self.flavor {
            RuntimeFlavor::Art => self.art.enumerate_loaded_classes(vm),
        }
    }

    pub(crate) fn enumerate_methods(
        &self,
        vm: &Vm,
        query: &str,
    ) -> Result<Vec<JavaMethodQueryGroup>> {
        match self.flavor {
            RuntimeFlavor::Art => self.art.enumerate_methods(vm, query),
        }
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

pub(crate) fn native_pointer_to_fn<T: Copy>(pointer: NativePointer) -> Result<T> {
    debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<*mut std::ffi::c_void>());
    Ok(unsafe { mem::transmute_copy(&pointer.0) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_capability_reasons_name_deferred_features() {
        let capabilities = Vm::dangling_for_tests().capabilities();

        assert_eq!(
            capabilities.heap_enumeration.unsupported_reason(),
            Some(HEAP_ENUMERATION_UNSUPPORTED)
        );
        assert_eq!(
            capabilities.deoptimization.unsupported_reason(),
            Some(DEOPTIMIZATION_UNSUPPORTED)
        );
        assert_eq!(
            capabilities.method_replacement.unsupported_reason(),
            Some("ArtMethod::PrettyMethod is unavailable")
        );
    }
}
