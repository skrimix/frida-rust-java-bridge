use super::*;
use super::{layout::*, support::*};

impl ArtModuleRange {
    pub(crate) fn from_module(module: &Module) -> Self {
        let range = module.range();
        let start = range.base_address().0 as usize;
        let end = start.saturating_add(range.size());
        Self { start, end }
    }

    pub(super) fn contains(&self, address: usize) -> bool {
        let address = normalize_address(address);
        address >= self.start && address < self.end
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
            vm_handle: vm.handle().as_ptr(),
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

pub(super) fn java_class_from_loaded(vm: &Vm, class: RawLoadedClass) -> Result<JavaClass> {
    let global = unsafe { GlobalRef::<ClassKind>::from_raw(vm.clone(), class.raw)? };
    Ok(JavaClass::from_global(vm.clone(), class.name, global))
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
