# Frida JS Port Ergonomics Notes

These notes come from porting representative snippets from `~/work/frida/examples.txt` into
`examples/frida_js_ergonomics_probe.rs`. The file is intentionally not a live test; it is a
compile-oriented probe for places where the Rust API feels smooth or visibly incomplete. Each port
keeps the source Frida JS snippet as a Rust raw string constant next to the Rust code representing
it, so the gap comments can be read against the original example without reopening
`~/work/frida/examples.txt`.

## Example Coverage

Fully represented, modulo normal Rust explicitness:

- String construction, byte-array creation, unambiguous name-only calls, overload selection, and
  `Charset.defaultCharset()`.
- Loaded-class enumeration.
- Wrapper member inspection for a target class.
- Global proxy setup through `ActivityThread`, `Context`, `ConnectivityManager`, and `ProxyInfo`.
- Default-constructor `TelephonyManager.getDeviceId()` sample, though it remains an Android API
  smell and should not become the preferred example shape.
- Main-thread toast scheduling.
- `StringBuilder.$init.overload("java.lang.String").implementation = ...` is now represented at
  the public facade level through `JavaConstructor::replace()`, including
  callback receiver and argument inspection.
- GumJS-style method selectors through `JavaClass::method()` / `static_method()`, including
  name-only unambiguous calls, type-list overload selection, arity selection, and `replace()`.
- Rock-paper-scissors `onClick` replacement, including callback receiver field writes through a
  borrowed local object view.
- Activity `onCreate` Wi-Fi toggle, including method calls on the callback receiver.
- `InputStream.read(byte[])`, including callback-local byte-array copy-out.
- `WebView.loadUrl(String)`, including callback-local string extraction.
- `StringBuilder.toString()`, including original return wrapping and string inspection.
- SharedPreferences `put*` hook family, including name-handle installation and cheap
  stringification for reference values.
- `String.equals(Object)`, including receiver/argument `Object.toString()` diagnostics.
- Raw JNI slot probe as a documented unsupported escape hatch.
- Original constructor call from constructor replacement.

Partially represented because non-reference ergonomics are still intentionally explicit:

- `WebView.loadUrl(String)` and the dynamic stacktrace examples still do not model GumJS
  `send(...)`; the probe records the value that would be sent instead.
- SharedPreferences primitive values are inspected as `JavaValue` primitives; only Java references
  use `Object.toString()` diagnostics.

Not implemented as Rust behavior yet:

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
- Name-only method calls and hooks now map cleanly through `method()` and `static_method()` when a
  method name has exactly one overload.
- Explicit overload calls remain clear through `overload()`, `static_overload()`, and
  `constructor()` when a method name is overloaded or the example intentionally documents the
  selected signature.
- Primitive arrays and object arrays are more explicit than JS arrays, but the ownership model is
  readable through `Java::new_byte_array()` and `JavaArray` helpers.
- `Java.cast()` maps well to `JavaClass::cast()` once the value is already a `JavaObject`.
- Main-thread scheduling has a direct Rust shape through `Java::schedule_on_main_thread()`.
- Loaded-class enumeration and wrapper metadata are already better typed than
  `Object.getOwnPropertyNames(Java.use(...).__proto__)`.

## Gaps Exposed By The Ports

1. Raw JNI slot introspection is not public.
   The JS vtable example can read `env.handle` directly and index slots. The Rust crate keeps
   `jni::env_function` and JNI slot constants private, so there is no supported user-code equivalent.

2. Dynamic hook families still have some ceremony.
   Name handles remove the signature list for unambiguous `put*` methods, but Rust still has to keep
   each installed guard and spell out callback-local argument inspection.

3. Zero-arg constructors are easy to write but not necessarily meaningful.
   The TelephonyManager example ports mechanically with `new_instance([], ())`, but real Android
   APIs often expect service lookup through `Context`. Examples should probably prefer the safer
   service/cast pattern.

## Candidate API Experiments

- Done: callback-local borrowed wrappers through
  `JavaHookContext::{receiver_object,arg_object,arg_array,arg_string}`.
- Done: callback-local references can be retained into owned `JavaObject` / `JavaArray` values
  through `JavaLocalObject::retain()` and `JavaLocalArray::retain()`.
- Done: `JavaObject::java_to_string()` and `JavaLocalObject::java_to_string()` provide common
  diagnostic `Object.toString()` behavior.
- Done: primitive field typed helpers cover boolean, byte, char, short, int, long, float, double,
  object, and array fields for instance and static handles.
- Done: constructor overloads have a guarded public `replace()` facade.
- Done: GumJS-style method selectors cover unambiguous instance/static calls, replacement
  installation, tuple type-list selectors, and tuple arity selectors; overloaded bare names report
  candidate signatures.
- Decide whether a safe original-constructor chaining story belongs in the public facade, or whether
  constructor callbacks should remain limited to receiver-initializing replacements.
- Consider a small raw JNI diagnostics escape hatch that exposes slot addresses without making the
  whole vtable helper surface public.
