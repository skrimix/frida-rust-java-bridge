# Current Behavior Notes

This crate targets Android Runtime (ART) only. These notes describe the current behavior. They are not
stability contracts: this project is private, pre-user, and exported Rust APIs may change when that
makes the bridge clearer or safer.

The default high-level shape is intentionally close to GumJS:

```rust
let java = Java::obtain()?;
java.perform(|java| {
    let activity = java.use_class("android.app.Activity")?;
    Ok(())
})?;
```

`Java::perform()` is the normal entry point for app classes. `Java::wait_for_app_loader()` is the
synchronous immediate-or-blocking path for code that can wait. `Java::attach()` is the lower-level
guard-shaped way to enter the same kind of synchronous attached scope yourself, while raw `Vm`/`Env`
access is for code that needs to talk about attachment or loader selection explicitly.

## Runtime And Attachment

- `Java::obtain()` discovers the current ART runtime through `JNI_GetCreatedJavaVMs` and
  returns a `Java` handle ready for JS-style `perform()` work. Low-level `find_class()` on that
  handle remains bootstrap-scoped. Runtime discovery remains internal plumbing; `Vm` is exposed as
  the low-level JNI attachment boundary behind `Java::vm()`.
- `Java::android_version()` returns the Android release string and SDK API level read from system
  properties. `Java::android_api_level()` exposes just the parsed SDK integer; ART layout probing
  uses the same API-level reader internally.
- `Java::attach()` enters a synchronous attached Java scope and returns a `JavaScope<'_>` guard.
  High-level APIs such as `Java::use_class()`, method calls, and field access attach the current
  thread as needed, so callers do not have to enter an explicit `attach()` scope for ordinary Java
  work. `attach()` is useful when several synchronous operations should reuse one attached scope,
  or when code needs direct `env()` access. `Java` remains the shareable VM plus optional loader
  scope; `JavaScope` additionally guarantees that the current thread has a valid `JNIEnv` for the
  lexical region and dereferences to `Java`; both `Java` and `JavaScope` implement `AsRef<Java>`,
  so helper APIs can accept either shape. `Env`, `AttachedEnv`, local references, and `JavaScope`
  are thread-affine.
- `Java::perform_now()` is the closure-shaped form of entering an immediate synchronous
  `JavaScope`. It preserves the receiver's loader scope and does not queue work, install app-loader
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

- A plain `Java` handle uses bootstrap-style `FindClass` lookup for low-level
  `Java::find_class()` calls.
- `Java::with_loader()` returns a new loader-backed handle that resolves classes through the
  supplied `ClassLoaderRef`. Loader-backed lookup validates the returned `Class.getName()` against
  the requested normalized name before promoting or caching the class; mismatches return
  `Error::ClassLookupMismatch`.
- `Java::app_class_loader()` synchronously resolves the current Android app loader through
  `ActivityThread.currentApplication().getClassLoader()` when an app `Application` is already
  available. It returns the loader object and does not publish it as the process default app loader.
  `Java::default_app_loader()` reports the already-published default without querying Android state
  or installing hooks.
- If `ActivityThread.currentApplication()` is null, app-loader selection returns
  `Error::AppClassLoaderUnavailable`. It does not fall back to enumerated/thread-context loaders.
- `Java::wait_for_app_loader(timeout)` blocks until the default app class loader is published and
  returns an app-loader-scoped `Java` handle. It first uses the already-published/default and
  immediate `ActivityThread.currentApplication()` paths, then installs the same deferred startup
  hooks used by `Java::perform()` and waits on the shared publication point. A zero timeout skips
  hook installation and only performs immediate checks. Timeout returns
  `Error::AppClassLoaderWaitTimedOut`; unsupported hook setup still returns `UnsupportedFeature`.
  This helper is intended for background/native helper threads and linear setup flows.
