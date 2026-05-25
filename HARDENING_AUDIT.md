# Hardening Audit

This file is the discovery notebook and implementation tracker for the hardening pass.

Hardening means finding places where the code can be wrong, unsound, misleadingly safe, racy,
version-fragile, or too trusting of ART/JNI behavior. It is broader than Rust `unsafe`: lifetime
shape, thread ownership, exception state, loader identity, callback failure, and runtime capability
reporting all count.

Hardening discovery is complete. The remaining hardening work is implementation, plus focused
re-reading of touched code as each fix lands.

## Process

Use two phases.

### Phase 1: Discovery And Documentation

Read module families with a safety and correctness lens. Record findings before changing code.
Include the expected failure mode, the caller-visible consequence, and the boundary that should own
the guarantee.

Start with the lightweight inventory captured during cleanup, then re-read areas that cleanup
patches touched. Existing findings are seed inventory only: each audit area still needs focused
discovery before implementation, and every existing finding should be revalidated, refined, or
closed during its focused pass.

Classify each finding as one of:

- Unsafe boundary: safe API relies on a guarantee the type system does not express.
- Lifetime: reference, attachment, callback, guard, or borrowed value may outlive its valid scope.
- Threading: thread-affine JNI or ART state may cross threads, race, or assume the wrong looper.
- Exception state: JNI exceptions may be ignored, overwritten, or converted inconsistently.
- Raw handle: raw JNI value can be forged, reused, or mis-owned in a safe path.
- Runtime matrix: Android version, ABI, ART symbol, or layout assumptions are too broad.
- Callback failure: panic, error, or wrong return path can leak state or leave a hook half-active.
- Test gap: behavior is risky and not covered by host, app-process, APK, or bootstrap tests.
- Other: anything that doesn't fit into the above categories.

### Phase 2: Hardening Implementation

Fix one bounded risk at a time. Prefer narrow type/API changes over comments that merely explain a
hazard. If a risk cannot be removed, move it to an explicit `unsafe` boundary and document the
caller contract.

For `Test gap` findings, choose the narrowest harness that can observe the behavior: host or unit
tests for parser, descriptor, and formatting logic; app-process tests for ordinary live-runtime
behavior; APK tests for early startup and real main-looper behavior; and `art_test` only for native
ART bootstrap and manual VM startup assumptions.

After each sprint, update findings with one of:

- Fixed: code changed and verification noted.
- Unsupported: behavior is intentionally unavailable, with the user-facing reason documented.
- Unsafe by design: the risk is caller-owned and the unsafe contract says so.
- Deferred: still risky, with the reason it cannot be fixed in this sprint.

## Hardening Rules

- Safe public APIs must not require callers to know JNI local-reference, attachment, or ART mutation
  rules unless the type system enforces those rules.
- Raw `jobject`, `jclass`, `jmethodID`, `JNIEnv`, ART pointers, and cloned method state should stay
  behind crate-private or explicit `unsafe` APIs.
- Every ART layout, symbol, ABI, and Android-version assumption should fail closed with a structured
  unsupported reason.
- JNI exception state should be checked and cleared only according to a single local rule for that
  call path.
- Callback panics and errors should not cross FFI boundaries.
- Guards should own lifecycle clearly: install, active use, error observation, revert, and drop.
- Thread-affine values should be visibly non-`Send`/non-`Sync` unless a type proves otherwise.
- Hardening should not make unsupported behavior look supported through best-effort fallbacks.
- "Questions" lists are just examples. Make your own checklists and use your best judgement to find any other issues.

## Audit Checklist

### Lifetimes And Reference Ownership

Focused discovery status: first focused pass completed for `LocalRef`, `BorrowedLocalRef`,
`GlobalRef`, high-level object/array wrappers, wrapper argument conversion, callback-local
reference views, and prepared JNI call argument cleanup. Global-reference drop failure visibility
and safe reference-lifetime erasure findings are pending implementation.

Look at `src/refs.rs`, `src/env/references.rs`, `src/java/object.rs`, `src/java/array.rs`,
`src/replacement/api.rs`, and callback-local reference views.

Questions:

- Can a local reference escape its attached scope?
- Can a borrowed hook argument be stored past callback return?
- Are global references clearly owned and released?
- Are null references represented distinctly from non-null objects where behavior requires it?
- Are casts and declared object returns binding references to the right loader/class context?

Findings:

Focused discovery notes:

- `LocalRef<'env, K>` owns JNI locals and deletes them through the originating `JNIEnv` on drop;
  its `Rc` marker keeps it non-`Send`/non-`Sync`. Safe constructors tie the local to an `Env`
  borrow, while `into_raw()` is explicitly unsafe and transfers deletion responsibility to the
  caller.
- `BorrowedLocalRef<'local, K>` is the callback/JNI-frame view used by `JavaLocalRef`,
  `JavaLocalObject`, and `JavaLocalArray`. It never deletes on drop, is non-`Send`/non-`Sync`, and
  is the right shape for callback `this`, argument, and original-return views as long as safe
  conversions do not copy it into lifetime-free raw containers.
- `GlobalRef<K>` is deliberately `Send + Sync` because JNI global references are VM-scoped. Its
  current `Drop` remains best-effort: deletion is attempted after attaching the dropping thread, and
  attachment failure silently leaks the global reference. The earlier cleanup-pass finding is still
  valid.
- High-level object and array returns from ordinary wrapper calls promote JNI local results to
  globals before returning `JavaObject`, `JavaRef`, or `JavaArray`, so local result references do not
  escape those call frames. Local result wrappers remain visible only in callback-local return paths.
- `PreparedJavaCallArgs` owns temporary Rust-string `jstring` locals for wrapper calls and deletes
  them after the JNI call, but cleanup is only installed once the whole argument list prepares
  successfully.
- Name-dispatched wrapper calls use reference dispatch scoring that checks object arguments against
  expected classes with `IsInstanceOf`. Exact selected overload calls and field writes only check
  that a `JavaValue` is reference-shaped, not that the object is assignable to the selected formal
  type.

### Finding: callback-local raw returns can escape without a lifetime

- Status: Unsafe by design
- Area: `src/replacement/api.rs`, `src/java/returns.rs`, `src/value.rs`
- Kind: Lifetime | Raw handle
- Failure mode: Safe helpers such as `JavaHookContext::call_original_current()` and
  `JavaHookContext::call_original_return()` previously returned `JavaHookReturn`, whose object and
  array lanes carry `RawJavaObject` without a Rust lifetime tying the reference to the active
  replacement callback.
- User-visible consequence: A caller could store a raw object/array return after the callback-local
  JNI reference was no longer valid, then later feed it back through unsafe or raw-return APIs and
  observe use-after-lifetime behavior at the JNI/ART boundary.
