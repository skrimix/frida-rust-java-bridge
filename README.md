# Frida Rust Java Bridge

[![Android Supported](https://img.shields.io/badge/Platform-Android%20ART-brightgreen.svg)]()
[![Rust Language](https://img.shields.io/badge/Language-Rust-orange.svg)]()

A high-level, typesafe Rust-native Java bridge designed for Frida agents running inside Android ART (Android Runtime) processes. 

`frida-rust-java-bridge` empowers you to write safe, expressive, and highly performant instrumentation agents in Rust. It takes care of JNI attachments, class-loader scopes, and raw memory mutations, allowing you to focus on interacting with, inspecting, and hooking Java code.

> [!IMPORTANT]
> This crate is currently in its pre-user/development phase. Exported APIs, module organizations, and names may shift as we refine safe wrappers and ART-specific mechanics.

---

## 🚀 Getting Started

To interact with Java, obtain the process handle via `Java::obtain()` and execute your instrumentation code within a `perform` block:

```ignore
use frida_rust_java_bridge::{Java, Result};

fn instrument_app() -> Result<()> {
    // 1. Get the primary JVM handle for the current Android process
    let java = Java::obtain()?;

    // 2. Perform actions safely within an attached thread environment
    java.perform(|java| {
        // Look up a class (resolving the correct application class loader)
        let activity_class = java.use_class("android.app.Activity")?;

        // Call static methods, construct instances, or hook methods!
        let class_name: String = activity_class.call("getName", ())?;
        println!("Active class name: {}", class_name);

        Ok(())
    })?;

    Ok(())
}
```

*Note: The code above is marked `ignore` because it is designed to run exclusively inside an Android process where Frida has injected your Rust agent.*

---

## 🗺️ Architectural Concept Map

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

## 🎨 Choose Your API Level

`frida-rust-java-bridge` provides two distinct ways to work with Java, matching your safety and performance requirements:

### 🌟 High-Level Facade (Recommended)
This safe API layer behaves like upstream Frida's JavaScript wrappers, handling thread attachment, class-loader resolution, reflection, and lifetime tracking:
* **Dynamic Class Resolution:** `Java::use_class` searches both the bootstrap and application class loaders.
* **Safe Hooks:** `JavaMethod::replace` and `JavaConstructor::replace` let you hook Java methods with clean Rust closures, returning a RAII `JavaHookGuard` that automatically restores the original code when dropped.
* **Automatic Thread Attachment:** High-level calls attach the current thread to ART and detach when finished.
* **Thread Scheduling:** Use `Java::schedule_on_main_thread` to marshal calls onto Android’s main UI thread.

### ⚙️ Low-Level Raw JNI Layer
For performance-critical code or deep JNI integrations, the low-level layer maps directly to native JNI specifications:
* **`Env` Wrapper:** Exposes raw JNI environment lookups, method calls, array region copies, and exception checks.
* **Explicit Reference Types:** Types like `LocalRef`, `BorrowedLocalRef`, and `GlobalRef` strictly control JNI reference lifecycles.
* **No Safety Net:** Operations bypass class-loader tracking and require explicit `unsafe` blocks. Use only if you can uphold JNI thread and reference boundaries.

---

## 🎯 Platform & Compatibility Scope

This crate is dedicated **exclusively** to modern **Android ART**. Dalvik, desktop JVMs (HotSpot), JVM TI, and JavaScript API bridges are outside the scope of this project.

* **Milestone Focus:** Currently optimized for `arm64-v8a` architectures.
* **Version Safety:** Dynamic tasks like method replacement, heap enumeration, and class-loader tracking probe the host ART process at runtime. If a symbol layout isn't supported on the current Android version, the bridge yields a structured, recoverable `UnsupportedFeature` error instead of causing instability.

---

## 📚 Project Documentation

To understand our development status, roadmaps, and guidelines, take a look at these supplementary files:

| Document | Purpose |
| --- | --- |
| [AGENTS.md](AGENTS.md) | **Developer Rules:** Guidelines for library structure, architecture, build instructions, and testing. |
| [CURRENT_BEHAVIOR.md](CURRENT_BEHAVIOR.md) | **State Snapshot:** Details on what features work on active test configurations. |
| [FEATURE_PROGRESS.md](FEATURE_PROGRESS.md) | **Capability Grid:** A matrix of upstream-aligned features. |
| [ROADMAP.md](ROADMAP.md) | **Sequence Plan:** Chronological plan for features, security hardening, and stable targets. |
| [DOCUMENTATION_PASS.md](DOCUMENTATION_PASS.md) | **Style Guide:** Our pedagogical standards and human-centric tone rules. |
| [DOCS_PROGRESS.md](DOCS_PROGRESS.md) | **Tracker:** Sprint status and completion log for our documentation overhaul. |

---

## 🛠️ Build & Dev Commands

Common commands are managed using standard `justfile` recipes:

* `just check` - Runs Android clippy with test features.
* `just build` / `just build-release` - Compiles Android arm64 crate targets.
* `just host-test` - Runs platform-independent target unit tests on the host.
* `just unit-test all` - Builds and runs unit tests on a connected Android device.
* `just app-test all` - Deploys and runs the full ART app-process integration harness.
