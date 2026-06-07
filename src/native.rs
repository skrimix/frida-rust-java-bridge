use std::{mem, sync::OnceLock};

use frida_gum::{Gum, NativePointer};

static PROCESS_GUM: OnceLock<Gum> = OnceLock::new();

pub(crate) fn process_gum() -> &'static Gum {
    PROCESS_GUM.get_or_init(Gum::obtain)
}

pub(crate) fn native_pointer_to_fn<T: Copy>(pointer: NativePointer) -> T {
    debug_assert_eq!(mem::size_of::<T>(), mem::size_of::<*mut std::ffi::c_void>());
    unsafe { mem::transmute_copy(&pointer.0) }
}
