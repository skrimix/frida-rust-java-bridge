# frida-java-bridge-rs Roadmap

## Paths
- frida-gum: `/home/skrimix/work/frida/frida-gum`
- frida-java-bridge: `/home/skrimix/work/frida/frida-java-bridge`
- frida-gum Rust bindings: `/home/skrimix/work/frida/frida-rust/frida-gum`
- gumjs: `/home/skrimix/work/frida/frida-gum/bindings/gumjs`

## Goal

Build a Rust-native Java runtime bridge for Frida that can eventually cover a meaningful subset of `frida-java-bridge`, while avoiding a first-pass attempt at feature parity with the current GumJS-based `Java` API.

The practical goal is not to transliterate the JavaScript codebase. The practical goal is to ship a usable Rust crate with:

- explicit VM attach/detach and JNI access
- class, object, method, and field wrappers
- predictable ownership and lifetime behavior
- enough runtime metadata to call methods and inspect classes
- a path toward hooking and method replacement on ART and HotSpot

## Reality Check

The current JS bridge is not just a wrapper around Gum. It is a runtime bridge with three major layers:

1. High-level Java API and wrapper system
2. JNI/JVMTI plumbing
3. VM-internal runtime patching for ART, Dalvik, and HotSpot

This means a Rust port should be treated as a re-architecture. The existing implementation is still the best reference for behavior and feature boundaries, but not necessarily for direct structure or API design.

## What Exists Today

From `frida-java-bridge`:

- `index.js`: public `Java` API surface and runtime initialization
- `lib/vm.js`: JavaVM attach/detach and `JNIEnv *` access
- `lib/env.js`: JNI vtable wrapper
- `lib/types.js`: type conversion and marshaling
- `lib/class-factory.js`: dynamic class/object wrappers and method replacement surface
- `lib/class-model.js`: fast metadata enumeration, partly implemented with `CModule`
- `lib/android.js`: ART/Dalvik internals, deopt, thread transitions, heap walking, hooks
- `lib/jvm.js`: HotSpot internals, class redefinition, method mangling
- `lib/jvmti.js`: JVMTI wrapper

From `frida-rust/frida-gum`:

- usable Rust bindings for core Gum APIs
- no Rust equivalent of Frida's full GumJS `Java` module
- no direct replacement for GumJS conveniences like `NativeFunction`, `NativeCallback`, `Memory`, `Script.bindWeak`, or `CModule`

In this Rust crate:

- Android ART is the active target; host JVM and Dalvik are deferred.
- `Runtime::obtain()` discovers `libart.so`, resolves `JNI_GetCreatedJavaVMs`, and returns the current `JavaVM`.
- `Vm` supports `GetEnv`, `AttachCurrentThread`, and `DetachCurrentThread`.
- `Env` exposes a minimal low-level JNI surface for class lookup, Java string creation/copying, exception checks/clearing, and local/global reference helpers.
- `src/bin/art_smoke.rs` is a standalone Android native smoke harness that loads ART, creates an in-process VM, obtains it through the crate, and verifies boot class lookup, string round-trips, and Java exception detection/clearing.
- Raw JNI definitions are local to the crate for now instead of using `jni-sys` or the higher-level `jni` crate.
- The current verification gates are `just check`, `just build`, and `just smoke`, all targeting arm64 Android.

## Non-Goals For V1

Do not start with these:

- drop-in `Java.use()` parity
- Dalvik support
- transparent JS-style overload dispatch
- deoptimization support
- ART method replacement across many Android releases
- HotSpot class redefinition parity
- heap scanning parity with `choose()`

These are valid later milestones, but they are the fastest way to stall the project if attempted up front.

## Recommended Strategy

Build the crate in layers, with each layer independently testable.

1. Core runtime detection and VM access
2. Safe-ish JNI wrapper layer
3. Rust-native reflection and invocation API
4. Metadata caching and convenience wrappers
5. Runtime-specific advanced features
6. Hooking and replacement APIs

Each layer should be useful before the next one begins.

