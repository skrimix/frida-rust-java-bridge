#![allow(dead_code)]

use std::{
    collections::{HashMap, HashSet},
    ffi::{CStr, c_char, c_int, c_void},
    fs,
    mem::ManuallyDrop,
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
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
    env::{Env, MethodKind},
    error::{Error, Result},
    java::{ClassLoaderKind, ClassLoaderRef, JavaChooseControl, JavaObject, RawJavaClass},
    jni, metadata,
    refs::{AsJClass, AsJObject, ClassKind, GlobalRef},
    runtime::{FeatureSupport, native_pointer_to_fn},
    signature::MethodSignature,
    vm::Vm,
};

mod backend;
mod enumeration;
mod layout;
mod replacement;
mod runnable_thread;
mod support;

#[cfg(test)]
mod tests;

pub(crate) use replacement::original_method_call_bypass;

const FEATURE_CLASS_LOADER_ENUMERATION: &str = "ART class-loader enumeration";
const FEATURE_LOADED_CLASS_ENUMERATION: &str = "ART loaded-class enumeration";
const FEATURE_METHOD_QUERY: &str = "ART direct method enumeration";
const FEATURE_HEAP_ENUMERATION: &str = "ART heap enumeration";
const FEATURE_METHOD_REPLACEMENT: &str = "ART method replacement";
const POINTER_SIZE: usize = std::mem::size_of::<*mut c_void>();
const STD_STRING_SIZE: usize = 3 * POINTER_SIZE;
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
static ORIGINAL_CALL_BYPASS_THREAD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CALL_BYPASS_OWNER_THREAD: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL_CALL_BYPASS_LOCK: Mutex<()> = Mutex::new(());
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
const VISIT_OBJECTS: &str = "_ZN3art2gc4Heap12VisitObjectsEPFvPNS_6mirror6ObjectEPvES5_";
const GET_INSTANCES: &str = "_ZN3art2gc4Heap12GetInstancesERNS_24VariableSizedHandleScopeENS_6HandleINS_6mirror5ClassEEEiRNSt3__16vectorINS4_INS5_6ObjectEEENS8_9allocatorISB_EEEE";
const GET_INSTANCES_ASSIGNABLE: &str = "_ZN3art2gc4Heap12GetInstancesERNS_24VariableSizedHandleScopeENS_6HandleINS_6mirror5ClassEEEbiRNSt3__16vectorINS4_INS5_6ObjectEEENS8_9allocatorISB_EEEE";
const GET_CLASS_DESCRIPTOR: &str = "_ZN3art6mirror5Class13GetDescriptorEPNSt3__112basic_stringIcNS2_11char_traitsIcEENS2_9allocatorIcEEEE";
const PRETTY_METHOD: &str = "_ZN3art9ArtMethod12PrettyMethodEb";
const PRETTY_METHOD_NULL_SAFE: &str = "_ZN3art12PrettyMethodEPNS_9ArtMethodEb";
const DECODE_GLOBAL_NO_THREAD: &str = "_ZN3art9JavaVMExt12DecodeGlobalEPv";
const DECODE_GLOBAL_WITH_THREAD: &str = "_ZN3art9JavaVMExt12DecodeGlobalEPNS_6ThreadEPv";
const THREAD_DECODE_GLOBAL_JOBJECT: &str = "_ZNK3art6Thread19DecodeGlobalJObjectEP8_jobject";
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
type VisitObjects = unsafe extern "C" fn(*mut c_void, HeapObjectCallback, *mut c_void);
type HeapObjectCallback = unsafe extern "C" fn(*mut c_void, *mut c_void);
type GetInstances = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, i32, *mut c_void);
type GetInstancesAssignable =
    unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, bool, i32, *mut c_void);
type DecodeGlobalNoThread = unsafe extern "C" fn(*mut jni::JavaVM, jni::jobject) -> usize;
type DecodeGlobalWithThread =
    unsafe extern "C" fn(*mut jni::JavaVM, *mut c_void, jni::jobject) -> usize;
type ThreadDecodeGlobalJObject = unsafe extern "C" fn(*mut c_void, jni::jobject) -> usize;
type GetOatQuickMethodHeader = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;

static ART_REPLACEMENT_CONTROLLER: OnceLock<Arc<ArtReplacementController>> = OnceLock::new();
static ORIGINAL_GET_OAT_QUICK_METHOD_HEADER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
pub(crate) struct ArtBackend {
    android_runtime: Option<ArtModuleRange>,
    add_global_ref: Option<AddGlobalRef>,
    suspend_all: Option<SuspendAll>,
    resume_all: Option<ResumeAll>,
    visit_class_loaders: Option<VisitClassLoaders>,
    visit_classes: Option<VisitClassesKind>,
    visit_objects: Option<VisitObjects>,
    get_instances: Option<GetInstancesKind>,
    decode_global: Option<DecodeGlobalKind>,
    get_class_descriptor: Option<GetClassDescriptor>,
    pretty_method: Option<PrettyMethodFunction>,
    decode_method_id: Option<DecodeMethodId>,
    set_jni_id_type: Option<*const c_void>,
    is_quick_resolution_stub: Option<IsQuickEntrypoint>,
    is_quick_to_interpreter_bridge: Option<IsQuickEntrypoint>,
    is_quick_generic_jni_stub: Option<IsQuickEntrypoint>,
    exception_clear: Option<*const c_void>,
    fatal_error: Option<*const c_void>,
    runnable_thread: Arc<OnceLock<runnable_thread::RunnableThreadTransition>>,
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

#[derive(Clone, Copy)]
enum GetInstancesKind {
    Exact(GetInstances),
    WithAssignable(GetInstancesAssignable),
}

#[derive(Clone, Copy)]
enum DecodeGlobalKind {
    NoThread(DecodeGlobalNoThread),
    WithThread(DecodeGlobalWithThread),
    Thread(ThreadDecodeGlobalJObject),
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

struct RawHeapInstance(jni::jobject);

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

struct ArtHeapInstanceProcessor<'callback> {
    add_global_ref: AddGlobalRef,
    vm_handle: *mut jni::JavaVM,
    thread: *mut c_void,
    needle_class_reference: u32,
    instances: &'callback mut Vec<RawHeapInstance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtRuntimeLayout {
    runtime: *mut c_void,
    heap: *mut c_void,
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
    hook_install: Mutex<()>,
    hooks: OnceLock<ArtReplacementHooks>,
}

#[derive(Debug, Default)]
struct ArtReplacementMappings {
    methods: HashMap<usize, ArtReplacementRecord>,
    jni_ids: HashMap<usize, usize>,
    replacements: HashMap<usize, usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ArtReplacementRecord {
    replacement: usize,
    dispatch_thunk_start: usize,
    dispatch_thunk_end: usize,
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
    _lock: Option<MutexGuard<'static, ()>>,
    previous: usize,
    previous_thread: usize,
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