- Hardening: Safe original-call helpers now extract through typed `FromJavaHookReturn`, so object
  and array returns become callback-local `JavaLocalObject` / `JavaLocalArray` views. The remaining
  raw original-call result path is `unsafe JavaHookContext::call_original_raw()`, whose caller
  contract states that object references are valid only while the replacement callback is executing.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`.
- Links: `CLEANUP_AUDIT.md` finding "raw hook return alias is a public user concept".

### Finding: prepared string argument locals can leak when later argument preparation fails

- Status: Fixed
- Area: `src/java/args.rs`
- Kind: Lifetime | Exception state
- Failure mode: `PreparedJavaCallArgs` stores raw `jstring` locals created for Rust string
  arguments and deletes them from `AttachedJavaCallArgs::drop()` after a successful whole-argument
  preparation. If a tuple, array, slice, or vector argument list creates one string local and a
  later argument fails type coercion or string creation, the partially built `PreparedJavaCallArgs`
  is dropped without deleting the locals it already accumulated.
- User-visible consequence: Repeated failed wrapper calls with an early Rust string argument can
  leak JNI local references on the attached thread. In error-heavy dispatch or validation paths this
  can exhaust the local reference table and make later Java work fail for an unrelated reason.
- Hardening: `PreparedJavaCallArgs` now borrows the preparing `Env` and deletes any accumulated
  temporary locals in `Drop`; successful call preparation explicitly transfers the values and local
  refs into `AttachedJavaCallArgs`, so normal call paths still delete exactly once after the JNI call.
- Verification: `cargo ndk -t arm64-v8a check --all-features`; `cargo ndk -t arm64-v8a clippy
  --all-features`; `just test all` on Quest 2 / Android 14, including app-process coverage that
  repeatedly prepares a Rust string argument followed by a bad trailing typed argument and then
  performs another valid string call on the same wrapper.

### Finding: safe Java argument containers erase local-reference lifetimes

- Status: Discovered
- Area: `src/java/args.rs`, `src/value.rs`, `src/replacement/api.rs`
- Kind: Lifetime | Raw handle
- Failure mode: Safe conversions from `&JavaObject<R>`, `&JavaRef<R>`, `&JavaArray<R>`, and the
  `java_args!` / `JavaArgs::push()` path copy object handles into `JavaValue`, which is `Copy` and
  has no lifetime. When `R` is a callback-local `BorrowedLocalRef`, callers can store a `JavaArgs`
  or `JavaValue` after the callback/JNI frame that produced the local reference has ended.
- User-visible consequence: A stale local JNI reference can later be passed through a safe-looking
  explicit argument list into a wrapper call, original-call helper, or raw value path. The actual
  failure would occur at JNI/ART time as a crash, wrong object access, or misleading Java exception
  instead of a Rust lifetime error.
- Proposed hardening: Split lifetime-free owned/global argument storage from callback-local
  argument views, or make local-reference-to-`JavaValue` conversions explicit unsafe/raw operations.
  Keep immediate wrapper calls ergonomic, but require stored `JavaArgs` to contain only primitives,
  nulls, raw unsafe handles, Rust strings, or retained/global Java references.
- Verification: Compile-fail or focused unit coverage proving a `JavaLocalObject` cannot be placed
  into a storable `JavaArgs` without `retain()` or an unsafe/raw API; app-process smoke coverage for
  immediate local argument forwarding after any API split.

### Finding: exact wrapper calls do not validate reference argument assignability

- Status: Fixed
- Area: `src/java/args.rs`, `src/java/wrapper.rs`, `src/java/dispatch.rs`
- Kind: Raw handle | Test gap
- Failure mode: `coerce_java_value()` accepts any non-null object handle for any object or array
  formal type because `JavaValue::matches_type()` only checks that the expected type is a
  reference. Name-dispatched calls run `reference_dispatch_score()` and use `IsInstanceOf` during
  overload selection, but exact overload calls such as `JavaMethod::call()` with a selected
  signature, `JavaConstructor::new_object()`, and `JavaField::set()` can pass an incompatible object
  reference directly to JNI from safe code.
- User-visible consequence: A caller can pass, for example, a `java.lang.Integer` where a selected
  overload expects `java.lang.String`. ART/JNI receives the mismatched reference and may raise a
  Java exception, corrupt the callee's type assumptions, or fail in a VM-specific way rather than
  returning a Rust `InvalidArgumentType` that names the bad argument.
- Hardening: Exact selected methods, constructors, and field writes now validate non-null reference
  arguments with `IsInstanceOf` against the selected formal type before JNI. The expected reference
  class is resolved through the selected wrapper's loader scope; `JavaValue` remains the low-level
  carrier for `Env` and `java::raw::Class` APIs.
- Verification: `cargo ndk -t arm64-v8a check --features app-process-test --lib`;
  `cargo ndk -t arm64-v8a clippy --all-features`; `just test all` on Quest 2 / Android 14,
  OnePlus device / Android 16, and Mi Max / Android 10, including wrong object argument coverage
  for exact selected method call, constructor call, and field set.

### Hidden Unsafety

Focused discovery status: selected receiver-boundary sprint completed. A raw JNI/member ID boundary
pass completed for `MethodId`, `FieldId`, reflected-member conversion, metadata ID exposure,
`Env` call/field helpers, and `java::raw::Class` member caching. A raw reference/value boundary pass
completed for top-level raw module exposure, `RawJavaObject`, `JavaValue`, `JavaArgs`,
object/array/reference conversions, `JavaHookReturn`, callback argument views, and public raw
extractors.

Look at all `unsafe` blocks and any safe functions that call raw JNI/ART helpers.

Questions:

- Does the public boundary expose the required caller guarantee?
- Is each unsafe block close to the invariant that justifies it?
- Are raw handles accepted only from crate-owned wrappers unless the API is unsafe?
- Are architecture assumptions checked before use?

Findings:

Focused discovery notes:

- `MethodId` and `FieldId` are public Android-gated low-level wrappers around `jmethodID` and
  `jfieldID`. They carry kind plus signature or field type, and their raw extractors are `unsafe`,
  but they do not carry the declaring class, class loader, or VM identity that produced the ID.
- `java::raw::Class` caches method and field IDs per class handle, so its own descriptor-based
  helpers normally resolve and consume IDs through the same class. The class handle documentation
  already warns that cached IDs are tied to that class identity. The unsafety appears when public
  `Env` helpers accept a detached `MethodId`/`FieldId` plus an arbitrary object or class supplied by
  the caller.
- `Env::new_object`, instance/static method calls, field gets/sets, and
  `Env::to_reflected_method()` / `Env::to_reflected_field()` validate kind, return type, field
  type, and argument count/value shape, but they do not validate that the supplied class or receiver
  matches the ID owner. For object arguments and object field values, low-level validation still
  only checks reference-shaped/null versus primitive-shaped values.
- High-level selected `JavaMethod` and `JavaField` handles re-resolve through their owning
  `raw::Class` before invoking JNI instead of handing their stored metadata ID directly to `Env`,
  so selected-wrapper calls currently share the receiver and assignability risks documented in the
  Java facade findings rather than depending on the public metadata ID for dispatch.
- Reflection-backed metadata currently stores public raw `jmethodID` / `jfieldID` values. These IDs
  are used internally for ART deoptimization and sorting, but external callers can copy them out
  without an owner class boundary. ART heap enumeration can also synthesize method metadata from an
  `ArtMethod` pointer without a JNI declaring-class token.
- `Env::from_reflected_method()` and `Env::from_reflected_field()` are safe public functions that
  accept caller-supplied kind/signature/type metadata and wrap the raw JNI ID returned by
  `FromReflectedMethod` / `FromReflectedField`. The current API trusts the caller to describe the
  reflected member accurately, then later safe call paths trust the resulting ID wrapper for
  argument and return validation.
- The normal high-level object boundary is better than the raw module layout suggests: external
  callers cannot implement `JavaObjectRef` or `JavaClassRef`, local-reference wrappers are
  non-`Send`/non-`Sync`, global-reference wrappers are the only safe cross-thread reference storage,
  and raw constructors/extractors on `Vm`, `Env`, refs, `RawJavaObject`, and `JavaValue::object_raw`
  are already `unsafe` with caller contracts.
- The remaining raw-reference hazard is the lifetime-free value carrier. `JavaValue` is public,
  `Copy`, and top-level re-exported, while its object lane stores `RawJavaObject`. Safe `From`
  conversions from `&LocalRef`, `&BorrowedLocalRef`, `&JavaRef`, `&JavaObject`, and `&JavaArray`
  copy the raw JNI handle into `JavaValue` without carrying the source lifetime. `JavaArgs`,
  `Vec<JavaValue>`, slices, tuples, `java_args!`, raw `Class` calls, low-level `Env` calls, and
  replacement original-call helpers can then store, clone, or replay those values after the local
  reference's JNI frame has ended.
- `JavaHookContext::args()`, `arg_object()`, `arg_array()`, and typed `arg<T>()` are the safer
  callback argument view: object and array lanes become `JavaLocalObject<'state>` /
  `JavaLocalArray<'state>` or typed primitives. The explicitly raw callback accessors
  `raw_arguments()`, `raw_arg_object()`, and `call_original_raw()` are already `unsafe`. The same
  lifetime-free raw return problem remains in safe `JavaHookReturn` constructors and
  `proceed()`, as documented in the replacement lifecycle findings.
- Top-level module exposure still mixes audiences. `lib.rs` re-exports only `JavaValue` from the
  raw value layer, but `pub mod jni`, `pub mod value`, and Android-gated `pub mod env` / `refs`
  make raw handles, `RawJavaObject`, and `Env` APIs discoverable beside the high-level `Java`
  facade. That is acceptable only if the documentation and module names make the raw layer an
  explicit advanced boundary rather than the first visible path for ordinary Java calls.

### Finding: raw JNI/reference surface needs one explicit public boundary

- Status: Discovered; revalidated during raw reference/value boundary sprint
- Area: `src/lib.rs`, `src/jni.rs`, `src/refs.rs`, `src/env/`, `src/vm.rs`, `src/value.rs`
- Kind: Unsafe boundary | Raw handle
- Failure mode: Raw JNI definitions and low-level reference/value types are publicly reachable
  beside high-level Java APIs. Most raw constructors and extractors are marked `unsafe`, but the
  crate does not yet present one cohesive public boundary that tells callers which raw handles may
  be forged, borrowed, retained, or moved across threads.
- User-visible consequence: Advanced callers may combine raw values from the wrong VM, thread,
  callback, or local-reference scope and only discover the mistake as a JNI/ART crash or corrupted
  exception state.
- Proposed hardening: During hardening, group raw JNI/reference APIs under an explicitly advanced
  or unsafe public surface and audit every raw-handle constructor/extractor for a precise caller
  contract. Keep normal Java object work on safe wrapper APIs.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; documentation review for every
  remaining public `unsafe fn` in the raw layer.
- Links: `CLEANUP_AUDIT.md` finding "top-level exports mix normal Java work with raw internals";
  `DOCUMENTATION_PASS.md` low-level JNI docs.

### Finding: raw Java values are lifetime-free but accepted by safe call surfaces

- Status: Discovered; revalidated during raw reference/value boundary sprint
- Area: `src/value.rs`, `src/refs.rs`, `src/java/args.rs`, `src/java/class.rs`,
  `src/java/dispatch.rs`, `src/replacement/api.rs`
- Kind: Unsafe boundary | Lifetime | Raw handle
- Failure mode: `JavaValue` is a lifetime-free `Copy` value whose object lane carries
  `RawJavaObject`. Safe conversions from owned/global and borrowed/local Java wrappers all create
  that same object lane. Safe containers and call adapters then accept `JavaValue` through
  `JavaArgs`, `Vec<JavaValue>`, slices, tuples, raw `Class` calls, selected wrapper calls, field
  writes, and replacement original-call helpers. When the source was a callback-local or
  attach-local reference, the type system no longer prevents the copied handle from being stored or
  used after its local JNI frame ended.
