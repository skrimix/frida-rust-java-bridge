use std::ffi::{CStr, CString, c_char};

use crate::error::{Error, Result};

const ANDROID_VERSION_FEATURE: &str = "Android version";
const PROP_VALUE_MAX: usize = 92;

/// Android version of the current device.
///
/// This includes the user-facing release string from `ro.build.version.release` and the SDK API
/// level from `ro.build.version.sdk`. ART integrations use the API level to decide which runtime
/// strategies are safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AndroidVersion {
    /// User-facing Android release string, such as `"14"` or `"15"`.
    pub release: String,
    /// Android SDK API level, such as `34` or `35`.
    pub api_level: i32,
}

pub(crate) fn android_version() -> Result<AndroidVersion> {
    Ok(AndroidVersion {
        release: android_property("ro.build.version.release", ANDROID_VERSION_FEATURE)?,
        api_level: android_api_level()?,
    })
}

pub(crate) fn android_api_level() -> Result<i32> {
    android_api_level_for_feature(ANDROID_VERSION_FEATURE)
}

pub(crate) fn android_api_level_for_feature(feature: &'static str) -> Result<i32> {
    let value = android_property("ro.build.version.sdk", feature)?;
    parse_android_api_level(feature, &value)
}

fn android_property(name: &'static str, feature: &'static str) -> Result<String> {
    let property_name = CString::new(name).expect("Android property name has no interior NUL");
    let mut value = [0 as c_char; PROP_VALUE_MAX];
    let len = unsafe { __system_property_get(property_name.as_ptr(), value.as_mut_ptr()) };
    if len <= 0 {
        return Err(Error::UnsupportedFeature {
            feature,
            reason: format!("unable to read {name}"),
        });
    }

    unsafe { CStr::from_ptr(value.as_ptr()) }
        .to_str()
        .map(str::to_owned)
        .map_err(|_| Error::UnsupportedFeature {
            feature,
            reason: format!("{name} is not valid UTF-8"),
        })
}

fn parse_android_api_level(feature: &'static str, value: &str) -> Result<i32> {
    value.parse().map_err(|_| Error::UnsupportedFeature {
        feature,
        reason: format!("ro.build.version.sdk is not an integer: {value:?}"),
    })
}

unsafe extern "C" {
    fn __system_property_get(name: *const c_char, value: *mut c_char) -> i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_android_api_level() {
        assert_eq!(parse_android_api_level("test", "36"), Ok(36));
    }

    #[test]
    fn android_api_level_parse_errors_include_property_value() {
        assert_eq!(
            parse_android_api_level("test feature", "vanilla-ice-cream"),
            Err(Error::UnsupportedFeature {
                feature: "test feature",
                reason: "ro.build.version.sdk is not an integer: \"vanilla-ice-cream\"".to_owned(),
            })
        );
    }
}
