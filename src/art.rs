#![allow(dead_code)]

use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, CString, c_char, c_int, c_void},
    fs,
    mem::ManuallyDrop,
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use frida_gum::{
    Module, NativePointer,
    instruction_writer::{
        Aarch64BranchCondition, Aarch64InstructionWriter, Aarch64Register, InstructionWriter,
    },
    interceptor::{Interceptor, InvocationContext, InvocationListener, Listener},
};

use crate::{
    env::MethodKind,
    error::{Error, Result},
    java::{ClassLoaderKind, ClassLoaderRef, JavaClass},
    jni, metadata,
    refs::{AsJClass, AsJObject, ClassKind, GlobalRef},
    runtime::{FeatureSupport, native_pointer_to_fn},
    signature::MethodSignature,
    vm::Vm,
};

mod thread_transition;

const FEATURE_CLASS_LOADER_ENUMERATION: &str = "ART class-loader enumeration";
const FEATURE_LOADED_CLASS_ENUMERATION: &str = "ART loaded-class enumeration";
const FEATURE_METHOD_QUERY: &str = "ART direct method enumeration";
const FEATURE_METHOD_REPLACEMENT: &str = "ART method replacement";
const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const STD_STRING_SIZE: usize = 3 * POINTER_SIZE;
const PROP_VALUE_MAX: usize = 92;
const K_POINTER_JNI_ID_TYPE: i32 = 0;
const K_ACC_PUBLIC: u32 = 0x0001;
const K_ACC_STATIC: u32 = 0x0008;
const K_ACC_FINAL: u32 = 0x0010;
const K_ACC_NATIVE: u32 = 0x0100;
const K_ACC_FAST_NATIVE: u32 = 0x00080000;
const K_ACC_CRITICAL_NATIVE: u32 = 0x00200000;
const K_ACC_JAVA_FLAGS_MASK: u32 = 0xffff;
const K_ACC_CONSTRUCTOR: u32 = 0x00010000;
const K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE: u32 = 0x40000000;
const K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG: u32 = 0x00100000;
const K_ACC_NTERP_INVOKE_FAST_PATH_FLAG: u32 = 0x00200000;
const K_ACC_PUBLIC_API: u32 = 0x10000000;
const K_ACC_SKIP_ACCESS_CHECKS: u32 = 0x00080000;
const K_ACC_SINGLE_IMPLEMENTATION: u32 = 0x08000000;
static ORIGINAL_CALL_BYPASS_METHOD: AtomicUsize = AtomicUsize::new(0);
const CLASS_LAYOUT_SCAN_LIMIT: usize = 0x100;
const METHOD_LAYOUT_SCAN_LIMIT: usize = 64;
const ART_METHOD_MIN_SIZE: usize = 16;
const ART_METHOD_MAX_SIZE: usize = 256;
const ART_METHOD_ARRAY_MAX_PROBE: usize = 100;
const ADD_GLOBAL_REF_OBJ_PTR: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadENS_6ObjPtrINS_6mirror6ObjectEEE";
const ADD_GLOBAL_REF_POINTER: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadEPNS_6mirror6ObjectE";
const SUSPEND_ALL_WITH_CAUSE: &str = "_ZN3art10ThreadList10SuspendAllEPKcb";
const SUSPEND_ALL_LEGACY: &str = "_ZN3art10ThreadList10SuspendAllEv";
const RESUME_ALL: &str = "_ZN3art10ThreadList9ResumeAllEv";
const VISIT_CLASS_LOADERS: &str =
    "_ZNK3art11ClassLinker17VisitClassLoadersEPNS_18ClassLoaderVisitorE";
const VISIT_CLASSES_VISITOR: &str = "_ZN3art11ClassLinker12VisitClassesEPNS_12ClassVisitorE";
const VISIT_CLASSES_CALLBACK: &str =
    "_ZN3art11ClassLinker12VisitClassesEPFbPNS_6mirror5ClassEPvES4_";
const GET_CLASS_DESCRIPTOR: &str = "_ZN3art6mirror5Class13GetDescriptorEPNSt3__112basic_stringIcNS2_11char_traitsIcEENS2_9allocatorIcEEEE";
const PRETTY_METHOD: &str = "_ZN3art9ArtMethod12PrettyMethodEb";
const PRETTY_METHOD_NULL_SAFE: &str = "_ZN3art12PrettyMethodEPNS_9ArtMethodEb";
const DECODE_METHOD_ID: &str = "_ZN3art3jni12JniIdManager14DecodeMethodIdEP10_jmethodID";
const SET_JNI_ID_TYPE: &str = "_ZN3art7Runtime12SetJniIdTypeENS_9JniIdTypeE";
const IS_QUICK_RESOLUTION_STUB: &str = "_ZNK3art11ClassLinker21IsQuickResolutionStubEPKv";
const IS_QUICK_TO_INTERPRETER_BRIDGE: &str =
    "_ZNK3art11ClassLinker26IsQuickToInterpreterBridgeEPKv";
const IS_QUICK_GENERIC_JNI_STUB: &str = "_ZNK3art11ClassLinker21IsQuickGenericJniStubEPKv";
const JNI_EXCEPTION_CLEAR: &str = "_ZN3art3JNIILb1EE14ExceptionClearEP7_JNIEnv";
const JNI_FATAL_ERROR: &str = "_ZN3art3JNIILb1EE10FatalErrorEP7_JNIEnvPKc";
const GET_OAT_QUICK_METHOD_HEADER_U32: &str = "_ZN3art9ArtMethod23GetOatQuickMethodHeaderEj";
const GET_OAT_QUICK_METHOD_HEADER_USIZE: &str = "_ZN3art9ArtMethod23GetOatQuickMethodHeaderEm";
const GC_COLLECT_GARBAGE_INTERNAL: &str =
    "_ZN3art2gc4Heap22CollectGarbageInternalENS0_9collector6GcTypeENS0_7GcCauseEbj";
const CONCURRENT_COPYING_COPYING_PHASE: &str =
    "_ZN3art2gc9collector17ConcurrentCopying12CopyingPhaseEv";
const CONCURRENT_COPYING_MARKING_PHASE: &str =
    "_ZN3art2gc9collector17ConcurrentCopying12MarkingPhaseEv";
const THREAD_RUN_FLIP_FUNCTION: &str = "_ZN3art6Thread15RunFlipFunctionEPS0_";
const THREAD_RUN_FLIP_FUNCTION_WITH_FLAG: &str = "_ZN3art6Thread15RunFlipFunctionEPS0_b";

type AddGlobalRef =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, *mut c_void) -> jni::jobject;
type GetClassDescriptor = unsafe extern "C" fn(*mut c_void, *mut ArtStdString) -> *const c_char;
type PrettyMethod = unsafe extern "C" fn(*mut ArtStdString, *mut c_void, bool);
type DecodeMethodId = unsafe extern "C" fn(*mut c_void, jni::jmethodID) -> *mut c_void;
type IsQuickEntrypoint = unsafe extern "C" fn(*mut c_void, *const c_void) -> bool;
type SuspendAllWithCause = unsafe extern "C" fn(*mut c_void, *const c_char, bool);
type SuspendAllLegacy = unsafe extern "C" fn(*mut c_void);
type ResumeAll = unsafe extern "C" fn(*mut c_void);
type VisitClassLoaders = unsafe extern "C" fn(*mut c_void, *mut ArtClassLoaderVisitor);
type VisitClasses = unsafe extern "C" fn(*mut c_void, *mut ArtClassVisitor);
type VisitClassesCallback = unsafe extern "C" fn(*mut c_void, ArtClassCallback, *mut c_void);
type ArtClassCallback = unsafe extern "C" fn(*mut c_void, *mut c_void) -> bool;
type GetOatQuickMethodHeader = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;

static ART_REPLACEMENT_CONTROLLER: OnceLock<Arc<ArtReplacementController>> = OnceLock::new();
static ORIGINAL_GET_OAT_QUICK_METHOD_HEADER: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
}

#[derive(Clone)]
pub(crate) struct ArtBackend {
    android_runtime: Option<ArtModuleRange>,
    add_global_ref: Option<AddGlobalRef>,
    suspend_all: Option<SuspendAll>,
    resume_all: Option<ResumeAll>,
    visit_class_loaders: Option<VisitClassLoaders>,
    visit_classes: Option<VisitClassesKind>,
    get_class_descriptor: Option<GetClassDescriptor>,
    pretty_method: Option<PrettyMethodFunction>,
    decode_method_id: Option<DecodeMethodId>,
    set_jni_id_type: Option<*const c_void>,
    is_quick_resolution_stub: Option<IsQuickEntrypoint>,
    is_quick_to_interpreter_bridge: Option<IsQuickEntrypoint>,
    is_quick_generic_jni_stub: Option<IsQuickEntrypoint>,
    exception_clear: Option<*const c_void>,
    fatal_error: Option<*const c_void>,
    thread_transition: Arc<OnceLock<thread_transition::ThreadTransition>>,
    replacement_controller: Arc<ArtReplacementController>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArtModuleRange {
    start: usize,
    end: usize,
}

#[derive(Clone, Copy)]
enum SuspendAll {
    WithCause(SuspendAllWithCause),
    Legacy(SuspendAllLegacy),
}

#[derive(Clone, Copy)]
enum VisitClassesKind {
    Visitor(VisitClasses),
    Callback(VisitClassesCallback),
}

#[repr(C)]
struct ArtClassLoaderVisitor {
    vtable: *const *const c_void,
    vtable_storage: [*const c_void; 3],
    loaders: *mut Vec<*mut c_void>,
}

#[repr(C)]
struct ArtClassVisitor {
    vtable: *const *const c_void,
    vtable_storage: [*const c_void; 3],
    context: *mut c_void,
    visit: ArtRustClassCallback,
}

struct RawClass(jni::jclass);

type ArtRustClassCallback = unsafe fn(*mut c_void, *mut c_void) -> bool;

#[repr(C)]
struct ArtStdString {
    storage: [usize; 3],
}

struct RawLoadedClass {
    name: String,
    raw: jni::jclass,
}

struct ArtClassProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    get_class_descriptor: GetClassDescriptor,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    seen: HashSet<usize>,
    classes: &'callback mut Vec<RawLoadedClass>,
    error: Option<Error>,
}

#[derive(Clone)]
struct PrettyMethodFunction {
    function: PrettyMethod,
    _thunk: Option<Arc<ExecutableMemory>>,
}

struct ExecutableMemory {
    pointer: NonNull<c_void>,
    length: usize,
}

unsafe impl Send for ExecutableMemory {}
unsafe impl Sync for ExecutableMemory {}

struct FindArtClassProcessor {
    get_class_descriptor: GetClassDescriptor,
    descriptor: &'static str,
    class: Option<*mut c_void>,
    error: Option<Error>,
}

struct RawMethodQueryGroup {
    loader_key: u32,
    loader: Option<jni::jobject>,
    classes: Vec<metadata::JavaMethodQueryClass>,
}

struct ArtMethodQueryProcessor<'callback> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtRuntimeLayout {
    runtime: *mut c_void,
    thread_list: *mut c_void,
    class_linker: *mut c_void,
    intern_table: *mut c_void,
    jni_id_manager: *mut c_void,
    jni_ids_indirection: Option<i32>,
}

