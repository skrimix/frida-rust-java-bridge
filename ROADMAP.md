# frida-java-bridge-rs Roadmap

## Scope

This crate is a Rust-native Java runtime bridge for Frida, targeting Android ART only. It is a
re-implementation path for the useful Android ART surface of `frida-java-bridge`, not a line-by-line
port of the GumJS implementation. Rust-native `Java.use`-style wrappers, app class-loader
selection, method replacement, heap/deoptimization features, and other ART-backed bridge behavior
are in scope even when their APIs are allowed to differ from GumJS.

This is a private pre-user experiment. There are no stable public contracts yet, and exported Rust
APIs may change freely when that makes the prototype clearer. Roadmap and behavior docs are planning
notes and current snapshots. "Soft-frozen" means useful and test-covered enough to avoid casual
churn for now, not finalized or externally promised.

The practical goal is to provide:

- explicit ART runtime discovery and JavaVM access
- thread attachment and `JNIEnv` access
- predictable local/global reference ownership
- descriptor parsing and explicit JNI value marshaling
- class, object, method, and field operations through a Rust API
- class-loader support, metadata discovery, and ART method replacement
- Rust-native `Java.use`-style wrappers and `.implementation`-style replacement ergonomics
- app-loader selection helpers, heap enumeration, and deoptimization once their ART paths are clear

Other Java runtimes are not a roadmap target. Dalvik, HotSpot, JVM TI, desktop JVMs, and a generic
multi-runtime backend should stay out of the plan unless the project is deliberately rescoped.

## Reference Paths

- `CURRENT_BEHAVIOR.md`: current behavior notes and soft-freeze drafts
- `../frida-java-bridge`: behavior and feature boundary reference
- `../frida-java-bridge/lib/vm.js`: JavaVM attach/detach model
- `../frida-java-bridge/lib/env.js`: JNI vtable wrapper reference
- `../frida-java-bridge/lib/types.js`: Java descriptor and value conversion reference
- `../frida-java-bridge/lib/class-factory.js`: wrapper, overload, loader, and replacement surface reference
- `../frida-java-bridge/lib/class-model.js`: class and method metadata reference
- `../frida-java-bridge/lib/android.js`: ART internals reference
- `../frida-gum`: Frida Gum source
- `../frida-rust/frida-gum`: Rust Gum bindings used for process/module discovery

`ROADMAP.md` is the source of truth for sequencing and project direction. For a scan-friendly
feature/status matrix aligned with upstream `PUBLIC_DOC.md`, see `FEATURE_PROGRESS.md`; that file
should describe current coverage, not carry a separate priority plan.

## Progress Snapshot

### Done

- Android ART is the only active runtime target.
- `Runtime::obtain()` discovers `libart.so`, resolves `JNI_GetCreatedJavaVMs`, and exposes the current VM.
- `Vm` supports `GetEnv`, `AttachCurrentThread`, and `DetachCurrentThread`.
- `Env` exposes low-level JNI access for class lookup, strings, exceptions, local/global references,
  constructors, instance/static methods, and instance/static fields.
- Typed `LocalRef` and `GlobalRef` wrappers manage JNI reference ownership.
- `JavaType`, `MethodSignature`, and `JavaValue` cover descriptor parsing, argument validation, and
  explicit JNI argument marshaling.
- `Java`, `JavaClass`, and `JavaObject` provide an owned, descriptor-explicit convenience layer over
  the low-level `Env` API, including global references and per-class method/field ID caches.
- `JavaArray` provides owned array references with object-array element helpers, primitive-array
  constructors/accessors for all JNI primitive element types, `JavaValue` argument conversion, and
  `JavaReturn::Array` extraction for array-returning methods and fields.
- `Java` supports opt-in loader-aware lookup through explicit `ClassLoaderRef` values. Bootstrap and
  loader-backed `Java` instances keep separate successful class caches.
- Synchronous app-loader selection is exposed through `Java::app_class_loader()`,
  `Java::with_app_loader()`, `Runtime::app_java()`, and `Vm::app_java()`. It uses
  `ActivityThread.currentApplication().getClassLoader()` when an app `Application` is already
  available and reports `AppClassLoaderUnavailable` instead of guessing when it is not. The
  experimental `Java::perform()`/`Runtime::perform()`/`Vm::perform()` path queues callbacks and
  drains them once the app loader is available through hidden ART replacement hooks on
  `LoadedApk.makeApplicationInner`/`makeApplication` and selected `ActivityThread.getPackageInfo`
  overloads when method-replacement prerequisites are present.
