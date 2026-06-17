#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, Result};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;

    java.perform(|java| {
        for class in java.enumerate_loaded_classes()?.into_iter().take(20) {
            println!("{}", class.name());
        }

        let string = java.use_class("java.lang.String")?;
        for method in string.declared_methods()? {
            println!("method {}{}", method.name, method.signature.descriptor());
        }
        for field in string.declared_fields()? {
            println!("field {}:{}", field.name, field.ty.descriptor());
        }

        Ok(())
    })?;

    Ok(())
}