- User-visible consequence: A user can construct a storable argument list or hook return from a
  safe borrowed Java wrapper, keep it past the valid JNI frame, and later pass a stale raw reference
  through a safe-looking call path. ART/JNI then sees a dangling local reference, wrong-thread
  reference, or wrong-VM reference instead of Rust reporting a lifetime or ownership error.
- Proposed hardening: Split normal argument/return storage from raw JNI value plumbing. Stored safe
  arguments should contain primitives, nulls, Rust strings, or retained/global Java references; local
  object/array views should either be consumed immediately through lifetime-bound call adapters or
  require `retain()` before entering a storable container. Keep lifetime-free raw `JavaValue`
  available only under an explicit unsafe/raw boundary, or introduce a lifetime-parameterized value
  view for callback-local forwarding.
- Verification: Compile-fail or focused compile coverage proving `JavaLocalObject` /
  `JavaLocalArray` cannot enter a storable `JavaArgs` or `JavaHookReturn` without `retain()` or an
  unsafe/raw API; app-process smoke coverage for immediate local argument forwarding and hook
  object/array returns after the API split; `cargo ndk -t arm64-v8a clippy --all-features`.
- Links: `HARDENING_AUDIT.md` findings "safe Java argument containers erase local-reference
  lifetimes", "safe proceed returns raw callback-local references", and "safe object and array
  hook-return conversions erase local-reference lifetimes".

### Finding: method and field IDs are not bound to their declaring class

- Status: Discovered; revalidated during raw JNI/member ID boundary sprint
- Area: `src/env/ids.rs`, `src/env/members.rs`, `src/env/calls.rs`, `src/env/fields.rs`,
  `src/metadata.rs`
- Kind: Unsafe boundary | Raw handle
- Failure mode: `MethodId`, `FieldId`, and public metadata IDs carry kind and descriptor/type
  information, but not the class or VM identity that produced the ID. Safe `Env` call/field helpers
  accept an object or class separately from the ID, so a caller can accidentally combine an ID with
  the wrong receiver/class in a safe call path. `Env::to_reflected_method()` and
  `Env::to_reflected_field()` have the same detached class-plus-ID shape. After the method/field
  facade rework, `JavaMethodMetadata::id` and `JavaFieldMetadata::id` also remain public raw
  `jmethodID` / `jfieldID` values, so callers can copy opaque IDs out of reflection metadata
  without any declaring-class or VM boundary.
- User-visible consequence: JNI receives a mismatched `jmethodID`/`jfieldID` and receiver/class
  pair. Depending on ART behavior, this can surface as a Java exception, wrong member access, or a
  VM-level crash rather than a Rust error naming the misuse.
- Proposed hardening: Bind selected low-level IDs to their declaring class/VM in the safe API, or
  make the detached-ID call helpers explicitly unsafe with a contract that the receiver/class must
  match the ID owner. Make metadata IDs private, or expose them only through explicit unsafe/raw
  accessors with a caller contract. Keep high-level `JavaMethod`/`JavaField` selected handles as
  the normal safe path.
- Verification: Host compile/unit coverage for any new typed ID shape; app-process smoke coverage
  for method and field calls after changing the Env surface.
- Links: `CLEANUP_AUDIT.md` low-level JNI surface findings; `DOCUMENTATION_PASS.md` low-level JNI
  docs.

### Finding: reflected member ID constructors trust caller-supplied metadata

- Status: Unsafe by design
- Area: `src/env/members.rs`, `src/metadata/reflection.rs`
- Kind: Unsafe boundary | Raw handle
- Failure mode: `Env::from_reflected_method()` and `Env::from_reflected_field()` are safe public
  functions, but the caller supplies the expected `MethodKind`, `MethodSignature`, `FieldKind`, or
  `JavaType`. The functions only check the JNI exception/null outcome from
  `FromReflectedMethod` / `FromReflectedField`; they do not verify the supplied metadata against the
  reflected `Method`, `Constructor`, or `Field` object before producing a safe `MethodId` or
  `FieldId`.
- User-visible consequence: A caller can create a safe low-level ID wrapper with the wrong argument
  signature, return type, field type, or static/instance kind. Later safe `Env` call/field helpers
  validate against the forged wrapper metadata, then pass a mismatched ID, class/object, and JNI
  argument frame to ART.
- Hardening: `Env::from_reflected_method()` and `Env::from_reflected_field()` are now explicit
  `unsafe` low-level APIs. Their caller contracts require the reflected member object and supplied
  kind/signature/type metadata to match. The high-level reflection metadata path remains safe by
  deriving kind/signature/type from Java reflection immediately before calling the unsafe wrappers.
- Verification: `cargo ndk -t arm64-v8a check --all-features`;
  `cargo ndk -t arm64-v8a clippy --all-features`; `just test all` on Quest 2 / Android 14,
  OnePlus device / Android 16, and Mi Max / Android 10, including the app-process metadata
  reflection checks.
- Links: `HARDENING_AUDIT.md` finding "method and field IDs are not bound to their declaring
  class".

### Finding: selected method and field handles accept unchecked receivers

- Status: Fixed
- Area: `src/java/wrapper.rs`
- Kind: Unsafe boundary | Raw handle
- Failure mode: Safe selected `JavaMethod` and `JavaField` handles carry the wrapper class and
  reflected member metadata, but detached calls such as `JavaMethod::call(&object, args)` and
  `JavaField::get(&object)` accept any crate-owned `JavaObjectRef` receiver. The receiver is passed
  to raw JNI member access without first checking that it is an instance of the selected handle's
  class.
- User-visible consequence: A caller can accidentally combine a selected member from one class or
  loader with an object from another class or loader in a safe API. ART/JNI may raise a Java
  exception, access the wrong slot, or crash instead of returning a Rust error that names the
  receiver mismatch.
- Focused discovery notes:
  `JavaClass::bind()` is the already-safe receiver entrypoint because it checks
  `JavaClass::is_instance()` before constructing `JavaBoundObject`. `JavaObject::method()`,
  `JavaObject::call*()`, `JavaObject::field()`, and `JavaBoundObject` operations stay within that
  checked bound-object path. The risky detached paths are `JavaMethod::call_raw()`,
  `JavaMethod::call()`, the typed `JavaMethod::call_*()` helpers, `JavaField::get_raw()`,
  `JavaField::get()`, the typed `JavaField::get_*()` helpers, and `JavaField::set()` /
  typed `set_*()` helpers when called with an arbitrary object receiver. `JavaBoundMethodOverload`
  and `JavaBoundFieldHandle` are safe only because their current constructors come from a checked
  `JavaClass::bind()` path; they delegate back through the same detached selected-handle APIs.
  `java::raw::Class::call_method()`, `get_field()`, and `set_field()` resolve the member on the
  selected class and then forward the supplied object plus detached `MethodId`/`FieldId` to
  `Env`. The low-level `Env` helpers validate method/field kind, return/type, and arguments, but
  do not validate that the receiver is an instance of the ID's declaring class.
- Hardening: Safe selected instance method calls, field reads, and field writes now validate the
  supplied receiver with `IsInstanceOf` against the selected wrapper class before preparing call
  arguments or field values. Mismatches return `Error::InvalidObjectType`; low-level `Env` and
  `java::raw::Class` member helpers remain the explicit raw/low-level boundary.
- Verification: `cargo ndk -t arm64-v8a check --features app-process-test --lib`;
  `cargo ndk -t arm64-v8a clippy --all-features`; `just test all` on Quest 2 / Android 14,
  OnePlus device / Android 16, and Mi Max / Android 10, including wrong-receiver selected method,
  field get, and field set coverage.
- Links: `CLEANUP_AUDIT.md` Java facade findings.

### Loader Scope And App-Loader Publication

Focused discovery status: first focused pass completed for `Java` loader scope, class lookup/cache
isolation, `ClassLoaderRef` validation, default app-loader publication, deferred startup hook drains,
and app-process/APK loader coverage. Early startup loader identity and custom loader result
validation findings are pending implementation.

Look at `src/java/handle.rs`, `src/java/loader.rs`, `src/java/lookup.rs`,
`src/java/perform.rs`, app-process loader checks, and the APK early-start harness.

Questions:

- Can a bare `use_class()` start using the wrong app loader after deferral?
- Are explicit loader-backed class caches isolated by actual loader identity?
- Does a custom class loader get to lie about the class returned for a requested name?
- Are startup hook drains one-shot and tied to the real app startup path?
- Do behavior notes distinguish loader provenance from stable identity?

Findings:

Focused discovery notes:

- Plain `Java::find_class()` stays bootstrap-scoped, while `Java::use_class()` on a bare handle
  delegates to the published default app-loader `Java` only after `Java::with_app_loader()` or
  `Java::perform()` has published one. Explicit `Java::with_loader()` handles keep their own class
  cache, so bootstrap, app, DexClassLoader, and enumerated loader lookups do not intentionally share
  cached `raw::Class` values.
- The published default app loader owns a dedicated wrapper cache. `publish_app_loader()` uses
  `IsSameObject` to avoid clearing that cache when the same loader is republished, and replaces both
  loader and cache when a different loader object is published. Existing loader-scoped `Java`
  handles remain scoped to the loader they were created with.
