use std::{
    collections::HashMap,
    ffi::CStr,
    ffi::{c_int, c_void},
    fs,
    mem::ManuallyDrop,
    ptr::{self, NonNull},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use frida_gum::{
    NativePointer,
    interceptor::{Interceptor, InvocationContext, InvocationListener, Listener},
};

use super::{
    ArtVmAccess,
    backend::ArtBackend,
    features::*,
    layout::*,
    memory::MemoryRanges,
    runtime_layout::{android_api_level, detect_runtime_layout_for_api},
};
use crate::{
    capabilities::FeatureSupport,
    error::{Error, Result},
    jni,
};

static JDWP_RECEIVE_CLIENT_FD: AtomicUsize = AtomicUsize::new(usize::MAX);
static ART_JDWP_SESSION: OnceLock<Arc<ArtJdwpSession>> = OnceLock::new();
static ART_JDWP_SESSION_LOCK: Mutex<()> = Mutex::new(());

struct ArtJdwpSession {
    _control_fd: c_int,
    _control_peer_fd: c_int,
    _client_fd: c_int,
    _client_peer_fd: c_int,
    _interceptor: Interceptor,
    _accept_listener: Box<ArtJdwpAcceptListener>,
    _accept_handle: ManuallyDrop<Listener>,
    _receive_client_fd: ReplacedJdwpReceiveClientFd,
}

struct ArtJdwpAcceptListener {
    control_fd: c_int,
    patched: bool,
}

struct ReplacedJdwpReceiveClientFd {
    function: NativePointer,
}

const K_FULL_DEOPTIMIZATION: u32 = 3;
const K_SELECTIVE_DEOPTIMIZATION: u32 = 5;
const JDWP_HANDSHAKE: &[u8; 14] = b"JDWP-Handshake";
const JDWP_STATE_CONTROL_SOCKET_SCAN_START: usize = 8252;
const JDWP_STATE_CONTROL_SOCKET_SCAN_LEN: usize = 256;
const JDWP_STATE_CONTROL_SOCKET_PATTERN_LEN: usize = 6;
const AF_UNIX: c_int = 1;
const SOCK_STREAM: c_int = 1;

impl ArtBackend {
    pub(crate) fn deoptimization_support(&self, vm: &impl ArtVmAccess) -> FeatureSupport {
        match self.detect_deoptimization_layout(vm) {
            Ok(_) => FeatureSupport::Supported,
            Err(Error::UnsupportedFeature { reason, .. }) => unsupported_support(reason),
            Err(error) => unsupported_support(error.to_string()),
        }
    }

    pub(crate) fn deoptimize_everything(&self, vm: &impl ArtVmAccess) -> Result<()> {
        let layout = self.detect_deoptimization_layout(vm)?;
        let env = vm.attach_current_thread()?;
        self.with_runnable_art_thread(&env, FEATURE_DEOPTIMIZATION, |_thread| {
            self.request_deoptimization(&layout, DeoptimizationRequest::Full)
        })
    }

    pub(crate) fn deoptimize_boot_image(&self, vm: &impl ArtVmAccess) -> Result<()> {
        let layout = self.detect_deoptimization_layout(vm)?;
        let env = vm.attach_current_thread()?;
        self.with_runnable_art_thread(&env, FEATURE_DEOPTIMIZATION, |_thread| {
            let deoptimize_boot_image = self
                .deoptimization
                .runtime_deoptimize_boot_image
                .ok_or_else(|| Error::UnsupportedFeature {
                    feature: FEATURE_DEOPTIMIZATION,
                    reason: "Runtime::DeoptimizeBootImage is unavailable".to_owned(),
                })?;
            unsafe { deoptimize_boot_image(layout.runtime.runtime) };
            Ok(())
        })
    }

    pub(crate) fn deoptimize_method(
        &self,
        vm: &impl ArtVmAccess,
        method_id: jni::jmethodID,
    ) -> Result<()> {
        let layout = self.detect_deoptimization_layout(vm)?;
        let env = vm.attach_current_thread()?;
        self.with_runnable_art_thread(&env, FEATURE_DEOPTIMIZATION, |_thread| {
            let method = self.resolve_deoptimization_method(&layout, method_id)?;
            if self
                .replacement_controller
                .replacement_for(method)
                .is_some()
            {
                return unsupported_feature(
                    FEATURE_DEOPTIMIZATION,
                    "selected ArtMethod has an active replacement; revert the replacement before deoptimizing it",
                );
            }
            self.request_deoptimization(&layout, DeoptimizationRequest::Selective(method))
        })
    }

    fn detect_deoptimization_layout(
        &self,
        vm: &impl ArtVmAccess,
    ) -> Result<ArtDeoptimizationLayout> {
        if !cfg!(target_arch = "aarch64") {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "only arm64-v8a is supported in this milestone",
            );
        }

        let api_level = android_api_level(FEATURE_DEOPTIMIZATION)?;
        if api_level < 26 {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                format!("Android API level {api_level} is below the API 26+ arm64 milestone"),
            );
        }
        if self.deoptimization.runtime_deoptimize_boot_image.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Runtime::DeoptimizeBootImage is unavailable",
            );
        }

        let runtime = detect_runtime_layout_for_api(
            // SAFETY: Deoptimization layout probing operates on the live process JavaVM.
            unsafe { vm.handle() },
            api_level,
            FEATURE_DEOPTIMIZATION,
        )?;
        let instrumentation = if api_level >= 30 {
            Some(self.detect_instrumentation(&runtime, api_level)?)
        } else {
            self.ensure_dbg_deoptimization_supported()?;
            None
        };

        Ok(ArtDeoptimizationLayout {
            api_level,
            runtime,
            instrumentation,
        })
    }

    fn detect_instrumentation(
        &self,
        runtime: &ArtRuntimeLayout,
        api_level: i32,
    ) -> Result<*mut c_void> {
        if self
            .deoptimization
            .instrumentation_deoptimize_everything
            .is_none()
        {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Instrumentation::DeoptimizeEverything is unavailable",
            );
        }
        if self.deoptimization.instrumentation_deoptimize.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Instrumentation::Deoptimize is unavailable",
            );
        }
        let deoptimize_boot_image = self
            .deoptimization
            .runtime_deoptimize_boot_image
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_DEOPTIMIZATION,
                reason: "Runtime::DeoptimizeBootImage is unavailable".to_owned(),
            })?;
        let Some(offset) = detect_instrumentation_offset(
            FEATURE_DEOPTIMIZATION,
            deoptimize_boot_image as *const c_void,
            art_apex_version(api_level) >= 360_000_000,
        )?
        else {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "unable to determine Runtime instrumentation field offset",
            );
        };
        let instrumentation_slot = (runtime.runtime as usize + offset) as *mut *mut c_void;
        let instrumentation = if art_apex_version(api_level) >= 360_000_000 {
            unsafe { instrumentation_slot.read() }
        } else {
            instrumentation_slot.cast::<c_void>()
        };
        if instrumentation.is_null() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Runtime instrumentation pointer is null",
            );
        }
        Ok(instrumentation)
    }

    fn ensure_dbg_deoptimization_supported(&self) -> Result<()> {
        if self.deoptimization.dbg_set_jdwp_allowed.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Dbg::SetJdwpAllowed is unavailable",
            );
        }
        if self.deoptimization.dbg_configure_jdwp.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Dbg::ConfigureJdwp is unavailable",
            );
        }
        if self.deoptimization.internal_start_debugger.is_none()
            && self.deoptimization.dbg_start_jdwp.is_none()
        {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Dbg::StartJdwp and InternalDebuggerControlCallback::StartDebugger are unavailable",
            );
        }
        if self.deoptimization.dbg_go_active.is_none() {
            return unsupported_feature(FEATURE_DEOPTIMIZATION, "Dbg::GoActive is unavailable");
        }
        if self.deoptimization.dbg_request_deoptimization.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Dbg::RequestDeoptimization is unavailable",
            );
        }
        if self.deoptimization.dbg_manage_deoptimization.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Dbg::ManageDeoptimization is unavailable",
            );
        }
        if self.deoptimization.dbg_registry.is_none() {
            return unsupported_feature(FEATURE_DEOPTIMIZATION, "Dbg::gRegistry is unavailable");
        }
        if self.deoptimization.dbg_debugger_active.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "Dbg::gDebuggerActive is unavailable",
            );
        }
        if self.deoptimization.jdwp_adb_state_accept.is_none() {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "JDWP::JdwpAdbState::Accept is unavailable",
            );
        }
        if self
            .deoptimization
            .jdwp_adb_state_receive_client_fd
            .is_none()
        {
            return unsupported_feature(
                FEATURE_DEOPTIMIZATION,
                "JDWP::JdwpAdbState::ReceiveClientFd is unavailable",
            );
        }
        Ok(())
    }

    fn request_deoptimization(
        &self,
        layout: &ArtDeoptimizationLayout,
        request: DeoptimizationRequest,
    ) -> Result<()> {
        if layout.api_level >= 30 {
            return self.request_instrumentation_deoptimization(layout, request);
        }

        self.ensure_jdwp_ready(layout.api_level)?;
        let request_deoptimization = self
            .deoptimization
            .dbg_request_deoptimization
            .expect("Dbg::RequestDeoptimization checked before deoptimization");
        let manage_deoptimization = self
            .deoptimization
            .dbg_manage_deoptimization
            .expect("Dbg::ManageDeoptimization checked before deoptimization");
        let request = ArtDbgDeoptimizationRequest::new(request);
        unsafe {
            request_deoptimization((&request as *const ArtDbgDeoptimizationRequest).cast());
            manage_deoptimization();
        }
        Ok(())
    }

    fn request_instrumentation_deoptimization(
        &self,
        layout: &ArtDeoptimizationLayout,
        request: DeoptimizationRequest,
    ) -> Result<()> {
        let instrumentation = layout
            .instrumentation
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_DEOPTIMIZATION,
                reason: "Runtime instrumentation pointer was not probed".to_owned(),
            })?;

        if let Some(enable_deoptimization) =
            self.deoptimization.instrumentation_enable_deoptimization
        {
            unsafe { enable_deoptimization(instrumentation) };
        }

        match request {
            DeoptimizationRequest::Full => {
                let deoptimize_everything = self
                    .deoptimization
                    .instrumentation_deoptimize_everything
                    .ok_or_else(|| Error::UnsupportedFeature {
                        feature: FEATURE_DEOPTIMIZATION,
                        reason: "Instrumentation::DeoptimizeEverything is unavailable".to_owned(),
                    })?;
                static KEY: &CStr = c"frida";
                unsafe { deoptimize_everything(instrumentation, KEY.as_ptr()) };
            }
            DeoptimizationRequest::Selective(method) => {
                let deoptimize =
                    self.deoptimization
                        .instrumentation_deoptimize
                        .ok_or_else(|| Error::UnsupportedFeature {
                            feature: FEATURE_DEOPTIMIZATION,
                            reason: "Instrumentation::Deoptimize is unavailable".to_owned(),
                        })?;
                unsafe { deoptimize(instrumentation, method) };
            }
        }
        Ok(())
    }

    fn ensure_jdwp_ready(&self, api_level: i32) -> Result<()> {
        self.ensure_dbg_deoptimization_supported()?;
        if !self.is_jdwp_started() {
            let _startup = ART_JDWP_SESSION_LOCK
                .lock()
                .expect("ART JDWP session startup mutex poisoned");
            if !self.is_jdwp_started() {
                if ART_JDWP_SESSION.get().is_none() {
                    let session = ArtJdwpSession::start(
                        self.deoptimization
                            .jdwp_adb_state_accept
                            .expect("JDWP Accept checked before startup"),
                        self.deoptimization
                            .jdwp_adb_state_receive_client_fd
                            .expect("JDWP ReceiveClientFd checked before startup"),
                    )?;
                    let _ = ART_JDWP_SESSION.set(Arc::new(session));
                }

                let set_allowed = self
                    .deoptimization
                    .dbg_set_jdwp_allowed
                    .expect("Dbg::SetJdwpAllowed checked before JDWP startup");
                let configure = self
                    .deoptimization
                    .dbg_configure_jdwp
                    .expect("Dbg::ConfigureJdwp checked before JDWP startup");
                let options = JdwpOptionsBytes::new(api_level);
                unsafe {
                    set_allowed(true);
                    configure(options.as_ptr());
                    if let Some(start_debugger) = self.deoptimization.internal_start_debugger {
                        start_debugger(ptr::null_mut());
                    } else {
                        self.deoptimization
                            .dbg_start_jdwp
                            .expect("Dbg::StartJdwp checked before JDWP startup")(
                        );
                    }
                }
            }
        }

        if !self.is_debugger_active() {
            let go_active = self
                .deoptimization
                .dbg_go_active
                .expect("Dbg::GoActive checked before deoptimization");
            unsafe { go_active() };
        }
        Ok(())
    }

    fn is_jdwp_started(&self) -> bool {
        self.deoptimization
            .dbg_registry
            .is_some_and(|registry| unsafe { registry.cast::<usize>().read() != 0 })
    }

    fn is_debugger_active(&self) -> bool {
        self.deoptimization
            .dbg_debugger_active
            .is_some_and(|active| unsafe { active.cast::<u8>().read() != 0 })
    }

    fn resolve_deoptimization_method(
        &self,
        layout: &ArtDeoptimizationLayout,
        method_id: jni::jmethodID,
    ) -> Result<*mut c_void> {
        if method_id.is_null() {
            return Err(Error::NullReturn {
                operation: "JNI method ID for deoptimization",
            });
        }
        let memory = MemoryRanges::current_for_feature(FEATURE_DEOPTIMIZATION)?;
        self.art_method_from_jni_id(&layout.runtime, method_id)
            .into_iter()
            .find(|method| {
                !method.is_null() && memory.contains(*method as usize, ART_METHOD_MIN_SIZE)
            })
            .ok_or_else(|| Error::UnsupportedFeature {
                feature: FEATURE_DEOPTIMIZATION,
                reason: "unable to resolve ArtMethod from JNI method ID".to_owned(),
            })
    }
}

