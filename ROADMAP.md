# frida-java-bridge-rs Roadmap

## Scope

This crate is a Rust-native Java runtime bridge for Frida, currently targeting Android ART only.
It is a re-implementation path for a useful subset of `frida-java-bridge`, not a line-by-line port
and not an early attempt at GumJS `Java` API parity.

The practical goal is to provide:

- explicit ART runtime discovery and JavaVM access
- thread attachment and `JNIEnv` access
- predictable local/global reference ownership
- descriptor parsing and explicit JNI value marshaling
- class, object, method, and field operations through a Rust API
- a later path toward class-loader support, metadata discovery, and ART method replacement

## Reference Paths

- `../frida-java-bridge`: behavior and feature boundary reference
- `../frida-java-bridge/lib/vm.js`: JavaVM attach/detach model
- `../frida-java-bridge/lib/env.js`: JNI vtable wrapper reference
- `../frida-java-bridge/lib/types.js`: Java descriptor and value conversion reference
- `../frida-java-bridge/lib/class-factory.js`: wrapper, overload, loader, and replacement surface reference
- `../frida-java-bridge/lib/class-model.js`: class and method metadata reference
- `../frida-java-bridge/lib/android.js`: ART internals reference
- `../frida-gum`: Frida Gum source
- `../frida-rust/frida-gum`: Rust Gum bindings used for process/module discovery

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
- `Java` supports opt-in loader-aware lookup through explicit `ClassLoaderRef` values. Bootstrap and
  loader-backed `Java` instances keep separate successful class caches.
- ART class-loader enumeration has a public API and a hardened API 26+ arm64 ART backend path using
  Runtime layout discovery, an `ExceptionClear`-based runnable-thread transition,
  `VisitClassLoaders`, `SuspendAll`/`ResumeAll`, and `JavaVMExt::AddGlobalRef`.
  Unsupported layouts and older APIs return structured
  `UnsupportedFeature` errors.
- Metadata V1 exposes loaded-class enumeration, per-class reflection metadata for declared
  constructors, methods, and fields, and a typed method-query helper layered on top of loaded-class
  enumeration.
- Android-targeted unit tests cover descriptor formatting, argument validation, JNI value marshaling,
  method/field guard behavior, and class-name normalization where no live VM is required.
- `src/bin/art_smoke.rs` creates an in-process ART VM and verifies runtime discovery, VM attachment,
  class lookup, string round trips, object construction, instance/static calls, field access, and
  Java exception handling through both low-level and convenience APIs.
- Verification recipes exist in `justfile` for Android arm64 check/build/smoke workflows.

### In Progress

- The convenience API is intentionally explicit: callers still provide descriptors and `JavaValue`
  arguments, while the wrapper layer owns global references and caches looked-up IDs.
- Loader lookup is explicit only; automatic app-loader selection and `Java.use()` parity remain out
  of scope.
- Loader V1 is in stabilization: public contracts, cache boundaries, unsupported-feature errors,
  and smoke coverage are being tightened before metadata discovery.
- Smoke coverage is the main live-runtime gate; host-testable units cover non-runtime parsing,
  validation, marshaling, and guard behavior.

### Next

- Keep loader V1 documented and covered by smoke tests, including explicit loader lookup,
  DexClassLoader lookup, and ART class-loader enumeration where supported.
- Keep metadata V1 hardened against device-specific ART layouts, large class sets, and query-shape
  edge cases.
- Broaden host-testable unit coverage around ownership invariants where they can be modeled safely.

### Later

- ART-specific metadata discovery and class enumeration.
- ART capability reporting for features such as class enumeration, loader enumeration, heap
  enumeration, deoptimization, and method replacement.
- Hook-friendly method metadata and a narrow ART method replacement prototype.
- HotSpot/JVMTI support only after the Android ART core is useful.

## Current Module Shape

- `src/lib.rs`: public Android-gated modules and re-exports
- `src/runtime.rs`: ART module discovery and JavaVM acquisition
- `src/vm.rs`: JavaVM wrapper and thread attachment
- `src/env.rs`: JNI vtable calls, method/field references, invocation, and exception handling
- `src/java.rs`: owned Rust-native convenience layer with class/object wrappers and ID caches
- `src/refs.rs`: typed local/global JNI reference wrappers
- `src/signature.rs`: Java type and method descriptor parsing
- `src/value.rs`: explicit JNI value representation and argument validation
- `src/jni.rs`: local raw JNI definitions and vtable slot constants
- `src/error.rs`: shared error and result types
- `src/bin/art_smoke.rs`: Android native smoke harness
- `smoke-fixtures/`: Java source and generated DEX used by the DexClassLoader smoke check; rebuild
  with `just smoke-fixture-dex`

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

- improve ergonomic conversions for object and array arguments
- keep adding unit tests for new descriptor and argument validation edge cases as they appear

Reference: `../frida-java-bridge/lib/types.js`.

