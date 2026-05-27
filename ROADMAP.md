# frida-java-bridge-rs Roadmap

## Scope

This crate is a Rust-native Java runtime bridge for Frida on Android ART. It follows the useful
Android behavior of `frida-java-bridge` while using Rust ownership, lifetimes, and `unsafe` markers
instead of cloning GumJS dynamics.

The project is private and pre-user. Exported Rust APIs, module names, and documentation terms may
change when that makes the bridge clearer or the ART behavior safer. Capability reports are binary:
a feature is either supported on the current runtime or unsupported with a reason. Operations that
need caller guarantees are marked `unsafe` at the API boundary.

In scope:

- ART runtime discovery, JavaVM access, and thread attachment
- JNI reference ownership, exception handling, descriptors, value marshaling, and checked calls
- class, object, array, method, field, loader, wrapper, and metadata APIs
- app-loader selection and deferred app startup work
- method replacement, heap enumeration, main-thread scheduling, and deoptimization where ART
  support can be made reliable

Out of scope unless the project is deliberately rescoped:

- Dalvik, HotSpot, JVM TI, desktop JVMs, or a generic multi-runtime backend
- JavaScript API compatibility or line-by-line GumJS parity
- implicit loader, attachment, descriptor, or ownership behavior that hides important runtime state

## References

- `CURRENT_BEHAVIOR.md`: detailed behavior notes
- `FEATURE_PROGRESS.md`: scan-friendly feature/status matrix aligned with upstream `PUBLIC_DOC.md`
- `FINALIZATION_PLAN.md`: final cleanup, hardening, and documentation sprint protocol
- `CLEANUP_AUDIT.md`: module-by-module cleanup discovery and implementation tracker
- `HARDENING_AUDIT.md`: lifetime, unsafety, and correctness audit tracker
- `DOCUMENTATION_PASS.md`: public documentation rewrite rules and checklist
- `DOCS_PROGRESS.md`: live documentation rewrite tracker and verification notes
- `../frida-java-bridge`: primary behavior and ART internals reference
- `../frida-java-bridge/lib/android.js`: ART internals reference
- `../frida-gum`: Frida Gum source
- `../frida-rust/frida-gum`: Rust Gum bindings used for process/module discovery

`ROADMAP.md` owns sequencing and priorities. Current behavior details belong in
`CURRENT_BEHAVIOR.md`; exhaustive feature coverage belongs in `FEATURE_PROGRESS.md`.

## Active Priorities

Finalization hardening implementation is complete. The active pre-user work is the public
documentation rewrite, final verification, and keeping behavior/status notes aligned with the
completed hardening boundaries. `DOCS_PROGRESS.md` tracks the documentation pass.

### 1. Method Replacement Hardening

Status: hardening implementation complete for the finalization sprint.

Goal: keep guarded wrapper-selected replacement dependable across the Android arm64 ART matrix.

- Treat Java stack-trace aborts, quick-frame failures, or restore failures as replacement integrity
  bugs until the ART behavior is understood.
- Keep replacement lifecycle explicit: the returned guard owns the hook and overlapping
  replacements are rejected. `JavaMethod::replace` should stay safe through callback-local
  reference views, return assignability checks, and active-callback teardown quiescence; keep safe
  constructor replacement constrained through the original-constructor initialization token, with
  unchecked receiver-initialization hooks remaining explicit `unsafe`.

### 2. Main-Thread Scheduling

Goal: provide predictable app-process callback scheduling on Android's main Java thread.


- Keep `app_process` command-line runs unsupported when `Looper.getMainLooper()` is null.
- Keep capability probing side-effect-light: no hook installation, callback enqueue, or looper
  wakeup during `JavaCapabilities` checks.

### 3. App-Loader Deferral

Goal: support early app startup without guessing the app loader.


- Use `ActivityThread.currentApplication()` only when it exposes a real `Application`.
- Keep bare `Java::use_class()` using the published default app loader without changing
  `Java::find_class()` bootstrap semantics.
- Return `AppClassLoaderUnavailable` instead of falling back to thread-context or enumerated loaders.
- Add an explicit app-loader override only when real Rust call sites need it.

### 4. Loader, Metadata, And Heap Coverage

Goal: keep runtime enumeration reliable while ART layout support expands.


- Keep unsupported layouts visible through structured errors and capability reports.

### 5. Test Coverage And Harness Hygiene

Goal: make regressions obvious without spreading runtime checks into the wrong harness.


- Put normal live-runtime behavior in the app-process harness.
- Keep `src/bin/art_test.rs` limited to native ART bootstrap and manual VM creation coverage.
- Leave failing runtime capabilities visible unless a limitation is intentional and documented.

## Additional Work

- subclass-inclusive heap enumeration and broader heap matrix hardening
- deoptimization hardening across the broader ART device matrix
- broader Android-version replacement support beyond the proven arm64 ART path
- Java backtraces, dex loading, and class registration
- full `ClassFactory` manager semantics, including cache directory policy, temp-file naming,
  `openClassFile()`, allocation-only `$alloc`, and init-only `$init`
- broader cache sharing keyed by loader identity plus class name, if ownership broadens beyond
  per-`Java` caches

## Verification

Use `cargo ndk` for build, check, and Android test workflows. Prefer `justfile` recipes:

- `just check`: Android arm64 clippy
- `just host-test`: host-target library unit tests for platform-independent logic
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

- Simple by default, powerful when needed.
- Prefer Rust-native APIs over GumJS-shaped compatibility.
- Keep attachment, loader scope, descriptors, ownership, and errors visible.
- Use safe APIs for ordinary Java work and explicit `unsafe` APIs for raw JNI or ART mutation.
- Use upstream `frida-java-bridge` as the behavior and feature-boundary reference.
