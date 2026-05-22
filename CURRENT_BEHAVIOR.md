# Current Behavior Notes

This crate targets Android ART only. These notes describe the current behavior. They are not
stability contracts: this project is private, pre-user, and exported Rust APIs may change when that
makes the bridge clearer or safer.

The current Rust API keeps VM attachment, JNI descriptors, reference ownership, and class-loader
boundaries explicit instead of cloning the GumJS `Java.use()` surface.

## Runtime And Attachment

- `Java::obtain()` discovers the current Android ART runtime through `JNI_GetCreatedJavaVMs` and
  returns a bootstrap-scoped `Java` handle. Runtime discovery remains internal plumbing; `Vm` is
  exposed as the low-level JNI attachment escape hatch behind `Java::vm()`.
- `Java::android_version()` returns the Android release string and SDK API level read from system
  properties. `Java::android_api_level()` exposes just the parsed SDK integer; ART layout probing
  uses the same API-level reader internally.
- `Java::attach()` returns an `AttachedJava<'_>` scoped view. `Java` remains the shareable VM plus
  optional loader scope; `AttachedJava` additionally guarantees that the current thread has a valid
  `JNIEnv` for the lexical region. `Env`, `AttachedEnv`, local references, and `AttachedJava` are
  thread-affine.
- `Java::perform_now()` attaches the current thread for a synchronous callback and passes
  `AttachedJava` while preserving the receiver's loader scope. It does not queue work, install
  app-loader hooks, or wait for `ActivityThread.currentApplication()`.

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
  supplied `ClassLoaderRef`.
- `Java::app_class_loader()` synchronously resolves the current Android app loader through
  `ActivityThread.currentApplication().getClassLoader()` when an app `Application` is already
  available. `Java::with_app_loader()` publishes that loader as the process default app loader and
  returns a loader-backed handle for it. `Java::default_app_loader()` reports the already-published
  default without querying Android state or installing hooks.
- If `ActivityThread.currentApplication()` is null, app-loader selection returns
  `Error::AppClassLoaderUnavailable`. It does not fall back to enumerated/thread-context loaders.
- `Java::perform()` registers Rust callbacks that run with an app-loader-scoped `AttachedJava`. If
  the app default app loader has already been published, the callback uses it immediately. If
  `ActivityThread.currentApplication()` already exposes an application loader, that loader is
  published and the callback runs synchronously before this method returns. Otherwise the callback is
  queued and process-global Android startup hooks are installed through the internal ART method
  replacement backend. The current hook set drains from
  `LoadedApk.makeApplicationInner`/`makeApplication` and supported
  `ActivityThread.getPackageInfo` overloads when those hook points are present; startup drains
  publish the discovered loader before invoking queued callbacks. Each callback is attached before
  invocation; attachment failure is recorded on the `PerformHandle`. Deferred setup returns
  `UnsupportedFeature` if neither make-application nor get-package-info hook coverage can be
  installed.
  The APK startup-agent test validates the intended early bind-time case: registration from
  `Agent_OnAttach` before `LoadedApk.makeApplication*` has created the real app `Application`.
  Registering from inside already-running app code is still covered by the immediate app-loader
  path, not by this early-start drain guarantee.
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
  `UnsupportedFeature`/error outcomes. `MainThreadTaskHandle` reports `Pending`, `Completed`, or
  `Failed`.
- Capabilities also report main-thread scheduling separately through `main_thread_scheduling`. The
  support probe checks for `epoll_wait`, `Looper.getMainLooper()`, and the `Handler` constructor /
  `sendEmptyMessage(int)` wakeup shape without installing the Gum hook, enqueueing callbacks, or
  sending a looper wakeup. Command-line `app_process` test runs currently report this capability as
  unsupported because `Looper.getMainLooper()` returns null; the APK early-start harness is the live
  validation path for real Android main-looper drain behavior.
- Successful low-level class caches are per `Java` instance. Bootstrap, system-loader,
  DexClassLoader, and enumerated-loader handles do not share cached `JavaClass` values. The
  published default app loader has a dedicated wrapper cache used by bare `Java::use_class()`;
  publishing a different app loader replaces that cache.