- ART class-loader enumeration has a current Rust API and a hardened API 26+ arm64 ART backend path
  using Runtime layout discovery, an `ExceptionClear`-based runnable-thread transition,
  `VisitClassLoaders`, `SuspendAll`/`ResumeAll`, and `JavaVMExt::AddGlobalRef`.
  Unsupported layouts and older APIs return structured
  `UnsupportedFeature` errors.
- The current metadata layer exposes loaded-class enumeration, per-class reflection metadata for
  declared constructors, methods, and fields, and a typed method-query helper layered on top of
  loaded-class enumeration.
- Loader and metadata behavior notes are documented, including class-loader cache isolation,
  `ClassLoaderKind`, method-query syntax, dotted user-facing class names, and unsupported-feature
  behavior.
- ART capability reporting is exposed through `Runtime`, `Vm`, and `Java`, with class-loader and
  loaded-class enumeration probed against the current ART layout, method replacement reported as
  experimental when its current prerequisites are available, app-loader deferral and main-thread
  scheduling reported through their own side-effect-light experimental/unsupported probes, and
  deferred advanced features reported as unsupported until their ART prototypes are ready to expose.
- Loader, metadata, and capability APIs are soft-frozen for the current test-covered shape.
- Android-targeted unit tests cover descriptor formatting, argument validation, JNI value marshaling,
  method/field guard behavior, class-name normalization, and unsupported runtime-layout outcomes
  where no live VM is required.
- `src/bin/art_test.rs` is intentionally limited to native ART bootstrap coverage: loading
  `libart.so`, calling `JNI_CreateJavaVM`, obtaining the created VM through `Runtime::obtain()`,
  attaching a thread, and running a small bootstrap-class JNI/convenience sanity check. It
  deliberately avoids full capability/method-replacement probing because manually created ART VMs
  can expose vendor framework initialization paths that are outside the normal app-process bridge
  surface; capability and replacement checks belong in the app-process harness.
- ART method replacement prerequisite probing now reaches the hidden-backend boundary across the
  current test matrix, including Mi Max SDK 29, newer SDK 34/36 ClassLinker layouts, and OPD2403's
  runtime-decorated native method flags.
- The app-process test target is the primary live-runtime gate for normal bridge behavior. It runs
  inside an already-created ART process with an app-provided class loader and covers low-level JNI
  helpers, convenience wrappers, explicit app-loader lookup, DexClassLoader lookup, metadata,
  loaded-class and class-loader enumeration, deferred `Java::perform()` hook setup when
  `ActivityThread.currentApplication()` is still null, capability reporting for app-loader
  deferral and main-thread scheduling, and experimental replacement checks. Command-line
  `app_process` currently reports main-thread scheduling as unsupported because
  `Looper.getMainLooper()` is null there; the APK early-start harness is the live main-looper drain
  gate.
- A dedicated APK startup-agent harness validates the deferred `Java::perform()` path from an
  early app bind point: it loads the bridge with `am start-activity --attach-agent-bind`, confirms
  synchronous app-loader lookup is unavailable before `Application` creation, queues a callback,
  proves it drains through the real app loader after `LoadedApk.makeApplication*`, and now uses the
  Rust main-thread scheduler to finish validation from Android's main thread. The APK early-start
  gate passes on the current matrix: Quest 2 SDK 34, Pixel 8 Pro SDK 36, OPD2403 SDK 36, and Mi Max
  SDK 29.
- Experimental main-thread helpers expose `is_main_thread()` and `schedule_on_main_thread()` on
  `Java`, `Runtime`, and `Vm`. Scheduling keeps upstream-like queue semantics, wakes the main looper
  with `Handler.sendEmptyMessage()`, and drains through a Gum `epoll_wait` hook while preserving the
  caller's loader-scoped `Java` handle. Main-thread scheduling now has explicit capability
  reporting for its probeable prerequisites without installing hooks or waking the looper.
