use std::{
    ffi::{CStr, c_char, c_void},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{Error, Java, MainThreadTaskStatus, PerformStatus, Result, jni};

const TEST_CLASS: &str = "frida.rust.java.bridge.performtest.EarlyPerformProbe";
const STATUS_PENDING: &str = "pending\n";
const STATUS_OK: &str = "ok\n";

static PERFORM_CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);
static MAIN_THREAD_CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Agent_OnAttach(
    _vm: *mut jni::JavaVM,
    options: *mut c_char,
    _reserved: *mut c_void,
) -> jni::jint {
    match run_agent(options) {
        Ok(()) => jni::JNI_OK,
        Err(error) => {
            let path = unsafe { status_path_from_options(options) }.unwrap_or_else(|_| {
                PathBuf::from("/data/local/tmp/frida-rust-java-bridge-apk-perform-agent-error.txt")
            });
            write_status(&path, &format!("error: {error}\n"));
            jni::JNI_ERR
        }
    }
}

fn run_agent(options: *mut c_char) -> Result<()> {
    let status_path = unsafe { status_path_from_options(options)? };
    write_status(&status_path, "attached\n");

    let java = Java::obtain()?;
    match java.app_class_loader() {
        Err(Error::AppClassLoaderUnavailable { reason })
            if reason.contains("ActivityThread.currentApplication() returned null") => {}
        Err(error) => return Err(error),
        Ok(_) => {
            return Err(apk_test_failure(
                "app class loader was available before Application creation",
            ));
        }
    }

    let bare_java = java.clone();
    let callback_status_path = status_path.clone();
    let handle = java.perform(move |app_java| {
        let result: Result<()> = (|| {
            let count = PERFORM_CALLBACK_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if count != 1 {
                return Err(apk_test_failure(format!(
                    "perform callback ran {count} times"
                )));
            }

            let loader = app_java.loader().ok_or_else(|| {
                apk_test_failure("perform callback received a bootstrap Java handle")
            })?;
            if loader.kind() != crate::ClassLoaderKind::App {
                return Err(apk_test_failure(format!(
                    "perform callback loader had unexpected kind {:?}",
                    loader.kind()
                )));
            }

            let default_loader = bare_java.default_app_loader().ok_or_else(|| {
                apk_test_failure("default app loader was not published before perform callback")
            })?;
            if default_loader.kind() != crate::ClassLoaderKind::App {
                return Err(apk_test_failure(format!(
                    "default app loader had unexpected kind {:?}",
                    default_loader.kind()
                )));
            }

            let bare_probe = bare_java.use_class(TEST_CLASS)?;
            let bare_answer = bare_probe.call::<jni::jint>("answer", ())?;
            if bare_answer != 42 {
                return Err(apk_test_failure(format!(
                    "bare Java::use_class EarlyPerformProbe.answer returned {bare_answer}"
                )));
            }

            let main_status_path = callback_status_path.clone();
            let task = app_java.schedule_on_main_thread(move |main_java| {
                let result: Result<()> = (|| {
                    let count = MAIN_THREAD_CALLBACK_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
                    if count != 1 {
                        return Err(apk_test_failure(format!(
                            "main-thread callback ran {count} times"
                        )));
                    }
                    if !main_java.is_main_thread()? {
                        return Err(apk_test_failure(
                            "scheduled callback did not run on the main thread",
                        ));
                    }

                    let probe = main_java.find_class(TEST_CLASS)?;
                    let answer = probe
                        .call_static("answer", "()I", &[])?
                        .into_int("EarlyPerformProbe.answer")?;
                    if answer != 42 {
                        return Err(apk_test_failure(format!(
                            "EarlyPerformProbe.answer returned {answer}"
                        )));
                    }

                    Ok(())
                })();

                match &result {
                    Ok(()) => write_status(&main_status_path, STATUS_OK),
                    Err(error) => write_status(&main_status_path, &format!("error: {error}\n")),
                }
                result
            })?;

            if task.status() != MainThreadTaskStatus::Pending {
                return Err(apk_test_failure(format!(
                    "main-thread callback was not queued: {:?}",
                    task.status()
                )));
            }

            Ok(())
        })();

        if let Err(error) = &result {
            write_status(&callback_status_path, &format!("error: {error}\n"));
        }
        result
    })?;

    if handle.status() != PerformStatus::Pending {
        return Err(apk_test_failure(format!(
            "perform callback was not queued: {:?}",
            handle.status()
        )));
    }

    write_status(&status_path, STATUS_PENDING);
    Ok(())
}

unsafe fn status_path_from_options(options: *mut c_char) -> Result<PathBuf> {
    if options.is_null() {
        return Err(apk_test_failure(
            "Agent_OnAttach options did not include a status path",
        ));
    }

    let value = unsafe { CStr::from_ptr(options) }.to_str()?;
    let path = value.strip_prefix("status=").unwrap_or(value).trim();
    if path.is_empty() {
        return Err(apk_test_failure("Agent_OnAttach status path was empty"));
    }

    Ok(PathBuf::from(path))
}

fn apk_test_failure(reason: impl Into<String>) -> Error {
    Error::TestFailure {
        harness: "apk_perform",
        reason: reason.into(),
    }
}

fn write_status(path: &Path, status: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, status);
}
