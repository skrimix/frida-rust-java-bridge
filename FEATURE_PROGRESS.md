# Feature Progress

This is the quick feature tracker for the Rust bridge. It is aligned with the public surface
documented by upstream `frida-java-bridge`, while keeping this crate's posture explicit: Android ART
only, Rust-native APIs, no stable public contracts yet, and no promise of drop-in GumJS parity.

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
| `Java.perform()` app-loader deferral | Planned | none | Automatic app class-loader discovery and deferred execution remain open. |
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
| Object arrays | Partial | `Java::new_object_array()`, `Env` object-array helpers | Primitive arrays and a Java-array wrapper equivalent are not implemented. |
| Primitive arrays | Planned | none | Needed for closer `Java.array()` parity. |
| String helpers | Done | `Java::new_string_utf()`, `JavaObject::get_string()` | Covers current string round trips. |

## Class Loaders And Class Factories

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Explicit loader-scoped class lookup | Done | `ClassLoaderRef`, `Java::with_loader()` | Loader-backed handles keep separate class caches. |
| System class loader | Done | `Java::system_class_loader()` | Useful explicit loader source. |
| Wrap existing loader object | Done | `Java::class_loader_from_object()` | Validates object type before creating `ClassLoaderRef`. |
| Enumerate class loaders | Done | `enumerate_class_loaders()` on `Runtime`, `Vm`, and `Java` | API 26+ arm64 ART backend; unsupported layouts return structured errors. |
| Default app class loader | Planned | none | Required before `Java.perform()` / default `Java.use()` semantics can feel upstream-like. |
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
| Method replacement prerequisites | Experimental | ART capability reason and hidden backend probes | The public capability still reports method replacement unsupported. |
| Selected static/instance method replacement | Experimental | `experimental` module | Hidden clone-active replacement supports selected primitive, `String`, and one-reference lanes. |
| Calling original implementation | Experimental | `experimental::OriginalMethod` and raw original-call paths | Thread-scoped original bypass exists for tested lanes. |
| Replace/revert lifecycle | Experimental | `experimental::MethodReplacement` | Dedicated app-process tests cover replace/revert/replace for selected methods. |
| `.implementation`-style API | Planned | none | Needs a supported exported replacement surface and wrapper integration. |
| Closure-backed replacement callbacks | Planned | none | Current experimental replacement requires exact JNI-native callback ABIs. |
| Arbitrary signatures and multi-reference args | Planned | none | Current accepted ABI shapes are intentionally narrow. |
| Deoptimize everything / boot image | Planned | capability reports unsupported | Needed for predictable replacement behavior across interpreted, JIT, and quick paths. |
| Register Java classes | Deferred | none | Upstream `registerClass()` parity is later than core loader/replacement work. |

## Deliberate Scope Differences

- Android ART behavior is the compatibility target. Dalvik, HotSpot, JVM TI, desktop JVM support,
  and a JavaScript compatibility layer are out of scope for now.

## Near-Term Focus

1. Automatic app-loader selection and default app-loader-scoped `Java` handles.
2. Exported method replacement ergonomics integrated with selected wrapper overloads.
3. Broader object/reference and array ergonomics where they unblock real replacement/use workflows.
4. Heap enumeration and deoptimization once the ART paths are understood and testable.
5. Later convenience APIs such as main-thread scheduling, backtraces, dex loading, and class
   registration.
