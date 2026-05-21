use std::fmt;

use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JavaType {
    Boolean,
    Byte,
    Char,
    Short,
    Int,
    Long,
    Float,
    Double,
    Void,
    Object(String),
    Array(Box<JavaType>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodSignature {
    arguments: Vec<JavaType>,
    return_type: JavaType,
}

impl JavaType {
    pub fn parse(descriptor: &str) -> Result<Self> {
        let mut parser = Parser::new(descriptor);
        let ty = parser.parse_type(false)?;
        if parser.is_finished() {
            Ok(ty)
        } else {
            Err(parser.error("trailing characters after type descriptor"))
        }
    }

    /// Parses an overload argument type from either a JNI descriptor or Java source-style name.
    ///
    /// Accepted examples include `I`, `Ljava/lang/String;`, `[I`, `int`,
    /// `java.lang.String`, and `java.lang.String[]`. `void` is rejected because this parser is
    /// intended for argument and field-like type positions.
    pub fn from_name(name: &str) -> Result<Self> {
        match Self::parse(name) {
            Ok(ty) => reject_void_argument_type(normalize_object_type(ty), name),
            Err(descriptor_error) => {
                if looks_like_descriptor(name) {
                    return Err(descriptor_error);
                }
                let pretty = Self::from_pretty_name(name);
                match pretty {
                    Ok(ty) => reject_void_argument_type(ty, name),
                    Err(_) => Err(descriptor_error),
                }
            }
        }
    }

    pub fn jni_return_name(&self) -> &'static str {
        match self {
            Self::Boolean => "boolean",
            Self::Byte => "byte",
            Self::Char => "char",
            Self::Short => "short",
            Self::Int => "int",
            Self::Long => "long",
            Self::Float => "float",
            Self::Double => "double",
            Self::Void => "void",
            Self::Object(_) | Self::Array(_) => "object",
        }
    }
}

fn looks_like_descriptor(name: &str) -> bool {
    matches!(
        name.as_bytes().first(),
        Some(b'Z' | b'B' | b'C' | b'S' | b'I' | b'J' | b'F' | b'D' | b'V' | b'L' | b'[')
    )
}

fn reject_void_argument_type(ty: JavaType, source: &str) -> Result<JavaType> {
    if matches!(ty, JavaType::Void) {
        Err(Error::InvalidSignature {
            signature: source.to_owned(),
            offset: 0,
            message: "void is not valid in this position",
        })
    } else {
        Ok(ty)
    }
}

fn normalize_object_type(ty: JavaType) -> JavaType {
    match ty {
        JavaType::Object(name) => JavaType::Object(name.replace('.', "/")),
        JavaType::Array(element) => JavaType::Array(Box::new(normalize_object_type(*element))),
        ty => ty,
    }
}

impl MethodSignature {
    pub fn parse(descriptor: &str) -> Result<Self> {
        let mut parser = Parser::new(descriptor);
        parser.expect(b'(')?;

        let mut arguments = Vec::new();
        while parser.peek() != Some(b')') {
            if parser.is_finished() {
                return Err(parser.error("method descriptor is missing ')'"));
            }
            arguments.push(parser.parse_type(false)?);
        }
        parser.expect(b')')?;

        let return_type = parser.parse_type(true)?;
        if !parser.is_finished() {
            return Err(parser.error("trailing characters after method descriptor"));
        }

        Ok(Self {
            arguments,
            return_type,
        })
    }

    pub fn arguments(&self) -> &[JavaType] {
        &self.arguments
    }

    pub fn return_type(&self) -> &JavaType {
        &self.return_type
    }

    pub fn new(arguments: Vec<JavaType>, return_type: JavaType) -> Self {
        Self {
            arguments,
            return_type,
        }
    }

    pub(crate) fn from_pretty_types(return_type: &str, arguments: &str) -> Result<Self> {
        let arguments = if arguments.trim().is_empty() {
            Vec::new()
        } else {
            arguments
                .split(',')
                .map(|argument| JavaType::from_pretty_name(argument.trim()))
                .collect::<Result<Vec<_>>>()?
        };
        let return_type = JavaType::from_pretty_name(return_type.trim())?;
        Ok(Self {
            arguments,
            return_type,
        })
    }

    pub(crate) fn validate_arguments(&self, args: &[crate::value::JavaValue]) -> Result<()> {
        if self.arguments.len() != args.len() {
            return Err(Error::InvalidArguments {
                expected: self.arguments.len(),
                actual: args.len(),
            });
        }

        for (index, (expected, actual)) in self.arguments.iter().zip(args).enumerate() {
            if !actual.matches_type(expected) {
                return Err(Error::InvalidArgumentType {
                    index,
                    expected: expected.to_string(),
                    actual: actual.type_name(),
                });
            }
        }

        Ok(())
    }
}

impl fmt::Display for JavaType {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Boolean => fmt.write_str("Z"),
            Self::Byte => fmt.write_str("B"),
            Self::Char => fmt.write_str("C"),
            Self::Short => fmt.write_str("S"),
            Self::Int => fmt.write_str("I"),
            Self::Long => fmt.write_str("J"),
            Self::Float => fmt.write_str("F"),
            Self::Double => fmt.write_str("D"),
            Self::Void => fmt.write_str("V"),
            Self::Object(name) => write!(fmt, "L{name};"),
            Self::Array(element) => write!(fmt, "[{element}"),
        }
    }
}

