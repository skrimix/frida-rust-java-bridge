# frida-java-bridge-rs Roadmap

## Scope

This crate is a Rust-native Java runtime bridge for Frida, currently targeting Android ART only.
It is a re-implementation path for a useful subset of `frida-java-bridge`, not a line-by-line port
and not an early attempt at GumJS `Java` API parity.

This is a private pre-user experiment. There are no stable public contracts yet, and exported Rust
APIs may change freely when that makes the prototype clearer. Roadmap and behavior docs are planning
notes and current snapshots. "Soft-frozen" means useful and smoke-covered enough to avoid casual
churn for now, not finalized or externally promised.

The practical goal is to provide:

- explicit ART runtime discovery and JavaVM access
- thread attachment and `JNIEnv` access
- predictable local/global reference ownership
- descriptor parsing and explicit JNI value marshaling
- class, object, method, and field operations through a Rust API
- a later path toward class-loader support, metadata discovery, and ART method replacement

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
  loaded-class enumeration probed against the current ART layout and advanced features explicitly
  reported as deferred.
- Loader, metadata, and capability APIs are soft-frozen for the current smoke-covered shape.
- Android-targeted unit tests cover descriptor formatting, argument validation, JNI value marshaling,
  method/field guard behavior, class-name normalization, and unsupported runtime-layout outcomes
  where no live VM is required.
- `src/bin/art_smoke.rs` is intentionally limited to native ART bootstrap coverage: loading
  `libart.so`, calling `JNI_CreateJavaVM`, obtaining the created VM through `Runtime::obtain()`,
  attaching a thread, and running a small bootstrap-class JNI/convenience sanity check.
- ART method replacement prerequisite probing now reaches the deferred-backend boundary across the
  current smoke matrix, including newer SDK 34/36 ClassLinker layouts and OPD2403's runtime-decorated
  native method flags.
- The app-process smoke target is the primary live-runtime gate for normal bridge behavior. It runs
  inside an already-created ART process with an app-provided class loader and covers low-level JNI
  helpers, convenience wrappers, explicit app-loader lookup, DexClassLoader lookup, metadata,
  loaded-class and class-loader enumeration, and experimental replacement checks.
- A hidden experimental ART method replacement prototype now makes cloned `ArtMethod` dispatch the
  active smoke path for selected static and instance methods: no-arg primitive/`void`, no-arg
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
  the target ART thread and method, and smoke coverage now includes selected static/instance
  primitive, `String`, and reference argument/return paths, including object arrays and null JNI
  values. Generated
  executable thunks are flushed from the instruction cache before use. A hidden overload-first
  experimental facade can replace selected `JavaMethodOverload` values and call originals through
  captured overload metadata with generic `IntoJavaArgs` argument containers and typed raw-return
  extraction. A hidden descriptor-driven raw JNI-native layer now covers the same smoked ABI
  shapes so future replacement signatures can be admitted through one classifier instead of only
  signature-specific helpers; it still requires exact explicit JNI-native callback ABIs. Dedicated
  lifecycle smoke coverage now exercises replace/revert/replace on the same static and instance
  `ArtMethod` through both direct helpers and the overload facade. `.implementation`-style APIs
  remain to be implemented.
- Verification recipes exist in `justfile` for Android arm64 check/build/smoke workflows.

### In Progress

- Loader lookup remains explicit; automatic app-loader selection remains to be implemented.
- Smoke coverage is the main live-runtime gate; host-testable units cover non-runtime parsing,
  validation, marshaling, and guard behavior.
- Clone-active replacement passes the current app-process smoke matrix on Quest 2 SDK 34, Pixel 8
  Pro SDK 36, OPD2403 SDK 36, and Mi Max SDK 29. Direct-helper and overload-facade
  replace/revert/replace lifecycle smoke now passes on that matrix. Broader ART instrumentation
  parity remains incomplete; keep closure-backed replacement callbacks, arbitrary replacement
  signatures beyond the currently smoked primitive/`String`/single-reference lanes, and finished
  replacement ergonomics remain.

### Next

- Keep hardening the hidden clone-active replacement prototype across the native and app-process
  smoke matrix. Keep arbitrary object/multi-reference signatures, closure-backed replacement
  callbacks, and exported replacement APIs deferred until quick-dispatch instrumentation is
  broader.
- Keep repeated replacement lifecycle behavior smoke-covered with dedicated fixture methods. The
  isolated replace/revert/replace case now passes across the current device matrix; investigate
  future lifecycle failures as backend cleanup or ART-dispatch regressions instead of hiding them.