- `JavaRef` stores only VM and JNI reference ownership. `JavaObject` stores a `JavaRef` plus the
  wrapper class used for high-level member lookup, so casts and declared object returns can create
  new wrapper views over the same Java value.
- High-level object and class-taking APIs accept sealed `JavaObjectRef` / `JavaClassRef` wrappers
  instead of user-implemented raw `jobject` providers. Raw JNI handles remain available through
  explicit `unsafe raw_*` escape hatches and low-level `Env` APIs. Internal raw extractor traits are
  crate-private, so there is no public safe raw-handle escape hatch.
- `JavaValue::Object` carries `RawJavaObject`, a private-field raw-reference wrapper. Safe
  high-level call arguments come from crate-owned wrappers; arbitrary raw `jobject` values require
  the explicit unsafe `JavaValue::object_raw()` / `RawJavaObject::from_raw_jobject()` lane.

## Wrapper Object Helpers

- `Java::use_class()` returns a Rust-native wrapper. Explicit loader-backed handles use their
  current class-loader scope. A bare bootstrap `Java` handle prefers the published default app
  loader once `Java::with_app_loader()` or `Java::perform()` has initialized it, matching upstream's
  default wrapper behavior without changing `Java::find_class()`.
- Wrapper overload selection remains explicit. Ordinary one-shot calls use
  `JavaClass::call::<T>("name", args)` for static methods and `object.call::<T>("name", args)` for
  instance methods; exact overloads use `call_overload("name", ["TypeA", "TypeB"], args)`.
  `JavaClass::method("name")`, `JavaClass::static_method("name")`, `overload()`, and
  `static_overload()` remain available as selected handles for metadata, reuse, and advanced code.
  Instance method selection includes inherited superclass/interface methods; static method and
  constructor selectors remain declared-only. There is no runtime argument-based overload dispatch
  in the current facade. Specific constructors use `JavaClass::new_overload(["Type"], args)` or a
  reusable `JavaClass::constructor(["Type"])` handle. `JavaClass::new(args)` is a shorthand for
  classes with exactly one declared constructor; classes with no constructors or multiple
  constructors report the same missing/ambiguous selector errors as other name-only wrapper
  selection.
- Wrapper and selected-overload calls accept unit, bare single arguments, tuples, arrays, slices,
  or vectors through `IntoJavaCallArgs`, while still marshaling through explicit `JavaValue` values
  internally. They also accept Rust `&str`, `String`, and `&String` values for
  `java.lang.String`, `java.lang.CharSequence`, and `java.lang.Object` parameters, including inside
  mixed tuples such as `(object, "text", 0)`. Selected calls and wrapper field writes also perform
  conservative descriptor-driven numeric coercion: `int` may narrow to `byte`, `short`, or `char`
  with range checks or widen to `long`, `float` may widen to `double`, and finite in-range `double`
  may narrow to `float`. Temporary Java string references are owned until the JNI call returns;
  low-level `Env` and `java::raw::Class` calls still take explicit `JavaValue` slices.
- Object-returning wrapper calls and fields bind non-null values to the declared return or field
  type using the selected wrapper's loader scope. `call_ref()` and `get_ref_field()` are available
  when callers want the unbound `JavaRef` instead of a wrapper-bound `JavaObject`.
- The default facade uses generic typed receiver operations. On a `JavaClass`, `call` operates on
  static methods and `get_field` / `set_field` operate on static fields. On `JavaObject` and
  `JavaLocalObject`, `call` operates on instance methods and
  `get_field` / `set_field` operate on instance fields. Selected method and field handles use `()`
  as the receiver for static members and an object reference for instance members.
- `JavaClass::replace("name", callback)` and `replace_overload("name", ["Type"], callback)` select
  an unambiguous static or instance method for guarded replacement without requiring an intermediate
  method handle. `unsafe JavaClass::replace_constructor(["Type"], callback)` wraps constructor
  replacement while keeping constructor initialization obligations explicit.
