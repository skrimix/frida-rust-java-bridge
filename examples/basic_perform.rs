#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, Result};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;

    java.perform(|java| {
        let integer = java.use_class("java.lang.Integer")?;
        let value: i32 = integer.call_with("parseInt", ["java.lang.String"], "42")?;
        println!("parsed value = {value}");

        let string = java.use_class("java.lang.String")?;
        let text: String = string.call_with("valueOf", ["int"], 42)?;
        println!("converted Java string = {text}");
        Ok(())
    })?;

    Ok(())
}
