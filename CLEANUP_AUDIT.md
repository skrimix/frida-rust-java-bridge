# Cleanup Audit

This file is the discovery notebook and implementation tracker for the cleanup pass.

Cleanup means making the crate easier to understand, smaller where possible, and better organized.
It is not a feature sprint. It is also not an API freeze exercise: this crate is pre-user, so exposed
Rust names and module boundaries may change when the result is clearer.

## Process

Use two phases.

### Phase 1: Discovery And Documentation

Read each module family and record findings before changing code. Prefer concrete notes with file
paths, why the shape is costly, and what kind of cleanup seems likely.

Classify each finding as one of:

- Delete: unused code, constants, wrappers, aliases, or helpers that no longer pay rent.
- Merge: duplicate concepts that should become one concept.
- Move: code living in the wrong module or at the wrong abstraction level.
- Rename: names that make users learn internal vocabulary or remember too many concepts.
- Simplify: flow that can be made more direct without losing safety or capabilities.
- Document: internal comments or maintainer notes are missing, misleading, or placed too far from the
  code they explain.
- Other: any other cleanup findings that do not fit into the above categories.

### Phase 2: Cleanup Implementation

Apply bounded patches. After each sprint, update the relevant finding with one of:

- Fixed: code changed and verification noted.
- Deferred: still wanted, with a reason.
- Rejected: intentionally kept, with the design reason.

## Cleanup Rules

- Prefer one user-facing concept over several near-synonyms.
- Keep raw JNI and ART concepts out of the high-level `java` facade unless the API is explicitly raw.
- Remove wrapper types and traits that only forward to another wrapper without adding ownership,
  lifetime, safety, or ergonomic value.
- Move code to the module that owns its reason for existing. For example, not runnable-thread related
  shared arm64 code should not live in `src/art/runnable_thread/arm64.rs`.
- Keep Android and ART capability reporting honest. A cleanup should not convert a clear unsupported
  error into fallback guessing.
- Keep tests near the behavior they protect. Do not use runtime harness tests for host-testable
  parser or formatting logic.
- "Look for" lists are not exhaustive. Use your best judgement to find issues.
- Do not combine unrelated style cleanup with safety fixes; link to `HARDENING_AUDIT.md` instead.
- Public API documentation findings belong in `DOCUMENTATION_PASS.md`. Use `Document` here for
  internal comments and maintainer-facing notes.

## Module Checklist

Use this checklist during discovery. Add notes under the section where the problem belongs, even if
the eventual fix touches multiple modules.

### Public Crate Shape

Files: `src/lib.rs`, `src/error.rs`, `src/android.rs`, `src/runtime.rs`, `src/vm.rs`.

Look for:

- Re-exports that make users learn internal module names.
- Error variants that are too internal, too vague, or duplicated.
- Helpers whose names imply broader runtime support than Android ART.

Findings:

### Finding: top-level exports mix normal Java work with raw internals

- Status: Fixed
- Area: `src/lib.rs`, `src/jni.rs`, `src/refs.rs`, `src/env/`, `src/vm.rs`
- Kind: Move | Document
- Why it mattered: The crate root exported `Java`, wrapper types, raw JNI definitions, low-level
  reference wrappers, `Env`, `Vm`, metadata, and modifier constants at the same level. This makes a
  new user learn internal concepts before the high-level `Java::perform()` / `use_class()` path is
  clear, and makes the final docs harder to keep within the intended concept budget.
