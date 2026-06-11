//! Centralized capability labels used in ART unsupported-feature reasons.
//!
//! Keeping these labels in one module makes probing paths report the same feature names even when
//! layout, symbol, and backend checks live in different ART submodules.

use crate::{
    capabilities::FeatureSupport,
    error::{Error, Result},
};

pub(super) const FEATURE_CLASS_LOADER_ENUMERATION: &str = "ART class-loader enumeration";
pub(super) const FEATURE_LOADED_CLASS_ENUMERATION: &str = "ART loaded-class enumeration";
pub(super) const FEATURE_METHOD_QUERY: &str = "ART direct method enumeration";
pub(super) const FEATURE_HEAP_ENUMERATION: &str = "ART heap enumeration";
pub(super) const FEATURE_METHOD_REPLACEMENT: &str = "ART method replacement";
pub(super) const FEATURE_DEOPTIMIZATION: &str = "ART deoptimization";

pub(super) fn unsupported_support(reason: impl Into<String>) -> FeatureSupport {
    FeatureSupport::Unsupported {
        reason: reason.into(),
    }
}

pub(super) fn unsupported_feature<T>(
    feature: &'static str,
    reason: impl Into<String>,
) -> Result<T> {
    Err(Error::UnsupportedFeature {
        feature,
        reason: reason.into(),
    })
}
