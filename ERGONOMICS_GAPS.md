# Frida JS Port Ergonomics Notes

These notes come from porting representative snippets from `~/work/frida/examples.txt` into
`examples/frida_js_ergonomics_probe.rs`. The file is intentionally not a live test; it is a
compile-oriented probe for places where the Rust API feels smooth or visibly incomplete. Each port
keeps the source Frida JS snippet as a Rust raw string constant next to the Rust code representing
it, so the gap comments can be read against the original example without reopening
`~/work/frida/examples.txt`.

## Example Coverage

Fully represented, modulo normal Rust explicitness:

- String construction, overload selection, byte-array creation, and `Charset.defaultCharset()`.
- Loaded-class enumeration.
- Wrapper member inspection for a target class.
- Global proxy setup through `ActivityThread`, `Context`, `ConnectivityManager`, and `ProxyInfo`.
- Default-constructor `TelephonyManager.getDeviceId()` sample, though it remains an Android API
  smell and should not become the preferred example shape.
- Main-thread toast scheduling.
- Raw JNI slot probe as a documented unsupported escape hatch.

Partially represented because callback-local reference ergonomics are missing:

- Rock-paper-scissors `onClick` replacement: original call is represented; `this.m/n/cnt.value`
  field writes are blocked by raw receiver handling and narrow field setter ergonomics.
- Activity `onCreate` Wi-Fi toggle: overload replacement is represented; `this.getSystemService`
  and `Java.cast(...)` from the callback receiver are blocked by raw receiver handling.
- `InputStream.read(byte[])`: original call is represented; byte-array copy-out from the raw
  callback argument is blocked.
- `WebView.loadUrl(String)`: original call is represented; `s.toString()`/`send(...)` is blocked by
  raw string argument handling and no Rust-side agent messaging helper.
- `StringBuilder.toString()`: replacement and original call are represented; returned
  `java.lang.String` slicing/logging is blocked by raw object returns in callbacks.
- SharedPreferences `put*` overload family: the repeated hook shape is represented; cheap JS-style
  logging of arbitrary reference values is blocked.
- `String.equals(Object)`: original call is represented; `this.toString()` and `obj.toString()` are
  blocked by raw receiver/argument handling and missing `Object.toString` convenience.

Not implemented as Rust behavior yet:

- `StringBuilder.$init.overload("java.lang.String").implementation = ...`; constructor
  replacement is intentionally outside the public `install_implementation()` facade today.
- The `StringBuilder`/`StringBuffer` dynamic class loop with conditional stacktrace and
  `send(...)`; the static Rust ports can select both classes, but the useful behavior depends on
  callback-local string inspection, a stacktrace helper, and an agent messaging surface that are not
  present yet.
- Direct JS-style JNI vtable pointer indexing from `env.handle`; the probe records the missing raw
  diagnostics hatch but keeps the crate-private vtable helpers private.

## What Already Maps Cleanly

- `Java.perform()` and app-loader scoped work map to `Java::perform()` or helper functions taking
  an app-loader-scoped `Java`.
- `Java.use()` maps cleanly to `Java::use_class()` when the class and loader are known.
- Explicit overload calls are verbose but clear through `overload()`, `static_overload()`, and
  `constructor()`.
- Primitive arrays and object arrays are more explicit than JS arrays, but the ownership model is
  readable through `Java::new_byte_array()` and `JavaArray` helpers.
- `Java.cast()` maps well to `JavaClassWrapper::cast()` once the value is already a `JavaObject`.
- Main-thread scheduling has a direct Rust shape through `Java::schedule_on_main_thread()`.
- Loaded-class enumeration and wrapper metadata are already better typed than
  `Object.getOwnPropertyNames(Java.use(...).__proto__)`.

## Gaps Exposed By The Ports

1. Replacement callbacks expose `this` as raw `jobject`.
   JS examples commonly call methods and set fields on `this`. Rust field handles and method calls
   need `JavaObject`, but `ImplementationInvocation::receiver()` currently exposes only a raw JNI
   reference.

2. Replacement object and array arguments are raw.
   Examples such as `InputStream.read(byte[])`, `WebView.loadUrl(String)`, and `String.equals(Object)`
   need callback-local wrappers for raw object arguments so code can call `get_string()`, copy bytes
   out of arrays, cast values, or call `toString()`.

3. Constructor replacement is missing from the public facade.
   `StringBuilder.$init.overload("java.lang.String").implementation = ...` cannot be represented
   with `JavaConstructorOverload` today.

4. Reference-to-string logging needs a convenience layer.
   Rust can inspect primitives in `JavaValue`, but "log this arbitrary Java reference like JS would"
   needs either an `Object.toString()` helper, a callback-local wrapper, or a class-aware display
   adapter.

5. Field setters are narrow.
   `JavaFieldHandle` has `set_int`, `set_object`, and `set_array`, but not typed helpers for
   boolean, long, float, double, and other primitives. The generic `set(..., JavaValue)` works, but
   ports of simple field edits feel unnecessarily uneven.

6. Raw JNI slot introspection is not public.
   The JS vtable example can read `env.handle` directly and index slots. The Rust crate keeps
   `jni::env_function` and JNI slot constants private, so there is no supported user-code equivalent.

7. Dynamic overload families are repetitive.
   The shared-preferences example exposes a common "hook several overloads and call original"
   pattern. Rust can do it, but it is boilerplate-heavy because each selected overload is a distinct
   value and callback argument display is still manual.

8. Zero-arg constructors are easy to write but not necessarily meaningful.
   The TelephonyManager example ports mechanically with `new_instance([], ())`, but real Android
   APIs often expect service lookup through `Context`. Examples should probably prefer the safer
   service/cast pattern.

## Candidate API Experiments

- Add callback-local borrowed wrappers, for example
  `ImplementationInvocation::{receiver_object,arg_object,arg_array,arg_string}`.
- Add a public way to retain a raw callback-local reference into `JavaObject` / `JavaArray` with an
  explicit lifetime or ownership name.
- Add `JavaObject::to_string()` or `Java::object_to_string(&JavaObject)` as a common diagnostic
  helper.
- Add missing primitive field typed helpers for parity with method return helpers.
- Decide whether constructor replacement belongs in the guarded public facade or remains an
  intentional prototype boundary.
- Consider a small raw JNI diagnostics escape hatch that exposes slot addresses without making the
  whole vtable helper surface public.
