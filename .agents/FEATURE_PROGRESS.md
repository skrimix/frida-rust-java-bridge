# Feature Progress

This is the quick feature tracker for the Rust bridge. It is aligned with the public surface
documented by upstream `frida-java-bridge`, while keeping this crate's posture explicit: Android ART
only, Rust-native APIs, no stable public contracts yet, and no promise of drop-in GumJS parity.

This file is a status matrix, not a second roadmap. `ROADMAP.md` owns sequencing, current focus, and
milestone planning; keep this tracker limited to what exists, what is missing, and how current Rust
APIs map to upstream concepts.

Reference: `../frida-java-bridge/PUBLIC_DOC.md`.

## Status Key

- Done: useful and test-covered enough to be part of the current Rust surface.
- Partial: a working subset exists, but an important behavior or ergonomic piece is missing.
- Planned: wanted for the Rust bridge, but not implemented yet.
- Deferred: plausible later work, but not part of the current core push.
- Out of scope: deliberately excluded unless the project is rescoped.

## Runtime And Attachment

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Java runtime availability | Done | `Java::obtain()` | Returns a Java facade ready for `perform()` work or a structured runtime discovery error instead of exposing a process-global boolean like `Java.available`; low-level `find_class()` remains bootstrap-scoped on the bare handle. |
| Android runtime target | Done | `Java::obtain()` | ART is the only active target. Dalvik and desktop JVMs are out of scope; runtime targeting is internal discovery state, not a public enum. |
| Android version / API level | Done | `Java::android_version()`, `Java::android_api_level()` | Reports Android release string and SDK API level; ART layout code uses the same API-level property reader internally. |
| VM handle and thread attachment | Done | `Java::vm()`, `Vm::{try_get_env,get_env,attach_current_thread,detach_current_thread}` | Covers the useful `Java.vm` core without JS callback wrapping. |
| `Java.performNow()`-style immediate attachment | Done | `Java::perform_now()`, `JavaScope` | Runs synchronously with the current thread attached, preserves the receiver's loader scope, and never queues app-loader work. |
| `Java.perform()` app-loader deferral | Partial | `Java::perform()`, `JavaScope`, `JavaCapabilities::app_loader_deferral` | Queues Rust callbacks and drains them with an app-loader-scoped `JavaScope`; deferred startup currently depends on ART method replacement and Android startup hooks spanning `ActivityThread.handleBindApplication`, `LoadedApk.makeApplication*`, and selected early `ActivityThread.getPackageInfo` overloads. Startup drains are one-shot and switch instrumented startup to the late make-application path. `perform()` returns `PerformResult<T>`, which is observational for JS-style side-effect work and can store an eventual Rust value such as a hook set. The side-effect-light capability probe reports startup-hook readiness before hook installation. APK early-start validation passes on the current connected-device matrix. |
| Main-thread scheduling | Partial | `Java::is_main_thread()`, `Java::schedule_on_main_thread()`, `JavaCapabilities::main_thread_scheduling` | Uses `Looper` checks, a process-global Rust queue, `Handler(Looper.getMainLooper()).sendEmptyMessage(1)`, and a Gum `epoll_wait` hook to drain queued callbacks on the main thread. Capability reporting checks scheduler prerequisites without enqueueing or waking the looper. Command-line `app_process` reports unsupported when no main looper exists; APK early-start validation covers real main-looper drain. |
| Method flag constants | Done | `ACC_PUBLIC`, `ACC_PRIVATE`, `ACC_PROTECTED`, `ACC_STATIC`, `ACC_FINAL`, `ACC_SYNCHRONIZED`, `ACC_BRIDGE`, `ACC_VARARGS`, `ACC_NATIVE`, `ACC_ABSTRACT`, `ACC_STRICT`, `ACC_SYNTHETIC` | Metadata still exposes raw `modifiers`; callers can use named constants for bit checks. |

