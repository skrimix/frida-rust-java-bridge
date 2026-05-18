#![allow(dead_code)]

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> frida_java_bridge_rs::Result<()> {
    let java = frida_java_bridge_rs::Java::obtain()?;
    let _ = java;
    Ok(())
}

#[cfg(target_os = "android")]
mod ports {
    use frida_java_bridge_rs::{
        Error, Java, JavaObject, JavaReturn, JavaValue, Result, jni,
        replacement::ImplementationGuard,
    };

    fn required_object(value: Option<JavaObject>, operation: &'static str) -> Result<JavaObject> {
        value.ok_or(Error::NullReturn { operation })
    }

    fn nullable_object_arg(value: Option<jni::jobject>) -> JavaValue {
        value.map_or(JavaValue::Null, JavaValue::Object)
    }

    pub fn construct_strings_and_select_overloads(java: &Java) -> Result<()> {
        let string = java.use_class("java.lang.String")?;

        let example_string_1 = string.new_instance(
            ["java.lang.String"],
            ("Hello World, this is an example string in Java.",),
        )?;
        let _len = string
            .overload("length", [])?
            .call_int(&example_string_1, ())?;

        let charset = java.use_class("java.nio.charset.Charset")?;
        let default_charset = required_object(
            charset
                .static_overload("defaultCharset", [])?
                .call_static_object(())?,
            "Charset.defaultCharset",
        )?;

        let bytes = b"This is a Rust string converted to a Java byte array."
            .iter()
            .map(|byte| *byte as jni::jbyte)
            .collect::<Vec<_>>();
        let byte_array = java.new_byte_array(&bytes)?;

        let _example_string_2 = string
            .constructor(["byte[]", "java.nio.charset.Charset"])?
            .new_object((&byte_array, &default_charset))?;

        Ok(())
    }

    pub fn enumerate_loaded_classes(java: &Java) -> Result<Vec<String>> {
        java.enumerate_loaded_classes()?
            .into_iter()
            .map(|class| Ok(class.name().to_owned()))
            .collect()
    }

    pub fn inspect_wrapper_members(java: &Java) -> Result<Vec<String>> {
        let class = java.use_class("com.company.CustomClass")?;
        let mut members = Vec::new();
        for method in class.declared_methods()? {
            members.push(format!("method {}{}", method.name, method.signature));
        }
        for field in class.declared_fields()? {
            members.push(format!("field {}:{}", field.name, field.ty));
        }
        Ok(members)
    }

    pub fn set_global_proxy(java: &Java) -> Result<()> {
        let activity_thread = java.use_class("android.app.ActivityThread")?;
        let context_wrapper = java.use_class("android.content.ContextWrapper")?;
        let context_class = java.use_class("android.content.Context")?;
        let connectivity_manager = java.use_class("android.net.ConnectivityManager")?;
        let proxy_info = java.use_class("android.net.ProxyInfo")?;

        let proxy = proxy_info.new_instance(
            ["java.lang.String", "int", "java.lang.String"],
            ("192.168.1.10", 8080 as jni::jint, ""),
        )?;
        let app = required_object(
            activity_thread
                .static_overload("currentApplication", [])?
                .call_static_object(())?,
            "ActivityThread.currentApplication",
        )?;
        let context = required_object(
            context_wrapper
                .overload("getApplicationContext", [])?
                .call_object(&app, ())?,
            "ContextWrapper.getApplicationContext",
        )?;
        let service = required_object(
            context_class
                .overload("getSystemService", ["java.lang.String"])?
                .call_object(&context, ("connectivity",))?,
            "Context.getSystemService(connectivity)",
        )?;

        let manager = connectivity_manager.cast(&service)?;
        connectivity_manager
            .overload("setGlobalProxy", ["android.net.ProxyInfo"])?
            .call_void(&manager, (&proxy,))?;
        Ok(())
    }

    pub fn get_imei_via_default_constructor(java: &Java) -> Result<Option<String>> {
        let telephony_manager = java.use_class("android.telephony.TelephonyManager")?;
        let manager = telephony_manager.new_instance([], ())?;
        telephony_manager
            .overload("getDeviceId", [])?
            .call_string(&manager, ())
    }

