# Frida Rust Java Bridge

[![Android Supported](https://img.shields.io/badge/Platform-Android-brightgreen.svg)]()
[![Rust Language](https://img.shields.io/badge/Language-Rust-orange.svg)]()
[![Docs](https://img.shields.io/badge/Docs-Online-blue.svg)](https://skrimix.github.io/frida-rust-java-bridge/)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)]()

A high-level Rust Java bridge designed for Frida agents running inside Android app processes.

> This crate is currently in alpha stage. Exported APIs, module organizations, and names may shift as safe wrappers and ART-specific mechanics are being refined.

---

## Adding to your project

Add the bridge as a Git dependency in your `Cargo.toml`:

```toml
[dependencies]
frida-rust-java-bridge = { git = "https://github.com/skrimix/frida-rust-java-bridge" }
```

Use Android NDK with [cargo-ndk](https://github.com/bbqsrc/cargo-ndk) for building:

```sh
cargo ndk -t arm64-v8a build
```

## Getting Started

Start by obtaining the Java runtime handle with `Java::obtain()`. To work with application classes,
you need to initialize the app class loader first. Choose the method that fits your use case:

### Non-blocking (callback-based)

Use `Java::perform()` when your code runs during app startup and cannot block the current thread.
The callback runs automatically once the app loader is ready:

```rust
use frida_rust_java_bridge::{Java, Result};

fn hook_during_startup() -> Result<()> {
    let java = Java::obtain()?;

    java.perform(|java| {
        let target = java.use_class("com.example.Target")?;
        let answer: i32 = target.call("answer", ())?;
        println!("answer = {}", answer);
        Ok(())
    })?;

    Ok(())
}
```

### Blocking (synchronous)

Use `Java::wait_for_app_loader()` when you can block and want straightforward, sequential code.
This returns immediately if the app loader is already known or currently available. Otherwise, it
waits until the loader is ready, then gives you a handle you can use directly:

```rust
use std::time::Duration;
use frida_rust_java_bridge::{Java, Result};

fn hook_with_blocking() -> Result<()> {
    let java = Java::obtain()?;
    let java = java.wait_for_app_loader(Duration::from_secs(5))?;
    let scope = java.attach()?;

    let target = scope.use_class("com.example.Target")?;
    let answer: i32 = target.call("answer", ())?;
    println!("answer = {}", answer);

    Ok(())
}
```

Use `Duration::ZERO` to check only the already-known and immediate
`ActivityThread.currentApplication()` paths without installing deferred startup hooks.

### After initialization

App-loader setup is a one-time step. Once initialized by either method above, later code can
call high-level Java APIs directly. You do not need to call `Java::attach()` before `use_class()`,
method calls, field access, or other wrapper operations; those attach the current thread as needed.
Use `Java::attach()` when you want several synchronous operations to reuse one attached scope, or
when you need direct JNI-style access through `JavaScope::env()`:

```rust
use frida_rust_java_bridge::{Java, Result};

fn use_java_after_init() -> Result<()> {
    let java = Java::obtain()?;  // App loader was already set up earlier
    let scope = java.attach()?;  // Optional, but avoids repeated attach checks in this block

    let activity = scope.use_class("android.app.Activity")?;
    let name: String = activity.call("getName", ())?;
    println!("Class name: {}", name);

    Ok(())
}
```

For system classes or when you already have the right loader scope, use `Java::perform_now()` to
run immediately without waiting for the app loader.

---

## Features

`frida-rust-java-bridge` provides high-level Rust wrappers for common Java instrumentation work:

* **Class access:** `Java::use_class` works with bootstrap and application classes.
* **Method and field calls:** Call Java members with Rust argument and return conversions.
* **Method and constructor replacement:** `JavaMethod::replace` and `JavaConstructor::replace`
  install guarded hooks and restore the original code when the guard is dropped.
* **Heap object listing:** (Android < 12 only) `Java::choose_instances` and `JavaClass::choose_instances` enumerate
  live instances of a class.
* **ART deoptimization:** Deoptimize everything, the boot image, or selected methods and
  constructors when the current ART runtime supports it.
* **Automatic thread attachment:** High-level calls attach the current thread to ART as needed.
* **Main-thread scheduling:** `Java::schedule_on_main_thread` runs work on Android's main UI thread.

Lower-level JNI and ART functionality is also available for cases that need direct access.

## Examples

The [`examples/`](examples/) directory has small Android-targeted workflows:

* [`basic_perform`](examples/basic_perform.rs) - app-loader setup and a simple static Java call.
* [`basic_blocking`](examples/basic_blocking.rs) - blocking app-loader setup for straightforward synchronous code.
* [`constructors_and_overloads`](examples/constructors_and_overloads.rs) - constructors, byte arrays, and explicit overload selection.
* [`metadata_and_enumeration`](examples/metadata_and_enumeration.rs) - loaded classes plus declared method and field metadata.
* [`main_thread`](examples/main_thread.rs) - scheduling work on Android's main thread.
* [`method_replacement`](examples/method_replacement.rs) - constructor and method replacements with hook guards.
* [`android_system_services`](examples/android_system_services.rs) - system service lookup, casting, and Android framework calls.
* [`raw_jni_slots`](examples/raw_jni_slots.rs) - direct JNI vtable slot inspection for low-level work.

---

## Compatibility

This crate currently supports only Android 8-16 on `arm64-v8a`.
Dalvik, desktop JVMs, JVM TI, and other Android architectures are not supported.

---

## Build & Dev Commands

Common commands are managed using standard `justfile` recipes:

* `just check` - Runs Android clippy with test features.
* `just build` / `just build-release` - Compiles Android arm64 crate targets.
* `just host-test` - Runs platform-independent target unit tests on the host.
* `just unit-test all` - Builds and runs unit tests on a connected Android device.
* `just app-test all` - Deploys and runs the full ART app-process integration harness.
* `just test-suite all` - Runs the full local and Android-backed test suite.

See [TESTING.md](TESTING.md) for the test harness map and when to use each flow.

---

## AI disclosure

This project was almost entirely written by Codex. I only did what I could to steer it 
to keep the architecture sane, API usable and features tested. This works for my use-case, 
but your mileage may vary. If you find rough edges, please open an issue, I'll try to fix them.

---

## License

Licensed under the [Apache-2.0 license](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in PyO3 by you, as defined in the Apache License, shall be dual-licensed as above, without any additional terms or conditions.
