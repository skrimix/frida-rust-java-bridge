# frida-java-bridge-rs Roadmap

## Scope

This crate is a Rust-native Java runtime bridge for Frida, targeting Android ART only. It is a
re-implementation path for the useful Android ART surface of `frida-java-bridge`, not a line-by-line
port of GumJS.

The project is private and pre-user. Exported Rust APIs, module names, and documentation terms may
change when that makes the prototype clearer or the ART behavior safer. A "soft-frozen" API is only
useful and test-covered enough to avoid casual churn for now; it is not stable, finalized, or
externally promised.

In scope:

- ART runtime discovery, JavaVM access, and thread attachment
- explicit JNI reference ownership and exception handling
- descriptor parsing, value marshaling, and checked invocation
- class, object, array, method, and field operations through a Rust API
- class-loader-scoped lookup, app-loader selection, and metadata discovery
- Rust-native `Java.use`-style wrappers
- experimental guarded `replace` method replacement on supported ART layouts
- app startup deferral, main-thread scheduling, heap enumeration, and deoptimization once their ART
  paths are proven

Out of scope unless the project is deliberately rescoped:

- Dalvik, HotSpot, JVM TI, desktop JVMs, or a generic multi-runtime backend
- JavaScript API compatibility or line-by-line GumJS parity
- hiding loader, attachment, descriptor, or ownership boundaries behind implicit behavior

## References

- `CURRENT_BEHAVIOR.md`: current behavior notes and soft-freeze drafts
- `FEATURE_PROGRESS.md`: scan-friendly feature/status matrix aligned with upstream `PUBLIC_DOC.md`
- `ERGONOMICS_GAPS.md`: notes from porting representative Frida JS snippets into Rust examples
- `../frida-java-bridge`: primary behavior and ART internals reference
- `../frida-java-bridge/lib/vm.js`: JavaVM attach/detach model
- `../frida-java-bridge/lib/env.js`: JNI vtable wrapper reference
- `../frida-java-bridge/lib/types.js`: descriptor and value conversion reference
- `../frida-java-bridge/lib/class-factory.js`: wrapper, overload, loader, and replacement surface
- `../frida-java-bridge/lib/class-model.js`: class and method metadata reference
- `../frida-java-bridge/lib/android.js`: ART internals reference
- `../frida-gum`: Frida Gum source
- `../frida-rust/frida-gum`: Rust Gum bindings used for process/module discovery

`ROADMAP.md` should describe sequencing and priorities. Current behavior details belong in
`CURRENT_BEHAVIOR.md`; exhaustive feature coverage belongs in `FEATURE_PROGRESS.md`.

## Current Shape

Soft-frozen:

- Android ART-only bridge acquisition through `Java::obtain()` and internal
  `JNI_GetCreatedJavaVMs` runtime discovery
- `Java` Android release/API-level helpers
- low-level `Vm` attachment helpers exposed through `Java::vm()`
- synchronous `Java::perform_now()` callbacks
- low-level `Env` JNI wrappers for lookup, invocation, fields, strings, exceptions, and references
- typed `LocalRef` / `GlobalRef` ownership
- descriptor parsing and explicit `JavaValue` / `JavaReturn` marshaling
- owned `Java`, high-level `JavaClass`, raw `RawJavaClass`, `JavaObject`, and
  `JavaArray` convenience APIs
- explicit class-loader-backed lookup through `ClassLoaderRef` and per-`Java` class caches
- synchronous app-loader selection from `ActivityThread.currentApplication()`
- upstream-like default app-loader wrapper lookup through bare `Java::use_class()` once
  `Java::with_app_loader()` or `Java::perform()` has published the app loader, while
  `Java::find_class()` stays explicitly bootstrap-scoped on bare handles
- loaded-class, class-loader, reflection metadata, and method-query APIs on supported ART layouts
- `JavaCapabilities` reporting for ART enumeration, app-loader deferral, main-thread scheduling,
  method replacement, heap enumeration, and deoptimization
- experimental exact-class heap instance enumeration through `Java::choose_instances()` and
  `JavaClass::choose_instances()` on supported ART heap layouts
- Rust-native wrapper APIs through `Java::use_class()`, GumJS-style method selectors for
  single-overload methods, type-list and arity overloads, typed helpers, casts, constructor convenience
  helpers, `IntoJavaArgs`, wrapper-level `IntoJavaCallArgs`, callback-local borrowed object/array
  views, and diagnostic Java `Object.toString()` helpers