## Proposed Crate Shape

Suggested module layout:

- `runtime`
- `vm`
- `env`
- `jni`
- `types`
- `signature`
- `class`
- `object`
- `method`
- `field`
- `array`
- `string`
- `loader`
- `cache`
- `error`
- `art`
- `hotspot`
- `jvmti`
- `hook`

Suggested top-level public types:

- `Runtime`
- `Vm`
- `Env<'vm>` or `AttachedEnv<'vm>`
- `ClassRef`
- `ObjectRef`
- `MethodRef`
- `FieldRef`
- `JavaType`
- `JavaValue`
- `MethodSignature`
- `ClassLoaderRef`

## API Design Principles

Prefer a Rust-native API over trying to clone the JS experience exactly.

Recommended direction:

- explicit signatures when invoking methods initially
- explicit `Result<T, Error>` everywhere
- explicit ownership between local refs, global refs, and borrowed refs
- no hidden thread attachment in low-level APIs
- higher-level helpers may attach automatically, but must make the boundary clear

Examples of a realistic early API:

```rust
let runtime = Runtime::obtain()?;
let vm = runtime.vm()?;
let env = vm.attach_current_thread()?;

let string_class = env.find_class("java/lang/String")?;
let ctor = string_class.get_constructor("([B)V")?;
let obj = ctor.new_object(&env, &[JavaValue::ByteArray(bytes)])?;

let hash_code = string_class.get_method("hashCode", "()I")?;
let value = hash_code.call_int(&env, &obj, &[])?;
```

This is less magical than `Java.use()`, but far more realistic to deliver early.

## Milestones

### Milestone 0: Skeleton And Scope

Deliverables:

- crate structure
- error model
- runtime detection for Android ART; desktop JVM support is not currently planned
- architecture decision notes

Tasks:

- add dependencies on `frida-gum`, `libc`, and any JNI helper crate only if it genuinely fits
- define error enums for JNI, JVMTI, symbol resolution, unsupported runtime, and version mismatch
- decide whether runtime backend selection is compile-time, runtime, or both

Exit criteria:

- `Runtime::obtain()` compiles and can detect whether Java is available

### Milestone 1: VM And Env Core

Reference files:

- `lib/vm.js`
- `lib/env.js`

Deliverables:

- JavaVM discovery
- thread attach/detach
- `JNIEnv` wrapper
- basic exception handling
- local/global reference helpers

Tasks:

- wrap `JNI_GetCreatedJavaVMs`
- model `JavaVM *` and `JNIEnv *`
- support `AttachCurrentThread`, `DetachCurrentThread`, and `GetEnv`
- expose core JNI operations:
  `FindClass`, `GetMethodID`, `GetStaticMethodID`, `GetFieldID`, `NewGlobalRef`, `DeleteGlobalRef`, `ExceptionOccurred`, `ExceptionClear`, `NewStringUTF`, `GetStringUTFChars`, and object/class helpers
- decide whether to cache JNI function pointers or call through the vtable every time

Exit criteria:

- can attach a thread and perform simple class lookup and string round-trips

### Milestone 2: Values, Signatures, And Marshaling

Reference file:

- `lib/types.js`

Deliverables:

- `JavaType`
- `JavaValue`
- JNI signature parser
- argument marshaling and return unmarshaling

Tasks:

- parse signatures like `Ljava/lang/String;`, `[I`, `(Ljava/lang/String;I)Z`
- model primitives, object refs, arrays, and strings
- support typed invocation helpers for primitive and object returns
- define clear rules around borrowed vs owned object references in arguments and return values

Exit criteria:

- can call constructors, instance methods, and static methods using explicit signatures

### Milestone 3: Rust-Native Reflection Layer

Reference files:

- `index.js`
- `lib/class-factory.js`

Deliverables:

- `ClassRef`, `ObjectRef`, `MethodRef`, `FieldRef`
- reflective lookup by name and signature
- convenience helpers for field get/set and method invoke

Tasks:

