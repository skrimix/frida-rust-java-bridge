# Frida JS Port Ergonomics Notes

These notes come from porting representative snippets from `~/work/frida/examples.txt` into
`examples/frida_js_ergonomics_probe.rs`. The file is intentionally not a live test; it is a
compile-oriented probe for places where the Rust API feels smooth or visibly incomplete. Each port
keeps the source Frida JS snippet as a Rust raw string constant next to the Rust code representing
it, so the gap comments can be read against the original example without reopening
`~/work/frida/examples.txt`.

## Example Coverage

Fully represented, modulo normal Rust explicitness:

- String construction, byte-array creation, unambiguous name-only calls, overload selection,
  unambiguous constructor shorthand, and `Charset.defaultCharset()`.
- Loaded-class enumeration.
- Wrapper member inspection for a target class.
- Global proxy setup through `ActivityThread`, `Context`, `ConnectivityManager`, and `ProxyInfo`.
- Default-constructor `TelephonyManager.getDeviceId()` sample, though it remains an Android API
  smell and should not become the preferred example shape.
- Main-thread toast scheduling.
- `StringBuilder.$init.overload("java.lang.String").implementation = ...` is now represented at
  the public facade level through `JavaClass::replace_constructor()`, including
  callback receiver and argument inspection.
- GumJS-style one-shot calls through receiver-based `call()` / `call_overload()` on classes and
  objects. A `JavaClass` receiver means static access, while object and bound-object receivers mean
  instance access.
- GumJS-style method replacement through direct `JavaClass::replace()` /
  `replace_overload()` for unambiguous static or instance methods. Selected method handles remain
  available when a hook target must be reused or inspected.
- Rock-paper-scissors `onClick` replacement, including callback receiver field writes through a
  borrowed local object view.
- Activity `onCreate` Wi-Fi toggle, including method calls on the callback receiver.
- `InputStream.read(byte[])`, including callback-local byte-array copy-out.
- `WebView.loadUrl(String)`, including callback-local string extraction.
- `StringBuilder.toString()`, including original return wrapping and string inspection.
- SharedPreferences `put*` hook family, including direct overload replacement and cheap
  stringification for reference values.
- `String.equals(Object)`, including receiver/argument `Object.toString()` diagnostics.
- Raw JNI slot probe with unsafe pointer calculations.
- Original constructor call from constructor replacement.
- Descriptor-driven numeric coercion for selected wrapper calls and field writes, with range checks
  for narrowing conversions.

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

## What Already Maps Cleanly

- `Java.perform()` and app-loader scoped work map to `Java::perform()`. The callback receives a
  `JavaScope`, which dereferences to `Java` and implements `AsRef<Java>`, so existing helper
  functions can take either `&JavaScope` directly or a plain `&Java` through deref coercion.
- `Java.use()` maps cleanly to `Java::use_class()` when the class and loader are known.
- Name-only method calls and hooks now map cleanly through `call()` and `replace()` when a method
  name has exactly one overload in the relevant receiver space.
- Explicit overload calls and hooks remain clear through `call_overload()` and
  `replace_overload()` when a method name is overloaded or the example intentionally documents the
  selected signature.
- Primitive arrays and object arrays are more explicit than JS arrays, but the ownership model is
  readable through `Java::new_byte_array()` and `JavaArray` helpers.
- `Java.cast()` maps to `JavaObject::cast(&TargetClass)` or `JavaClass::cast(&object)`, producing a
  new wrapper-bound object view over the same Java value.
- Main-thread scheduling has a direct Rust shape through `Java::schedule_on_main_thread()`.
- Loaded-class enumeration and wrapper metadata are already better typed than
  `Object.getOwnPropertyNames(Java.use(...).__proto__)`.

## Gaps Exposed By The Ports

1. Dynamic hook families still have some ceremony.
   Name handles remove the signature list for unambiguous `put*` methods, but Rust still has to keep
   each installed guard and spell out callback-local argument inspection.
