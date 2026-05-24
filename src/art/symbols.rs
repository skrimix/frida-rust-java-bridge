pub(super) const ADD_GLOBAL_REF_OBJ_PTR: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadENS_6ObjPtrINS_6mirror6ObjectEEE";
pub(super) const ADD_GLOBAL_REF_POINTER: &str =
    "_ZN3art9JavaVMExt12AddGlobalRefEPNS_6ThreadEPNS_6mirror6ObjectE";
pub(super) const DECODE_GLOBAL_NO_THREAD: &str = "_ZN3art9JavaVMExt12DecodeGlobalEPv";
pub(super) const DECODE_GLOBAL_WITH_THREAD: &str = "_ZN3art9JavaVMExt12DecodeGlobalEPNS_6ThreadEPv";
pub(super) const THREAD_DECODE_GLOBAL_JOBJECT: &str =
    "_ZNK3art6Thread19DecodeGlobalJObjectEP8_jobject";
pub(super) const DECODE_METHOD_ID: &str = "_ZN3art3jni12JniIdManager14DecodeMethodIdEP10_jmethodID";
pub(super) const SET_JNI_ID_TYPE: &str = "_ZN3art7Runtime12SetJniIdTypeENS_9JniIdTypeE";

pub(super) const SUSPEND_ALL_WITH_CAUSE: &str = "_ZN3art10ThreadList10SuspendAllEPKcb";
pub(super) const SUSPEND_ALL_LEGACY: &str = "_ZN3art10ThreadList10SuspendAllEv";
pub(super) const RESUME_ALL: &str = "_ZN3art10ThreadList9ResumeAllEv";

pub(super) const VISIT_CLASS_LOADERS: &str =
    "_ZNK3art11ClassLinker17VisitClassLoadersEPNS_18ClassLoaderVisitorE";
pub(super) const VISIT_CLASSES_VISITOR: &str =
    "_ZN3art11ClassLinker12VisitClassesEPNS_12ClassVisitorE";
pub(super) const VISIT_CLASSES_CALLBACK: &str =
    "_ZN3art11ClassLinker12VisitClassesEPFbPNS_6mirror5ClassEPvES4_";
pub(super) const VISIT_OBJECTS: &str = "_ZN3art2gc4Heap12VisitObjectsEPFvPNS_6mirror6ObjectEPvES5_";
pub(super) const GET_INSTANCES: &str = "_ZN3art2gc4Heap12GetInstancesERNS_24VariableSizedHandleScopeENS_6HandleINS_6mirror5ClassEEEiRNSt3__16vectorINS4_INS5_6ObjectEEENS8_9allocatorISB_EEEE";
pub(super) const GET_INSTANCES_ASSIGNABLE: &str = "_ZN3art2gc4Heap12GetInstancesERNS_24VariableSizedHandleScopeENS_6HandleINS_6mirror5ClassEEEbiRNSt3__16vectorINS4_INS5_6ObjectEEENS8_9allocatorISB_EEEE";
pub(super) const GET_CLASS_DESCRIPTOR: &str = "_ZN3art6mirror5Class13GetDescriptorEPNSt3__112basic_stringIcNS2_11char_traitsIcEENS2_9allocatorIcEEEE";
pub(super) const PRETTY_METHOD: &str = "_ZN3art9ArtMethod12PrettyMethodEb";
pub(super) const PRETTY_METHOD_NULL_SAFE: &str = "_ZN3art12PrettyMethodEPNS_9ArtMethodEb";

pub(super) const IS_QUICK_RESOLUTION_STUB: &str =
    "_ZNK3art11ClassLinker21IsQuickResolutionStubEPKv";
pub(super) const IS_QUICK_TO_INTERPRETER_BRIDGE: &str =
    "_ZNK3art11ClassLinker26IsQuickToInterpreterBridgeEPKv";
pub(super) const IS_QUICK_GENERIC_JNI_STUB: &str =
    "_ZNK3art11ClassLinker21IsQuickGenericJniStubEPKv";
