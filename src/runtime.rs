use std::{
    mem,
    ptr::{self, NonNull},
    sync::{Arc, OnceLock},
};

use frida_gum::{Gum, NativePointer, Process};

use crate::{
    art::{ArtBackend, ArtModuleRange},
    error::{Error, Result},
    java::{
        ClassLoaderRef, JavaClass, app_loader_deferral_support, main_thread_scheduling_support,
    },
    jni,
    metadata::JavaMethodQueryGroup,
    vm::Vm,
};

const JNI_GET_CREATED_JAVA_VMS: &str = "JNI_GetCreatedJavaVMs";
const HEAP_ENUMERATION_UNSUPPORTED: &str =
    "heap enumeration is outside the current loader/metadata prototype and is not implemented yet";
const DEOPTIMIZATION_UNSUPPORTED: &str =
    "deoptimization is outside the current loader/metadata prototype and is not implemented yet";

static PROCESS_GUM: OnceLock<Gum> = OnceLock::new();

pub(crate) fn process_gum() -> &'static Gum {
    PROCESS_GUM.get_or_init(Gum::obtain)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeFlavor {
    Art,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaCapabilities {
    pub flavor: RuntimeFlavor,
    pub class_loader_enumeration: FeatureSupport,
    pub loaded_class_enumeration: FeatureSupport,
    pub app_loader_deferral: FeatureSupport,
    pub main_thread_scheduling: FeatureSupport,
    pub heap_enumeration: FeatureSupport,
    pub deoptimization: FeatureSupport,
    pub method_replacement: FeatureSupport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureSupport {
    Supported,
    Experimental { reason: String },
    Unsupported { reason: String },
}

impl FeatureSupport {
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    pub fn is_experimental(&self) -> bool {
        matches!(self, Self::Experimental { .. })
    }

    pub fn experimental_reason(&self) -> Option<&str> {
        match self {
            Self::Experimental { reason } => Some(reason),
            Self::Supported | Self::Unsupported { .. } => None,
        }
    }

    pub fn unsupported_reason(&self) -> Option<&str> {
        match self {
            Self::Supported | Self::Experimental { .. } => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }

    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Experimental { reason } | Self::Unsupported { reason } => Some(reason),
        }
    }
}

#[derive(Clone)]
pub(crate) struct Runtime {
    inner: Arc<RuntimeInner>,
}

pub(crate) struct RuntimeInner {
    pub(crate) _gum: &'static Gum,
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
                flavor: RuntimeFlavor::Art,
                art: art_backend,
            }),
        })
    }

    pub(crate) fn vm(&self) -> Vm {
        Vm::from_runtime(self.inner.clone())
    }
}

impl RuntimeInner {
    pub(crate) fn capabilities(&self, vm: &Vm) -> JavaCapabilities {
        match self.flavor {
            RuntimeFlavor::Art => {
                let method_replacement = self.art.method_replacement_support(vm);
                JavaCapabilities {
                    flavor: RuntimeFlavor::Art,
                    class_loader_enumeration: self.art.class_loader_enumeration_support(self.vm),
                    loaded_class_enumeration: self.art.loaded_class_enumeration_support(self.vm),
                    app_loader_deferral: app_loader_deferral_support(vm, &method_replacement),
                    main_thread_scheduling: main_thread_scheduling_support(vm),
                    heap_enumeration: FeatureSupport::Unsupported {
                        reason: HEAP_ENUMERATION_UNSUPPORTED.to_owned(),
                    },
                    deoptimization: FeatureSupport::Unsupported {
                        reason: DEOPTIMIZATION_UNSUPPORTED.to_owned(),
                    },
                    method_replacement,
                }
            }
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
    Error::check_jni_result(JNI_GET_CREATED_JAVA_VMS, result)?;

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
            Some(
                "ART interpreter DoCall entrypoint is unavailable for cloned replacement dispatch"
            )
        );
        assert_eq!(
            capabilities.app_loader_deferral.unsupported_reason(),
            Some(
                "method replacement prerequisites are unavailable: ART interpreter DoCall entrypoint is unavailable for cloned replacement dispatch"
            )
        );
        assert_eq!(
            capabilities.main_thread_scheduling.unsupported_reason(),
            Some("Java VM handle is unavailable in unit tests")
        );
    }

    #[test]
    fn experimental_capability_reports_reason_without_stable_support() {
        let support = FeatureSupport::Experimental {
            reason: "prototype is available".to_owned(),
        };

        assert!(!support.is_supported());
        assert!(support.is_experimental());
        assert_eq!(
            support.experimental_reason(),
            Some("prototype is available")
        );
        assert_eq!(support.unsupported_reason(), None);
        assert_eq!(support.reason(), Some("prototype is available"));
    }
}