- implement wrapper types around JNI handles
- support constructors, instance methods, static methods, instance fields, and static fields
- allow `ClassLoader`-aware class resolution later, but do not block this milestone on it
- add caching of looked-up method and field IDs where safe

Exit criteria:

- enough functionality to write small Rust programs that use Java objects without raw JNI plumbing

### Milestone 4: Metadata Discovery

Reference files:

- `lib/class-model.js`
- `index.js` methods like class enumeration and method enumeration

Deliverables:

- loaded class enumeration
- method enumeration for a given class
- basic class loader visibility

Tasks:

- start with JVMTI-backed enumeration on JVM where available
- on Android, begin with class enumeration only if it can be done with a contained ART-specific backend
- do not begin with a `CModule` analogue; write Rust/C helpers only when profiling shows a need
- define one internal metadata representation that can be populated from ART or HotSpot backends

Exit criteria:

- can enumerate classes and inspect methods on at least one supported runtime family

### Milestone 5: Runtime Backends

Deliverables:

- explicit `art` backend
- explicit `hotspot` backend
- backend capability reporting

Tasks:

- factor runtime-specific behavior behind traits or enums rather than burying conditionals everywhere
- expose capabilities such as:
  class enumeration, class loader enumeration, heap enumeration, deopt support, method replacement support
- choose one runtime family as the first-class backend

Recommendation:

- do ART first if the main use-case is Android instrumentation
- do HotSpot first if the immediate goal is faster iteration and simpler debugging on desktop JVMs

Exit criteria:

- the crate can report what the current runtime supports without guesswork

### Milestone 6: Class Loaders And Non-Boot Resolution

Reference files:

- `index.js` loader enumeration
- `lib/class-factory.js` loader-aware use path

Deliverables:

- `ClassLoaderRef`
- explicit loader-aware class resolution
- per-loader caches

Tasks:

- add APIs to resolve classes against a chosen loader
- cache wrapper metadata keyed by loader identity plus class name
- clarify whether object wrappers remember their defining class loader or only their class

Exit criteria:

- can resolve app classes not visible from the bootstrap loader

### Milestone 7: Hook-Friendly Invocation And Replacement Surface

Reference files:

- `lib/class-factory.js`
- `lib/android.js`
- `lib/jvm.js`

Deliverables:

- low-level hook API for replacing Java method entrypoints
- stable internal representation of Java method metadata needed for patching

Tasks:

- separate method invocation from method replacement logic
- define a hook API that is honest about runtime limitations
- avoid promising JS-like `method.implementation = ...` until the low-level replacement path is proven
- decide whether replacement callbacks are pure Rust closures, C ABI shims, or a narrower function-pointer model

Exit criteria:

- one supported runtime can replace or intercept a narrow set of Java methods reliably in tests

### Milestone 8: ART Advanced Features

Reference file:

- `lib/android.js`

Deliverables:

- ART thread-state helpers
- selected method replacement paths
- optional backtrace support
- optional deopt support

Tasks:

- port only the pieces required for the chosen hook path
- add version-gated symbol resolution and capability detection
- keep Android API-level support explicit and documented
- write integration tests against a small supported Android matrix before broadening support

Exit criteria:

- a documented subset of Android versions supports method interception/replacement

### Milestone 9: HotSpot Advanced Features

Reference file:

- `lib/jvm.js`

Deliverables:

- supported method replacement or redefinition path on selected JDKs
- JVMTI-assisted metadata and maintenance operations

Tasks:

- support one JDK family first, likely modern LTS builds
- avoid chasing all historical JDK internals initially
- keep symbol-resolution logic isolated from the rest of the crate

Exit criteria:

- one documented JDK range supports advanced instrumentation features

## Suggested Initial Deliverable Order

If the goal is to get something useful quickly, implement in this order:

1. Runtime detection
2. VM attach/detach
3. JNIEnv wrapper
4. Strings and primitive values
5. Class and method lookup
6. Method invocation with explicit signatures
7. Field get/set
8. Object/global ref ownership model
9. Class loader support
10. Enumeration
11. Hooking or replacement on one runtime

