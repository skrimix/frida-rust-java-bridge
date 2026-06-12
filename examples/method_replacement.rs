#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, JavaHookSet, JavaLocalObject, PerformResult, Result};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;
    let _hooks = install_string_builder_hooks(&java)?;
    // Store this perform result in agent state. Dropping it restores the installed hooks after the
    // deferred setup callback has run.

    Ok(())
}

#[cfg(target_os = "android")]
fn install_string_builder_hooks(java: &Java) -> Result<PerformResult<JavaHookSet>> {
    java.perform(|java| {
        let string_builder = java.use_class("java.lang.StringBuilder")?;

        let constructor_guard =
            string_builder.replace_constructor(["java.lang.String"], |ctx| {
                let arg = ctx.arg_object(0)?;
                if let Some(arg) = &arg {
                    let preview = arg
                        .java_to_string()?
                        .replace('\n', "")
                        .chars()
                        .take(40)
                        .collect::<String>();
                    println!("new StringBuilder({preview:?})");
                }

                ctx.call_original::<()>(arg.as_ref())?;
                ctx.ret(())
            })?;

        let to_string_guard = string_builder.replace("toString", |ctx| {
            let result: JavaLocalObject = ctx.call_original(())?;
            let preview = result
                .get_string()?
                .replace('\n', "")
                .chars()
                .take(40)
                .collect::<String>();
            println!("StringBuilder.toString() => {preview}");

            ctx.ret(result)
        })?;

        let mut hooks = JavaHookSet::new();
        hooks.push(constructor_guard);
        hooks.push(to_string_guard);

        Ok(hooks)
    })
}
