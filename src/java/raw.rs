//! Low-level Java handles used by explicit JNI-style operations.
//!
//! Most callers should use [`JavaClass`](super::JavaClass) from [`Java::use_class`](super::Java::use_class)
//! for reflection-backed member lookup and typed wrapper calls. Values in this module are safe
//! crate-owned handles, but their methods ask for explicit descriptors and
//! [`JavaValue`](crate::JavaValue) lists instead of wrapper-style Rust arguments.

use std::sync::Arc;

use super::class::JavaClassInner;

/// An owned global reference to a Java class plus cached method and field IDs.
///
/// The cached JNI IDs are tied to this class' defining identity. Instances from a different loader
/// should be resolved through that loader's [`Java`](super::Java) value instead of reusing this
/// class handle. `name()` returns a Java binary name such as `java.lang.String`, matching the
/// upstream `frida-java-bridge` user-facing class-name convention. Descriptors and
/// [`JavaType`](crate::JavaType) values still use JNI slash-style names such as
/// `Ljava/lang/String;`.
#[derive(Clone)]
pub struct Class {
    pub(crate) inner: Arc<JavaClassInner>,
}