impl ArtRuntimeLayout {
    fn uses_indirect_jni_ids(&self) -> bool {
        !self.jni_id_manager.is_null()
            && self
                .jni_ids_indirection
                .is_some_and(|indirection| indirection != K_POINTER_JNI_ID_TYPE)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodQueryLayout {
    class_methods_offset: usize,
    class_copied_methods_offset: usize,
    method_size: usize,
    method_access_flags_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtArray {
    data: *mut c_void,
    length: usize,
}

#[derive(Debug, Default)]
struct MemoryRanges {
    ranges: Vec<MemoryRange>,
}

#[derive(Debug)]
struct MemoryRange {
    start: usize,
    end: usize,
    executable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodRuntimeLayout {
    method_size: usize,
    access_flags_offset: usize,
    jni_code_offset: usize,
    quick_code_offset: usize,
    interpreter_code_offset: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtClassLinkerTrampolines {
    quick_resolution_trampoline: *mut c_void,
    quick_imt_conflict_trampoline: *mut c_void,
    quick_generic_jni_trampoline: *mut c_void,
    quick_to_interpreter_bridge_trampoline: *mut c_void,
}

#[derive(Debug, Clone, Copy)]
struct ArtClassLinkerEntrypointPredicates {
    is_quick_resolution_stub: IsQuickEntrypoint,
    is_quick_to_interpreter_bridge: IsQuickEntrypoint,
    is_quick_generic_jni_stub: IsQuickEntrypoint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodReplacementLayout {
    api_level: i32,
    runtime: ArtRuntimeLayout,
    method: ArtMethodRuntimeLayout,
    trampolines: ArtClassLinkerTrampolines,
    thread_managed_stack_offset: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodSnapshot {
    access_flags: u32,
    jni_code: *mut c_void,
    quick_code: *mut c_void,
    interpreter_code: Option<*mut c_void>,
}

struct ArtMethodClone {
    method: NonNull<c_void>,
    length: usize,
}

struct ArtReplacementController {
    do_call_entries: Vec<usize>,
    get_oat_quick_method_header: Option<*const c_void>,
    gc_synchronization_entries: Vec<GcSynchronizationEntry>,
    mappings: Mutex<ArtReplacementMappings>,
    quick_entrypoint_hooks: Mutex<ArtQuickEntrypointHooks>,
    hooks: OnceLock<ArtReplacementHooks>,
}

#[derive(Debug, Default)]
struct ArtReplacementMappings {
    methods: HashMap<usize, ArtReplacementRecord>,
    replacements: HashMap<usize, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtReplacementRecord {
    replacement: usize,
    synchronization: ArtReplacementSynchronization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtReplacementSynchronization {
    quick_code_offset: usize,
    thread_managed_stack_offset: usize,
    nterp_entrypoint: Option<usize>,
    quick_to_interpreter_bridge: usize,
}

#[derive(Default)]
struct ArtQuickEntrypointHooks {
    addresses: HashSet<usize>,
    hooks: Vec<HookedQuickEntrypoint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GcSynchronizationEntry {
    address: usize,
    timing: GcSynchronizationTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GcSynchronizationTiming {
    OnEnter,
    OnLeave,
}

struct ArtReplacementHooks {
    _interceptor: Interceptor,
    _listeners: Vec<HookedInterpreterDoCall>,
    _gc_listeners: Vec<HookedGcSynchronization>,
    _get_oat_quick_method_header: Option<ReplacedGetOatQuickMethodHeader>,
}

struct HookedInterpreterDoCall {
    _listener: Box<ArtMethodTranslationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct HookedGcSynchronization {
    _listener: Box<ArtReplacementSynchronizationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct HookedQuickEntrypoint {
    _interceptor: Interceptor,
    _listener: Box<ArtMethodTranslationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct ReplacedGetOatQuickMethodHeader {
    function: NativePointer,
    original: NativePointer,
}

struct ArtMethodTranslationListener {
    controller: Arc<ArtReplacementController>,
    source: ArtMethodTranslationSource,
}

struct ArtReplacementSynchronizationListener {
    controller: Arc<ArtReplacementController>,
    timing: GcSynchronizationTiming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ArtMethodTranslationSource {
    InterpreterDoCall,
    QuickEntrypoint,
}

struct ArtMethodDispatchThunk {
    pointer: NonNull<c_void>,
    length: usize,
}

pub(crate) struct OriginalMethodCallBypass {
    previous: usize,
}

pub(crate) struct ArtMethodReplacementGuard {
    backend: ArtBackend,
    vm: Vm,
    method: *mut c_void,
    cloned_method: ArtMethodClone,
    dispatch_thunk: ArtMethodDispatchThunk,
    layout: ArtMethodReplacementLayout,
    original: ArtMethodSnapshot,
    original_patched: ArtMethodSnapshot,
    clone_patched: ArtMethodSnapshot,
    reverted: bool,
}

impl ArtReplacementController {
    fn new(module: &Module) -> Self {
        Self {
            do_call_entries: find_interpreter_do_call_entries(module),
            get_oat_quick_method_header: resolve_pointer_any(
                module,
                &[
                    GET_OAT_QUICK_METHOD_HEADER_USIZE,
                    GET_OAT_QUICK_METHOD_HEADER_U32,
                ],
            ),
            gc_synchronization_entries: find_gc_synchronization_entries(module),
            mappings: Mutex::new(ArtReplacementMappings::default()),
            quick_entrypoint_hooks: Mutex::new(ArtQuickEntrypointHooks::default()),
            hooks: OnceLock::new(),
        }
    }

    #[cfg(test)]
    fn empty_for_tests() -> Self {
        Self {
            do_call_entries: Vec::new(),
            get_oat_quick_method_header: None,
            gc_synchronization_entries: Vec::new(),
            mappings: Mutex::new(ArtReplacementMappings::default()),
            quick_entrypoint_hooks: Mutex::new(ArtQuickEntrypointHooks::default()),
            hooks: OnceLock::new(),
        }
    }

    fn ensure_dispatch_supported(&self) -> Result<()> {
        if self.do_call_entries.is_empty() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART interpreter DoCall entrypoint is unavailable for cloned replacement dispatch",
            );
        }
        Ok(())
    }

    fn ensure_hooks(self: &Arc<Self>) -> Result<()> {
        self.ensure_dispatch_supported()?;
        if self.hooks.get().is_some() {
            return Ok(());
        }

        let _ = ART_REPLACEMENT_CONTROLLER.set(self.clone());
        let hooks = ArtReplacementHooks::install(self.clone())?;
        let _ = self.hooks.set(hooks);
        Ok(())
    }

    fn ensure_quick_entrypoint_hooks(
        self: &Arc<Self>,
        trampolines: &ArtClassLinkerTrampolines,
    ) -> Result<()> {
        let mut quick_hooks = self
            .quick_entrypoint_hooks
            .lock()
            .expect("ART replacement quick hooks mutex poisoned");
        for entrypoint in [
            trampolines.quick_generic_jni_trampoline,
            trampolines.quick_resolution_trampoline,
        ] {
            let address = entrypoint as usize;
            if address == 0 || !quick_hooks.addresses.insert(address) {
                continue;
            }

            let gum = frida_gum::Gum::obtain();
            let mut interceptor = Interceptor::obtain(&gum);
            let mut listener = Box::new(ArtMethodTranslationListener {
                controller: self.clone(),
                source: ArtMethodTranslationSource::QuickEntrypoint,
            });
            let handle = interceptor
                .attach(NativePointer(entrypoint), listener.as_mut())
                .map_err(|error| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: format!("unable to hook ART quick entrypoint: {error:?}"),
                })?;
            quick_hooks.hooks.push(HookedQuickEntrypoint {
                _interceptor: interceptor,
                _listener: listener,
                _handle: ManuallyDrop::new(handle),
            });
        }
        Ok(())
    }

    fn register(
        &self,
        original: *mut c_void,
        replacement: *mut c_void,
        synchronization: ArtReplacementSynchronization,
    ) {
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.methods.insert(
            original as usize,
            ArtReplacementRecord {
                replacement: replacement as usize,
                synchronization,
            },
        );
        mappings
            .replacements
            .insert(replacement as usize, original as usize);
    }

    fn unregister(&self, original: *mut c_void) {
        let mut mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        if let Some(record) = mappings.methods.remove(&(original as usize)) {
            mappings.replacements.remove(&record.replacement);
        }
    }

    fn replacement_for(&self, original: *mut c_void) -> Option<*mut c_void> {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings
            .methods
            .get(&(original as usize))
            .map(|record| record.replacement as *mut c_void)
    }

    fn is_replacement_method(&self, method: *mut c_void) -> bool {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        mappings.replacements.contains_key(&(method as usize))
    }

    fn translate_method_argument(&self, method: usize) -> usize {
        self.translate_method_argument_for_thread(method, 0)
    }

    fn translate_method_argument_for_thread(&self, method: usize, thread: usize) -> usize {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        let Some(record) = mappings.methods.get(&method) else {
            return method;
        };
        if replacement_frame_is_active(
            record.replacement,
            thread,
            record.synchronization.thread_managed_stack_offset,
        ) {
            method
        } else {
            record.replacement
        }
    }

    fn synchronize_replacement_methods(&self) {
        let mappings = self
            .mappings
            .lock()
            .expect("ART replacement mappings mutex poisoned");
        for (original, record) in &mappings.methods {
            unsafe {
                let original_declaring_class = *original as *const u32;
                let replacement_declaring_class = record.replacement as *mut u32;
                let declaring_class = ptr::read_unaligned(original_declaring_class);
                ptr::write_unaligned(replacement_declaring_class, declaring_class);

                if let Some(nterp_entrypoint) = record.synchronization.nterp_entrypoint {
                    let original_quick_code =
                        (*original + record.synchronization.quick_code_offset) as *mut usize;
                    if ptr::read_unaligned(original_quick_code) == nterp_entrypoint {
                        ptr::write_unaligned(
                            original_quick_code,
                            record.synchronization.quick_to_interpreter_bridge,
                        );
                    }
                }
            }
        }
    }
}

impl ArtReplacementHooks {
    fn install(controller: Arc<ArtReplacementController>) -> Result<Self> {
        let gum = frida_gum::Gum::obtain();
        let mut interceptor = Interceptor::obtain(&gum);
        let mut listeners = Vec::new();
        let mut gc_listeners = Vec::new();

        for address in &controller.do_call_entries {
            let mut listener = Box::new(ArtMethodTranslationListener {
                controller: controller.clone(),
                source: ArtMethodTranslationSource::InterpreterDoCall,
            });
            let handle = interceptor
                .attach(NativePointer(*address as *mut c_void), listener.as_mut())
                .map_err(|error| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: format!("unable to hook ART interpreter DoCall: {error:?}"),
                })?;
            listeners.push(HookedInterpreterDoCall {
                _listener: listener,
                _handle: ManuallyDrop::new(handle),
            });
        }

        for entry in &controller.gc_synchronization_entries {
            let mut listener = Box::new(ArtReplacementSynchronizationListener {
                controller: controller.clone(),
                timing: entry.timing,
            });
            let handle = interceptor
                .attach(
                    NativePointer(entry.address as *mut c_void),
                    listener.as_mut(),
                )
                .map_err(|error| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: format!("unable to hook ART replacement GC synchronization: {error:?}"),
                })?;
            gc_listeners.push(HookedGcSynchronization {
                _listener: listener,
                _handle: ManuallyDrop::new(handle),
            });
        }

        let get_oat_quick_method_header =
            if let Some(function) = controller.get_oat_quick_method_header {
                match interceptor.replace(
                    NativePointer(function as *mut c_void),
                    NativePointer(on_art_method_get_oat_quick_method_header as *mut c_void),
                    NativePointer(ptr::null_mut()),
                ) {
                    Ok(original) => {
                        ORIGINAL_GET_OAT_QUICK_METHOD_HEADER
                            .store(original.0 as usize, Ordering::SeqCst);
                        Some(ReplacedGetOatQuickMethodHeader {
                            function: NativePointer(function as *mut c_void),
                            original,
                        })
                    }
                    Err(error) => {
                        return Err(Error::UnsupportedFeature {
                            feature: FEATURE_METHOD_REPLACEMENT,
                            reason: format!(
                                "unable to hook ArtMethod::GetOatQuickMethodHeader: {error:?}"
                            ),
                        });
                    }
                }
            } else {
                None
            };

        Ok(Self {
            _interceptor: interceptor,
            _listeners: listeners,
            _gc_listeners: gc_listeners,
            _get_oat_quick_method_header: get_oat_quick_method_header,
        })
    }
}

impl InvocationListener for ArtMethodTranslationListener {
    fn on_enter(&mut self, context: InvocationContext<'_>) {
        let method = context.arg(0);
        let thread = self.art_thread(&context);
        let translated = self
            .controller
            .translate_method_argument_for_thread(method, thread);
        if translated != method {
            context.set_arg(0, translated);
        }
    }

    fn on_leave(&mut self, _context: InvocationContext<'_>) {}
}

impl ArtMethodTranslationListener {
    fn art_thread(&self, context: &InvocationContext<'_>) -> usize {
        match self.source {
            ArtMethodTranslationSource::InterpreterDoCall => context.arg(1),
            ArtMethodTranslationSource::QuickEntrypoint => {
                #[cfg(target_arch = "aarch64")]
                {
                    context.cpu_context().reg(19) as usize
                }

                #[cfg(not(target_arch = "aarch64"))]
                {
                    0
                }
            }
        }
    }
}

impl InvocationListener for ArtReplacementSynchronizationListener {
    fn on_enter(&mut self, _context: InvocationContext<'_>) {
        if self.timing == GcSynchronizationTiming::OnEnter {
            self.controller.synchronize_replacement_methods();
        }
    }

    fn on_leave(&mut self, _context: InvocationContext<'_>) {
        if self.timing == GcSynchronizationTiming::OnLeave {
            self.controller.synchronize_replacement_methods();
        }
    }
}

unsafe extern "C" fn on_art_method_get_oat_quick_method_header(
    method: *mut c_void,
    pc: usize,
) -> *mut c_void {
    if ART_REPLACEMENT_CONTROLLER
        .get()
        .is_some_and(|controller| controller.is_replacement_method(method))
    {
        return ptr::null_mut();
    }

    let original = ORIGINAL_GET_OAT_QUICK_METHOD_HEADER.load(Ordering::SeqCst);
    if original == 0 {
        return ptr::null_mut();
    }

    let original: GetOatQuickMethodHeader = unsafe { std::mem::transmute(original) };
    unsafe { original(method, pc) }
}

// Gum's interceptor objects are process-global and protected internally. The controller only
// mutates its map through a mutex, and hooks are installed once for the lifetime of the backend.
unsafe impl Send for ArtReplacementController {}
unsafe impl Sync for ArtReplacementController {}
unsafe impl Send for ArtReplacementHooks {}
unsafe impl Sync for ArtReplacementHooks {}

impl ArtMethodReplacementGuard {
    pub(crate) fn revert(&mut self) -> Result<()> {
        if self.reverted {
            return Ok(());
        }
        self.backend.replacement_controller.unregister(self.method);
        self.backend
            .restore_method(&self.vm, self.method, &self.layout, self.original)?;
        self.reverted = true;
        Ok(())
    }

    pub(crate) fn debug_summary(&self) -> String {
        format!(
            "backend=clone-active, method={:?}, cloned_method={:?}, dispatch_thunk={:?}, api_level={}, jni_ids_indirection={:?}, uses_indirect_jni_ids={}, method_size={}, access_flags_offset={}, jni_code_offset={}, quick_code_offset={}, interpreter_code_offset={:?}, thread_managed_stack_offset={}, quick_generic_jni_trampoline={:?}, quick_to_interpreter_bridge_trampoline={:?}, do_call_hooks={}, quick_entrypoint_hooks={}, get_oat_quick_method_header_hook={}, gc_synchronization_hooks={}, original={{access_flags=0x{:08x}, jni_code={:?}, quick_code={:?}, interpreter_code={:?}}}, original_patched={{access_flags=0x{:08x}, jni_code={:?}, quick_code={:?}, interpreter_code={:?}}}, clone_patched={{access_flags=0x{:08x}, jni_code={:?}, quick_code={:?}, interpreter_code={:?}}}",
            self.method,
            self.cloned_method.as_ptr(),
            self.dispatch_thunk.as_ptr(),
            self.layout.api_level,
            self.layout.runtime.jni_ids_indirection,
            self.layout.runtime.uses_indirect_jni_ids(),
            self.layout.method.method_size,
            self.layout.method.access_flags_offset,
            self.layout.method.jni_code_offset,
            self.layout.method.quick_code_offset,
            self.layout.method.interpreter_code_offset,
            self.layout.thread_managed_stack_offset,
            self.layout.trampolines.quick_generic_jni_trampoline,
            self.layout
                .trampolines
                .quick_to_interpreter_bridge_trampoline,
            self.backend.replacement_controller.do_call_entries.len(),
            self.backend
                .replacement_controller
                .quick_entrypoint_hooks
                .lock()
                .expect("ART replacement quick hooks mutex poisoned")
                .hooks
                .len(),
            self.backend
                .replacement_controller
                .get_oat_quick_method_header
                .is_some(),
            self.backend
                .replacement_controller
                .gc_synchronization_entries
                .len(),
            self.original.access_flags,
            self.original.jni_code,
            self.original.quick_code,
            self.original.interpreter_code,
            self.original_patched.access_flags,
            self.original_patched.jni_code,
            self.original_patched.quick_code,
            self.original_patched.interpreter_code,
            self.clone_patched.access_flags,
            self.clone_patched.jni_code,
            self.clone_patched.quick_code,
            self.clone_patched.interpreter_code,
        )
    }
}

impl Drop for ArtMethodReplacementGuard {
    fn drop(&mut self) {
        let _ = self.revert();
    }
}

impl ArtMethodClone {
    fn copy_from(
        method: *mut c_void,
        layout: &ArtMethodRuntimeLayout,
        memory: &MemoryRanges,
    ) -> Result<Self> {
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;

        if method.is_null() || !memory.contains(method as usize, layout.method_size) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "target ArtMethod is not readable for cloning",
            );
        }
        if layout.method_size == 0 {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "target ArtMethod clone size is zero",
            );
        }

        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                layout.method_size,
                PROT_READ | PROT_WRITE,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to allocate cloned ArtMethod",
            );
        }

        unsafe {
            ptr::copy_nonoverlapping(
                method.cast::<u8>(),
                pointer.cast::<u8>(),
                layout.method_size,
            );
        }
        let Some(method) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, layout.method_size) };
            return Err(Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            method,
            length: layout.method_size,
        })
    }

    fn as_ptr(&self) -> *mut c_void {
        self.method.as_ptr()
    }

    fn memory_ranges(&self) -> MemoryRanges {
        MemoryRanges {
            ranges: vec![MemoryRange {
                start: self.as_ptr() as usize,
                end: self.as_ptr() as usize + self.length,
                executable: false,
            }],
        }
    }
}

impl Drop for ArtMethodClone {
    fn drop(&mut self) {
        unsafe {
            munmap(self.as_ptr(), self.length);
        }
    }
}

pub(crate) fn original_method_call_bypass(method: usize) -> OriginalMethodCallBypass {
    let previous = ORIGINAL_CALL_BYPASS_METHOD.swap(method, Ordering::SeqCst);
    OriginalMethodCallBypass { previous }
}

impl Drop for OriginalMethodCallBypass {
    fn drop(&mut self) {
        ORIGINAL_CALL_BYPASS_METHOD.store(self.previous, Ordering::SeqCst);
    }
}

fn write_art_method_dispatch_thunk(
    code: *mut c_void,
    cloned_method: *mut c_void,
    original_dispatch_code: *mut c_void,
    quick_code_offset: usize,
    thread_managed_stack_offset: usize,
) -> Result<()> {
    const CHECK_LINK: u64 = 1;
    const ORIGINAL: u64 = 2;
    const REPLACEMENT: u64 = 3;

    let writer = Aarch64InstructionWriter::new(code as u64);

    write_original_call_bypass_check(&writer, ORIGINAL)?;

    put_cbz_label(&writer, Aarch64Register::X19, REPLACEMENT);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(
            Aarch64Register::X16,
            Aarch64Register::X19,
            thread_managed_stack_offset as u64,
        ),
        "emit managed-stack load",
    )?;
    put_cbz_label(&writer, Aarch64Register::X16, CHECK_LINK);
    writer.put_b_label(REPLACEMENT);

    writer.put_label(CHECK_LINK);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(
            Aarch64Register::X16,
            Aarch64Register::X19,
            (thread_managed_stack_offset + POINTER_SIZE) as u64,
        ),
        "emit managed-stack link load",
    )?;
    put_cbz_label(&writer, Aarch64Register::X16, REPLACEMENT);
    write_replacement_frame_check(&writer, ORIGINAL, REPLACEMENT, cloned_method)?;

    writer.put_label(ORIGINAL);
    ensure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X16, original_dispatch_code as u64),
        "emit original dispatch load",
    )?;
    ensure_writer(
        writer.put_br_reg(Aarch64Register::X16),
        "emit original dispatch branch",
    )?;

    writer.put_label(REPLACEMENT);
    ensure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X0, cloned_method as u64),
        "emit cloned ArtMethod load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_reg_offset(
            Aarch64Register::X16,
            Aarch64Register::X0,
            quick_code_offset as u64,
        ),
        "emit cloned quick-entrypoint load",
    )?;
    ensure_writer(
        writer.put_br_reg(Aarch64Register::X16),
        "emit replacement dispatch branch",
    )?;
    writer.put_nop();

    ensure_writer(writer.flush(), "flush ART method dispatch thunk")
}

fn write_replacement_frame_check(
    writer: &Aarch64InstructionWriter,
    original_label: u64,
    replacement_label: u64,
    cloned_method: *mut c_void,
) -> Result<()> {
    ensure_writer(
        put_and_reg_reg_imm(writer, Aarch64Register::X16, Aarch64Register::X16, !0x3u64),
        "emit managed-stack frame tag mask",
    )?;
    put_cbz_label(writer, Aarch64Register::X16, replacement_label);
    ensure_writer(
        writer.put_ldr_reg_reg_offset(Aarch64Register::X16, Aarch64Register::X16, 0),
        "emit top quick-frame ArtMethod load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_u64(Aarch64Register::X17, cloned_method as u64),
        "emit cloned ArtMethod comparison load",
    )?;
    ensure_writer(
        writer.put_cmp_reg_reg(Aarch64Register::X16, Aarch64Register::X17),
        "emit cloned ArtMethod comparison",
    )?;
    writer.put_bcond_label(Aarch64BranchCondition::Eq, original_label);
    writer.put_b_label(replacement_label);
    Ok(())
}

fn write_original_call_bypass_check(
    writer: &Aarch64InstructionWriter,
    original_label: u64,
) -> Result<()> {
    ensure_writer(
        writer.put_ldr_reg_u64(
            Aarch64Register::X16,
            (&ORIGINAL_CALL_BYPASS_METHOD as *const AtomicUsize) as u64,
        ),
        "emit original-call bypass cell load",
    )?;
    ensure_writer(
        writer.put_ldr_reg_reg_offset(Aarch64Register::X16, Aarch64Register::X16, 0),
        "emit original-call bypass method load",
    )?;
    ensure_writer(
        writer.put_cmp_reg_reg(Aarch64Register::X0, Aarch64Register::X16),
        "emit original-call bypass comparison",
    )?;
    writer.put_bcond_label(Aarch64BranchCondition::Eq, original_label);
    Ok(())
}

fn put_cbz_label(writer: &Aarch64InstructionWriter, reg: Aarch64Register, label: u64) {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_cbz_reg_label(
            writer.raw_writer(),
            reg as u32,
            label as *const c_void,
        );
    }
}

fn put_and_reg_reg_imm(
    writer: &Aarch64InstructionWriter,
    dst: Aarch64Register,
    left: Aarch64Register,
    right: u64,
) -> bool {
    unsafe {
        frida_gum_sys::gum_arm64_writer_put_and_reg_reg_imm(
            writer.raw_writer(),
            dst as u32,
            left as u32,
            right,
        ) != 0
    }
}

fn ensure_writer(ok: bool, operation: &'static str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("{operation} failed while generating dispatch thunk"),
        )
    }
}