- `Java::perform()` registers Rust callbacks that run with an app-loader-scoped `JavaScope`. If
  the app default app loader has already been published, the callback uses it immediately. If
  `ActivityThread.currentApplication()` already exposes an application loader, that loader is
  published and the callback runs synchronously before this method returns. Otherwise the callback is
  queued and process-global Android startup hooks are installed through the internal ART method
  replacement backend. The current hook set observes
  `ActivityThread.handleBindApplication`, drains from
  `LoadedApk.makeApplicationInner`/`makeApplication`, and may use supported
  `ActivityThread.getPackageInfo` overloads for the early non-instrumented startup path. Startup
  drains are one-shot: `handleBindApplication` switches instrumented startup to the late
  make-application path, and once a startup drain publishes a loader, later startup-hook callbacks
  do not republish the default app loader. Each callback is attached before invocation; attachment
  failure and callback panic are recorded on the `PerformResult<T>`. The result is only for callers
  that want to observe queued startup work or keep the callback's eventual value; ordinary
  `Java.perform()`-style call sites can ignore it after `?`. Deferred setup returns
  `UnsupportedFeature` if neither make-application nor get-package-info hook coverage can be
  installed.
  The APK startup-agent test validates the intended early bind-time case: registration from
  `Agent_OnAttach` before `LoadedApk.makeApplication*` has created the real app `Application`.
  Registering from inside already-running app code is still covered by the immediate app-loader
  path, not by this early-start drain guarantee.
- `Java::perform()` returns a `PerformResult<T>` that exposes the `PerformHandle` status and owns
  the callback's eventual value, which is useful for deferred setup that returns hook guards or other
  lifetime tokens. JS-style side-effect callbacks naturally use `T = ()`.
- A common setup pattern is to call `Java::perform()` once to wait for and publish the app class
  loader, then call high-level Java APIs directly for later synchronous Java work without wrapping
  every operation in another callback. This only applies after the `perform()` callback has actually
  run, or after `Java::wait_for_app_loader()` has succeeded. `attach()` can make a batch of
  synchronous operations more efficient by reusing one lexical `JavaScope`; it does not defer app
  startup, discover the app loader by itself, or make low-level `find_class()` on a bare `Java`
  handle app-loader scoped.
- `Java::capabilities()` returns `JavaCapabilities`, reporting app-loader deferral separately from
  raw method replacement through `app_loader_deferral`. The capability is `Supported` when
  method-replacement prerequisites and at least one Android startup hook shape are probeable without
  installing hooks. Missing replacement prerequisites or missing
  `LoadedApk.makeApplication*`/`ActivityThread.getPackageInfo` hook shapes are reported as
  `Unsupported` with the concrete reason.
- `Java::is_main_thread()` compares `Looper.myLooper()` with `Looper.getMainLooper()`. Threads
  without a Java looper report `false`.
- `Java::schedule_on_main_thread()` queues `Send + 'static` Rust callbacks and wakes the Android
  main looper with `Handler(Looper.getMainLooper()).sendEmptyMessage(1)`. Scheduling always queues,
  including when called from the main thread, matching upstream's scheduling behavior rather than
  running inline. The callback receives a clone of the scheduling `Java` handle, preserving its
  loader scope. The current drain point is a process-global Gum hook on `epoll_wait`; missing
  `epoll_wait`, hook installation failure, or main-looper wakeup failure are explicit
  `UnsupportedFeature`/error outcomes. Callback panics are recorded as failed task statuses, and
  later queued callbacks continue draining. `MainThreadTaskHandle` reports `Pending`, `Completed`,
  or `Failed`, and `wait_for_completion(timeout)` blocks until a final status is available or
  returns `Error::MainThreadTaskWaitTimedOut`. Do not call the wait method from Android's main
  thread because that can prevent queued main-thread work from draining.
- Capabilities also report main-thread scheduling separately through `main_thread_scheduling`. The
  support probe checks for `epoll_wait`, `Looper.getMainLooper()`, and the `Handler` constructor /
  `sendEmptyMessage(int)` wakeup shape without installing the Gum hook, enqueueing callbacks, or
  sending a looper wakeup. Command-line `app_process` test runs currently report this capability as
  unsupported because `Looper.getMainLooper()` returns null; the APK early-start harness is the live
  validation path for real Android main-looper drain behavior.
- Successful low-level class caches are per `Java` instance. Bootstrap, system-loader,
  DexClassLoader, and enumerated-loader handles do not share cached `JavaClass` values. The
  published default app loader has a dedicated wrapper cache used by bare `Java::use_class()`;
  publishing a different app loader replaces that cache. Loader-backed classes are cached only after
  the returned Java class identity has matched the requested name.
- `JavaObject` stores VM, JNI reference ownership, and the wrapper class used for high-level member
  lookup. Casts and declared object returns can create new wrapper views over the same Java value
  without exposing a separate unbound object-reference type.