#[derive(Debug, Clone, Copy)]
enum DeoptimizationRequest {
    Full,
    Selective(*mut c_void),
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct ArtDbgDeoptimizationRequest {
    kind: u32,
    padding: u32,
    method: *mut c_void,
}

impl ArtDbgDeoptimizationRequest {
    fn new(request: DeoptimizationRequest) -> Self {
        match request {
            DeoptimizationRequest::Full => Self {
                kind: K_FULL_DEOPTIMIZATION,
                padding: 0,
                method: ptr::null_mut(),
            },
            DeoptimizationRequest::Selective(method) => Self {
                kind: K_SELECTIVE_DEOPTIMIZATION,
                padding: 0,
                method,
            },
        }
    }
}

struct JdwpOptionsBytes {
    bytes: [u8; 8 + STD_STRING_SIZE + 2],
}

impl JdwpOptionsBytes {
    fn new(api_level: i32) -> Self {
        let transport: u32 = if api_level < 28 { 2 } else { 3 };
        let mut bytes = [0; 8 + STD_STRING_SIZE + 2];
        bytes[0..4].copy_from_slice(&transport.to_ne_bytes());
        bytes[4] = 1;
        bytes[5] = 0;
        bytes[8 + STD_STRING_SIZE..8 + STD_STRING_SIZE + 2].copy_from_slice(&0u16.to_ne_bytes());
        Self { bytes }
    }