### 3. Rust-Native Reflection Layer

Status: V1 complete; further reflection ergonomics remain incremental.

Goal:

Make common Java interaction possible without every caller manually threading together `Env`,
`ClassRef`, `MethodRef`, and `FieldRef`, while keeping descriptors and JNI value conversion explicit.

Delivered:

- `Runtime::java()` and `Vm::java()` entrypoints
- owned `JavaClass` and `JavaObject` wrappers backed by JNI global references
- explicit-signature constructor, method, static method, field, and static field helpers
- per-class caches for looked-up constructor, method, and field IDs
- smoke coverage for class lookup, strings, calls, fields, caching, and exception handling

Out of scope for this milestone:

- JS-style overload dispatch
- `Java.use()` compatibility
- method replacement
- app class-loader magic

Reference: `../frida-java-bridge/lib/class-factory.js`.

### 4. Class Loaders And App Class Resolution

Status: V1 complete; stabilization in progress.

Goal:

Resolve non-boot classes and model class-loader-specific identity.

Delivered:

- introduce `ClassLoaderRef`
- support explicit loader-aware class lookup through `ClassLoader.loadClass()` and array descriptor
  lookup through `Class.forName(name, false, loader)`
- isolate successful class lookup caches per `Java` instance
- add JNI object-class/type helpers used by loader validation
- add a DexClassLoader smoke fixture proving explicit loader lookup can resolve a non-bootstrap class
- add an API 26+ arm64 ART loader-enumeration backend path
- document loader-backed lookup semantics, cache isolation, and current object-wrapper boundaries

Remaining work:

- keep hardening unsupported-layout and missing-symbol behavior as more devices are tested
- key shared caches by loader identity plus class name only if cache ownership broadens
- broaden loader enumeration support beyond the current API 26+ arm64 milestone

Reference: `../frida-java-bridge/index.js`, `../frida-java-bridge/lib/class-factory.js`.

### 5. Metadata Discovery

Status: V1 complete; stabilization in progress.

Goal:

Discover loaded classes and inspect method/field metadata on supported runtimes.

Delivered:

- typed `JavaClassMetadata`, `JavaMethodMetadata`, and `JavaFieldMetadata`
- reflection-backed declared constructor, method, and field metadata
- ART loaded-class enumeration through `ClassLinker::VisitClasses`
- query helper for `class!method` patterns with `/i`, `/s`, and `/u` modifiers
- smoke coverage for DexClassLoader metadata, overloads, fields, loaded-class enumeration, and
  method queries

Remaining work:

- continue hardening ART loaded-class enumeration across Android versions and OEM builds
- decide whether to add lower-level ART method/field layout metadata before method replacement
- expand query compatibility only where it helps real Rust workflows

Reference: `../frida-java-bridge/lib/class-model.js`.

### 6. ART Capability Reporting

Status: planned.

Goal:

Make ART feature support explicit without introducing a premature multi-runtime backend boundary.

Planned work:

- expose capability reporting for class enumeration, loader enumeration, heap enumeration, deopt,
  and replacement support
- keep unsupported runtime behavior explicit in errors
- let later method-replacement work consume capability reports before attempting ART internals

HotSpot, JVMTI, and a true backend abstraction remain deferred until ART is useful enough and a
second runtime creates concrete design pressure.

### 7. Hooking And ART Advanced Features

Status: future.

Goal:

Prototype a narrow, documented method interception or replacement path on ART.

Planned work:

- define hook-facing method metadata
- add Android API-level-gated ART symbol resolution
- support one narrow replacement path before generalizing
- use ART capability reporting to expose replacement availability
- document the supported Android matrix before expanding it

Reference: `../frida-java-bridge/lib/android.js`.

## Non-Goals For Now

- drop-in `Java.use()` parity
- Dalvik support
- HotSpot/JVMTI support
- transparent JS-style overload dispatch
- heap scanning parity with `Java.choose()`
- broad Android-version method replacement before a narrow path is proven

## Testing Strategy

Use `cargo ndk` for build, check, and smoke workflows.

Current gates:

- `just check`: Android arm64 clippy
- `just test-build`: Android arm64 unit-test binary compilation
- `just build`: Android arm64 debug build
- `just smoke`: build, deploy, and run the ART smoke harness through `adb`

Add host-testable unit tests where behavior does not require a live VM:

- signature parsing
- descriptor formatting
- argument validation
- reference ownership rules where they can be modeled safely

Keep Android runtime checks in the smoke harness until a dedicated integration-test layout exists.

## Design Principles

- Prefer a Rust-native API over cloning the GumJS API.
- Keep low-level APIs explicit about thread attachment, signatures, ownership, and errors.
- Allow higher-level helpers later, but make attachment and loader boundaries visible.
- Use the upstream Java bridge as the behavioral reference, especially for feature boundaries and
  ART internals, while choosing Rust structures that fit this crate.
