use std::{
    collections::HashSet,
    ffi::{CStr, c_void},
    ptr,
    sync::Arc,
};

use frida_gum::Module;

use super::{
    backend::ArtModuleRange,
    backend::{AddGlobalRef, GetClassDescriptor, PrettyMethod},
    features::*,
    layout::*,
    memory::{ExecutableMemory, MemoryRanges},
    strings::ArtStdString,
};
use crate::{
    env::{Env, MethodKind},
    error::{Error, Result},
    java::{JavaChooseControl, JavaObject, raw},
    jni, metadata,
    refs::{ClassKind, GlobalRef},
    signature::MethodSignature,
    vm::Vm,
};

#[repr(C)]
pub(super) struct ArtClassLoaderVisitor {
    pub(super) vtable: *const *const c_void,
    pub(super) vtable_storage: [*const c_void; 3],
    pub(super) loaders: *mut Vec<*mut c_void>,
}

#[repr(C)]
pub(super) struct ArtClassVisitor {
    pub(super) vtable: *const *const c_void,
    pub(super) vtable_storage: [*const c_void; 3],
    pub(super) context: *mut c_void,
    pub(super) visit: ArtRustClassCallback,
}

pub(super) struct RawHeapInstance(pub(super) jni::jobject);

pub(super) type ArtRustClassCallback = unsafe fn(*mut c_void, *mut c_void) -> bool;

pub(super) struct RawLoadedClass {
    name: String,
    pub(super) raw: jni::jclass,
}

pub(super) struct ArtClassProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    get_class_descriptor: GetClassDescriptor,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    seen: HashSet<usize>,
    classes: &'callback mut Vec<RawLoadedClass>,
    error: Option<Error>,
}

#[derive(Clone)]
pub(super) struct PrettyMethodFunction {
    pub(super) function: PrettyMethod,
    pub(super) _thunk: Option<Arc<ExecutableMemory>>,
}

pub(super) struct FindArtClassProcessor {
    get_class_descriptor: GetClassDescriptor,
    descriptor: &'static str,
    class: Option<*mut c_void>,
    error: Option<Error>,
}

pub(super) struct RawMethodQueryGroup {
    pub(super) loader_key: u32,
    pub(super) loader: Option<jni::jobject>,
    pub(super) classes: Vec<metadata::JavaMethodQueryClass>,
}

pub(super) struct ArtMethodQueryProcessor<'callback> {
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

pub(super) struct ArtHeapInstanceProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    needle_class_reference: u32,
    instances: &'callback mut Vec<RawHeapInstance>,
}

pub(super) use handle_scope::{ArtHandleVector, FakeVariableSizedHandleScope};

pub(super) unsafe extern "C" fn on_visit_class_loader(
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

pub(super) unsafe extern "C" fn on_visit_class(
    visitor: *mut ArtClassVisitor,
    class: *mut c_void,
) -> bool {
    if visitor.is_null() || class.is_null() {
        return true;
    }

    let visitor = unsafe { &mut *visitor };
    unsafe { (visitor.visit)(visitor.context, class) }
}

pub(super) unsafe extern "C" fn on_visit_class_callback(
    class: *mut c_void,
    context: *mut c_void,
) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_loaded_class(context, class) }
}

pub(super) unsafe extern "C" fn on_visit_method_query_callback(
    class: *mut c_void,
    context: *mut c_void,
) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_method_query_class(context, class) }
}

pub(super) unsafe extern "C" fn on_visit_heap_object(object: *mut c_void, context: *mut c_void) {
    if object.is_null() || context.is_null() {
        return;
    }

    let processor = unsafe { &mut *context.cast::<ArtHeapInstanceProcessor<'_>>() };
    processor.visit(object);
}

pub(super) unsafe fn visit_loaded_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<ArtClassProcessor<'_>>() };
    processor.visit(class)
}

pub(super) unsafe fn visit_find_art_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<FindArtClassProcessor>() };
    processor.visit(class)
}

pub(super) unsafe fn visit_method_query_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<ArtMethodQueryProcessor<'_>>() };
    processor.visit(class)
}

pub(super) fn class_name_from_descriptor(descriptor: &str) -> String {
    if descriptor.starts_with('L') && descriptor.ends_with(';') {
        descriptor[1..descriptor.len() - 1].replace('/', ".")
    } else {
        descriptor.replace('/', ".")
    }
}

impl ArtModuleRange {
    pub(crate) fn from_module(module: &Module) -> Self {
        let range = module.range();
        let start = range.base_address().0 as usize;
        let end = start.saturating_add(range.size());
        Self { start, end }
    }

    pub(super) fn contains(&self, address: usize) -> bool {
        let address = normalize_address(address);
        let start = normalize_address(self.start);
        let end = normalize_address(self.end);
        address >= start && address < end
    }
}

