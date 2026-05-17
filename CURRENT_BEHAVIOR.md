# Current Behavior Notes

This crate targets Android ART only. These notes describe the current prototype behavior and the
soft-frozen draft shapes that are useful enough to avoid casual churn for now. They are not stability
contracts: this project is private, pre-user, and all exported Rust APIs may change when that makes
the bridge clearer or safer.

The current Rust API keeps VM attachment, JNI descriptors, reference ownership, and class-loader
boundaries explicit instead of cloning the GumJS `Java.use()` surface.

## Runtime And Attachment

- `Runtime::android_version()`, `Vm::android_version()`, and `Java::android_version()` return the
  Android release string and SDK API level read from system properties. The `android_api_level()`
  helpers expose just the parsed SDK integer; ART layout probing uses the same API-level reader
  internally.
- `Runtime::perform_now()` and `Vm::perform_now()` attach the current thread for a synchronous
  callback and pass a bootstrap-scoped `Java` handle. `Java::perform_now()` does the same while
  preserving the receiver's loader scope. These helpers do not queue work, install app-loader
  hooks, or wait for `ActivityThread.currentApplication()`.

## Class Names And Descriptors

- User-facing class names returned by `JavaClass::name()`, `JavaClassMetadata::name`, and
  `JavaMethodQueryClass::name` use Java binary names, matching upstream `frida-java-bridge`
  metadata output: `java.lang.String`, `com.example.Outer$Inner`.
- Array class names follow `java.lang.Class.getName()` style: `[I`, `[Ljava.lang.String;`.
- Descriptors and `JavaType` values remain JNI descriptor/internal-name based:
  `Ljava/lang/String;`, `[Ljava/lang/String;`.
- `Java::find_class()` accepts dotted binary names, slash-style JNI internal names, object
  descriptors, and array descriptors. Returned names are normalized to dotted names after lookup.

## Class Loader Scope

- A plain `Java` handle uses bootstrap-style `FindClass` lookup.
- `Java::with_loader()` returns a new loader-backed handle that resolves classes through the
  supplied `ClassLoaderRef`.
- `Java::app_class_loader()` synchronously resolves the current Android app loader through
  `ActivityThread.currentApplication().getClassLoader()` when an app `Application` is already
  available. `Java::with_app_loader()`, `Runtime::app_java()`, and `Vm::app_java()` return
  loader-backed handles for that app loader.
- If `ActivityThread.currentApplication()` is null, app-loader selection returns
  `Error::AppClassLoaderUnavailable`. It does not fall back to enumerated/thread-context loaders.
- `Java::perform()`, `Runtime::perform()`, and `Vm::perform()` register Rust callbacks that run
  with an app-loader-scoped `Java`. If the app loader is already available the callback runs
  synchronously and the returned `PerformHandle` reports `Completed` or `Failed`. Otherwise the
  callback is queued and process-global, experimental Android startup hooks are installed through
  the hidden ART method replacement backend. The current hook set drains from
  `LoadedApk.makeApplicationInner`/`makeApplication` and supported
  `ActivityThread.getPackageInfo` overloads when those hook points are present. Deferred setup
  returns `UnsupportedFeature` if neither make-application nor get-package-info hook coverage can
  be installed.
  The APK startup-agent test validates the intended early bind-time case: registration from
  `Agent_OnAttach` before `LoadedApk.makeApplication*` has created the real app `Application`.
  Registering from inside already-running app code is still covered by the immediate app-loader
  path, not by this early-start drain guarantee.
- `Runtime::capabilities()`, `Vm::capabilities()`, and `Java::capabilities()` report app-loader
  deferral separately from raw method replacement through `app_loader_deferral`. The capability is
  `Experimental` only when method-replacement prerequisites and at least one supported Android
  startup hook shape are probeable without installing hooks. Missing replacement prerequisites or
  missing `LoadedApk.makeApplication*`/`ActivityThread.getPackageInfo` hook shapes are reported as
  `Unsupported` with the concrete reason.
- `Java::is_main_thread()`, `Runtime::is_main_thread()`, and `Vm::is_main_thread()` compare
  `Looper.myLooper()` with `Looper.getMainLooper()`. Threads without a Java looper report `false`.
