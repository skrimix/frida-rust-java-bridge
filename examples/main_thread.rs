#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, JavaObject, Result};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;

    java.perform(|java| {
        java.schedule_on_main_thread(|java| {
            let activity_thread = java.use_class("android.app.ActivityThread")?;
            let toast = java.use_class("android.widget.Toast")?;

            let app: JavaObject = activity_thread.call("currentApplication", ())?;
            let context: JavaObject = app.call("getApplicationContext", ())?;
            let message = "Hello from Rust on the Android main thread";
            let toast_object: JavaObject = toast.call("makeText", (&context, message, 0))?;
            toast_object.call::<()>("show", ())?;

            Ok(())
        })?;

        Ok(())
    })?;

    Ok(())
}
