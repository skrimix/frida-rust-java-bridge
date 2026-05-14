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

- `V1_CONTRACTS.md`: loader and metadata V1 public contracts
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
- Loader and metadata V1 contracts are documented, including class-loader cache isolation,
  `ClassLoaderKind`, method-query syntax, dotted user-facing class names, and unsupported-feature
  behavior.
- ART capability reporting is exposed through `Runtime`, `Vm`, and `Java`, with class-loader and
  loaded-class enumeration probed against the current ART layout and advanced features explicitly
  reported as deferred.
- Loader, metadata, and capability V1 stabilization is complete for the current API surface.
- Android-targeted unit tests cover descriptor formatting, argument validation, JNI value marshaling,
  method/field guard behavior, class-name normalization, and unsupported runtime-layout outcomes
  where no live VM is required.
- `src/bin/art_smoke.rs` creates an in-process ART VM and verifies runtime discovery, VM attachment,
  class lookup, string round trips, object construction, instance/static calls, field access, and
  Java exception handling through both low-level and convenience APIs.
- ART method replacement prerequisite probing now reaches the deferred-backend boundary across the
  current smoke matrix, including newer SDK 34/36 ClassLinker layouts and OPD2403's runtime-decorated
  native method flags.
- A hidden experimental ART method replacement prototype can directly patch, verify, and restore one
  static `()I` method for smoke validation, including cached-class and wrapper call paths. Patch and
  restore now validate executable replacement prerequisites and run under ART thread suspension when
  available. Public `.implementation`-style APIs remain deferred.
- Verification recipes exist in `justfile` for Android arm64 check/build/smoke workflows.

### In Progress

- Loader lookup remains explicit; automatic app-loader selection remains deferred.
- Smoke coverage is the main live-runtime gate; host-testable units cover non-runtime parsing,
  validation, marshaling, and guard behavior.

### Next

- Keep hardening the hidden static `()I` method replacement prototype across the smoke matrix before
  broadening signatures or exposing a public replacement API.
- Keep method replacement publicly unsupported until a replacement backend exists, but make its
  capability reason report whether current ART prerequisites are available or which prerequisite is
  missing.
- Keep loader and metadata V1 hardened against device-specific ART layouts, large class sets,
  query-shape edge cases, and capability/error consistency.
- Broaden host-testable unit coverage around ownership and ART-layout invariants where they can be
  modeled safely.

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
- `V1_CONTRACTS.md`: loader/metadata V1 public API contracts

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

Status: complete for V1.

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
- add a DexClassLoader smoke fixture proving explicit loader lookup can resolve a non-bootstrap class
- add an API 26+ arm64 ART loader-enumeration backend path
- document loader-backed lookup semantics, cache isolation, `ClassLoaderKind`, and current
  object-wrapper boundaries

Future work:

- keep hardening unsupported-layout and missing-symbol behavior as more devices are tested
- key shared caches by loader identity plus class name only if cache ownership broadens
- broaden loader enumeration support beyond the current API 26+ arm64 milestone

Reference: `../frida-java-bridge/index.js`, `../frida-java-bridge/lib/class-factory.js`.

### 5. Metadata Discovery

Status: complete for V1.

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
- smoke coverage for DexClassLoader metadata, overloads, fields, loaded-class enumeration, and
  method queries

Future work:

- continue hardening ART loaded-class enumeration across Android versions and OEM builds
- decide whether to extend lower-level ART layout metadata to declared fields/wrapper metadata
  before method replacement
- expand query compatibility only where it helps real Rust workflows

Reference: `../frida-java-bridge/lib/class-model.js`.

### 6. ART Capability Reporting

Status: complete for V1.

Goal:

Make ART feature support explicit without introducing a premature multi-runtime backend boundary.

Delivered:

- expose `RuntimeCapabilities` through `Runtime`, `Vm`, and `Java`
- report current support for ART class-loader and loaded-class enumeration using the same symbol and
  layout probes as the public enumeration APIs
- cover unsupported runtime-layout outcomes with host-testable seams
- report heap enumeration, deoptimization, and method replacement as explicit unsupported features

Future work:

- keep unsupported runtime behavior explicit in errors
- let later method-replacement work consume capability reports before attempting ART internals

HotSpot, JVMTI, and a true backend abstraction remain deferred until ART is useful enough and a
second runtime creates concrete design pressure.

### 7. Java.use-Style Wrapper Layer

Status: complete for wrapper ergonomics; method replacement remains deferred.

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
- smoke coverage exercises bootstrap and DexClassLoader-backed wrappers

Remaining work:

- keep `.implementation` and method replacement deferred until ART hooking has a narrow prototype

### 8. Hooking And ART Advanced Features

Status: in progress.

Goal:

Prototype a narrow, documented method interception or replacement path on ART.

Planned work:

- harden upstream-aligned ART method replacement prerequisite probes first
- validate runtime/ClassLinker layout candidates before reporting replacement readiness
- use ART's exported ClassLinker quick-entrypoint predicates as a fallback when newer layouts no
  longer expose the upstream intern-table anchor within the old scan window
- handle direct vs indirect JNI method IDs using ART's `Runtime.jni_ids_indirection_`
- keep `.implementation` and public replacement APIs deferred until probes pass across the smoke
  matrix
- then support one narrow replacement path before generalizing
- use ART capability reporting to expose replacement availability
- document the supported Android matrix before expanding it

Reference: `../frida-java-bridge/lib/android.js`.

## Non-Goals For Now

- drop-in GumJS `Java.use()` parity, even though Rust-native `use_class()` wrappers are supported
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