impl ArtMethodDispatchThunk {
    fn new(
        cloned_method: *mut c_void,
        original_dispatch_code: *mut c_void,
        quick_code_offset: usize,
        thread_managed_stack_offset: usize,
    ) -> Result<Self> {
        const PROT_READ: c_int = 0x1;
        const PROT_WRITE: c_int = 0x2;
        const PROT_EXEC: c_int = 0x4;
        const MAP_PRIVATE: c_int = 0x02;
        const MAP_ANONYMOUS: c_int = 0x20;
        const MAP_FAILED: isize = -1;
        const LENGTH: usize = 4096;

        if cloned_method.is_null() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "cloned ArtMethod is null for dispatch thunk",
            );
        }
        if original_dispatch_code.is_null() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "original ArtMethod dispatch entrypoint is null for dispatch thunk",
            );
        }
        if !quick_code_offset.is_multiple_of(POINTER_SIZE) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod quick entrypoint offset is not pointer-aligned",
            );
        }
        if !thread_managed_stack_offset.is_multiple_of(POINTER_SIZE) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART Thread managed stack offset is not pointer-aligned",
            );
        }
        if quick_code_offset / POINTER_SIZE > 0x0fff {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod quick entrypoint offset is too large for dispatch thunk",
            );
        }
        if thread_managed_stack_offset / POINTER_SIZE > 0x0fff {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART Thread managed stack offset is too large for dispatch thunk",
            );
        }
        if (thread_managed_stack_offset + POINTER_SIZE) / POINTER_SIZE > 0x0fff {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ART Thread managed stack link offset is too large for dispatch thunk",
            );
        }

        let pointer = unsafe {
            mmap(
                ptr::null_mut(),
                LENGTH,
                PROT_READ | PROT_WRITE | PROT_EXEC,
                MAP_PRIVATE | MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if pointer as isize == MAP_FAILED {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to allocate ArtMethod dispatch thunk",
            );
        }

        write_art_method_dispatch_thunk(
            pointer,
            cloned_method,
            original_dispatch_code,
            quick_code_offset,
            thread_managed_stack_offset,
        )?;
        unsafe { frida_gum_sys::gum_clear_cache(pointer, LENGTH as u64) };

        let Some(pointer) = NonNull::new(pointer) else {
            unsafe { munmap(pointer, LENGTH) };
            return Err(Error::NullReturn { operation: "mmap" });
        };
        Ok(Self {
            pointer,
            length: LENGTH,
        })
    }

    fn as_ptr(&self) -> *mut c_void {
        self.pointer.as_ptr()
    }
}

impl Drop for ArtMethodDispatchThunk {
    fn drop(&mut self) {
        unsafe {
            munmap(self.as_ptr(), self.length);
        }
    }
}

impl ArtBackend {
    pub(crate) fn from_module(module: &Module, android_runtime: Option<ArtModuleRange>) -> Self {
        Self {
            android_runtime,
            add_global_ref: resolve_any(module, &[ADD_GLOBAL_REF_OBJ_PTR, ADD_GLOBAL_REF_POINTER]),
            suspend_all: resolve_suspend_all(module),
            resume_all: resolve(module, RESUME_ALL),
            visit_class_loaders: resolve(module, VISIT_CLASS_LOADERS),
            visit_classes: resolve_visit_classes(module),
            get_class_descriptor: resolve(module, GET_CLASS_DESCRIPTOR),
            pretty_method: resolve_pretty_method(module),
            decode_method_id: resolve(module, DECODE_METHOD_ID),
            set_jni_id_type: resolve_pointer(module, SET_JNI_ID_TYPE),
            is_quick_resolution_stub: resolve(module, IS_QUICK_RESOLUTION_STUB),
            is_quick_to_interpreter_bridge: resolve(module, IS_QUICK_TO_INTERPRETER_BRIDGE),
            is_quick_generic_jni_stub: resolve(module, IS_QUICK_GENERIC_JNI_STUB),
            exception_clear: resolve_pointer(module, JNI_EXCEPTION_CLEAR),
            fatal_error: resolve_pointer(module, JNI_FATAL_ERROR),
            thread_transition: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::new(module)),
        }
    }

    #[cfg(test)]
    pub(crate) fn empty_for_tests() -> Self {
        Self {
            android_runtime: None,
            add_global_ref: None,
            suspend_all: None,
            resume_all: None,
            visit_class_loaders: None,
            visit_classes: None,
            get_class_descriptor: None,
            pretty_method: None,
            decode_method_id: None,
            set_jni_id_type: None,
            is_quick_resolution_stub: None,
            is_quick_to_interpreter_bridge: None,
            is_quick_generic_jni_stub: None,
            exception_clear: None,
            fatal_error: None,
            thread_transition: Arc::new(OnceLock::new()),
            replacement_controller: Arc::new(ArtReplacementController::empty_for_tests()),
        }
    }

    pub(crate) fn enumerate_class_loaders(&self, vm: &Vm) -> Result<Vec<ClassLoaderRef>> {
        self.ensure_class_loader_enumeration_supported(vm.handle())?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm.handle(), FEATURE_CLASS_LOADER_ENUMERATION)
            .expect("runtime layout support checked before class-loader enumeration");
        let mut loader_globals = Vec::new();

        self.with_runnable_art_thread(&env, FEATURE_CLASS_LOADER_ENUMERATION, |thread| {
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before enumeration");
            let visit_class_loaders = self
                .visit_class_loaders
                .expect("visit_class_loaders symbol checked before enumeration");
            let mut loader_objects = Vec::new();
            let mut visitor = ArtClassLoaderVisitor::new(&mut loader_objects);
            visitor.initialize_vtable();

            let _suspended = SuspendedAllThreads::new(
                self.suspend_all
                    .expect("suspend_all symbol checked before enumeration"),
                self.resume_all
                    .expect("resume_all symbol checked before enumeration"),
                layout.thread_list,
            );

            // SAFETY: All pointers were resolved from ART, the current thread is in runnable
            // state for ART internal object access, and all ART threads are suspended while the
            // class-linker visitor walks loader objects.
            unsafe {
                visit_class_loaders(layout.class_linker, &mut visitor);
            }

            let vm_handle = vm.handle().as_ptr();
            for loader in visitor.take_loaders() {
                // SAFETY: `loader` is an ART mirror::ClassLoader object delivered by
                // VisitClassLoaders for this VM. AddGlobalRef turns it into a JNI global handle.
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

        loader_globals
            .into_iter()
            .map(|loader| unsafe {
                ClassLoaderRef::from_global_raw(
                    vm.clone(),
                    loader,
                    crate::java::ClassLoaderKind::Enumerated,
                )
            })
            .collect()
    }

    pub(crate) fn enumerate_loaded_classes(&self, vm: &Vm) -> Result<Vec<JavaClass>> {
        self.ensure_loaded_class_enumeration_supported(vm.handle())?;
        let env = vm.attach_current_thread()?;
        let layout = detect_runtime_layout(vm.handle(), FEATURE_LOADED_CLASS_ENUMERATION)
            .expect("runtime layout support checked before loaded-class enumeration");
        let mut class_globals = Vec::new();

        self.with_runnable_art_thread(&env, FEATURE_LOADED_CLASS_ENUMERATION, |thread| {
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before class enumeration");
            let visit_classes = self
                .visit_classes
                .expect("visit_classes symbol checked before class enumeration");
            let get_class_descriptor = self
                .get_class_descriptor
                .expect("get_class_descriptor symbol checked before class enumeration");
            let mut processor = ArtClassProcessor::new(
                add_global_ref,
                get_class_descriptor,
                vm,
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

        let mut classes = Vec::with_capacity(class_globals.len());
        while let Some(raw_class) = class_globals.pop() {
            let raw = raw_class.raw;
            match java_class_from_loaded(vm, raw_class) {
                Ok(class) => classes.push(class),
                Err(error) => {
                    unsafe { env.delete_global_ref_raw(raw) };
                    for remaining in class_globals {
                        unsafe { env.delete_global_ref_raw(remaining.raw) };
                    }
                    return Err(error);
                }
            }
        }
        classes.reverse();
        Ok(classes)
    }

    pub(crate) fn enumerate_methods(
        &self,
        vm: &Vm,
        query: &str,
    ) -> Result<Vec<metadata::JavaMethodQueryGroup>> {
        let query = metadata::parse_method_query(query)?;
        self.ensure_method_query_supported(vm.handle())?;

        let env = vm.attach_current_thread()?;
        let runtime_layout = detect_runtime_layout(vm.handle(), FEATURE_METHOD_QUERY)
            .expect("runtime layout support checked before ART method query");
        let memory = MemoryRanges::current()?;

        let thread_class = env.find_class("java/lang/Thread")?;
        let thread_get_name = env.get_method(&thread_class, "getName", "()Ljava/lang/String;")?;
        let thread_is_alive = env.get_method(&thread_class, "isAlive", "()Z")?;
        let thread_current_thread =
            env.get_static_method(&thread_class, "currentThread", "()Ljava/lang/Thread;")?;
        let system_class = env.find_class("java/lang/System")?;
        let system_current_time_millis =
            env.get_static_method(&system_class, "currentTimeMillis", "()J")?;

        let mut raw_groups = Vec::new();
        let query_result = self.with_runnable_art_thread(&env, FEATURE_METHOD_QUERY, |thread| {
            let visit_classes = self
                .visit_classes
                .expect("visit_classes symbol checked before method query");
            let add_global_ref = self
                .add_global_ref
                .expect("add_global_ref symbol checked before method query");
            let get_class_descriptor = self
                .get_class_descriptor
                .expect("get_class_descriptor symbol checked before method query");
            let pretty_method = self
                .pretty_method
                .clone()
                .expect("pretty_method symbol checked before method query");

            let thread_method = self.art_method_from_jni_id(&runtime_layout, thread_get_name.raw());
            let thread_is_alive_method =
                self.art_method_from_jni_id(&runtime_layout, thread_is_alive.raw());
            let thread_current_thread_method =
                self.art_method_from_jni_id(&runtime_layout, thread_current_thread.raw());
            let process_method =
                self.art_method_from_jni_id(&runtime_layout, system_current_time_millis.raw());
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
                vm,
                thread,
                &query,
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

        raw_method_groups_to_public(vm, raw_groups)
    }

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

    pub(crate) fn method_replacement_support(&self, vm: &Vm) -> FeatureSupport {
        match self.detect_method_replacement_prerequisites(vm) {
            Ok(_) => unsupported_support(
                "ART method replacement prerequisites are available for hidden experimental selected static and instance clone-active replacement; public replacement API is not implemented yet",
            ),
            Err(Error::UnsupportedFeature { reason, .. }) => unsupported_support(reason),
            Err(error) => unsupported_support(error.to_string()),
        }
    }

    pub(crate) fn replace_method(
        &self,
        vm: &Vm,
        kind: MethodKind,
        method_id: jni::jmethodID,
        replacement: *mut c_void,
    ) -> Result<ArtMethodReplacementGuard> {
        if replacement.is_null() {
            return Err(Error::NullReturn {
                operation: "ART replacement function",
            });
        }

        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        validate_replacement_function(replacement, &memory)?;
        let api_level = android_api_level(FEATURE_METHOD_REPLACEMENT)?;
        let layout = self.detect_method_replacement_prerequisites(vm)?;
        self.replacement_controller.ensure_hooks()?;
        self.replacement_controller
            .ensure_quick_entrypoint_hooks(&layout.trampolines)?;
        let mut guard = None;

        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |_thread| {
            let candidates = self.art_method_from_jni_id(&layout.runtime, method_id);
            let compile_dont_bother = compile_dont_bother_flag(api_level);
            let mut saw_readable_candidate = false;
            let mut saw_wrong_kind_candidate = false;
            for method in candidates {
                let Ok(original) = snapshot_art_method(method, &layout.method, &memory) else {
                    continue;
                };
                saw_readable_candidate = true;
                if !art_method_kind_matches(original, kind) {
                    saw_wrong_kind_candidate = true;
                    continue;
                }
                let clone_patched = patched_replacement_method(
                    original,
                    replacement,
                    layout.trampolines.quick_generic_jni_trampoline,
                    compile_dont_bother,
                );
                let cloned_method = clone_replacement_art_method(
                    method,
                    &layout.method,
                    original,
                    clone_patched,
                    &memory,
                )?;
                let dispatch_thunk = ArtMethodDispatchThunk::new(
                    cloned_method.as_ptr(),
                    layout.trampolines.quick_to_interpreter_bridge_trampoline,
                    layout.method.quick_code_offset,
                    layout.thread_managed_stack_offset,
                )?;
                let original_patched = patched_original_method_for_clone_dispatch(
                    original,
                    dispatch_thunk.as_ptr(),
                    compile_dont_bother,
                );
                let _suspended = self.suspend_all_threads(&layout.runtime)?;
                self.replacement_controller.register(
                    method,
                    cloned_method.as_ptr(),
                    ArtReplacementSynchronization {
                        quick_code_offset: layout.method.quick_code_offset,
                        thread_managed_stack_offset: layout.thread_managed_stack_offset,
                        nterp_entrypoint: None,
                        quick_to_interpreter_bridge: layout
                            .trampolines
                            .quick_to_interpreter_bridge_trampoline
                            as usize,
                    },
                );
                if let Err(error) = patch_art_method_verified(
                    method,
                    &layout.method,
                    original,
                    original_patched,
                    &memory,
                ) {
                    self.replacement_controller.unregister(method);
                    return Err(error);
                }
                self.replacement_controller
                    .synchronize_replacement_methods();
                guard = Some(ArtMethodReplacementGuard {
                    backend: self.clone(),
                    vm: vm.clone(),
                    method,
                    cloned_method,
                    dispatch_thunk,
                    layout,
                    original,
                    original_patched,
                    clone_patched,
                    reverted: false,
                });
                return Ok(());
            }

            if saw_wrong_kind_candidate {
                let reason = match kind {
                    MethodKind::Static => "resolved target ArtMethod is not static",
                    MethodKind::Instance => "resolved target ArtMethod is static",
                    MethodKind::Constructor => "resolved target ArtMethod is a constructor",
                };
                return unsupported_feature(FEATURE_METHOD_REPLACEMENT, reason);
            }
            if saw_readable_candidate {
                let reason = match kind {
                    MethodKind::Static => {
                        "unable to resolve a static target ArtMethod from JNI method ID"
                    }
                    MethodKind::Instance => {
                        "unable to resolve an instance target ArtMethod from JNI method ID"
                    }
                    MethodKind::Constructor => {
                        "unable to resolve a constructor target ArtMethod from JNI method ID"
                    }
                };
                return unsupported_feature(FEATURE_METHOD_REPLACEMENT, reason);
            }
            unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to resolve target ArtMethod from JNI method ID: no readable candidates",
            )
        })?;

        guard.ok_or(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "method replacement did not produce a guard".to_owned(),
        })
    }

    fn restore_method(
        &self,
        vm: &Vm,
        method: *mut c_void,
        layout: &ArtMethodReplacementLayout,
        original: ArtMethodSnapshot,
    ) -> Result<()> {
        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |_thread| {
            let _suspended = self.suspend_all_threads(&layout.runtime)?;
            restore_art_method_verified(method, &layout.method, original, &memory)
        })
    }

    fn ensure_class_loader_enumeration_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(
            FEATURE_CLASS_LOADER_ENUMERATION,
            self.class_loader_enumeration_support(vm),
        )
    }

    fn ensure_loaded_class_enumeration_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(
            FEATURE_LOADED_CLASS_ENUMERATION,
            self.loaded_class_enumeration_support(vm),
        )
    }

    fn ensure_method_query_supported(&self, vm: NonNull<jni::JavaVM>) -> Result<()> {
        ensure_feature_supported(FEATURE_METHOD_QUERY, self.method_query_support(vm))
    }

    fn detect_method_replacement_prerequisites(
        &self,
        vm: &Vm,
    ) -> Result<ArtMethodReplacementLayout> {
        self.replacement_controller.ensure_dispatch_supported()?;
        if self.pretty_method.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ArtMethod::PrettyMethod is unavailable",
            );
        }
        if !cfg!(target_arch = "aarch64") {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "only arm64-v8a is supported in this milestone",
            );
        }
        if self.suspend_all.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ThreadList::SuspendAll is unavailable for safe method patching",
            );
        }
        if self.resume_all.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "ThreadList::ResumeAll is unavailable for safe method patching",
            );
        }
        let android_runtime = self
            .android_runtime
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "libandroid_runtime.so is unavailable".to_owned(),
            })?;

        let env = vm.attach_current_thread()?;
        let memory = MemoryRanges::current_for_feature(FEATURE_METHOD_REPLACEMENT)?;
        let api_level = android_api_level(FEATURE_METHOD_REPLACEMENT)?;
        let (runtime_layout, trampolines) = detect_runtime_layout_for_method_replacement(
            vm.handle(),
            api_level,
            self.set_jni_id_type,
            self.class_linker_entrypoint_predicates(),
            &memory,
            FEATURE_METHOD_REPLACEMENT,
        )?;
        validate_replacement_trampoline(&trampolines, &memory)?;
        if runtime_layout.uses_indirect_jni_ids() && self.decode_method_id.is_none() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "JniIdManager::DecodeMethodId is unavailable for indirect JNI method IDs",
            );
        }
        let layout_method = env
            .find_class("android/os/Process")
            .and_then(|class| env.get_static_method(&class, "getElapsedCpuTime", "()J"))
            .or_else(|_| {
                let system_class = env.find_class("java/lang/System")?;
                env.get_static_method(&system_class, "currentTimeMillis", "()J")
            })?;

        let mut layout = None;
        self.with_runnable_art_thread(&env, FEATURE_METHOD_REPLACEMENT, |thread| {
            let process_method = self.art_method_from_jni_id(&runtime_layout, layout_method.raw());
            let method_layout = detect_art_method_replacement_layout(
                &process_method,
                android_runtime,
                api_level,
                &memory,
                true,
                FEATURE_METHOD_REPLACEMENT,
            )?;
            layout = Some(ArtMethodReplacementLayout {
                api_level,
                runtime: runtime_layout,
                method: method_layout,
                trampolines,
                thread_managed_stack_offset: detect_art_thread_managed_stack_offset(
                    FEATURE_METHOD_REPLACEMENT,
                    thread,
                    env.handle().as_ptr().cast(),
                )?,
            });
            Ok(())
        })?;

        layout.ok_or(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "method replacement prerequisites were not probed".to_owned(),
        })
    }

    fn art_method_from_jni_id(
        &self,
        layout: &ArtRuntimeLayout,
        method_id: jni::jmethodID,
    ) -> Vec<*mut c_void> {
        if layout.uses_indirect_jni_ids() {
            return self
                .decode_method_id
                .and_then(|decode_method_id| {
                    let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
                    (!decoded.is_null()).then_some(decoded)
                })
                .into_iter()
                .collect();
        }

        let mut candidates = vec![method_id.cast::<c_void>()];
        if layout.jni_ids_indirection.is_none()
            && let Some(decode_method_id) = self.decode_method_id
            && !layout.jni_id_manager.is_null()
        {
            let decoded = unsafe { decode_method_id(layout.jni_id_manager, method_id) };
            if !decoded.is_null() && !candidates.contains(&decoded) {
                candidates.push(decoded);
            }
        }

        candidates
    }

    fn class_linker_entrypoint_predicates(&self) -> Option<ArtClassLinkerEntrypointPredicates> {
        Some(ArtClassLinkerEntrypointPredicates {
            is_quick_resolution_stub: self.is_quick_resolution_stub?,
            is_quick_to_interpreter_bridge: self.is_quick_to_interpreter_bridge?,
            is_quick_generic_jni_stub: self.is_quick_generic_jni_stub?,
        })
    }

    fn suspend_all_threads(&self, layout: &ArtRuntimeLayout) -> Result<SuspendedAllThreads> {
        let suspend_all = self.suspend_all.ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "ThreadList::SuspendAll is unavailable for safe method patching".to_owned(),
        })?;
        let resume_all = self.resume_all.ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "ThreadList::ResumeAll is unavailable for safe method patching".to_owned(),
        })?;
        Ok(SuspendedAllThreads::new(
            suspend_all,
            resume_all,
            layout.thread_list,
        ))
    }

    fn with_runnable_art_thread(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
        f: impl FnOnce(*mut c_void) -> Result<()>,
    ) -> Result<()> {
        let transition = self.thread_transition(env, feature)?;
        transition.run(feature, env, f)
    }

    fn thread_transition(
        &self,
        env: &crate::env::Env<'_>,
        feature: &'static str,
    ) -> Result<&thread_transition::ThreadTransition> {
        if let Some(transition) = self.thread_transition.get() {
            return Ok(transition);
        }

        let transition =
            thread_transition::build(feature, env, self.exception_clear, self.fatal_error)?;
        let _ = self.thread_transition.set(transition);
        Ok(self
            .thread_transition
            .get()
            .expect("thread transition was just initialized"))
    }
}