- A hidden experimental ART method replacement prototype now makes cloned `ArtMethod` dispatch the
  active test path for selected static and instance methods: no-arg primitive/`void`, no-arg
  `String` return, all currently exposed static and instance no-arg primitive return lanes, mixed
  primitive/wide static and instance argument signatures, `String` argument/return paths, and
  one-reference-argument/reference-return paths covering `Object`, object arrays, typed app classes,
  and null JNI values. The `()I`, `()Z`, primitive-argument, `String -> String`, and reference echo paths include
  cached-class and wrapper call coverage where useful; clone patching, clone-active dispatch,
  GC-during-active replacement, and restore validate executable replacement prerequisites and run
  under ART thread suspension when available. Clone-active quick dispatch now routes the original
  method through an executable cloned-method thunk instead of trying to continue through ART's
  interpreter bridge with the replacement clone. The thunk can detect replacement-originated JNI
  calls through ART's linked managed stack and dispatch hidden raw original calls through ART's
  quick-to-interpreter bridge without globally reverting the hook. Original-call bypass is scoped to
  the target ART thread and method, and test coverage now includes selected static/instance
  primitive, `String`, and reference argument/return paths, including object arrays and null JNI
  values. Generated
  executable thunks are flushed from the instruction cache before use. Internal overload-first,
  raw closure, and descriptor-driven JNI-native scaffolding can replace selected
  `JavaMethodOverload` values and call originals through captured overload metadata with generic
  `IntoJavaArgs` argument containers and typed raw-return extraction. Dedicated lifecycle test
  coverage now exercises replace/revert/replace on the same static and instance `ArtMethod`
  through direct helpers, internal raw layers, and the public `.implementation` facade, and
  overlapping active replacements for the same resolved `ArtMethod` are rejected. The public
  overload replacement surface is now intentionally pruned to unsafe
  `JavaMethodOverload::implementation`, `ImplementationInvocation`, `ImplementationReturn`, and
  `ImplementationGuard`; original calls are exposed through `ImplementationInvocation::call_original`.
  Callback failures are recorded on the replacement guard before returning JNI default values.
  Explicit guard ownership is the intended Rust lifecycle instead of a JS-style setter slot;
  explicit reverts are retryable on failure, and drop-time restore failure keeps replacement
  executable state mapped instead of leaving ART with freed thunk memory. Broader replacement
  ergonomics remain planned.
- Verification recipes exist in `justfile` for Android arm64 check/build/test workflows.

### In Progress

- Loader lookup remains explicit by default; synchronous app-loader-scoped handles are available,
  and an experimental deferred `Java.perform()`-style queue exists for early app startup. It
  currently depends on the hidden ART replacement backend and Android startup hook points around
  `LoadedApk.makeApplication*` and selected `ActivityThread.getPackageInfo` overloads. The
  app-process harness now validates hook
  installation and pending callback behavior when `ActivityThread.currentApplication()` is null;
  the APK startup-agent harness validates callback drain after the real app `Application` appears
  and schedules follow-up work onto Android's main thread.
- Main-thread scheduling has a first experimental Rust surface and needs device-matrix hardening
  around the `epoll_wait` drain point and main-looper wakeup behavior. Its public handle/status
  shape is being treated as a soft-freeze candidate while capability reporting keeps missing
  prerequisites explicit.
- Test coverage is the main live-runtime gate; host-testable units cover non-runtime parsing,
  validation, marshaling, and guard behavior.
- Clone-active replacement and deferred app-loader hook setup pass the current app-process test
  matrix on Quest 2 SDK 34, Pixel 8 Pro SDK 36, OPD2403 SDK 36, and Mi Max SDK 29. Direct-helper and
  public `.implementation` replace/revert/replace lifecycle tests now pass on that matrix. The
  guarded implementation facade is implemented for selected currently supported ABI lanes and needs
  matrix hardening. Broader ART instrumentation parity remains incomplete; arbitrary replacement
  signatures beyond the currently tested primitive/`String`/single-reference lanes and finished
  replacement ergonomics are still planned work.

### Next

- Keep the APK early-start `Java.perform()` validation in the current matrix gate and keep using
  `RuntimeCapabilities::app_loader_deferral` as the readiness signal. Synchronous app-loader
  selection should keep returning explicit unavailable errors when no `Application` exists yet, and
  the scheduled main-thread callback should remain the final success signal.