- `ClassLoaderRef` construction validates that the wrapped object is a `java.lang.ClassLoader`.
  `ClassLoaderKind` remains provenance only; it is not a stable identity key and does not prove two
  loader references name the same Java loader.
- Loader-backed lookup uses `ClassLoader.loadClass(String)` for ordinary classes and
  `Class.forName(name, false, loader)` for array descriptors. The app-process harness covers
  immediate app-loader selection, bare-wrapper lookup after default publication, explicit
  DexClassLoader lookup, and enumerated loader smoke checks. The APK harness covers the intended
  early-start case where a deferred `perform()` callback runs with the app loader and bare
  `use_class()` observes the published default.
- Deferred startup hooks call the original Android method first, then publish and drain from the
  returned `Application` or `LoadedApk` class loader. Unlike upstream's one-shot early/late
  selection, the current Rust hook pair can keep publishing from every non-null supported
  `LoadedApk.getPackageInfo()` or `LoadedApk.makeApplication*` result for process lifetime.

### Finding: deferred `getPackageInfo` drain can publish an unproven app loader

- Status: Discovered
- Area: `src/java/perform.rs`, `src/apk_perform_test.rs`
- Kind: Runtime matrix | Test gap
- Failure mode: The deferred app-loader hook installed on `ActivityThread.getPackageInfo()` drains
  pending `Java::perform()` callbacks from any non-null returned `LoadedApk`, publishes that
  object's `getClassLoader()` result as the default app loader, and leaves both startup hooks active.
  There is no one-shot `initialized` guard, no early-versus-late bind-state distinction, and no
  check that the `LoadedApk` belongs to the process' eventual `Application`.
- User-visible consequence: During complicated startup, instrumentation, split/resource package
  resolution, or an unexpected Android framework call order, a queued or newly registered
  `Java::perform()` callback could run under a loader that is merely available early rather than the
  real app loader. A later `makeApplication` drain may replace the default cache, but the callbacks
  already run under the first published loader keep whatever hooks, classes, or objects they created.
- Proposed hardening: Mirror the upstream lifecycle shape more closely: choose a one-shot early
  drain path only before bind-time instrumentation forces a late `makeApplication` path, mark the
  app-loader drain initialized after the first accepted publication, and stop treating arbitrary
  later `getPackageInfo()` results as default-loader evidence. If early `LoadedApk` publication
  remains supported, validate enough package/application context to explain why that loader is the
  app loader and keep unsupported startup shapes visible.
- Verification: Add APK startup coverage that simulates or observes an unrelated
  `getPackageInfo()` result before `Application` creation, or factor the drain-selection state into
  a host-testable unit and cover early, late, duplicate, and replacement cases. Re-run
  `just apk-perform-test all` for implementation changes.
- Links: `ROADMAP.md` app-loader deferral priority; `CURRENT_BEHAVIOR.md` class-loader scope notes;
  upstream `../frida-java-bridge/index.js` pending VM op initialization flow.

### Finding: loader-backed class lookup trusts custom loader results

- Status: Fixed
- Area: `src/java/lookup.rs`, `src/java/handle.rs`
- Kind: Raw handle | Test gap
- Failure mode: Safe `Java::with_loader(...).find_class()` calls a caller-supplied
  `ClassLoader.loadClass(String)` and caches the returned `java.lang.Class` under the normalized
  requested name without checking that `Class.getName()` matches the requested binary name or array
  descriptor. `ClassLoaderRef` validates the receiver type, but not the behavioral contract of a
  custom loader implementation.
- User-visible consequence: A broken or hostile app class loader can return `java.lang.String` for
  a request such as `com.example.Target`, causing the crate to build a safe `raw::Class` or
  `JavaClass` wrapper whose displayed/cache name is the requested target while the underlying JNI
  class is a different type. Later member lookup or replacement failures would then be reported
  against the wrong class identity.
- Hardening: Loader-backed lookup now calls `Class.getName()` on the class returned by
  `ClassLoader.loadClass()` or `Class.forName(..., loader)` before promoting/caching it. A mismatch
  returns `Error::ClassLookupMismatch` with the requested and actual Java names, so a broken custom
  loader cannot populate a `Java` class cache under the wrong identity.
- Verification: `cargo fmt --check`; `just app-process-test-dex`; `cargo ndk -t arm64-v8a check
  --features app-process-test --lib`; `just check`; `just test all` on Quest 2 / Android 14,
  OPD2403 / Android 16, and Mi Max / Android 10. App-process coverage includes a
  `MisleadingClassLoader` that returns `java.lang.String` for `TestSubject` and expects
  `Error::ClassLookupMismatch`.
- Links: `CURRENT_BEHAVIOR.md` class-loader scope and `ClassLoaderKind` notes.

### Threading And Attachment

Focused discovery status: first focused pass completed for `Vm` attachment, `JavaScope`,
deferred `perform()`, main-thread scheduling, and runnable ART thread transitions. Callback panic
containment has been implemented. Runnable-transition reentrancy has been hardened with a
same-thread active-transition guard.

Look at `src/vm.rs`, `src/java/perform.rs`, `src/java/main_thread.rs`, `src/art/runnable_thread.rs`,
and `src/art/runnable_thread/arm64.rs`.

Questions:

- Can attached env values cross threads?
- Does deferred `perform()` preserve loader scope and callback lifetime correctly?
- Does main-thread scheduling behave predictably when the main looper is absent?
- Are runnable-thread and architecture-specific pieces separated cleanly enough to audit?

Findings:

Focused discovery notes:

- `Env`, `AttachedEnv`, `JavaScope`, `LocalRef`, `BorrowedLocalRef`, `JavaLocalRef`,
  `JavaLocalObject`, and `JavaLocalArray` remain visibly thread-affine through `Rc` markers and
  static assertions. `Java`, `JavaClass`, `JavaObject`, `JavaArray`, `JavaRef`, `raw::Class`, and
  `ClassLoaderRef` are `Send + Sync` because they hold VM-scoped global references or cloned VM /
  loader handles rather than local JNI frame state.
- `Vm::try_get_env()` and `Vm::get_env()` are already explicit `unsafe` APIs. Safe entry remains
  `Vm::attach_current_thread()` / `Java::attach()`, where `AttachedEnv` only detaches on drop when
  this crate created the attachment. `Vm::detach_current_thread()` is unsafe and documents that no
  live `Env`, `AttachedEnv`, local references, or other thread-local JNI state may remain.
- `Java::perform_now()` and `Java::attach()` keep attached scopes lexical and non-`Send`.
  Deferred `Java::perform()` accepts `Send + 'static` callbacks because they may run later from an
  Android startup hook thread; the callback receives a fresh app-loader-scoped `JavaScope` created
  immediately before invocation.
- `Java::perform()` preserves loader scope in both immediate and deferred paths: successful
  synchronous app-loader lookup publishes the loader before callback invocation, and startup-hook
  drains publish the loader before draining queued callbacks. `AppPerformState::drain_with_app_java`
  swaps the pending queue out before invoking callbacks, so callbacks can enqueue new work without
  holding the queue mutex.
- `Java::schedule_on_main_thread()` stores a clone of the scheduling `Java` handle with each task,
  so explicit loader scope is preserved when the task drains. The queue is process-global, protected
  by a mutex, and drains only from the stored main-thread id. Capability probing remains
  side-effect-light: it checks `epoll_wait`, `Looper.getMainLooper()`, and the `Handler` wakeup
  shape without installing hooks, enqueueing callbacks, or sending a looper message.
- The app-process harness covers the unsupported command-line main-looper case and skips live drain
  when `app_process` is already running on the main thread. The APK harness covers the real
  early-start path where deferred `perform()` publishes the app loader and then schedules a
  main-thread callback that runs exactly once with the app loader preserved.
- `RunnableThreadTransition::run()` uses a thread-local callback slot while the generated ART
  transition code moves the current thread into runnable state. The generated C callback catches
  panics from the Rust closure body, and `run()` clears the slot after the transition returns, but
  the slot is a single per-thread value and has no occupied-slot guard.

### Finding: deferred perform callback panic leaves the handle pending

- Status: Fixed
- Area: `src/java/perform.rs`, `src/replacement/closure.rs`, `src/apk_perform_test.rs`
- Kind: Callback failure | Threading
- Failure mode: `complete_perform()` invokes queued `Java::perform()` callbacks directly and only
  records `PerformStatus` after the callback returns a `Result`. A panic in a deferred callback
  unwinds out of `complete_perform()` before the status is updated. When the callback is running
  from the app-loader startup hook, the replacement closure catches the unwind at the hook boundary
  and converts it into replacement failure, but the `PerformHandle` itself remains `Pending`.
- User-visible consequence: A caller observing the returned `PerformResult<T>` can wait forever or
  keep seeing `Pending` even though the deferred callback has already panicked and will not be
  retried. In the startup-hook path this also mixes user callback failure into the internal
  replacement error channel instead of the perform result.
- Hardening: `complete_perform()` now records attachment failures before callback entry and wraps
  callback invocation in `catch_unwind(AssertUnwindSafe(...))`. A panic becomes
  `PerformStatus::Failed(Error::UnsupportedFeature { feature: "Java::perform callback", ... })`,
  leaving the replacement hook boundary as last-resort FFI containment instead of the first place a
  user callback panic is observed.