- Cleanup: Removed crate-root re-exports for `Env`, `AttachedEnv`, method/field IDs and kinds,
  `Vm`, `RawJavaObject`, and `JavaRawReturn`. Kept the high-level Java facade, common returns,
  errors, capabilities, signatures, `JavaValue`, and replacement facade types easy to import from
  the crate root; raw JNI/VM/reference types remain available through their owning modules.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`.
- Links: `DOCUMENTATION_PASS.md` concept budget; `HARDENING_AUDIT.md` raw-handle finding.

### Finding: capability support has duplicate reason accessors

- Status: Fixed
- Area: `src/runtime.rs`
- Kind: Delete | Rename
- Why it mattered: `FeatureSupport::reason()` and `FeatureSupport::unsupported_reason()` returned the
  same information. The duplicate names made callers choose between equivalent API spellings and
  weakened the binary supported/unsupported vocabulary used in roadmap docs.
- Cleanup: Removed `FeatureSupport::reason()` and kept `unsupported_reason()` as the single
  supported/unsupported explanation accessor. Updated internal harness logging and unit assertions.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `just test all`; `just
  apk-perform-test all`.
- Links: `FEATURE_PROGRESS.md` capability rows.

### Finding: root `java_args!` docs teach hook internals first

- Status: Fixed
- Area: `src/lib.rs`
- Kind: Document | Move
- Why it mattered: The only crate-root macro described itself through "raw descriptor and
  original-call helpers" and "long hook original-call lists". That is accurate internally, but it
  makes the first public macro sound replacement-specific instead of a general explicit Java
  argument builder.
- Cleanup: Rewrote the macro docs around long explicit Java argument lists for method calls, with no
  replacement- or descriptor-first framing.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `just unit-test-build` for existing
  `java_args!` unit coverage.
- Links: `DOCUMENTATION_PASS.md` public API doc rules.

### Safe JNI Environment

Files: `src/env/`, `src/jni.rs`, `src/refs.rs`, `src/value.rs`, `src/signature.rs`,
`src/metadata.rs`, `src/modifiers.rs`.

Look for:

- Duplicated argument, value, or reference conversion paths.
- Public raw-handle escape hatches that should be unsafe or crate-private.
- Vtable helpers or constants that are unused or belong closer to ART/JNI internals.
- Descriptor and signature helpers with names that are hard for non-JNI users to follow.
- Macros that hide simple control flow or create too much special syntax.

Findings:

### Finding: raw null references have two public value spellings

- Status: Fixed
- Area: `src/value.rs`, `src/java/args.rs`, `src/replacement/api.rs`
- Kind: Merge | Document
- Why it mattered: Reference arguments could be represented as `JavaValue::Null` or as
  `JavaValue::Object(RawJavaObject)` containing a null JNI handle through
  `JavaValue::object_raw(ptr::null_mut())`. Both validate as reference arguments, but they report
  different type names (`null` versus `object`) and make raw-reference plumbing harder to explain.
- Cleanup: `JavaValue::object_raw(ptr::null_mut())` now normalizes to `JavaValue::Null`, and the raw
  constructor docs point callers to `JavaValue::Null` as the ordinary null spelling.
- Verification: focused `JavaValue` unit assertion; `just unit-test-build`; `cargo ndk -t
  arm64-v8a clippy --all-features`; `just test all`.
- Links: `HARDENING_AUDIT.md` raw JNI/reference boundary finding.

### Finding: primitive array APIs collapse element identity into `ArrayRef`

- Status: Rejected
- Area: `src/env/arrays.rs`, `src/refs.rs`
- Kind: Rename | Simplify
- Why it matters: `new_int_array`, `new_boolean_array`, and the other primitive constructors all
  return the same `ArrayRef`, while region getters and setters accept any object-like reference.
  This keeps the low-level surface compact, but it means users and maintainers must remember the
  intended primitive element type outside the type name once the array is returned.
- Decision: Kept this as an intentionally raw JNI-style surface and documented near the primitive
  array API table that primitive element identity is caller-tracked through the chosen accessor.
  Typed primitive wrappers would add names without strengthening ownership or element-kind
  guarantees in this low-level layer.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `just unit-test-build`.
- Links: `HARDENING_AUDIT.md` primitive array region validation finding.

### Finding: macro-generated primitive Env methods are hard to audit locally

- Status: Fixed
- Area: `src/env/macros.rs`, `src/env/calls.rs`, `src/env/fields.rs`, `src/env/arrays.rs`
- Kind: Simplify | Document
- Why it matters: The primitive call, field, and array methods are generated from large macros that
  mix public method names, JNI slots, raw function pointer types, conversion closures, and operation
  labels. The generated code is regular, but reviewing exception handling or slot correctness
  requires expanding the macro mentally across many cases.
- Cleanup: Kept the macro approach as the smallest surface and added a local maintainer note that
  makes the slot/function-type/type-validation/exception-check invariants explicit.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `just unit-test-build`.
- Links: `HARDENING_AUDIT.md` exception-state and primitive-array findings.

Reviewed during low-level JNI sprint: `src/signature.rs`, `src/metadata.rs`, and
`src/modifiers.rs` have focused host-testable helpers and no additional cleanup findings beyond
the raw/reference surface notes above.

### High-Level Java Facade

Files: `src/java/`.

Look for:

- Too many entry points for the same user task.
- Wrapper, object, class, method, and field concepts that can be consolidated.
- Names or docs that leak loader, descriptor, or JNI vocabulary into ordinary usage.
- Internal dispatch flows that can be made linear or moved behind selected handles.
- Cache ownership that is harder to explain than the behavior requires.

Findings:

### Finding: method and overload selection have too many public spellings

- Status: Fixed
- Area: `src/java/wrapper.rs`
- Kind: Merge | Rename
- Why it mattered: `JavaClass` exposed selector traits plus separate static/instance method and
  overload spellings, so users had to learn an API matrix instead of one method-group story.
- Cleanup: Replaced the public selector matrix with Frida-like method groups:
  `method("name")`, `.overload(["Type"])`, `call_with`, and `replace_with`. Static-vs-instance now
  lives on the selected overload instead of in public selector names.
- Verification: `just check`; `just test all`.
- Links: `DOCUMENTATION_PASS.md` Java facade docs.

### Finding: class-level replacement hides static versus instance selection

- Status: Fixed
- Area: `src/java/wrapper.rs`, `src/replacement/api.rs`
- Kind: Simplify | Rename
- Why it mattered: Class-level replacement already selected from a combined static/instance method
  set, while ordinary calls used separate static/instance selection paths.
- Cleanup: Calls and hooks now share the same method-group selection model. `replace` and
  `replace_with` are convenience layers over `method("name")` and selected `JavaMethod::replace`.
- Verification: `just check`; `just test all`.
- Links: `HARDENING_AUDIT.md` replacement lifecycle findings.

### Finding: `Java` and `JavaScope` duplicate forwarding surfaces

- Status: Fixed
- Area: `src/java/handle.rs`
- Kind: Simplify
- Why it matters: `JavaScope` repeats many `Java` methods with small attachment-aware differences,
  including loader selection, class lookup, array creation, enumeration, and scheduling helpers.
  This is understandable for ergonomics, but duplicated bodies increase drift risk around loader
  behavior and error semantics.
- Cleanup: Kept the ergonomic `JavaScope` surface, but centralized the duplicate wrapper lookup,
  system class-loader lookup, class-loader object wrapping, and boolean-array conversion helpers.
  `JavaScope` still reuses its attached `Env` for attached variants while sharing the same behavior
  as `Java`.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`.
