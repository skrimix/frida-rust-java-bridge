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
    fn rejects_void_arguments_and_missing_object_end() {
        assert!(MethodSignature::parse("(V)V").is_err());
        assert!(JavaType::parse("Ljava/lang/String").is_err());
    }

    #[test]
    fn rejects_trailing_characters() {
        assert!(JavaType::parse("II").is_err());
        assert!(MethodSignature::parse("()VZ").is_err());
    }
}
