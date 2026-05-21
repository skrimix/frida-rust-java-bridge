use super::layout::*;
use super::*;

impl ArtStdString {
    pub(super) fn to_string(&self) -> Result<String> {
        let data = self.data();
        if data.is_null() {
            return Err(Error::NullReturn {
                operation: "std::string::c_str",
            });
        }
        unsafe { CStr::from_ptr(data) }
            .to_str()
            .map(str::to_owned)
            .map_err(Error::from)
    }

    pub(super) fn data(&self) -> *const c_char {
        if self.storage[0] & 1 != 0 {
            self.storage[2] as *const c_char
        } else {
            (self as *const Self).cast::<u8>().wrapping_add(1).cast()
        }
    }

    pub(super) fn destroy(&mut self) {
        if self.storage[0] & 1 != 0 {
            unsafe { free(self.storage[2] as *mut c_void) };
        }
    }
}

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

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

pub(super) struct SuspendedAllThreads {
    resume_all: ResumeAll,
    thread_list: *mut c_void,
}

impl SuspendedAllThreads {
    pub(super) fn new(
        suspend_all: SuspendAll,
        resume_all: ResumeAll,
        thread_list: *mut c_void,
    ) -> Self {
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

impl ExecutableMemory {
    #[cfg(target_arch = "aarch64")]
    pub(super) fn aarch64_pretty_method_thunk(target: *const c_void) -> Result<Self> {
        let _gum = crate::runtime::process_gum();
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;

        let length = 32;
        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                length,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return unsupported_method_query("unable to allocate PrettyMethod ABI thunk");
        }

        let mut code = [0u8; 32];
        write_u32_le(&mut code, 0, 0xaa0003e8); // mov x8, x0
        write_u32_le(&mut code, 4, 0xaa0103e0); // mov x0, x1
        write_u32_le(&mut code, 8, 0xaa0203e1); // mov x1, x2
        write_u32_le(&mut code, 12, 0x58000047); // ldr x7, #8
        write_u32_le(&mut code, 16, 0xd61f00e0); // br x7
        write_u64_le(&mut code, 20, target as usize as u64);

        unsafe {
            ptr::copy_nonoverlapping(code.as_ptr(), pointer.cast::<u8>(), code.len());
            frida_gum_sys::gum_clear_cache(pointer, length as u64);
            if mprotect(pointer, length, PROT_READ | PROT_EXEC) != 0 {
                munmap(pointer, length);
                return unsupported_method_query("unable to protect PrettyMethod ABI thunk");
            }
        }

        let pointer = NonNull::new(pointer).ok_or(Error::NullReturn { operation: "mmap" })?;
        Ok(Self { pointer, length })
    }
}

impl Drop for ExecutableMemory {
    fn drop(&mut self) {
        unsafe {
            munmap(self.pointer.as_ptr(), self.length);
        }
    }
}

impl MemoryRanges {
    pub(super) fn current() -> Result<Self> {
        Self::current_for_feature(FEATURE_METHOD_QUERY)
    }

    pub(super) fn current_for_feature(feature: &'static str) -> Result<Self> {
        let maps =
            fs::read_to_string("/proc/self/maps").map_err(|error| Error::UnsupportedFeature {
                feature,
                reason: format!("unable to read /proc/self/maps: {error}"),
            })?;
        let mut ranges = Vec::new();
        for line in maps.lines() {
            let mut columns = line.split_whitespace();
            let Some(addresses) = columns.next() else {
                continue;
            };
            let Some(perms) = columns.next() else {
                continue;
            };
            if !perms.starts_with('r') {
                continue;
            }
            let Some((start, end)) = addresses.split_once('-') else {
                continue;
            };
            let (Ok(start), Ok(end)) = (
                usize::from_str_radix(start, 16),
                usize::from_str_radix(end, 16),
            ) else {
                continue;
            };
            ranges.push(MemoryRange {
                start,
                end,
                executable: perms.as_bytes().get(2) == Some(&b'x'),
            });
        }
        Ok(Self { ranges })
    }

    pub(super) fn contains(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges.iter().any(|range| {
            let range_start = normalize_address(range.start);
            let range_end = normalize_address(range.end);
            address >= range_start && end <= range_end
        })
    }

