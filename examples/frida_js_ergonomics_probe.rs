#![allow(dead_code)]

#[cfg(not(target_os = "android"))]
fn main() {}

#[cfg(target_os = "android")]
fn main() -> frida_rust_java_bridge::Result<()> {
    let java = frida_rust_java_bridge::Java::obtain()?;
    let _ = java;
    Ok(())
}

#[cfg(target_os = "android")]
mod ports {
    use std::ffi::c_void;

    use frida_rust_java_bridge::{
        Java, JavaLocalArray, JavaLocalObject, JavaObject, PerformResult, Result, jni,
        replacement::{JavaHookGuard, JavaHookSet},
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
        java.perform(|java| {
            let string = java.use_class("java.lang.String")?;

            let example_string_1 = string.new_with(
                ["java.lang.String"],
                ("Hello World, this is an example string in Java.",),
            )?;
            let _len: i32 = example_string_1.call("length", ())?;

            let charset = java.use_class("java.nio.charset.Charset")?;
            let default_charset: JavaObject = charset.call("defaultCharset", ())?;

            let bytes = b"This is a Rust string converted to a Java byte array."
                .iter()
                .map(|byte| *byte as jni::jbyte)
                .collect::<Vec<_>>();
            let byte_array = java.new_byte_array(&bytes)?;

            let _example_string_2 = string
                .constructor(["byte[]", "java.nio.charset.Charset"])?
                .new_object((&byte_array, &default_charset))?;
            Ok(())
        })?;

        Ok(())
    }

