use super::*;

impl fmt::Display for raw::Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl fmt::Debug for raw::Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Class")
            .field("name", &self.name())
            .field("class", &self.as_jclass())
            .finish()
    }
}

impl fmt::Display for JavaClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.class, f)
    }
}

impl fmt::Debug for JavaClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaClass")
            .field("class", &self.class)
            .finish()
    }
}

impl JavaClass {
    pub fn java_display(&self) -> String {
        format!("<class: {}>", self.name())
    }
}

impl fmt::Display for JavaConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "function {}.<init>{}",
            self.class.name(),
            self.signature()
        )
    }
}

impl fmt::Debug for JavaConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaConstructor")
            .field("class", &self.class.name())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl JavaConstructor {
    pub fn java_display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for JavaMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "function {}.{}{}",
            self.class.name(),
            self.name(),
            self.signature()
        )
    }
}

impl fmt::Debug for JavaMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaMethod")
            .field("class", &self.class.name())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl JavaMethod {
    pub fn java_display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for JavaField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "field {}.{}: {}",
            self.class.name(),
            self.name(),
            self.ty()
        )
    }
}

impl fmt::Debug for JavaField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaField")
            .field("class", &self.class.name())
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl JavaField {
    pub fn java_display(&self) -> String {
        self.to_string()
    }
}

impl fmt::Debug for JavaBoundObject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundObject")
            .field("class", &self.class)
            .field("object", &self.object.as_jobject())
            .finish()
    }
}

impl fmt::Debug for JavaBoundMethodOverload<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundMethodOverload")
            .field("object", &self.object.as_jobject())
            .field("overload", &self.overload)
            .finish()
    }
}

impl fmt::Debug for JavaBoundFieldHandle<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JavaBoundFieldHandle")
            .field("object", &self.object.as_jobject())
            .field("field", &self.field)
            .finish()
    }
}

impl JavaReturn<JavaObject, JavaArray> {
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

    type OwnedReturn = JavaReturn<JavaObject, JavaArray>;

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
        assert_eq!(OwnedReturn::Void.java_display(), Ok("void".to_owned()));
        assert_eq!(
            OwnedReturn::Boolean(true).java_display(),
            Ok("true".to_owned())
        );
        assert_eq!(OwnedReturn::Byte(-7).java_display(), Ok("-7".to_owned()));
        assert_eq!(
            OwnedReturn::Char('A' as jni::jchar).java_display(),
            Ok("A".to_owned())
        );
        assert_eq!(
            OwnedReturn::Short(-300).java_display(),
            Ok("-300".to_owned())
        );
        assert_eq!(OwnedReturn::Int(42).java_display(), Ok("42".to_owned()));
        assert_eq!(
            OwnedReturn::Long(9001).java_display(),
            Ok("9001".to_owned())
        );
        assert_eq!(OwnedReturn::Float(1.5).java_display(), Ok("1.5".to_owned()));
        assert_eq!(
            OwnedReturn::Double(2.5).java_display(),
            Ok("2.5".to_owned())
        );
        assert_eq!(
            OwnedReturn::Object(None).java_display(),
            Ok("null".to_owned())
        );
        assert_eq!(
            OwnedReturn::Array(None).java_display(),
            Ok("null".to_owned())
        );
    }

    #[test]
    fn displays_wrapper_metadata_summaries() {
        let class = JavaClass::from_raw(test_class());
        assert_eq!(
            class.class.to_string(),
            "frida.java.bridge.rs.test.TestSubject"
        );
        assert_eq!(class.to_string(), "frida.java.bridge.rs.test.TestSubject");
        assert_eq!(
            class.java_display(),
            "<class: frida.java.bridge.rs.test.TestSubject>"
        );
        assert!(format!("{class:?}").contains("frida.java.bridge.rs.test.TestSubject"));

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
            constructor.to_string(),
            "function frida.java.bridge.rs.test.TestSubject.<init>(I)V"
        );
        assert!(format!("{constructor:?}").contains("JavaConstructor"));
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
            method.to_string(),
            "function frida.java.bridge.rs.test.TestSubject.answer()I"
        );
        assert!(format!("{method:?}").contains("JavaMethod"));
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
            field.to_string(),
            "field frida.java.bridge.rs.test.TestSubject.number: I"
        );
        assert!(format!("{field:?}").contains("JavaField"));
        assert_eq!(
            field.java_display(),
            "field frida.java.bridge.rs.test.TestSubject.number: I"
        );

        let object =
            unsafe { JavaRef::from_global_raw(Vm::dangling_for_tests(), std::ptr::dangling_mut()) }
                .unwrap();
        let bound_object = JavaBoundObject {
            class,
            object: &object,
        };
        assert!(format!("{bound_object:?}").contains("JavaBoundObject"));

        let bound_method = JavaBoundMethodOverload {
            object: &object,
            overload: method,
        };
        assert!(format!("{bound_method:?}").contains("JavaBoundMethodOverload"));

        let bound_field = JavaBoundFieldHandle {
            object: &object,
            field,
        };
        assert!(format!("{bound_field:?}").contains("JavaBoundFieldHandle"));
    }
}
