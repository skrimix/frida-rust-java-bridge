use super::*;

pub(super) fn app_class_loader_from_activity_thread(
    env: &Env<'_>,
    vm: &Vm,
) -> Result<ClassLoaderRef> {
    let activity_thread_class = env.find_class("android/app/ActivityThread")?;
    let current_application = env.lookup_static_method(
        &activity_thread_class,
        "currentApplication",
        "()Landroid/app/Application;",
    )?;
    // SAFETY: `current_application` was resolved from `activity_thread_class` immediately above.
    let application = unsafe {
        env.call_static_object_method(&activity_thread_class, &current_application, &[])?
    }
        .ok_or_else(|| Error::AppClassLoaderUnavailable {
            reason: "ActivityThread.currentApplication() returned null; use Java::perform for deferred app-loader initialization".to_owned(),
    })?;
    class_loader_from_get_class_loader(env, vm, &application, "Application.getClassLoader")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_loader_errors() {
        let unsupported = Error::UnsupportedFeature {
            feature: "ART class-loader enumeration",
            reason: "missing symbol".to_owned(),
        };
        assert_eq!(
            unsupported.to_string(),
            "ART class-loader enumeration is not supported: missing symbol"
        );

        let invalid = Error::InvalidObjectType {
            operation: "Java::class_loader_from_object",
            expected: "java.lang.ClassLoader",
            actual: "java.lang.String".to_owned(),
        };
        assert_eq!(
            invalid.to_string(),
            "Java::class_loader_from_object expected java.lang.ClassLoader, got java.lang.String"
        );
    }
}
