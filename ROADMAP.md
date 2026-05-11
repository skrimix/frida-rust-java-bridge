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
- `src/bin/art_smoke.rs` creates an in-process ART VM and verifies runtime discovery, VM attachment,
  class lookup, string round trips, object construction, instance/static calls, field access, and
  Java exception handling.
- Verification recipes exist in `justfile` for Android arm64 check/build/smoke workflows.

### In Progress

- The current API is still low-level and explicit: callers work directly through `Env`, signatures,
  `MethodRef`, `FieldRef`, and raw-ish JNI values.
- Class/object/method/field concepts are represented, but not yet split into higher-level ergonomic
  modules with caching or loader awareness.
- Smoke coverage is the main live-runtime gate; host-testable units are limited to descriptor/value
  logic.

### Next

- Add a Rust-native reflection/convenience layer over the current `Env` operations.
- Introduce loader-aware class resolution so app classes can be resolved outside the bootstrap loader.
- Add metadata and method/field lookup caching where JNI identity and lifetime rules make it safe.
- Broaden host-testable unit coverage around signatures, argument validation, and ownership invariants.

### Later

- ART-specific metadata discovery and class enumeration.
- Backend capability reporting for features such as class enumeration, loader enumeration, heap
  enumeration, deoptimization, and method replacement.
- Hook-friendly method metadata and a narrow ART method replacement prototype.
- HotSpot/JVMTI support only after the Android ART core is useful.

## Current Module Shape

- `src/lib.rs`: public Android-gated modules and re-exports
- `src/runtime.rs`: ART module discovery and JavaVM acquisition
- `src/vm.rs`: JavaVM wrapper and thread attachment
- `src/env.rs`: JNI vtable calls, method/field references, invocation, and exception handling
- `src/refs.rs`: typed local/global JNI reference wrappers
- `src/signature.rs`: Java type and method descriptor parsing
- `src/value.rs`: explicit JNI value representation and argument validation
- `src/jni.rs`: local raw JNI definitions and vtable slot constants
- `src/error.rs`: shared error and result types
- `src/bin/art_smoke.rs`: Android native smoke harness

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
- add more unit tests for invalid descriptors and argument validation edge cases

Reference: `../frida-java-bridge/lib/types.js`.

### 3. Rust-Native Reflection Layer

Status: next major milestone.

Goal:

Make common Java interaction possible without every caller manually threading together `Env`,
`ClassRef`, `MethodRef`, `FieldRef`, string descriptors, and low-level `JavaValue` lists.

Planned work:

- define higher-level class/object/method/field wrappers around the existing low-level operations
- provide explicit-signature lookup and call helpers that remain honest about JNI errors
- add safe caching for looked-up method and field IDs
- keep raw JNI escape hatches available, but clearly marked unsafe where appropriate

Out of scope for this milestone:

- JS-style overload dispatch
- `Java.use()` compatibility
- method replacement
- app class-loader magic

Reference: `../frida-java-bridge/lib/class-factory.js`.

### 4. Class Loaders And App Class Resolution

Status: planned.

Goal:

Resolve non-boot classes and model class-loader-specific identity.

Planned work:

- introduce `ClassLoaderRef`
- support explicit loader-aware class lookup
- key caches by loader identity plus class name where needed
- document how object wrappers relate to defining class loaders

Reference: `../frida-java-bridge/index.js`, `../frida-java-bridge/lib/class-factory.js`.

### 5. Metadata Discovery

Status: planned.

Goal:

Discover loaded classes and inspect method/field metadata on supported runtimes.

Planned work:

- define an internal metadata representation independent of the runtime backend
- start with a contained ART-specific path when practical
- avoid building a `CModule` analogue unless profiling shows it is needed
- add live-runtime tests for supported Android versions

Reference: `../frida-java-bridge/lib/class-model.js`.

### 6. Runtime Backends And Capabilities

Status: planned.

Goal:

Make runtime-specific support explicit instead of implicit.

Planned work:

- factor ART-specific behavior behind a backend boundary
- expose capability reporting for class enumeration, loader enumeration, heap enumeration, deopt,
  and replacement support
- keep unsupported runtime behavior explicit in errors

HotSpot and JVMTI remain deferred until the ART-first path is useful.

### 7. Hooking And ART Advanced Features

Status: future.

Goal:

Prototype a narrow, documented method interception or replacement path on ART.

Planned work:

- define hook-facing method metadata
- add Android API-level-gated ART symbol resolution
- support one narrow replacement path before generalizing
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