- High-level object and class-taking APIs accept sealed `JavaObjectRef` / `JavaClassRef` wrappers
  instead of user-implemented raw `jobject` providers. Raw JNI handles remain available through
  explicit `unsafe raw_*` APIs and low-level `Env` APIs. Internal raw extractor traits are
  crate-private, so there is no public safe raw-handle path.
- Low-level reflected-member ID wrapping through `Env::from_reflected_method()` and
  `Env::from_reflected_field()` is `unsafe`: callers must guarantee that the supplied kind,
  signature, or field type matches the reflected member. High-level metadata enumeration derives
  that data from Java reflection before using the raw conversion.
- `JavaValue<R>` is the shared Java value shape for arguments, returns, and hook inspection. It has
  `Void`, primitive variants, and one nullable reference lane `Object(Option<R>)`. Normal call
  arguments use the default raw-reference payload, normal returns use `JavaReturnRef`, and hook
  argument inspection uses callback-local reference payloads. Arbitrary raw `jobject` values still
  require the explicit unsafe `JavaValue::object_raw()` / `RawJavaObject::from_raw_jobject()` lane.

## Wrapper Object Helpers

- `Java::use_class()` returns a Rust-native wrapper. Explicit loader-backed handles use their
  current class-loader scope. A bare bootstrap `Java` handle prefers the published default app
  loader once `Java::perform()` or `Java::wait_for_app_loader()` has initialized it,
  matching upstream's default wrapper behavior without changing `Java::find_class()`.
- Wrapper method selection is Frida-like and explicit. `JavaClass::method("name")` returns a method
  group containing the currently visible non-constructor overloads; exact overload selection returns
  the selected `JavaMethod`. `JavaObject::method("name")` and
  `JavaBoundObject::method("name")` return the same method group bound to a receiver. Ordinary
  one-shot calls use `JavaClass::call::<T>("name", args)` or `object.call::<T>("name", args)`;
  these name-only calls dispatch to the best compatible overload from the runtime argument shape.
  Dispatch is deterministic and Frida-like, but intentionally smaller than Java compiler overload
  resolution: exact primitive/value matches beat numeric coercions, Rust strings prefer
  `String` before `CharSequence` before `Object`, concrete references and arrays prefer exact
  descriptors before broader reference targets, and remaining ties keep wrapper metadata order.
  Exact overloads use `call_with("name", ["TypeA", "TypeB"], args)` when behavior matters.
  Static-vs-instance is selected-overload metadata: class-bound calls dispatch only across static
  overloads because there is no `this`, while object-bound static calls use the class and still have
  no `this`. Method selection follows an upstream-like declared-first superclass walk: declared
  static and instance methods on the selected class are visible, any declared method name shadows
  superclass methods with the same name, and otherwise superclass static and instance methods are
  visible. Interface inherited/default methods are not walked for class-wrapper lookup.
  Specific constructors use `JavaClass::new_object_with(["Type"], args)` or a reusable
  `JavaClass::constructor(["Type"])` handle. `JavaClass::new_object(args)` uses the same ranked
  argument dispatch across declared constructors.
- Wrapper and selected-overload calls accept unit, bare single arguments, tuples, arrays, slices,
  or vectors through `IntoJavaCallArgs`, while still marshaling through explicit `JavaValue` values
  internally. They also accept Rust `&str`, `String`, and `&String` values for
  `java.lang.String`, `java.lang.CharSequence`, and `java.lang.Object` parameters, including inside
  mixed tuples such as `(object, "text", 0)`. Selected calls and wrapper field writes also perform
  conservative descriptor-driven numeric coercion: `int` may narrow to `byte`, `short`, or `char`
  with range checks or widen to `long`, `float` may widen to `double`, and finite in-range `double`
  may narrow to `float`. Exact selected calls, constructors, and field writes validate non-null
  reference arguments against the selected formal type before invoking JNI. Temporary Java string
  references are owned until the JNI call returns; low-level `Env` and `java::raw::Class` calls
  still take explicit `JavaValue` slices.
- Object-returning wrapper calls and fields bind non-null values to the declared return or field
  type using the selected wrapper's loader scope and return `JavaObject` values directly.
