# Second-Opinion Cleanup Audit

This file is the independent discovery notebook and implementation tracker for the second-opinion
cleanup pass described in `SECOND_OPINION_CLEANUP_PASS.md`.

Do not use this file to restate the first cleanup pass. Start from the code, record independent
observations here, then compare with `CLEANUP_AUDIT.md` after each module family review.

## Status Legend

- Discovered: recorded during second-opinion discovery, not implemented yet.
- Fixed: code changed and verification noted.
- Deferred: still wanted, with a reason.
- Rejected: intentionally kept, with the design reason.
- Moved To Hardening: primarily safety/correctness; tracked in `HARDENING_AUDIT.md`.
- Moved To Documentation: primarily public documentation; tracked in `DOCUMENTATION_PASS.md`.

## Cleanup Categories

- Delete: unused code, stale aliases, compatibility shims, helpers, or constants.
- Merge: duplicate concepts or near-synonyms.
- Move: wrong module or abstraction layer.
- Rename: misleading, too-internal, or overly broad names.
- Simplify: more direct flow or type shape without behavior loss.
- Document: maintainer-facing comments only.
- Reject Previous: first-pass cleanup decision that needs reconsideration.
- Other: anything else.

## Review Protocol

For every section below:

1. Read the code first.
2. Record findings or explicitly write `Reviewed: no second-opinion cleanup findings.`
3. Compare with `CLEANUP_AUDIT.md`.
4. Add the comparison result to each finding.
5. Link safety/correctness issues to `HARDENING_AUDIT.md` instead of implementing them as cleanup.

## Public Crate Shape

Files: `src/lib.rs`, `src/error.rs`, `src/android.rs`, `src/runtime.rs`, `src/vm.rs`.

Findings:

### Finding: `RuntimeFlavor` match dispatch in `RuntimeInner` is dead abstraction

- Status: Discovered
- Area: `src/runtime.rs`
- Kind: Simplify
- Independent observation: Every method on `RuntimeInner` (`capabilities`, `enumerate_class_loaders`,
  `enumerate_loaded_classes`, `enumerate_methods`, `choose_instances`, `deoptimize_everything`,
  `deoptimize_boot_image`, `deoptimize_method`) follows an identical pattern:
  `match self.flavor { RuntimeFlavor::Art => self.art.xxx(vm) }`. The enum has a single variant and
  this is an Android-ART-only crate. The match arms add eight identical exhaustiveness stubs that
  visually imply runtime selection is active when it is not.
- Why it matters: Every new capability or delegation method must reproduce the match boilerplate.
  Readers scanning the file see a dispatch surface that looks polymorphic but does exactly one thing.
  This inflates the conceptual footprint of `RuntimeInner` for no behavioral gain.
- Proposed cleanup: Remove the `match self.flavor` dispatch and call `self.art.*` directly.
  `RuntimeFlavor` can remain as a public capability reporting enum (it already appears in
  `JavaCapabilities::flavor`) but the internal dispatch should not route through it.
- First-pass comparison: Overlapping unresolved — `CLEANUP_AUDIT.md` records this as "Discovered"
  with the same proposed fix.
- Verification: `just check`; `just build`.
- Links: `CLEANUP_AUDIT.md` finding "`RuntimeFlavor` single-variant dispatch adds a dead
  abstraction".

### Finding: `Vm` duplicates `RuntimeInner` forwarding methods

- Status: Discovered
- Area: `src/vm.rs`, `src/runtime.rs`
- Kind: Simplify
- Independent observation: `Vm` has a set of `pub(crate)` methods (`capabilities`,
  `enumerate_class_loaders`, `enumerate_loaded_classes`, `enumerate_methods`, `choose_instances`,
  `deoptimize_everything`, `deoptimize_boot_image`, `deoptimize_method_id`) that each simply
  delegate to the identically-named method on `self.runtime`. This creates a two-layer forwarding
  chain: `Vm → RuntimeInner → ArtBackend`. `Vm` is the owner of the `Arc<RuntimeInner>` so it makes
  sense for it to provide access, but the 1:1 forwarding method count is high.
- Why it matters: Adding a new runtime feature means adding a method in `ArtBackend`, a forwarding
  method in `RuntimeInner`, and another forwarding method in `Vm`. This triple-bounce increases
  maintenance cost. The `RuntimeInner` layer exists partly because of the `RuntimeFlavor` match
  dispatch; if that is removed, the two forwarding layers become even harder to justify.
- Proposed cleanup: If the `RuntimeFlavor` dispatch is removed, consider whether `RuntimeInner` can
  be slimmed to a plain data struct with `ArtBackend` accessed directly through `Vm`, or whether
  `Vm` methods can delegate directly to `self.runtime.art.*`. This would collapse the triple-bounce
  to a single indirection.