- Links: `CURRENT_BEHAVIOR.md` app-loader and `perform()` sections.

### Finding: wrapper call traits are public but effectively sealed

- Status: Fixed
- Area: `src/lib.rs`, `src/java/mod.rs`, `src/java/args.rs`, `src/java/wrapper.rs`
- Kind: Simplify | Document
- Why it mattered: `IntoJavaCallArgs` and `IntoJavaFieldValue` are public facade traits, but their
  required methods mention `PreparedJavaArgValues` / `PreparedJavaFieldValue`, and
  `IntoJavaCallArgs` inherits from crate-private `IntoJavaDispatchArgs`. The crate root currently
  allows `private_bounds` and `private_interfaces`, so users see traits that look implementable even
  though they are effectively sealed implementation plumbing.
- Cleanup: Documented `IntoJavaCallArgs` as sealed through its private dispatch supertrait, sealed
  `IntoJavaFieldValue`, and documented the prepared/dispatch types as internal plumbing that are
  public only because sealed trait methods mention them.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`.
- Links: `HARDENING_AUDIT.md` raw JNI/reference boundary finding.

### Finding: stale descriptor helpers overlap with selected handles and raw class APIs

- Status: Fixed
- Area: `src/java/wrapper.rs`
- Kind: Delete | Move | Simplify
- Why it mattered: `JavaClass::new_object_raw`, `call_raw`, `call_static_raw`, `get_static_field_raw`,
  and their `ensure_*` helpers remained crate-private with `#[allow(dead_code)]` after the method and
  field selector rework. They preserve a descriptor-oriented wrapper lane beside selected
  `JavaMethod` / `JavaField` handles and `java::raw::Class`, making it less clear which layer owns
  raw descriptor calls.
