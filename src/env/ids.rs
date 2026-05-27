use super::*;

/// Kind of Java callable represented by a method ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodKind {
    /// Java constructor (`<init>`).
    Constructor,
    /// Instance method that requires a receiver object.
    Instance,
    /// Static method called on a class.
    Static,
}

/// Resolved Java method or constructor ID plus the signature used to resolve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodId {
    pub(super) raw: jni::jmethodID,
    pub(super) kind: MethodKind,
    pub(super) signature: MethodSignature,
}

/// Kind of Java field represented by a field ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// Instance field that requires a receiver object.
    Instance,
    /// Static field stored on a class.
    Static,
}

/// Resolved Java field ID plus the type used to resolve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldId {
    pub(super) raw: jni::jfieldID,
    pub(super) kind: FieldKind,
    pub(super) ty: JavaType,
}

// JNI method and field IDs are VM-stable identifiers tied to their defining class.
unsafe impl Send for MethodId {}
unsafe impl Sync for MethodId {}
unsafe impl Send for FieldId {}
unsafe impl Sync for FieldId {}

impl MethodId {
    /// Returns the raw JNI method ID.
    ///
    /// # Safety
    ///
    /// The caller must only use this ID with the VM/class identity it was resolved from and must
    /// uphold the JNI contract for calls made with it.
    pub unsafe fn raw(&self) -> jni::jmethodID {
        self.raw
    }

    /// Returns whether this ID names a constructor, instance method, or static method.
    pub fn kind(&self) -> MethodKind {
        self.kind
    }

    /// Returns the parsed method signature used to resolve this ID.
    pub fn signature(&self) -> &MethodSignature {
        &self.signature
    }

    pub(super) fn ensure_kind(&self, expected: MethodKind, operation: &'static str) -> Result<()> {
        if self.kind == expected {
            Ok(())
        } else {
            Err(Error::WrongMethodKind { operation })
        }
    }

    pub(super) fn ensure_instance_return(
        &self,
        expected: JavaType,
        operation: &'static str,
    ) -> Result<()> {
        self.ensure_kind(MethodKind::Instance, operation)?;
        self.ensure_return(expected, operation)
    }

    pub(super) fn ensure_static_return(
        &self,
        expected: JavaType,
        operation: &'static str,
    ) -> Result<()> {
        self.ensure_kind(MethodKind::Static, operation)?;
        self.ensure_return(expected, operation)
    }

    fn ensure_return(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        let actual = self.signature.return_type();
        let matches = if expected.is_reference() {
            actual.is_reference()
        } else {
            actual == &expected
        };

        if matches {
            Ok(())
        } else {
            Err(Error::InvalidReturnType {
                operation,
                expected: expected.jni_return_name(),
                actual: actual.to_string(),
            })
        }
    }
}

impl FieldId {
    /// Returns the raw JNI field ID.
    ///
    /// # Safety
    ///
    /// The caller must only use this ID with the VM/class identity it was resolved from and must
    /// uphold the JNI contract for calls made with it.
    pub unsafe fn raw(&self) -> jni::jfieldID {
        self.raw
    }

    /// Returns whether this ID names an instance or static field.
    pub fn kind(&self) -> FieldKind {
        self.kind
    }

    /// Returns the parsed Java field type used to resolve this ID.
    pub fn ty(&self) -> &JavaType {
        &self.ty
    }

    pub(super) fn ensure_kind(&self, expected: FieldKind, operation: &'static str) -> Result<()> {
        if self.kind == expected {
            Ok(())
        } else {
            Err(Error::WrongFieldKind { operation })
        }
    }

    pub(super) fn ensure_instance_type(
        &self,
        expected: JavaType,
        operation: &'static str,
    ) -> Result<()> {
        self.ensure_kind(FieldKind::Instance, operation)?;
        self.ensure_type(expected, operation)
    }

    pub(super) fn ensure_static_type(
        &self,
        expected: JavaType,
        operation: &'static str,
    ) -> Result<()> {
        self.ensure_kind(FieldKind::Static, operation)?;
        self.ensure_type(expected, operation)
    }

    fn ensure_type(&self, expected: JavaType, operation: &'static str) -> Result<()> {
        let matches = if expected.is_reference() {
            self.ty.is_reference()
        } else {
            self.ty == expected
        };

        if matches {
            Ok(())
        } else {
            Err(Error::InvalidFieldType {
                operation,
                expected: expected.jni_return_name(),
                actual: self.ty.to_string(),
            })
        }
    }
}