impl ArtModuleRange {
    pub(crate) fn from_module(module: &Module) -> Self {
        let range = module.range();
        let start = range.base_address().0 as usize;
        let end = start.saturating_add(range.size());
        Self { start, end }
    }

    fn contains(&self, address: usize) -> bool {
        let address = normalize_address(address);
        address >= self.start && address < self.end
    }
}

impl ArtClassLoaderVisitor {
    fn new(loaders: &mut Vec<*mut c_void>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            loaders,
        }
    }

    fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class_loader as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }

    fn take_loaders(&mut self) -> Vec<*mut c_void> {
        let loaders = unsafe { &mut *self.loaders };
        std::mem::take(loaders)
    }
}

impl ArtClassVisitor {
    fn new_loaded(processor: &mut ArtClassProcessor<'_>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut ArtClassProcessor<'_>).cast(),
            visit: visit_loaded_class,
        }
    }

    fn new_finder(processor: &mut FindArtClassProcessor) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut FindArtClassProcessor).cast(),
            visit: visit_find_art_class,
        }
    }

    fn new_method_query(processor: &mut ArtMethodQueryProcessor<'_>) -> Self {
        Self {
            vtable: std::ptr::null(),
            vtable_storage: [std::ptr::null(); 3],
            context: (processor as *mut ArtMethodQueryProcessor<'_>).cast(),
            visit: visit_method_query_class,
        }
    }

    fn initialize_vtable(&mut self) {
        self.vtable_storage[2] = on_visit_class as *const c_void;
        self.vtable = self.vtable_storage.as_ptr();
    }
}

impl<'callback> ArtClassProcessor<'callback> {
    fn new(
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

    fn visit(&mut self, class: *mut c_void) -> bool {
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

    fn take_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn promote(&self, class: *mut c_void) -> Result<RawLoadedClass> {
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
    fn call(&self, method: *mut c_void, with_signature: bool) -> Result<String> {
        let mut storage = ArtStdString { storage: [0; 3] };
        unsafe { (self.function)(&mut storage, method, with_signature) };
        let result = storage.to_string();
        storage.destroy();
        result
    }
}

impl FindArtClassProcessor {
    fn new(get_class_descriptor: GetClassDescriptor, descriptor: &'static str) -> Self {
        Self {
            get_class_descriptor,
            descriptor,
            class: None,
            error: None,
        }
    }

    fn visit(&mut self, class: *mut c_void) -> bool {
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

    fn take_result(&mut self) -> Result<*mut c_void> {
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
    fn new(
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

    fn visit(&mut self, class: *mut c_void) -> bool {
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

    fn take_error(&mut self) -> Result<()> {
        if let Some(error) = self.error.take() {
            Err(error)
        } else {
            Ok(())
        }
    }

    fn collect_class(&mut self, class: *mut c_void) -> Result<()> {
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

fn java_class_from_loaded(vm: &Vm, class: RawLoadedClass) -> Result<JavaClass> {
    let global = unsafe { GlobalRef::<ClassKind>::from_raw(vm.clone(), class.raw)? };
    Ok(JavaClass::from_global(vm.clone(), class.name, global))
}

fn class_descriptor_from_art(
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

fn art_method_metadata(
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

fn detect_method_query_layout(
    visit_classes: VisitClassesKind,
    class_linker: *mut c_void,
    get_class_descriptor: GetClassDescriptor,
    thread_method_candidates: &[Vec<*mut c_void>],
    process_method_candidates: Vec<*mut c_void>,
    memory: &MemoryRanges,
) -> Result<ArtMethodQueryLayout> {
    let method_layout =
        detect_art_method_runtime_layout(&process_method_candidates, memory, FEATURE_METHOD_QUERY)?;
    let thread_class = find_art_class_by_descriptor(
        visit_classes,
        class_linker,
        get_class_descriptor,
        "Ljava/lang/Thread;",
    )?;
    let class_layout = detect_thread_class_method_layout(
        thread_class,
        thread_method_candidates,
        method_layout.method_size,
        memory,
    )?;
    Ok(ArtMethodQueryLayout {
        class_methods_offset: class_layout.class_methods_offset,
        class_copied_methods_offset: class_layout.class_copied_methods_offset,
        method_size: class_layout.method_size,
        method_access_flags_offset: method_layout.access_flags_offset,
    })
}

fn find_art_class_by_descriptor(
    visit_classes: VisitClassesKind,
    class_linker: *mut c_void,
    get_class_descriptor: GetClassDescriptor,
    descriptor: &'static str,
) -> Result<*mut c_void> {
    let mut processor = FindArtClassProcessor::new(get_class_descriptor, descriptor);
    match visit_classes {
        VisitClassesKind::Visitor(visit_classes) => {
            let mut visitor = ArtClassVisitor::new_finder(&mut processor);
            visitor.initialize_vtable();
            unsafe { visit_classes(class_linker, &mut visitor) };
        }
        VisitClassesKind::Callback(visit_classes) => unsafe {
            visit_classes(
                class_linker,
                on_visit_find_art_class_callback,
                (&mut processor as *mut FindArtClassProcessor).cast(),
            );
        },
    }
    processor.take_result()
}

unsafe extern "C" fn on_visit_find_art_class_callback(
    class: *mut c_void,
    context: *mut c_void,
) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_find_art_class(context, class) }
}

fn detect_thread_class_method_layout(
    thread_class: *mut c_void,
    method_candidates: &[Vec<*mut c_void>],
    method_size: usize,
    memory: &MemoryRanges,
) -> Result<ArtMethodQueryLayout> {
    for methods_offset in (0..CLASS_LAYOUT_SCAN_LIMIT).step_by(4) {
        let Some(array) = read_art_array(thread_class, methods_offset, POINTER_SIZE, memory) else {
            continue;
        };
        if array.length == 0 || array.length > ART_METHOD_ARRAY_MAX_PROBE {
            continue;
        }

        let Some(array_bytes) = array.length.checked_mul(method_size) else {
            continue;
        };
        if !memory.contains(array.data as usize, array_bytes) {
            continue;
        }
        if !art_method_array_contains_all(array, method_size, method_candidates) {
            continue;
        }

        let copied_methods_offset =
            detect_copied_methods_offset(thread_class, methods_offset, array.length, memory)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_QUERY,
                    reason: "unable to determine mirror::Class copied-method count offset"
                        .to_owned(),
                })?;
        return Ok(ArtMethodQueryLayout {
            class_methods_offset: methods_offset,
            class_copied_methods_offset: copied_methods_offset,
            method_size,
            method_access_flags_offset: 0,
        });
    }

    unsupported_method_query("unable to determine mirror::Class methods layout")
}

fn detect_class_linker_trampolines(
    layout: &ArtRuntimeLayout,
    api_level: i32,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
) -> Result<ArtClassLinkerTrampolines> {
    if layout.intern_table.is_null() {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "ART Runtime intern table pointer is null",
        );
    }

    let start_offset = if POINTER_SIZE == 4 { 100 } else { 200 };
    let end_offset = start_offset + (100 * POINTER_SIZE);
    for offset in (start_offset..end_offset).step_by(POINTER_SIZE) {
        let Some(value) = read_usize(
            (layout.class_linker as usize + offset) as *const c_void,
            memory,
        ) else {
            continue;
        };
        if value != layout.intern_table as usize {
            continue;
        }

        let delta = if api_level >= 30 {
            6
        } else if api_level >= 29 {
            4
        } else {
            3
        };
        let quick_generic_jni_offset = offset + (delta * POINTER_SIZE);
        let quick_resolution_offset = if api_level >= 23 {
            quick_generic_jni_offset - (2 * POINTER_SIZE)
        } else {
            quick_generic_jni_offset - (3 * POINTER_SIZE)
        };

        let trampolines = ArtClassLinkerTrampolines {
            quick_resolution_trampoline: read_trampoline(
                layout.class_linker,
                quick_resolution_offset,
                memory,
                "quick resolution trampoline",
            )?,
            quick_imt_conflict_trampoline: read_trampoline(
                layout.class_linker,
                quick_generic_jni_offset - POINTER_SIZE,
                memory,
                "quick IMT conflict trampoline",
            )?,
            quick_generic_jni_trampoline: read_trampoline(
                layout.class_linker,
                quick_generic_jni_offset,
                memory,
                "quick generic JNI trampoline",
            )?,
            quick_to_interpreter_bridge_trampoline: read_trampoline(
                layout.class_linker,
                quick_generic_jni_offset + POINTER_SIZE,
                memory,
                "quick-to-interpreter bridge trampoline",
            )?,
        };
        return Ok(trampolines);
    }

    detect_class_linker_trampolines_by_predicate(layout, predicates, memory)
}

fn detect_class_linker_trampolines_by_predicate(
    layout: &ArtRuntimeLayout,
    predicates: Option<ArtClassLinkerEntrypointPredicates>,
    memory: &MemoryRanges,
) -> Result<ArtClassLinkerTrampolines> {
    let Some(predicates) = predicates else {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "unable to determine ClassLinker trampoline offsets: intern table anchor was not found and ClassLinker quick-entrypoint predicate symbols are unavailable",
        );
    };

    let start_offset = if POINTER_SIZE == 4 { 100 } else { 200 };
    let end_offset = start_offset + (512 * POINTER_SIZE);
    let mut candidate = None;
    for quick_resolution_offset in
        (start_offset..end_offset - (3 * POINTER_SIZE)).step_by(POINTER_SIZE)
    {
        let Some(quick_resolution) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset) as *const c_void,
            memory,
        ) else {
            continue;
        };
        let Some(_quick_imt_conflict) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset + POINTER_SIZE)
                as *const c_void,
            memory,
        ) else {
            continue;
        };
        let Some(quick_generic_jni) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset + (2 * POINTER_SIZE))
                as *const c_void,
            memory,
        ) else {
            continue;
        };
        let Some(quick_to_interpreter) = read_usize(
            (layout.class_linker as usize + quick_resolution_offset + (3 * POINTER_SIZE))
                as *const c_void,
            memory,
        ) else {
            continue;
        };

        let class_linker = normalize_address(layout.class_linker as usize) as *mut c_void;
        let is_match = unsafe {
            (predicates.is_quick_resolution_stub)(class_linker, quick_resolution as *const c_void)
                && (predicates.is_quick_generic_jni_stub)(
                    class_linker,
                    quick_generic_jni as *const c_void,
                )
                && (predicates.is_quick_to_interpreter_bridge)(
                    class_linker,
                    quick_to_interpreter as *const c_void,
                )
        };
        if !is_match {
            continue;
        }

        let quick_generic_offset = quick_resolution_offset + (2 * POINTER_SIZE);
        let trampolines = ArtClassLinkerTrampolines {
            quick_resolution_trampoline: read_trampoline(
                layout.class_linker,
                quick_resolution_offset,
                memory,
                "quick resolution trampoline",
            )?,
            quick_imt_conflict_trampoline: read_trampoline(
                layout.class_linker,
                quick_resolution_offset + POINTER_SIZE,
                memory,
                "quick IMT conflict trampoline",
            )?,
            quick_generic_jni_trampoline: read_trampoline(
                layout.class_linker,
                quick_generic_offset,
                memory,
                "quick generic JNI trampoline",
            )?,
            quick_to_interpreter_bridge_trampoline: read_trampoline(
                layout.class_linker,
                quick_generic_offset + POINTER_SIZE,
                memory,
                "quick-to-interpreter bridge trampoline",
            )?,
        };
        if candidate.replace(trampolines).is_some() {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "unable to determine ClassLinker trampoline offsets: predicate scan found multiple candidates",
            );
        }
    }

    if let Some(trampolines) = candidate {
        return Ok(trampolines);
    }
    unsupported_feature(
        FEATURE_METHOD_REPLACEMENT,
        "unable to determine ClassLinker trampoline offsets: intern table anchor was not found and predicate scan found no quick trampoline sequence",
    )
}

fn read_trampoline(
    class_linker: *mut c_void,
    offset: usize,
    memory: &MemoryRanges,
    name: &'static str,
) -> Result<*mut c_void> {
    let Some(value) = read_usize((class_linker as usize + offset) as *const c_void, memory) else {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("unable to read ClassLinker {name} at offset {offset:#x}"),
        );
    };
    if value == 0 {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("ClassLinker {name} at offset {offset:#x} is null"),
        );
    }
    if !memory.contains_executable(value, 1) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            format!("ClassLinker {name} at offset {offset:#x} is not executable"),
        );
    }
    Ok(value as *mut c_void)
}

fn art_method_array_contains_all(
    array: ArtArray,
    method_size: usize,
    method_candidates: &[Vec<*mut c_void>],
) -> bool {
    method_candidates.iter().all(|candidates| {
        (0..array.length).any(|index| {
            let method = unsafe { array.data.byte_add(index * method_size) };
            candidates.contains(&method)
        })
    })
}

fn detect_copied_methods_offset(
    class: *mut c_void,
    methods_offset: usize,
    method_count: usize,
    memory: &MemoryRanges,
) -> Option<usize> {
    if method_count > u16::MAX as usize {
        return None;
    }
    for offset in (methods_offset..CLASS_LAYOUT_SCAN_LIMIT).step_by(4) {
        let value = read_u16((class as usize + offset) as *const c_void, memory)?;
        if value as usize == method_count {
            return Some(offset);
        }
    }
    None
}

fn detect_art_method_runtime_layout(
    method_candidates: &[*mut c_void],
    memory: &MemoryRanges,
    feature: &'static str,
) -> Result<ArtMethodRuntimeLayout> {
    let expected_native = 0x0001 | K_ACC_STATIC | K_ACC_NATIVE;
    let expected_final_native = expected_native | K_ACC_FINAL;
    let mask = !(K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_PUBLIC_API
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG);
    for &method in method_candidates {
        if method.is_null() || !memory.contains(method as usize, METHOD_LAYOUT_SCAN_LIMIT) {
            continue;
        }
        let mut access_flags_offset = None;
        for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(4) {
            let Some(flags) = read_u32((method as usize + offset) as *const c_void, memory) else {
                continue;
            };
            let relevant_flags = flags & mask;
            if relevant_flags == expected_native || relevant_flags == expected_final_native {
                access_flags_offset = Some(offset);
                break;
            }
        }

        let Some(access_flags_offset) = access_flags_offset else {
            continue;
        };
        let Some(entrypoints) = detect_art_method_entrypoints(method, memory) else {
            continue;
        };
        return Ok(ArtMethodRuntimeLayout {
            method_size: entrypoints.method_size,
            access_flags_offset,
            jni_code_offset: entrypoints.jni_code_offset,
            quick_code_offset: entrypoints.quick_code_offset,
            interpreter_code_offset: entrypoints.interpreter_code_offset,
        });
    }

    unsupported_feature(feature, "unable to determine ArtMethod runtime layout")
}

