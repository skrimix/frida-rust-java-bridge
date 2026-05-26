use super::*;

impl Env<'_> {
    pub fn lookup_instance_method(
        &self,
        class: &impl AsJClass,
        name: &str,
        signature: &str,
    ) -> Result<MethodId> {
        let signature = MethodSignature::parse(signature)?;
        let raw = self.get_method_id_raw(class.as_jclass(), name, &signature)?;
        Ok(MethodId {
            raw,
            kind: MethodKind::Instance,
            signature,
        })
    }

    /// Wraps a raw JNI method ID with its expected kind and signature.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid method ID for a class in this VM, and `kind`/`signature` must describe
    /// that method accurately.
    pub unsafe fn method_from_raw(
        &self,
        raw: jni::jmethodID,
        kind: MethodKind,
        signature: MethodSignature,
    ) -> Result<MethodId> {
        if raw.is_null() {
            Err(Error::NullReturn {
                operation: "JNI method ID",
            })
        } else {
            Ok(MethodId {
                raw,
                kind,
                signature,
            })
        }
    }

    /// Wraps a raw JNI field ID with its expected kind and type.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid field ID for a class in this VM, and `kind`/`ty` must describe that
    /// field accurately.
    pub unsafe fn field_from_raw(
        &self,
        raw: jni::jfieldID,
        kind: FieldKind,
        ty: JavaType,
    ) -> Result<FieldId> {
        if raw.is_null() {
            Err(Error::NullReturn {
                operation: "JNI field ID",
            })
        } else {
            Ok(FieldId { raw, kind, ty })
        }
    }

    /// Converts a reflected Java method or constructor object into a JNI method ID.
    ///
    /// # Safety
    ///
    /// `method` must be a valid `java.lang.reflect.Method` or `java.lang.reflect.Constructor`
    /// object for this VM, and `kind`/`signature` must accurately describe that reflected member.
    /// Supplying forged metadata creates a low-level ID wrapper whose later checked calls validate
    /// against the forged metadata rather than the real ART member.
    pub unsafe fn from_reflected_method(
        &self,
        method: &impl AsJObject,
        kind: MethodKind,
        signature: MethodSignature,
    ) -> Result<MethodId> {
        let from_reflected_method =
            self.function::<jni::FromReflectedMethod>(jni::ENV_FROM_REFLECTED_METHOD);
        let raw = unsafe { from_reflected_method(self.handle.as_ptr(), method.as_jobject()) };
        self.check_pending_exception("JNIEnv::FromReflectedMethod")?;
        unsafe { self.method_from_raw(raw, kind, signature) }
    }

    /// Converts a reflected Java field object into a JNI field ID.
    ///
    /// # Safety
    ///
    /// `field` must be a valid `java.lang.reflect.Field` object for this VM, and `kind`/`ty` must
    /// accurately describe that reflected field. Supplying forged metadata creates a low-level ID
    /// wrapper whose later checked field helpers validate against the forged metadata rather than
    /// the real ART field.
    pub unsafe fn from_reflected_field(
        &self,
        field: &impl AsJObject,
        kind: FieldKind,
        ty: JavaType,
    ) -> Result<FieldId> {
        let from_reflected_field =
            self.function::<jni::FromReflectedField>(jni::ENV_FROM_REFLECTED_FIELD);
        let raw = unsafe { from_reflected_field(self.handle.as_ptr(), field.as_jobject()) };
        self.check_pending_exception("JNIEnv::FromReflectedField")?;
        unsafe { self.field_from_raw(raw, kind, ty) }
    }

    /// Converts a detached JNI method ID into a reflected Java method or constructor object.
    ///
    /// # Safety
    ///
    /// `method` must have been resolved from `class` in this VM, and `class` must still name the
    /// declaring class identity expected by that JNI ID.
    pub unsafe fn to_reflected_method(
        &self,
        class: &impl AsJClass,
        method: &MethodId,
    ) -> Result<ObjectRef<'_>> {
        let to_reflected_method =
            self.function::<jni::ToReflectedMethod>(jni::ENV_TO_REFLECTED_METHOD);
        let is_static = if method.kind == MethodKind::Static {
            jni::JNI_TRUE
        } else {
            jni::JNI_FALSE
        };
        let reflected = unsafe {
            to_reflected_method(
                self.handle.as_ptr(),
                class.as_jclass(),
                method.raw,
                is_static,
            )
        };
        self.check_pending_exception("JNIEnv::ToReflectedMethod")?;
        unsafe { LocalRef::from_raw(self, reflected) }
    }

    /// Converts a detached JNI field ID into a reflected Java field object.
    ///
    /// # Safety
    ///
    /// `field` must have been resolved from `class` in this VM, and `class` must still name the
    /// declaring class identity expected by that JNI ID.
    pub unsafe fn to_reflected_field(
        &self,
        class: &impl AsJClass,
        field: &FieldId,
    ) -> Result<ObjectRef<'_>> {
        let to_reflected_field =
            self.function::<jni::ToReflectedField>(jni::ENV_TO_REFLECTED_FIELD);
        let is_static = if field.kind == FieldKind::Static {
            jni::JNI_TRUE
        } else {
            jni::JNI_FALSE
        };
        let reflected = unsafe {
            to_reflected_field(
                self.handle.as_ptr(),
                class.as_jclass(),
                field.raw,
                is_static,
            )
        };
        self.check_pending_exception("JNIEnv::ToReflectedField")?;
        unsafe { LocalRef::from_raw(self, reflected) }
    }

    pub fn lookup_constructor(&self, class: &impl AsJClass, signature: &str) -> Result<MethodId> {
        let signature = MethodSignature::parse(signature)?;
        if signature.return_type() != &JavaType::Void {
            return Err(Error::InvalidReturnType {
                operation: "JNIEnv::GetMethodID(<init>)",
                expected: "void",
                actual: signature.return_type().to_string(),
            });
        }

        let raw = self.get_method_id_raw(class.as_jclass(), "<init>", &signature)?;
        Ok(MethodId {
            raw,
            kind: MethodKind::Constructor,
            signature,
        })
    }

    pub fn lookup_static_method(
        &self,
        class: &impl AsJClass,
        name: &str,
        signature: &str,
    ) -> Result<MethodId> {
        let signature = MethodSignature::parse(signature)?;
        let raw = self.get_static_method_id_raw(class.as_jclass(), name, &signature)?;
        Ok(MethodId {
            raw,
            kind: MethodKind::Static,
            signature,
        })
    }

    pub fn lookup_instance_field(
        &self,
        class: &impl AsJClass,
        name: &str,
        ty: &str,
    ) -> Result<FieldId> {
        let ty = JavaType::parse(ty)?;
        let raw = self.get_field_id_raw(class.as_jclass(), name, &ty)?;
        Ok(FieldId {
            raw,
            kind: FieldKind::Instance,
            ty,
        })
    }

    pub fn lookup_static_field(
        &self,
        class: &impl AsJClass,
        name: &str,
        ty: &str,
    ) -> Result<FieldId> {
        let ty = JavaType::parse(ty)?;
        let raw = self.get_static_field_id_raw(class.as_jclass(), name, &ty)?;
        Ok(FieldId {
            raw,
            kind: FieldKind::Static,
            ty,
        })
    }

    fn get_method_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        signature: &MethodSignature,
    ) -> Result<jni::jmethodID> {
        let name = CString::new(name)?;
        let signature = CString::new(signature.to_string())?;
        let get_method_id = self.function::<jni::GetMethodId>(jni::ENV_GET_METHOD_ID);
        let method = unsafe {
            get_method_id(
                self.handle.as_ptr(),
                class,
                name.as_ptr(),
                signature.as_ptr(),
            )
        };
        self.check_pending_exception("JNIEnv::GetMethodID")?;
        if method.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetMethodID",
            })
        } else {
            Ok(method)
        }
    }

    fn get_static_method_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        signature: &MethodSignature,
    ) -> Result<jni::jmethodID> {
        let name = CString::new(name)?;
        let signature = CString::new(signature.to_string())?;
        let get_static_method_id =
            self.function::<jni::GetStaticMethodId>(jni::ENV_GET_STATIC_METHOD_ID);
        let method = unsafe {
            get_static_method_id(
                self.handle.as_ptr(),
                class,
                name.as_ptr(),
                signature.as_ptr(),
            )
        };
        self.check_pending_exception("JNIEnv::GetStaticMethodID")?;
        if method.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetStaticMethodID",
            })
        } else {
            Ok(method)
        }
    }

    fn get_field_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        ty: &JavaType,
    ) -> Result<jni::jfieldID> {
        let name = CString::new(name)?;
        let ty = CString::new(ty.to_string())?;
        let get_field_id = self.function::<jni::GetFieldId>(jni::ENV_GET_FIELD_ID);
        let field =
            unsafe { get_field_id(self.handle.as_ptr(), class, name.as_ptr(), ty.as_ptr()) };
        self.check_pending_exception("JNIEnv::GetFieldID")?;
        if field.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetFieldID",
            })
        } else {
            Ok(field)
        }
    }

    fn get_static_field_id_raw(
        &self,
        class: jni::jclass,
        name: &str,
        ty: &JavaType,
    ) -> Result<jni::jfieldID> {
        let name = CString::new(name)?;
        let ty = CString::new(ty.to_string())?;
        let get_static_field_id =
            self.function::<jni::GetStaticFieldId>(jni::ENV_GET_STATIC_FIELD_ID);
        let field =
            unsafe { get_static_field_id(self.handle.as_ptr(), class, name.as_ptr(), ty.as_ptr()) };
        self.check_pending_exception("JNIEnv::GetStaticFieldID")?;
        if field.is_null() {
            Err(Error::NullReturn {
                operation: "JNIEnv::GetStaticFieldID",
            })
        } else {
            Ok(field)
        }
    }
}
