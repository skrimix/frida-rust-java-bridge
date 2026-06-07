# Repository Guidelines

## Project Posture

This is an experimental pre-1.0 crate, not a crate with stable public contracts. Exported Rust APIs,
module names, and documentation terms are allowed to change when that makes the bridge clearer or the
ART behavior safer. Treat roadmap and behavior docs as planning notes and current snapshots, not
compatibility promises.

Capabilities are either supported or unsupported with a reason. Safety is expressed through Rust API
boundaries: prefer safe APIs for normal Java work and explicit `unsafe` APIs for raw JNI or ART
mutation. Prefer clear design, accurate test coverage, and honest failure reporting over preserving
accidental API shapes.

## Project Structure & Module Organization

This is a Rust crate targeting Android ART only. Core library code lives in `src/`:

- `src/lib.rs` exposes the current Android-gated modules, shared descriptor/value modules, and
  public re-exports.
- `src/android.rs`, `src/runtime.rs`, and `src/vm.rs` implement Android version probing, ART
  runtime discovery, JavaVM access, and thread attachment.
- `src/env/` contains the safe JNI environment surface: IDs, calls, fields, arrays, strings,
  references, exceptions, member lookup, and helper macros.
- `src/error.rs` defines shared error and result types.
- `src/jni.rs` contains local raw JNI definitions and vtable slot helpers.
- `src/refs.rs`, `src/value.rs`, `src/signature.rs`, `src/metadata.rs`, and `src/modifiers.rs`
  define typed reference ownership, Java values, descriptor parsing, reflection metadata, and access
  flag constants.
- `src/java/` contains the high-level Rust Java facade: class lookup, loader scoping,
  `Java::perform()`, main-thread scheduling, wrappers, objects, arrays, call arguments, returns,
  dispatch helpers, and wrapper macros.
- `src/art/` contains ART-specific internals for layout probing, class/heap enumeration, method
  replacement support, runnable-thread handling, and backend glue. Keep direct ART mutation behind
  explicit unsafe boundaries.
- `src/replacement/` contains the public guarded method/constructor replacement facade plus the
  closure trampoline, original-call handling, lifecycle guard, and backend adapter.
- `src/app_process_test.rs` and `src/app_process_test/` are the primary app-process live-runtime
  harness, compiled into the cdylib with the `app-process-test` feature.
- `src/apk_perform_test.rs` is the APK startup-agent harness for early `Java::perform()` draining,
  compiled with the `apk-perform-test` feature.
- `src/bin/art_test.rs` is the native ART bootstrap test harness and should stay limited to
  native VM creation/startup coverage.
- `examples/frida_js_ergonomics_probe.rs` is a compile-oriented probe for Rust API ergonomics
  against representative Frida JS snippets; it is not a live runtime test.
- `test-fixtures/src/`, `test-fixtures/dex/`, and `test-fixtures/apk/` hold Java sources, dex/APK
  fixtures, and Android manifest/assets used by the app-process and APK harnesses. Generated output
  goes under `test-fixtures/build/`.

There is no committed top-level `tests/` directory yet. Add focused unit or integration tests when
host-testable logic appears; keep Android runtime checks in the app-process or APK harnesses unless
they specifically need native bootstrap coverage.

Update `CURRENT_BEHAVIOR.md` forcurrent behavior notes, and `FEATURE_PROGRESS.md` for the 
upstream-aligned status matrix. You can introduce other markdown files for tracking your progress if you want.

## Reference Material

- `.agents/CURRENT_BEHAVIOR.md`: current behavior notes
- `.agents/FEATURE_PROGRESS.md`: scan-friendly feature/status matrix aligned with upstream `PUBLIC_DOC.md`
- `../frida-gum`: Frida Gum source code
- `../frida-java-bridge`: Frida Java Bridge source code. This project is a reimplementation of it, so make sure to use it as reference when implementing new features or analyzing workflows.
- `../frida-java-bridge/lib/android.js`: ART internals and behavior reference
- `../frida-rust/frida-gum`: Frida Gum bindings for Rust.
- `~/work/android/art`: ART source code repo
- `~/work/android/base`: Android framework source code

## Build, Test, and Development Commands

Use the `justfile` recipes where possible:

- `just check` runs Android arm64 clippy with the `app-process-test` and `apk-perform-test`
  feature gates enabled.
- `just build` builds the Android arm64 debug crate.
- `just build-release` builds the Android arm64 release artifact.
- `just unit-test-build` builds Android arm64 library unit tests without running them.
- `just unit-test [serial|all]` builds and runs the Android arm64 unit tests through `cargo-ndk-runner`; without an argument it requires exactly one connected device.
- `just unit-test-all` is a convenience alias for `just unit-test all`.
- `just test-fixture-dex` rebuilds the dex fixture used by class-loader and dex-loading checks.
- `just app-process-test-build` builds the primary app-process test jar and cdylib.
- `just test-build` aliases `just app-process-test-build`.
- `just app-test-deploy [serial|all]` pushes app-process harness artifacts to
  `/data/local/tmp/frida-rust-java-bridge/`.
- `just app-test-run [serial|all]` runs the deployed app-process ART harness.
- `just app-test [serial|all]` builds, deploys, and runs the app-process ART harness.
- `just app-test-all` is a convenience alias for `just app-test all`.
- `just art-test-build` builds the native ART bootstrap `art_test` binary.
- `just devices` lists connected `adb` devices with serial, model/device name, and SDK version.
- `just test-deploy [serial|all]`, `just test-run [serial|all]`, `just test [serial|all]`, and
  `just test-all` are compatibility aliases for the app-process harness recipes.
- `just apk-perform-test-lib` builds the cdylib with the `apk-perform-test` feature.
- `just apk-perform-test-apk` builds and signs the APK early-start fixture.
- `just apk-perform-test-build` aliases `just apk-perform-test-apk`.
- `just apk-perform-test-deploy [serial|all]` installs the APK early-start fixture.
- `just apk-perform-test-run [serial|all]` starts the APK with the native agent attached and polls
  the status provider.
- `just apk-perform-test [serial|all]` builds, deploys, and runs the APK early-start `Java::perform()` drain check.
- `just apk-perform-test-all` is a convenience alias for `just apk-perform-test all`.
- `just art-test-deploy [serial|all]` pushes the native ART bootstrap binary.
- `just art-test-run [serial|all]` runs the native ART bootstrap binary with the required ART
  library environment.
- `just art-test [serial|all]` builds, deploys, and runs the native ART bootstrap test check.
- `just art-test-all` is a convenience alias for `just art-test all`.
- `just host-test`: limited host-target library unit tests for platform-independent logic

Prerequisites include Rust, `cargo-ndk`, the Android NDK/toolchain, `adb`, a JDK, and the Android
SDK build tools used by the fixture builders (`d8`, `aapt2`, `zipalign`, and `apksigner` as
applicable).

Add more recipes to `justfile` for new commands and workflows you introduce.
Always use `cargo ndk` for build/check/test operations.

## Testing Guidelines

Current verification gates are `just check`, `just build`, `just unit-test all`, `just test all`,
`just apk-perform-test all`, and `just art-test all`. Run `just test` for changes touching
live-runtime behavior, app-loader lookup, JNI vtable access, exception handling,
metadata/enumeration, method replacement, main-thread scheduling, or reference ownership. Run
`just apk-perform-test` for changes touching early app startup, deferred `Java::perform()`, app
loader publication from startup hooks, or main-looper scheduling in a real APK process. Run
`just art-test` for changes touching native ART loading, manual VM creation, startup signal-chain
handling, or bootstrap-only VM attachment. Name future integration tests after the behavior under
test, for example `tests/string_round_trip.rs`.

New Android runtime test coverage should usually go in the app-process harness. Use the APK harness
for behavior that only appears during real app startup or requires a main Android looper. Keep
`art_test` focused on the native-bootstrap behaviors that cannot be validated from an
already-created ART process.

Do not turn off or newly gate a feature just because the test harness exposes a bug on a device or Android version. This crate is still pre-use; prefer leaving the test failure visible and fixing the underlying runtime behavior. Only report a capability as unsupported when the limitation is intentional or a well-understood missing implementation, and document that decision in `CURRENT_BEHAVIOR.md`.

## Commit Guidelines

Recent commits use short imperative subjects, for example `Add Android ART test harness` and `Prepare build`. Use longer descriptions when needed.

Commit at your own discretion between and after making changes.