fn detect_art_method_replacement_layout(
    method_candidates: &[*mut c_void],
    native_runtime: ArtModuleRange,
    api_level: i32,
    memory: &MemoryRanges,
    allow_executable_entrypoint_fallback: bool,
    feature: &'static str,
) -> Result<ArtMethodRuntimeLayout> {
    let expected_native = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
    let expected_non_final_native = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE;
    let entrypoint_field_size = if api_level <= 21 { 8 } else { POINTER_SIZE };
    let mut saw_candidate = false;
    let mut saw_native_runtime_entrypoint = false;
    let mut saw_any_executable_entrypoint = false;
    let mut saw_access_flags = false;

    for &method in method_candidates {
        if method.is_null() || !memory.contains(method as usize, METHOD_LAYOUT_SCAN_LIMIT) {
            continue;
        }
        saw_candidate = true;

        let mut jni_code_offset = None;
        let mut executable_jni_code_offset = None;
        let mut access_flags_offset = None;
        for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(4) {
            if jni_code_offset.is_none()
                && let Some(address) =
                    read_usize((method as usize + offset) as *const c_void, memory)
            {
                if native_runtime.contains(address) {
                    jni_code_offset = Some(offset);
                    saw_native_runtime_entrypoint = true;
                    saw_any_executable_entrypoint = true;
                } else if executable_jni_code_offset.is_none()
                    && allow_executable_entrypoint_fallback
                    && memory.contains_executable(address, 1)
                {
                    executable_jni_code_offset = Some(offset);
                    saw_any_executable_entrypoint = true;
                }
            }

            if access_flags_offset.is_none()
                && let Some(flags) = read_u32((method as usize + offset) as *const c_void, memory)
                && matches!(
                    flags & K_ACC_JAVA_FLAGS_MASK,
                    value if value == expected_native || value == expected_non_final_native
                )
            {
                access_flags_offset = Some(offset);
                saw_access_flags = true;
            }

            if jni_code_offset.is_some() && access_flags_offset.is_some() {
                break;
            }
        }

        let jni_code_offset = jni_code_offset.or(executable_jni_code_offset);
        let (Some(jni_code_offset), Some(access_flags_offset)) =
            (jni_code_offset, access_flags_offset)
        else {
            continue;
        };

        let Some(quick_code_offset) = jni_code_offset.checked_add(entrypoint_field_size) else {
            continue;
        };
        let Some(method_size) =
            quick_code_offset.checked_add(if api_level <= 21 { 32 } else { POINTER_SIZE })
        else {
            continue;
        };
        if !(ART_METHOD_MIN_SIZE..=ART_METHOD_MAX_SIZE).contains(&method_size)
            || !memory.contains(method as usize, method_size)
        {
            continue;
        }

        return Ok(ArtMethodRuntimeLayout {
            method_size,
            access_flags_offset,
            jni_code_offset,
            quick_code_offset,
            interpreter_code_offset: None,
        });
    }

    let reason = if !saw_candidate {
        "unable to determine ArtMethod runtime layout: no readable method candidates"
    } else if !saw_any_executable_entrypoint {
        "unable to determine ArtMethod runtime layout: native entrypoint is not executable"
    } else if !saw_native_runtime_entrypoint && !allow_executable_entrypoint_fallback {
        "unable to determine ArtMethod runtime layout: native entrypoint is outside libandroid_runtime.so"
    } else if !saw_access_flags {
        "unable to determine ArtMethod runtime layout: native access flags were not found"
    } else {
        "unable to determine ArtMethod runtime layout: derived layout is not readable"
    };
    unsupported_feature(feature, reason)
}

fn snapshot_art_method(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    memory: &MemoryRanges,
) -> Result<ArtMethodSnapshot> {
    if method.is_null() || !memory.contains(method as usize, layout.method_size) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "target ArtMethod is not readable",
        );
    }

    let access_flags = read_u32(
        unsafe { method.byte_add(layout.access_flags_offset) },
        memory,
    )
    .ok_or_else(|| Error::UnsupportedFeature {
        feature: FEATURE_METHOD_REPLACEMENT,
        reason: "target ArtMethod access flags are not readable".to_owned(),
    })?;
    let jni_code = read_usize(unsafe { method.byte_add(layout.jni_code_offset) }, memory)
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "target ArtMethod JNI entrypoint is not readable".to_owned(),
        })? as *mut c_void;
    let quick_code = read_usize(unsafe { method.byte_add(layout.quick_code_offset) }, memory)
        .ok_or_else(|| Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: "target ArtMethod quick entrypoint is not readable".to_owned(),
        })? as *mut c_void;
    let interpreter_code = layout
        .interpreter_code_offset
        .map(|offset| {
            read_usize(unsafe { method.byte_add(offset) }, memory)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature: FEATURE_METHOD_REPLACEMENT,
                    reason: "target ArtMethod interpreter entrypoint is not readable".to_owned(),
                })
                .map(|value| value as *mut c_void)
        })
        .transpose()?;

    Ok(ArtMethodSnapshot {
        access_flags,
        jni_code,
        quick_code,
        interpreter_code,
    })
}

fn validate_replacement_function(replacement: *mut c_void, memory: &MemoryRanges) -> Result<()> {
    if replacement.is_null() {
        return Err(Error::NullReturn {
            operation: "ART replacement function",
        });
    }
    if !memory.contains_executable(replacement as usize, 1) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "replacement function is not executable",
        );
    }
    Ok(())
}

fn validate_replacement_trampoline(
    trampolines: &ArtClassLinkerTrampolines,
    memory: &MemoryRanges,
) -> Result<()> {
    let trampoline = trampolines.quick_generic_jni_trampoline;
    if trampoline.is_null() || !memory.contains_executable(trampoline as usize, 1) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "ClassLinker quick generic JNI trampoline is unavailable or not executable",
        );
    }
    Ok(())
}

fn art_quick_entrypoint_from_trampoline(
    trampoline: *mut c_void,
    thread: *mut c_void,
    memory: &MemoryRanges,
) -> Result<*mut c_void> {
    if trampoline.is_null() || !memory.contains_executable(trampoline as usize, 4) {
        return unsupported_feature(
            FEATURE_METHOD_REPLACEMENT,
            "quick-to-interpreter bridge trampoline is not executable",
        );
    }
    if thread.is_null() {
        return unsupported_feature(FEATURE_METHOD_REPLACEMENT, "ART thread pointer is null");
    }

    let instruction = unsafe { trampoline.cast::<u32>().read() };
    if let Some(offset) = aarch64_ldr_unsigned_immediate_offset(instruction) {
        let Some(slot) = (thread as usize).checked_add(offset) else {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "quick-to-interpreter bridge thread entrypoint slot overflowed",
            );
        };
        if !memory.contains(slot, POINTER_SIZE) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "quick-to-interpreter bridge thread entrypoint slot is not readable",
            );
        }
        let pointer = unsafe { thread.byte_add(offset).cast::<usize>().read() as *mut c_void };
        if pointer.is_null() || !memory.contains_executable(pointer as usize, 4) {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                "resolved quick-to-interpreter bridge entrypoint is not executable",
            );
        }
        Ok(pointer)
    } else {
        Ok(trampoline)
    }
}

fn aarch64_ldr_unsigned_immediate_offset(instruction: u32) -> Option<usize> {
    if instruction & 0xffc0_0000 != 0xf940_0000 {
        return None;
    }
    Some(((instruction >> 10) as usize & 0x0fff) * 8)
}

fn art_method_kind_matches(snapshot: ArtMethodSnapshot, kind: MethodKind) -> bool {
    match kind {
        MethodKind::Static => snapshot.access_flags & K_ACC_STATIC != 0,
        MethodKind::Instance => snapshot.access_flags & (K_ACC_STATIC | K_ACC_CONSTRUCTOR) == 0,
        MethodKind::Constructor => snapshot.access_flags & K_ACC_CONSTRUCTOR != 0,
    }
}

fn detect_art_thread_managed_stack_offset(
    feature: &'static str,
    thread: *mut c_void,
    env: *mut c_void,
) -> Result<usize> {
    if thread.is_null() {
        return unsupported_feature(feature, "ART Thread pointer is null");
    }

    let thread = thread.cast::<usize>();
    let env_value = env as usize;
    for offset in (144..256).step_by(POINTER_SIZE) {
        let value = unsafe { thread.byte_add(offset).read() };
        if value == env_value {
            return offset
                .checked_sub(4 * POINTER_SIZE)
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature,
                    reason: "ART Thread managed stack offset underflowed".to_owned(),
                });
        }
    }

    unsupported_feature(
        feature,
        "unable to determine ART Thread managed stack offset",
    )
}

fn replacement_frame_is_active(
    replacement: usize,
    thread: usize,
    thread_managed_stack_offset: usize,
) -> bool {
    if replacement == 0 || thread == 0 {
        return false;
    }

    unsafe {
        let managed_stack = (thread + thread_managed_stack_offset) as *const usize;
        let top_quick_frame = ptr::read_unaligned(managed_stack) & !0x3usize;
        if top_quick_frame != 0 {
            return false;
        }

        let link = ptr::read_unaligned(managed_stack.byte_add(POINTER_SIZE));
        if link == 0 {
            return false;
        }

        let link_top_quick_frame = ptr::read_unaligned(link as *const usize) & !0x3usize;
        if link_top_quick_frame == 0 {
            return false;
        }

        ptr::read_unaligned(link_top_quick_frame as *const usize) == replacement
    }
}

fn patched_replacement_method(
    original: ArtMethodSnapshot,
    replacement: *mut c_void,
    quick_generic_jni_trampoline: *mut c_void,
    compile_dont_bother: u32,
) -> ArtMethodSnapshot {
    let removed_flags = K_ACC_CRITICAL_NATIVE
        | K_ACC_FAST_NATIVE
        | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
        | K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_SINGLE_IMPLEMENTATION
        | K_ACC_SKIP_ACCESS_CHECKS;
    ArtMethodSnapshot {
        access_flags: (original.access_flags & !removed_flags) | K_ACC_NATIVE | compile_dont_bother,
        jni_code: replacement,
        quick_code: quick_generic_jni_trampoline,
        interpreter_code: original.interpreter_code,
    }
}

fn patched_original_method_for_clone_dispatch(
    original: ArtMethodSnapshot,
    quick_to_interpreter_bridge_trampoline: *mut c_void,
    compile_dont_bother: u32,
) -> ArtMethodSnapshot {
    let removed_flags = K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
        | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
        | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
        | K_ACC_SINGLE_IMPLEMENTATION
        | K_ACC_SKIP_ACCESS_CHECKS;
    ArtMethodSnapshot {
        access_flags: (original.access_flags & !removed_flags) | compile_dont_bother,
        jni_code: original.jni_code,
        quick_code: quick_to_interpreter_bridge_trampoline,
        interpreter_code: original.interpreter_code,
    }
}

fn patch_art_method_verified(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    original: ArtMethodSnapshot,
    patched: ArtMethodSnapshot,
    memory: &MemoryRanges,
) -> Result<()> {
    patch_art_method(method, layout, patched);
    match snapshot_art_method(method, layout, memory) {
        Ok(snapshot) if snapshot == patched => Ok(()),
        Ok(snapshot) => {
            patch_art_method(method, layout, original);
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: format!(
                    "target ArtMethod patch verification failed: expected {patched:?}, got {snapshot:?}"
                ),
            })
        }
        Err(error) => {
            patch_art_method(method, layout, original);
            Err(error)
        }
    }
}

fn clone_replacement_art_method(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    original: ArtMethodSnapshot,
    patched: ArtMethodSnapshot,
    memory: &MemoryRanges,
) -> Result<ArtMethodClone> {
    let cloned_method = ArtMethodClone::copy_from(method, layout, memory)?;
    let clone_memory = cloned_method.memory_ranges();
    match snapshot_art_method(cloned_method.as_ptr(), layout, &clone_memory) {
        Ok(snapshot) if snapshot == original => {}
        Ok(snapshot) => {
            return unsupported_feature(
                FEATURE_METHOD_REPLACEMENT,
                format!(
                    "cloned ArtMethod snapshot mismatch: expected {original:?}, got {snapshot:?}"
                ),
            );
        }
        Err(error) => return Err(error),
    }
    patch_art_method_verified(
        cloned_method.as_ptr(),
        layout,
        original,
        patched,
        &clone_memory,
    )?;
    Ok(cloned_method)
}

fn restore_art_method_verified(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    original: ArtMethodSnapshot,
    memory: &MemoryRanges,
) -> Result<()> {
    patch_art_method(method, layout, original);
    match snapshot_art_method(method, layout, memory) {
        Ok(snapshot) if snapshot == original => Ok(()),
        Ok(snapshot) => Err(Error::UnsupportedFeature {
            feature: FEATURE_METHOD_REPLACEMENT,
            reason: format!(
                "target ArtMethod restore verification failed: expected {original:?}, got {snapshot:?}"
            ),
        }),
        Err(error) => Err(error),
    }
}

fn patch_art_method(
    method: *mut c_void,
    layout: &ArtMethodRuntimeLayout,
    snapshot: ArtMethodSnapshot,
) {
    write_u32(
        unsafe { method.byte_add(layout.access_flags_offset) },
        snapshot.access_flags,
    );
    write_usize(
        unsafe { method.byte_add(layout.jni_code_offset) },
        snapshot.jni_code as usize,
    );
    write_usize(
        unsafe { method.byte_add(layout.quick_code_offset) },
        snapshot.quick_code as usize,
    );
    if let (Some(offset), Some(interpreter_code)) =
        (layout.interpreter_code_offset, snapshot.interpreter_code)
    {
        write_usize(
            unsafe { method.byte_add(offset) },
            interpreter_code as usize,
        );
    }
}

fn compile_dont_bother_flag(api_level: i32) -> u32 {
    if api_level >= 27 {
        0x02000000
    } else if api_level >= 24 {
        0x01000000
    } else {
        0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtMethodEntrypoints {
    method_size: usize,
    jni_code_offset: usize,
    quick_code_offset: usize,
    interpreter_code_offset: Option<usize>,
}

fn detect_art_method_entrypoints(
    method: *mut c_void,
    memory: &MemoryRanges,
) -> Option<ArtMethodEntrypoints> {
    let mut previous_executable_pointer_offset: Option<usize> = None;
    for offset in (0..METHOD_LAYOUT_SCAN_LIMIT).step_by(POINTER_SIZE) {
        let value = read_usize((method as usize + offset) as *const c_void, memory)?;
        if !memory.contains_executable(value, 1) {
            continue;
        }

        if let Some(previous) = previous_executable_pointer_offset
            && offset == previous + POINTER_SIZE
        {
            let size = offset + POINTER_SIZE;
            if (ART_METHOD_MIN_SIZE..=ART_METHOD_MAX_SIZE).contains(&size) {
                let interpreter_code_offset =
                    previous.checked_sub(POINTER_SIZE).filter(|&offset| {
                        let pointer =
                            read_usize((method as usize + offset) as *const c_void, memory);
                        pointer.is_some_and(|pointer| memory.contains_executable(pointer, 1))
                    });
                return Some(ArtMethodEntrypoints {
                    method_size: size,
                    jni_code_offset: previous,
                    quick_code_offset: offset,
                    interpreter_code_offset,
                });
            }
        }
        previous_executable_pointer_offset = Some(offset);
    }

    None
}

fn read_art_array(
    object: *mut c_void,
    offset: usize,
    length_size: usize,
    memory: &MemoryRanges,
) -> Option<ArtArray> {
    let header = read_usize((object as usize + offset) as *const c_void, memory)? as *mut c_void;
    if header.is_null() || !memory.contains(header as usize, length_size) {
        return None;
    }

    let length = if length_size == 4 {
        read_u32(header.cast(), memory)? as usize
    } else {
        read_usize(header.cast(), memory)?
    };
    if length == 0 {
        return None;
    }

    let data = unsafe { header.byte_add(length_size) };
    Some(ArtArray { data, length })
}

fn read_usize(pointer: *const c_void, memory: &MemoryRanges) -> Option<usize> {
    let address = normalize_address(pointer as usize);
    if !memory.contains(address, POINTER_SIZE) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(address as *const usize) })
}

fn read_u32(pointer: *const c_void, memory: &MemoryRanges) -> Option<u32> {
    let address = normalize_address(pointer as usize);
    if !memory.contains(address, std::mem::size_of::<u32>()) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(address as *const u32) })
}

fn read_u16(pointer: *const c_void, memory: &MemoryRanges) -> Option<u16> {
    let address = normalize_address(pointer as usize);
    if !memory.contains(address, std::mem::size_of::<u16>()) {
        return None;
    }
    Some(unsafe { ptr::read_unaligned(address as *const u16) })
}

fn write_usize(pointer: *mut c_void, value: usize) {
    let address = normalize_address(pointer as usize);
    unsafe { ptr::write_unaligned(address as *mut usize, value) };
}

fn write_u32(pointer: *mut c_void, value: u32) {
    let address = normalize_address(pointer as usize);
    unsafe { ptr::write_unaligned(address as *mut u32, value) };
}

fn normalize_address(address: usize) -> usize {
    #[cfg(target_arch = "aarch64")]
    {
        if POINTER_SIZE == 8 {
            return address & 0x00ff_ffff_ffff_ffff;
        }
    }
    address
}

fn class_loader_key(class: *mut c_void) -> u32 {
    unsafe { ptr::read_unaligned((class as usize + (2 * 4)) as *const u32) }
}

fn raw_method_groups_to_public(
    vm: &Vm,
    raw_groups: Vec<RawMethodQueryGroup>,
) -> Result<Vec<metadata::JavaMethodQueryGroup>> {
    let env = vm.attach_current_thread()?;
    let mut groups = Vec::with_capacity(raw_groups.len());
    let mut remaining_loaders = raw_groups
        .iter()
        .filter_map(|group| group.loader)
        .collect::<Vec<_>>();

    for group in raw_groups {
        if let Some(raw) = group.loader
            && let Some(index) = remaining_loaders.iter().position(|loader| *loader == raw)
        {
            remaining_loaders.remove(index);
        }

        let loader = match group.loader {
            Some(raw) => match unsafe {
                ClassLoaderRef::from_global_raw(vm.clone(), raw, ClassLoaderKind::Enumerated)
            } {
                Ok(loader) => Some(loader),
                Err(error) => {
                    for raw in remaining_loaders {
                        unsafe { env.delete_global_ref_raw(raw) };
                    }
                    return Err(error);
                }
            },
            None => None,
        };
        groups.push(metadata::JavaMethodQueryGroup {
            loader,
            classes: group.classes,
        });
    }

    Ok(groups)
}

fn unsupported_method_query<T>(reason: impl Into<String>) -> Result<T> {
    unsupported_feature(FEATURE_METHOD_QUERY, reason)
}

impl ArtStdString {
    fn to_string(&self) -> Result<String> {
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

    fn data(&self) -> *const c_char {
        if self.storage[0] & 1 != 0 {
            self.storage[2] as *const c_char
        } else {
            (self as *const Self).cast::<u8>().wrapping_add(1).cast()
        }
    }

    fn destroy(&mut self) {
        if self.storage[0] & 1 != 0 {
            unsafe { free(self.storage[2] as *mut c_void) };
        }
    }
}

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

unsafe extern "C" fn on_visit_class_loader(
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

unsafe extern "C" fn on_visit_class(visitor: *mut ArtClassVisitor, class: *mut c_void) -> bool {
    if visitor.is_null() || class.is_null() {
        return true;
    }

    let visitor = unsafe { &mut *visitor };
    unsafe { (visitor.visit)(visitor.context, class) }
}

unsafe extern "C" fn on_visit_class_callback(class: *mut c_void, context: *mut c_void) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_loaded_class(context, class) }
}

unsafe extern "C" fn on_visit_method_query_callback(
    class: *mut c_void,
    context: *mut c_void,
) -> bool {
    if class.is_null() || context.is_null() {
        return true;
    }

    unsafe { visit_method_query_class(context, class) }
}