impl fmt::Display for MethodSignature {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str("(")?;
        for argument in &self.arguments {
            write!(fmt, "{argument}")?;
        }
        write!(fmt, "){}", self.return_type)
    }
}

impl TryFrom<&str> for MethodSignature {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl TryFrom<&str> for JavaType {
    type Error = Error;

    fn try_from(value: &str) -> Result<Self> {
        Self::parse(value)
    }
}

impl From<&JavaType> for String {
    fn from(value: &JavaType) -> Self {
        value.to_string()
    }
}

impl From<&MethodSignature> for String {
    fn from(value: &MethodSignature) -> Self {
        value.to_string()
    }
}

impl JavaType {
    pub(crate) fn is_reference(&self) -> bool {
        matches!(self, Self::Object(_) | Self::Array(_))
    }

    pub(crate) fn from_pretty_name(name: &str) -> Result<Self> {
        let mut element = name.trim();
        let mut dimensions = 0;
        while let Some(stripped) = element.strip_suffix("[]") {
            dimensions += 1;
            element = stripped.trim_end();
        }

        let mut ty = match element {
            "boolean" => Self::Boolean,
            "byte" => Self::Byte,
            "char" => Self::Char,
            "short" => Self::Short,
            "int" => Self::Int,
            "long" => Self::Long,
            "float" => Self::Float,
            "double" => Self::Double,
            "void" if dimensions == 0 => Self::Void,
            "void" => {
                return Err(Error::InvalidSignature {
                    signature: name.to_owned(),
                    offset: 0,
                    message: "array element cannot be void",
                });
            }
            _ if element.is_empty() => {
                return Err(Error::InvalidSignature {
                    signature: name.to_owned(),
                    offset: 0,
                    message: "expected type descriptor",
                });
            }
            _ => Self::Object(element.replace('.', "/")),
        };

        for _ in 0..dimensions {
            if matches!(ty, Self::Void) {
                return Err(Error::InvalidSignature {
                    signature: name.to_owned(),
                    offset: 0,
                    message: "array element cannot be void",
                });
            }
            ty = Self::Array(Box::new(ty));
        }

        Ok(ty)
    }
}

struct Parser<'a> {
    descriptor: &'a str,
    offset: usize,
}

impl<'a> Parser<'a> {
    fn new(descriptor: &'a str) -> Self {
        Self {
            descriptor,
            offset: 0,
        }
    }

    fn parse_type(&mut self, allow_void: bool) -> Result<JavaType> {
        let Some(byte) = self.next() else {
            return Err(self.error("expected type descriptor"));
        };

        match byte {
            b'Z' => Ok(JavaType::Boolean),
            b'B' => Ok(JavaType::Byte),
            b'C' => Ok(JavaType::Char),
            b'S' => Ok(JavaType::Short),
            b'I' => Ok(JavaType::Int),
            b'J' => Ok(JavaType::Long),
            b'F' => Ok(JavaType::Float),
            b'D' => Ok(JavaType::Double),
            b'V' if allow_void => Ok(JavaType::Void),
            b'V' => Err(self.error("void is not valid in this position")),
            b'L' => self.parse_object(),
            b'[' => {
                let element = self.parse_type(false)?;
                if matches!(element, JavaType::Void) {
                    return Err(self.error("array element cannot be void"));
                }
                Ok(JavaType::Array(Box::new(element)))
            }
            _ => Err(self.error("unknown type descriptor")),
        }
    }

    fn parse_object(&mut self) -> Result<JavaType> {
        let start = self.offset;
        while let Some(byte) = self.peek() {
            if byte == b';' {
                let name = &self.descriptor[start..self.offset];
                self.offset += 1;
                if name.is_empty() {
                    return Err(self.error("object type is missing a class name"));
                }
                return Ok(JavaType::Object(name.to_owned()));
            }
            self.offset += 1;
        }

        Err(self.error("object type is missing ';'"))
    }

    fn expect(&mut self, expected: u8) -> Result<()> {
        match self.next() {
            Some(actual) if actual == expected => Ok(()),
            _ => Err(self.error("unexpected method descriptor character")),
        }
    }

    fn next(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.offset += 1;
        Some(byte)
    }

    fn peek(&self) -> Option<u8> {
        self.descriptor.as_bytes().get(self.offset).copied()
    }

    fn is_finished(&self) -> bool {
        self.offset == self.descriptor.len()
    }

