# Testing

This crate has several test flows because ART behavior changes with the process shape. The harnesses
are separate on purpose, but the common entry point is:

```sh
just test-suite [serial|all]
```

With no argument, `test-suite` expects exactly one connected adb device. Pass a serial to target one
device, or `all` to run the device-backed flows on every connected device.

## What Each Flow Covers

- `just host-test`: host-target Rust unit tests for platform-independent code.
- `just check`: Android arm64 clippy for the main crate and the ART self-test cdylib, plus
  Android example compilation.
- `just unit-test [serial|all]`: Rust library unit tests running through `cargo-ndk-runner`.
- `just app-test [serial|all]`: app_process harness for live JNI, class-loader, metadata,
  enumeration, replacement, and main-thread behavior in an already-created ART process.
- `just apk-perform-test [serial|all]`: APK startup harness for early `Java::perform()`, app-loader
  publication, and main-looper scheduling during real app startup.
- `just art-test [serial|all]`: native bootstrap integration-test target for manual ART loading
  and VM creation.

## Where The Pieces Live

- `src/art_selftest/app_process/`: internal Rust checks for the app_process harness.
- `src/art_selftest/apk_perform.rs`: internal Rust checks for the APK startup harness.
- `crates/art-selftest-cdylib/`: Android cdylib wrapper that exports the app_process JNI method
  and APK startup agent entrypoint.
- `tests/art_bootstrap.rs`: native ART bootstrap integration-test target. It uses
  `harness = false` because the runner must start it with ART-specific environment variables.
- `tests/fixtures/app-process/`: Java classes used by the app_process harness.
- `tests/fixtures/apk/`: Android manifest and Java classes used by the APK startup harness.
- `target/test-fixtures/`: generated classes, dex files, APKs, jars, and signing material.