    pub(super) fn contains_executable(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges.iter().any(|range| {
            let range_start = normalize_address(range.start);
            let range_end = normalize_address(range.end);
            range.executable && address >= range_start && end <= range_end
        })
    }
}

pub(super) fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

pub(super) fn write_u64_le(buffer: &mut [u8], offset: usize, value: u64) {
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

unsafe extern "C" {
    pub(super) fn mmap(
        address: *mut c_void,
        length: usize,
        protection: c_int,
        flags: c_int,
        file_descriptor: c_int,
        offset: isize,
    ) -> *mut c_void;
    pub(super) fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
    pub(super) fn munmap(address: *mut c_void, length: usize) -> c_int;
}

pub(super) fn detect_runtime_layout(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let api_level = android_api_level(feature)?;
    detect_runtime_layout_for_api(vm, api_level, feature)
}

pub(super) fn detect_runtime_layout_for_api(
    vm: NonNull<jni::JavaVM>,
    api_level: i32,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let runtime = art_runtime_from_vm(vm);
    detect_runtime_layout_from_runtime(api_level, runtime, vm.as_ptr() as usize, feature)
}

pub(super) fn detect_runtime_layout_for_method_replacement(
    vm: NonNull<jni::JavaVM>,
    api_level: i32,
    set_jni_id_type: Option<*const c_void>,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<(ArtRuntimeLayout, ArtClassLinkerTrampolines)> {
    let runtime = art_runtime_from_vm(vm);
    detect_runtime_layout_and_trampolines_from_runtime(
        api_level,
        runtime,
        vm.as_ptr() as usize,
        set_jni_id_type,
        predicates,
        memory,
        feature,
    )
}

pub(super) fn detect_runtime_layout_from_runtime(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    if api_level < 26 {
        return unsupported_feature(
            feature,
            format!("Android API level {api_level} is below the API 26+ arm64 milestone"),
        );
    }
    if runtime.is_null() {
        return unsupported_feature(feature, "ART Runtime pointer is null");
    }

    let runtime = runtime.cast::<usize>();
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
            let heap_offset = heap_offset_for_api(api_level, thread_list_offset);
            if heap_offset >= thread_list_offset {
                continue;
            }
            let heap = unsafe { runtime.byte_add(heap_offset).read() as *mut c_void };
            let thread_list = unsafe { runtime.byte_add(thread_list_offset).read() as *mut c_void };
            let class_linker =
                unsafe { runtime.byte_add(class_linker_offset).read() as *mut c_void };
            let intern_table =
                unsafe { runtime.byte_add(intern_table_offset).read() as *mut c_void };
            let jni_id_manager = if api_level >= 30 {
                unsafe { runtime.byte_add(offset - POINTER_SIZE).read() as *mut c_void }
            } else {
                std::ptr::null_mut()
            };

            if heap.is_null()
                || thread_list.is_null()
                || class_linker.is_null()
                || intern_table.is_null()
            {
                continue;
            }

            return Ok(ArtRuntimeLayout {
                runtime: runtime.cast(),
                heap,
                thread_list,
                class_linker,
                intern_table,
                jni_id_manager,
                jni_ids_indirection: None,
            });
        }
    }

    unsupported_feature(feature, "unable to determine ART Runtime field offsets")
}

pub(super) fn detect_runtime_layout_and_trampolines_from_runtime(
    api_level: i32,
    runtime: *mut c_void,
    vm_value: usize,
    set_jni_id_type: Option<*const c_void>,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<(ArtRuntimeLayout, ArtClassLinkerTrampolines)> {
    if api_level < 26 {
        return unsupported_feature(
            feature,
            format!("Android API level {api_level} is below the API 26+ arm64 milestone"),
        );
    }
    if runtime.is_null() {
        return unsupported_feature(feature, "ART Runtime pointer is null");
    }

    let runtime = runtime.cast::<usize>();
    let mut found_vm = false;
    let mut candidate_failure = None;
    for offset in (384..(384 + (100 * POINTER_SIZE))).step_by(POINTER_SIZE) {
        let value = unsafe { runtime.byte_add(offset).read() };
        if value != vm_value {
            continue;
        }
        found_vm = true;

        for class_linker_offset in class_linker_offsets_for_api(api_level, offset) {
            if class_linker_offset < (2 * POINTER_SIZE) {
                continue;
            }

            let intern_table_offset = class_linker_offset - POINTER_SIZE;
            let thread_list_offset = intern_table_offset - POINTER_SIZE;
            let heap_offset = heap_offset_for_api(api_level, thread_list_offset);
            if heap_offset >= thread_list_offset {
                continue;
            }
            let heap = unsafe { runtime.byte_add(heap_offset).read() as *mut c_void };
            let thread_list = unsafe { runtime.byte_add(thread_list_offset).read() as *mut c_void };
            let class_linker =
                unsafe { runtime.byte_add(class_linker_offset).read() as *mut c_void };
            let intern_table =
                unsafe { runtime.byte_add(intern_table_offset).read() as *mut c_void };
            let jni_id_manager = if api_level >= 30 {
                unsafe { runtime.byte_add(offset - POINTER_SIZE).read() as *mut c_void }
            } else {
                std::ptr::null_mut()
            };

            if heap.is_null()
                || thread_list.is_null()
                || class_linker.is_null()
                || intern_table.is_null()
            {
                continue;
            }

            let jni_ids_indirection =
                detect_jni_ids_indirection(runtime.cast(), set_jni_id_type, memory, feature)?;

            let layout = ArtRuntimeLayout {
                runtime: runtime.cast(),
                heap,
                thread_list,
                class_linker,
                intern_table,
                jni_id_manager,
                jni_ids_indirection,
            };

            match detect_class_linker_trampolines(&layout, api_level, predicates, memory) {
                Ok(trampolines) => return Ok((layout, trampolines)),
                Err(Error::UnsupportedFeature { reason, .. }) => {
                    candidate_failure.get_or_insert(reason);
                }
                Err(error) => {
                    candidate_failure.get_or_insert(error.to_string());
                }
            }
        }
    }

    if let Some(reason) = candidate_failure {
        return unsupported_feature(feature, reason);
    }
    if found_vm {
        return unsupported_feature(
            feature,
            "unable to determine ART Runtime field offsets: no non-null ClassLinker candidates",
        );
    }

    unsupported_feature(feature, "unable to determine ART Runtime field offsets")
}

pub(super) fn class_linker_offsets_for_api(api_level: i32, vm_offset: usize) -> Vec<usize> {
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

pub(super) fn heap_offset_for_api(api_level: i32, thread_list_offset: usize) -> usize {
    if api_level >= 34 {
        thread_list_offset.saturating_sub(9 * POINTER_SIZE)
    } else if api_level >= 24 {
        thread_list_offset.saturating_sub(8 * POINTER_SIZE)
    } else if api_level >= 23 {
        thread_list_offset.saturating_sub(7 * POINTER_SIZE)
    } else {
        thread_list_offset.saturating_sub(4 * POINTER_SIZE)
    }
}

pub(super) fn detect_jni_ids_indirection(
    runtime: *mut c_void,
    set_jni_id_type: Option<*const c_void>,
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<Option<i32>> {
    let Some(set_jni_id_type) = set_jni_id_type else {
        return Ok(None);
    };
    let Some(offset) = detect_jni_ids_indirection_offset(feature, set_jni_id_type)? else {
        return Ok(None);
    };
    Ok(read_u32((runtime as usize + offset) as *const c_void, memory).map(|value| value as i32))
}

#[cfg(target_arch = "aarch64")]
pub(super) fn detect_jni_ids_indirection_offset(
    feature: &'static str,
    set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    runnable_thread::detect_jni_ids_indirection_offset(feature, set_jni_id_type)
}

#[cfg(not(target_arch = "aarch64"))]
pub(super) fn detect_jni_ids_indirection_offset(
    _feature: &'static str,
    _set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    Ok(None)
}

pub(super) fn android_api_level(feature: &'static str) -> Result<i32> {
    crate::android::android_api_level_for_feature(feature)
}

pub(super) fn runtime_layout_support(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> FeatureSupport {
    match detect_runtime_layout(vm, feature) {
        Ok(_) => FeatureSupport::Supported,
        Err(Error::UnsupportedFeature { reason, .. }) => FeatureSupport::Unsupported { reason },
        Err(error) => FeatureSupport::Unsupported {
            reason: error.to_string(),
        },
    }
}

pub(super) fn ensure_feature_supported(
    feature: &'static str,
    support: FeatureSupport,
) -> Result<()> {
    match support {
        FeatureSupport::Supported => Ok(()),
        FeatureSupport::Unsupported { reason } => unsupported_feature(feature, reason),
    }
}

pub(super) fn unsupported_support(reason: impl Into<String>) -> FeatureSupport {
    FeatureSupport::Unsupported {
        reason: reason.into(),
    }
}

pub(super) fn unsupported_feature<T>(
    feature: &'static str,
    reason: impl Into<String>,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}

pub(super) fn resolve<T: Copy>(module: &Module, symbol: &'static str) -> Option<T> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .and_then(|pointer| native_pointer_to_fn(pointer).ok())
}

pub(super) fn resolve_get_instances(module: &Module) -> Option<GetInstancesKind> {
    resolve(module, GET_INSTANCES)
        .map(GetInstancesKind::Exact)
        .or_else(|| resolve(module, GET_INSTANCES_ASSIGNABLE).map(GetInstancesKind::WithAssignable))
}

pub(super) fn resolve_decode_global(module: &Module) -> Option<DecodeGlobalKind> {
    resolve(module, DECODE_GLOBAL_NO_THREAD)
        .map(DecodeGlobalKind::NoThread)
        .or_else(|| resolve(module, DECODE_GLOBAL_WITH_THREAD).map(DecodeGlobalKind::WithThread))
        .or_else(|| resolve(module, THREAD_DECODE_GLOBAL_JOBJECT).map(DecodeGlobalKind::Thread))
}

pub(super) fn resolve_pointer(module: &Module, symbol: &'static str) -> Option<*const c_void> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .map(|pointer| pointer.0 as *const c_void)
}

pub(super) fn resolve_pointer_any(
    module: &Module,
    symbols: &[&'static str],
) -> Option<*const c_void> {
    symbols
        .iter()
        .find_map(|symbol| resolve_pointer(module, symbol))
}

pub(super) fn resolve_any<T: Copy>(module: &Module, symbols: &[&'static str]) -> Option<T> {
    symbols.iter().find_map(|symbol| resolve(module, symbol))
}

pub(super) fn resolve_pretty_method(module: &Module) -> Option<PrettyMethodFunction> {
    let pointer = resolve_pointer_any(module, &[PRETTY_METHOD, PRETTY_METHOD_NULL_SAFE])?;
    #[cfg(target_arch = "aarch64")]
    {
        let thunk = Arc::new(ExecutableMemory::aarch64_pretty_method_thunk(pointer).ok()?);
        let function = unsafe {
            std::mem::transmute_copy::<*mut c_void, PrettyMethod>(&thunk.pointer.as_ptr())
        };
        Some(PrettyMethodFunction {
            function,
            _thunk: Some(thunk),
        })
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let function = native_pointer_to_fn(frida_gum::NativePointer(pointer as usize)).ok()?;
        Some(PrettyMethodFunction {
            function,
            _thunk: None,
        })
    }
}

pub(super) fn resolve_suspend_all(module: &Module) -> Option<SuspendAll> {
    resolve(module, SUSPEND_ALL_WITH_CAUSE)
        .map(SuspendAll::WithCause)
        .or_else(|| resolve(module, SUSPEND_ALL_LEGACY).map(SuspendAll::Legacy))
}

pub(super) fn resolve_visit_classes(module: &Module) -> Option<VisitClassesKind> {
    resolve(module, VISIT_CLASSES_VISITOR)
        .map(VisitClassesKind::Visitor)
        .or_else(|| resolve(module, VISIT_CLASSES_CALLBACK).map(VisitClassesKind::Callback))
}

pub(super) fn find_interpreter_do_call_entries(module: &Module) -> Vec<usize> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    for export in module.enumerate_exports() {
        if is_interpreter_do_call_symbol(&export.name) && seen.insert(export.address) {
            entries.push(export.address);
        }
    }
    for symbol in module.enumerate_symbols() {
        if is_interpreter_do_call_symbol(&symbol.name) && seen.insert(symbol.address) {
            entries.push(symbol.address);
        }
    }

    entries
}

pub(super) fn find_gc_synchronization_entries(module: &Module) -> Vec<GcSynchronizationEntry> {
    let mut seen = HashSet::new();
    let mut entries = Vec::new();

    if let Some(address) = resolve_pointer(module, GC_COLLECT_GARBAGE_INTERNAL) {
        push_gc_synchronization_entry(
            &mut entries,
            &mut seen,
            address,
            GcSynchronizationTiming::OnLeave,
        );
    }
    if let Some(address) = resolve_pointer_any(
        module,
        &[
            CONCURRENT_COPYING_COPYING_PHASE,
            CONCURRENT_COPYING_MARKING_PHASE,
        ],
    ) {
        push_gc_synchronization_entry(
            &mut entries,
            &mut seen,
            address,
            GcSynchronizationTiming::OnLeave,
        );
    }
    if let Some(address) = resolve_pointer_any(
        module,
        &[THREAD_RUN_FLIP_FUNCTION, THREAD_RUN_FLIP_FUNCTION_WITH_FLAG],
    ) {
        push_gc_synchronization_entry(
            &mut entries,
            &mut seen,
            address,
            GcSynchronizationTiming::OnEnter,
        );
    }

    entries
}

pub(super) fn push_gc_synchronization_entry(
    entries: &mut Vec<GcSynchronizationEntry>,
    seen: &mut HashSet<usize>,
    address: *const c_void,
    timing: GcSynchronizationTiming,
) {
    let address = address as usize;
    if seen.insert(address) {
        entries.push(GcSynchronizationEntry { address, timing });
    }
}

pub(super) fn is_interpreter_do_call_symbol(name: &str) -> bool {
    name.starts_with("_ZN3art11interpreter6DoCall")
        && name.contains("ArtMethod")
        && name.contains("ShadowFrame")
        && name.contains("JValue")
}

pub(super) fn class_name_from_descriptor(descriptor: &str) -> String {
    if descriptor.starts_with('L') && descriptor.ends_with(';') {
        descriptor[1..descriptor.len() - 1].replace('/', ".")
    } else {
        descriptor.replace('/', ".")
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
pub(super) fn art_runtime_from_vm(vm: NonNull<jni::JavaVM>) -> *mut c_void {
    unsafe { vm.as_ptr().cast::<*mut c_void>().add(1).read() }
}