    fn as_ptr(&self) -> *const c_void {
        self.bytes.as_ptr().cast()
    }
}

impl ArtJdwpSession {
    fn start(accept: *const c_void, receive_client_fd: *const c_void) -> Result<Self> {
        let control_pair = socket_pair()?;
        let client_pair = socket_pair()?;
        JDWP_RECEIVE_CLIENT_FD.store(client_pair[1] as usize, Ordering::SeqCst);

        let mut interceptor = Interceptor::obtain(crate::native::process_gum());
        let mut accept_listener = Box::new(ArtJdwpAcceptListener {
            control_fd: control_pair[1],
            patched: false,
        });
        let accept_handle = interceptor
            .attach(
                NativePointer(accept as *mut c_void),
                accept_listener.as_mut(),
            )
            .map_err(|error| Error::UnsupportedFeature {
                feature: FEATURE_DEOPTIMIZATION,
                reason: format!("unable to hook JDWP Accept: {error:?}"),
            })?;
        interceptor
            .replace(
                NativePointer(receive_client_fd as *mut c_void),
                NativePointer(on_jdwp_receive_client_fd as *mut c_void),
                NativePointer(ptr::null_mut()),
            )
            .map_err(|error| Error::UnsupportedFeature {
                feature: FEATURE_DEOPTIMIZATION,
                reason: format!("unable to replace JDWP ReceiveClientFd: {error:?}"),
            })?;

        start_jdwp_handshake(client_pair[0]);

        Ok(Self {
            _control_fd: control_pair[0],
            _control_peer_fd: control_pair[1],
            _client_fd: client_pair[0],
            _client_peer_fd: client_pair[1],
            _interceptor: interceptor,
            _accept_listener: accept_listener,
            _accept_handle: ManuallyDrop::new(accept_handle),
            _receive_client_fd: ReplacedJdwpReceiveClientFd {
                function: NativePointer(receive_client_fd as *mut c_void),
            },
        })
    }
}

// The JDWP hooks are process-global Gum interceptor state. The session is installed once and then
// only kept alive, while mutable callback state is restricted to the listener's Gum invocation.
unsafe impl Send for ArtJdwpSession {}
unsafe impl Sync for ArtJdwpSession {}

impl InvocationListener for ArtJdwpAcceptListener {
    fn on_enter(&mut self, context: InvocationContext<'_>) {
        if self.patched {
            return;
        }

        let state = context.arg(0) as *mut u8;
        if state.is_null() {
            return;
        }

        let Ok(memory) = MemoryRanges::current_for_feature(FEATURE_DEOPTIMIZATION) else {
            return;
        };
        if unsafe { patch_jdwp_control_socket(state, self.control_fd, &memory) } {
            self.patched = true;
        }
    }

