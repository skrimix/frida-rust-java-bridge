use super::*;

impl ClassLoaderRef {
    pub fn kind(&self) -> ClassLoaderKind {
        self.kind
    }

    pub fn vm(&self) -> &Vm {
        &self.vm
    }

    pub fn as_jobject(&self) -> jni::jobject {
        self.object.as_jobject()
    }

    #[allow(dead_code)]
    pub(crate) unsafe fn from_global_raw(
        vm: Vm,
        raw: jni::jobject,
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        let attached_vm = vm.clone();
        let env = attached_vm.attach_current_thread()?;
        let object = unsafe { GlobalRef::from_raw(vm.clone(), raw)? };
        let loader = Self {
            vm,
            object: Arc::new(object),
            kind,
        };
        validate_class_loader(&env, &loader, "ClassLoaderRef::from_global_raw")?;
        Ok(loader)
    }

    pub(crate) fn from_object_ref(
        env: &Env<'_>,
        vm: &Vm,
        object: &(impl AsJObject + ?Sized),
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        let reference = unsafe { env.new_global_ref_raw(object.as_jobject())? };
        let object = unsafe { GlobalRef::from_raw(vm.clone(), reference)? };
        let loader = Self {
            vm: vm.clone(),
            object: Arc::new(object),
            kind,
        };
        validate_class_loader(env, &loader, "Java::class_loader_from_object")?;
        Ok(loader)
    }

    pub(super) fn from_java_object(
        env: &Env<'_>,
        vm: &Vm,
        object: &JavaObject,
        kind: ClassLoaderKind,
    ) -> Result<Self> {
        Self::from_object_ref(env, vm, object, kind)
    }
}

impl fmt::Debug for ClassLoaderRef {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("ClassLoaderRef")
            .field("kind", &self.kind)
            .field("object", &self.as_jobject())
            .finish()
    }
}

fn validate_class_loader(
    env: &Env<'_>,
    loader: &ClassLoaderRef,
    operation: &'static str,
) -> Result<()> {
    let class_loader_class = env.find_class("java/lang/ClassLoader")?;
    if env.is_instance_of(loader, &class_loader_class)? {
        Ok(())
    } else {
        let actual = env.get_object_class(loader)?;
        Err(Error::InvalidObjectType {
            operation,
            expected: "java.lang.ClassLoader",
            actual: format!("{:p}", actual.as_jclass()),
        })
    }
}

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
    let application = env
        .call_static_object_method(&activity_thread_class, &current_application, &[])?
        .ok_or_else(|| Error::AppClassLoaderUnavailable {
            reason: "ActivityThread.currentApplication() returned null; use Java::perform for deferred app-loader initialization".to_owned(),
        })?;
    class_loader_from_get_class_loader(env, vm, &application, "Application.getClassLoader")
}

pub(super) fn app_perform_state(vm: Vm) -> &'static AppPerformState {
    APP_PERFORM_STATE.get_or_init(|| AppPerformState::new(vm))
}

pub(super) fn default_app_loader() -> Option<ClassLoaderRef> {
    APP_PERFORM_STATE
        .get()
        .and_then(AppPerformState::default_loader)
}

pub(super) fn default_app_java(vm: &Vm) -> Option<Java> {
    APP_PERFORM_STATE
        .get()
        .and_then(|state| state.default_java(vm))
}

impl AsJObject for ClassLoaderRef {
    fn as_jobject(&self) -> jni::jobject {
        self.as_jobject()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_class_loader_kind_is_distinct() {
        assert_ne!(ClassLoaderKind::App, ClassLoaderKind::Object);
        assert_ne!(ClassLoaderKind::App, ClassLoaderKind::System);
        assert_ne!(ClassLoaderKind::App, ClassLoaderKind::Enumerated);
        assert_eq!(format!("{:?}", ClassLoaderKind::App), "App");
    }

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
