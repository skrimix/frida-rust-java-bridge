# Current Behavior Notes

This crate targets Android ART only. These notes describe the current prototype behavior and the
soft-frozen draft shapes that are useful enough to avoid casual churn for now. They are not stability
contracts: this project is private, pre-user, and all exported Rust APIs may change when that makes
the bridge clearer or safer.

The current Rust API keeps VM attachment, JNI descriptors, reference ownership, and class-loader
boundaries explicit instead of cloning the GumJS `Java.use()` surface.

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
  `Error::AppClassLoaderUnavailable`. It does not defer, hook app startup, or fall back to
  enumerated/thread-context loaders.
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

## Unsupported Features

Unsupported runtime capabilities are explicit:

- ART class-loader and loaded-class enumeration return `Error::UnsupportedFeature` when required
  symbols, architecture support, API level, thread transition, or runtime layout detection are not
  available.
- `Runtime::capabilities()`, `Vm::capabilities()`, and `Java::capabilities()` report the same
  support decisions used by the current enumeration APIs.
- Heap enumeration, deoptimization, and finished ergonomic method replacement are intentionally
  reported as unsupported until they get their own prototype lanes. Hidden test-only method
  replacement probes may report that ART prerequisites, cloned `ArtMethod` preparation, and
  safe-patching guardrails are available for selected static and instance primitive/void, `String`,
  and one-reference-argument methods, including object-array argument/return test coverage. The
  active hidden path uses cloned-method dispatch and has
  thread-scoped, stack-aware raw original invocation for selected static and instance primitive,
  `String`, and reference argument/return paths, including object arrays and null JNI values.
  Original calls may be made through captured overload metadata with `IntoJavaArgs` containers and
  typed raw-return extraction. An overload-first facade exists under `experimental` for selected
  `JavaMethodOverload` values, and a descriptor-driven raw JNI-native layer accepts the same
  currently tested ABI shapes. Both still take explicit `unsafe extern "C"` JNI callbacks and
  remain high-risk prototype APIs. Dedicated test coverage exercises replace/revert/replace
  lifecycle behavior on the same static and instance `ArtMethod` through direct helpers, the raw
  JNI-native layer, and the overload facade. Test failures should remain visible when ART
  instrumentation is incomplete; this still does not make replacement a soft-frozen capability.
  Arbitrary object/multi-reference signatures, closure-backed callbacks,
  deoptimization, and `.implementation`-style APIs remain outside the current prototype boundary.

The current live-runtime ART enumeration and hidden replacement milestone is API 26+ on arm64.
Hardening should keep device-specific failures visible until the underlying ART layout or behavior
is understood and fixed. Replacement hardening uses both the native in-process test harness and the
app-process test harness.