2. Deferred `Java::perform()` setup can return hook guards, but callers still need to keep the
   returned `PerformResult<JavaHookSet>` alive.
   This avoids every caller having to spell out their own shared storage, while still being honest
   about the callback possibly running later.


## Candidate API Experiments

- Done: callback-local borrowed wrappers through
  `JavaHookContext::{this_object,arg_object,arg_array}`.
- Done: callback-local references can be retained into owned `JavaObject`, `JavaRef`, or
  `JavaArray` values through `retain()` on their local counterparts.
- Done: `JavaObject` / `JavaArray` and their callback-local aliases now share generic reference
  storage. `JavaObject` carries the selected wrapper class for member lookup, while `JavaRef`
  remains the unbound JNI-reference handle.
- Done: replacement callbacks have a safe iterable argument view through
  `JavaHookContext::{arguments,args,arg_value}` and `JavaHookArgument`, so exploratory matching and
  logging no longer require raw `JavaValue` access.
- Done: hook argument/original-return conversion supports `String` and `Option<String>` through
  `JavaHookContext::{arg,call_original}`.
- Done: `JavaHookContext::arg()` supports typed object and array extraction through
  `JavaLocalObject`, `Option<JavaLocalObject>`, `JavaLocalArray`, and
  `Option<JavaLocalArray>`, with non-null forms rejecting Java null.
- Done: `JavaMethod::replace()` is safe; raw JNI argument/original-return lanes and raw object
  returns remain explicit unsafe escape hatches. `JavaConstructor::replace()` remains unsafe.
- Done: `JavaObject::java_to_string()` and `JavaLocalObject::java_to_string()` provide common
  diagnostic `Object.toString()` behavior.
- Done: primitive field typed helpers cover boolean, byte, char, short, int, long, float, double,
  object, and array fields for instance and static handles.
- Done: constructor overloads have a guarded public `replace()` facade.
- Done: `JavaClass::new(args)` resolves classes with exactly one declared constructor and reports
  normal missing/ambiguous selector errors otherwise.
- Done: GumJS-style method selectors cover unambiguous instance/static calls, replacement
  installation, tuple type-list selectors, and tuple arity selectors; overloaded bare names report
  candidate signatures.
- Done: selected-overload calls accept a bare single argument, so one-argument calls like
  `getSystemService("connectivity")` no longer need one-element tuple syntax.
- Done: wrapper calls auto-convert Rust `&str`, `String`, and `&String` arguments into temporary
  Java strings for selected `java.lang.String`, `java.lang.CharSequence`, and `java.lang.Object`
  parameters, including mixed object/string/primitive tuples.
- Done: `call_original*` accepts a bare single `JavaValue`-convertible argument, so callback-local
  object/array references and primitive originals no longer need one-element tuple syntax.
- Done: replacement callbacks can pass through the original implementation with the current
  callback arguments through `JavaHookContext::call_original_current()`, avoiding raw argument
  cloning for simple logging hooks.
- Done: replacement callbacks can request explicit original-return pass-through with
  `JavaHookContext::call_original_return(args)` or `call_original::<JavaHookReturn>(args)`.
  `JavaHookReturn` is now a hook-facing alias for the raw-reference `JavaReturn` specialization,
  keeping wrapper and hook returns in one container family.
- Done: selected wrapper calls and field writes perform conservative numeric coercion for in-range
  `int` to `byte`/`short`/`char`/`long`, `float` to `double`, and in-range finite `double` to
  `float`.
- Done: `java_display()` provides diagnostic text for Java objects, arrays, raw wrapper returns,
  hook arguments, and class/member wrappers. `JavaHookContext::arg_display()` remains the
  single-argument hook convenience wrapper over that shared display behavior.
- Deferred: typed tuple extraction from hook arguments may still be useful, but is intentionally
  out of scope until a compact design proves necessary.
- Decide whether a safe original-constructor chaining story belongs in the public facade, or whether
  constructor callbacks should remain limited to receiver-initializing replacements.
- Consider a small raw JNI diagnostics helper that exposes slot addresses with named slot constants
  without making the whole internal vtable helper surface public.