- The default facade uses generic typed receiver operations. On a `JavaClass`, `call` can invoke
  only static selected methods because no receiver is available; `get_field` / `set_field` can
  operate only on static selected fields for the same reason. On `JavaObject` and
  `JavaLocalObject`, `call` can invoke instance methods with `this` and static methods without
  `this`; `get_field` / `set_field` can access instance fields with the receiver and static fields
  through the receiver's class. Field selection is unified: `JavaClass::field("name")`,
  `JavaObject::field("name")`, and `JavaBoundObject::field("name")` select a visible static or
  instance field by name. Field selection also follows a declared-first superclass walk: any
  declared field name on the selected class shadows superclass fields with the same name, and
  otherwise superclass static and instance fields are visible. Detached selected instance method
  and field handles validate that the supplied receiver is an instance of the selected wrapper class
  before invoking JNI and return `InvalidObjectType` on mismatch.
- `JavaClass::replace("name", callback)` and `replace_with("name", ["Type"], callback)` select
  an unambiguous static or instance method for guarded replacement without requiring an intermediate
  method handle. `JavaClass::replace_constructor(["Type"], callback)` selects a constructor
  overload for guarded replacement and uses the same `JavaHookContext` callback shape as method
  hooks.
- `JavaObject::class()` returns the selected wrapper class used for member lookup, while
  `runtime_class()` exposes an uncached wrapper for the object's exact runtime class. Use
  `JavaClass::cast()` or `JavaObject::cast()` to create a validated wrapper view over the same Java
  value with a different selected class.
- `JavaObject` and `JavaArray` are default-global high-level wrappers over crate-owned JNI
  reference storage. `JavaArray` is an array-specific view backed by the same object wrapper core
  plus an explicit element type. Their local counterparts, `JavaLocalObject<'_>` and
  `JavaLocalArray<'_>`, are aliases over the same wrapper APIs with borrowed callback-local
  storage.
- `JavaObject::retain()`, `JavaArray::retain()`, `JavaLocalObject::retain()`, and
  `JavaLocalArray::retain()` create owned global references to the same Java value while preserving
  the selected wrapper class for object and array views. Callback-local borrowed views do not delete
  references on drop and can be passed to wrapper calls and field helpers while the producing
  callback/JNI frame is alive.
- `refs::LocalRef<'env, K>` is the lower-level owning JNI local-reference wrapper used by `Env`
  APIs. It deletes its local reference on drop and is intentionally separate from callback-local
  borrowed views.
- `JavaObject::java_to_string()`, `JavaArray::java_to_string()`, and their callback-local
  counterparts call Java `Object.toString()`. `get_string()` remains the direct helper for known
  `java.lang.String` values.
- `java::raw::Class::is_instance()`, `JavaClass::is_instance()`, and `JavaClass::cast()` validate
  runtime object type with JNI `IsInstanceOf`.
- `JavaClass::cast()` returns a retained `JavaObject` bound to that class after validation. It
  creates a new wrapper view over the same Java value; `JavaObject::cast()` is the receiver-side
  spelling for the same operation.

## Arrays

- `JavaArray` owns an object-backed JNI reference plus an explicit `JavaType` element type. Arrays
  can be passed as `JavaValue` arguments, and array-returning methods/fields produce the unified
  `JavaReturn` value with a `JavaReturnRef::Array` payload in the single reference lane.
- `Java::new_object_array()` creates object arrays with nullable elements, and `JavaArray` exposes
  nullable object element get/set helpers.
- `Java::new_boolean_array()`, `new_byte_array()`, `new_char_array()`, `new_short_array()`,
  `new_int_array()`, `new_long_array()`, `new_float_array()`, and `new_double_array()` create
  primitive arrays. `JavaArray` exposes full-array copy-in/copy-out helpers for each primitive
  type, backed by JNI region APIs.
- Low-level `Env::*_array_region()` helpers validate empty primitive regions as no-copy operations:
  null arrays are rejected, `start` must be in `0..=array_length`, and element kind is not checked
  when the requested region length is zero.
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
- `Java::capabilities()` reports the same support decisions used by the current enumeration APIs,
  including app-loader deferral and main-thread scheduling.
- Heap enumeration is supported when ART heap visitor prerequisites are available.
  `Java::choose_instances()` and `JavaClass::choose_instances()` enumerate live instances
  whose runtime class exactly matches the resolved class; callbacks return
  `JavaChooseControl::Continue` or `JavaChooseControl::Stop`, and objects must be retained inside
  the callback if they should outlive it. Unsupported ART layouts or missing heap symbols return
  `Error::UnsupportedFeature`. The current supported range is Android versions before Android 12 
  (newer requires JVM TI support).
  The `Heap::GetInstances` fallback also requires a readable ART thread slot and writable top
  handle-scope slot before installing its temporary ART handle scope.