## Class, Object, And Value Access

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Low-level JNI class/method/field/string access | Done | `Env`, `MethodRef`, `FieldRef`, typed JNI refs | Covers explicit lookup, invocation, fields, strings, exceptions, and reference ownership. |
| Descriptor parsing and value marshaling | Done | `JavaType`, `MethodSignature`, `JavaValue<R>` | Host-testable parsing, validation, JNI argument conversion, void/null/reference lanes, and shared return/hook value shape are covered. |
| Raw class/object access | Done | `java::raw::Class`, `JavaObject`, `JavaReturn` | Global-reference-backed helpers over low-level `Env`; object handles carry a selected wrapper class, and `JavaReturn` is the owned-reference `JavaValue` specialization. |
| Rust-native `Java.use()`-style wrappers | Done | `Java::use_class()`, `JavaClass`, `JavaObject` | Default high-level facade with ranked runtime overload dispatch, explicit overload selection, and wrapper-bound object views. Bare handles prefer the published default app loader for wrapper lookup, while explicit loader handles preserve their scope. |
| Constructors, methods, static methods, fields | Done | `JavaClass::{new,new_with,constructor}`, `JavaConstructor`, `JavaMethod`, `JavaField` | Reflection-validated overload/member handles with typed helpers for primitive, object, and array fields; selected instance handles validate receiver class before JNI; exact selected calls validate reference argument assignability; `new()` and name-only calls use ranked argument dispatch, while exact overload handles remain available. |
| Automatic JS-style overload dispatch | Done | `JavaClass::call`, `JavaObject::call`, `JavaBoundObject::call`, `JavaClass::new` | Name-only calls and constructors pick the best compatible overload using a deterministic Frida-like ranking. This is not full Java compiler overload resolution; use exact overload selection when a broad compatible overload should be chosen deliberately. |
| Object retain | Done | `JavaObject::retain()`, `JavaLocalObject::retain()`, `JavaLocalArray::retain()` | Equivalent ownership goal to `Java.retain()`, with callback-local borrowed views retaining into owned globals when needed. |
| Object cast/type checks | Done | `JavaClass::is_instance()`, `JavaClass::{is_instance,cast}`, `JavaObject::cast()` | Validates runtime type and returns a wrapper-bound object view over the same Java value. |
| Object arrays | Done | `Java::new_object_array()`, `JavaArray`, `Env` object-array helpers | Object arrays have nullable element access and can be passed through `JavaValue`; array returns use the unified `JavaReturn` reference lane with a `JavaReturnRef::Array` payload. |
| Primitive arrays | Done | `Java::{new_boolean_array,new_byte_array,new_char_array,new_short_array,new_int_array,new_long_array,new_float_array,new_double_array}`, `JavaArray` primitive accessors | High-level helpers use full-array copy-in/copy-out semantics backed by JNI region APIs, not JS-style mutable array proxy behavior. |
| String helpers | Done | `Java::new_string_utf()`, `JavaObject::get_string()`, `JavaLocalObject::get_string()`, `java_to_string()` | Covers string round trips plus diagnostic `Object.toString()` on owned and callback-local object views. |

## Class Loaders And Class Factories

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Explicit loader-scoped class lookup | Done | `ClassLoaderRef`, `Java::with_loader()` | Loader-backed handles keep separate class caches and reject custom loader results whose returned `Class.getName()` does not match the requested normalized name. |
| System class loader | Done | `Java::system_class_loader()` | Useful explicit loader source. |
| Wrap existing loader object | Done | `Java::class_loader_from_object()` | Validates object type before creating `ClassLoaderRef`. |
| Enumerate class loaders | Done | `Java::enumerate_class_loaders()` | API 26+ arm64 ART backend; unsupported layouts return structured errors. |
| Default app class loader | Done | `Java::perform()`, `Java::app_class_loader()`, `Java::with_app_loader()`, `Java::default_app_loader()` | `perform()` is the normal app-class entry point. Synchronous helpers use `ActivityThread.currentApplication().getClassLoader()` and keep explicit unavailable errors. `with_app_loader()` and successful `perform()` paths publish the default app loader used by bare `use_class()` wrapper lookup; low-level `find_class()` remains bootstrap-scoped on bare handles. Deferred startup remains covered by the `Java.perform()` row. |
| ClassFactory manager | Partial | clone `Java` with `with_loader()` plus default app wrapper cache | Loader-specific class access exists through `Java`, and the default app wrapper path has a dedicated cache. A full `ClassFactory`, cache directory, temp-file naming, allocation/init-only constructor helpers, and dex-loading manager remain a larger deferred design surface. |
| Open/load dex class file | Deferred | manual DexClassLoader usage in test harness | A first-class `openClassFile()` equivalent is not implemented. |

## Enumeration And Metadata

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Enumerate loaded classes | Done | `Java::enumerate_loaded_classes()` | ART-backed class enumeration with reflection metadata helpers. |
| Class metadata | Done | `JavaClassMetadata`, `JavaMethodMetadata`, `JavaFieldMetadata` | Declared constructors, methods, and fields. |
| Enumerate methods by query | Done | `Java::enumerate_methods("class!method/modifiers")` | Supports `/i`, `/s`, `/u`; `/s` exposes JNI descriptors rather than upstream source-style strings. |
| Heap instance enumeration | Partial | `Java::choose_instances()`, `JavaClass::choose_instances()`, `JavaCapabilities::heap_enumeration` | Exact-class ART heap enumeration using a callback that returns `JavaChooseControl::{Continue, Stop}`. Only Android <12 supported. Missing ART heap symbols/layouts return structured unsupported errors. |
| Java backtrace | Planned | none | Useful but not started. |

## Replacement, Hooks, And ART Advanced Features