    pub fn show_toast_on_main_thread(java: &Java) -> Result<()> {
        java.schedule_on_main_thread(|java| {
            let activity_thread = java.use_class("android.app.ActivityThread")?;
            let context_wrapper = java.use_class("android.content.ContextWrapper")?;
            let toast = java.use_class("android.widget.Toast")?;

            let app = required_object(
                activity_thread
                    .static_overload("currentApplication", [])?
                    .call_static_object(())?,
                "ActivityThread.currentApplication",
            )?;
            let context = required_object(
                context_wrapper
                    .overload("getApplicationContext", [])?
                    .call_object(&app, ())?,
                "ContextWrapper.getApplicationContext",
            )?;
            let text = java.new_string_utf("Text to Toast here")?;
            let toast_object = required_object(
                toast
                    .static_overload(
                        "makeText",
                        ["android.content.Context", "java.lang.CharSequence", "int"],
                    )?
                    .call_static_object((&context, &text, 0 as jni::jint))?,
                "Toast.makeText",
            )?;
            toast.overload("show", [])?.call_void(&toast_object, ())?;
            Ok(())
        })?;
        Ok(())
    }

    pub unsafe fn hook_on_click(java: &Java) -> Result<ImplementationGuard> {
        let main_activity =
            java.use_class("com.example.seccon2015.rock_paper_scissors.MainActivity")?;
        let on_click = main_activity.overload("onClick", ["android.view.View"])?;
        let guard = unsafe {
            on_click.install_implementation(|invocation| {
                let view: Option<jni::jobject> = invocation.arg(0)?;
                invocation.call_original((nullable_object_arg(view),))?;

                // Ergonomics gap: the callback receiver is only exposed as raw jobject today.
                // There is no public borrowed/retained JavaObject wrapper for invocation.receiver(),
                // so the JS pattern `this.m.value = 0` cannot be written through field handles yet.
                let _receiver = invocation.receiver();

                Ok(())
            })?
        };
        Ok(guard)
    }

    pub unsafe fn hook_activity_wifi_toggle(java: &Java) -> Result<ImplementationGuard> {
        let activity = java.use_class("android.app.Activity")?;
        let _wifi_manager = java.use_class("android.net.wifi.WifiManager")?;
        let on_create = activity.overload("onCreate", ["android.os.Bundle"])?;

        let guard = unsafe {
            on_create.install_implementation(|invocation| {
                let bundle: Option<jni::jobject> = invocation.arg(0)?;

                // Ergonomics gap: `Java.cast(this.getSystemService("wifi"), WifiManager)` needs
                // a public wrapper for the raw receiver before high-level calls can continue.
                let _receiver = invocation.receiver();

                invocation.call_original((nullable_object_arg(bundle),))?;
                Ok(())
            })?
        };
        Ok(guard)
    }

    pub unsafe fn hook_input_stream_read(java: &Java) -> Result<ImplementationGuard> {
        let input_stream = java.use_class("java.io.InputStream")?;
        let read = input_stream.overload("read", ["byte[]"])?;

        let guard = unsafe {
            read.install_implementation(|invocation| {
                let buffer: Option<jni::jobject> = invocation.arg(0)?;
                let retval: jni::jint =
                    invocation.call_original_as((nullable_object_arg(buffer),))?;

                // Ergonomics gap: `[B` arrives as raw jobject. JavaArray has copy-out helpers,
                // but there is no public borrowed JavaArray view for callback arguments yet.
                let _raw_buffer = buffer;

                Ok(retval)
            })?
        };
        Ok(guard)
    }

    pub unsafe fn hook_webview_load_url(java: &Java) -> Result<ImplementationGuard> {
        let webview = java.use_class("android.webkit.WebView")?;
        let load_url = webview.overload("loadUrl", ["java.lang.String"])?;

        let guard = unsafe {
            load_url.install_implementation(|invocation| {
                let url: Option<jni::jobject> = invocation.arg(0)?;

                // Ergonomics gap: raw String arguments cannot currently be borrowed as JavaObject
                // and converted with JavaObject::get_string() inside the replacement callback.
                let _raw_url = url;

                invocation.call_original((nullable_object_arg(url),))?;
                Ok(())
            })?
        };
        Ok(guard)
    }

