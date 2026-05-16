# Feature Progress

This is the quick feature tracker for the Rust bridge. It is aligned with the public surface
documented by upstream `frida-java-bridge`, while keeping this crate's posture explicit: Android ART
only, Rust-native APIs, no stable public contracts yet, and no promise of drop-in GumJS parity.

This file is a status matrix, not a second roadmap. `ROADMAP.md` owns sequencing, current focus, and
milestone planning; keep this tracker limited to what exists, what is missing, and how current Rust
APIs map to upstream concepts.

Reference: `../frida-java-bridge/PUBLIC_DOC.md`.

## Status Key

- Done: useful, test-covered, and soft-frozen enough to avoid casual churn.
- Partial: a working subset exists, but an important behavior or ergonomic piece is missing.
- Experimental: implemented behind hidden or high-risk prototype APIs; not a supported surface.
- Planned: wanted for the Rust bridge, but not implemented yet.
- Deferred: plausible later work, but not part of the current core push.
- Out of scope: deliberately excluded unless the project is rescoped.

## Runtime And Attachment

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Java runtime availability | Done | `Runtime::obtain()` | Returns structured errors instead of exposing a process-global boolean like `Java.available`. |
| Android runtime target | Done | `RuntimeFlavor::Art` | ART is the only active target. Dalvik and desktop JVMs are out of scope. |
| Android version / API level | Partial | internal ART API-level reads | API level is probed for ART layout decisions; no public `androidVersion` equivalent yet. |
| VM handle and thread attachment | Done | `Runtime::vm()`, `Vm::{try_get_env,get_env,attach_current_thread,detach_current_thread}` | Covers the useful `Java.vm` core without JS callback wrapping. |
| `Java.performNow()`-style immediate attachment | Partial | call `attach_current_thread()` or use `Runtime::java()` / `Vm::java()` helpers | No callback wrapper is needed for current Rust APIs, but a convenience closure helper may still be useful. |
| `Java.perform()` app-loader deferral | Experimental | `Java::perform()`, `Runtime::perform()`, `Vm::perform()` | Queues Rust callbacks and drains them with an app-loader-scoped `Java`; deferred startup currently depends on hidden ART method replacement and Android startup hooks spanning `LoadedApk.makeApplication*` and selected `ActivityThread.getPackageInfo` overloads. A dedicated APK startup-agent harness now validates the early bind-time drain path. |
| Main-thread scheduling | Planned | none | Upstream `scheduleOnMainThread()` / `isMainThread()` equivalents are not implemented. |
| Method flag constants | Partial | raw `modifiers` fields in metadata | Constants can be added when callers need named access-flag helpers. |

## Class, Object, And Value Access

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Low-level JNI class/method/field/string access | Done | `Env`, `MethodRef`, `FieldRef`, typed JNI refs | Covers explicit lookup, invocation, fields, strings, exceptions, and reference ownership. |
| Descriptor parsing and value marshaling | Done | `JavaType`, `MethodSignature`, `JavaValue` | Host-testable parsing, validation, and JNI argument conversion are covered. |
| Owned class/object wrappers | Done | `JavaClass`, `JavaObject`, `JavaReturn` | Global-reference-backed helpers over low-level `Env`. |
| Rust-native `Java.use()`-style wrappers | Done | `Java::use_class()`, `JavaClassWrapper` | Explicit overload selection; not a JS dynamic wrapper. |
| Constructors, methods, static methods, fields | Done | `JavaClassWrapper`, `JavaConstructorOverload`, `JavaMethodOverload`, `JavaFieldHandle` | Reflection-validated overload/member handles with typed convenience helpers. |
| Automatic JS-style overload dispatch | Deferred | none | Explicit Rust overload selection is preferred until real ergonomics demand more. |
| Object retain | Done | `JavaObject::retain()` | Equivalent ownership goal to `Java.retain()`, scoped to Rust objects. |
| Object cast/type checks | Done | `JavaClass::is_instance()`, `JavaClassWrapper::{is_instance,cast}` | Validates runtime type without inferring loader identity. |
| Object arrays | Done | `Java::new_object_array()`, `JavaArray`, `Env` object-array helpers | Object arrays have nullable element access and can be passed through `JavaValue`; array returns use `JavaReturn::Array`. |
| Primitive arrays | Done | `Java::{new_boolean_array,new_byte_array,new_char_array,new_short_array,new_int_array,new_long_array,new_float_array,new_double_array}`, `JavaArray` primitive accessors | High-level helpers use full-array copy-in/copy-out semantics backed by JNI region APIs, not JS-style mutable array proxy behavior. |
| String helpers | Done | `Java::new_string_utf()`, `JavaObject::get_string()` | Covers current string round trips. |