- Keep hardening the experimental main-thread scheduler through APK-process validation, using
  `RuntimeCapabilities::main_thread_scheduling` to keep missing hook or wakeup support visible as
  structured errors.
- Keep hardening the hidden clone-active replacement prototype across the native and app-process
  test matrix. Keep arbitrary object/multi-reference signatures, broader implementation/backend
  signature support, and richer replacement APIs on the plan, gated on broader quick-dispatch
  instrumentation.
- Harden the public guarded `.implementation` path and its internal raw closure-backed scaffolding
  across the current app-process and APK early-start matrix; keep callback failure reporting
  explicit and default-return behavior documented.
- Keep object/reference and array ergonomics aligned with real replacement or wrapper workflows;
  primitive/object array construction and extraction now have a first explicit Rust surface.
- Keep repeated replacement lifecycle behavior test-covered with dedicated fixture methods. The
  isolated replace/revert/replace case now passes across the current device matrix; investigate
  future lifecycle failures as backend cleanup or ART-dispatch regressions instead of hiding them.
- Keep method replacement APIs experimental until a broader backend/API exists, with capability
  reporting distinguishing experimental availability from unsupported missing prerequisites.
- Keep loader and metadata behavior hardened against device-specific ART layouts, large class sets,
  query-shape edge cases, and capability/error consistency.
- Broaden host-testable unit coverage around ownership and ART-layout invariants where they can be
  modeled safely.

### Later

- Hardened deferred app-loader initialization and app-loader-scoped default `Java` workflows for
  early app startup.
- More complete Rust-native `Java.use`-style ergonomics, including overload/member surfaces that
  are comfortable to use without hiding loader boundaries.
- More polished `.implementation`-style replacement ergonomics once the hidden ART backend is safe
  enough to expose beyond the current guarded experimental subset.
- Heap enumeration and deoptimization on ART, with explicit capability reporting and test coverage.
- Broader ART device/version hardening for loader enumeration, metadata, and replacement.
- Convenience APIs such as Java backtraces, dex loading, and class registration after the core
  loader/replacement/metadata paths are less volatile.

## Current Module Shape

- `src/lib.rs`: current Android-gated modules and re-exports
- `src/runtime.rs`: ART module discovery and JavaVM acquisition
- `src/vm.rs`: JavaVM wrapper and thread attachment
- `src/env.rs`: JNI vtable calls, method/field references, invocation, and exception handling
- `src/java.rs`: owned Rust-native convenience layer with class/object wrappers and ID caches
  - `src/java/main_thread.rs`: experimental main-thread detection and scheduling queue
- `src/refs.rs`: typed local/global JNI reference wrappers
- `src/signature.rs`: Java type and method descriptor parsing
- `src/value.rs`: explicit JNI value representation and argument validation
- `src/jni.rs`: local raw JNI definitions and vtable slot constants
- `src/error.rs`: shared error and result types
- `src/art/`: Android ART backend internals, split by concern:
  - `mod.rs`: shared ART types, symbols, and facade wiring
  - `backend.rs`: `ArtBackend` entrypoints for enumeration, method query, and replacement
  - `enumeration.rs`: class-loader, loaded-class, and method-query visitors/processors
  - `layout.rs`: ART runtime/ClassLinker/ArtMethod layout probing and patch helpers
  - `replacement.rs`: hidden clone-active method replacement controller, hooks, guard, and thunk
    generation
  - `runnable_thread.rs`: runnable ART thread transition wrapper
  - `runnable_thread/arm64.rs`: AArch64 transition recompilation and instruction decoding helpers
  - `support.rs`: std-string, memory-range, symbol-resolution, suspend-all, and native support
    helpers
- `src/bin/art_test.rs`: native ART bootstrap test harness
- `src/app_process_test.rs`: primary app-process live-runtime test harness, compiled into the
  cdylib with the `app-process-test` feature; detailed checks live under
  `src/app_process_test/`:
  - `checks.rs`: low-level JNI, convenience, loader, DexClassLoader, and metadata checks
  - `replacement_checks.rs`: main hidden replacement test flow
  - `replacement_lifecycle.rs`: replace/revert/replace lifecycle replay checks
  - `assertions.rs`: shared test assertions and mismatch helpers
  - `replacement_callbacks.rs`: JNI-native replacement callback functions