- First-pass comparison: New — the first audit noted the `RuntimeFlavor` dispatch but did not call
  out the `Vm → RuntimeInner` forwarding layer separately.
- Verification: `just check`; `just build`.
- Links: Previous finding in this file about `RuntimeFlavor` dispatch.

### Finding: `Vm::gum()` and `RuntimeInner::_gum` naming inconsistency

- Status: Discovered
- Area: `src/runtime.rs`, `src/vm.rs`
- Kind: Rename
- Independent observation: `RuntimeInner::_gum` is named with a leading underscore suggesting it is
  unused or only kept for lifetime purposes, but it is actively read by `Vm::gum()` (which calls
  `self.runtime._gum`). Additionally, there is a process-global `process_gum()` function that many
  ART internals use directly, making it unclear whether the `Vm::gum()` accessor or
  `runtime::process_gum()` is the canonical entry point.
- Why it matters: The underscore naming is misleading — readers expect `_` prefixed fields to be dead
  or structural-only. The dual access pattern (`process_gum()` vs `Vm::gum()`) makes gum access
  harder to trace.
- Proposed cleanup: Rename `_gum` to `gum`. Either document both access patterns explicitly or unify
  callers onto one pattern.
- First-pass comparison: Overlapping unresolved — `CLEANUP_AUDIT.md` records this as "Discovered"
  in the cross-family Gum singleton finding.
- Verification: `just check`.
- Links: `CLEANUP_AUDIT.md` finding "Gum access has both process-global and VM accessor shapes".

### Finding: `Error` enum is very large with some near-duplicate variants

- Status: Discovered
- Area: `src/error.rs`
- Kind: Simplify | Merge
- Independent observation: `Error` has 28 variants (counted: `ArtRuntimeNotFound`, `SymbolNotFound`,
  `UnsupportedFeature`, `AppClassLoaderUnavailable`, `NoCreatedJavaVm`, `JniCallFailed`,
  `JavaException`, `NullReturn`, `InvalidSignature`, `InvalidArguments`, `InvalidArgumentType`,
  `InvalidArgumentValue`, `InvalidReturnType`, `InvalidFieldType`, `InvalidFieldValueType`,
  `InvalidFieldValue`, `InvalidObjectType`, `InvalidQuery`, `MethodNotFound`, `MethodNameNotFound`,
  `OverloadNotFound`, `NoCompatibleOverload`, `AmbiguousOverload`, `AmbiguousMethod`, `FieldNotFound`,
  `FieldNameNotFound`, `AmbiguousField`, `WrongMethodKind`, `WrongFieldKind`,
  `InvalidReplacementImplementation`, `UnsupportedReplacementImplementation`,
  `InvalidReplacementState`, `InteriorNul`, `InvalidUtf8`, `InvalidUtf16`). Some groups look like
  they could be collapsed:
  - `MethodNotFound` / `MethodNameNotFound` / `OverloadNotFound` carry similar semantics with
    slightly different fields.
  - `FieldNotFound` / `FieldNameNotFound` are a similar pair.
  - `InvalidFieldType` / `InvalidFieldValueType` / `InvalidFieldValue` — three field validation
    variants versus two for arguments (`InvalidArgumentType` / `InvalidArgumentValue`).
- Why it matters: Each variant adds a constructor call-site and an error message format string.
  Users writing `match` on errors encounter a very long enum. While these variants are individually
  reasonable for diagnostics, the sheer count increases conceptual weight.
- Proposed cleanup: This may be intentional granularity for good diagnostics. Defer unless the
  variant count becomes a concrete teachability problem. At minimum, add a maintainer note grouping
  the variants logically (JNI/runtime errors, signature/argument validation, member lookup,
  replacement, encoding).
- First-pass comparison: New — the first audit did not flag the `Error` enum size.
- Verification: No behavior change if only documenting.
- Links: None.

### Finding: `AndroidVersion::api_level` uses `jni::jint` in a public type

- Status: Discovered
- Area: `src/android.rs`
- Kind: Rename
- Independent observation: `AndroidVersion` is `pub` and its `api_level` field is typed as
  `jni::jint` (which is `i32`). The `jni` module is public, but using a JNI type alias for an
  Android API level leaks JNI vocabulary into a user-facing Android concept.
- Why it matters: A user reading `AndroidVersion.api_level` would expect a plain `i32` or `u32`, not
  a JNI type. The typedef `jint = i32` makes this harmless at the type level, but it makes the
  import path and docs teach `jni::jint` unnecessarily.