## Class Loaders And Class Factories

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Explicit loader-scoped class lookup | Done | `ClassLoaderRef`, `Java::with_loader()` | Loader-backed handles keep separate class caches. |
| System class loader | Done | `Java::system_class_loader()` | Useful explicit loader source. |
| Wrap existing loader object | Done | `Java::class_loader_from_object()` | Validates object type before creating `ClassLoaderRef`. |
| Enumerate class loaders | Done | `enumerate_class_loaders()` on `Runtime`, `Vm`, and `Java` | API 26+ arm64 ART backend; unsupported layouts return structured errors. |
| Default app class loader | Partial | `Java::app_class_loader()`, `Java::with_app_loader()`, `Runtime::app_java()`, `Vm::app_java()`, `perform()` helpers | Synchronous helpers use `ActivityThread.currentApplication().getClassLoader()` and keep explicit unavailable errors; `perform()` adds an experimental deferred path with app-process hook setup coverage and APK startup-agent drain coverage. |
| ClassFactory manager | Partial | clone `Java` with `with_loader()` | There is no global `ClassFactory`, cache directory, or temp-file naming API. |
| Open/load dex class file | Deferred | manual DexClassLoader usage in test harness | A first-class `openClassFile()` equivalent is not implemented. |

## Enumeration And Metadata

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Enumerate loaded classes | Done | `enumerate_loaded_classes()` on `Runtime`, `Vm`, and `Java` | ART-backed class enumeration with reflection metadata helpers. |
| Class metadata | Done | `JavaClassMetadata`, `JavaMethodMetadata`, `JavaFieldMetadata` | Declared constructors, methods, and fields. |
| Enumerate methods by query | Done | `Java::enumerate_methods("class!method/modifiers")` | Supports `/i`, `/s`, `/u`; `/s` exposes JNI descriptors rather than upstream source-style strings. |
| Heap instance enumeration | Planned | capability reports unsupported | Upstream `Java.choose()` equivalent remains future ART work. |
| Java backtrace | Planned | none | Useful but not started. |

## Replacement, Hooks, And ART Advanced Features

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Method replacement prerequisites | Experimental | ART capability reason and hidden backend probes | Capability reports experimental availability when current ART prerequisites are present, otherwise structured unsupported reasons. |
| Selected static/instance method replacement | Experimental | `experimental` module | Hidden clone-active replacement supports selected primitive, `String`, one-reference/reference-return lanes, one-reference/void instance hooks, and exact startup-hook ABIs used by deferred app-loader initialization. |
| Calling original implementation | Experimental | `experimental::OriginalMethod` and raw original-call paths | Thread-scoped original bypass exists for tested lanes. |
| Replace/revert lifecycle | Experimental | `experimental::MethodReplacement` | Dedicated app-process tests cover replace/revert/replace for selected methods. |
| Overload replacement ergonomics | Experimental | `JavaMethodOverload::{replace,replace_native,original}` | Unsafe, ABI-explicit, and limited to the backend's currently tested JNI-native callback shapes. |
| `.implementation`-style API | Planned | none | Closure-backed/native Rust ergonomics are still not implemented. |
| Closure-backed replacement callbacks | Planned | none | Current experimental replacement requires exact JNI-native callback ABIs. |
| Arbitrary signatures and multi-reference args | Planned | none | Current accepted ABI shapes are intentionally narrow. |
| Deoptimize everything / boot image | Planned | capability reports unsupported | Needed for predictable replacement behavior across interpreted, JIT, and quick paths. |
| Register Java classes | Deferred | none | Upstream `registerClass()` parity is later than core loader/replacement work. |

## Deliberate Scope Differences

- Android ART behavior is the compatibility target. Dalvik, HotSpot, JVM TI, desktop JVM support,
  and a JavaScript compatibility layer are out of scope for now.

## Planning Boundary

Use `ROADMAP.md` for priority order and milestone planning. This tracker should change when feature
status changes, when a Rust API shape changes, or when an upstream concept is deliberately moved in
or out of scope.