- Deoptimization is exposed through `Java::deoptimize_everything()`,
  `Java::deoptimize_boot_image()`, `JavaMethod::deoptimize()`, and
  `JavaConstructor::deoptimize()`. The current ART milestone is Android API 26+ on arm64.
  Boot-image deoptimization calls `Runtime::DeoptimizeBootImage`; API 30+ full and selected
  deoptimization use ART Instrumentation, while API 26-29 use ART's Dbg/JDWP deoptimization
  request path. `JavaCapabilities::deoptimization` reports supported only when the current runtime
  has the symbols and layout probes needed by all public deoptimization operations; missing
  prerequisites return `Error::UnsupportedFeature` with the concrete reason. Deoptimizing a method
  while this crate has an active replacement installed for the same resolved `ArtMethod` is rejected
  with `UnsupportedFeature`; callers should revert the replacement before selected-method
  deoptimization.
  Method replacement is reported as supported when current ART prerequisites are available, and
  unsupported when a prerequisite is missing. Method
  replacement probes may report that ART prerequisites, cloned `ArtMethod` preparation, and
  safe-patching guardrails are available for the descriptor-driven closure-backed replacement path.
  The internal path uses cloned-method dispatch and has thread-scoped, stack-aware raw original
  invocation for static, instance, and constructor callbacks, including object arrays and null JNI
  values.
  The intended ergonomic path is class-level direct replacement, for example:
  `let activity = java.use_class("android.app.Activity")?;`,
  and `let guard = activity.replace("onResume", |ctx| { ctx.call_original::<()>(())?; ctx.ret(()) })?;`.
  Original calls may be made from public `replace` callbacks through
  `JavaHookContext::call_original()` with `IntoJavaArgs` containers, including bare single
  `JavaValue`-convertible arguments, `ctx.args()` for current-argument forwarding, and
  `java_args![...]` / `JavaArgs` for long explicit lists. Raw original calls with explicit argument
  lists remain unsafe through `JavaHookContext::call_original_raw()`. `JavaHookReturn` is the hook-facing
  `JavaValue` specialization with raw reference payloads; normal wrapper calls use `JavaReturn`,
  which is the same value shape with owned wrapper-reference payloads. Selected `JavaMethod` and
  `JavaConstructor` values expose safe `replace()` as the public replacement API. Replacement uses
  public callback/return/guard types under `java::replacement::*`; it returns an explicit
  `JavaHookGuard`, receives `JavaHookContext`, and returns primitives or explicit `JavaHookReturn`
  values with iterable safe argument views and typed argument helpers. Public admission accepts
  descriptors that fit the current arm64 hook limits, including mixed primitive/reference arguments
  and arrays. Constructor callbacks are exposed as `<init>` / `MethodKind::Constructor`, receive
  the allocated receiver, and return void, usually through `ctx.ret(())`. `call_original()` invokes
  the selected original constructor on that receiver when the callback chooses to forward to it;
  callbacks that skip the original constructor are responsible for leaving the receiver usable by
  later Java code.
  Unsupported facade signatures fail before installation with errors naming the method kind, method
  name, and a concise reason.
  Backend callback machinery, captured original-method handles, and backend replacement admission
  remain crate-internal scaffolding for the public facade, app startup hooks, and backend coverage. Callback
  errors, panics, or wrong return kinds are stored on the guard and may be reported immediately
  through `JavaHookGuard::on_error()` / `set_error_handler()`. Non-Java callback failures return
  the JNI default value for the Java method's return type. Java exceptions raised by original-call
  helpers or safe Java wrapper calls inside the callback are logged/recorded the same way but are
  restored before returning to Java when the callback returns that
  Java-backed error, so the Java caller observes the original throwable instead of a default value.
  Replacement callbacks expose borrowed local helpers through
  `JavaHookContext::{arguments,arg_value,arg_display,this_object,arg_object,arg_array}` and
  original-call helpers for object and array returns. `JavaHookContext::arg()` and
  `call_original()` support `String` and `Option<String>` conversions for Java string lanes.
  `call_original()` can extract either callback-local `JavaLocalObject` / `JavaLocalArray` values or
  retained owned `JavaObject` / `JavaArray` values. `arg()` also supports `JavaLocalObject`,
  `Option<JavaLocalObject>`, `JavaLocalArray`, and `Option<JavaLocalArray>` for
  descriptor-matching object and array parameters. Callback-local object/array wrappers borrow from
  the invocation lifetime. Replacement callbacks may safely return `String`, `&str`, `JavaObject`,
  `JavaArray`, borrowed wrapper references, or nullable variants through `IntoJavaHookReturn`; the
  replacement layer creates a callback-local JNI reference before handing wrapper and Rust string
  returns back to ART. Replacement callbacks run inside an internal JNI local frame: temporary locals are
  discarded when the callback exits, while accepted object/array returns are promoted through
  `PopLocalFrame` before control returns to ART. Lifetime-bound `JavaLocalObject` /
  `JavaLocalArray` values can be returned safely by converting them while the invocation is still
  live, for example
  `invocation.ret(invocation.arg_object(0)?)`. This conversion returns an explicit `JavaHookReturn`,
  avoiding the single-`R` lifetime limit on `replace()` callback returns. Raw object/array return
  construction remains explicit `unsafe` through `JavaHookReturn::object()`, `array()`,
  `raw_object()`, and `raw_array()`. Explicit null branches remain safe through
  `JavaHookReturn::null_object()` / `null_array()`.
  `arg_is_null(index)` provides a descriptor-checked shorthand for common nullable object/array
  branches.
  `JavaObject`, `JavaLocalObject`, `JavaArray`, and `JavaLocalArray` expose `java_to_string()` for
  Java `Object.toString()` text. Owned `JavaReturn` and `JavaHookArgument` expose `java_display()`
  for diagnostic text. Primitive, null, and void values are formatted directly; reference values use
  Java's `Object.toString()` behavior, so arrays intentionally display as Java array references such
  as `[I@...` rather than expanded contents. `arg_display()` is the hook-context single-argument
  convenience wrapper over the same display behavior. Class, constructor, method, and field
  wrappers expose their metadata through explicit accessors such as `name()`, `signature()`, and
  `ty()`, while `JavaType::descriptor()` and `MethodSignature::descriptor()` return JNI descriptor
  text. These views are valid only while the callback is executing; retain them before storing them
  elsewhere. Safe argument iteration wraps reference lanes as callback-local `JavaLocalObject` /
  `JavaLocalArray` values. Hook
  callbacks no longer accept or return bare `jni::jobject` through safe conversion traits; raw
  argument/original-return access plus raw object extraction and raw object/array returns are
  explicit unsafe APIs.
  Object and array returns are checked against the selected Java return descriptor before returning
  to ART; mismatches are recorded on the guard and cause the Java caller to receive null/default.
  A second active replacement for the same resolved `ArtMethod` is rejected; callers must explicitly
  revert or drop the first guard before replacing the method again. Explicit guard reverts are
  retryable on failure. This explicit guard lifecycle is the intended Rust model rather than a
  temporary substitute for GumJS-style assignment to an `implementation` property. If a live guard is
  dropped and restore fails, replacement clone/thunk memory is intentionally kept mapped instead of
  freeing executable state that ART may still reference; when callback state is still available,
  drop-time restore failures are recorded through the same hook error channel used by callback
  failures. Callback state tracks active invocations; guard teardown waits for other active
  callbacks to drain before restoring and freeing state, or leaks the live replacement state/thunk if
  teardown is attempted from inside the same callback and records that lifecycle error before
  leaking. Use explicit `revert()` when teardown failure must be observed as a `Result`.
  Dedicated test coverage exercises replace/revert/replace lifecycle behavior on the same static
  and instance `ArtMethod` through the public `replace` guard and internal backend helpers.
  Test failures should remain visible when ART instrumentation is incomplete.
  The internal backend and public `replace()` facade use the same descriptor-driven arm64 support
  boundary for arbitrary method and constructor signatures. Constructor replacement has a public
  guarded overload facade with safe callback-local original-constructor initialization, but still
  has no `$alloc` / `$new` allocation ergonomics.

The current live-runtime ART enumeration, deoptimization, and replacement milestone is API 26+ on arm64.
Hardening should keep device-specific failures visible until the underlying ART layout or behavior
is understood and fixed. Replacement hardening uses both the native in-process test harness and the
app-process test harness. The app-process replacement suite forces `Throwable` stack capture while
a replacement quick frame is active, because replacement corruption can make ART abort while
resolving quick frames. The harness prints native failures before throwing so both the native error
and any Java stack remain visible.
