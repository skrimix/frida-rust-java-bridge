use std::{
    ffi::{CStr, c_char, c_void},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use crate::{Error, PerformStatus, Result, Runtime, jni};

const TEST_CLASS: &str = "frida.java.bridge.rs.performtest.EarlyPerformProbe";
const STATUS_PENDING: &str = "pending\n";
const STATUS_OK: &str = "ok\n";

static PERFORM_CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

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
                PathBuf::from("/data/local/tmp/frida-java-bridge-rs-apk-perform-agent-error.txt")
            });
            write_status(&path, &format!("error: {error}\n"));
            jni::JNI_ERR
        }
    }
}

fn run_agent(options: *mut c_char) -> Result<()> {
    let status_path = unsafe { status_path_from_options(options)? };
    write_status(&status_path, "attached\n");

    let runtime = Runtime::obtain()?;
    match runtime.app_class_loader() {
        Err(Error::AppClassLoaderUnavailable { reason })
            if reason.contains("ActivityThread.currentApplication() returned null") => {}
        Err(error) => return Err(error),
        Ok(_) => {
            return Err(Error::UnsupportedFeature {
                feature: "APK early-start perform test",
                reason: "app class loader was available before Application creation".to_owned(),
            });
        }
    }

    let callback_status_path = status_path.clone();
    let handle = runtime.perform(move |app_java| {
        let result: Result<()> = (|| {
            let count = PERFORM_CALLBACK_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if count != 1 {
                return Err(Error::UnsupportedFeature {
                    feature: "APK early-start perform test",
                    reason: format!("perform callback ran {count} times"),
                });
            }

            let loader = app_java.loader().ok_or_else(|| Error::UnsupportedFeature {
                feature: "APK early-start perform test",
                reason: "perform callback received a bootstrap Java handle".to_owned(),
            })?;
            if loader.kind() != crate::ClassLoaderKind::App {
                return Err(Error::UnsupportedFeature {
                    feature: "APK early-start perform test",
                    reason: format!(
                        "perform callback loader had unexpected kind {:?}",
                        loader.kind()
                    ),
                });
            }

            let probe = app_java.find_class(TEST_CLASS)?;
            let answer = probe
                .call_static("answer", "()I", &[])?
                .into_int("EarlyPerformProbe.answer")?;
            if answer != 42 {
                return Err(Error::UnsupportedFeature {
                    feature: "APK early-start perform test",
                    reason: format!("EarlyPerformProbe.answer returned {answer}"),
                });
            }

            Ok(())
        })();

        match &result {
            Ok(()) => write_status(&callback_status_path, STATUS_OK),
            Err(error) => write_status(&callback_status_path, &format!("error: {error}\n")),
        }
        result
    })?;

    if handle.status() != PerformStatus::Pending {
        return Err(Error::UnsupportedFeature {
            feature: "APK early-start perform test",
            reason: format!("perform callback was not queued: {:?}", handle.status()),
        });
    }

    write_status(&status_path, STATUS_PENDING);
    Ok(())
}

unsafe fn status_path_from_options(options: *mut c_char) -> Result<PathBuf> {
    if options.is_null() {
        return Err(Error::UnsupportedFeature {
            feature: "APK early-start perform test",
            reason: "Agent_OnAttach options did not include a status path".to_owned(),
        });
    }

    let value = unsafe { CStr::from_ptr(options) }.to_str()?;
    let path = value.strip_prefix("status=").unwrap_or(value).trim();
    if path.is_empty() {
        return Err(Error::UnsupportedFeature {
            feature: "APK early-start perform test",
            reason: "Agent_OnAttach status path was empty".to_owned(),
        });
    }

    Ok(PathBuf::from(path))
}

fn write_status(path: &Path, status: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, status);
}