- `JavaObject::class()` returns the selected wrapper class used for member lookup, while
  `runtime_class()` exposes an uncached wrapper for the object's exact runtime class. `JavaRef`
  provides the unbound low-level reference lane and can be promoted through `bind_runtime()` or
  `JavaClass::cast()`.
- `JavaObject` and `JavaArray` are default-global high-level wrappers over crate-owned JNI
  reference storage. Their local counterparts, `JavaLocalObject<'_>` and `JavaLocalArray<'_>`, are
  aliases over the same wrapper APIs with borrowed callback-local storage. `JavaRef` and
  `JavaLocalRef<'_>` are the matching unbound object-reference wrappers.
- `JavaObject::retain()`, `JavaRef::retain()`, `JavaArray::retain()`, `JavaLocalObject::retain()`, and
  `JavaLocalArray::retain()` create owned global references to the same Java value. Callback-local
  borrowed views do not delete references on drop and can be passed to wrapper calls and field
  helpers while the producing callback/JNI frame is alive.
- `refs::LocalRef<'env, K>` is the lower-level owning JNI local-reference wrapper used by `Env`
  APIs. It deletes its local reference on drop and is intentionally separate from callback-local
  borrowed views.
- `JavaObject::java_to_string()` and `JavaLocalObject::java_to_string()` call Java
  `Object.toString()` for diagnostics. `get_string()` remains the direct helper for known
  `java.lang.String` values.
- `java::raw::Class::is_instance()`, `JavaClass::is_instance()`, and `JavaClass::cast()` validate
  runtime object type with JNI `IsInstanceOf`.
- `JavaClass::cast()` returns a retained `JavaObject` bound to that class after validation. It
  creates a new wrapper view over the same Java value; `JavaObject::cast()` is the receiver-side
  spelling for the same operation.

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
- `Java::capabilities()` reports the same support decisions used by the current enumeration APIs,
  including app-loader deferral and main-thread scheduling.
- Heap enumeration is supported when ART heap visitor prerequisites are available.
  `Java::choose_instances()` and `JavaClass::choose_instances()` enumerate live instances
  whose runtime class exactly matches the resolved class; callbacks return
  `JavaChooseControl::Continue` or `JavaChooseControl::Stop`, and objects must be retained inside
  the callback if they should outlive it. Unsupported ART layouts or missing heap symbols return
  `Error::UnsupportedFeature`.