- Proposed cleanup: Change the field type to plain `i32`. The internal parse function can continue
  using `jni::jint` if desired.
- First-pass comparison: New — the first audit did not flag this.
- Verification: `just check`.
- Links: None.

### Finding: `android_api_level_for_feature` is a near-duplicate of `android_api_level`

- Status: Discovered
- Area: `src/android.rs`
- Kind: Merge
- Independent observation: `android_api_level()` calls
  `android_property("ro.build.version.sdk", ANDROID_VERSION_FEATURE)` and then
  `parse_android_api_level(ANDROID_VERSION_FEATURE, &value)`. `android_api_level_for_feature(feature)`
  does the same thing with a caller-supplied `feature` label. The only difference is the feature
  string used in error context. Both are `pub(crate)`.
- Why it matters: `android_api_level()` is a special case of `android_api_level_for_feature()` with a
  hardcoded feature name. Two functions for the same operation with one trivially expressible in
  terms of the other adds a minor naming burden.
- Proposed cleanup: Make `android_api_level()` call `android_api_level_for_feature(ANDROID_VERSION_FEATURE)`,
  or remove `android_api_level()` and have callers pass the feature label. The internal-only
  visibility makes this safe.
- First-pass comparison: New — the first audit did not flag this.
- Verification: `just check`.
- Links: None.

## Safe JNI Environment And Values

Files: `src/env/`, `src/jni.rs`, `src/refs.rs`, `src/value.rs`, `src/signature.rs`,
`src/metadata.rs`, `src/modifiers.rs`.

Findings:

### Finding: `AsJObject` / `AsJClass` traits duplicate the sealed `JavaObjectRefSealed` / `JavaClassRefSealed`

- Status: Discovered
- Area: `src/refs.rs`
- Kind: Merge
- Independent observation: `src/refs.rs` defines two trait hierarchies:
  1. `sealed::JavaObjectRefSealed` → `JavaObjectRef` (public, sealed) with `as_jobject()`
  2. `AsJObject` (pub(crate)) with `as_jobject()`

  The `AsJObject` trait has a blanket impl `impl<T: JavaObjectRef> AsJObject for T` that just
  forwards to the sealed method. The same pattern holds for `AsJClass` / `JavaClassRefSealed` /
  `JavaClassRef`. This means there are two parallel trait hierarchies with the same method name, one
  public-sealed and one crate-internal, with a blanket bridge between them.
- Why it matters: Internal code uses `AsJObject` and `AsJClass` everywhere (in `src/env/`,
  `src/metadata.rs`, etc.) as the crate-internal access pattern. But they are just a crate-internal
  mirror of the sealed public traits. This adds conceptual indirection: contributors must understand
  both hierarchies and the blanket bridge to know why bounds use `AsJObject` instead of
  `JavaObjectRef`.
- Proposed cleanup: Consider whether crate-internal code can use `JavaObjectRef` directly (since the
  sealed trait ensures only crate types implement it). If the `?Sized` bounds or ergonomic reasons
  require the separate internal trait, at minimum add a maintainer note explaining the two-layer
  design. Alternatively, if `JavaObjectRef` is only used for the sealed marker and never as a bound
  in user-facing APIs, `AsJObject` could become the single trait with the sealed story.
- First-pass comparison: New — the first audit did not flag the dual-trait hierarchy.
- Verification: `just check`.
- Links: None.

### Finding: `refs.rs` `From<&Ref> for JavaValue` impls at module bottom

- Status: Discovered
- Area: `src/refs.rs`
- Kind: Move
- Independent observation: The bottom of `src/refs.rs` (lines 367–383) has three `From` impls
  converting `&LocalRef<K>`, `&GlobalRef<K>`, and `&BorrowedLocalRef<K>` into `JavaValue`. These
  appear after the `#[cfg(test)] mod tests` block. These implementations couple the reference module
  to the value module. They also sit after the test module, which is unusual Rust file organization.
- Why it matters: Having trait impls after the `tests` module is surprising to readers scanning the
  file. The coupling between refs and value types is reasonable, but the placement is easy to miss.
- Proposed cleanup: Move the `From` impls above the `#[cfg(test)]` block to follow standard Rust
  file layout (impls before tests). The cross-module coupling is acceptable since `JavaValue` is a
  core type.
- First-pass comparison: New — the first audit did not flag this.
- Verification: `just check`.
- Links: None.

### Finding: `jni.rs` public types expose raw JNI without re-export gating