- Keep method replacement APIs unsupported until a broader backend/API exists, but make
  its capability reason report whether current ART prerequisites are available or which prerequisite
  is missing.
- Keep loader and metadata behavior hardened against device-specific ART layouts, large class sets,
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

- `src/lib.rs`: current Android-gated modules and re-exports
- `src/runtime.rs`: ART module discovery and JavaVM acquisition
- `src/vm.rs`: JavaVM wrapper and thread attachment
- `src/env.rs`: JNI vtable calls, method/field references, invocation, and exception handling
- `src/java.rs`: owned Rust-native convenience layer with class/object wrappers and ID caches
- `src/refs.rs`: typed local/global JNI reference wrappers
- `src/signature.rs`: Java type and method descriptor parsing
- `src/value.rs`: explicit JNI value representation and argument validation
- `src/jni.rs`: local raw JNI definitions and vtable slot constants
- `src/error.rs`: shared error and result types
- `src/bin/art_smoke.rs`: native ART bootstrap smoke harness
- `src/app_process_smoke.rs`: primary app-process live-runtime smoke harness, compiled into the
  cdylib with the `app-process-smoke` feature
- `smoke-fixtures/`: Java source, app-process jar, and generated DEX used by smoke checks; rebuild
  with `just app-process-smoke-dex`
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

- improve ergonomic conversions for object and array arguments
- keep adding unit tests for new descriptor and argument validation edge cases as they appear

Reference: `../frida-java-bridge/lib/types.js`.

### 3. Rust-Native Reflection Layer

Status: soft-frozen; further reflection ergonomics remain incremental.

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
- smoke coverage for DexClassLoader metadata, overloads, fields, loaded-class enumeration, and
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
- cover unsupported runtime-layout outcomes with host-testable seams
- report heap enumeration, deoptimization, and method replacement as explicit unsupported features

Future work:

- keep unsupported runtime behavior explicit in errors
- let later method-replacement work consume capability reports before attempting ART internals

HotSpot, JVMTI, and a true backend abstraction remain deferred until ART is useful enough and a
second runtime creates concrete design pressure.

### 7. Java.use-Style Wrapper Layer

Status: soft-frozen for wrapper ergonomics; exported replacement APIs remain in progress.

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
- smoke coverage exercises bootstrap and DexClassLoader-backed wrappers

Remaining work:

- `.implementation` and exported method replacement APIs to be implemented

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
  one-reference-argument/reference-return methods, including object-array reference smoke coverage
- raw original invocation from replacements using a thread-scoped ART bypass
- smoke coverage for cached classes, wrappers, GC-during-active replacement, object arrays, null JNI values,
  restore, and isolated replace/revert/replace lifecycle checks through direct helpers and the
  overload facade
- ART capability reporting continues to mark  method replacement unsupported, with the
  reason describing whether hidden prerequisites are available or which prerequisite is missing
- experimental overload-based replacement facade for selected `JavaMethodOverload` values, backed
  by explicit JNI-native callback variants, a descriptor-driven raw JNI-native layer, overload
  metadata for original calls, generic original-call arguments, and typed raw-return extraction

Planned work:

- `.implementation` and exported replacement APIs
- document the supported Android matrix before expanding it
- keep isolated smoke coverage for replacing, reverting, and replacing the same `ArtMethod` again;
  use any future failure to debug stale clone/thunk/controller state left by restore
- arbitrary object/multi-reference signatures and closure-backed replacement callbacks

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
- `just smoke`: build, deploy, and run the primary app-process ART smoke harness through `adb`
- `just app-smoke`: compatibility alias for the app-process ART smoke harness
- `just art-smoke`: build, deploy, and run the native ART bootstrap smoke harness through `adb`

Add host-testable unit tests where behavior does not require a live VM:

- signature parsing
- descriptor formatting
- argument validation
- reference ownership rules where they can be modeled safely

Keep Android runtime checks in the smoke harness until a dedicated integration-test layout exists.
New runtime smoke coverage should go in the app-process harness unless it specifically validates
native ART startup or manual VM creation.

## Design Principles

- Prefer a Rust-native API over cloning the GumJS API.
- Keep low-level APIs explicit about thread attachment, signatures, ownership, and errors.
- Allow higher-level helpers later, but make attachment and loader boundaries visible.
- Use the upstream Java bridge as the behavioral reference, especially for feature boundaries and
  ART internals, while choosing Rust structures that fit this crate.