pub(super) const GET_OAT_QUICK_METHOD_HEADER_U32: &str =
    "_ZN3art9ArtMethod23GetOatQuickMethodHeaderEj";
pub(super) const GET_OAT_QUICK_METHOD_HEADER_USIZE: &str =
    "_ZN3art9ArtMethod23GetOatQuickMethodHeaderEm";
pub(super) const GC_COLLECT_GARBAGE_INTERNAL: &str =
    "_ZN3art2gc4Heap22CollectGarbageInternalENS0_9collector6GcTypeENS0_7GcCauseEbj";
pub(super) const CONCURRENT_COPYING_COPYING_PHASE: &str =
    "_ZN3art2gc9collector17ConcurrentCopying12CopyingPhaseEv";
pub(super) const CONCURRENT_COPYING_MARKING_PHASE: &str =
    "_ZN3art2gc9collector17ConcurrentCopying12MarkingPhaseEv";
pub(super) const THREAD_RUN_FLIP_FUNCTION: &str = "_ZN3art6Thread15RunFlipFunctionEPS0_";
pub(super) const THREAD_RUN_FLIP_FUNCTION_WITH_FLAG: &str = "_ZN3art6Thread15RunFlipFunctionEPS0_b";

pub(super) const JNI_EXCEPTION_CLEAR: &str = "_ZN3art3JNIILb1EE14ExceptionClearEP7_JNIEnv";
pub(super) const JNI_FATAL_ERROR: &str = "_ZN3art3JNIILb1EE10FatalErrorEP7_JNIEnvPKc";

pub(super) const DBG_SET_JDWP_ALLOWED: &str = "_ZN3art3Dbg14SetJdwpAllowedEb";
pub(super) const DBG_CONFIGURE_JDWP: &str = "_ZN3art3Dbg13ConfigureJdwpERKNS_4JDWP11JdwpOptionsE";
pub(super) const INTERNAL_DEBUGGER_CONTROL_START_DEBUGGER: &str =
    "_ZN3art31InternalDebuggerControlCallback13StartDebuggerEv";
pub(super) const DBG_START_JDWP: &str = "_ZN3art3Dbg9StartJdwpEv";
pub(super) const DBG_GO_ACTIVE: &str = "_ZN3art3Dbg8GoActiveEv";
pub(super) const DBG_REQUEST_DEOPTIMIZATION: &str =
    "_ZN3art3Dbg21RequestDeoptimizationERKNS_21DeoptimizationRequestE";
pub(super) const DBG_MANAGE_DEOPTIMIZATION: &str = "_ZN3art3Dbg20ManageDeoptimizationEv";
pub(super) const DBG_REGISTRY: &str = "_ZN3art3Dbg9gRegistryE";
pub(super) const DBG_DEBUGGER_ACTIVE: &str = "_ZN3art3Dbg15gDebuggerActiveE";
pub(super) const INSTRUMENTATION_ENABLE_DEOPTIMIZATION: &str =
    "_ZN3art15instrumentation15Instrumentation20EnableDeoptimizationEv";
pub(super) const INSTRUMENTATION_DEOPTIMIZE_EVERYTHING: &str =
    "_ZN3art15instrumentation15Instrumentation20DeoptimizeEverythingEPKc";
pub(super) const INSTRUMENTATION_DEOPTIMIZE: &str =
    "_ZN3art15instrumentation15Instrumentation10DeoptimizeEPNS_9ArtMethodE";
pub(super) const RUNTIME_DEOPTIMIZE_BOOT_IMAGE: &str = "_ZN3art7Runtime19DeoptimizeBootImageEv";
pub(super) const JDWP_ADB_STATE_ACCEPT: &str = "_ZN3art4JDWP12JdwpAdbState6AcceptEv";
pub(super) const JDWP_ADB_STATE_RECEIVE_CLIENT_FD: &str =
    "_ZN3art4JDWP12JdwpAdbState15ReceiveClientFdEv";