- `test-fixtures/`: Java source, app-process jar, and generated DEX used by test checks; rebuild
  with `just app-process-test-dex`
- `CURRENT_BEHAVIOR.md`: current loader/metadata behavior notes and soft-freeze drafts

## Milestones

### 0. Skeleton And Scope

Status: complete.

Delivered:

- crate structure
- error model
- Android-only runtime scope
- local JNI definitions
- Android arm64 build recipes

### 1. VM And Env Core

Status: complete.

Delivered:

- ART `JNI_GetCreatedJavaVMs` discovery
- `JavaVM *` and `JNIEnv *` wrappers
- `GetEnv`, `AttachCurrentThread`, and `DetachCurrentThread`
- basic exception handling
- class lookup
- string creation and copying
- local/global reference helpers

Reference: `../frida-java-bridge/lib/vm.js`, `../frida-java-bridge/lib/env.js`.

### 2. Values, Signatures, And Explicit Invocation

Status: complete for the current low-level API.

Delivered:

- Java type descriptor parser
- method signature parser
- explicit JNI value enum
- argument count/type validation
- constructor lookup and object construction
- instance/static method lookup and primitive/object/void invocation
- instance/static field lookup and primitive/object reads/writes

Remaining polish:

- keep improving ergonomic conversions where real wrapper/replacement call sites expose friction
- keep adding unit tests for new descriptor and argument validation edge cases as they appear

Reference: `../frida-java-bridge/lib/types.js`.

### 3. Rust-Native Reflection Layer

Status: soft-frozen; further reflection ergonomics remain incremental.

Goal:

Make common Java interaction possible without every caller manually threading together `Env`,
`ClassRef`, `MethodId`, and `FieldId`, while keeping descriptors and JNI value conversion explicit.

Delivered:

- `Runtime::java()` and `Vm::java()` entrypoints
- owned `JavaClass` and `JavaObject` wrappers backed by JNI global references
- explicit-signature constructor, method, static method, field, and static field helpers
- per-class caches for looked-up constructor, method, and field IDs
- test coverage for class lookup, strings, calls, fields, caching, and exception handling

Moved to later milestones:

- JS-style overload dispatch
- Rust-native `Java.use`-style wrappers
- method replacement
- automatic app class-loader selection

Reference: `../frida-java-bridge/lib/class-factory.js`.

### 4. Class Loaders And App Class Resolution

Status: soft-frozen.

Goal:

Resolve non-boot classes and model class-loader-specific identity.

Delivered:

- introduce `ClassLoaderRef`
- support explicit loader-aware class lookup through `ClassLoader.loadClass()` and array descriptor
  lookup through `Class.forName(name, false, loader)`
- isolate successful class lookup caches per `Java` instance
- expose user-facing class wrapper names as Java binary names while keeping JNI descriptors
  slash-style
- add JNI object-class/type helpers used by loader validation
- add a DexClassLoader test fixture proving explicit loader lookup can resolve a non-bootstrap class
- add an API 26+ arm64 ART loader-enumeration backend path
- add synchronous app-loader selection from `ActivityThread.currentApplication()` with
  app-loader-scoped `Java` handles and explicit unavailable errors
- add an experimental deferred app-loader queue through `Java::perform()`, `Runtime::perform()`,
  and `Vm::perform()`, backed by hidden Android startup replacement hooks when immediate app-loader
  lookup is unavailable
- document loader-backed lookup semantics, cache isolation, `ClassLoaderKind`, and current
  object-wrapper boundaries

Future work:

- validate deferred app-loader selection for early app startup across more devices, keep
  unsupported-backend errors explicit, and add explicit override behavior if callers need it
- keep hardening unsupported-layout and missing-symbol behavior as more devices are tested
- key shared caches by loader identity plus class name only if cache ownership broadens
- broaden loader enumeration support beyond the current API 26+ arm64 milestone

Reference: `../frida-java-bridge/index.js`, `../frida-java-bridge/lib/class-factory.js`.

### 5. Metadata Discovery

Status: soft-frozen.

Goal:

