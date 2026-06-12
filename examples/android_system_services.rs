#[cfg(target_os = "android")]
use frida_rust_java_bridge::{Java, JavaHookGuard, JavaObject, Result};

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> Result<()> {
    let java = Java::obtain()?;

    let _on_create_hook = java.perform(|java| {
        set_global_proxy(&java)?;
        disable_wifi_from_activity_hook(&java)
    })?;
    // Store this perform result in agent state. Dropping it restores the Activity.onCreate hook
    // after the deferred setup callback has run.

    Ok(())
}

#[cfg(target_os = "android")]
fn set_global_proxy(java: &Java) -> Result<()> {
    let activity_thread = java.use_class("android.app.ActivityThread")?;
    let connectivity_manager = java.use_class("android.net.ConnectivityManager")?;
    let proxy_info = java.use_class("android.net.ProxyInfo")?;

    let proxy = proxy_info.new_object_with(
        ["java.lang.String", "int", "java.lang.String"],
        ("192.168.1.10", 8080, ""),
    )?;
    let app: JavaObject = activity_thread.call("currentApplication", ())?;
    let context: JavaObject = app.call("getApplicationContext", ())?;
    let service: JavaObject = context.call("getSystemService", "connectivity")?;

    let manager = service.cast(&connectivity_manager)?;
    manager.call::<()>("setGlobalProxy", &proxy)?;

    Ok(())
}

#[cfg(target_os = "android")]
fn disable_wifi_from_activity_hook(java: &Java) -> Result<JavaHookGuard> {
    let activity = java.use_class("android.app.Activity")?;
    let wifi_manager = java.use_class("android.net.wifi.WifiManager")?;

    // Keep the returned guard alive while the Activity.onCreate replacement should stay installed.
    activity.replace_with("onCreate", ["android.os.Bundle"], move |ctx| {
        let bundle = ctx.arg_object(0)?;
        let this = ctx.this_object()?;
        let service: JavaObject = this.call("getSystemService", "wifi")?;
        let manager = service.cast(&wifi_manager)?;
        let enabled: bool = manager.call("isWifiEnabled", ())?;
        println!("wifi enabled = {enabled}");
        manager.call::<()>("setWifiEnabled", false)?;

        ctx.call_original::<()>(bundle.as_ref())?;
        ctx.ret(())
    })
}
