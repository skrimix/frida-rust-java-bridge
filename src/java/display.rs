use super::*;

impl JavaClass {
    pub fn java_display(&self) -> String {
        format!("<class: {}>", self.name())
    }
}

impl JavaConstructor {
    pub fn java_display(&self) -> String {
        format!("function {}.<init>{}", self.class.name(), self.signature())
    }
}

impl JavaMethod {
    pub fn java_display(&self) -> String {
        format!(
            "function {}.{}{}",
            self.class.name(),
            self.name(),
            self.signature()
        )
    }
}

impl JavaField {
    pub fn java_display(&self) -> String {
        format!("field {}.{}: {}", self.class.name(), self.name(), self.ty())
    }
}

impl JavaReturn {
    pub fn java_display(&self) -> Result<String> {
        Ok(match self {
            Self::Void => "void".to_owned(),
            Self::Boolean(value) => value.to_string(),
            Self::Byte(value) => value.to_string(),
            Self::Char(value) => display_java_char(*value),
            Self::Short(value) => value.to_string(),
            Self::Int(value) => value.to_string(),
            Self::Long(value) => value.to_string(),
            Self::Float(value) => value.to_string(),
            Self::Double(value) => value.to_string(),
            Self::Object(Some(value)) => value.java_display()?,
            Self::Object(None) | Self::Array(None) => "null".to_owned(),
            Self::Array(Some(value)) => value.java_display()?,
        })
    }
}

pub(crate) fn display_java_char(value: jni::jchar) -> String {
    char::from_u32(value as u32)
        .map(|value| value.to_string())
        .unwrap_or_else(|| format!("\\u{value:04X}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_class() -> RawJavaClass {
        let raw = std::ptr::dangling_mut();
        let global = unsafe {
            GlobalRef::from_raw(Vm::dangling_for_tests(), raw)
                .expect("dangling non-null class ref should wrap")
        };
        RawJavaClass::from_global(
            Vm::dangling_for_tests(),
            "frida.java.bridge.rs.test.TestSubject".to_owned(),
            global,
        )
    }

    #[test]
    fn displays_java_chars() {
        assert_eq!(display_java_char('A' as jni::jchar), "A");
        assert_eq!(display_java_char(0xD800), "\\uD800");
    }

    #[test]
    fn displays_primitive_and_null_returns() {
        assert_eq!(JavaReturn::Void.java_display(), Ok("void".to_owned()));
        assert_eq!(
            JavaReturn::Boolean(true).java_display(),
            Ok("true".to_owned())
        );
        assert_eq!(JavaReturn::Byte(-7).java_display(), Ok("-7".to_owned()));
        assert_eq!(
            JavaReturn::Char('A' as jni::jchar).java_display(),
            Ok("A".to_owned())
        );
        assert_eq!(
            JavaReturn::Short(-300).java_display(),
            Ok("-300".to_owned())
        );
        assert_eq!(JavaReturn::Int(42).java_display(), Ok("42".to_owned()));
        assert_eq!(JavaReturn::Long(9001).java_display(), Ok("9001".to_owned()));
        assert_eq!(JavaReturn::Float(1.5).java_display(), Ok("1.5".to_owned()));
        assert_eq!(JavaReturn::Double(2.5).java_display(), Ok("2.5".to_owned()));
        assert_eq!(
            JavaReturn::Object(None).java_display(),
            Ok("null".to_owned())
        );
        assert_eq!(
            JavaReturn::Array(None).java_display(),
            Ok("null".to_owned())
        );
    }

    #[test]
    fn displays_wrapper_metadata_summaries() {
        let class = JavaClass::new(test_class());
        assert_eq!(
            class.java_display(),
            "<class: frida.java.bridge.rs.test.TestSubject>"
        );

        let constructor = JavaConstructor {
            class: class.class.clone(),
            metadata: JavaMethodMetadata {
                name: "<init>".to_owned(),
                kind: MethodKind::Constructor,
                signature: MethodSignature::parse("(I)V").unwrap(),
                modifiers: 0,
                id: std::ptr::dangling_mut(),
            },
        };
        assert_eq!(
            constructor.java_display(),
            "function frida.java.bridge.rs.test.TestSubject.<init>(I)V"
        );

        let method = JavaMethod {
            class: class.class.clone(),
            metadata: JavaMethodMetadata {
                name: "answer".to_owned(),
                kind: MethodKind::Static,
                signature: MethodSignature::parse("()I").unwrap(),
                modifiers: 0,
                id: std::ptr::dangling_mut(),
            },
        };
        assert_eq!(
            method.java_display(),
            "function frida.java.bridge.rs.test.TestSubject.answer()I"
        );

        let field = JavaField {
            class: class.class.clone(),
            metadata: JavaFieldMetadata {
                name: "number".to_owned(),
                kind: FieldKind::Instance,
                ty: JavaType::Int,
                modifiers: 0,
                id: std::ptr::dangling_mut(),
            },
        };
        assert_eq!(
            field.java_display(),
            "field frida.java.bridge.rs.test.TestSubject.number: I"
        );
    }
}
