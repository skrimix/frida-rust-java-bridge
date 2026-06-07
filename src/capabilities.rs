/// The set of features supported by the bridge on the current device.
///
/// Since different Android versions and device environments lay out their runtime structures differently,
/// not every feature (such as deoptimization or class-loader enumeration) is guaranteed to work everywhere.
/// You can query these capabilities beforehand to choose the best fallback strategy. Probing these
/// capabilities is entirely safe and will not install hooks, enqueue callbacks, or modify VM state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaCapabilities {
    /// Whether class-loader enumeration is available.
    pub class_loader_enumeration: FeatureSupport,
    /// Whether loaded Java classes can be enumerated.
    pub loaded_class_enumeration: FeatureSupport,
    /// Whether `Java::perform()` can defer callbacks until the app class loader is published.
    pub app_loader_deferral: FeatureSupport,
    /// Whether callbacks can be queued onto Android's main Java thread.
    pub main_thread_scheduling: FeatureSupport,
    /// Whether live heap instances can be enumerated for a class.
    pub heap_enumeration: FeatureSupport,
    /// Whether ART deoptimization operations are available.
    pub deoptimization: FeatureSupport,
    /// Whether guarded Java method replacement can be installed.
    pub method_replacement: FeatureSupport,
}

/// Indicates whether a specific runtime feature is supported on the current device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureSupport {
    /// The feature is fully functional and ready to use in this process.
    Supported,
    /// The feature is unavailable, with a reason suitable for diagnostics.
    Unsupported { reason: String },
}

impl FeatureSupport {
    /// Returns `true` if the feature can be safely used on this device.
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }

    /// Returns a description of why the feature is unavailable, or `None` if it is supported.
    pub fn unsupported_reason(&self) -> Option<&str> {
        match self {
            Self::Supported => None,
            Self::Unsupported { reason } => Some(reason),
        }
    }
}