- explicit raw class access through `RawJavaClass` for descriptor-string calls and
  `JavaValue` slice paths that should not dominate the default facade
- the public wrapper-selected `JavaMethod::replace()` facade shape for arbitrary
  non-constructor descriptors accepted by the descriptor-driven arm64 closure trampoline, including
  explicit `JavaHookGuard` ownership,
  duplicate active replacement rejection, retryable explicit revert, callback error/panic
  recording, JNI default fallback returns, typed argument/original-call helpers, tested
  object/object-array/null reference handling, multi-reference, mixed primitive, wide primitive,
  float-mix, array argument/return, callback-local receiver/argument wrapping, and stack-spill
  shapes

Experimental:

- deferred `Java::perform()` callbacks for early app startup, installed through hidden ART
  replacement hooks on supported `LoadedApk` / `ActivityThread` startup methods
- main-thread scheduling through `Handler.sendEmptyMessage()` wakeups and a Gum `epoll_wait` drain
  hook
- exact-class heap instance enumeration through hidden ART `Heap::VisitObjects` /
  `Heap::GetInstances` paths, with callback stop support and explicit unsupported capability
  reporting
- clone-active ART method replacement for arbitrary descriptor-driven public closure lanes,
  including the first guarded constructor overload facade
- closure-backed startup hooks, captured-original handles, and backend replacement scaffolding kept
  behind the soft-frozen public facade for app-loader deferral and backend coverage

Known successful live gates include the app-process and APK early-start harnesses on the current
matrix of Quest 2 SDK 34, Pixel 8 Pro SDK 36, OPD2403 SDK 36, and Mi Max SDK 29. Treat that as a
snapshot, not a support claim.

## Active Priorities

### 1. Method Replacement Hardening

Goal: make the hidden ART replacement backend trustworthy enough for a small public experimental
surface.

Next work:

- keep `replacement::*` focused on the tested guarded wrapper-selected hook facade:
  `let activity = java.use_class("android.app.Activity")?;`,
  `let on_resume = activity.method("onResume")?;`,
  `let guard = unsafe { on_resume.replace(|ctx| { ctx.call_original_void(())?; Ok(()) })? };`.
  Raw closure, startup-hook, and captured-original scaffolding stays crate-internal
- keep the public facade's supported ABI admission errors explicit about method kind, method name,
  and a concise reason
- keep descriptor-driven arm64 closure replacement moving in stages:
  1. done: use `ClosureReplacementLayout` as the shared AAPCS64 argument/return map
  2. done: use one trampoline that captures register and stack-passed Java arguments into a
     `jvalue` buffer
  3. done: dispatch through one Rust callback path that decodes arguments from `MethodSignature`
     and writes the JNI return slot
  4. done: broaden public `replace()` admission for the descriptor-driven shapes
     already covered by live app-process checks
  5. done: admit arbitrary non-constructor public `replace()` descriptors through
     the descriptor-driven arm64 closure layout boundary
  6. done: expose guarded public constructor overload replacement through
     `JavaConstructor::replace()` with callback-local original-constructor
     calls, but without `$new` / `$alloc` ergonomics
- keep growing live coverage before broader constructor ergonomics; `$new` / `$alloc` allocation
  ergonomics remain intentionally unsupported
- keep callback-local reference helpers focused on borrowed views plus explicit `retain()`; avoid
  hiding JNI lifetime boundaries behind JS-style object proxies
- preserve explicit guard ownership as the Rust lifecycle; reject overlapping replacements for the
  same resolved `ArtMethod`
- investigate any Java stack-trace or quick-frame abort as a replacement-integrity failure, not as
  a harmless test issue
- document the supported Android/API/device matrix before declaring a wider replacement milestone

### 2. Main-Thread Scheduling

Goal: turn the current scheduler prototype into a predictable app-process primitive.

Next work:

- harden the `epoll_wait` drain point and main-looper wakeup behavior in APK-process validation
- keep command-line `app_process` reporting `UnsupportedFeature` when `Looper.getMainLooper()` is
  null
- keep capability probing side-effect-light: no hook installation, callback enqueue, or looper
  wakeup during `JavaCapabilities` checks
- decide whether `MainThreadTaskHandle` / status reporting is soft-frozen after more matrix runs

### 3. App-Loader Deferral

Goal: support early app startup without guessing the app loader.

Next work:

- treat synchronous app-loader lookup as available only when `ActivityThread.currentApplication()`
  has a real `Application`
- keep bare `Java::use_class()` using the published default app loader without changing
  `Java::find_class()` bootstrap semantics
- use `AppClassLoaderUnavailable` instead of falling back to thread-context or enumerated loaders
- keep deferred startup hook support tied to explicit replacement and Android startup-hook
  capability probes
- add an explicit override path only if real Rust call sites need one

### 4. Loader And Metadata Coverage

Goal: keep loader and metadata APIs reliable while ART layout support expands.

Next work:

- harden class-loader and loaded-class enumeration against OEM layout differences
- keep method-query syntax compatible where it helps Rust workflows, not for JS parity by default
- extend lower-level ART metadata only when wrapper or replacement work needs it
- keep unsupported layouts visible through structured errors and capability reports

### 5. Test Coverage And Harness Hygiene

Goal: make regressions obvious without spreading live-runtime checks into the wrong harness.

Next work:

- add host-testable unit coverage for descriptor, marshaling, ownership, and guard invariants as
  they become modelable without a live VM
- put normal live-runtime behavior in the app-process harness
- keep `src/bin/art_test.rs` limited to native ART bootstrap and manual VM creation coverage
- leave failing runtime capabilities visible unless a limitation is an intentional prototype
  boundary; document intentional boundaries in this file or `CURRENT_BEHAVIOR.md`

## Deferred Work

- subclass-inclusive heap enumeration and broader heap enumeration matrix hardening
- deoptimization, with capability reporting and live-runtime tests
- broader Android-version replacement support beyond the proven arm64 ART path
- more polished replacement ergonomics once backend safety is less volatile
- Java backtraces, dex loading, and class registration
- full `ClassFactory` manager semantics, including cache directory policy, temp-file naming,
  `openClassFile()`, allocation-only `$alloc`, and init-only `$init`
- broader cache sharing keyed by loader identity plus class name beyond the default app-wrapper
  cache, if ownership broadens beyond per-`Java` caches

## Module Map

- `src/lib.rs`: Android-gated modules and re-exports
- `src/android.rs`: Android release/API-level property helpers
- `src/modifiers.rs`: public Java access-flag constants
- `src/runtime.rs`: ART module discovery and JavaVM acquisition
- `src/vm.rs`: JavaVM wrapper and thread attachment
- `src/env.rs`: JNI vtable calls, method/field references, invocation, and exception handling
- `src/java/`: owned Rust-native convenience layer with class/object wrappers and ID caches
- `src/java/main_thread.rs`: experimental main-thread detection and scheduling queue
- `src/replacement/`: experimental method replacement facade plus crate-internal closure,
  original-call, backend, and trampoline scaffolding
- `src/refs.rs`: typed local/global JNI reference wrappers
- `src/signature.rs`: Java type and method descriptor parsing
- `src/value.rs`: explicit JNI value representation and argument validation
- `src/jni.rs`: local raw JNI definitions and vtable slot constants
- `src/error.rs`: shared error and result types
- `src/art/`: Android ART backend internals
- `src/bin/art_test.rs`: native ART bootstrap test harness
- `src/app_process_test.rs`: primary app-process live-runtime test harness
- `src/app_process_test/`: app-process checks, replacement checks, callback helpers, and assertions
- `test-fixtures/`: Java source, app-process jar, and generated DEX used by runtime checks

## Verification

Use `cargo ndk` for build, check, and Android test workflows. Prefer `justfile` recipes:

- `just check`: Android arm64 clippy
- `just build`: Android arm64 debug build
- `just unit-test all`: Android arm64 unit tests through `cargo-ndk-runner`
- `just test all`: primary app-process ART harness through `adb`
- `just apk-perform-test all`: APK startup-agent deferred `perform()` test
- `just art-test all`: native ART bootstrap test harness

Run `just test` for live-runtime changes touching app-loader lookup, JNI vtable access, exception
handling, metadata/enumeration, method replacement, main-thread scheduling, or reference ownership.
Run `just art-test` for native ART loading, manual VM creation, startup signal-chain behavior, or
bootstrap-only VM attachment.

## Design Principles

- Prefer Rust-native APIs over cloning GumJS shapes.
- Keep attachment, loader scope, descriptors, ownership, and errors explicit.
- Use upstream `frida-java-bridge` as the behavior and feature-boundary reference.
- Add higher-level ergonomics only after the lower-level ART behavior is understood and tested.
