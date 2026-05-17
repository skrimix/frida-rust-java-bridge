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
| Android version / API level | Done | `Runtime::android_version()`, `Vm::android_version()`, `Java::android_version()`, plus `android_api_level()` helpers | Reports Android release string and SDK API level; ART layout code uses the same API-level property reader internally. |
| VM handle and thread attachment | Done | `Runtime::vm()`, `Vm::{try_get_env,get_env,attach_current_thread,detach_current_thread}` | Covers the useful `Java.vm` core without JS callback wrapping. |
| `Java.performNow()`-style immediate attachment | Done | `Runtime::perform_now()`, `Vm::perform_now()`, `Java::perform_now()` | Runs synchronously with the current thread attached. Runtime/VM callbacks receive bootstrap-scoped `Java`; `Java::perform_now()` preserves the receiver's loader scope and never queues app-loader work. |
| `Java.perform()` app-loader deferral | Experimental | `Java::perform()`, `Runtime::perform()`, `Vm::perform()`, `RuntimeCapabilities::app_loader_deferral` | Queues Rust callbacks and drains them with an app-loader-scoped `Java`; deferred startup currently depends on hidden ART method replacement and Android startup hooks spanning `LoadedApk.makeApplication*` and selected `ActivityThread.getPackageInfo` overloads. The handle/status behavior is a soft-freeze candidate, and a side-effect-light capability probe reports startup-hook readiness before hook installation. APK early-start validation passes on the current four-device matrix. |
| Main-thread scheduling | Experimental | `Java::is_main_thread()`, `Java::schedule_on_main_thread()`, plus `Runtime`/`Vm` helpers, `RuntimeCapabilities::main_thread_scheduling` | Uses `Looper` checks, a process-global Rust queue, `Handler(Looper.getMainLooper()).sendEmptyMessage(1)`, and a Gum `epoll_wait` hook to drain queued callbacks on the main thread. The handle/status behavior is a soft-freeze candidate, and capability reporting checks scheduler prerequisites without enqueueing or waking the looper. Command-line `app_process` reports unsupported when no main looper exists; APK early-start validation covers real main-looper drain. |
| Method flag constants | Done | `ACC_PUBLIC`, `ACC_PRIVATE`, `ACC_PROTECTED`, `ACC_STATIC`, `ACC_FINAL`, `ACC_SYNCHRONIZED`, `ACC_BRIDGE`, `ACC_VARARGS`, `ACC_NATIVE`, `ACC_ABSTRACT`, `ACC_STRICT`, `ACC_SYNTHETIC` | Metadata still exposes raw `modifiers`; callers can use named constants for bit checks. |

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
| Default app class loader | Partial | `Java::app_class_loader()`, `Java::with_app_loader()`, `Runtime::app_java()`, `Vm::app_java()`, `perform()` helpers | Synchronous helpers use `ActivityThread.currentApplication().getClassLoader()` and keep explicit unavailable errors; `perform()` adds an experimental deferred path. Remaining work is broader default-loader workflow design, not a small helper gap. |
| ClassFactory manager | Partial | clone `Java` with `with_loader()` | Loader-specific class access exists through `Java`; a global `ClassFactory`, cache directory, temp-file naming, and dex-loading manager remain a larger deferred design surface. |
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
| Hidden ART method replacement backend | Experimental | clone-active ART replacement internals | Hidden clone-active replacement supports selected primitive, `String`, one-reference/reference-return lanes, one-reference/void hooks, and exact startup-hook ABIs used by deferred app-loader initialization. This remains experimental even where the public facade is soft-frozen. |
| Public guarded replacement facade | Done | `JavaMethodOverload::install_implementation`, `replacement::{ImplementationInvocation, ImplementationReturn, ImplementationGuard}` | Soft-frozen for the currently admitted and test-covered lanes. Unsupported signatures fail before installation with errors naming method kind, name, descriptor, and admitted lanes. |
| Calling original implementation from facade callbacks | Done | `ImplementationInvocation::{call_original,call_original_as}` | Covered for tested static and instance lanes. Thread-scoped raw original handles remain internal. |
| Replace/revert lifecycle | Done | `ImplementationGuard` | Dedicated app-process tests cover guard ownership, duplicate active replacement rejection, explicit revert, idempotent successful revert, and replace/revert/replace for selected methods. |
| Callback failure handling | Done | `ImplementationGuard::{last_error,take_last_error}` plus JNI default fallback returns | Callback errors, panics, and wrong return kinds are recorded on the guard and cause Java callers to receive the JNI default value for the method return type. |
| Typed facade argument/return helpers | Done | `FromJavaValue`, `FromImplementationReturn`, `IntoImplementationReturn`, `ImplementationReturn` | Primitive conversions, typed argument extraction, object/object-array borrowed returns, and null-reference handling are covered for the admitted lanes. |
| Raw closure-backed replacement callbacks | Experimental | internal raw closure backend | Raw invocation closures are supported for selected overload ABI shapes as crate-internal scaffolding for the public facade, startup hooks, and live-runtime harnesses. |
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