Discover loaded classes and inspect method/field metadata on supported runtimes.

Delivered:

- typed `JavaClassMetadata`, `JavaMethodMetadata`, and `JavaFieldMetadata`
- reflection-backed declared constructor, method, and field metadata
- ART loaded-class enumeration through `ClassLinker::VisitClasses`
- loaded-class enumeration reads ART class descriptors directly during class-linker visits and
  avoids JNI/reflection class-name lookup while visiting loaded classes
- ART-direct method queries use class descriptors, loader heap references, ART method arrays,
  access flags, and `ArtMethod::PrettyMethod`, with reflection fallback when direct prerequisites
  are unavailable
- query helper for `class!method` patterns with `/i`, `/s`, and `/u` modifiers
- upstream-compatible dotted class names for loaded-class and method-query metadata output
- test coverage for DexClassLoader metadata, overloads, fields, loaded-class enumeration, and
  method queries

Future work:

- continue hardening ART loaded-class enumeration across Android versions and OEM builds
- decide whether to extend lower-level ART layout metadata to declared fields/wrapper metadata
  before method replacement
- expand query compatibility only where it helps real Rust workflows

Reference: `../frida-java-bridge/lib/class-model.js`.

### 6. ART Capability Reporting

Status: soft-frozen.

Goal:

Make ART feature support explicit without introducing a premature multi-runtime backend boundary.

Delivered:

- expose `RuntimeCapabilities` through `Runtime`, `Vm`, and `Java`
- report current support for ART class-loader and loaded-class enumeration using the same symbol and
  layout probes as the enumeration APIs
- report app-loader deferral and main-thread scheduling independently from raw method replacement,
  using side-effect-light probes that do not install hooks, enqueue callbacks, or wake the main
  looper
- cover unsupported runtime-layout outcomes with host-testable seams
- report method replacement as experimental when prerequisites are present and as explicitly
  unsupported when a prerequisite is missing
- report heap enumeration and deoptimization as explicit unsupported features

Future work:

- keep unsupported runtime behavior explicit in errors
- keep replacement, heap, and deoptimization capability reports aligned with the actual ART backend
  probes before attempting internals

This capability layer should stay ART-focused. Do not add HotSpot/JVM TI capability placeholders or
a generic backend abstraction unless the project is intentionally rescoped away from Android ART.

### 7. Java.use-Style Wrapper Layer

Status: soft-frozen for wrapper ergonomics; replacement APIs remain experimental.

Goal:

Add a permanent Rust-native wrapper surface inspired by upstream `ClassFactory.use()` without
claiming drop-in GumJS parity.

Delivered:

- `Java::use_class()` resolves a class in the current loader scope and returns a `JavaClassWrapper`
- `JavaClassWrapper` exposes class name, underlying `JavaClass`, constructors, methods, and fields
- wrapper calls validate explicit overload signatures against reflection metadata before delegating
  to existing constructor, method, and field helpers
- `JavaConstructorOverload`, `JavaMethodOverload`, and `JavaFieldHandle` provide Rust-native
  overload/member handles selected by argument type lists and field names
- wrapper overload selectors accept both `JavaType` values and descriptor/source-style type names
- wrapper metadata and resolved JNI IDs are cached through the wrapper and underlying class layer
- wrapper metadata snapshots expose cached declared methods and fields directly
- object helpers support explicit retain, runtime type checks, and cast validation without inferring
  class-loader identity
- `JavaReturn` exposes typed extractors for ergonomic result handling
- wrapper and overload calls accept `IntoJavaArgs` containers, including unit, tuples, arrays,
  slices, and vectors, and selected overload/field handles expose typed convenience helpers for
  common primitive, object, and string-return paths
- test coverage exercises bootstrap and DexClassLoader-backed wrappers

Remaining work:

- broader replacement ergonomics beyond the current guarded experimental `.implementation` subset

### 8. Hooking And ART Advanced Features

Status: in progress.

Goal:

Prototype a narrow, documented method interception or replacement path on ART.

Delivered so far:

- harden upstream-aligned ART method replacement prerequisite probes first
- validate runtime/ClassLinker layout candidates before reporting replacement readiness
- use ART's exported ClassLinker quick-entrypoint predicates as a fallback when newer layouts no
  longer expose the upstream intern-table anchor within the old scan window