unsafe fn visit_loaded_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<ArtClassProcessor<'_>>() };
    processor.visit(class)
}

unsafe fn visit_find_art_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<FindArtClassProcessor>() };
    processor.visit(class)
}

unsafe fn visit_method_query_class(context: *mut c_void, class: *mut c_void) -> bool {
    let processor = unsafe { &mut *context.cast::<ArtMethodQueryProcessor<'_>>() };
    processor.visit(class)
}

struct SuspendedAllThreads {
    resume_all: ResumeAll,
    thread_list: *mut c_void,
}

impl SuspendedAllThreads {
    fn new(suspend_all: SuspendAll, resume_all: ResumeAll, thread_list: *mut c_void) -> Self {
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
    fn aarch64_pretty_method_thunk(target: *const c_void) -> Result<Self> {
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
    fn current() -> Result<Self> {
        Self::current_for_feature(FEATURE_METHOD_QUERY)
    }

    fn current_for_feature(feature: &'static str) -> Result<Self> {
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

    fn contains(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges
            .iter()
            .any(|range| address >= range.start && end <= range.end)
    }

    fn contains_executable(&self, address: usize, length: usize) -> bool {
        let address = normalize_address(address);
        let Some(end) = address.checked_add(length) else {
            return false;
        };
        self.ranges
            .iter()
            .any(|range| range.executable && address >= range.start && end <= range.end)
    }
}

fn write_u32_le(buffer: &mut [u8], offset: usize, value: u32) {
    buffer[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64_le(buffer: &mut [u8], offset: usize, value: u64) {
    buffer[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

unsafe extern "C" {
    fn mmap(
        address: *mut c_void,
        length: usize,
        protection: c_int,
        flags: c_int,
        file_descriptor: c_int,
        offset: isize,
    ) -> *mut c_void;
    fn mprotect(address: *mut c_void, length: usize, protection: c_int) -> c_int;
    fn munmap(address: *mut c_void, length: usize) -> c_int;
}

fn detect_runtime_layout(
    vm: NonNull<jni::JavaVM>,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let api_level = android_api_level(feature)?;
    detect_runtime_layout_for_api(vm, api_level, feature)
}

fn detect_runtime_layout_for_api(
    vm: NonNull<jni::JavaVM>,
    api_level: i32,
    feature: &'static str,
) -> Result<ArtRuntimeLayout> {
    let runtime = art_runtime_from_vm(vm);
    detect_runtime_layout_from_runtime(api_level, runtime, vm.as_ptr() as usize, feature)
}

fn detect_runtime_layout_for_method_replacement(
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

fn detect_runtime_layout_from_runtime(
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

            if thread_list.is_null() || class_linker.is_null() || intern_table.is_null() {
                continue;
            }

            return Ok(ArtRuntimeLayout {
                runtime: runtime.cast(),
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

fn detect_runtime_layout_and_trampolines_from_runtime(
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

            if thread_list.is_null() || class_linker.is_null() || intern_table.is_null() {
                continue;
            }

            let jni_ids_indirection =
                detect_jni_ids_indirection(runtime.cast(), set_jni_id_type, memory, feature)?;

            let layout = ArtRuntimeLayout {
                runtime: runtime.cast(),
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

fn class_linker_offsets_for_api(api_level: i32, vm_offset: usize) -> Vec<usize> {
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

fn detect_jni_ids_indirection(
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
fn detect_jni_ids_indirection_offset(
    feature: &'static str,
    set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    thread_transition::detect_jni_ids_indirection_offset(feature, set_jni_id_type)
}

#[cfg(not(target_arch = "aarch64"))]
fn detect_jni_ids_indirection_offset(
    _feature: &'static str,
    _set_jni_id_type: *const c_void,
) -> Result<Option<usize>> {
    Ok(None)
}

fn android_api_level(feature: &'static str) -> Result<i32> {
    let name = CString::new("ro.build.version.sdk").expect("property name has no interior NUL");
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return unsupported_feature(feature, "unable to read ro.build.version.sdk");
    }

    let value = unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .map_err(|_| Error::UnsupportedFeature {
            feature,
            reason: "ro.build.version.sdk is not valid UTF-8".to_owned(),
        })?;

    value.parse().map_err(|_| Error::UnsupportedFeature {
        feature,
        reason: format!("ro.build.version.sdk is not an integer: {value:?}"),
    })
}

fn runtime_layout_support(vm: NonNull<jni::JavaVM>, feature: &'static str) -> FeatureSupport {
    match detect_runtime_layout(vm, feature) {
        Ok(_) => FeatureSupport::Supported,
        Err(Error::UnsupportedFeature { reason, .. }) => FeatureSupport::Unsupported { reason },
        Err(error) => FeatureSupport::Unsupported {
            reason: error.to_string(),
        },
    }
}

fn ensure_feature_supported(feature: &'static str, support: FeatureSupport) -> Result<()> {
    match support {
        FeatureSupport::Supported => Ok(()),
        FeatureSupport::Unsupported { reason } => unsupported_feature(feature, reason),
    }
}

fn unsupported_support(reason: impl Into<String>) -> FeatureSupport {
    FeatureSupport::Unsupported {
        reason: reason.into(),
    }
}

fn unsupported_feature<T>(feature: &'static str, reason: impl Into<String>) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}

fn resolve<T: Copy>(module: &Module, symbol: &'static str) -> Option<T> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .and_then(|pointer| native_pointer_to_fn(pointer).ok())
}

fn resolve_pointer(module: &Module, symbol: &'static str) -> Option<*const c_void> {
    module
        .find_export_by_name(symbol)
        .or_else(|| module.find_symbol_by_name(symbol))
        .map(|pointer| pointer.0 as *const c_void)
}

fn resolve_pointer_any(module: &Module, symbols: &[&'static str]) -> Option<*const c_void> {
    symbols
        .iter()
        .find_map(|symbol| resolve_pointer(module, symbol))
}

fn resolve_any<T: Copy>(module: &Module, symbols: &[&'static str]) -> Option<T> {
    symbols.iter().find_map(|symbol| resolve(module, symbol))
}

fn resolve_pretty_method(module: &Module) -> Option<PrettyMethodFunction> {
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

fn resolve_suspend_all(module: &Module) -> Option<SuspendAll> {
    resolve(module, SUSPEND_ALL_WITH_CAUSE)
        .map(SuspendAll::WithCause)
        .or_else(|| resolve(module, SUSPEND_ALL_LEGACY).map(SuspendAll::Legacy))
}

fn resolve_visit_classes(module: &Module) -> Option<VisitClassesKind> {
    resolve(module, VISIT_CLASSES_VISITOR)
        .map(VisitClassesKind::Visitor)
        .or_else(|| resolve(module, VISIT_CLASSES_CALLBACK).map(VisitClassesKind::Callback))
}

fn find_interpreter_do_call_entries(module: &Module) -> Vec<usize> {
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

fn find_gc_synchronization_entries(module: &Module) -> Vec<GcSynchronizationEntry> {
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

fn push_gc_synchronization_entry(
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

fn is_interpreter_do_call_symbol(name: &str) -> bool {
    name.starts_with("_ZN3art11interpreter6DoCall")
        && name.contains("ArtMethod")
        && name.contains("ShadowFrame")
        && name.contains("JValue")
}

fn class_name_from_descriptor(descriptor: &str) -> String {
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
fn art_runtime_from_vm(vm: NonNull<jni::JavaVM>) -> *mut c_void {
    unsafe { vm.as_ptr().cast::<*mut c_void>().add(1).read() }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUICK_RESOLUTION_TEST_STUB: usize = 0x1000_0000;
    const QUICK_IMT_CONFLICT_TEST_STUB: usize = 0x1000_1000;
    const QUICK_GENERIC_JNI_TEST_STUB: usize = 0x1000_2000;
    const QUICK_TO_INTERPRETER_TEST_STUB: usize = 0x1000_3000;

    unsafe extern "C" fn dummy_add_global_ref(
        _vm: *mut jni::JavaVM,
        _thread: *mut c_void,
        _object: *mut c_void,
    ) -> jni::jobject {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn dummy_suspend_all(_thread_list: *mut c_void) {}

    unsafe extern "C" fn dummy_resume_all(_thread_list: *mut c_void) {}

    unsafe extern "C" fn dummy_visit_class_loaders(
        _class_linker: *mut c_void,
        _visitor: *mut ArtClassLoaderVisitor,
    ) {
    }

    unsafe extern "C" fn dummy_visit_classes(
        _class_linker: *mut c_void,
        _visitor: *mut ArtClassVisitor,
    ) {
    }

    unsafe extern "C" fn dummy_pretty_method(
        _result: *mut ArtStdString,
        _method: *mut c_void,
        _with_signature: bool,
    ) {
    }

    unsafe extern "C" fn dummy_decode_method_id(
        _jni_id_manager: *mut c_void,
        _method_id: jni::jmethodID,
    ) -> *mut c_void {
        0x1234usize as *mut c_void
    }

    unsafe extern "C" fn dummy_is_quick_resolution_stub(
        _class_linker: *mut c_void,
        entrypoint: *const c_void,
    ) -> bool {
        entrypoint as usize == QUICK_RESOLUTION_TEST_STUB
    }

    unsafe extern "C" fn dummy_is_quick_to_interpreter_bridge(
        _class_linker: *mut c_void,
        entrypoint: *const c_void,
    ) -> bool {
        entrypoint as usize == QUICK_TO_INTERPRETER_TEST_STUB
    }

    unsafe extern "C" fn dummy_is_quick_generic_jni_stub(
        _class_linker: *mut c_void,
        entrypoint: *const c_void,
    ) -> bool {
        entrypoint as usize == QUICK_GENERIC_JNI_TEST_STUB
    }

    fn dummy_entrypoint_predicates() -> ArtClassLinkerEntrypointPredicates {
        ArtClassLinkerEntrypointPredicates {
            is_quick_resolution_stub: dummy_is_quick_resolution_stub,
            is_quick_to_interpreter_bridge: dummy_is_quick_to_interpreter_bridge,
            is_quick_generic_jni_stub: dummy_is_quick_generic_jni_stub,
        }
    }

    #[test]
    fn derives_api_26_runtime_offsets() {
        let vm_offset = 512;
        assert_eq!(
            class_linker_offsets_for_api(26, vm_offset),
            vec![vm_offset - STD_STRING_SIZE - (2 * POINTER_SIZE)]
        );
    }

    #[test]
    fn derives_api_30_runtime_offset_candidates() {
        let vm_offset = 512;
        assert_eq!(
            class_linker_offsets_for_api(30, vm_offset),
            vec![
                vm_offset - (3 * POINTER_SIZE),
                vm_offset - (4 * POINTER_SIZE)
            ]
        );
    }

    #[test]
    fn detects_runtime_layout_from_supported_offsets() {
        let vm_offset = 512;
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];
        let vm_value = 0x1234usize;
        let thread_list = 0x2000usize as *mut c_void;
        let class_linker = 0x3000usize as *mut c_void;
        let intern_table = 0x3500usize as *mut c_void;
        let jni_id_manager = 0x4000usize as *mut c_void;

        runtime[vm_offset / POINTER_SIZE] = vm_value;
        runtime[(vm_offset - POINTER_SIZE) / POINTER_SIZE] = jni_id_manager as usize;
        runtime[(vm_offset - (5 * POINTER_SIZE)) / POINTER_SIZE] = thread_list as usize;
        runtime[(vm_offset - (4 * POINTER_SIZE)) / POINTER_SIZE] = intern_table as usize;
        runtime[(vm_offset - (3 * POINTER_SIZE)) / POINTER_SIZE] = class_linker as usize;

        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                runtime.as_mut_ptr().cast(),
                vm_value,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Ok(ArtRuntimeLayout {
                runtime: runtime.as_mut_ptr().cast(),
                thread_list,
                class_linker,
                intern_table,
                jni_id_manager,
                jni_ids_indirection: None,
            })
        );
    }

    #[test]
    fn replacement_runtime_layout_rejects_invalid_class_linker_candidate() {
        let vm_offset = 512;
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];
        let mut invalid_class_linker = vec![0u8; 320];
        let mut valid_class_linker = vec![0u8; 320];
        let mut code = vec![0u8; 96];
        let vm_value = 0x1234usize;
        let thread_list = 0x2000usize as *mut c_void;
        let intern_table = 0x3500usize as *mut c_void;
        let quick_resolution = code.as_mut_ptr() as usize;
        let quick_imt_conflict = unsafe { code.as_mut_ptr().add(16) as usize };
        let quick_generic_jni = unsafe { code.as_mut_ptr().add(32) as usize };
        let quick_to_interpreter = unsafe { code.as_mut_ptr().add(48) as usize };
        let anchor_offset = 200;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);

        runtime[vm_offset / POINTER_SIZE] = vm_value;
        runtime[(vm_offset - (6 * POINTER_SIZE)) / POINTER_SIZE] = thread_list as usize;
        runtime[(vm_offset - (5 * POINTER_SIZE)) / POINTER_SIZE] = intern_table as usize;
        runtime[(vm_offset - (4 * POINTER_SIZE)) / POINTER_SIZE] =
            valid_class_linker.as_mut_ptr() as usize;
        runtime[(vm_offset - (3 * POINTER_SIZE)) / POINTER_SIZE] =
            invalid_class_linker.as_mut_ptr() as usize;

        valid_class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        valid_class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        valid_class_linker[quick_generic_offset - POINTER_SIZE..quick_generic_offset]
            .copy_from_slice(&quick_imt_conflict.to_ne_bytes());
        valid_class_linker[quick_generic_offset..quick_generic_offset + POINTER_SIZE]
            .copy_from_slice(&quick_generic_jni.to_ne_bytes());
        valid_class_linker
            [quick_generic_offset + POINTER_SIZE..quick_generic_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_to_interpreter.to_ne_bytes());

        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: invalid_class_linker.as_ptr() as usize,
                    end: invalid_class_linker.as_ptr() as usize + invalid_class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: valid_class_linker.as_ptr() as usize,
                    end: valid_class_linker.as_ptr() as usize + valid_class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        let (layout, trampolines) = detect_runtime_layout_and_trampolines_from_runtime(
            30,
            runtime.as_mut_ptr().cast(),
            vm_value,
            None,
            None,
            &memory,
            FEATURE_METHOD_REPLACEMENT,
        )
        .unwrap();

        assert_eq!(layout.class_linker, valid_class_linker.as_mut_ptr().cast());
        assert_eq!(
            trampolines.quick_generic_jni_trampoline,
            quick_generic_jni as *mut c_void
        );
    }

    #[test]
    fn direct_jni_method_ids_are_not_decoded() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.decode_method_id = Some(dummy_decode_method_id);
        let layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: std::ptr::dangling_mut(),
            intern_table: std::ptr::dangling_mut(),
            jni_id_manager: 0x7777usize as *mut c_void,
            jni_ids_indirection: Some(K_POINTER_JNI_ID_TYPE),
        };
        let method_id = 0x5555usize as jni::jmethodID;

        assert_eq!(
            backend.art_method_from_jni_id(&layout, method_id),
            vec![method_id.cast::<c_void>()]
        );
    }

    #[test]
    fn unknown_jni_method_id_mode_tries_raw_and_decoded_candidates() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.decode_method_id = Some(dummy_decode_method_id);
        let layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: std::ptr::dangling_mut(),
            intern_table: std::ptr::dangling_mut(),
            jni_id_manager: 0x7777usize as *mut c_void,
            jni_ids_indirection: None,
        };
        let method_id = 0x5555usize as jni::jmethodID;

        assert_eq!(
            backend.art_method_from_jni_id(&layout, method_id),
            vec![method_id.cast::<c_void>(), 0x1234usize as *mut c_void]
        );
    }

    #[test]
    fn indirect_jni_method_ids_are_decoded() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.decode_method_id = Some(dummy_decode_method_id);
        let layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: std::ptr::dangling_mut(),
            intern_table: std::ptr::dangling_mut(),
            jni_id_manager: 0x7777usize as *mut c_void,
            jni_ids_indirection: Some(1),
        };

        assert_eq!(
            backend.art_method_from_jni_id(&layout, 0x5555usize as jni::jmethodID),
            vec![0x1234usize as *mut c_void]
        );
    }

    #[test]
    fn detects_thread_class_method_array_layout_from_known_method() {
        let method_size = 24;
        let method_count = 3usize;
        let mut class_object = vec![0u8; CLASS_LAYOUT_SCAN_LIMIT];
        let mut methods = vec![0u8; POINTER_SIZE + (method_count * method_size)];
        let methods_offset = 32;
        let copied_methods_offset = 44;
        let methods_header = methods.as_mut_ptr() as usize;
        let known_method = unsafe {
            methods
                .as_mut_ptr()
                .byte_add(POINTER_SIZE + method_size)
                .cast::<c_void>()
        };
        class_object[methods_offset..methods_offset + POINTER_SIZE]
            .copy_from_slice(&methods_header.to_ne_bytes());
        methods[..POINTER_SIZE].copy_from_slice(&method_count.to_ne_bytes());
        class_object[copied_methods_offset..copied_methods_offset + 2]
            .copy_from_slice(&(method_count as u16).to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_object.as_ptr() as usize,
                    end: class_object.as_ptr() as usize + class_object.len(),
                    executable: false,
                },
                MemoryRange {
                    start: methods.as_ptr() as usize,
                    end: methods.as_ptr() as usize + methods.len(),
                    executable: false,
                },
            ],
        };

        let layout = detect_thread_class_method_layout(
            class_object.as_mut_ptr().cast(),
            &[vec![known_method]],
            method_size,
            &memory,
        )
        .unwrap();

        assert_eq!(layout.class_methods_offset, methods_offset);
        assert_eq!(layout.class_copied_methods_offset, copied_methods_offset);
        assert_eq!(layout.method_size, method_size);
    }

    #[test]
    fn detects_art_method_runtime_layout_from_access_flags_and_entrypoints() {
        let mut method = vec![0u8; 80];
        let mut code = vec![0u8; 64];
        let jni_code = code.as_mut_ptr() as usize;
        let quick_code = unsafe { code.as_mut_ptr().add(16) as usize };
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_NATIVE;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[access_flags_offset..access_flags_offset + 4]
            .copy_from_slice(&access_flags.to_ne_bytes());
        method[jni_code_offset..jni_code_offset + POINTER_SIZE]
            .copy_from_slice(&jni_code.to_ne_bytes());
        method[quick_code_offset..quick_code_offset + POINTER_SIZE]
            .copy_from_slice(&quick_code.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_runtime_layout(
                &[method.as_mut_ptr().cast()],
                &memory,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn detects_art_method_replacement_layout_from_runtime_native_entrypoint() {
        let mut method = vec![0u8; 80];
        let mut runtime_code = vec![0u8; 64];
        let native_entrypoint = unsafe { runtime_code.as_mut_ptr().add(16) as usize };
        let access_flags = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE | 0x8000_0000;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[access_flags_offset..access_flags_offset + 4]
            .copy_from_slice(&access_flags.to_ne_bytes());
        method[jni_code_offset..jni_code_offset + POINTER_SIZE]
            .copy_from_slice(&native_entrypoint.to_ne_bytes());
        method[quick_code_offset..quick_code_offset + POINTER_SIZE]
            .copy_from_slice(&(0x5555usize).to_ne_bytes());
        let runtime_range = ArtModuleRange {
            start: runtime_code.as_ptr() as usize,
            end: runtime_code.as_ptr() as usize + runtime_code.len(),
        };
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: runtime_code.as_ptr() as usize,
                    end: runtime_code.as_ptr() as usize + runtime_code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                runtime_range,
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn snapshots_patches_and_restores_art_method_fields() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 40,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: Some(32),
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC
                | K_ACC_STATIC
                | K_ACC_FINAL
                | K_ACC_FAST_NATIVE
                | K_ACC_CRITICAL_NATIVE
                | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
                | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
                | K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
                | K_ACC_SINGLE_IMPLEMENTATION
                | K_ACC_SKIP_ACCESS_CHECKS,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: Some(0x3333usize as *mut c_void),
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );

        let patched = patched_replacement_method(
            original,
            0x4444usize as *mut c_void,
            0x5555usize as *mut c_void,
            compile_dont_bother_flag(30),
        );
        patch_art_method(method.as_mut_ptr().cast(), &layout, patched);
        let patched_snapshot =
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory).unwrap();
        assert_eq!(patched_snapshot.jni_code, 0x4444usize as *mut c_void);
        assert_eq!(patched_snapshot.quick_code, 0x5555usize as *mut c_void);
        assert_eq!(
            patched_snapshot.interpreter_code,
            Some(0x3333usize as *mut c_void)
        );
        assert_ne!(patched_snapshot.access_flags & K_ACC_NATIVE, 0);
        assert_ne!(
            patched_snapshot.access_flags & compile_dont_bother_flag(30),
            0
        );
        assert_eq!(patched_snapshot.access_flags & K_ACC_FAST_NATIVE, 0);
        assert_eq!(patched_snapshot.access_flags & K_ACC_CRITICAL_NATIVE, 0);
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG,
            0
        );
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_NTERP_INVOKE_FAST_PATH_FLAG,
            0
        );
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE,
            0
        );
        assert_eq!(
            patched_snapshot.access_flags & K_ACC_SINGLE_IMPLEMENTATION,
            0
        );
        assert_eq!(patched_snapshot.access_flags & K_ACC_SKIP_ACCESS_CHECKS, 0);

        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn verified_patch_restores_original_on_mismatch() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        let mismatched = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE,
            jni_code: 0x3333usize as *mut c_void,
            quick_code: 0x4444usize as *mut c_void,
            interpreter_code: Some(0x5555usize as *mut c_void),
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        let error = patch_art_method_verified(
            method.as_mut_ptr().cast(),
            &layout,
            original,
            mismatched,
            &memory,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                ..
            }
        ));
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn verified_restore_checks_restored_snapshot() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        let patched = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE,
            jni_code: 0x3333usize as *mut c_void,
            quick_code: 0x4444usize as *mut c_void,
            interpreter_code: None,
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, patched);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        restore_art_method_verified(method.as_mut_ptr().cast(), &layout, original, &memory)
            .unwrap();
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn cloned_art_method_copies_original_bytes() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        let cloned = ArtMethodClone::copy_from(method.as_mut_ptr().cast(), &layout, &memory)
            .expect("ArtMethod clone allocation failed");
        let clone_memory = cloned.memory_ranges();
        assert_eq!(
            snapshot_art_method(cloned.as_ptr(), &layout, &clone_memory),
            Ok(original)
        );
        let original_bytes = &method[..layout.method_size];
        let cloned_bytes =
            unsafe { std::slice::from_raw_parts(cloned.as_ptr().cast::<u8>(), layout.method_size) };
        assert_eq!(cloned_bytes, original_bytes);
    }

    #[test]
    fn cloned_replacement_method_patches_clone_without_touching_original() {
        let mut method = vec![0u8; 80];
        let layout = ArtMethodRuntimeLayout {
            method_size: 40,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: Some(32),
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: Some(0x3333usize as *mut c_void),
        };
        let patched = patched_replacement_method(
            original,
            0x4444usize as *mut c_void,
            0x5555usize as *mut c_void,
            compile_dont_bother_flag(30),
        );
        patch_art_method(method.as_mut_ptr().cast(), &layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        let cloned = clone_replacement_art_method(
            method.as_mut_ptr().cast(),
            &layout,
            original,
            patched,
            &memory,
        )
        .expect("replacement ArtMethod clone failed");
        let clone_memory = cloned.memory_ranges();
        assert_eq!(
            snapshot_art_method(cloned.as_ptr(), &layout, &clone_memory),
            Ok(patched)
        );
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
        drop(cloned);
        assert_eq!(
            snapshot_art_method(method.as_mut_ptr().cast(), &layout, &memory),
            Ok(original)
        );
    }

    #[test]
    fn original_clone_dispatch_patch_preserves_jni_entrypoint() {
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC
                | K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE
                | K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG
                | K_ACC_NTERP_INVOKE_FAST_PATH_FLAG
                | K_ACC_SINGLE_IMPLEMENTATION
                | K_ACC_SKIP_ACCESS_CHECKS,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: Some(0x3333usize as *mut c_void),
        };

        let patched = patched_original_method_for_clone_dispatch(
            original,
            QUICK_TO_INTERPRETER_TEST_STUB as *mut c_void,
            compile_dont_bother_flag(30),
        );

        assert_eq!(patched.jni_code, original.jni_code);
        assert_eq!(
            patched.quick_code,
            QUICK_TO_INTERPRETER_TEST_STUB as *mut c_void
        );
        assert_eq!(patched.interpreter_code, original.interpreter_code);
        assert_eq!(patched.access_flags & K_ACC_PUBLIC, K_ACC_PUBLIC);
        assert_eq!(
            patched.access_flags & K_ACC_FAST_INTERPRETER_TO_INTERPRETER_INVOKE,
            0
        );
        assert_eq!(
            patched.access_flags & K_ACC_NTERP_ENTRY_POINT_FAST_PATH_FLAG,
            0
        );
        assert_eq!(patched.access_flags & K_ACC_SINGLE_IMPLEMENTATION, 0);
        assert_eq!(patched.access_flags & K_ACC_SKIP_ACCESS_CHECKS, 0);
        assert_ne!(patched.access_flags & compile_dont_bother_flag(30), 0);
    }

    #[test]
    fn detects_art_thread_managed_stack_offset_from_jni_env_field() {
        let mut thread = [0usize; 40];
        let env = 0x1234usize as *mut c_void;
        let jni_env_offset = 176;
        thread[jni_env_offset / POINTER_SIZE] = env as usize;

        let managed_stack_offset =
            detect_art_thread_managed_stack_offset("test feature", thread.as_mut_ptr().cast(), env)
                .expect("managed stack offset was not detected");

        assert_eq!(managed_stack_offset, jni_env_offset - (4 * POINTER_SIZE));
    }

    #[test]
    fn replacement_frame_detection_requires_linked_replacement_quick_frame() {
        let replacement = 0x1234_5678usize;
        let mut linked_quick_frame = [replacement];
        let mut linked_stack = [0usize; 3];
        linked_stack[0] = linked_quick_frame.as_mut_ptr() as usize | 1;

        let managed_stack_offset = 4 * POINTER_SIZE;
        let mut thread = [0usize; 16];
        thread[(managed_stack_offset + POINTER_SIZE) / POINTER_SIZE] =
            linked_stack.as_mut_ptr() as usize;

        assert!(replacement_frame_is_active(
            replacement,
            thread.as_ptr() as usize,
            managed_stack_offset,
        ));

        let mut current_quick_frame = [0xabcdusize];
        thread[managed_stack_offset / POINTER_SIZE] = current_quick_frame.as_mut_ptr() as usize;
        assert!(!replacement_frame_is_active(
            replacement,
            thread.as_ptr() as usize,
            managed_stack_offset,
        ));

        current_quick_frame[0] = replacement;
        assert_eq!(current_quick_frame[0], replacement);
        assert!(!replacement_frame_is_active(
            replacement,
            thread.as_ptr() as usize,
            managed_stack_offset,
        ));
    }

    #[test]
    fn replacement_controller_translates_registered_methods() {
        let controller = ArtReplacementController::empty_for_tests();
        let original = 0x1000usize as *mut c_void;
        let replacement = 0x2000usize as *mut c_void;

        assert_eq!(
            controller.translate_method_argument(original as usize),
            original as usize
        );
        controller.register(
            original,
            replacement,
            ArtReplacementSynchronization {
                quick_code_offset: POINTER_SIZE,
                thread_managed_stack_offset: 0,
                nterp_entrypoint: None,
                quick_to_interpreter_bridge: 0,
            },
        );
        assert_eq!(
            controller.translate_method_argument(original as usize),
            replacement as usize
        );
        assert!(controller.is_replacement_method(replacement));
        assert!(!controller.is_replacement_method(original));
        controller.unregister(original);
        assert_eq!(
            controller.translate_method_argument(original as usize),
            original as usize
        );
        assert!(!controller.is_replacement_method(replacement));
    }

    #[test]
    fn replacement_controller_synchronizes_clone_declaring_class() {
        let controller = ArtReplacementController::empty_for_tests();
        let mut original = vec![0u8; 12];
        let mut replacement = vec![0u8; 12];
        let original_flags = K_ACC_PUBLIC | K_ACC_STATIC | compile_dont_bother_flag(30);
        write_u32(original.as_mut_ptr().cast(), 0xaaaa_bbbb);
        write_u32(
            unsafe { original.as_mut_ptr().byte_add(4).cast() },
            original_flags,
        );
        write_u32(replacement.as_mut_ptr().cast(), 0xcccc_dddd);
        write_u32(
            unsafe { replacement.as_mut_ptr().byte_add(4).cast() },
            K_ACC_NATIVE,
        );

        controller.register(
            original.as_mut_ptr().cast(),
            replacement.as_mut_ptr().cast(),
            ArtReplacementSynchronization {
                quick_code_offset: 8,
                thread_managed_stack_offset: 0,
                nterp_entrypoint: None,
                quick_to_interpreter_bridge: 0,
            },
        );
        controller.synchronize_replacement_methods();

        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: replacement.as_ptr() as usize,
                end: replacement.as_ptr() as usize + replacement.len(),
                executable: false,
            }],
        };
        assert_eq!(
            read_u32(unsafe { replacement.as_ptr().byte_add(4).cast() }, &memory),
            Some(K_ACC_NATIVE)
        );
        assert_eq!(
            read_u32(replacement.as_ptr().cast(), &memory),
            Some(0xaaaa_bbbb)
        );
    }

    #[test]
    fn replacement_controller_rewrites_original_nterp_quick_code() {
        let controller = ArtReplacementController::empty_for_tests();
        let mut original = vec![0u8; 24];
        let mut replacement = vec![0u8; 24];
        let nterp = 0x1000usize;
        let quick_to_interpreter = 0x2000usize;
        write_usize(unsafe { original.as_mut_ptr().byte_add(16).cast() }, nterp);

        controller.register(
            original.as_mut_ptr().cast(),
            replacement.as_mut_ptr().cast(),
            ArtReplacementSynchronization {
                quick_code_offset: 16,
                thread_managed_stack_offset: 0,
                nterp_entrypoint: Some(nterp),
                quick_to_interpreter_bridge: quick_to_interpreter,
            },
        );
        controller.synchronize_replacement_methods();

        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: original.as_ptr() as usize,
                end: original.as_ptr() as usize + original.len(),
                executable: false,
            }],
        };
        assert_eq!(
            read_usize(unsafe { original.as_ptr().byte_add(16).cast() }, &memory),
            Some(quick_to_interpreter)
        );
    }

    #[test]
    fn replacement_guard_debug_summary_includes_cloned_method() {
        let mut method = vec![0u8; 80];
        let method_layout = ArtMethodRuntimeLayout {
            method_size: 32,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let original = ArtMethodSnapshot {
            access_flags: K_ACC_PUBLIC | K_ACC_STATIC,
            jni_code: 0x1111usize as *mut c_void,
            quick_code: 0x2222usize as *mut c_void,
            interpreter_code: None,
        };
        let patched = patched_replacement_method(
            original,
            0x3333usize as *mut c_void,
            QUICK_GENERIC_JNI_TEST_STUB as *mut c_void,
            compile_dont_bother_flag(30),
        );
        patch_art_method(method.as_mut_ptr().cast(), &method_layout, original);
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };
        let cloned_method = clone_replacement_art_method(
            method.as_mut_ptr().cast(),
            &method_layout,
            original,
            patched,
            &memory,
        )
        .expect("replacement ArtMethod clone failed");
        let cloned_pointer = format!("{:?}", cloned_method.as_ptr());
        let dispatch_thunk = ArtMethodDispatchThunk::new(
            cloned_method.as_ptr(),
            original.quick_code,
            method_layout.quick_code_offset,
            160,
        )
        .expect("replacement dispatch thunk allocation failed");
        let original_patched = patched_original_method_for_clone_dispatch(
            original,
            dispatch_thunk.as_ptr(),
            compile_dont_bother_flag(30),
        );
        let guard = ArtMethodReplacementGuard {
            backend: ArtBackend::empty_for_tests(),
            vm: Vm::dangling_for_tests(),
            method: method.as_mut_ptr().cast(),
            cloned_method,
            dispatch_thunk,
            layout: ArtMethodReplacementLayout {
                api_level: 30,
                runtime: ArtRuntimeLayout {
                    runtime: 0x1000usize as *mut c_void,
                    thread_list: 0x2000usize as *mut c_void,
                    class_linker: 0x3000usize as *mut c_void,
                    intern_table: 0x4000usize as *mut c_void,
                    jni_id_manager: ptr::null_mut(),
                    jni_ids_indirection: None,
                },
                method: method_layout,
                trampolines: ArtClassLinkerTrampolines {
                    quick_resolution_trampoline: QUICK_RESOLUTION_TEST_STUB as *mut c_void,
                    quick_imt_conflict_trampoline: QUICK_IMT_CONFLICT_TEST_STUB as *mut c_void,
                    quick_generic_jni_trampoline: QUICK_GENERIC_JNI_TEST_STUB as *mut c_void,
                    quick_to_interpreter_bridge_trampoline: QUICK_TO_INTERPRETER_TEST_STUB
                        as *mut c_void,
                },
                thread_managed_stack_offset: 160,
            },
            original,
            original_patched,
            clone_patched: patched,
            reverted: true,
        };

        let summary = guard.debug_summary();
        assert!(summary.contains("backend=clone-active"));
        assert!(!summary.contains("clone-prepared-direct-active"));
        assert!(summary.contains("cloned_method="));
        assert!(summary.contains(&cloned_pointer));
        assert!(summary.contains("dispatch_thunk="));
        assert!(summary.contains("original_patched={access_flags="));
        assert!(summary.contains("clone_patched={access_flags="));
        assert!(summary.contains("quick_to_interpreter_bridge_trampoline="));
        assert!(summary.contains("thread_managed_stack_offset=160"));
        assert!(summary.contains("do_call_hooks=0"));
        assert!(summary.contains("quick_entrypoint_hooks=0"));
        assert!(summary.contains("get_oat_quick_method_header_hook=false"));
        assert!(summary.contains("gc_synchronization_hooks=0"));
    }

    #[test]
    fn rejects_non_executable_replacement_function() {
        let mut code = vec![0u8; 8];
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: code.as_ptr() as usize,
                end: code.as_ptr() as usize + code.len(),
                executable: false,
            }],
        };

        assert_eq!(
            validate_replacement_function(code.as_mut_ptr().cast(), &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "replacement function is not executable".to_owned(),
            })
        );
    }

    #[test]
    fn accepts_executable_replacement_function() {
        let mut code = vec![0u8; 8];
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: code.as_ptr() as usize,
                end: code.as_ptr() as usize + code.len(),
                executable: true,
            }],
        };

        assert_eq!(
            validate_replacement_function(code.as_mut_ptr().cast(), &memory),
            Ok(())
        );
    }

    #[test]
    fn rejects_missing_replacement_trampoline() {
        let trampolines = ArtClassLinkerTrampolines {
            quick_resolution_trampoline: 0x1000usize as *mut c_void,
            quick_imt_conflict_trampoline: 0x2000usize as *mut c_void,
            quick_generic_jni_trampoline: std::ptr::null_mut(),
            quick_to_interpreter_bridge_trampoline: 0x3000usize as *mut c_void,
        };
        let memory = MemoryRanges { ranges: Vec::new() };

        assert_eq!(
            validate_replacement_trampoline(&trampolines, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ClassLinker quick generic JNI trampoline is unavailable or not executable"
                    .to_owned(),
            })
        );
    }

    #[test]
    fn rejects_null_replacement_function_before_runtime_work() {
        let backend = ArtBackend::empty_for_tests();
        let error = match backend.replace_method(
            &Vm::dangling_for_tests(),
            MethodKind::Static,
            0x1234usize as jni::jmethodID,
            std::ptr::null_mut(),
        ) {
            Err(error) => error,
            Ok(_) => panic!("null replacement function unexpectedly succeeded"),
        };
        assert_eq!(
            error,
            Error::NullReturn {
                operation: "ART replacement function"
            }
        );
    }

    #[test]
    fn snapshot_rejects_unreadable_art_method() {
        let layout = ArtMethodRuntimeLayout {
            method_size: 40,
            access_flags_offset: 4,
            jni_code_offset: 16,
            quick_code_offset: 24,
            interpreter_code_offset: None,
        };
        let memory = MemoryRanges { ranges: Vec::new() };

        assert_eq!(
            snapshot_art_method(0x1234usize as *mut c_void, &layout, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "target ArtMethod is not readable".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_replacement_layout_without_runtime_native_entrypoint() {
        let mut method = vec![0u8; 80];
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        method[24..24 + POINTER_SIZE].copy_from_slice(&(0x7777usize).to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                ArtModuleRange {
                    start: 0x1000,
                    end: 0x2000,
                },
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "unable to determine ArtMethod runtime layout: native entrypoint is not executable".to_owned(),
            })
        );
    }

    #[test]
    fn detects_replacement_layout_with_non_final_native_access_flags() {
        let mut method = vec![0u8; 80];
        let mut runtime_code = vec![0u8; 64];
        let native_entrypoint = unsafe { runtime_code.as_mut_ptr().add(16) as usize };
        let access_flags = K_ACC_PUBLIC | K_ACC_STATIC | K_ACC_NATIVE | 0x0008_0000;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        method[24..24 + POINTER_SIZE].copy_from_slice(&native_entrypoint.to_ne_bytes());
        let runtime_range = ArtModuleRange {
            start: runtime_code.as_ptr() as usize,
            end: runtime_code.as_ptr() as usize + runtime_code.len(),
        };
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: runtime_code.as_ptr() as usize,
                    end: runtime_code.as_ptr() as usize + runtime_code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                runtime_range,
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn rejects_replacement_layout_without_public_native_access_flags() {
        let mut method = vec![0u8; 80];
        let mut runtime_code = vec![0u8; 64];
        let native_entrypoint = unsafe { runtime_code.as_mut_ptr().add(16) as usize };
        let access_flags = K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        method[24..24 + POINTER_SIZE].copy_from_slice(&native_entrypoint.to_ne_bytes());
        let runtime_range = ArtModuleRange {
            start: runtime_code.as_ptr() as usize,
            end: runtime_code.as_ptr() as usize + runtime_code.len(),
        };
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: runtime_code.as_ptr() as usize,
                    end: runtime_code.as_ptr() as usize + runtime_code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                runtime_range,
                30,
                &memory,
                false,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "unable to determine ArtMethod runtime layout: native access flags were not found".to_owned(),
            })
        );
    }

    #[test]
    fn detects_replacement_layout_from_executable_entrypoint_fallback() {
        let mut method = vec![0u8; 80];
        let mut code = vec![0u8; 64];
        let native_entrypoint = unsafe { code.as_mut_ptr().add(16) as usize };
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_FINAL | K_ACC_NATIVE;
        let access_flags_offset = 4;
        let jni_code_offset = 24;
        let quick_code_offset = jni_code_offset + POINTER_SIZE;
        method[access_flags_offset..access_flags_offset + 4]
            .copy_from_slice(&access_flags.to_ne_bytes());
        method[jni_code_offset..jni_code_offset + POINTER_SIZE]
            .copy_from_slice(&native_entrypoint.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_replacement_layout(
                &[method.as_mut_ptr().cast()],
                ArtModuleRange {
                    start: 0x1000,
                    end: 0x2000,
                },
                30,
                &memory,
                true,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Ok(ArtMethodRuntimeLayout {
                method_size: quick_code_offset + POINTER_SIZE,
                access_flags_offset,
                jni_code_offset,
                quick_code_offset,
                interpreter_code_offset: None,
            })
        );
    }

    #[test]
    fn replacement_prerequisites_do_not_require_exception_clear_symbol() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.pretty_method = Some(PrettyMethodFunction {
            function: dummy_pretty_method,
            _thunk: None,
        });
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);

        assert_eq!(
            backend.method_replacement_support(&Vm::dangling_for_tests()),
            FeatureSupport::Unsupported {
                reason: "libandroid_runtime.so is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_art_method_layout_without_executable_entrypoints() {
        let mut method = vec![0u8; 80];
        let access_flags = 0x0001u32 | K_ACC_STATIC | K_ACC_NATIVE;
        method[4..8].copy_from_slice(&access_flags.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: method.as_ptr() as usize,
                end: method.as_ptr() as usize + method.len(),
                executable: false,
            }],
        };

        assert_eq!(
            detect_art_method_runtime_layout(
                &[method.as_mut_ptr().cast()],
                &memory,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "unable to determine ArtMethod runtime layout".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_art_method_layout_without_native_access_flags() {
        let mut method = vec![0u8; 80];
        let mut code = vec![0u8; 64];
        let jni_code = code.as_mut_ptr() as usize;
        let quick_code = unsafe { code.as_mut_ptr().add(16) as usize };
        method[24..24 + POINTER_SIZE].copy_from_slice(&jni_code.to_ne_bytes());
        method[24 + POINTER_SIZE..24 + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_code.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: method.as_ptr() as usize,
                    end: method.as_ptr() as usize + method.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };

        assert_eq!(
            detect_art_method_runtime_layout(
                &[method.as_mut_ptr().cast()],
                &memory,
                FEATURE_METHOD_REPLACEMENT,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "unable to determine ArtMethod runtime layout".to_owned(),
            })
        );
    }

    #[test]
    fn detects_class_linker_trampolines_from_intern_table_anchor() {
        let mut class_linker = vec![0u8; 320];
        let mut code = vec![0u8; 96];
        let intern_table = 0x4444usize as *mut c_void;
        let anchor_offset = 200;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);
        let quick_resolution = code.as_mut_ptr() as usize;
        let quick_imt_conflict = unsafe { code.as_mut_ptr().add(16) as usize };
        let quick_generic_jni = unsafe { code.as_mut_ptr().add(32) as usize };
        let quick_to_interpreter = unsafe { code.as_mut_ptr().add(48) as usize };
        class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        class_linker[quick_generic_offset - POINTER_SIZE..quick_generic_offset]
            .copy_from_slice(&quick_imt_conflict.to_ne_bytes());
        class_linker[quick_generic_offset..quick_generic_offset + POINTER_SIZE]
            .copy_from_slice(&quick_generic_jni.to_ne_bytes());
        class_linker
            [quick_generic_offset + POINTER_SIZE..quick_generic_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_to_interpreter.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: code.as_ptr() as usize,
                    end: code.as_ptr() as usize + code.len(),
                    executable: true,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Ok(ArtClassLinkerTrampolines {
                quick_resolution_trampoline: quick_resolution as *mut c_void,
                quick_imt_conflict_trampoline: quick_imt_conflict as *mut c_void,
                quick_generic_jni_trampoline: quick_generic_jni as *mut c_void,
                quick_to_interpreter_bridge_trampoline: quick_to_interpreter as *mut c_void,
            })
        );
    }

    #[test]
    fn reports_missing_class_linker_intern_table_anchor() {
        let mut class_linker = vec![0u8; 1000];
        let memory = MemoryRanges {
            ranges: vec![MemoryRange {
                start: class_linker.as_ptr() as usize,
                end: class_linker.as_ptr() as usize + class_linker.len(),
                executable: false,
            }],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table: 0x4444usize as *mut c_void,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason:
                    "unable to determine ClassLinker trampoline offsets: intern table anchor was not found and ClassLinker quick-entrypoint predicate symbols are unavailable"
                        .to_owned(),
            })
        );
    }

    #[test]
    fn detects_class_linker_trampolines_from_predicate_scan() {
        let mut class_linker = vec![0u8; 5000];
        let intern_table = 0x4444usize as *mut c_void;
        let quick_resolution_offset = 424;
        class_linker[quick_resolution_offset..quick_resolution_offset + POINTER_SIZE]
            .copy_from_slice(&QUICK_RESOLUTION_TEST_STUB.to_ne_bytes());
        class_linker
            [quick_resolution_offset + POINTER_SIZE..quick_resolution_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_IMT_CONFLICT_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (2 * POINTER_SIZE)
            ..quick_resolution_offset + (3 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_GENERIC_JNI_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (3 * POINTER_SIZE)
            ..quick_resolution_offset + (4 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_TO_INTERPRETER_TEST_STUB.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: true,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(
                &runtime_layout,
                36,
                Some(dummy_entrypoint_predicates()),
                &memory
            ),
            Ok(ArtClassLinkerTrampolines {
                quick_resolution_trampoline: QUICK_RESOLUTION_TEST_STUB as *mut c_void,
                quick_imt_conflict_trampoline: QUICK_IMT_CONFLICT_TEST_STUB as *mut c_void,
                quick_generic_jni_trampoline: QUICK_GENERIC_JNI_TEST_STUB as *mut c_void,
                quick_to_interpreter_bridge_trampoline: QUICK_TO_INTERPRETER_TEST_STUB
                    as *mut c_void,
            })
        );
    }

    #[cfg(target_arch = "aarch64")]
    #[test]
    fn detects_class_linker_trampolines_with_tagged_class_linker_pointer() {
        let mut class_linker = vec![0u8; 5000];
        let intern_table = 0x4444usize as *mut c_void;
        let anchor_offset = 424;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);
        let quick_resolution = QUICK_RESOLUTION_TEST_STUB;
        let quick_imt_conflict = QUICK_IMT_CONFLICT_TEST_STUB;
        let quick_generic_jni = QUICK_GENERIC_JNI_TEST_STUB;
        let quick_to_interpreter = QUICK_TO_INTERPRETER_TEST_STUB;
        class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        class_linker[quick_generic_offset - POINTER_SIZE..quick_generic_offset]
            .copy_from_slice(&quick_imt_conflict.to_ne_bytes());
        class_linker[quick_generic_offset..quick_generic_offset + POINTER_SIZE]
            .copy_from_slice(&quick_generic_jni.to_ne_bytes());
        class_linker
            [quick_generic_offset + POINTER_SIZE..quick_generic_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&quick_to_interpreter.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: true,
                },
            ],
        };
        let tagged_class_linker =
            ((class_linker.as_mut_ptr() as usize) | 0xab00_0000_0000_0000) as *mut c_void;
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: tagged_class_linker,
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Ok(ArtClassLinkerTrampolines {
                quick_resolution_trampoline: quick_resolution as *mut c_void,
                quick_imt_conflict_trampoline: quick_imt_conflict as *mut c_void,
                quick_generic_jni_trampoline: quick_generic_jni as *mut c_void,
                quick_to_interpreter_bridge_trampoline: quick_to_interpreter as *mut c_void,
            })
        );
    }

    #[test]
    fn reports_non_executable_predicate_trampoline() {
        let mut class_linker = vec![0u8; 5000];
        let quick_resolution_offset = 424;
        class_linker[quick_resolution_offset..quick_resolution_offset + POINTER_SIZE]
            .copy_from_slice(&QUICK_RESOLUTION_TEST_STUB.to_ne_bytes());
        class_linker
            [quick_resolution_offset + POINTER_SIZE..quick_resolution_offset + (2 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_IMT_CONFLICT_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (2 * POINTER_SIZE)
            ..quick_resolution_offset + (3 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_GENERIC_JNI_TEST_STUB.to_ne_bytes());
        class_linker[quick_resolution_offset + (3 * POINTER_SIZE)
            ..quick_resolution_offset + (4 * POINTER_SIZE)]
            .copy_from_slice(&QUICK_TO_INTERPRETER_TEST_STUB.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: false,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table: 0x4444usize as *mut c_void,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(
                &runtime_layout,
                36,
                Some(dummy_entrypoint_predicates()),
                &memory
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ClassLinker quick resolution trampoline at offset 0x1a8 is not executable"
                    .to_owned(),
            })
        );
    }

    #[test]
    fn reports_ambiguous_predicate_trampoline_candidates() {
        let mut class_linker = vec![0u8; 5000];
        for quick_resolution_offset in [424, 520] {
            class_linker[quick_resolution_offset..quick_resolution_offset + POINTER_SIZE]
                .copy_from_slice(&QUICK_RESOLUTION_TEST_STUB.to_ne_bytes());
            class_linker[quick_resolution_offset + POINTER_SIZE
                ..quick_resolution_offset + (2 * POINTER_SIZE)]
                .copy_from_slice(&QUICK_IMT_CONFLICT_TEST_STUB.to_ne_bytes());
            class_linker[quick_resolution_offset + (2 * POINTER_SIZE)
                ..quick_resolution_offset + (3 * POINTER_SIZE)]
                .copy_from_slice(&QUICK_GENERIC_JNI_TEST_STUB.to_ne_bytes());
            class_linker[quick_resolution_offset + (3 * POINTER_SIZE)
                ..quick_resolution_offset + (4 * POINTER_SIZE)]
                .copy_from_slice(&QUICK_TO_INTERPRETER_TEST_STUB.to_ne_bytes());
        }
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: QUICK_RESOLUTION_TEST_STUB,
                    end: QUICK_TO_INTERPRETER_TEST_STUB + 0x1000,
                    executable: true,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table: 0x4444usize as *mut c_void,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(
                &runtime_layout,
                36,
                Some(dummy_entrypoint_predicates()),
                &memory
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason:
                    "unable to determine ClassLinker trampoline offsets: predicate scan found multiple candidates"
                        .to_owned(),
            })
        );
    }

    #[test]
    fn reports_non_executable_class_linker_trampoline() {
        let mut class_linker = vec![0u8; 320];
        let data = vec![0u8; 96];
        let intern_table = 0x4444usize as *mut c_void;
        let anchor_offset = 200;
        let quick_generic_offset = anchor_offset + (6 * POINTER_SIZE);
        let quick_resolution = data.as_ptr() as usize;
        class_linker[anchor_offset..anchor_offset + POINTER_SIZE]
            .copy_from_slice(&(intern_table as usize).to_ne_bytes());
        class_linker
            [quick_generic_offset - (2 * POINTER_SIZE)..quick_generic_offset - POINTER_SIZE]
            .copy_from_slice(&quick_resolution.to_ne_bytes());
        let memory = MemoryRanges {
            ranges: vec![
                MemoryRange {
                    start: class_linker.as_ptr() as usize,
                    end: class_linker.as_ptr() as usize + class_linker.len(),
                    executable: false,
                },
                MemoryRange {
                    start: data.as_ptr() as usize,
                    end: data.as_ptr() as usize + data.len(),
                    executable: false,
                },
            ],
        };
        let runtime_layout = ArtRuntimeLayout {
            runtime: std::ptr::dangling_mut(),
            thread_list: std::ptr::dangling_mut(),
            class_linker: class_linker.as_mut_ptr().cast(),
            intern_table,
            jni_id_manager: std::ptr::null_mut(),
            jni_ids_indirection: None,
        };

        assert_eq!(
            detect_class_linker_trampolines(&runtime_layout, 30, None, &memory),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_METHOD_REPLACEMENT,
                reason: "ClassLinker quick resolution trampoline at offset 0xe8 is not executable"
                    .to_owned(),
            })
        );
    }

    #[test]
    fn rejects_pre_api_26_runtime_layout() {
        assert_eq!(
            detect_runtime_layout_from_runtime(
                25,
                std::ptr::dangling_mut::<usize>().cast(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "Android API level 25 is below the API 26+ arm64 milestone".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_null_runtime_layout() {
        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                std::ptr::null_mut(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "ART Runtime pointer is null".to_owned(),
            })
        );
    }

    #[test]
    fn rejects_unknown_runtime_layout() {
        let mut runtime = vec![0usize; 384 / POINTER_SIZE + 100];

        assert_eq!(
            detect_runtime_layout_from_runtime(
                30,
                runtime.as_mut_ptr().cast(),
                0x1234,
                FEATURE_LOADED_CLASS_ENUMERATION,
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_LOADED_CLASS_ENUMERATION,
                reason: "unable to determine ART Runtime field offsets".to_owned(),
            })
        );
    }

    #[test]
    fn maps_unsupported_support_to_matching_feature_error() {
        assert_eq!(
            ensure_feature_supported(
                FEATURE_CLASS_LOADER_ENUMERATION,
                FeatureSupport::Unsupported {
                    reason: "test reason".to_owned(),
                },
            ),
            Err(Error::UnsupportedFeature {
                feature: FEATURE_CLASS_LOADER_ENUMERATION,
                reason: "test reason".to_owned(),
            })
        );
    }

    #[test]
    fn initializes_class_loader_visitor_vtable_after_placement() {
        let mut loaders = Vec::new();
        let mut visitor = ArtClassLoaderVisitor::new(&mut loaders);
        assert!(visitor.vtable.is_null());

        visitor.initialize_vtable();

        assert_eq!(visitor.vtable, visitor.vtable_storage.as_ptr());
        assert_eq!(
            visitor.vtable_storage[2],
            on_visit_class_loader as *const c_void
        );
    }

    #[test]
    fn reports_missing_visit_class_loaders_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "VisitClassLoaders is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_suspend_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ThreadList::SuspendAll is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_resume_all_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ThreadList::ResumeAll is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_visit_classes_as_unsupported() {
        let backend = ArtBackend::empty_for_tests();

        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "ClassLinker::VisitClasses is unavailable".to_owned(),
            }
        );
    }

    #[test]
    fn reports_missing_loaded_class_add_global_ref_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_classes = Some(VisitClassesKind::Visitor(dummy_visit_classes));

        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "JavaVMExt::AddGlobalRef is unavailable".to_owned(),
            }
        );
    }

    #[cfg(not(target_arch = "aarch64"))]
    #[test]
    fn reports_non_arm64_architecture_as_unsupported() {
        let mut backend = ArtBackend::empty_for_tests();
        backend.visit_class_loaders = Some(dummy_visit_class_loaders);
        backend.visit_classes = Some(VisitClassesKind::Visitor(dummy_visit_classes));
        backend.add_global_ref = Some(dummy_add_global_ref);
        backend.suspend_all = Some(SuspendAll::Legacy(dummy_suspend_all));
        backend.resume_all = Some(dummy_resume_all);

        assert_eq!(
            backend.class_loader_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            }
        );
        assert_eq!(
            backend.loaded_class_enumeration_support(NonNull::dangling()),
            FeatureSupport::Unsupported {
                reason: "only arm64-v8a is supported in this milestone".to_owned(),
            }
        );
    }
}
