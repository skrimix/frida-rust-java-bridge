//! Android runtime test harnesses.
//!
//! Each harness runs in a different process shape, so they stay separate:
//! app_process for already-created VMs and APK startup for early app-loader work.

#[cfg(feature = "apk-perform-test")]
mod apk_perform;
#[cfg(feature = "app-process-test")]
mod app_process;