| Feature | Status | Current Rust shape | Notes |
| --- | --- | --- | --- |
| Method replacement prerequisites | Done | `JavaCapabilities::method_replacement` | Capability reports supported when current ART prerequisites are present, otherwise structured unsupported reasons. |
| ART method replacement backend | Partial | clone-active ART replacement internals | Internal clone-active replacement supports selected primitive, `String`, reference, multi-reference, mixed primitive, wide primitive, float-mix, stack-spill, and exact startup-hook ABIs used by deferred app-loader initialization. Direct backend mutation remains internal/unsafe; the method facade is safe. |
| Public guarded replacement facade | Done | `JavaMethod::replace`, `JavaConstructor::replace`, `java::replacement::{JavaHookContext, JavaConstructorHookContext, JavaConstructorInitialized, JavaHookArgument, JavaHookReturn, JavaHookGuard}` | `JavaHookReturn` is the raw-reference `JavaValue` specialization used at the method hook boundary; primitive, Rust string, null, owned wrapper, borrowed wrapper, and callback-local object/array returns are safe through `IntoJavaHookReturn`, with invocation-local views converted through `JavaHookContext::ret()` because direct `Ok(JavaLocalObject<'_>)` cannot fit `replace()`'s single return type. Raw JNI returns remain explicit `unsafe`. Replacement callbacks run inside an internal JNI local frame, and accepted object/array returns are promoted before returning to ART. `JavaMethod::replace` is safe; `JavaConstructor::replace` is safe through an initialization token returned by original-constructor calls, while unchecked constructor replacement remains explicit `unsafe`. Supports arbitrary non-constructor descriptors handled by the descriptor-driven arm64 closure path, with live coverage for multi-reference, mixed primitive, wide primitive, float-mix, array, stack-spill, safe argument iteration, Rust string returns, callback-local object/array returns, owned original object/array returns, and invalid object-return rejection. Unsupported signatures fail before installation with errors naming method kind, name, and a concise reason. |
| Calling original implementation from facade callbacks | Done | `JavaHookContext::{call_original,call_original_current,call_original_raw}`, `java_args!`, `JavaArgs` | Covered for tested static and instance lanes, including current-argument pass-through, long explicit argument lists, callback-local reference extraction, and retained owned object/array extraction. Thread-scoped raw original handles remain internal. |
| Replace/revert lifecycle | Done | `JavaHookGuard` | Dedicated app-process tests cover guard ownership, duplicate active replacement rejection, explicit revert, idempotent successful revert, and replace/revert/replace for selected methods. |
| Callback failure handling | Done | `JavaHookGuard::{on_error,set_error_handler,last_error,take_last_error}` plus JNI default fallback returns | Callback errors, panics, and wrong return kinds are recorded on the guard, optionally reported through an installed error handler as they happen, and cause Java callers to receive the JNI default value for the method return type. |
| Typed facade argument/return helpers | Done | `JavaHookContext::{arguments,arg_value,arg_display,arg_is_null,arg::<String>,this_object,arg_object,arg_array,return_value,call_original::<String>,call_original_object,call_original_array}`, `JavaHookArgument`, `JavaLocalObject`, `JavaLocalArray`, safe wrapper and string hook returns | Primitive/string conversions, diagnostic argument display, safe iterable argument views, callback-local reference views, typed array extraction, safe object/object-array wrapper returns, owned original object/array returns, invocation-local object/array return conversion, return assignability checks, and null-reference handling are covered through the public closure trampoline path. |
| Java-aware diagnostic display | Done | `java_display()` on Java objects, arrays, returns, hook arguments, and metadata wrappers | Provides console-log-style diagnostic text without implementing Rust `Display`. Values use direct primitive/null/void formatting and Java `Object.toString()` for references; arrays intentionally keep Java array `toString()` output. Metadata wrappers expose infallible class/member summaries. |
| Raw closure-backed replacement callbacks | Done | internal descriptor-driven arm64 closure backend | Raw invocation closures are crate-internal scaffolding for the public facade, startup hooks, and live-runtime harnesses. The generic trampoline captures register and stack-passed arguments into a `jvalue` frame and dispatches through one Rust decoder path. |
| Arbitrary signatures and multi-reference args | Done | guarded facade plus internal raw-closure coverage | Public `replace()` admits arbitrary non-constructor descriptors that fit the current closure replacement limits. Constructor replacement is admitted through the safe initialization-token facade or the explicit unsafe unchecked facade; broader raw JNI-native admission remains deferred. |
| Deoptimization | Partial | `Java::deoptimize_everything`, `Java::deoptimize_boot_image`, `JavaMethod::deoptimize`, `JavaConstructor::deoptimize`, `JavaCapabilities::deoptimization` | Android ART API 26+ arm64 path is implemented through boot-image deoptimization, API 30+ Instrumentation deoptimization, and API 26-29 Dbg/JDWP deoptimization. Capability reports unsupported with concrete missing prerequisites when the current ART symbols or layout probes are unavailable. Live app-process coverage exercises supported devices and verifies structured unsupported errors otherwise. |
| Register Java classes | Deferred | none | Upstream `registerClass()` parity is later than core loader/replacement work. |

## Deliberate Scope Differences

- Android ART behavior is the compatibility target. Dalvik, HotSpot, JVM TI, desktop JVM support,
  and a JavaScript compatibility layer are out of scope for now.

## Planning Boundary

Use `ROADMAP.md` for priority order and milestone planning. This tracker should change when feature
status changes, when a Rust API shape changes, or when an upstream concept is deliberately moved in
or out of scope.
