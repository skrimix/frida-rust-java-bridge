# Frida Rust Java Bridge

[![Android Supported](https://img.shields.io/badge/Platform-Android-brightgreen.svg)]()
[![Rust Language](https://img.shields.io/badge/Language-Rust-orange.svg)]()

A high-level, typesafe Rust-native Java bridge designed for Frida agents running inside Android Runtime (ART) processes.

`frida-rust-java-bridge` empowers you to write safe, expressive, and highly performant instrumentation agents in Rust. It takes care of JNI attachments, class-loader scopes, and raw memory mutations, allowing you to focus on interacting with, inspecting, and hooking Java code.

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

## Architectural Concept Map

To help you navigate the library, here is how the primary abstractions fit together:

```
            +---------------------------------------+
            |             Java::obtain()            |
            +-------------------+-------------------+
                                |
             Is startup complete & app loader ready?
              /                                   \
            Yes                                    No (Deferred)
            /                                       \
  +--------v-------+                           +-----v-------+
  | perform_now()  |                           |  perform()  |
  +--------+-------+                           +-----+-------+
           |                                         |
           +--------------------+--------------------+
                                |
                   +------------v-------------+
                   |  Active JNI Thread Scope |
                   +------------+-------------+
                                |
         +----------------------+----------------------+
         |                                             |
+--------v--------+                           +--------v--------+
|  High-Level API |                           |  Low-Level API  |
|  (Safe & Sweet) |                           |  (Raw & Unsafe) |
+--------+--------+                           +--------+--------+
         |                                             |
 - JavaClass (reflection & hooks)              - Env (direct JNIEnv mapping)
 - JavaObject (global reference)               - refs::LocalRef / GlobalRef
 - JavaArray (primitive/object lists)          - jni (raw C-style bindings)
 - IntoJavaArgs / JavaArgs                     - RawJavaObject
```

---

## Choose Your API Level

`frida-rust-java-bridge` provides two distinct ways to work with Java, matching your safety and performance requirements:

### High-Level Facade (Recommended)
This safe API layer behaves like Frida's JavaScript API wrappers, handling thread attachment, class-loader resolution, reflection, and lifetime tracking:
* **Dynamic Class Resolution:** `Java::use_class` searches both the bootstrap and application class loaders.
* **Safe Hooks:** `JavaMethod::replace` and `JavaConstructor::replace` let you hook Java methods with clean Rust closures, returning a RAII `JavaHookGuard` that automatically restores the original code when dropped.
* **Automatic Thread Attachment:** High-level calls attach the current thread to ART and detach when finished.
* **Thread Scheduling:** Use `Java::schedule_on_main_thread` to marshal calls onto Android’s main UI thread.

### Low-Level Raw JNI Layer
For performance-critical code or deep JNI integrations, the low-level layer maps directly to native JNI specifications:
* **`Env` Wrapper:** Exposes raw JNI environment lookups, method calls, array region copies, and exception checks.
* **Explicit Reference Types:** Types like `LocalRef`, `BorrowedLocalRef`, and `GlobalRef` strictly control JNI reference lifecycles.
* **No Safety Net:** Operations bypass class-loader tracking and require explicit `unsafe` blocks. Use only if you can uphold JNI thread and reference boundaries.

---

## Platform & Compatibility Scope

This crate is currently dedicated to modern **ART**. Dalvik, desktop JVMs (HotSpot), and JVM TI are not supported.

* **Milestone Focus:** Currently optimized for `arm64-v8a` architectures.
* **Version Safety:** Dynamic tasks like method replacement, heap enumeration, and class-loader tracking probe the host ART process at runtime. If a symbol layout isn't supported on the current Android version, the bridge yields a structured, recoverable `UnsupportedFeature` error instead of causing instability.

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