    pub unsafe fn hook_string_builder_to_string(java: &Java) -> Result<ImplementationGuard> {
        let string_builder = java.use_class("java.lang.StringBuilder")?;

        let _ctor = string_builder.constructor(["java.lang.String"])?;
        // Feature gap: JavaConstructorOverload has no install_implementation facade yet.

        let to_string = string_builder.overload("toString", [])?;
        let guard = unsafe {
            to_string.install_implementation(|invocation| {
                let result: Option<jni::jobject> = invocation.call_original_as(())?;

                // Ergonomics gap: the returned java.lang.String is raw here, so slicing/logging
                // the partial contents needs a public raw-to-wrapper helper or callback-local ref.
                let _raw_result = result;

                Ok(result)
            })?
        };
        Ok(guard)
    }

    pub unsafe fn hook_shared_preferences_puts(
        java: &Java,
    ) -> Result<Vec<ImplementationGuard>> {
        let editor = java.use_class("android.app.SharedPreferencesImpl$EditorImpl")?;
        let specs = [
            ("putString", ["java.lang.String", "java.lang.String"]),
            ("putInt", ["java.lang.String", "int"]),
            ("putFloat", ["java.lang.String", "float"]),
            ("putBoolean", ["java.lang.String", "boolean"]),
            ("putLong", ["java.lang.String", "long"]),
            ("putStringSet", ["java.lang.String", "java.util.Set"]),
        ];

        let mut guards = Vec::new();
        for (name, args) in specs {
            let overload = editor.overload(name, args)?;
            let guard = unsafe {
                overload.install_implementation(|invocation| {
                    let key = invocation.args().first().copied().unwrap_or(JavaValue::Null);
                    let value = invocation.args().get(1).copied().unwrap_or(JavaValue::Null);

                    // Ergonomics gap: dynamic "log any JavaValue nicely" support is missing.
                    // Primitive values are inspectable, but reference values need class-aware
                    // wrappers before Rust can mirror JS's cheap string coercion.
                    let _would_log = (key, value);

                    invocation.call_original(invocation.args())
                })?
            };
            guards.push(guard);
        }
        Ok(guards)
    }

    pub unsafe fn hook_string_equals(java: &Java) -> Result<ImplementationGuard> {
        let string = java.use_class("java.lang.String")?;
        let equals = string.overload("equals", ["java.lang.Object"])?;

        let guard = unsafe {
            equals.install_implementation(|invocation| {
                let obj: Option<jni::jobject> = invocation.arg(0)?;
                let response: bool = invocation.call_original_as((nullable_object_arg(obj),))?;

                // Ergonomics gap: `this.toString()` and `obj.toString()` need receiver/argument
                // wrapping in callback scope, plus a convenient Object.toString helper.
                let _receiver = invocation.receiver();

                Ok(response)
            })?
        };
        Ok(guard)
    }

    pub fn raw_jni_slot_probe(java: &Java) -> Result<()> {
        let env = java.vm().attach_current_thread()?;
        let _env_handle = env.handle();

        // Feature gap: JS can read arbitrary JNI vtable slots from env.handle. Our raw slot
        // constants and env_function helper are crate-private, so there is no supported way to
        // express "RegisterNatives = 215" or "FindClass = 6" probes from user code.
        Ok(())
    }

    pub fn describe_return(value: JavaReturn) -> &'static str {
        match value {
            JavaReturn::Void => "void",
            JavaReturn::Boolean(_) => "boolean",
            JavaReturn::Byte(_) => "byte",
            JavaReturn::Char(_) => "char",
            JavaReturn::Short(_) => "short",
            JavaReturn::Int(_) => "int",
            JavaReturn::Long(_) => "long",
            JavaReturn::Float(_) => "float",
            JavaReturn::Double(_) => "double",
            JavaReturn::Object(_) => "object",
            JavaReturn::Array(_) => "array",
        }
    }
}