- Cleanup: Migrated app-process and APK harness calls to selected constructor/method handles, while
  leaving descriptor-level checks on `java::raw::Class`. Deleted the dead wrapper raw helpers and
  their `ensure_*` validation helpers.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`; `just apk-perform-test
  all`.
- Links: `CLEANUP_AUDIT.md` finding "method and overload selection have too many public spellings".

### Finding: bound method dispatch reports object-bound failures as instance failures

- Status: Fixed
- Area: `src/java/wrapper.rs`, `src/error.rs`
- Kind: Rename | Document
- Why it matters: `JavaBoundMethodGroup::call()` dispatches over both visible instance and static
  methods because an object-bound wrapper can call either, but `MethodDispatchTarget::BoundMethod`
  currently formats no-compatible-overload errors through the `instance` kind. That makes failure
  messages imply a narrower search than the bound dispatch actually performed.
- Cleanup: Object-bound no-compatible-overload errors now report `kind: "method"` while candidate
  entries continue to name exact `instance` or `static` overloads.
- Verification: focused wrapper dispatch unit coverage; `cargo ndk -t arm64-v8a clippy
  --all-features`; `cargo ndk -t arm64-v8a build --example frida_js_ergonomics_probe
  --all-features`; `just test all`.
- Links: `CURRENT_BEHAVIOR.md` wrapper object helper notes.

### Finding: metadata accessors sit beside selected-handle accessors

- Status: Fixed
- Area: `src/java/wrapper.rs`, `src/metadata.rs`
- Kind: Rename | Move | Document
- Why it matters: `JavaClass::methods(name)` / `fields(name)` return metadata lists, while
  `method(name)` / `field(name)` return selected facade handles. The names are close enough that
  final docs may need to explain an extra distinction in the most common wrapper section.
- Cleanup: Removed `JavaClass::methods(name)` and `JavaClass::fields(name)` with no compatibility
  aliases. Named method metadata is available through `JavaClass::method(name)?.overloads()`, and
  field metadata stays behind declared metadata lists or selected `JavaField::metadata()`.
- Verification: updated app-process wrapper metadata check; `cargo ndk -t arm64-v8a clippy
  --all-features`; `cargo ndk -t arm64-v8a build --example frida_js_ergonomics_probe
  --all-features`; `just test all`.
- Links: `DOCUMENTATION_PASS.md` Java facade docs.

### ART Internals

Files: `src/art/`.

Look for:

- Layout, symbol, and version probes duplicated across backend modules.
- Internal helpers that mix capability probing, mutation, and runtime action.
- Unsupported-feature reasons that are built ad hoc instead of consistently reported.
- Direct ART mutation paths that should have narrower module visibility.

Findings:

### Finding: ART module root owns too many backend details

- Status: Fixed
- Area: `src/art/mod.rs`, `src/art/backend.rs`, `src/art/support.rs`
- Kind: Move | Simplify
- Why it matters: `src/art/mod.rs` currently holds feature names, ART symbol names, function
  typedefs, process-global replacement state, constants for access flags/layout probing, and the
  backend struct shape. That makes every ART submodule look coupled to one large namespace, even
  when a constant or symbol only belongs to deoptimization, enumeration, runnable-thread transition,
  or replacement.
- Cleanup: Slimmed `src/art/mod.rs` down to module declarations and crate-visible re-exports. Moved
  feature labels into `src/art/features.rs`, ART symbol names into `src/art/symbols.rs`, backend
  typedefs/enums and `ArtBackend` into `src/art/backend.rs`, layout structs/constants into
  `src/art/layout.rs`, enumeration processors into `src/art/enumeration.rs`, replacement globals and
  guard state into `src/art/replacement.rs`, and JDWP deoptimization globals into
  `src/art/deoptimization.rs`. Touched modules now import their ART dependencies directly instead
  of relying on the root namespace.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `just unit-test-build`; `cargo ndk
  -t arm64-v8a build --example frida_js_ergonomics_probe --all-features`; `just test all`.
- Links: `HARDENING_AUDIT.md` ART layout and mutation inventory.

### Finding: runtime layout probing has split but overlapping flows

- Status: Discovered
- Area: `src/art/support.rs`, `src/art/layout.rs`, `src/art/backend.rs`
- Kind: Merge | Simplify
- Why it matters: runtime field discovery is implemented once for general enumeration/deoptimization
  and again for method replacement plus trampoline discovery. Both flows scan the JavaVM anchor,
  derive heap/thread-list/class-linker/intern-table offsets, and produce similar unsupported
  reasons, but only the replacement path threads memory-range validation and candidate failures
  through the scan.
- Proposed cleanup: Factor the common runtime-field candidate scan into one helper that yields
  candidate layouts and keeps feature-specific validation, such as trampoline discovery, as a
  caller-provided step. Preserve the existing fail-closed unsupported reasons.
- Sprint note: intentionally left for a later cleanup/hardening sprint; this root split only moved
  ownership and imports.
- Verification: existing `src/art/tests.rs` runtime-layout and trampoline tests; `cargo ndk -t
  arm64-v8a clippy --all-features`.
- Links: `HARDENING_AUDIT.md` ART layout readability finding.

### Finding: fake ART handle-scope helpers need a clearer ownership home

- Status: Discovered
- Area: `src/art/enumeration.rs`
- Kind: Move | Document
- Why it matters: heap enumeration via `Heap::GetInstances` builds a fake
  `VariableSizedHandleScope`, mutates the ART thread's top handle-scope pointer, and later restores
  it. The code is currently adjacent to enumeration processors, but its reason for existing is ART
  handle-scope construction and teardown, not heap filtering.
- Proposed cleanup: Either move the fake handle-scope/vector helpers into a small internal
  handle-scope section/module with a maintainer note, or add a local comment documenting the ART
  layout assumptions and teardown contract before later hardening work touches it.
- Sprint note: intentionally left for a later cleanup/hardening sprint; this root split did not
  change heap enumeration behavior.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; app-process heap enumeration
  coverage if behavior changes.
- Links: `HARDENING_AUDIT.md` ART layout and mutation inventory.

### Replacement Facade And Backend

Files: `src/replacement/`.

Look for:

- Public callback concepts that expose trampoline or JNI-frame implementation details.
- Duplicated original-call argument builders or return conversion paths.
- Backend adapters whose boundaries are unclear.
- Lifecycle names that make the guard model harder to remember.
- Internal raw replacement pieces that can be hidden or renamed after facade stabilization.

Findings:

### Finding: raw hook return alias is a public user concept

- Status: Fixed
- Area: `src/replacement/api.rs`, `src/java/returns.rs`
- Kind: Rename | Move | Document
- Why it mattered: `JavaHookReturn` was publicly described as the raw-reference specialization of
  `JavaReturn`, and several safe original-call helpers return it directly. Normal replacement users
  should think in terms of returning `()`, primitives, strings, objects, arrays, or typed
  original-call results, not in terms of raw reference lanes.
- Cleanup: Kept `JavaHookReturn` for explicit replacement callback returns, but removed the safe
  original-call helpers that returned it directly. `call_original_current()` and
  `call_original_return()` now extract through typed `FromJavaHookReturn`; raw original-call returns
  require `unsafe call_original_raw()`.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`.
