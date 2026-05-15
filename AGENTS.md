# Repository Guidelines

## Project Posture

This is a private pre-user experiment, not a crate with stable public contracts. Exported Rust APIs,
module names, and documentation terms are allowed to change when that makes the prototype clearer or
the ART behavior safer. Treat roadmap and behavior docs as planning notes and current snapshots, not
compatibility promises.

Everything in the crate is experimental. A soft-frozen API means "useful and smoke-covered enough to
avoid casual churn for now"; it does not mean stable, finalized, or externally promised. Prefer clear
design, accurate smoke coverage, and honest failure reporting over preserving accidental API shapes.

## Project Structure & Module Organization

This is a Rust crate targeting Android ART only. Core library code lives in `src/`:

- `src/lib.rs` exposes the current Android-only modules and re-exports.
- `src/runtime.rs`, `src/vm.rs`, and `src/env.rs` implement ART runtime discovery, JavaVM access, and JNI environment helpers.
- `src/jni.rs` contains local raw JNI definitions and vtable slot helpers.
- `src/error.rs` defines shared error and result types.
- `src/app_process_smoke.rs` is the primary app-process live-runtime smoke harness, compiled into
  the cdylib with the `app-process-smoke` feature.
- `src/bin/art_smoke.rs` is the native ART bootstrap smoke harness and should stay limited to
  native VM creation/startup coverage.

There is no committed `tests/` directory yet. Add focused unit or integration tests when host-testable logic appears; keep Android runtime checks in the smoke harness.

Reference and edit `ROADMAP.md` for the current state of the project and plans. You can introduce other markdown files for tracking your progress if you want.

## Reference Material

- `ROADMAP.md`: current project state and plans
- `CURRENT_BEHAVIOR.md`: current behavior notes and soft-freeze drafts
- `../frida-gum`: Frida Gum source code
- `../frida-java-bridge`: Frida Java Bridge source code. Important: always reference that when working on the project, as this crate is a re-implementation of it.
- `../frida-rust/frida-gum`: Frida Gum bindings for Rust.
- `~/work/art`: ART source code repo

## Build, Test, and Development Commands

Use the `justfile` recipes where possible:

- `just check` runs `cargo ndk -t arm64-v8a clippy`.
- `just build` builds the Android arm64 debug crate.
- `just build-release` builds the Android arm64 release artifact.
- `just smoke-build` builds the primary app-process smoke artifacts.
- `just art-smoke-build` builds the native ART bootstrap `art_smoke` binary.
- `just devices` lists connected `adb` devices with serial, model/device name, and SDK version.
- `just smoke-deploy [serial|all]` pushes the primary app-process smoke artifacts to `/data/local/tmp/frida-java-bridge-rs/` on a selected device or all connected devices.
- `just smoke-run [serial|all]` runs the deployed app-process ART smoke check on a selected device or all connected devices.
- `just smoke [serial|all]` builds, deploys, and runs the primary app-process ART smoke check with `adb`; without an argument it requires exactly one connected device.
- `just smoke-all` is a convenience alias for `just smoke all`.
- `just art-smoke [serial|all]` builds, deploys, and runs the native ART bootstrap smoke check.
- `just art-smoke-all` is a convenience alias for `just art-smoke all`.

Prerequisites include Rust, `cargo-ndk`, the Android NDK/toolchain, and `adb` for device smoke runs.

Add more recipes to `justfile` for new commands and workflows you introduce.
Always use `cargo ndk` for build/check/test operations.

## Testing Guidelines

Current verification gates are `just check`, `just build`, and `just smoke`. Run `just smoke` for changes touching live-runtime behavior, app-loader lookup, JNI vtable access, exception handling, metadata/enumeration, method replacement, or reference ownership. Run `just art-smoke` for changes touching native ART loading, manual VM creation, startup signal-chain handling, or bootstrap-only VM attachment. Name future integration tests after the behavior under test, for example `tests/string_round_trip.rs`.

New Android runtime smoke coverage should usually go in the app-process harness. Keep `art_smoke`
focused on the native-bootstrap behaviors that cannot be validated from an already-created ART
process.

Do not turn off or newly gate a feature just because the smoke harness exposes a bug on a device or Android version. This crate is still pre-use; prefer leaving the test failure visible and fixing the underlying runtime behavior. Only report a capability as unsupported when the limitation is an intentional prototype boundary or a well-understood missing implementation, and document that decision in `ROADMAP.md` or `CURRENT_BEHAVIOR.md`.

## Commit Guidelines

Recent commits use short imperative subjects, for example `Add Android ART smoke harness` and `Prepare build`. Use longer descriptions when needed.

Commit at your own discretion between and after making changes.