    fn error(&self, message: &'static str) -> Error {
        Error::InvalidSignature {
            signature: self.descriptor.to_owned(),
            offset: self.offset,
            message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_primitive_and_reference_types() {
        assert_eq!(JavaType::parse("I").unwrap(), JavaType::Int);
        assert_eq!(
            JavaType::parse("Ljava/lang/String;").unwrap(),
            JavaType::Object("java/lang/String".to_owned())
        );
        assert_eq!(
            JavaType::parse("[[I").unwrap(),
            JavaType::Array(Box::new(JavaType::Array(Box::new(JavaType::Int))))
        );
    }

    #[test]
    fn formats_types_after_parsing() {
        for descriptor in ["Z", "B", "C", "S", "I", "J", "F", "D"] {
            assert_eq!(JavaType::parse(descriptor).unwrap().to_string(), descriptor);
        }

        assert_eq!(
            JavaType::parse("[Ljava/lang/String;").unwrap().to_string(),
            "[Ljava/lang/String;"
        );
    }

    #[test]
    fn parses_method_signatures() {
        let signature = MethodSignature::parse("(Ljava/lang/String;I)Z").unwrap();
        assert_eq!(
            signature.arguments(),
            &[
                JavaType::Object("java/lang/String".to_owned()),
                JavaType::Int
            ]
        );
        assert_eq!(signature.return_type(), &JavaType::Boolean);
        assert_eq!(signature.to_string(), "(Ljava/lang/String;I)Z");
    }

    #[test]
    fn parses_constructed_method_signatures() {
        let signature = MethodSignature::new(
            vec![
                JavaType::Array(Box::new(JavaType::Object("java/lang/String".to_owned()))),
                JavaType::Long,
            ],
            JavaType::Void,
        );

        assert_eq!(signature.to_string(), "([Ljava/lang/String;J)V");
    }

    #[test]
    fn parses_pretty_method_types() {
        assert_eq!(
            JavaType::from_pretty_name("java.lang.String[][]").unwrap(),
            JavaType::Array(Box::new(JavaType::Array(Box::new(JavaType::Object(
                "java/lang/String".to_owned()
            )))))
        );
        assert_eq!(
            MethodSignature::from_pretty_types("java.lang.String", "int, java.lang.Object[]")
                .unwrap()
                .to_string(),
            "(I[Ljava/lang/Object;)Ljava/lang/String;"
        );
    }

    #[test]
    fn parses_type_names_for_overload_arguments() {
        assert_eq!(JavaType::from_name("I").unwrap(), JavaType::Int);
        assert_eq!(JavaType::from_name("int").unwrap(), JavaType::Int);
        assert_eq!(
            JavaType::from_name("Ljava/lang/String;").unwrap(),
            JavaType::Object("java/lang/String".to_owned())
        );
        assert_eq!(
            JavaType::from_name("Ljava.lang.String;").unwrap(),
            JavaType::Object("java/lang/String".to_owned())
        );
        assert_eq!(
            JavaType::from_name("java.lang.String[]").unwrap(),
            JavaType::Array(Box::new(JavaType::Object("java/lang/String".to_owned())))
        );
        assert_eq!(
            JavaType::from_name("[Ljava.lang.String;").unwrap(),
            JavaType::Array(Box::new(JavaType::Object("java/lang/String".to_owned())))
        );
    }

    #[test]
    fn rejects_void_type_names_for_overload_arguments() {
        assert!(JavaType::from_name("void").is_err());
        assert!(JavaType::from_name("V").is_err());
    }

    #[test]
    fn rejects_void_arguments_and_missing_object_end() {
        assert!(JavaType::parse("V").is_err());
        assert!(MethodSignature::parse("(V)V").is_err());
        assert!(JavaType::parse("Ljava/lang/String").is_err());
    }

    #[test]
    fn rejects_malformed_reference_descriptors() {
        assert!(JavaType::parse("L;").is_err());
        assert!(JavaType::parse("[").is_err());
        assert!(MethodSignature::parse("(I").is_err());
        assert!(MethodSignature::parse("I)V").is_err());
    }

    #[test]
    fn rejects_trailing_characters() {
        assert!(JavaType::parse("II").is_err());
        assert!(MethodSignature::parse("()VZ").is_err());
    }

    #[test]
    fn validates_method_arguments() {
        let signature = MethodSignature::parse("(ILjava/lang/String;[I)V").unwrap();
        assert_eq!(
            signature.validate_arguments(&[
                crate::value::JavaValue::Int(7),
                crate::value::JavaValue::Null,
                unsafe { crate::value::JavaValue::object_raw(std::ptr::null_mut()) },
            ]),
            Ok(())
        );
    }

    #[test]
    fn reports_argument_count_and_type_errors() {
        let signature = MethodSignature::parse("(IJ)V").unwrap();

        assert_eq!(
            signature.validate_arguments(&[crate::value::JavaValue::Int(7)]),
            Err(Error::InvalidArguments {
                expected: 2,
                actual: 1,
            })
        );
        assert_eq!(
            signature.validate_arguments(&[
                crate::value::JavaValue::Int(7),
                crate::value::JavaValue::Int(9),
            ]),
            Err(Error::InvalidArgumentType {
                index: 1,
                expected: "J".to_owned(),
                actual: "int",
            })
        );
    }
}
