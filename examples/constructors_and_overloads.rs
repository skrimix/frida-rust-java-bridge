#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, JavaObject, Result, jni};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;

    java.perform(|java| {
        let string = java.use_class("java.lang.String")?;

        let text =
            string.new_object_with(["java.lang.String"], "Hello from frida-rust-java-bridge")?;
        let length: i32 = text.call("length", ())?;
        println!("text length = {length}");

        let charset = java.use_class("java.nio.charset.Charset")?;
        let default_charset: JavaObject = charset.call("defaultCharset", ())?;

        let bytes = b"Rust bytes converted to a Java string"
            .iter()
            .map(|byte| *byte as jni::jbyte)
            .collect::<Vec<_>>();
        let byte_array = java.new_byte_array(&bytes)?;

        let from_bytes = string
            .constructor(["byte[]", "java.nio.charset.Charset"])?
            .new_object((&byte_array, &default_charset))?;
        println!("from bytes = {}", from_bytes.java_to_string()?);

        Ok(())
    })?;

    Ok(())
}