- Deoptimization is intentionally reported as unsupported until its ART path is implemented.
  Method replacement is reported as supported when current ART prerequisites are available, and
  unsupported when a prerequisite is missing. Method
  replacement probes may report that ART prerequisites, cloned `ArtMethod` preparation, and
  safe-patching guardrails are available for the descriptor-driven closure-backed replacement path.
  The internal path uses cloned-method dispatch and has thread-scoped, stack-aware raw original
  invocation for static, instance, and constructor callbacks, including object arrays and null JNI
  values.
  The intended ergonomic path is class-level direct replacement, for example:
  `let activity = java.use_class("android.app.Activity")?;`,
  and `let guard = activity.replace("onResume", |ctx| { ctx.call_original_void(())?; Ok(()) })?;`.
  Original calls may be made from public `replace` callbacks through
  `JavaHookContext::call_original()` with `IntoJavaArgs` containers, including bare single
  `JavaValue`-convertible arguments. Simple pass-through hooks can use
  `JavaHookContext::call_original_current()` to invoke the original implementation with the current
  callback arguments, or `JavaHookContext::call_original_return(args)` to get the original result
  as an explicit `JavaHookReturn`. Selected `JavaMethod` values expose safe `replace()` as the public
  replacement entrypoint; `JavaConstructor::replace()` remains unsafe because constructor
  callbacks must uphold receiver-initialization semantics. Replacement uses public
  callback/return/guard types under `replacement::*`; it returns an explicit `JavaHookGuard`,
  receives `JavaHookContext`, and returns `JavaHookReturn` with iterable safe argument views,
  typed argument helpers, and borrowed object/array return helpers. Public admission uses the
  descriptor-driven arm64 closure layout path for arbitrary
  descriptors that fit the current hook limits, including mixed primitive/reference
  arguments, arrays, and stack-passed arguments. Constructor callbacks are exposed as `<init>` /
  `MethodKind::Constructor`, receive the allocated receiver, must return void, and
  `call_original*()` invokes the selected original constructor on that receiver and returns void.
  Unsupported facade signatures fail before installation with errors naming the method kind, method
  name, and a concise reason.
  Raw closure callbacks, captured original-method handles, and backend replacement admission remain
  crate-internal scaffolding for the public facade, app startup hooks, and backend coverage. Callback
  errors, panics, or wrong return kinds are stored on the guard and return the JNI default value for
  the Java method's return type.
  Replacement callbacks expose borrowed local helpers through
  `JavaHookContext::{arguments,arg_value,arg_display,this_object,arg_object,arg_array}` and
  original-call helpers for object and array returns. `JavaHookContext::arg()` and
  `call_original()` support `String` and `Option<String>` conversions for Java string lanes, and
  `arg()` also supports `JavaLocalObject`, `Option<JavaLocalObject>`, `JavaLocalArray`, and
  `Option<JavaLocalArray>` for descriptor-matching object and array parameters. Callback-local
  object/array wrappers borrow from the invocation lifetime, so returning those values from a
  callback still goes through explicit `JavaHookReturn::object(...)` /
  `JavaHookReturn::array(...)` wrappers.
  `JavaObject`, `JavaLocalObject`, `JavaArray`, `JavaLocalArray`, `JavaReturn`, and
  `JavaHookArgument` expose `java_display()` for diagnostic text. Primitive, null, and void values
  are formatted directly; reference values use Java's `Object.toString()` behavior, so arrays
  intentionally display as Java array references such as `[I@...` rather than expanded contents.
  `arg_display()` is the hook-context single-argument convenience wrapper over the same display
  behavior. `JavaClass`, `JavaConstructor`, `JavaMethod`, and `JavaField` expose infallible
  metadata summaries through `java_display()`. These views are valid only while
  the callback is executing; retain them before storing them elsewhere. Safe argument iteration
  wraps reference lanes as callback-local `JavaLocalObject` / `JavaLocalArray` values. Hook callbacks no longer
  accept or return bare `jni::jobject` through safe conversion traits or public
  `JavaHookReturn` variants; wrapper returns are the safe path, and raw argument/original-return
  access plus raw object returns are explicit unsafe escape hatches.
  Object and array returns are checked against the selected Java return descriptor before returning
  to ART; mismatches are recorded on the guard and cause the Java caller to receive null/default.
  A second active replacement for the same resolved `ArtMethod` is rejected; callers must explicitly
  revert or drop the first guard before replacing the method again. Explicit guard reverts are
  retryable on failure. This explicit guard lifecycle is the intended Rust model rather than a
  temporary substitute for GumJS-style assignment to an `implementation` property. If a live guard is
  dropped and restore fails, replacement clone/thunk memory is intentionally kept mapped instead of
  freeing executable state that ART may still reference. Callback state tracks active invocations;
  guard teardown waits for other active callbacks to drain before restoring and freeing state, or
  leaks the live replacement state/thunk if teardown is attempted from inside the same callback.
  Dedicated test coverage exercises replace/revert/replace lifecycle behavior on the same static
  and instance `ArtMethod` through the public `replace` guard and internal closure-backed helpers.
  Test failures should remain visible when ART instrumentation is incomplete.
  The internal raw closure backend and public `replace()` facade use the same
  descriptor-driven arm64 trampoline boundary for arbitrary method and constructor signatures,
  including mixed primitive/reference arguments and stack-passed arguments. Constructor replacement
  has a public guarded overload facade and callback-local original-constructor calls, but still has
  no `$alloc` / `$new` allocation ergonomics.

The current live-runtime ART enumeration and replacement milestone is API 26+ on arm64.
Hardening should keep device-specific failures visible until the underlying ART layout or behavior
is understood and fixed. Replacement hardening uses both the native in-process test harness and the
app-process test harness. When replacement corrupts ART's view of a method, even ordinary Java
exception stack capture may abort while resolving quick frames; the app-process harness prints the
native failure before throwing so both the native error and any Java stack remain visible.
