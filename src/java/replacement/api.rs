use std::fmt;

use crate::{Result, env::MethodKind, signature::MethodSignature};

use super::closure::ClosureMethodReplacement;

/// Owns one installed Java method or constructor replacement.
///
/// Keep the guard alive for as long as the replacement should remain installed. Dropping the guard
/// attempts to restore the original implementation, but explicit [`JavaHookGuard::revert`] is the
/// way to observe restore errors.
pub struct JavaHookGuard {
    inner: ClosureMethodReplacement,
}

/// Error or panic reported by an installed Java replacement callback.
///
/// Replacement callbacks run later, when Java calls the hooked method. Callback failures still
/// cause Java callers to receive the JNI default value for the method return type, except for Java
/// exceptions from original-call helpers or Java wrapper calls, which are rethrown to the Java
/// caller when the callback returns the Java-backed error.
/// Callers can attach an [`JavaHookGuard::on_error`] reporter to observe failures as they happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JavaHookError {
    kind: MethodKind,
    name: String,
    signature: MethodSignature,
    message: String,
}

/// Owns several hook guards and can revert them together.
///
/// This is useful when a `perform()` callback installs a group of related replacements and wants to
/// keep one returned value alive for all of them.
///
/// ```no_run
/// use frida_rust_java_bridge::{Java, JavaHookSet, PerformResult, Result};
///
/// fn hook_string_builder(java: &Java) -> Result<PerformResult<JavaHookSet>> {
///     java.perform(|java| {
///         let string_builder = java.use_class("java.lang.StringBuilder")?;
///
///         let init_guard = string_builder.replace_constructor(["java.lang.String"], |ctx| {
///             let arg = ctx.arg_display(0)?;
///             println!("StringBuilder created with {arg}");
///             ctx.call_original(ctx.args())?;
///             ctx.ret(())
///         })?;
///
///         let to_string_guard = string_builder.replace("toString", |ctx| {
///             let result = ctx.call_original::<String>(())?;
///             println!("StringBuilder.toString() => {result}");
///             ctx.ret(result)
///         })?;
///
///         let mut hooks = JavaHookSet::new();
///         hooks.push(init_guard);
///         hooks.push(to_string_guard);
///         Ok(hooks)
///     })
/// }
/// ```
#[derive(Default)]
pub struct JavaHookSet {
    guards: Vec<JavaHookGuard>,
}

impl JavaHookGuard {
    pub(super) fn from_replacement(inner: ClosureMethodReplacement) -> Self {
        Self { inner }
    }

    /// Restores the original method now.
    ///
    /// This is safe to call more than once; after a successful restore, later calls are no-ops. If
    /// restore reports an error, the replacement stays active. Dropping a guard that has not been
    /// successfully restored also attempts a restore, but drop cannot return teardown errors. Use
    /// explicit `revert()` when restore failure must be observed as a `Result`.
    pub fn revert(&mut self) -> Result<()> {
        self.inner.revert()
    }

    /// Installs an error reporter and returns this guard.
    ///
    /// The reporter is called on the Java thread that encountered the callback failure, after the
    /// same error has been recorded for [`JavaHookGuard::last_error`].
    ///
    /// ```no_run
    /// use frida_rust_java_bridge::{Java, JavaHookGuard, Result};
    ///
    /// fn hook_with_logging(java: &Java) -> Result<JavaHookGuard> {
    ///     let class = java.use_class("com.example.app.MyClass")?;
    ///     let guard = class
    ///         .replace("fallible", |ctx| {
    ///             println!("fallible called");
    ///             ctx.call_original(ctx.args())
    ///         })?
    ///         .on_error(|error| eprintln!("hook error: {error}"));
    ///     Ok(guard)
    /// }
    /// ```
    pub fn on_error<F>(self, handler: F) -> Self
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        self.set_error_handler(handler);
        self
    }

    /// Installs or replaces the error reporter for this guard.
    ///
    /// Use this when a guard has already been stored somewhere. For builder-style installation at
    /// the replacement call site, prefer [`JavaHookGuard::on_error`].
    pub fn set_error_handler<F>(&self, handler: F)
    where
        F: Fn(JavaHookError) + Send + Sync + 'static,
    {
        self.inner.set_error_handler(handler);
    }

    /// Clears the installed error reporter, if any.
    pub fn clear_error_handler(&self) {
        self.inner.clear_error_handler();
    }

    /// Returns the most recent callback, panic, or best-effort teardown error recorded by the
    /// replacement.
    ///
    /// Callback failures cause Java callers to receive the JNI default value for the method's
    /// return type unless the callback failure preserved a Java exception for rethrow, and the
    /// error is kept here for explicit inspection while the guard is still alive.
    pub fn last_error(&self) -> Option<String> {
        self.inner.last_error()
    }

    /// Returns and clears the most recent callback, panic, or best-effort teardown error recorded
    /// by the replacement.
    pub fn take_last_error(&self) -> Option<String> {
        self.inner.take_last_error()
    }
}

