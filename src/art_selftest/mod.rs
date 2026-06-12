//! Internal Android runtime self-tests.
//!
//! Each self-test runs in a different process shape, so they stay separate:
//! app_process for already-created VMs and APK startup for early app-loader work.

pub mod apk_perform;
pub mod app_process;