- Verification: `cargo fmt --check`; `cargo ndk -t arm64-v8a check --all-features`; `cargo ndk -t
  arm64-v8a clippy --all-features`; `just unit-test-build`; `just apk-perform-test all` on Quest 2
  / Android 14, OPD2403 / Android 16, and Mi Max / Android 10.

### Finding: main-thread scheduled callback panics are not contained by the scheduler

- Status: Fixed
- Area: `src/java/main_thread.rs`, `src/apk_perform_test.rs`, `src/app_process_test/checks.rs`
- Kind: Callback failure | Threading
- Failure mode: `MainThreadState::drain_if_main_thread()` invokes each queued
  `schedule_on_main_thread()` callback and records `Completed` or `Failed` only when the callback
  returns normally. A panic skips the status update. Because draining is entered from the Gum
  `epoll_wait` invocation listener, the panic may also unwind through the scheduler's native hook
  boundary unless the underlying Gum binding happens to contain it.
- User-visible consequence: A scheduled task can remain `Pending` forever after a panic, and a
  Rust panic from user callback code can escape through a native scheduler callback instead of
  becoming an observable `MainThreadTaskStatus::Failed` outcome.
- Hardening: `drain_if_main_thread()` now wraps each scheduled callback in
  `catch_unwind(AssertUnwindSafe(...))`, records a failed `MainThreadTaskStatus` for panics, and
  continues draining later queued tasks so one callback cannot strand the scheduler queue.
- Verification: `cargo fmt --check`; `cargo ndk -t arm64-v8a check --all-features`; `cargo ndk -t
  arm64-v8a clippy --all-features`; `just unit-test-build`; `just apk-perform-test all` on Quest 2
  / Android 14, OPD2403 / Android 16, and Mi Max / Android 10.

### Finding: runnable ART thread transition callback slot is not reentrancy-guarded

- Status: Fixed
- Area: `src/art/runnable_thread.rs`, `src/art/backend.rs`
- Kind: Threading | Runtime matrix
- Failure mode: `RunnableThreadTransition::run()` stores a single raw callback pointer in a
  thread-local slot, calls the generated transition code, and clears the slot afterward. If a future
  ART operation re-enters `with_runnable_art_thread()` on the same Rust thread before the outer
  transition has consumed its callback, the inner call can overwrite the slot and make the outer
  transition report "unable to perform runnable thread transition" or dispatch the wrong callback.
- User-visible consequence: Nested enumeration, replacement, heap, or deoptimization work could
  fail with a misleading unsupported reason, or in the worst case run a callback under the wrong ART
  thread-state assumption.
- Hardening: `RunnableThreadTransition::run()` now installs the transition callback through a
  scoped TLS guard that marks the current thread as actively transitioning until the outer
  transition returns. The ART completion callback consumes only the callback pointer, not the active
  state, so nested same-thread transitions fail closed with `UnsupportedFeature` instead of
  overwriting the outer slot.
- Verification: `cargo fmt --check`; `cargo ndk -t arm64-v8a check --all-features`;
  `just unit-test-build`; `just check`; focused unit coverage for occupied-slot rejection and for
  keeping the active guard set after the callback pointer has been consumed.

### Exceptions And JNI Call State

Focused discovery status: first focused pass completed for normal `Env` calls, member lookup,
string extraction, primitive/object array helpers, reflection dispatch helpers, exception summary
conversion, and replacement original-call paths. Empty primitive region validation, string accessor
ordering, and original-call local-reference cleanup findings are pending implementation.

Look at `src/env/calls.rs`, `src/env/fields.rs`, `src/env/members.rs`, `src/env/exceptions.rs`,
`src/java/dispatch.rs`, and replacement original-call paths.

Questions:

- Does each JNI call path handle pending exceptions consistently?
- Can a pending exception poison later helper calls?
- Are Java exceptions surfaced as Rust errors where users expect them?
- Are diagnostic calls like `toString()` careful about exception state?

Findings:

Focused discovery notes:

- Normal `Env` method calls, field get/set calls, object/reference helpers, object-array helpers,
  member lookup/reflection conversion helpers, `FindClass`, and string allocation check pending
  Java exceptions immediately after the raw JNI operation that can fail and convert the pending
  throwable into `Error::JavaException`.
- `check_pending_exception()` clears the pending exception, tries to retain a global throwable for
  higher-level rethrow/inspection, and summarizes the original throwable through `toString()`.
  Secondary failures while creating the global throwable or formatting the summary are intentionally
  cleared so the original exception remains the reported failure.
- Replacement original-call helpers use `check_pending_exception_preserve_raw()` after invoking the
  original method. This temporarily takes the pending Java exception for summary generation and then
  rethrows it, allowing the closure error path to record the Rust error while preserving Java
  exception delivery back through the replacement trampoline. The app-process harness already covers
  original-call and wrapper-call Java exception rethrow/conversion behavior.
- Safe replacement callbacks that return `Err(Error::JavaException { throwable: Some(..), .. })`
  rethrow the retained Java throwable. Other callback errors and panics preserve any already-pending
  Java exception by taking and rethrowing it around error recording.
- Reflection dispatch helpers (`src/metadata/reflection.rs` and the wrapper dispatch scoring path)
  use the same checked `Env` member calls, so Java reflection failures are surfaced as ordinary
  `Error::JavaException` values rather than leaving a pending exception for later helper calls.

### Finding: original instance-call lookup leaks the class local reference on errors

- Status: Fixed
- Area: `src/replacement/original_call.rs`
- Kind: Exception state | Lifetime
- Failure mode: `call_original_instance_method()` obtains the receiver class through
  `JNIEnv::GetObjectClass`, then enters a block that can return early with `?` or `return Err(...)`
  while preparing arguments, building C strings, looking up the method ID, checking lookup
  exceptions, or invoking the original method. The `DeleteLocalRef` for the receiver class only runs
  after that block completes successfully.
- User-visible consequence: Repeated original-call failures from a replacement callback can leak a
  local class reference on the replacement thread. In a hot hook, that can exhaust the JNI local
  reference table or make an exception-heavy replacement fail later with unrelated local-reference
  pressure.
- Hardening: `call_original_instance_method()` now wraps the receiver class local reference in a
  scoped cleanup guard immediately after `GetObjectClass` succeeds, so argument validation, method
  lookup, original invocation, and Java-exception propagation all delete the local on exit.
- Verification: `cargo ndk -t arm64-v8a check --features app-process-test --lib`;
  `cargo ndk -t arm64-v8a clippy --all-features`; `just test all` on Quest 2 / Android 14,
  including repeated instance original-call Java exception coverage followed by another instance
  original call on the same replacement path.

### Finding: UTF-16 string extraction checks only after `GetStringChars`

- Status: Fixed
- Area: `src/env/strings.rs`, `src/env/exceptions.rs`
- Kind: Exception state
- Failure mode: `Env::get_string_raw()` calls `GetStringLength` and then `GetStringChars`, but only
  checks pending exceptions if `GetStringChars` returns null. The exception-summary helper's
  internal `java_string_to_lossy_string()` follows the same ordering. If `GetStringLength` raises
  for an invalid or wrong-kind raw `jstring`, later JNI work can run while a Java exception is
  already pending.
- User-visible consequence: Unsafe raw string misuse can be reported with a misleading
  `GetStringChars`/null-return outcome, and diagnostic exception summarization may perform extra JNI
  calls while the detail extraction path is already in an exceptional state. Safe `StringRef` callers
  are normally protected by the wrapper type, but raw/public unsafe callers own only the raw handle
  guarantee, not pending-exception cleanup policy.
- Hardening: `Env::get_string_raw()` now rejects null raw `jstring` values before JNI, checks
  pending exceptions immediately after `GetStringLength`, and only then calls `GetStringChars`.
  The exception-summary helper mirrors the same ordering and stops if length extraction raises while
  building a diagnostic string.
- Verification: `cargo ndk -t arm64-v8a check --features app-process-test --lib`;
  `cargo ndk -t arm64-v8a clippy --all-features`; `just test all` on Quest 2 / Android 14,
  OnePlus device / Android 16, and Mi Max / Android 10, including low-level app-process coverage
  for null raw `jstring` rejection. Wrong-kind raw string probes were not added because invalid raw
  JNI references can abort ART instead of producing catchable Java exceptions.

### Finding: empty primitive array regions skip JNI validation

- Status: Fixed
- Area: `src/env/arrays.rs`
- Kind: Exception state
- Failure mode: `get_primitive_array_region()` and `set_primitive_array_region()` return `Ok(())`
  immediately for empty output/input slices without calling JNI. That avoids zero-length JNI calls,
  but also bypasses ART validation of the array reference and start index.
- User-visible consequence: Safe low-level calls can report success for an invalid array reference,
  wrong array kind, or invalid start index when the requested region length is zero, while otherwise
  equivalent non-empty calls would surface a Java exception.
- Hardening: Empty primitive regions are now an explicit no-copy policy. The helpers reject null
  arrays before JNI, validate `start` through `GetArrayLength`, and return an `InvalidArgumentValue`
  for out-of-range starts. Element kind is intentionally not checked for zero-length regions because
  ART aborts on some invalid zero-length typed-region probes instead of surfacing a catchable Java
  exception, and no elements are read or written.