    pub fn hook_string_builder_constructor_and_to_string(
        java: &Java,
    ) -> Result<PerformResult<JavaHookSet>> {
        java.perform(|java| {
            let string_builder = java.use_class("java.lang.StringBuilder")?;
            let constructor_guard =
                string_builder.replace_constructor(["java.lang.String"], |ctx| {
                    let _this = ctx.this_object()?;
                    let arg = ctx.arg_object(0)?;
                    let _typed_arg: Option<JavaLocalObject> = ctx.arg(0)?;
                    if let Some(arg) = &arg {
                        let partial = arg
                            .java_to_string()?
                            .replace('\n', "")
                            .chars()
                            .take(10)
                            .collect::<String>();
                        println!("new StringBuilder(\"{partial}\");");
                    }

                    ctx.call_original(arg.as_ref())
                })?;

            let to_string_guard = string_builder.replace("toString", |ctx| {
                let result: JavaLocalObject = ctx.call_original(())?;
                let partial = result
                    .get_string()?
                    .replace('\n', "")
                    .chars()
                    .take(10)
                    .collect::<String>();
                println!("StringBuilder.toString(); => {partial}");

                ctx.ret(result)
            })?;

            let mut hook_set = JavaHookSet::new();
            hook_set.push(constructor_guard);
            hook_set.push(to_string_guard);
            Ok(hook_set)
        })
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

    pub fn enumerate_loaded_classes(java: &Java) -> Result<PerformResult<()>> {
        java.perform(|java| {
            let classes = java.enumerate_loaded_classes()?;
            for class in classes {
                println!("{class}");
            }
            Ok(())
        })
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

        let proxy = proxy_info.new_with(
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

            let app: JavaObject = activity_thread.call("currentApplication", ())?;
            let context: JavaObject = app.call("getApplicationContext", ())?;
            let toast_object: JavaObject =
                toast.call("makeText", (&context, "Text to Toast here", 0))?;
            toast_object.call::<()>("show", ())?;
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
        let guard = main_activity.replace("onClick", |ctx| {
            let view = ctx.arg_object(0)?;
            ctx.call_original::<()>(view.as_ref())?;

            let this = ctx.this_object()?;
            this.set_field("m", 0)?;
            this.set_field("n", 1)?;
            this.set_field("cnt", 999)?;
            let cnt: i32 = this.get_field("cnt")?;
            println!("Done:{cnt}");

            ctx.ret(())
        })?;
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
        let guard = activity.replace_with("onCreate", ["android.os.Bundle"], move |ctx| {
            let bundle = ctx.arg_object(0)?;
            let this = ctx.this_object()?;
            let service: JavaObject = this.call("getSystemService", "wifi")?;
            let manager = service.cast(&wifi_manager)?;
            let _enabled: bool = manager.call("isWifiEnabled", ())?;
            manager.call::<()>("setWifiEnabled", false)?;

            ctx.call_original::<()>(bundle.as_ref())?;
            ctx.ret(())
        })?;
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
        let guard = input_stream.replace_with("read", ["byte[]"], |ctx| {
            let buffer: JavaLocalArray = ctx.arg(0)?;
            let _buffer_alt = ctx.arg_array(0)?;
            let retval: i32 = ctx.call_original(&buffer)?;
            let bytes = buffer.get_bytes()?;
            let _preview = String::from_utf8_lossy(
                &bytes
                    .into_iter()
                    .take(retval.max(0) as usize)
                    .map(|value| value as u8)
                    .collect::<Vec<_>>(),
            )
            .into_owned();

            ctx.ret(retval)
        })?;
        Ok(guard)
    }

    const JS_HOOK_WEBVIEW_LOAD_URL: &str = r##"
Java.use("android.webkit.WebView").loadUrl.overload("java.lang.String").implementation = function (s) {
  console.log(s.toString());
  this.loadUrl.overload("java.lang.String").call(this, s);
};
"##;

    pub unsafe fn hook_webview_load_url(java: &Java) -> Result<JavaHookGuard> {
        let webview = java.use_class("android.webkit.WebView")?;
        let guard = webview.replace_with("loadUrl", ["java.lang.String"], |ctx| {
            let url: JavaLocalObject = ctx.arg(0)?;
            let url_text = url.get_string()?;
            println!("url_text = {url_text}");

            ctx.call_original_current::<()>()?;
            ctx.ret(())
        })?;
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
        let guard = string_builder.replace("toString", |ctx| {
            let result = ctx.call_original_object(())?;
            if let Some(result) = &result {
                let partial = result
                    .get_string()?
                    .replace('\n', "")
                    .chars()
                    .take(10)
                    .collect::<String>();
                println!("StringBuilder.toString(); => {partial}");
            }

            ctx.ret(result)
        })?;
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

    pub fn hook_shared_preferences_puts(java: &Java) -> Result<JavaHookSet> {
        let editor = java.use_class("android.app.SharedPreferencesImpl$EditorImpl")?;
        let targets = [
            ("putString", "java.lang.String"),
            ("putInt", "int"),
            ("putFloat", "float"),
            ("putBoolean", "boolean"),
            ("putLong", "long"),
            ("putStringSet", "java.util.Set"),
        ];

        let mut guards = JavaHookSet::new();
        for (name, value_type) in targets {
            let guard =
                editor.replace_with(name, ["java.lang.String", value_type], move |ctx| {
                    let key = ctx.arg_display(0)?;
                    let value = ctx.arg_display(1)?;
                    println!("Shared preference updated: {key} = {value}");
                    ctx.proceed()
                })?;
            guards.push(guard);
        }
        Ok(guards)
    }

    const JS_HOOK_REPLACEMENT_ERROR_LOGGING: &str = r##"
Java.perform(function () {
  var targetClass = Java.use('com.example.app.MyClass');
  targetClass.fallible.implementation = function (arg) {
    try {
      console.log("fallible called with ", arg);
      return this.fallible(arg);
    } catch (e) {
      console.log('error: ', e.stack);
      throw e;
    }
  };
});
"##;

    pub fn hook_replacement_error_logging(java: &Java) -> Result<JavaHookGuard> {
        let class = java.use_class("com.example.app.MyClass")?;
        let guard = class
            .replace("fallible", |ctx| {
                let arg: String = ctx.arg(0)?;
                println!("fallible called with {arg}");
                ctx.proceed()
            })?
            .on_error(|error| eprintln!("error: {error}"));
        Ok(guard)
    }

    const JS_HOOK_STRING_EQUALS: &str = r##"
Java.perform(function () {
  var str = Java.use('java.lang.String');
  var objectClass = 'java.lang.Object';
  str.equals.overload(objectClass).implementation = function (obj) {
    var response = str.equals.overload(objectClass).call(this, obj);
    if (obj) {
      if (obj.toString().length > 5) {
        console.log(str.toString.call(this) + ' == ' + obj.toString() + ' ? ' + response);
      }
    }
    return response;
  };
});
"##;

    pub unsafe fn hook_string_equals(java: &Java) -> Result<JavaHookGuard> {
        let string = java.use_class("java.lang.String")?;
        let guard = string
            .replace_with("equals", ["java.lang.Object"], |ctx| {
                let obj = ctx.arg_object(0)?;
                let response: bool = ctx.call_original(obj.as_ref())?;

                let this = ctx.this_object()?;
                if let Some(obj) = &obj {
                    let left = this.java_to_string()?;
                    let right = obj.java_to_string()?;
                    println!("{left} == {right} ? {response}");
                }

                ctx.ret(response)
            })?
            .on_error(|error| eprintln!("replacement callback failed: {error}"));
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

    pub unsafe fn raw_jni_slot_probe(java: &Java) -> Result<(*const c_void, *const c_void)> {
        unsafe fn get_native_address(
            env: std::ptr::NonNull<jni::JNIEnv>,
            slot: usize,
        ) -> *const c_void {
            let functions = unsafe { *(env.as_ptr().cast::<*const *const c_void>()) };
            unsafe { *functions.add(slot) }
        }

        let env = java.vm().attach_current_thread()?;
        let env_handle = unsafe { env.handle() };

        const REGISTER_NATIVES: usize = 215;
        const FIND_CLASS: usize = 6;

        let register_natives = unsafe { get_native_address(env_handle, REGISTER_NATIVES) };
        let find_class = unsafe { get_native_address(env_handle, FIND_CLASS) };

        Ok((register_natives, find_class))
    }
}