    fn on_leave(&mut self, _context: InvocationContext<'_>) {}
}

unsafe extern "C" fn on_jdwp_receive_client_fd(_state: *mut c_void) -> c_int {
    take_jdwp_receive_client_fd()
}

fn take_jdwp_receive_client_fd() -> c_int {
    let fd = JDWP_RECEIVE_CLIENT_FD.swap(usize::MAX, Ordering::SeqCst);
    if fd == usize::MAX { -1 } else { fd as c_int }
}

unsafe fn patch_jdwp_control_socket(
    state: *mut u8,
    control_fd: c_int,
    memory: &MemoryRanges,
) -> bool {
    let Some(scan_start) = (state as usize).checked_add(JDWP_STATE_CONTROL_SOCKET_SCAN_START)
    else {
        return false;
    };
    if !memory.contains(scan_start, JDWP_STATE_CONTROL_SOCKET_SCAN_LEN) {
        return false;
    }

    let scan_start = unsafe { state.add(JDWP_STATE_CONTROL_SOCKET_SCAN_START) };
    for offset in
        0..JDWP_STATE_CONTROL_SOCKET_SCAN_LEN.saturating_sub(JDWP_STATE_CONTROL_SOCKET_PATTERN_LEN)
    {
        let candidate = unsafe { scan_start.add(offset) };
        if unsafe {
            candidate.read() == 0
                && candidate.add(1).read() == 0xff
                && candidate.add(2).read() == 0xff
                && candidate.add(3).read() == 0xff
                && candidate.add(4).read() == 0xff
                && candidate.add(5).read() == 0
        } {
            let control_socket = unsafe { candidate.add(1) };
            if !memory.contains_writable(control_socket as usize, std::mem::size_of::<c_int>()) {
                return false;
            }
            unsafe { control_socket.cast::<c_int>().write_unaligned(control_fd) };
            return true;
        }
    }
    false
}

impl Drop for ReplacedJdwpReceiveClientFd {
    fn drop(&mut self) {
        JDWP_RECEIVE_CLIENT_FD.store(usize::MAX, Ordering::SeqCst);
        let mut interceptor = Interceptor::obtain(crate::native::process_gum());
        interceptor.revert(self.function);
    }
}

fn socket_pair() -> Result<[c_int; 2]> {
    let mut fds = [0; 2];
    let result = unsafe { socketpair(AF_UNIX, SOCK_STREAM, 0, fds.as_mut_ptr()) };
    if result != 0 {
        return unsupported_feature(FEATURE_DEOPTIMIZATION, "unable to create JDWP socketpair");
    }
    Ok(fds)
}

fn start_jdwp_handshake(fd: c_int) {
    let _ = std::thread::Builder::new().spawn(move || unsafe {
        let _ = write(fd, JDWP_HANDSHAKE.as_ptr().cast(), JDWP_HANDSHAKE.len());
        let mut response = [0u8; JDWP_HANDSHAKE.len()];
        let _ = read(fd, response.as_mut_ptr().cast(), response.len());
    });
}

pub(super) fn art_apex_version(api_level: i32) -> i32 {
    let Ok(mount_info) = fs::read_to_string("/proc/self/mountinfo") else {
        return api_level * 10_000_000;
    };

    let mut art_source = None;
    let mut source_versions = HashMap::new();
    for line in mount_info.lines() {
        let elements = line.split_whitespace().collect::<Vec<_>>();
        if elements.len() <= 10 {
            continue;
        }
        let mount_root = elements[4];
        if !mount_root.starts_with("/apex/com.android.art") {
            continue;
        }
        let mount_source = elements[10];
        if let Some((_, version)) = mount_root.split_once('@') {
            source_versions.insert(mount_source.to_owned(), version.to_owned());
        } else {
            art_source = Some(mount_source.to_owned());
        }
    }

    art_source
        .and_then(|source| source_versions.get(&source).cloned())
        .and_then(|version| version.parse::<i32>().ok())
        .unwrap_or(api_level * 10_000_000)
}

#[cfg(target_arch = "aarch64")]
fn detect_instrumentation_offset(
    feature: &'static str,
    deoptimize_boot_image: *const c_void,
    instrumentation_is_pointer: bool,
) -> Result<Option<usize>> {
    use frida_gum_sys as gum_sys;

    let mut relocator = Arm64Relocator::new(deoptimize_boot_image as u64);
    for _ in 0..30 {
        let (offset, instruction) = relocator.read_one();
        if offset == 0 || instruction.is_null() {
            return Ok(None);
        }

        let instruction = unsafe { &*instruction };
        let detail = NonNull::new(instruction.detail).ok_or_else(|| Error::UnsupportedFeature {
            feature,
            reason: format!(
                "unable to decode Runtime::DeoptimizeBootImage instruction detail at {:#x}",
                instruction.address
            ),
        })?;
        let arm64 = unsafe { detail.as_ref().__bindgen_anon_1.arm64 };
        let operands = &arm64.operands[..arm64.op_count as usize];

        let maybe_offset = if instrumentation_is_pointer {
            if instruction.id != gum_sys::arm64_insn_ARM64_INS_LDR {
                None
            } else {
                let rt = arm64_operand_reg(operands, 0);
                let mem = arm64_operand_mem(operands, 1);
                match (rt, mem) {
                    (Some(rt), Some((base, disp)))
                        if rt != gum_sys::arm64_reg_ARM64_REG_X0
                            && base == gum_sys::arm64_reg_ARM64_REG_X0 =>
                    {
                        Some(disp)
                    }
                    _ => None,
                }
            }
        } else if instruction.id == gum_sys::arm64_insn_ARM64_INS_ADD {
            let rd = arm64_operand_reg(operands, 0);
            let rn = arm64_operand_reg(operands, 1);
            let imm = arm64_operand_imm(operands, 2);
            match (rd, rn, imm) {
                (Some(rd), Some(rn), Some(imm))
                    if rd != gum_sys::arm64_reg_ARM64_REG_SP
                        && rn != gum_sys::arm64_reg_ARM64_REG_SP =>
                {
                    Some(imm)
                }
                _ => None,
            }
        } else {
            None
        };

        if let Some(offset) = maybe_offset
            && (0x100..=0x400).contains(&offset)
        {
            return Ok(Some(offset as usize));
        }
    }
    Ok(None)
}

#[cfg(not(target_arch = "aarch64"))]
fn detect_instrumentation_offset(
    _feature: &'static str,
    _deoptimize_boot_image: *const c_void,
    _instrumentation_is_pointer: bool,
) -> Result<Option<usize>> {
    Ok(None)
}

#[cfg(target_arch = "aarch64")]
struct Arm64Relocator {
    inner: *mut c_void,
}

#[cfg(target_arch = "aarch64")]
impl Arm64Relocator {
    fn new(input_code: u64) -> Self {
        unsafe extern "C" {
            fn gum_arm64_relocator_new(
                input_code: *const c_void,
                output: *mut frida_gum_sys::_GumArm64Writer,
            ) -> *mut c_void;
        }

        Self {
            inner: unsafe { gum_arm64_relocator_new(input_code as *const c_void, ptr::null_mut()) },
        }
    }