- Status: Discovered
- Area: `src/jni.rs`, `src/lib.rs`
- Kind: Document
- Independent observation: `src/jni.rs` is declared `pub mod jni` in `lib.rs` (line 18) without
  `cfg(target_os = "android")`, meaning all JNI type definitions (`jobject`, `jclass`, `jvalue`,
  `JavaVM`, `JNIEnv`, etc.) are public on all platforms. This is consistent with `value.rs`,
  `signature.rs`, and `modifiers.rs` also being platform-unconditional. The function pointer type
  aliases (`AttachCurrentThread`, `FindClass`, etc.) are `pub(crate)` so they stay internal.
- Why it matters: The decision to make raw JNI types platform-unconditional is fine for the use case
  (compile-checking on host), but it is undocumented. A reader might wonder whether these should be
  android-gated like `env`, `vm`, and `refs`.
- Proposed cleanup: Add a short maintainer note in `lib.rs` or `jni.rs` explaining that JNI type
  definitions are kept platform-unconditional so `value.rs`, `signature.rs`, and host-compiled code
  can reference them. The function-pointer types and slot constants are already crate-internal.
- First-pass comparison: New — the first audit flagged re-export cleanup but not the unconditional
  platform gating rationale.
- Verification: No behavior change.
- Links: `CLEANUP_AUDIT.md` finding "top-level exports mix normal Java work with raw internals".

### Finding: `ENV_FATAL_ERROR` constant lacks a corresponding function-pointer type alias

- Status: Discovered
- Area: `src/jni.rs`, `src/art/runnable_thread.rs`
- Kind: Document
- Independent observation: `ENV_FATAL_ERROR` (line 251, slot 18) is defined as a `pub(crate)` constant
  and IS used in `src/art/runnable_thread.rs` to resolve the `FatalError` function pointer for ART
  thread transition code generation. However, unlike every other ENV slot constant, it has no
  corresponding function-pointer type alias (e.g., `type FatalError = ...`). The callsite reads it
  as a raw `*const c_void` instead of a typed function pointer.
- Why it matters: The missing type alias is a minor inconsistency in the otherwise regular JNI
  constant/type table. It makes the `FatalError` slot harder to audit for correctness since there is
  no function signature to cross-check.
- Proposed cleanup: Add a `pub(crate) type FatalError = unsafe extern "C" fn(*mut JNIEnv, *const c_char);`
  type alias matching the JNI spec. This is low priority since the callsite only needs the raw
  pointer address.
- First-pass comparison: New — the first audit did not flag this.
- Verification: `just check`.
- Links: None.

### Finding: `metadata.rs` re-lookups `java/lang/Class` per reflection call

- Status: Discovered
- Area: `src/metadata.rs`
- Kind: Simplify
- Independent observation: Functions like `class_descriptor`, `class_loader`, `class_superclass`,
  `call_class_object_array_method`, and the reflection helpers each call
  `env.find_class("java/lang/Class")` independently. The `declared_methods` flow, for example, calls
  `find_class("java/lang/Class")` once for getting the method array and then again through
  `method_metadata_from_reflection` → `get_object_class(reflected)` for each reflected method
  (which returns the executable class, not `java/lang/Class` directly, but the pattern is similar).
- Why it matters: Each `find_class` call goes through JNI, CString allocation, and exception
  checking. For bulk metadata operations (like `enumerate_methods` over many classes), this repeated
  lookup adds avoidable overhead.
- Proposed cleanup: Thread a looked-up `ClassRef` for `java/lang/Class` through the metadata
  helpers, or cache it in the `Env`/lookup session. This is primarily a performance concern and may
  be deferred if metadata calls are not performance-critical.
- First-pass comparison: New — the first audit reviewed metadata but did not flag repeated
  `find_class` lookups.
- Verification: No behavior change; `just test all` for correctness.
- Links: None.

### Finding: `metadata.rs` has large crate-internal helper surface that mixes concerns

- Status: Discovered
- Area: `src/metadata.rs`
- Kind: Move | Document
- Independent observation: `metadata.rs` is 891 lines and contains: public metadata types
  (`JavaClassMetadata`, `JavaMethodMetadata`, `JavaFieldMetadata`, `JavaMethodQueryGroup`,
  `JavaMethodQueryClass`), crate-internal reflection helpers (`class_metadata`, `declared_methods`,
  `visible_methods`, `declared_fields`, `visible_fields`), method query parsing and glob matching
  (`parse_method_query`, `glob_matches`, `is_platform_class`, `normalize_case`), descriptor/name
  conversion (`class_name_to_descriptor`, `class_name_from_descriptor`), and JNI reflection plumbing
  (`call_string`, `call_object`, `call_int`, `call_class_array`, `object_array_elements`).
  The glob matcher and query parser are host-testable pure functions. The JNI reflection plumbing is
  environment-dependent helper code. The descriptor converters are used across the crate.
