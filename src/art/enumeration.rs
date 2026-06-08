use std::{
    collections::HashSet,
    ffi::{CStr, c_void},
    ptr::{self, NonNull},
    sync::Arc,
};

use frida_gum::Module;

use super::{
    ArtVmAccess,
    backend::{
        AddGlobalRef, ArtBackend, ArtModuleRange, DecodeGlobalKind, GetClassDescriptor,
        GetInstancesKind, PrettyMethod, VisitClassesKind, VisitObjects,
    },
    features::*,
    layout::*,
    memory::{ExecutableMemory, MemoryRanges},
    runtime_layout::{
        detect_runtime_layout, ensure_feature_supported, runtime_layout_support,
        unsupported_support,
    },
    strings::ArtStdString,
    threads::SuspendedAllThreads,
};
use crate::{
    capabilities::FeatureSupport,
    env::Env,
    env::MethodKind,
    error::{Error, Result},
    jni, method_query,
    signature::{MethodSignature, class_name_from_descriptor},
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

pub(crate) struct ArtHeapInstanceHandle {
    pub(crate) raw: jni::jobject,
}

pub(super) type ArtRustClassCallback = unsafe fn(*mut c_void, *mut c_void) -> bool;

pub(crate) struct ArtClassLoaderHandle {
    pub(crate) raw: jni::jobject,
}

pub(crate) struct ArtLoadedClassHandle {
    pub(crate) name: String,
    pub(crate) raw: jni::jclass,
}

pub(super) struct ArtClassProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    get_class_descriptor: GetClassDescriptor,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    seen: HashSet<usize>,
    classes: &'callback mut Vec<ArtLoadedClassHandle>,
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

pub(crate) struct ArtMethodQueryGroup {
    pub(super) loader_key: u32,
    pub(crate) loader: Option<jni::jobject>,
    pub(crate) classes: Vec<ArtMethodQueryClass>,
}

pub(crate) struct ArtMethodQueryClass {
    pub(crate) name: String,
    pub(crate) methods: Vec<ArtMethodMetadata>,
}

pub(crate) struct ArtMethodMetadata {
    pub(crate) name: String,
    pub(crate) kind: MethodKind,
    pub(crate) signature: MethodSignature,
    pub(crate) modifiers: jni::jint,
    pub(crate) id: jni::jmethodID,
}

pub(super) struct ArtMethodQueryProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    get_class_descriptor: GetClassDescriptor,
    pretty_method: PrettyMethodFunction,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    query: &'callback method_query::MethodQuery,
    layout: ArtMethodQueryLayout,
    memory: &'callback MemoryRanges,
    seen_classes: HashSet<usize>,
    groups: &'callback mut Vec<ArtMethodQueryGroup>,
    error: Option<Error>,
}