## JS Module To Rust Mapping

Useful mapping from the existing codebase:

- `index.js` -> `runtime`, `api`, convenience layer
- `lib/vm.js` -> `vm`
- `lib/env.js` -> `env` or `jni::env`
- `lib/types.js` -> `types`, `signature`, `value`
- `lib/result.js` -> `error`
- `lib/jvmti.js` -> `jvmti`
- `lib/class-factory.js` -> `class`, `object`, `method`, `field`, `loader`, `cache`
- `lib/class-model.js` -> `metadata`
- `lib/android.js` -> `art`
- `lib/jvm.js` -> `hotspot`

## Major Technical Decisions To Make Early

### 1. How safe should the public API be?

Options:

- expose a mostly safe wrapper with carefully hidden `unsafe`
- expose low-level unsafe APIs plus a smaller safe convenience layer

Recommendation:

- keep low-level JNI operations explicit and partly unsafe where correctness depends on caller guarantees
- keep higher-level object and method wrappers safe where possible

### 2. How dynamic should method invocation be?

Options:

- typed methods like `call_int`, `call_object`, `call_void`
- generic `call(&[JavaValue]) -> JavaValue`

Recommendation:

- implement both eventually
- start with typed methods backed by parsed signatures

### 3. How should references be modeled?

Recommendation:

- separate local refs from global refs in the type system if practical
- if that becomes too heavy early on, at least keep them distinct internally and document semantics clearly

### 4. How much should depend on Frida Gum?

Recommendation:

- use `frida-gum` for process/module/symbol and patching support
- do not force Gum-specific concepts into the pure JNI layer if not needed
- make the basic VM/JNI pieces usable even when advanced patching is disabled

## Testing Strategy

Use staged testing from the beginning.

### Unit tests

- signature parsing
- value marshaling
- descriptor formatting
- handle ownership rules where testable without a live VM

### Integration tests on desktop JVM

- attach to a JVM
- class lookup
- string conversion
- instance construction
- method invocation
- field access

### Integration tests on Android ART

- attach in-process
- class lookup through app loader
- method invocation on framework and app classes
- later, method interception or replacement

### Compatibility matrix

Start narrow and expand deliberately.

Examples:

- JDK 17 or 21 first
- Android API 29 or 30 first
- arm64 first

## Risks

- ART internals remain version-fragile and expensive to maintain
- Rust lifetime correctness may conflict with JNI's operational model if over-constrained
- trying to preserve JS ergonomics too early will slow down the core bridge
- hooking support may dominate the schedule if started before reflection and invocation are solid
- multi-runtime support can sprawl unless backend boundaries are designed early

## Success Criteria

The project is succeeding if, before any advanced hooking work, it can already do the following reliably:

- detect a Java runtime
- attach the current thread
- find classes and methods
- construct objects
- call instance and static methods
- read and write fields
- handle strings and arrays correctly
- produce meaningful errors on JNI exceptions

At that point the crate is already useful, even without parity with Frida's JS `Java` API.

## Recommended First Work Items

Concrete first tasks for this repository:

1. Done: add module scaffolding for `error`, `runtime`, `vm`, `env`, and `jni`.
2. Done: keep `frida-gum` as the only Android-target dependency needed for runtime discovery.
3. Done: implement ART runtime discovery and `JNI_GetCreatedJavaVMs` lookup.
4. Done: implement `Vm` with attach/detach and `get_env`.
5. Done: implement a minimal `Env` wrapper for class lookup, exception handling, reference helpers, and UTF-8 strings.
6. Next: add a live Android smoke test harness before building higher-level wrappers.
7. Next: add signatures, values, and explicit method/field lookup.

## Practical Principle

At every stage, prefer a working Rust-native bridge over a clever imitation of GumJS.

If a future version grows a compatibility layer that feels like `Java.use()`, it should sit on top of a solid runtime core instead of forcing the core to mimic JavaScript from day one.