- handle direct vs indirect JNI method IDs using ART's `Runtime.jni_ids_indirection_`
- hidden clone-active replacement for selected static and instance primitive, `String`, and
  one-reference-argument/reference-return methods, including object-array reference test coverage
- raw original invocation from replacements using a thread-scoped ART bypass
- test coverage for cached classes, wrappers, GC-during-active replacement, object arrays, null JNI values,
  restore, duplicate active replacement rejection, and isolated replace/revert/replace lifecycle
  checks through direct helpers and the overload facade
- ART capability reporting marks method replacement experimental when prerequisites are available
  and unsupported when a prerequisite is missing
- internal overload-based replacement scaffolding for selected `JavaMethodOverload` values, backed
  by explicit JNI-native callback variants, a descriptor-driven raw JNI-native layer, overload
  metadata for original calls, generic original-call arguments, and typed raw-return extraction
- selected overloads expose only unsafe `JavaMethodOverload::implementation` as the public
  experimental facade. It keeps the same ABI subset while providing friendlier invocation,
  `ImplementationInvocation::call_original`, borrowed object/array return helpers, and an explicit
  `ImplementationGuard`. Internal raw/native helpers remain available to the test harness and
  deferred app-loader hooks. Callback errors or panics are stored on the guard and return JNI
  default values.

Planned work:

- richer replacement ergonomics beyond the current guarded `.implementation` wrapper, while keeping
  explicit guard ownership as the default Rust lifecycle
- continue integrating replacement ergonomics with the Rust-native wrapper layer without exposing
  the internal unsafe JNI-native helper tiers
- document the supported Android matrix before expanding it
- keep isolated test coverage for replacing, reverting, and replacing the same `ArtMethod` again;
  use any future failure to debug stale clone/thunk/controller state left by restore
- investigate Java exception stack-trace aborts as a replacement-integrity signal: when ART sees a
  patched quick frame it cannot map back to a Dex PC, native harness failures should remain visible
  and the replacement backend should either teach ART about the frame or avoid creating it
- arbitrary object/multi-reference signatures and closure-backed replacement callbacks beyond the
  exact startup-hook ABIs admitted for `Java.perform()`
- deoptimization support needed to make replacement behavior predictable across interpreted,
  JIT-compiled, and quick-compiled call paths

Reference: `../frida-java-bridge/lib/android.js`.

## Non-Goals For Now

- non-ART Java runtime support, including Dalvik, HotSpot, JVM TI, and desktop JVMs
- a generic multi-runtime backend abstraction
- line-by-line GumJS implementation parity or a JavaScript API compatibility layer
- transparent JS-style overload dispatch before explicit Rust-native overload APIs are proven
- broad Android-version method replacement before a narrow path is proven

## Testing Strategy

Use `cargo ndk` for build, check, and test workflows.

Current gates:

- `just check`: Android arm64 clippy
- `just unit-test-build`: Android arm64 unit-test binary compilation
- `just unit-test`: build, deploy, and run unit test
- `just host-test`: build and run the Android arm64 unit tests through `cargo-ndk-runner`
- `just test-build`: build the primary app-process ART test artifacts
- `just build`: Android arm64 debug build
- `just test`: build, deploy, and run the primary app-process ART test harness through `adb`
- `just app-test`: compatibility alias for the app-process ART test harness
- `just apk-perform-test`: build, deploy, and run the APK startup-agent deferred `perform()` test
- `just art-test`: build, deploy, and run the native ART bootstrap test harness through `adb`

Add host-testable unit tests where behavior does not require a live VM:

- signature parsing
- descriptor formatting
- argument validation
- reference ownership rules where they can be modeled safely

Keep Android runtime checks in the test harness until a dedicated integration-test layout exists.
New runtime test coverage should go in the app-process harness unless it specifically validates
native ART startup or manual VM creation.

## Design Principles

- Prefer a Rust-native API over cloning the GumJS API.
- Keep low-level APIs explicit about thread attachment, signatures, ownership, and errors.
- Allow higher-level helpers later, but make attachment and loader boundaries visible.
- Use the upstream Java bridge as the behavioral reference, especially for feature boundaries and
  ART internals, while choosing Rust structures that fit this crate.
