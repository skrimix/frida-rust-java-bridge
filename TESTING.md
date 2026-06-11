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
- `just check`: Android arm64 clippy with all feature-gated harness code compiled.
- `just unit-test [serial|all]`: Rust library unit tests running through `cargo-ndk-runner`.
- `just app-test [serial|all]`: app_process harness for live JNI, class-loader, metadata,
  enumeration, replacement, and main-thread behavior in an already-created ART process.
- `just apk-perform-test [serial|all]`: APK startup harness for early `Java::perform()`, app-loader
  publication, and main-looper scheduling during real app startup.
- `just art-test [serial|all]`: native bootstrap integration-test target for manual ART loading
  and VM creation.

## Where The Pieces Live

- `src/test_harness/app_process/`: Rust side of the app_process harness.
- `src/test_harness/apk_perform.rs`: Rust agent used by the APK startup harness.
- `tests/art_bootstrap.rs`: native ART bootstrap integration-test target. It uses
  `harness = false` because the runner must start it with ART-specific environment variables.
- `test-fixtures/src/`: Java classes used by the app_process harness.
- `test-fixtures/apk/`: Android manifest and Java classes used by the APK startup harness.
- `test-fixtures/dex/`: committed dex fixture loaded by app-process checks.
- `test-fixtures/build/`: generated classes, dex files, APKs, and signing material.

The older aliases are still available. In particular, `just test [serial|all]` and `just test-all`
remain compatibility names for the app_process harness only.