- Verification: `cargo ndk -t arm64-v8a check --features app-process-test --lib`;
  `cargo ndk -t arm64-v8a clippy --all-features`; `just test all` on Quest 2 / Android 14,
  OnePlus device / Android 16, and Mi Max / Android 10, covering valid end-position empty get/set,
  invalid start rejection, null rejection, and explicit wrong-kind no-op behavior.
- Links: `CLEANUP_AUDIT.md` primitive array API finding.

### ART Layouts, Symbols, And Mutation

Focused discovery status: first focused pass completed for runtime layout probing, method-query and
replacement layout derivation, heap enumeration ART mutation, method patch/restore verification, and
deoptimization/JDWP setup. Runtime memory validation, fake handle-scope layout validation, and JDWP
hook lifecycle findings are pending implementation.

Look at `src/art/layout.rs`, `src/art/runtime_layout.rs`, `src/art/backend.rs`,
`src/art/replacement.rs`, `src/art/enumeration.rs`, and `src/art/deoptimization.rs`.

Questions:

- Does every layout probe validate enough before reading or writing?
- Are unsupported Android versions, ABIs, or symbol sets reported with clear reasons?
- Are mutation operations isolated from pure capability probing?
- Can failed restore or partial install leave ART state inconsistent?

Findings:

Focused discovery notes:

- ART capability gates fail closed on missing symbols, unsupported ABI, and Android API level before
  most mutation paths run. Class-loader enumeration and method patch/restore suspend ART threads
  before walking or mutating structures; method replacement also verifies snapshots after patch and
  restore and rolls back immediately when verification fails.
- Method replacement layout probing is stricter than the shared runtime layout scan: it reads current
  process memory maps, validates trampoline entrypoints as executable, validates target replacement
  functions, and checks candidate ArtMethod snapshots before patching. The general
  enumeration/deoptimization runtime scan still reads candidate Runtime fields directly and accepts
  non-null pointers before later feature-specific validation.
- Method-query layout derivation validates mirror::Class method arrays and ArtMethod entrypoints
  against `MemoryRanges`, bounds method array lengths, and treats unknown layouts as unsupported.
  `PrettyMethod` calls remain an ART-owned string ABI boundary, but the current wrapper destroys the
  temporary ART string after conversion.
- Heap enumeration has two distinct mutation profiles. `VisitObjects` only walks and promotes
  matching objects to JNI globals. `GetInstances` installs a synthetic
  `VariableSizedHandleScope`, links it into the current ART thread, passes a synthetic class handle
  to ART, then restores the previous top handle scope in `dispose()`/`Drop`.
- Deoptimization support is deliberately side-effecting when used: API 26-29 may start JDWP and
  temporarily replace ART JDWP transport functions, while API 30+ uses Instrumentation entrypoints
  derived from `Runtime::DeoptimizeBootImage` disassembly. Capability probing reports unsupported
  reasons without starting JDWP, but `deoptimization_support()` still runs layout and
  instrumentation offset discovery.

### Finding: general runtime layout scan reads candidate fields without memory-range validation

- Status: Discovered; revalidated during ART layout/symbol/mutation sprint
- Area: `src/art/runtime_layout.rs`, `src/art/backend.rs`, `src/art/deoptimization.rs`
- Kind: Runtime matrix
- Failure mode: `detect_runtime_layout_from_runtime()` scans offsets from the ART Runtime pointer
  and reads candidate heap/thread-list/class-linker/intern-table fields directly. The method
  replacement layout path also validates trampoline candidates against `MemoryRanges`, but the
  general enumeration/deoptimization path accepts non-null pointers from the same offset scan before
  later feature code uses them.
- User-visible consequence: On an unexpected ART layout, tagged-pointer behavior, or partially
  mismatched runtime build, a safe capability probe or enumeration/deoptimization call may trust a
  non-null but wrong ART field longer than necessary before returning unsupported, increasing the
  chance of a crash instead of a structured unsupported reason.
- Proposed hardening: Reuse one runtime-layout candidate scanner that can check derived ART object
  pointers against readable process memory before accepting the layout, while preserving the
  existing structured unsupported reasons for unknown layouts.
- Verification: extend existing host ART layout tests for invalid readable/unreadable candidates;
  app-process enumeration/deoptimization smoke coverage after implementation.
- Links: `CLEANUP_AUDIT.md` finding "runtime layout probing has split but overlapping flows".

### Finding: fake ART handle scope mutates thread-local handle state in a safe heap API

- Status: Discovered; revalidated during ART layout/symbol/mutation sprint
- Area: `src/art/enumeration.rs`, `src/art/backend.rs`
- Kind: Unsafe boundary | Runtime matrix
- Failure mode: heap enumeration through `Heap::GetInstances` installs a synthetic
  `VariableSizedHandleScope` by writing the current ART thread's top handle-scope pointer, then
  restores it in `dispose()`/`Drop`. The safe public `choose_instances` path owns the guarantee that
  the inferred top-handle-scope offset and fake structure layout match the runtime.
- User-visible consequence: If the inferred thread offset or handle-scope layout is wrong for an ART
  build, heap enumeration can corrupt thread-local ART handle state instead of reporting
  `UnsupportedFeature`.
- Proposed hardening: Treat this as an explicit ART layout prerequisite for heap enumeration:
  validate the inferred top handle-scope slot against readable memory, keep construction/restore in
  one narrow helper, and add a maintainer note or tests around the expected layout. If the guarantee
  cannot be made reliable on a runtime, report heap enumeration as unsupported for that path.
- Verification: host tests for handle-scope offset/restore helpers where possible; app-process heap
  enumeration checks on supported devices.
- Links: `CLEANUP_AUDIT.md` finding "fake ART handle-scope helpers need a clearer ownership home".

### Finding: JDWP deoptimization hooks stay installed after the startup handshake

- Status: Discovered
- Area: `src/art/deoptimization.rs`
- Kind: Runtime matrix | Callback failure
- Failure mode: API 26-29 deoptimization starts a process-global `ArtJdwpSession` once and stores it
  in a `OnceLock`. The session keeps the `JdwpAdbState::Accept` listener and
  `JdwpAdbState::ReceiveClientFd` replacement alive for process lifetime; `ReceiveClientFd` keeps
  returning the stored client peer FD instead of reverting after the first startup handshake. The
  accept listener also scans and writes a control socket field at a hard-coded offset on every
  callback.
- User-visible consequence: A safe deoptimization request can permanently alter ART's JDWP transport
  behavior for the process. Later debugger startup, repeated deoptimization setup, or a changed ART
  JDWP state layout may reuse a stale FD or write to the wrong JDWP state slot instead of failing
  with a structured unsupported reason.
- Proposed hardening: Make the JDWP transport hook lifecycle one-shot and observable: revert
  `ReceiveClientFd` after the startup handshake, detach or disable the accept listener after it has
  patched one state object, reset the atomic FD when the one-shot path is consumed, and validate the
  scanned control-socket slot before writing. Keep the process-global session only for state that
  must remain alive after JDWP has started.
- Verification: Host/unit state-machine coverage for one-shot JDWP hook consumption and repeated
  `ensure_jdwp_ready()` calls; `cargo ndk -t arm64-v8a clippy --all-features`; `just art-test all`
  or targeted device coverage if the live JDWP startup path changes.

### Replacement Callback Lifecycle

Focused discovery status: first focused pass completed for public callback return lifetime
boundaries. Lifecycle teardown findings were revalidated, but implementation is still pending.

Initial static pass covered `src/replacement/api.rs`, `src/replacement/closure.rs`,
`src/replacement/original.rs`, `src/replacement/original_call.rs`, `src/replacement/backend.rs`,
ART restore paths used by `MethodReplacement`, and app-process replacement lifecycle coverage. The
first focused pass then re-read callback-local return paths in `src/replacement/api.rs`, the
internal startup-hook forwarding in `src/java/perform.rs`, and the pass-through examples in
`examples/frida_js_ergonomics_probe.rs`.

Look at `src/replacement/api.rs`, `src/replacement/closure.rs`,
`src/replacement/trampoline.rs`, `src/replacement/original.rs`,
`src/replacement/original_call.rs`, `src/replacement/backend.rs`, and internal startup-hook use in
`src/java/perform.rs`.

Questions:

- Are panics contained before returning to Java?
- Is callback-local state removed on all exit paths?
- Can original-call handles outlive the active replacement/thread scope?
- Are wrong return kinds and assignability failures handled before JNI sees invalid data?

Findings:

Focused discovery notes:

- Public callback return path map:
  `JavaHookContext::call_original_raw()` is explicitly unsafe and returns `JavaHookReturn`;
  `JavaHookContext::proceed()` is safe and returns the same raw-lane `JavaHookReturn`;
  `JavaHookContext::call_original*` typed helpers extract object and array returns into
  callback-local `JavaLocalObject<'state>` / `JavaLocalArray<'state>`; `JavaHookReturn::raw_*` and
  `into_raw_*` are unsafe; `JavaHookReturn::object` / `array`, object/array `From` impls,
  object/array `IntoJavaHookReturn` impls, and `AsJavaHookReturn` are safe public constructors that
  can place object references into the raw object/array lanes.