impl ArtClassLoaderVisitor {
    pub(super) fn new(loaders: &mut Vec<*mut c_void>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            loaders,
        }
    }

    pub(super) fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class_loader as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }

    pub(super) fn take_loaders(&mut self) -> Vec<*mut c_void> {
        let loaders = unsafe { &mut *self.loaders };
        std::mem::take(loaders)
    }
}

impl ArtClassVisitor {
    pub(super) fn new_loaded(processor: &mut ArtClassProcessor<'_>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut ArtClassProcessor<'_>).cast(),
            visit: visit_loaded_class,
        }
    }

    pub(super) fn new_finder(processor: &mut FindArtClassProcessor) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut FindArtClassProcessor).cast(),
            visit: visit_find_art_class,
        }
    }

    pub(super) fn new_method_query(processor: &mut ArtMethodQueryProcessor<'_>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut ArtMethodQueryProcessor<'_>).cast(),
            visit: visit_method_query_class,
        }
    }

    pub(super) fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }
}

impl<'callback> ArtClassProcessor<'callback> {
    pub(super) fn new(
        add_global_ref: AddGlobalRef,
        get_class_descriptor: GetClassDescriptor,
        vm: &'callback Vm,
        thread: *mut c_void,
        classes: &'callback mut Vec<RawLoadedClass>,
    ) -> Self {
        Self {
            add_global_ref,
            get_class_descriptor,
            // SAFETY: This processor only stores the live process JavaVM pointer for ART global
            // reference creation during the current enumeration pass.
            vm_handle: unsafe { vm.handle() }.as_ptr(),
            thread,
            seen: HashSet::new(),
            classes,
            error: None,
        }
    }