    fn read_one(&mut self) -> (u32, *const frida_gum_sys::cs_insn) {
        unsafe extern "C" {
            fn gum_arm64_relocator_read_one(
                relocator: *mut c_void,
                instruction: *mut *const frida_gum_sys::cs_insn,
            ) -> u32;
        }

        let mut instruction = ptr::null();
        let offset = unsafe { gum_arm64_relocator_read_one(self.inner, &mut instruction) };
        (offset, instruction)
    }
}

#[cfg(target_arch = "aarch64")]
impl Drop for Arm64Relocator {
    fn drop(&mut self) {
        unsafe extern "C" {
            fn gum_arm64_relocator_unref(relocator: *mut c_void);
        }

        unsafe { gum_arm64_relocator_unref(self.inner) };
    }
}

#[cfg(target_arch = "aarch64")]
fn arm64_operand_reg(operands: &[frida_gum_sys::cs_arm64_op], index: usize) -> Option<u32> {
    let operand = operands.get(index)?;
    if operand.type_ == frida_gum_sys::arm64_op_type_ARM64_OP_REG {
        Some(unsafe { operand.__bindgen_anon_1.reg })
    } else {
        None
    }
}

#[cfg(target_arch = "aarch64")]
fn arm64_operand_imm(operands: &[frida_gum_sys::cs_arm64_op], index: usize) -> Option<i64> {
    let operand = operands.get(index)?;
    if operand.type_ == frida_gum_sys::arm64_op_type_ARM64_OP_IMM {
        Some(unsafe { operand.__bindgen_anon_1.imm })
    } else {
        None
    }
}

#[cfg(target_arch = "aarch64")]
fn arm64_operand_mem(operands: &[frida_gum_sys::cs_arm64_op], index: usize) -> Option<(u32, i64)> {
    let operand = operands.get(index)?;
    if operand.type_ != frida_gum_sys::arm64_op_type_ARM64_OP_MEM {
        return None;
    }

    let mem = unsafe { operand.__bindgen_anon_1.mem };
    Some((mem.base, mem.disp as i64))
}

unsafe extern "C" {
    fn socketpair(domain: c_int, ty: c_int, protocol: c_int, sv: *mut c_int) -> c_int;
    fn read(fd: c_int, buf: *mut c_void, count: usize) -> isize;
    fn write(fd: c_int, buf: *const c_void, count: usize) -> isize;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dbg_deoptimization_request_layout_matches_frida_js_api_shape() {
        assert_eq!(
            std::mem::size_of::<ArtDbgDeoptimizationRequest>(),
            8 + POINTER_SIZE
        );

        let full = ArtDbgDeoptimizationRequest::new(DeoptimizationRequest::Full);
        assert_eq!(full.kind, K_FULL_DEOPTIMIZATION);
        assert!(full.method.is_null());

        let method = 0x1234usize as *mut c_void;
        let selective = ArtDbgDeoptimizationRequest::new(DeoptimizationRequest::Selective(method));
        assert_eq!(selective.kind, K_SELECTIVE_DEOPTIMIZATION);
        assert_eq!(selective.method, method);
    }

    #[test]
    fn jdwp_options_match_android_adb_transport_by_api_level() {
        let api_27 = JdwpOptionsBytes::new(27);
        assert_eq!(
            u32::from_ne_bytes(api_27.bytes[0..4].try_into().unwrap()),
            2
        );
        assert_eq!(api_27.bytes[4], 1);
        assert_eq!(api_27.bytes[5], 0);

        let api_28 = JdwpOptionsBytes::new(28);
        assert_eq!(
            u32::from_ne_bytes(api_28.bytes[0..4].try_into().unwrap()),
            3
        );
        assert_eq!(
            u16::from_ne_bytes(
                api_28.bytes[8 + STD_STRING_SIZE..8 + STD_STRING_SIZE + 2]
                    .try_into()
                    .unwrap()
            ),
            0
        );
    }

    #[test]
    fn jdwp_receive_client_fd_is_consumed_once() {
        JDWP_RECEIVE_CLIENT_FD.store(123, Ordering::SeqCst);
        assert_eq!(take_jdwp_receive_client_fd(), 123);
        assert_eq!(take_jdwp_receive_client_fd(), -1);
    }

    #[test]
    fn jdwp_control_socket_patch_validates_memory_and_patches_once() {
        let mut state =
            vec![0u8; JDWP_STATE_CONTROL_SOCKET_SCAN_START + JDWP_STATE_CONTROL_SOCKET_SCAN_LEN];
        let pattern_offset = JDWP_STATE_CONTROL_SOCKET_SCAN_START + 7;
        state[pattern_offset..pattern_offset + JDWP_STATE_CONTROL_SOCKET_PATTERN_LEN]
            .copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0]);
        let memory = MemoryRanges {
            ranges: vec![crate::art::memory::MemoryRange {
                start: state.as_ptr() as usize,
                end: state.as_ptr() as usize + state.len(),
                writable: true,
                executable: false,
            }],
        };

        let patched = unsafe { patch_jdwp_control_socket(state.as_mut_ptr(), 77, &memory) };
        assert!(patched);
        assert_eq!(
            unsafe {
                state
                    .as_ptr()
                    .add(pattern_offset + 1)
                    .cast::<c_int>()
                    .read_unaligned()
            },
            77
        );
    }

    #[test]
    fn jdwp_control_socket_patch_rejects_unwritable_slot() {
        let mut state =
            vec![0u8; JDWP_STATE_CONTROL_SOCKET_SCAN_START + JDWP_STATE_CONTROL_SOCKET_SCAN_LEN];
        let pattern_offset = JDWP_STATE_CONTROL_SOCKET_SCAN_START + 7;
        state[pattern_offset..pattern_offset + JDWP_STATE_CONTROL_SOCKET_PATTERN_LEN]
            .copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0]);
        let memory = MemoryRanges {
            ranges: vec![crate::art::memory::MemoryRange {
                start: state.as_ptr() as usize,
                end: state.as_ptr() as usize + state.len(),
                writable: false,
                executable: false,
            }],
        };

        let patched = unsafe { patch_jdwp_control_socket(state.as_mut_ptr(), 77, &memory) };
        assert!(!patched);
    }
}
