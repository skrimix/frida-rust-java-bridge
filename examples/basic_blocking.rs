#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, Result};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    use std::time::Duration;

    let java = Java::obtain()?;
    let java = java.wait_for_app_loader(Duration::from_secs(5))?;
    let scope = java.attach()?;

    let integer = scope.use_class("java.lang.Integer")?;
    let value: i32 = integer.call_with("parseInt", ["java.lang.String"], "42")?;
    println!("parsed value = {value}");

    Ok(())
}