- `src/java/perform.rs` intentionally uses `unsafe call_original_raw()` plus `raw_arguments()` for
  startup hook forwarding, then immediately returns the raw object to Java after app-loader
  publication. This is an internal startup hook shape, not the normal public pass-through API, and
  should remain separated from public `proceed()` hardening.
- The only current public-style `proceed()` call sites found are in
  `examples/frida_js_ergonomics_probe.rs`, where the callbacks return `invocation.proceed()` from
  `put*`/`fallible` pass-through hooks. Those examples should remain ergonomic after hardening, but
  can tolerate an API that returns a pass-through wrapper or requires an explicitly typed original
  return when the Java return type is object/array.
- Callback errors and panics are caught before returning to Java in the closure state, and the
  app-process harness covers ordinary callback errors, wrong return kinds, panics, Java-backed
  errors, safe constructor failures, and active-callback revert waiting.
- Argument marshalling from the trampoline frame currently does not call JNI, so it has no known
  pending-exception state to preserve. If future hardening adds JNI validation to this path, it
  should use the same pending-exception preservation rule as callback error handling.
- Safe constructor replacement failures attempt to install an `IllegalStateException` when the
  callback error does not already carry a Java throwable. The focused pass revalidated that
  panic-before-initialization is handled in `install_constructor_hook()` before returning to the
  closure state, but the app-process harness currently covers explicit safe constructor failure
  rather than the panic-before-initialization branch.
- `JavaHookSet::revert_all()` now attempts every guard in reverse installation order and returns
  the first restore error after all guards have been asked to restore. `JavaHookGuard::drop()`,
  `ClosureMethodReplacement::drop()`, and `MethodReplacement::drop()` remain intentionally
  non-panicking and may leak hook state after active-callback or restore-failure paths. Explicit
  `revert()` remains the only user-visible restore error observation path.

### Finding: hook-set batch revert can leave later guards active after one restore failure

- Status: Fixed
- Area: `src/replacement/api.rs`
- Kind: Callback failure
- Failure mode: `JavaHookSet::revert_all()` returns on the first `JavaHookGuard::revert()` error
  while iterating in reverse order, so older guards that have not yet been visited remain active.
- User-visible consequence: A caller using `JavaHookSet` as a lifecycle owner may believe teardown
  has been attempted for the whole set, while some hooks were never asked to restore after an
  unrelated restore failure.
- Hardening: `JavaHookSet::revert_all()` now uses an all-attempting reverse teardown helper. It
  preserves the existing `Result<()>` surface by returning the first restore error encountered in
  reverse order after every guard has been asked to restore.
- Verification: `cargo fmt --check`; `cargo ndk -t arm64-v8a check --all-features`;
  `just unit-test-build`; `just check`; focused unit coverage for the reverse teardown helper
  confirms that later restore failures do not prevent older guards from being attempted.
- Links: `HARDENING_AUDIT.md` finding "batch hook teardown failure has no focused non-device test".

### Finding: safe proceed returns raw callback-local references

- Status: Discovered; revalidated in first focused lifecycle pass
- Area: `src/replacement/api.rs`
- Kind: Lifetime | Raw handle
- Failure mode: `JavaHookContext::call_original_raw()` is explicitly unsafe, but
  `JavaHookContext::proceed()` is safe and returns `JavaHookReturn`. For object and array returns,
  that value carries raw callback-local JNI references with no Rust lifetime. The common pass-through
  use is safe when returned immediately from the callback, but the type also lets a caller store,
  inspect, or reuse the raw return after the callback scope.
- User-visible consequence: A callback author can accidentally make a safe-looking pass-through
  helper produce a raw object/array handle that outlives the replacement callback, then feed that
  stale reference back through another raw or hook-return path.
- Proposed hardening: Split the ergonomic pass-through operation from raw original-call results.
  Prefer making the raw shape explicit as `unsafe proceed_raw()` or by folding it into
  `call_original_raw(self.inner.arguments())`, then add a safe `proceed()` return type that is
  lifetime-bound to the callback and can only be converted into the callback's immediate return. If
  that is too invasive, make `proceed()` generic over `FromJavaHookReturn<'state>` and update
  object/array pass-through examples to spell the expected local return type.
- Verification: Compile coverage for representative pass-through hooks; app-process smoke coverage
  for object and array pass-through after changing the API.
- Links: `HARDENING_AUDIT.md` finding "callback-local raw returns can escape without a lifetime".

### Finding: safe object and array hook-return conversions erase local-reference lifetimes

- Status: Discovered
- Area: `src/replacement/api.rs`
- Kind: Lifetime | Raw handle
- Failure mode: `JavaHookReturn` is a public alias for `JavaReturn<RawJavaObject, RawJavaObject>`.
  Safe constructors and conversions such as `JavaHookReturn::object()`, `JavaHookReturn::array()`,
  `From<JavaLocalObject<'_>>`, `From<JavaLocalArray<'_>>`, `IntoJavaHookReturn` for local
  object/array wrappers, and `AsJavaHookReturn` for nullable wrappers all copy object references
  into raw object/array lanes without retaining the source lifetime. This is intended for immediate
  callback returns, but the resulting `JavaHookReturn` can be stored or reused after a local
  reference scope ends.
- User-visible consequence: A caller can construct a hook return from a callback-local or
  attach-local Java reference through a safe conversion, keep the raw-lane return past that scope,
  and later return or inspect a stale JNI reference from safe-looking replacement code.
- Proposed hardening: Introduce a lifetime-bound public hook-return wrapper for safe object/array
  returns, or split raw `JavaHookReturn` from safe callback return values so lifetime-erasing
  object/array lanes are only reachable through unsafe/raw APIs. Keep primitive, null, and borrowed
  global/object-wrapper returns ergonomic, but require object/array local-reference returns to carry
  the callback or env lifetime until the closure boundary consumes them.
- Verification: Compile coverage for returning `&JavaObject`, `JavaLocalObject`, nullable object,
  `&JavaArray`, `JavaLocalArray`, and nullable array from replacement callbacks; app-process object
  and array return/pass-through smoke coverage after the API split.
- Links: `HARDENING_AUDIT.md` finding "safe proceed returns raw callback-local references";
  `CLEANUP_AUDIT.md` finding "raw hook return alias is a public user concept".

### Finding: guard drop and restore failure visibility is intentionally lossy but not fully audited

- Status: Discovered; revalidated in first focused lifecycle pass
- Area: `src/replacement/closure.rs`, `src/replacement/backend.rs`, `src/art/replacement.rs`
- Kind: Callback failure | Runtime matrix
- Failure mode: Explicit `JavaHookGuard::revert()` reports restore failure and keeps the replacement
  active, but `Drop` must stay non-panicking. If teardown runs while the current callback is active,
  or if backend restore fails during drop, the closure/thunk/replacement state is leaked to avoid
  freeing code or state that ART may still reference. The first focused lifecycle pass revalidated
  this shape as intentional, but callers still only observe restore errors through explicit
  `revert()`.
- User-visible consequence: A guard dropped without explicit `revert()` can leave a replacement and
  its support state live for process lifetime after a restore failure, without a caller-observable
  error unless they used explicit revert before drop.
- Proposed hardening: Keep `Drop` non-panicking, but decide whether the public lifecycle should
  recommend or require explicit `revert()` for error observation, record drop-time restore failures
  in guard state before leaking where possible, or expose a lifecycle owner that makes teardown
  outcome explicit.
- Verification: Host tests for any fake lifecycle state machine; app-process lifecycle coverage for
  explicit revert and active-callback teardown behavior.
- Links: `CURRENT_BEHAVIOR.md` replacement lifecycle notes.

### Test Matrix

Focused discovery status: first focused pass completed for host/unit tests, Android unit-test
recipes, app-process live-runtime checks, APK early-start coverage, native ART bootstrap coverage,
and hardening findings that already name missing tests. Custom loader and host unit-gate findings
are pending implementation.

Questions:

- Which risky behavior has only compile coverage?
- Which host-testable logic is only exercised through device tests?
- Which app startup behavior requires APK coverage?
- Which native ART bootstrap assumption belongs in `art_test` only?

Findings:

Focused discovery notes:

- The harness split matches the project posture. `just unit-test` runs Rust unit tests through
  `cargo-ndk-runner`, `just test` runs the app-process live-runtime harness, the APK perform
  recipe covers real APK early startup and main-looper scheduling, and `just art-test` stays focused
  on native ART loading plus manual VM creation.
- App-process coverage is broad on successful Java facade behavior: low-level JNI helpers,
  app-loader lookup, DexClassLoader lookup, metadata/enumeration, heap enumeration when supported,
  selected wrappers, replacement installation/revert, callback argument/return shapes, Java
  exception propagation, replacement callback errors, replacement callback panics, and
  active-callback revert waiting.
- The APK harness covers the intended early-start happy path where `Java::perform()` stays pending
  before `Application` creation, drains exactly once with an app-loader-scoped `Java`, publishes the
  default app loader before bare `use_class()`, and schedules a main-thread callback that runs once.
  It does not currently cover ambiguous loader publication, deferred callback failure/panic, or
  repeated startup-hook events.
- Host/unit coverage exists for many parser, selector, layout, replacement-planning, perform-state,
  and main-thread state-machine helpers. The ordinary host gate is now available for
  platform-independent library tests through `cargo test --lib` / `just host-test`; Android-gated
  modules still use the `cargo ndk` and device recipes below.