- `Java::schedule_on_main_thread()`, `Runtime::schedule_on_main_thread()`, and
  `Vm::schedule_on_main_thread()` queue `Send + 'static` Rust callbacks and wake the Android main
  looper with `Handler(Looper.getMainLooper()).sendEmptyMessage(1)`. Scheduling always queues,
  including when called from the main thread, matching upstream's scheduling behavior rather than
  running inline. The callback receives a clone of the scheduling `Java` handle, preserving its
  loader scope. The current drain point is a process-global Gum hook on `epoll_wait`; missing
  `epoll_wait`, hook installation failure, or main-looper wakeup failure are explicit
  `UnsupportedFeature`/error outcomes. `MainThreadTaskHandle` reports `Pending`, `Completed`, or
  `Failed`.
- Capabilities also report main-thread scheduling separately through `main_thread_scheduling`. The
  support probe checks for `epoll_wait`, `Looper.getMainLooper()`, and the `Handler` constructor /
  `sendEmptyMessage(int)` wakeup shape without installing the Gum hook, enqueueing callbacks, or
  sending a looper wakeup. The scheduling API remains experimental; its handle/status shape is a
  soft-freeze candidate after matrix hardening. Command-line `app_process` test runs currently
  report this capability as unsupported because `Looper.getMainLooper()` returns null; the APK
  early-start harness is the live validation path for real Android main-looper drain behavior.
- Successful class caches are per `Java` instance. Bootstrap, system-loader, DexClassLoader, and
  app/enumerated-loader handles do not share cached `JavaClass` values.
- `JavaObject` stores only VM and JNI reference ownership. It does not infer or remember the
  defining class loader; callers should keep using the relevant loader-backed `Java` handle for
  follow-up class/member lookup.

## Wrapper Object Helpers

- `Java::use_class()` returns a Rust-native wrapper around the current handle's class-loader scope.
- Wrapper overload selection remains explicit through argument type lists or descriptor/source-style
  type names; there is no automatic JS-style overload dispatch in the current facade.
- Wrapper and selected-overload calls accept unit, tuples, arrays, slices, or vectors through
  `IntoJavaArgs`, while still marshaling through explicit `JavaValue` values internally.
- Selected method overloads and field handles expose narrow typed helpers for common primitive,
  object, and string-return paths so callers do not need to manually unwrap every `JavaReturn`.
- `JavaObject` is already an owned global JNI reference. `JavaObject::retain()` creates another
  owned global reference to the same Java object.
- `JavaClass::is_instance()`, `JavaClassWrapper::is_instance()`, and `JavaClassWrapper::cast()`
  validate runtime object type with JNI `IsInstanceOf`.
- `JavaClassWrapper::cast()` returns a retained object after validation. It does not infer,
  discover, or switch to the object's defining class loader.

## Arrays

- `JavaArray` owns a global JNI reference plus an explicit `JavaType` element type. Arrays can be
  passed as `JavaValue` arguments and array-returning methods/fields produce `JavaReturn::Array`.
- `Java::new_object_array()` creates object arrays with nullable elements, and `JavaArray` exposes
  nullable object element get/set helpers.
- `Java::new_boolean_array()`, `new_byte_array()`, `new_char_array()`, `new_short_array()`,
  `new_int_array()`, `new_long_array()`, `new_float_array()`, and `new_double_array()` create
  primitive arrays. `JavaArray` exposes full-array copy-in/copy-out helpers for each primitive
  type, backed by JNI region APIs.
- Boolean arrays use `bool` at the high-level `Java`/`JavaArray` boundary and JNI `jboolean`
  internally. This is not a JS-style mutable array proxy.

## `ClassLoaderKind`

- `System`: returned by `ClassLoader.getSystemClassLoader()`.
- `App`: selected from the current Android `Application` by the synchronous app-loader resolver.
- `Object`: explicitly wrapped from a Java object after runtime type validation.
- `Enumerated`: discovered through ART class-loader enumeration.

`ClassLoaderKind` describes provenance only. It is not a stable loader identity key.

## Method Queries

`Java::enumerate_methods()` accepts `class!method` queries:

- Class patterns use dotted Java binary names and simple `*` / `?` glob matching.
- Method patterns use declared Java method names. Constructors are exposed as `$init`; class
  initializers are skipped.
- `/i` enables ASCII case-insensitive matching.
- `/s` matches signature-aware method names such as `overload(Ljava/lang/String;)Ljava/lang/String;`.
- `/u` skips bootstrap/platform classes such as `java.*`, `android.*`, and `com.android.*`.
- Without `/s`, overloads with the same method name are de-duplicated per class.
- `JavaMethodMetadata::modifiers` and `JavaFieldMetadata::modifiers` remain raw Java reflection
  bitfields. Public constants such as `ACC_PRIVATE`, `ACC_STATIC`, and `ACC_SYNTHETIC` are
  available for named bit checks.

## Unsupported Features

Unsupported runtime capabilities are explicit:

- ART class-loader and loaded-class enumeration return `Error::UnsupportedFeature` when required
  symbols, architecture support, API level, thread transition, or runtime layout detection are not
  available.
- `Runtime::capabilities()`, `Vm::capabilities()`, and `Java::capabilities()` report the same
  support decisions used by the current enumeration APIs, plus explicit experimental/unsupported
  support for app-loader deferral and main-thread scheduling.
- Heap enumeration and deoptimization are intentionally reported as unsupported until they get
  their own prototype lanes. Method replacement is reported as experimental when current ART
  prerequisites are available, and unsupported when a prerequisite is missing. Method
  replacement probes may report that ART prerequisites, cloned `ArtMethod` preparation, and
  safe-patching guardrails are available for selected static and instance primitive/void, `String`,
  and one-reference-argument methods, including object-array argument/return test coverage. The
  active hidden path uses cloned-method dispatch and has
  thread-scoped, stack-aware raw original invocation for selected static and instance primitive,
  `String`, and reference argument/return paths, including object arrays and null JNI values.
  A few exact startup-hook ABIs are admitted for deferred app-loader initialization; they are not
  general arbitrary multi-reference replacement support.
  Original calls may be made from public `.implementation` callbacks through
  `ImplementationInvocation::call_original()` with `IntoJavaArgs` containers. Selected
  `JavaMethodOverload` values expose only unsafe `implementation()` as the public experimental
  replacement facade; it returns an explicit `ImplementationGuard`, receives
  `ImplementationInvocation`, and returns `ImplementationReturn` with borrowed object/array return
  helpers. Raw JNI-native helpers, raw closure callbacks, captured original-method handles, and
  descriptor-driven replacement admission remain crate-internal scaffolding for the app startup
  hooks and live-runtime harness. Callback errors, panics, or wrong return kinds are stored on the
  guard and return the JNI default value for the Java method's return type. This public API remains
  a high-risk prototype.
  A second active replacement for the same resolved `ArtMethod` is rejected; callers must explicitly
  revert or drop the first guard before replacing the method again. Explicit guard reverts are
  retryable on failure. This explicit guard lifecycle is the intended Rust model rather than a
  temporary substitute for GumJS-style assignment to an `implementation` property. If a live guard is
  dropped and restore fails, replacement clone/thunk memory is intentionally kept mapped instead of
  freeing executable state that ART may still reference.
  Dedicated test coverage exercises replace/revert/replace lifecycle behavior on the same static
  and instance `ArtMethod` through the public `.implementation` guard plus internal direct,
  raw JNI-native, and closure-backed helpers. Test failures should remain visible when ART
  instrumentation is incomplete; this still does not make replacement a soft-frozen capability.
  Arbitrary object/multi-reference signatures, deoptimization, and broader closure ergonomics remain
  outside the current prototype boundary.

The current live-runtime ART enumeration and hidden replacement milestone is API 26+ on arm64.
Hardening should keep device-specific failures visible until the underlying ART layout or behavior
is understood and fixed. Replacement hardening uses both the native in-process test harness and the
app-process test harness. When replacement corrupts ART's view of a method, even ordinary Java
exception stack capture may abort while resolving quick frames; the app-process harness prints the
native failure before throwing so both the native error and any Java stack remain visible.