pub(super) struct ArtHeapInstanceProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    needle_class_reference: u32,
    instances: &'callback mut Vec<ArtHeapInstanceHandle>,
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
        vm_handle: NonNull<jni::JavaVM>,
        thread: *mut c_void,
        classes: &'callback mut Vec<ArtLoadedClassHandle>,
    ) -> Self {
        Self {
            add_global_ref,
            get_class_descriptor,
            // SAFETY: This processor only stores the live process JavaVM pointer for ART global
            // reference creation during the current enumeration pass.
            vm_handle: vm_handle.as_ptr(),
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

    pub(super) fn promote(&self, class: *mut c_void) -> Result<ArtLoadedClassHandle> {
        let descriptor = class_descriptor_from_art(class, self.get_class_descriptor)?;
        let raw = unsafe { (self.add_global_ref)(self.vm_handle, self.thread, class) };
        if raw.is_null() {
            return Err(Error::NullReturn {
                operation: "JavaVMExt::AddGlobalRef",
            });
        }

        Ok(ArtLoadedClassHandle {
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
        vm_handle: NonNull<jni::JavaVM>,
        thread: *mut c_void,
        query: &'callback method_query::MethodQuery,
        layout: ArtMethodQueryLayout,
        memory: &'callback MemoryRanges,
        groups: &'callback mut Vec<ArtMethodQueryGroup>,
    ) -> Self {
        Self {
            add_global_ref,
            get_class_descriptor,
            pretty_method,
            // SAFETY: This processor only stores the live process JavaVM pointer for ART global
            // reference creation during the current method query pass.
            vm_handle: vm_handle.as_ptr(),
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
        if self.query.skip_system_classes && method_query::is_platform_class(&class_name) {
            return Ok(());
        }

        let class_match_name = method_query::normalize_case(&class_name, self.query.ignore_case);
        if !method_query::glob_matches(&self.query.class_pattern, &class_match_name) {
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

            let display_name = method_query::query_method_name(
                metadata.kind,
                &metadata.name,
                &metadata.signature,
                self.query.include_signature,
            );
            if !self.query.include_signature && !seen.insert(display_name.clone()) {
                continue;
            }

            let method_match_name =
                method_query::normalize_case(&display_name, self.query.ignore_case);
            if method_query::glob_matches(&self.query.method_pattern, &method_match_name) {
                methods.push(metadata);
            }
        }

        if methods.is_empty() {
            return Ok(());
        }

        let group_index = self.find_or_add_group(loader_key)?;
        self.groups[group_index].classes.push(ArtMethodQueryClass {
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

        self.groups.push(ArtMethodQueryGroup {
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
        vm_handle: NonNull<jni::JavaVM>,
        thread: *mut c_void,
        needle_class_reference: u32,
        instances: &'callback mut Vec<ArtHeapInstanceHandle>,
    ) -> Self {
        Self {
            add_global_ref,
            // SAFETY: This processor only stores the live process JavaVM pointer for ART global
            // reference creation during the current heap enumeration pass.
            vm_handle: vm_handle.as_ptr(),
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
            self.instances.push(ArtHeapInstanceHandle { raw });
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

impl ArtBackend {
    pub(crate) fn enumerate_class_loader_handles(
        &self,
        vm: &impl ArtVmAccess,
    ) -> Result<Vec<ArtClassLoaderHandle>> {
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
                .common
                .add_global_ref
                .expect("add_global_ref symbol checked before enumeration");
            let visit_class_loaders = self
                .enumeration
                .visit_class_loaders
                .expect("visit_class_loaders symbol checked before enumeration");
            let mut loader_objects = Vec::new();
            let mut visitor = ArtClassLoaderVisitor::new(&mut loader_objects);
            visitor.initialize_vtable();

            let _suspended = SuspendedAllThreads::new(
                self.common
                    .suspend_all
                    .expect("suspend_all symbol checked before enumeration"),
                self.common
                    .resume_all
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
                // VisitClassLoaders for this process ART runtime. AddGlobalRef turns it into a JNI global handle.
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

        Ok(loader_globals
            .into_iter()
            .map(|raw| ArtClassLoaderHandle { raw })
            .collect())
    }

    pub(crate) fn class_loader_enumeration_support(
        &self,
        vm: NonNull<jni::JavaVM>,
    ) -> FeatureSupport {
        if self.enumeration.visit_class_loaders.is_none() {
            return unsupported_support("VisitClassLoaders is unavailable");
        }
        if self.common.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.common.suspend_all.is_none() {
            return unsupported_support("ThreadList::SuspendAll is unavailable");
        }
        if self.common.resume_all.is_none() {
            return unsupported_support("ThreadList::ResumeAll is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_CLASS_LOADER_ENUMERATION)
    }

    fn ensure_class_loader_enumeration_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(
            FEATURE_CLASS_LOADER_ENUMERATION,
            self.class_loader_enumeration_support(vm),
        )
    }

    pub(crate) fn enumerate_loaded_class_handles(
        &self,
        vm: &impl ArtVmAccess,
    ) -> Result<Vec<ArtLoadedClassHandle>> {
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
                .common
                .add_global_ref
                .expect("add_global_ref symbol checked before class enumeration");
            let visit_classes = self
                .enumeration
                .visit_classes
                .expect("visit_classes symbol checked before class enumeration");
            let get_class_descriptor = self
                .enumeration
                .get_class_descriptor
                .expect("get_class_descriptor symbol checked before class enumeration");
            let mut processor = ArtClassProcessor::new(
                add_global_ref,
                get_class_descriptor,
                vm_handle,
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

        Ok(class_globals)
    }

    pub(crate) fn loaded_class_enumeration_support(
        &self,
        vm: NonNull<jni::JavaVM>,
    ) -> FeatureSupport {
        if self.enumeration.visit_classes.is_none() {
            return unsupported_support("ClassLinker::VisitClasses is unavailable");
        }
        if self.common.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.enumeration.get_class_descriptor.is_none() {
            return unsupported_support("mirror::Class::GetDescriptor is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_LOADED_CLASS_ENUMERATION)
    }

    fn ensure_loaded_class_enumeration_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(
            FEATURE_LOADED_CLASS_ENUMERATION,
            self.loaded_class_enumeration_support(vm),
        )
    }

    pub(crate) fn enumerate_methods(
        &self,
        vm: &impl ArtVmAccess,
        query: &method_query::MethodQuery,
    ) -> Result<Vec<ArtMethodQueryGroup>> {
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
                .enumeration
                .visit_classes
                .expect("visit_classes symbol checked before method query");
            let add_global_ref = self
                .common
                .add_global_ref
                .expect("add_global_ref symbol checked before method query");
            let get_class_descriptor = self
                .enumeration
                .get_class_descriptor
                .expect("get_class_descriptor symbol checked before method query");
            let pretty_method = self
                .enumeration
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
                vm_handle,
                thread,
                query,
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

        Ok(raw_groups)
    }

    pub(crate) fn method_query_support(&self, vm: NonNull<jni::JavaVM>) -> FeatureSupport {
        if self.enumeration.visit_classes.is_none() {
            return unsupported_support("ClassLinker::VisitClasses is unavailable");
        }
        if self.common.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.enumeration.get_class_descriptor.is_none() {
            return unsupported_support("mirror::Class::GetDescriptor is unavailable");
        }
        if self.enumeration.pretty_method.is_none() {
            return unsupported_support("ArtMethod::PrettyMethod is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_METHOD_QUERY)
    }

    fn ensure_method_query_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(FEATURE_METHOD_QUERY, self.method_query_support(vm))
    }

    pub(crate) fn enumerate_heap_instance_handles(
        &self,
        vm: &impl ArtVmAccess,
        class: jni::jobject,
    ) -> Result<Vec<ArtHeapInstanceHandle>> {
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
                let needle = self.decode_global_object_reference(vm, thread, class)?;
                match self.heap.visit_objects {
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
                            self.heap
                                .get_instances
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
                unsafe { env.delete_global_ref_raw(raw.raw) };
            }
            return Err(error);
        }

        Ok(raw_instances)
    }

    pub(crate) fn heap_enumeration_support(&self, vm: NonNull<jni::JavaVM>) -> FeatureSupport {
        if self.heap.visit_objects.is_none() && self.heap.get_instances.is_none() {
            return unsupported_support(
                "Heap::VisitObjects and Heap::GetInstances are unavailable",
            );
        }
        if self.common.add_global_ref.is_none() {
            return unsupported_support("JavaVMExt::AddGlobalRef is unavailable");
        }
        if self.heap.decode_global.is_none() {
            return unsupported_support("JavaVMExt::DecodeGlobal is unavailable");
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_support("only arm64-v8a is supported in this milestone");
        }
        runtime_layout_support(vm, FEATURE_HEAP_ENUMERATION)
    }

    fn choose_instances_with_visit_objects(
        &self,
        vm: &impl ArtVmAccess,
        thread: *mut c_void,
        layout: &ArtRuntimeLayout,
        needle_class_reference: u32,
        visit_objects: VisitObjects,
        instances: &mut Vec<ArtHeapInstanceHandle>,
    ) -> Result<()> {
        let add_global_ref = self
            .common
            .add_global_ref
            .expect("add_global_ref symbol checked before heap enumeration");
        let mut processor = ArtHeapInstanceProcessor::new(
            add_global_ref,
            unsafe { vm.handle() },
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
        vm: &impl ArtVmAccess,
        env: &Env<'_>,
        thread: *mut c_void,
        layout: &ArtRuntimeLayout,
        needle_class_reference: u32,
        get_instances: GetInstancesKind,
        instances: &mut Vec<ArtHeapInstanceHandle>,
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
            instances.push(ArtHeapInstanceHandle { raw });
        }
        vector.dispose();
        scope.dispose(thread);
        Ok(())
    }

    fn decode_global_object_reference(
        &self,
        vm: &impl ArtVmAccess,
        thread: *mut c_void,
        object: jni::jobject,
    ) -> Result<u32> {
        let decode_global = self
            .heap
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
) -> Result<Option<ArtMethodMetadata>> {
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

    Ok(Some(ArtMethodMetadata {
        name: name.to_owned(),
        kind,
        signature,
        modifiers: (access_flags & 0xffff) as jni::jint,
        id: method.cast(),
    }))
}

pub(super) fn object_class_reference(object: *mut c_void) -> u32 {
    unsafe { ptr::read_unaligned(object.cast::<u32>()) }
}
