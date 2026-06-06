use std::ptr::NonNull;

use super::{
    backend::ArtBackend,
    features::*,
    runtime_layout::{ensure_feature_supported, runtime_layout_support, unsupported_support},
};
use crate::{
    error::{Error, Result},
    jni,
    runtime::FeatureSupport,
    vm::Vm,
};

impl ArtBackend {
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

    pub(crate) fn method_replacement_support(&self, vm: &Vm) -> FeatureSupport {
        match self.detect_method_replacement_prerequisites(vm) {
            Ok(_) => FeatureSupport::Supported,
            Err(Error::UnsupportedFeature { reason, .. }) => unsupported_support(reason),
            Err(error) => unsupported_support(error.to_string()),
        }
    }

    pub(super) fn ensure_class_loader_enumeration_supported(
        &self,
        vm: NonNull<jni::JavaVM>,
    ) -> Result<()> {
        ensure_feature_supported(
            FEATURE_CLASS_LOADER_ENUMERATION,
            self.class_loader_enumeration_support(vm),
        )
    }

    pub(super) fn ensure_loaded_class_enumeration_supported(
        &self,
        vm: NonNull<jni::JavaVM>,
    ) -> Result<()> {
        ensure_feature_supported(
            FEATURE_LOADED_CLASS_ENUMERATION,
            self.loaded_class_enumeration_support(vm),
        )
    }

    pub(super) fn ensure_method_query_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(FEATURE_METHOD_QUERY, self.method_query_support(vm))
    }
}