impl JavaHookError {
    pub(crate) fn new(
        kind: MethodKind,
        name: String,
        signature: MethodSignature,
        message: String,
    ) -> Self {
        Self {
            kind,
            name,
            signature,
            message,
        }
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn signature(&self) -> &MethodSignature {
        &self.signature
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for JavaHookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {}{}: {}",
            hook_kind_name(self.kind),
            self.name,
            self.signature,
            self.message
        )
    }
}

pub(super) fn hook_kind_name(kind: MethodKind) -> &'static str {
    match kind {
        MethodKind::Static => "static method",
        MethodKind::Instance => "instance method",
        MethodKind::Constructor => "constructor",
    }
}

impl JavaHookSet {
    /// Creates an empty hook set.
    pub fn new() -> Self {
        Self { guards: Vec::new() }
    }

    /// Returns the number of guards stored in this set.
    pub fn len(&self) -> usize {
        self.guards.len()
    }

    /// Returns whether this set contains no guards.
    pub fn is_empty(&self) -> bool {
        self.guards.is_empty()
    }

    /// Adds an already-created guard to this set.
    pub fn push(&mut self, guard: JavaHookGuard) {
        self.guards.push(guard);
    }

    /// Restores every guard in reverse installation order.
    ///
    /// All guards are asked to restore even if an earlier restore fails. The first restore error
    /// encountered in reverse order is returned after the remaining guards have been attempted.
    pub fn revert_all(&mut self) -> Result<()> {
        revert_all_in_reverse(&mut self.guards, JavaHookGuard::revert)
    }

    /// Returns the most recent recorded error from each guard that has one.
    pub fn last_errors(&self) -> Vec<String> {
        self.guards
            .iter()
            .filter_map(JavaHookGuard::last_error)
            .collect()
    }
}

fn revert_all_in_reverse<T>(
    guards: &mut [T],
    mut revert: impl FnMut(&mut T) -> Result<()>,
) -> Result<()> {
    let mut first_error = None;
    for guard in guards.iter_mut().rev() {
        if let Err(error) = revert(guard)
            && first_error.is_none()
        {
            first_error = Some(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;

    #[test]
    fn hook_set_revert_loop_attempts_every_guard_after_failure() {
        #[derive(Debug)]
        struct FakeGuard {
            id: usize,
            result: Result<()>,
        }

        let first_error = Error::UnsupportedFeature {
            feature: "test hook revert",
            reason: "middle guard failed".to_owned(),
        };
        let later_error = Error::UnsupportedFeature {
            feature: "test hook revert",
            reason: "newest guard failed".to_owned(),
        };
        let mut guards = [
            FakeGuard {
                id: 0,
                result: Ok(()),
            },
            FakeGuard {
                id: 1,
                result: Err(first_error.clone()),
            },
            FakeGuard {
                id: 2,
                result: Err(later_error.clone()),
            },
        ];
        let mut attempted = Vec::new();

        let result = revert_all_in_reverse(&mut guards, |guard| {
            attempted.push(guard.id);
            guard.result.clone()
        });

        assert_eq!(attempted, [2, 1, 0]);
        assert_eq!(result, Err(later_error));
    }
}