- The biggest test gaps are negative boundaries where the desired hardening should fail before JNI
  or ART sees a bad combination: wrong selected receivers, wrong exact reference arguments, hostile
  custom loader results, panic-to-status conversion for deferred/main-thread callbacks, and
  zero-length primitive array validation policy.

### Finding: selected receiver mismatch lacks negative app-process coverage

- Status: Fixed
- Area: `src/java/wrapper.rs`, `src/app_process_test/checks.rs`
- Kind: Test gap
- Failure mode: safe selected method and field handles can be called with an arbitrary object
  receiver, but the app-process harness currently covers successful matching receivers and does not
  exercise wrong-receiver method calls, field reads, or field writes.
- User-visible consequence: receiver validation can remain absent, or later be added with an
  unstable error shape, without a live-runtime test proving the crate reports a clear Rust error
  before JNI sees a mismatched selected member/receiver pair.
- Hardening: App-process coverage now selects `TestSubject.instanceNumber()` and `TestSubject.number`
  and passes a `java.lang.String` receiver, asserting `Error::InvalidObjectType` for method call,
  field get, and field set.
- Verification: `just test all` on Quest 2 / Android 14, OnePlus device / Android 16, and Mi Max /
  Android 10.
- Links: `HARDENING_AUDIT.md` finding "selected method and field handles accept unchecked
  receivers".

### Finding: batch hook teardown failure has no focused non-device test

- Status: Fixed
- Area: `src/replacement/api.rs`, `src/replacement/closure.rs`
- Kind: Test gap
- Failure mode: the known `JavaHookSet::revert_all()` fail-fast behavior is only documented in the
  audit and exercised indirectly through successful app-process lifecycle cases. There is no narrow
  test that simulates one guard failing to restore and verifies whether later guards are attempted.
- User-visible consequence: a future cleanup could preserve or change batch teardown semantics
  without an immediate test explaining the intended behavior.
- Hardening: Added focused unit coverage for the shared reverse teardown helper used by
  `JavaHookSet::revert_all()`. The test uses fake guards to prove all guards are attempted in
  reverse installation order and that the first reverse-order restore error is returned.
- Verification: `cargo fmt --check`; `cargo ndk -t arm64-v8a check --all-features`;
  `just unit-test-build`; `just check`.
- Links: `HARDENING_AUDIT.md` replacement lifecycle finding for `JavaHookSet::revert_all()`.

### Finding: empty primitive array region policy lacks runtime coverage

- Status: Fixed
- Area: `src/env/arrays.rs`, `src/app_process_test/checks.rs`
- Kind: Test gap
- Failure mode: primitive array region tests cover normal non-empty get/set calls, but the
  zero-length get/set fast path identified earlier has no app-process case for null arrays,
  wrong-kind arrays, invalid starts, or an explicitly accepted no-op policy.
- User-visible consequence: the low-level safe Env API can keep reporting success for invalid empty
  regions, or a later fix can change that policy, without a device test documenting the expected JNI
  behavior.
- Hardening: App-process low-level JNI checks now cover the chosen empty-region policy for valid
  end-position empty get/set, invalid start rejection, null rejection, and explicit wrong-kind no-op
  behavior.
- Verification: `just test all` on Quest 2 / Android 14, OnePlus device / Android 16, and Mi Max /
  Android 10.
- Links: `HARDENING_AUDIT.md` finding "empty primitive array regions skip JNI validation".

### Finding: host unit-test gate is currently unavailable

- Status: Fixed
- Area: `src/lib.rs`, `src/error.rs`, `justfile`
- Kind: Test gap
- Failure mode: `cargo test --lib` on the host currently fails before running host-testable
  selector, argument, and metadata tests because `src/error.rs` imports Android-gated `vm::Vm`.
  Parser, dispatch, and formatting logic must therefore use broader Android build or device-oriented
  gates even when the behavior itself is host-testable.
- User-visible consequence: Contributors get slower and less targeted feedback for changes that do
  not need a live Android runtime, and verification instructions can drift between cleanup,
  hardening, and roadmap work.
- Hardening: `JavaThrowable` now keeps its retained-throwable `Vm` payload behind the Android
  implementation boundary, with a host-only opaque shape that preserves `Error::JavaException`
  formatting and equality tests without importing Android-gated VM modules. Added `just host-test`
  and listed it in `ROADMAP.md` so the host gate is explicit.
- Verification: `cargo test --lib`; `cargo fmt --check`; `just host-test`;
  `cargo ndk -t arm64-v8a check --all-features`.
- Links: `ROADMAP.md` verification section.

### Finding: exact selected reference-argument mismatch lacks negative app-process coverage

- Status: Fixed
- Area: `src/java/args.rs`, `src/java/wrapper.rs`, `src/app_process_test/checks.rs`
- Kind: Test gap
- Failure mode: The app-process harness covers primitive argument type/range failures and positive
  object argument calls, but it does not exercise exact selected method, constructor, or field
  paths with an incompatible non-null object reference. The corresponding hardening finding says
  these paths currently accept reference-shaped values without `IsInstanceOf` assignability checks.
- User-visible consequence: Safe selected wrapper calls can continue to pass wrong object types to
  JNI, or a future fix can return an unstable or overly broad error, without a live-runtime test
  proving the facade rejects the bad argument before ART receives it.
- Hardening: App-process coverage now checks exact selected `staticEcho(String)` with a
  `TestSubject` argument, `RuntimeException(String)` construction with a `TestSubject` argument,
  and `TestSubject.subjectValue` assignment with a `String` value, expecting explicit
  `InvalidArgumentType` / `InvalidFieldValueType` errors.
- Verification: `just test all` on Quest 2 / Android 14, OnePlus device / Android 16, and Mi Max /
  Android 10.
- Links: `HARDENING_AUDIT.md` finding "exact wrapper calls do not validate reference argument
  assignability".

### Finding: custom loader result mismatch has no hostile-loader fixture

- Status: Fixed
- Area: `src/java/lookup.rs`, `src/app_process_test/checks.rs`, `test-fixtures/src/`
- Kind: Test gap
- Failure mode: Current loader coverage proves positive app-loader, DexClassLoader, and enumerated
  loader lookups. It does not include a custom `ClassLoader` whose `loadClass(String)` deliberately
  returns a different `Class` than requested, which is the negative case identified by the
  loader-backed lookup hardening finding.
- User-visible consequence: Loader-backed lookup can keep caching a class under the requested name
  even when the returned Java class has a different identity, or a future validation fix can regress
  descriptor/binary-name comparison, without a device test showing the user-facing failure.
- Hardening: Added `MisleadingClassLoader` to the app-process fixture jar and a negative
  app-loader-surface check asserting that `Java::with_loader(...).find_class(TEST_SUBJECT)` returns
  `Error::ClassLookupMismatch` when the custom loader returns `java.lang.String.class`.
- Verification: `cargo fmt --check`; `just app-process-test-dex`; `cargo ndk -t arm64-v8a check
  --features app-process-test --lib`; `just check`; `just test all` on Quest 2 / Android 14,
  OPD2403 / Android 16, and Mi Max / Android 10.
- Links: `HARDENING_AUDIT.md` finding "loader-backed class lookup trusts custom loader results".

### Finding: deferred and main-thread callback panic status lacks focused unit coverage

- Status: Fixed
- Area: `src/java/perform.rs`, `src/java/main_thread.rs`
- Kind: Test gap | Callback failure
- Failure mode: Unit tests cover normal completion, returned callback errors, FIFO draining, and
  wrong-thread non-draining for perform/main-thread state machines. They do not cover callbacks that
  panic. The current implementation records status only after a callback returns, so panic handling
  is a known hardening target without a narrow regression test.
- User-visible consequence: A deferred `PerformHandle` or `MainThreadTaskHandle` can remain
  `Pending` forever after callback panic, and a future panic-containment fix could fail to continue
  draining later tasks without a focused state-machine test catching it.
- Hardening: Added focused panic-regression unit coverage for the shared `Java::perform` callback
  status transition and for main-thread drain panic handling. The main-thread test suppresses the
  intentional panic hook, asserts the first handle becomes `Failed`, and verifies a later queued
  callback still completes.
- Verification: `cargo fmt --check`; `cargo ndk -t arm64-v8a check --all-features`; `cargo ndk -t
  arm64-v8a clippy --all-features`; `just unit-test-build`; `just apk-perform-test all` on Quest 2
  / Android 14, OPD2403 / Android 16, and Mi Max / Android 10.
- Links: `HARDENING_AUDIT.md` findings "deferred perform callback panic leaves the handle pending"
  and "main-thread scheduled callback panics are not contained by the scheduler".

Reviewed during cleanup discovery: `src/art/tests.rs` already has broad host coverage for ART
layout derivation, patch/restore verification, trampoline probing, JNI method-ID decoding, and
replacement diagnostics. Keep those tests as the first gate for ART refactors, then use app-process
tests for live runtime behavior.

## Finding Template

```md
### Finding: short title

- Status: Discovered | Fixed | Unsupported | Unsafe by design | Deferred
- Area: module or file path
- Kind: Unsafe boundary | Lifetime | Threading | Exception state | Raw handle | Runtime matrix | Callback failure | Test gap
- Failure mode:
- User-visible consequence:
- Proposed hardening:
- Verification:
- Links:
```
