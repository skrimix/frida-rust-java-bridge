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
        Error, Java, JavaObject, JavaValue, Result, jni,
        replacement::{JavaHookGuard, JavaHookReturn},
    };

    const JS_STRING_CONSTRUCTION_AND_BUILDER_HOOKS: &str = r##"
Java.perform(() => {
  const JavaString = Java.use('java.lang.String');
  const exampleString1 = JavaString.$new('Hello World, this is an example string in Java.');
  console.log('[+] exampleString1: ' + exampleString1);
  console.log('[+] exampleString1.length(): ' + exampleString1.length());

  const Charset = Java.use('java.nio.charset.Charset');
  const charset = Charset.defaultCharset();
  const charArray = 'This is a Javascript string converted to a byte array.'.split('').map(function(c) {
    return c.charCodeAt(0);
  });

  const exampleString2 = JavaString.$new.overload('[B', 'java.nio.charset.Charset').call(JavaString, charArray, charset);
  console.log('[+] exampleString2: ' + exampleString2);
  console.log('[+] exampleString2.length(): ' + exampleString2.length());

  const StringBuilder = Java.use('java.lang.StringBuilder');
  const ctor = StringBuilder.$init.overload('java.lang.String');
  ctor.implementation = function (arg) {
    let partial = '';
    const result = ctor.call(this, arg);
    if (arg !== null) {
      partial = arg.toString().replace('\n', '').slice(0, 10);
    }
    console.log('new StringBuilder("' + partial + '");');
    return result;
  };
  console.log('[+] new StringBuilder(java.lang.String) hooked');

  const toString = StringBuilder.toString;
  toString.implementation = function () {
    const result = toString.call(this);
    let partial = '';
    if (result !== null) {
      partial = result.toString().replace('\n', '').slice(0, 10);
    }
    console.log('StringBuilder.toString(); => ' + partial);
    return result;
  };
  console.log('[+] StringBuilder.toString() hooked');
});
"##;

    pub fn construct_strings_and_select_overloads(java: &Java) -> Result<()> {
        let string = java.use_class("java.lang.String")?;

        let example_string_1 = string.new_instance(
            ["java.lang.String"],
            ("Hello World, this is an example string in Java.",),
        )?;
        let _len = string
            .method("length")?
            .call::<jni::jint>(&example_string_1, ())?;

        let charset = java.use_class("java.nio.charset.Charset")?;
        let default_charset = charset
            .static_method("defaultCharset")?
            .call_static::<JavaObject>(())?;

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

    pub unsafe fn hook_string_builder_constructor_and_to_string(
        java: &Java,
    ) -> Result<Vec<JavaHookGuard>> {
        let string_builder = java.use_class("java.lang.StringBuilder")?;
        let string_constructor = string_builder.constructor(["java.lang.String"])?;
        let to_string = string_builder.method("toString")?;

        let constructor_guard = unsafe {
            string_constructor.replace(|invocation| {
                let _this = invocation.this_object()?.ok_or(Error::NullReturn {
                    operation: "StringBuilder.<init> this",
                })?;
                let arg = invocation.arg_object(0)?;
                if let Some(arg) = &arg {
                    let partial = arg
                        .java_to_string()?
                        .replace('\n', "")
                        .chars()
                        .take(10)
                        .collect::<String>();
                    let _would_log = format!("new StringBuilder(\"{partial}\");");
                }

                invocation.call_original_void((arg.as_ref(),))?;
                Ok(())
            })?
        };

        let to_string_guard = unsafe {
            to_string.replace(|invocation| {
                let result = invocation.call_original_object(())?;
                if let Some(result) = &result {
                    let partial = result
                        .get_string()?
                        .replace('\n', "")
                        .chars()
                        .take(10)
                        .collect::<String>();
                    let _would_log = format!("StringBuilder.toString(); => {partial}");
                }

                Ok(JavaHookReturn::object(result.as_ref()))
            })?
        };

        Ok(vec![constructor_guard, to_string_guard])
    }

    const JS_ENUMERATE_LOADED_CLASSES: &str = r##"
Java.perform(function () {
  Java.enumerateLoadedClasses({
    onMatch: function (c) {
      console.log(c);
    },
  });
});
"##;

    pub fn enumerate_loaded_classes(java: &Java) -> Result<Vec<String>> {
        java.enumerate_loaded_classes()?
            .into_iter()
            .map(|class| Ok(class.name().to_owned()))
            .collect()
    }

    const JS_INSPECT_WRAPPER_MEMBERS: &str = r##"
Object.getOwnPropertyNames(Java.use('com.company.CustomClass').__proto__).join('\n\t')
"##;

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

    const JS_SET_GLOBAL_PROXY: &str = r##"
var ActivityThread      = Java.use('android.app.ActivityThread');
var ConnectivityManager = Java.use('android.net.ConnectivityManager');
var ProxyInfo           = Java.use('android.net.ProxyInfo');

var proxyInfo = ProxyInfo.$new('192.168.1.10', 8080, '');
var context = ActivityThread.currentApplication().getApplicationContext();
var connectivityManager = Java.cast(context.getSystemService('connectivity'), ConnectivityManager);
connectivityManager.setGlobalProxy(proxyInfo);
"##;

    pub fn set_global_proxy(java: &Java) -> Result<()> {
        let activity_thread = java.use_class("android.app.ActivityThread")?;
        let connectivity_manager = java.use_class("android.net.ConnectivityManager")?;
        let proxy_info = java.use_class("android.net.ProxyInfo")?;

        let proxy = proxy_info.new_instance(
            ["java.lang.String", "int", "java.lang.String"],
            ("192.168.1.10", 8080, ""),
        )?;
        let app = activity_thread
            .static_method("currentApplication")?
            .call_static::<JavaObject>(())?;
        let context = app
            .method("getApplicationContext")?
            .call::<JavaObject>(())?;
        let service = context
            .method(("getSystemService", ["java.lang.String"]))?
            .call::<JavaObject>(("connectivity",))?;

        let manager = connectivity_manager.cast(&service)?;
        manager.method("setGlobalProxy")?.call::<()>((&proxy,))?;
        Ok(())
    }

    const JS_GET_IMEI_DEFAULT_CONSTRUCTOR: &str = r##"
function getIMEI() {
  console.log('IMEI =', Java.use("android.telephony.TelephonyManager").$new().getDeviceId());
}
Java.perform(getIMEI);
"##;

    pub fn get_imei_via_default_constructor(java: &Java) -> Result<Option<String>> {
        let telephony_manager = java.use_class("android.telephony.TelephonyManager")?;
        let manager = telephony_manager.new_instance([], ())?;
        manager
            .method(("getDeviceId", []))?
            .call::<Option<String>>(())
    }

    const JS_SHOW_TOAST_ON_MAIN_THREAD: &str = r##"
Java.scheduleOnMainThread(() => {
  Java.use("android.widget.Toast")
    .makeText(
      Java.use("android.app.ActivityThread").currentApplication().getApplicationContext(),
      Java.use("java.lang.StringBuilder").$new("Text to Toast here"),
      0
    )
    .show();
});
"##;

    pub fn show_toast_on_main_thread(java: &Java) -> Result<()> {
        java.schedule_on_main_thread(|java| {
            let activity_thread = java.use_class("android.app.ActivityThread")?;
            let toast = java.use_class("android.widget.Toast")?;

            let app = activity_thread
                .static_method("currentApplication")?
                .call_static::<JavaObject>(())?;
            let context = app
                .method("getApplicationContext")?
                .call::<JavaObject>(())?;
            let text = java.new_string_utf("Text to Toast here")?;
            let toast_object = toast
                .static_method((
                    "makeText",
                    ["android.content.Context", "java.lang.CharSequence", "int"],
                ))?
                .call_static::<JavaObject>((&context, &text, 0))?;
            toast_object.method("show")?.call::<()>(())?;
            Ok(())
        })?;
        Ok(())
    }

    const JS_HOOK_ON_CLICK: &str = r##"
const MainActivity = Java.use('com.example.seccon2015.rock_paper_scissors.MainActivity');

const onClick = MainActivity.onClick;
onClick.implementation = function (v) {
  console.log('onClick');
  onClick.call(this, v);

  this.m.value = 0;
  this.n.value = 1;
  this.cnt.value = 999;

  console.log('Done:' + JSON.stringify(this.cnt));
};
"##;

    pub unsafe fn hook_on_click(java: &Java) -> Result<JavaHookGuard> {
        let main_activity =
            java.use_class("com.example.seccon2015.rock_paper_scissors.MainActivity")?;
        let on_click = main_activity.method("onClick")?;
        let m_field = main_activity.field_handle("m")?;
        let n_field = main_activity.field_handle("n")?;
        let cnt_field = main_activity.field_handle("cnt")?;

        let guard = unsafe {
            on_click.replace(move |invocation| {
                let view = invocation.arg_object(0)?;
                invocation.call_original::<()>((view.as_ref(),))?;

                let this = invocation.this_object()?.ok_or(Error::NullReturn {
                    operation: "JavaHookContext::this_object",
                })?;
                m_field.set(&this, 0)?;
                n_field.set(&this, 1)?;
                cnt_field.set(&this, 999)?;
                let cnt = cnt_field.get::<jni::jint>(&this)?;
                let _would_log = format!("Done:{cnt}");

                Ok(())
            })?
        };
        Ok(guard)
    }

    const JS_HOOK_ACTIVITY_WIFI_TOGGLE: &str = r##"
var WifiManager = Java.use("android.net.wifi.WifiManager");
Java.use("android.app.Activity").onCreate.overload("android.os.Bundle").implementation = function (bundle) {
  var wManager = Java.cast(this.getSystemService("wifi"), WifiManager);
  console.log('isWifiEnabled ?', wManager.isWifiEnabled());
  wManager.setWifiEnabled(false);
  this.$init(bundle);
};
"##;

    pub unsafe fn hook_activity_wifi_toggle(java: &Java) -> Result<JavaHookGuard> {
        let activity = java.use_class("android.app.Activity")?;
        let wifi_manager = java.use_class("android.net.wifi.WifiManager")?;
        let wifi_manager_class = wifi_manager.class().clone();
        let get_system_service = activity.method(("getSystemService", ["java.lang.String"]))?;
        let on_create = activity.method(("onCreate", ["android.os.Bundle"]))?;
        let is_wifi_enabled = wifi_manager.method("isWifiEnabled")?;
        let set_wifi_enabled = wifi_manager.method(("setWifiEnabled", ["boolean"]))?;

        let guard = unsafe {
            on_create.replace(move |invocation| {
                let bundle = invocation.arg_object(0)?;
                let this = invocation.this_object()?.ok_or(Error::NullReturn {
                    operation: "JavaHookContext::this_object",
                })?;
                let service = get_system_service.call::<JavaObject>(&this, ("wifi",))?;
                if !wifi_manager_class.is_instance(&service)? {
                    return Err(Error::InvalidObjectType {
                        operation: "Activity.getSystemService(wifi)",
                        expected: "android.net.wifi.WifiManager",
                        actual: service.java_to_string()?,
                    });
                }
                let _enabled = is_wifi_enabled.call::<bool>(&service, ())?;
                set_wifi_enabled.call::<bool>(&service, (false,))?;

                invocation.call_original::<()>((bundle.as_ref(),))?;
                Ok(())
            })?
        };
        Ok(guard)
    }

    const JS_HOOK_INPUT_STREAM_READ: &str = r##"
function binaryToHexToAscii(array, readLimit) {
  var result = [];
  readLimit = readLimit || 100;
  for (var i = 0; i < readLimit; ++i) {
    result.push(String.fromCharCode(parseInt(('0' + (array[i] & 0xFF).toString(16)).slice(-2), 16)));
  }
  return result.join('');
}

function hookInputStream() {
  Java.use('java.io.InputStream')['read'].overload('[B').implementation = function (b) {
    var retval = this.read(b);
    var resp = binaryToHexToAscii(b);
    if (!new RegExp(['Mmm'].join('|')).test(resp)) {
      console.log(resp);
    }
    if (new RegExp(['AAA', 'BBB', 'CCC'].join('|')).test(resp)) {
      send(binaryToHexToAscii(b, 1200));
    }
    return retval;
  };
}

Java.perform(hookInputStream);
"##;

    pub unsafe fn hook_input_stream_read(java: &Java) -> Result<JavaHookGuard> {
        let input_stream = java.use_class("java.io.InputStream")?;
        let read = input_stream.method(("read", ["byte[]"]))?;

        let guard = unsafe {
            read.replace(|invocation| {
                let buffer = invocation.arg_array(0)?;
                let retval: jni::jint = invocation.call_original((buffer.as_ref(),))?;
                if let Some(buffer) = buffer {
                    let bytes = buffer.get_bytes()?;
                    let _preview = String::from_utf8_lossy(
                        &bytes
                            .into_iter()
                            .take(retval.max(0) as usize)
                            .map(|value| value as u8)
                            .collect::<Vec<_>>(),
                    )
                    .into_owned();
                }

                Ok(retval)
            })?
        };
        Ok(guard)
    }

    const JS_HOOK_WEBVIEW_LOAD_URL: &str = r##"
Java.use("android.webkit.WebView").loadUrl.overload("java.lang.String").implementation = function (s) {
  send(s.toString());
  this.loadUrl.overload("java.lang.String").call(this, s);
};
"##;

    pub unsafe fn hook_webview_load_url(java: &Java) -> Result<JavaHookGuard> {
        let webview = java.use_class("android.webkit.WebView")?;
        let load_url = webview.method(("loadUrl", ["java.lang.String"]))?;

        let guard = unsafe {
            load_url.replace(|invocation| {
                let url = invocation.arg_object(0)?;
                let url_text = url.as_ref().map(|url| url.get_string()).transpose()?;
                let _would_send = url_text.as_deref();

                invocation.call_original::<()>((url.as_ref(),))?;
                Ok(())
            })?
        };
        Ok(guard)
    }

    const JS_HOOK_STRING_BUILDER_OR_BUFFER_TOSTRING: &str = r##"
Java.perform(function () {
  ['java.lang.StringBuilder', 'java.lang.StringBuffer'].forEach(function (clazz, i) {
    console.log('[?] ' + i + ' = ' + clazz);
    var func = 'toString';
    Java.use(clazz)[func].implementation = function () {
      var ret = this[func]();
      if (ret.indexOf('') != -1) {
        Java.perform(function () {
          var jAndroidLog = Java.use("android.util.Log");
          var jException = Java.use("java.lang.Exception");
          console.log(jAndroidLog.getStackTraceString(jException.$new()));
        });
      }
      send('[' + i + '] ' + ret);
      return ret;
    };
  });
});
"##;

    pub unsafe fn hook_string_builder_to_string(java: &Java) -> Result<JavaHookGuard> {
        let string_builder = java.use_class("java.lang.StringBuilder")?;
        let to_string = string_builder.method("toString")?;
        let guard = unsafe {
            to_string.replace(|invocation| {
                let result = invocation.call_original_object(())?;
                if let Some(result) = &result {
                    let partial = result
                        .get_string()?
                        .replace('\n', "")
                        .chars()
                        .take(10)
                        .collect::<String>();
                    let _would_log = format!("StringBuilder.toString(); => {partial}");
                }

                Ok(JavaHookReturn::object(result.as_ref()))
            })?
        };
        Ok(guard)
    }

    const JS_HOOK_SHARED_PREFERENCES_PUTS: &str = r##"
Java.perform(function () {
  var shared_pref_class = Java.use('android.app.SharedPreferencesImpl$EditorImpl');

  shared_pref_class.putString.overload('java.lang.String', 'java.lang.String').implementation = function (k, v) {
    console.log('Shared preference updated: ', k, '=', v);
    return this.putString(k, v);
  };

  shared_pref_class.putInt.overload('java.lang.String', 'int').implementation = function (k, v) {
    console.log('Shared preference updated: ', k, '=', v);
    return this.putInt(k, v);
  };

  shared_pref_class.putFloat.overload('java.lang.String', 'float').implementation = function (k, v) {
    console.log('Shared preference updated: ', k, '=', v);
    return this.putFloat(k, v);
  };

  shared_pref_class.putBoolean.overload('java.lang.String', 'boolean').implementation = function (k, v) {
    console.log('Shared preference updated: ', k, '=', v);
    return this.putBoolean(k, v);
  };

  shared_pref_class.putLong.overload('java.lang.String', 'long').implementation = function (k, v) {
    console.log('Shared preference updated: ', k, '=', v);
    return this.putLong(k, v);
  };

  shared_pref_class.putStringSet.overload('java.lang.String', java.util.Set).implementation = function (k, v) {
    console.log('Shared preference updated: ', k, '=', v);
    return this.putStringSet(k, v);
  };
});
"##;

    pub unsafe fn hook_shared_preferences_puts(java: &Java) -> Result<Vec<JavaHookGuard>> {
        let editor = java.use_class("android.app.SharedPreferencesImpl$EditorImpl")?;
        let names = [
            "putString",
            "putInt",
            "putFloat",
            "putBoolean",
            "putLong",
            "putStringSet",
        ];

        let mut guards = Vec::new();
        for name in names {
            let method = editor.method((
                name,
                ["java.lang.String", shared_preference_value_type(name)],
            ))?;
            let guard = unsafe {
                method.replace(move |invocation| {
                    let key = invocation
                        .args()
                        .first()
                        .copied()
                        .unwrap_or(JavaValue::Null);
                    let value = invocation.args().get(1).copied().unwrap_or(JavaValue::Null);
                    let key_text = invocation.arg_string(0)?;
                    let value_text = match value {
                        JavaValue::Object(_) | JavaValue::Null => invocation
                            .arg_object(1)?
                            .map(|object| object.java_to_string())
                            .transpose()?,
                        _ => None,
                    };
                    let _would_log = (key, value, key_text, value_text);

                    invocation.call_original::<frida_java_bridge_rs::replacement::JavaHookReturn>(
                        invocation.args(),
                    )
                })?
            };
            guards.push(guard);
        }
        Ok(guards)
    }

    const JS_HOOK_STRING_EQUALS: &str = r##"
Java.perform(function () {
  var str = Java.use('java.lang.String');
  var objectClass = 'java.lang.Object';
  str.equals.overload(objectClass).implementation = function (obj) {
    var response = str.equals.overload(objectClass).call(this, obj);
    if (obj) {
      if (obj.toString().length > 5) {
        send(str.toString.call(this) + ' == ' + obj.toString() + ' ? ' + response);
      }
    }
    return response;
  };
});
"##;

    pub unsafe fn hook_string_equals(java: &Java) -> Result<JavaHookGuard> {
        let string = java.use_class("java.lang.String")?;
        let equals = string.method(("equals", ["java.lang.Object"]))?;

        let guard = unsafe {
            equals.replace(|invocation| {
                let obj = invocation.arg_object(0)?;
                let response: bool = invocation.call_original((obj.as_ref(),))?;

                let this = invocation.this_object()?.ok_or(Error::NullReturn {
                    operation: "JavaHookContext::this_object",
                })?;
                if let Some(obj) = &obj {
                    let left = this.java_to_string()?;
                    let right = obj.java_to_string()?;
                    let _would_send = format!("{left} == {right} ? {response}");
                }

                Ok(response)
            })?
        };
        Ok(guard)
    }

    const JS_RAW_JNI_SLOT_PROBE: &str = r##"
var pSize = Process.pointerSize;
var env = Java.vm.getEnv();
var RegisterNatives = 215;
var FindClassIndex = 6;
function getNativeAddress(idx) {
  return env.handle.readPointer().add(idx * pSize).readPointer();
}
"##;

    pub fn raw_jni_slot_probe(java: &Java) -> Result<()> {
        let env = java.vm().attach_current_thread()?;
        let _env_handle = env.handle();

        // Feature gap: JS can read arbitrary JNI vtable slots from env.handle. Our raw slot
        // constants and env_function helper are crate-private, so there is no supported way to
        // express "RegisterNatives = 215" or "FindClass = 6" probes from user code.
        Ok(())
    }

    fn shared_preference_value_type(name: &str) -> &'static str {
        match name {
            "putString" => "java.lang.String",
            "putInt" => "int",
            "putFloat" => "float",
            "putBoolean" => "boolean",
            "putLong" => "long",
            "putStringSet" => "java.util.Set",
            _ => "java.lang.Object",
        }
    }
}
