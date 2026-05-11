# Repository Guidelines

## Project Structure & Module Organization

This is a Rust crate targeting Android ART only. Core library code lives in `src/`:

- `src/lib.rs` exposes the public Android-only modules and re-exports.
- `src/runtime.rs`, `src/vm.rs`, and `src/env.rs` implement ART runtime discovery, JavaVM access, and JNI environment helpers.
- `src/jni.rs` contains local raw JNI definitions and vtable slot helpers.
- `src/error.rs` defines shared error and result types.
- `src/bin/art_smoke.rs` is the Android native smoke harness.

There is no committed `tests/` directory yet. Add focused unit or integration tests when host-testable logic appears; keep Android runtime checks in the smoke harness.

Reference and edit `ROADMAP.md` for the current state of the project and plans. You can introduce other markdown files for tracking your progress if you want.

## Build, Test, and Development Commands

Use the `justfile` recipes where possible:

- `just check` runs `cargo ndk -t arm64-v8a clippy`.
- `just build` builds the Android arm64 debug crate.
- `just build-release` builds the Android arm64 release artifact.
- `just smoke-build` builds the `art_smoke` binary.
- `just smoke-deploy` pushes `art_smoke` to `/data/local/tmp/frida-java-bridge-rs/` on a connected device.
- `just smoke` builds, deploys, and runs the ART smoke check with `adb`.

Prerequisites include Rust, `cargo-ndk`, the Android NDK/toolchain, and `adb` for device smoke runs.

## Testing Guidelines

Current verification gates are `just check`, `just build`, and `just smoke`. Run `just smoke` for changes touching ART discovery, VM attachment, JNI vtable access, exception handling, or reference ownership. Name future integration tests after the behavior under test, for example `tests/string_round_trip.rs`.

## Commit Guidelines

Recent commits use short imperative subjects, for example `Add Android ART smoke harness` and `Prepare build`. Use longer descriptions when needed.

Commit at your own discretion between and after making changes.