- Why it matters: Having all these concerns in one file makes it harder to navigate and increases the
  chance of unrelated changes touching the same file. The glob matcher and query parser in particular
  are self-contained and host-testable.
- Proposed cleanup: Consider splitting query parsing/glob matching into a submodule or separate file.
  The descriptor converters could also be moved closer to `signature.rs`. However, this is primarily
  an organizational preference and the current shape works — defer unless the file continues growing.
- First-pass comparison: New — the first audit noted that metadata was "reviewed with no additional
  findings."
- Verification: No behavior change if only moving code.
- Links: None.

### Finding: modifier constants use long flat re-export list from crate root

- Status: Discovered
- Area: `src/modifiers.rs`, `src/lib.rs`
- Kind: Other
- Independent observation: `lib.rs` lines 48–51 re-export all 12 `ACC_*` constants individually.
  This creates a long import list at the crate root for what is effectively a set of JNI modifier
  bitflags. The constants are typed as `jni::jint` (i32), matching Java's `Modifier` class.
- Why it matters: A `bitflags!` or newtype wrapper would provide `contains()`, `intersects()`, and
  `Display` for free and collapse the 12 exports into one type. But the current flat constants are
  simple and match the JNI convention.
- Proposed cleanup: Same conclusion as first audit: keep for now, reconsider if docs need a cleaner
  story.
- First-pass comparison: Overlapping unresolved — `CLEANUP_AUDIT.md` records this as "Deferred" with
  the same reasoning.
- Verification: N/A.
- Links: `CLEANUP_AUDIT.md` finding "modifier constants remain a bare JNI bitmask surface".

### Finding: `JavaValue::Object` variant is `#[doc(hidden)]` but structurally public

- Status: Discovered
- Area: `src/value.rs`
- Kind: Document
- Independent observation: `JavaValue::Object(RawJavaObject)` is annotated `#[doc(hidden)]`, keeping
  it out of generated docs, but since the enum is `pub` and non-exhaustive patterns are not enforced,
  external code can still construct and match this variant. `RawJavaObject` itself has a public
  `unsafe fn from_raw_jobject()` constructor, so the entire raw-reference pathway is accessible.
- Why it matters: The `#[doc(hidden)]` is the only guardrail. A maintainer note explaining why the
  variant is structurally visible but discouraged would clarify intent.
- Proposed cleanup: Add a short maintainer comment near `#[doc(hidden)]` explaining the design
  choice.
- First-pass comparison: Overlapping unresolved — `CLEANUP_AUDIT.md` records this as "Discovered"
  with the same proposed fix.
- Verification: No behavior change.
- Links: `CLEANUP_AUDIT.md` finding "`JavaValue::Object` is hidden from docs but structurally
  public".

## High-Level Java Facade

Files: `src/java/`, plus `src/replacement/api.rs` where it is part of the public Java facade.

Findings:

_Not reviewed yet._

## ART Internals

Files: `src/art/`, plus `src/runtime.rs` and `src/vm.rs` where runtime discovery or ART access is
involved.

Findings:

_Not reviewed yet._

## Replacement Facade And Backend

Files: `src/replacement/`, `src/art/replacement.rs`, and replacement entry points in
`src/java/wrapper.rs`.

Findings:

_Not reviewed yet._

## Harnesses, Fixtures, And Examples

Files: `src/app_process_test.rs`, `src/app_process_test/`, `src/apk_perform_test.rs`,
`src/bin/art_test.rs`, `examples/`, `test-fixtures/`, `justfile`.

Findings:

_Not reviewed yet._

## Behavior And Status Docs

Files: `ROADMAP.md`, `CURRENT_BEHAVIOR.md`, `FEATURE_PROGRESS.md`, `FINALIZATION_PLAN.md`,
`DOCUMENTATION_PASS.md`, `CLEANUP_AUDIT.md`, `HARDENING_AUDIT.md`.

Findings:

_Not reviewed yet._

## Cross-Family Dependencies

Use this section when one finding cannot be implemented cleanly within a single module family.

_None recorded yet._

## Finding Template

```md
### Finding: short title

- Status: Discovered | Fixed | Deferred | Rejected | Moved To Hardening | Moved To Documentation
- Area: module or file path
- Kind: Delete | Merge | Move | Rename | Simplify | Document | Reject Previous | Other
- Independent observation:
- Why it matters:
- Proposed cleanup:
- First-pass comparison: New | Previously fixed | Previously rejected | Overlapping unresolved | Needs reconciliation
- Verification:
- Links:
```
