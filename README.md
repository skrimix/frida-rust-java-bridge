# Frida Rust Java Bridge

[![Android Supported](https://img.shields.io/badge/Platform-Android-brightgreen.svg)]()
[![Rust Language](https://img.shields.io/badge/Language-Rust-orange.svg)]()

A high-level Rust Java bridge designed for Frida agents running inside Android app processes.

> This crate is currently in alpha stage. Exported APIs, module organizations, and names may shift as we refine safe wrappers and ART-specific mechanics.

---

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
* **Automatic thread attachment:** High-level calls attach the current thread to ART as needed.
* **Main-thread scheduling:** `Java::schedule_on_main_thread` runs work on Android's main UI thread.

Lower-level JNI and ART functionality is also available for cases that need direct access.

---

## Platform & Compatibility Scope

This crate currently supports Android 8-16 on `arm64-v8a`. Dalvik, desktop JVMs, JVM TI,
and other Android architectures are not supported.

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