- Links: `HARDENING_AUDIT.md` callback-local raw return finding; `DOCUMENTATION_PASS.md`
  replacement docs.

### Finding: replacement lifecycle helpers expose backend diagnostics as first-class API

- Status: Fixed
- Area: `src/replacement/api.rs`
- Kind: Document | Simplify
- Why it matters: `JavaHookGuard::debug_summary()` exposes backend-oriented diagnostics next to the
  lifecycle methods users need (`revert`, `on_error`, `last_error`). This may be useful while
  stabilizing ART replacement, but it teaches backend state as part of the public guard model.
- Cleanup: Removed `JavaHookGuard::debug_summary()` and the closure-level forwarding path from the
  public replacement facade. Kept the ART backend diagnostic summary available where host ART tests
  exercise cloned-method formatting directly.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`; `just apk-perform-test
  all`.
- Links: `DOCUMENTATION_PASS.md` replacement docs.

### Harnesses, Fixtures, And Examples

Files: `src/app_process_test.rs`, `src/app_process_test/`, `src/apk_perform_test.rs`,
`src/bin/art_test.rs`, `examples/`, `test-fixtures/`.

Look for:

- Tests asserting implementation detail instead of behavior.
- Harness helpers duplicated across app-process, APK, and native bootstrap paths.
- Example names or snippets that teach old API shapes.

Findings:

### Finding: live harness asserts replacement backend debug strings

- Status: Fixed
- Area: `src/app_process_test/assertions.rs`, `src/app_process_test/replacement_checks.rs`,
  `src/app_process_test/replacement_lifecycle.rs`
- Kind: Simplify | Document
- Why it matters: the app-process replacement harness checks `JavaHookGuard::debug_summary()` for
  strings such as `backend=clone-active`, `original_patched=`, and `clone_patched=`. That confirms
  the current backend during stabilization, but it makes live behavior tests depend on diagnostic
  text and reinforces `debug_summary()` as user-facing API.
- Cleanup: Removed the app-process backend-summary helpers and all live harness assertions on cloned
  method diagnostic strings. The live harness now relies on the existing replacement, restore,
  duplicate-registration, GC replay, stack-visitor, callback-drain, and error-observation checks.
  ART diagnostic summary formatting remains covered by host ART tests.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; `cargo ndk -t arm64-v8a build
  --example frida_js_ergonomics_probe --all-features`; `just test all`; `just apk-perform-test
  all`.
- Links: `CLEANUP_AUDIT.md` finding "replacement lifecycle helpers expose backend diagnostics as
  first-class API".

### Finding: ergonomics probe intentionally preserves old API pressure

- Status: Fixed
- Area: `examples/frida_js_ergonomics_probe.rs`
- Kind: Document | Rename
- Why it mattered: the probe compiled representative Frida JS snippets against transitional Rust
  selector names, which could accidentally keep compatibility aliases alive.
- Cleanup: Updated the probe to the preferred `replace_with` exact-overload convenience and removed
  references to the removed method/static overload selector names.
- Verification: `just check`; `cargo ndk -t arm64-v8a build --example frida_js_ergonomics_probe
  --all-features`.
- Links: `CLEANUP_AUDIT.md` finding "method and overload selection have too many public spellings".

## Cross-Cutting Finding Template

Use this shape when adding findings:

```md
### Finding: short title

- Status: Discovered | Fixed | Deferred | Rejected
- Area: module or file path
- Kind: Delete | Merge | Move | Rename | Simplify | Document | Other
- Why it matters:
- Proposed cleanup:
- Verification:
- Links:
```
