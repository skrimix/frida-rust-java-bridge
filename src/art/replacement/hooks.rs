use std::{
    collections::HashSet,
    ffi::c_void,
    mem::ManuallyDrop,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use frida_gum::{
    NativePointer,
    interceptor::{Interceptor, InvocationContext, InvocationListener, Listener},
};

use super::super::{
    backend::GetOatQuickMethodHeader, features::FEATURE_METHOD_REPLACEMENT,
    layout::ArtClassLinkerTrampolines,
};
use super::controller::{
    ArtReplacementController, GcSynchronizationTiming, global_replacement_controller,
};
use crate::error::{Error, Result};

static ORIGINAL_GET_OAT_QUICK_METHOD_HEADER: AtomicUsize = AtomicUsize::new(0);

#[derive(Default)]
pub(super) struct ArtQuickEntrypointHooks {
    addresses: HashSet<usize>,
    pub(super) hooks: Vec<HookedQuickEntrypoint>,
}

pub(super) struct ArtReplacementHooks {
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

pub(super) struct HookedQuickEntrypoint {
    _interceptor: Interceptor,
    _listener: Box<ArtMethodTranslationListener>,
    _handle: ManuallyDrop<Listener>,
}

struct ReplacedGetOatQuickMethodHeader {
    _function: NativePointer,
    _original: NativePointer,
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

impl ArtReplacementController {
    pub(super) fn ensure_quick_entrypoint_hooks(
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
            trampolines.quick_to_interpreter_bridge_trampoline,
        ] {
            let address = entrypoint as usize;
            if address == 0 || !quick_hooks.addresses.insert(address) {
                continue;
            }

            let mut interceptor = Interceptor::obtain(crate::native::process_gum());
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
}

impl ArtReplacementHooks {
    pub(super) fn install(controller: Arc<ArtReplacementController>) -> Result<Self> {
        let mut interceptor = Interceptor::obtain(crate::native::process_gum());
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
                            _function: NativePointer(function as *mut c_void),
                            _original: original,
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
    if global_replacement_controller().is_some_and(|controller| {
        controller.is_replacement_method(method) || controller.has_dispatch_thunk_pc(method, pc)
    }) {
        return ptr::null_mut();
    }

    let original = ORIGINAL_GET_OAT_QUICK_METHOD_HEADER.load(Ordering::SeqCst);
    if original == 0 {
        return ptr::null_mut();
    }

    let original: GetOatQuickMethodHeader = unsafe { std::mem::transmute(original) };
    unsafe { original(method, pc) }
}

unsafe impl Send for ArtReplacementHooks {}
unsafe impl Sync for ArtReplacementHooks {}