    pub(super) fn visit(&mut self, class: *mut c_void) -> bool {
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

    pub(super) fn take_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    pub(super) fn promote(&self, class: *mut c_void) -> Result<RawLoadedClass> {
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
    pub(super) fn call(&self, method: *mut c_void, with_signature: bool) -> Result<String> {
        let mut storage = ArtStdString { storage: [0; 3] };
        unsafe { (self.function)(&mut storage, method, with_signature) };
        let result = storage.to_string();
        storage.destroy();
        result
    }
}

impl FindArtClassProcessor {
    pub(super) fn new(get_class_descriptor: GetClassDescriptor, descriptor: &'static str) -> Self {
        Self {
            get_class_descriptor,
            descriptor,
            class: None,
            error: None,
        }
    }

    pub(super) fn visit(&mut self, class: *mut c_void) -> bool {
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

    pub(super) fn take_result(&mut self) -> Result<*mut c_void> {
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
    pub(super) fn new(
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
            // SAFETY: This processor only stores the live process JavaVM pointer for ART global
            // reference creation during the current method query pass.
            vm_handle: unsafe { vm.handle() }.as_ptr(),
            thread,
            query,
            layout,
            memory,
            seen_classes: HashSet::new(),
            groups,
            error: None,
        }
    }

    pub(super) fn visit(&mut self, class: *mut c_void) -> bool {
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

    pub(super) fn take_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    pub(super) fn collect_class(&mut self, class: *mut c_void) -> Result<()> {
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

impl<'callback> ArtHeapInstanceProcessor<'callback> {
    pub(super) fn new(
        add_global_ref: AddGlobalRef,
        vm: &'callback Vm,
        thread: *mut c_void,
        needle_class_reference: u32,
        instances: &'callback mut Vec<RawHeapInstance>,
    ) -> Self {
        Self {
            add_global_ref,
            // SAFETY: This processor only stores the live process JavaVM pointer for ART global
            // reference creation during the current heap enumeration pass.
            vm_handle: unsafe { vm.handle() }.as_ptr(),
            thread,
            needle_class_reference,
            instances,
        }
    }

    pub(super) fn visit(&mut self, object: *mut c_void) {
        if object.is_null() || object_class_reference(object) != self.needle_class_reference {
            return;
        }

        let raw = unsafe { (self.add_global_ref)(self.vm_handle, self.thread, object) };
        if !raw.is_null() {
            self.instances.push(RawHeapInstance(raw));
        }
    }
}

mod handle_scope {
    use super::*;

    // Heap::GetInstances requires ART Handle/VariableSizedHandleScope values, so this helper
    // temporarily links a synthetic scope into the current ART thread after validating the inferred
    // top handle-scope slot. Drop/dispose must restore the previous top handle-scope pointer.
    #[derive(Default)]
    pub(in crate::art) struct ArtHandleVector {
        storage: [usize; 3],
    }

    impl ArtHandleVector {
        pub(in crate::art) fn as_mut_ptr(&mut self) -> *mut c_void {
            self.storage.as_mut_ptr().cast()
        }

        pub(in crate::art) fn handles(&self) -> Vec<*mut c_void> {
            let begin = self.storage[0] as *mut *mut c_void;
            let end = self.storage[1] as *mut *mut c_void;
            if begin.is_null() || end.is_null() || (end as usize) < (begin as usize) {
                return Vec::new();
            }

            let count = ((end as usize) - (begin as usize)) / POINTER_SIZE;
            (0..count)
                .map(|index| unsafe { *begin.add(index) })
                .collect()
        }

        pub(in crate::art) fn dispose(&mut self) {
            let begin = self.storage[0] as *mut c_void;
            if !begin.is_null() {
                unsafe { free(begin) };
            }
            self.storage = [0; 3];
        }
    }

    pub(in crate::art) struct FakeVariableSizedHandleScope {
        scope: Box<[usize; 4]>,
        first_scope: Box<[usize; FIXED_HANDLE_SCOPE_WORDS]>,
        top_handle_scope: *mut *mut c_void,
        previous_top: *mut c_void,
    }

    impl FakeVariableSizedHandleScope {
        pub(in crate::art) fn new(
            thread: *mut c_void,
            env: *mut c_void,
            memory: &MemoryRanges,
        ) -> Result<Self> {
            let mut scope = Box::new([0usize; 4]);
            let mut first_scope = Box::new([0usize; FIXED_HANDLE_SCOPE_WORDS]);
            let top_handle_scope = thread_top_handle_scope(thread, env, memory)?;
            let previous_top = unsafe { *top_handle_scope };

            write_i32(
                first_scope.as_mut_ptr().cast(),
                BHS_OFFSET_NUM_REFS,
                FIXED_HANDLE_SCOPE_REFS,
            );
            write_u32(
                first_scope.as_mut_ptr().cast(),
                FIXED_HANDLE_SCOPE_POS_OFFSET,
                0,
            );

            scope[0] = previous_top as usize;
            write_i32(scope.as_mut_ptr().cast(), BHS_OFFSET_NUM_REFS, -1);
            scope[VSHS_OFFSET_SELF / POINTER_SIZE] = thread as usize;
            scope[VSHS_OFFSET_CURRENT_SCOPE / POINTER_SIZE] = first_scope.as_mut_ptr() as usize;

            unsafe { *top_handle_scope = scope.as_mut_ptr().cast() };

            Ok(Self {
                scope,
                first_scope,
                top_handle_scope,
                previous_top,
            })
        }

        pub(in crate::art) fn as_mut_ptr(&mut self) -> *mut c_void {
            self.scope.as_mut_ptr().cast()
        }

        pub(in crate::art) fn new_handle(&mut self, object: u32) -> Result<*mut c_void> {
            let scope = self.first_scope.as_mut_ptr().cast::<u8>();
            let position = unsafe {
                scope
                    .add(FIXED_HANDLE_SCOPE_POS_OFFSET)
                    .cast::<u32>()
                    .read()
            };
            if position >= FIXED_HANDLE_SCOPE_REFS as u32 {
                return Err(Error::UnsupportedFeature {
                    feature: FEATURE_HEAP_ENUMERATION,
                    reason:
                        "fake variable-sized handle scope exhausted while creating class handle"
                            .to_owned(),
                });
            }

            let handle = unsafe {
                scope
                    .add(FIXED_HANDLE_SCOPE_REFS_OFFSET + (position as usize * 4))
                    .cast::<u32>()
            };
            unsafe { handle.write(object) };
            write_u32(
                self.first_scope.as_mut_ptr().cast(),
                FIXED_HANDLE_SCOPE_POS_OFFSET,
                position + 1,
            );
            Ok(handle.cast())
        }

        pub(in crate::art) fn dispose(&mut self, _thread: *mut c_void) {
            if self.scope[VSHS_OFFSET_SELF / POINTER_SIZE] == 0 {
                return;
            }
            unsafe { *self.top_handle_scope = self.previous_top };

            let first = self.first_scope.as_mut_ptr().cast::<c_void>();
            let mut current = self.scope[VSHS_OFFSET_CURRENT_SCOPE / POINTER_SIZE] as *mut c_void;
            while !current.is_null() && current != first {
                let next = unsafe { *(current.cast::<*mut c_void>()) };
                unsafe { free(current) };
                current = next;
            }
            self.scope[VSHS_OFFSET_SELF / POINTER_SIZE] = 0;
        }
    }

    impl Drop for FakeVariableSizedHandleScope {
        fn drop(&mut self) {
            let thread = self.scope[VSHS_OFFSET_SELF / POINTER_SIZE] as *mut c_void;
            if !thread.is_null() {
                self.dispose(thread);
            }
        }
    }

    fn thread_top_handle_scope(
        thread: *mut c_void,
        env: *mut c_void,
        memory: &MemoryRanges,
    ) -> Result<*mut *mut c_void> {
        if thread.is_null() || env.is_null() {
            return Err(Error::UnsupportedFeature {
                feature: FEATURE_HEAP_ENUMERATION,
                reason: "ART Thread or JNIEnv pointer is null".to_owned(),
            });
        }
        let thread_words = thread.cast::<usize>();
        let env_value = env as usize;
        for offset in (144..256).step_by(POINTER_SIZE) {
            let word_address = thread as usize + offset;
            if !memory.contains(word_address, POINTER_SIZE) {
                continue;
            }
            let value = unsafe { thread_words.byte_add(offset).read() };
            if value == env_value {
                let top_handle_scope_offset = offset + (10 * POINTER_SIZE);
                let top_handle_scope_address = thread as usize + top_handle_scope_offset;
                if !memory.contains_writable(top_handle_scope_address, POINTER_SIZE) {
                    return Err(Error::UnsupportedFeature {
                        feature: FEATURE_HEAP_ENUMERATION,
                        reason: "ART Thread top handle-scope slot is not writable".to_owned(),
                    });
                }
                return Ok(unsafe {
                    thread
                        .cast::<u8>()
                        .add(top_handle_scope_offset)
                        .cast::<*mut c_void>()
                });
            }
        }

        Err(Error::UnsupportedFeature {
            feature: FEATURE_HEAP_ENUMERATION,
            reason: "unable to determine ArtThread top handle-scope offset".to_owned(),
        })
    }

    fn write_i32(base: *mut c_void, offset: usize, value: i32) {
        unsafe { base.cast::<u8>().add(offset).cast::<i32>().write(value) };
    }

    fn write_u32(base: *mut c_void, offset: usize, value: u32) {
        unsafe { base.cast::<u8>().add(offset).cast::<u32>().write(value) };
    }

    const BHS_OFFSET_NUM_REFS: usize = POINTER_SIZE;
    const FIXED_HANDLE_SCOPE_SIZE: usize = 64;
    const FIXED_HANDLE_SCOPE_WORDS: usize = FIXED_HANDLE_SCOPE_SIZE / POINTER_SIZE;
    const FIXED_HANDLE_SCOPE_REFS_OFFSET: usize = POINTER_SIZE + 4;
    const FIXED_HANDLE_SCOPE_REFS: i32 =
        ((FIXED_HANDLE_SCOPE_SIZE - POINTER_SIZE - 4 - 4) / 4) as i32;
    const FIXED_HANDLE_SCOPE_POS_OFFSET: usize =
        FIXED_HANDLE_SCOPE_REFS_OFFSET + (FIXED_HANDLE_SCOPE_REFS as usize * 4);
    const VSHS_OFFSET_SELF: usize = (POINTER_SIZE + 4).next_multiple_of(POINTER_SIZE);
    const VSHS_OFFSET_CURRENT_SCOPE: usize = VSHS_OFFSET_SELF + POINTER_SIZE;

    unsafe extern "C" {
        fn free(ptr: *mut c_void);
    }
}

pub(super) fn java_class_from_loaded(vm: &Vm, class: RawLoadedClass) -> Result<raw::Class> {
    let global = unsafe { GlobalRef::<ClassKind>::from_raw(vm.clone(), class.raw)? };
    Ok(raw::Class::from_global(vm.clone(), class.name, global))
}

pub(super) fn class_descriptor_from_art(
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

pub(super) fn art_method_metadata(
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

pub(super) fn deliver_heap_instances(
    vm: &Vm,
    env: &Env<'_>,
    mut raw_instances: Vec<RawHeapInstance>,
    callback: &mut dyn FnMut(&JavaObject) -> Result<JavaChooseControl>,
) -> Result<()> {
    raw_instances.reverse();
    while let Some(raw) = raw_instances.pop() {
        let object = match unsafe { JavaObject::from_global_raw_runtime(vm.clone(), raw.0) } {
            Ok(object) => object,
            Err(error) => {
                for remaining in raw_instances {
                    unsafe { env.delete_global_ref_raw(remaining.0) };
                }
                return Err(error);
            }
        };

        let control = callback(&object);
        drop(object);
        match control? {
            JavaChooseControl::Continue => {}
            JavaChooseControl::Stop => {
                for remaining in raw_instances {
                    unsafe { env.delete_global_ref_raw(remaining.0) };
                }
                return Ok(());
            }
        }
    }

    Ok(())
}

pub(super) fn object_class_reference(object: *mut c_void) -> u32 {
    unsafe { ptr::read_unaligned(object.cast::<u32>()) }
}
