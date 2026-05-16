use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MethodKind {
    Constructor,
    Instance,
    Static,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodId {
    pub(super) raw: jni::jmethodID,
    pub(super) kind: MethodKind,
    pub(super) signature: MethodSignature,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    Instance,
    Static,
}

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
    pub fn raw(&self) -> jni::jmethodID {
        self.raw
    }

    pub fn kind(&self) -> MethodKind {
        self.kind
    }

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
    pub fn raw(&self) -> jni::jfieldID {
        self.raw
    }

    pub fn kind(&self) -> FieldKind {
        self.kind
    }

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
