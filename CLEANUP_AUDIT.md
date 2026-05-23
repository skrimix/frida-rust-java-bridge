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

- Status: Discovered
- Area: `src/lib.rs`, `src/jni.rs`, `src/refs.rs`, `src/env/`, `src/vm.rs`
- Kind: Move | Document
- Why it matters: The crate root exports `Java`, wrapper types, raw JNI definitions, low-level
  reference wrappers, `Env`, `Vm`, metadata, and modifier constants at the same level. This makes a
  new user learn internal concepts before the high-level `Java::perform()` / `use_class()` path is
  clear, and makes the final docs harder to keep within the intended concept budget.
- Proposed cleanup: Keep `Java`, wrapper types, common returns, errors, capabilities, signatures,
  and hook guard types easy to import from the crate root. Move or clearly group raw JNI/reference
  surfaces under an explicitly advanced namespace in docs and exports, or stop re-exporting raw
  modules that are only needed by internal or unsafe callers.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; compile
  `examples/frida_js_ergonomics_probe.rs` if public imports are changed.
- Links: `DOCUMENTATION_PASS.md` concept budget; `HARDENING_AUDIT.md` raw-handle finding.

### Finding: capability support has duplicate reason accessors

- Status: Discovered
- Area: `src/runtime.rs`
- Kind: Delete | Rename
- Why it matters: `FeatureSupport::reason()` and `FeatureSupport::unsupported_reason()` return the
  same information. The duplicate names make callers choose between equivalent API spellings and
  weaken the binary supported/unsupported vocabulary used in roadmap docs.
- Proposed cleanup: Keep one accessor, preferably the one that best matches final documentation
  wording, and update internal/test call sites during the cleanup patch.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`.
- Links: `FEATURE_PROGRESS.md` capability rows.

### Finding: root `java_args!` docs teach hook internals first

- Status: Discovered
- Area: `src/lib.rs`
- Kind: Document | Move
- Why it matters: The only crate-root macro currently describes itself through "raw descriptor and
  original-call helpers" and "long hook original-call lists". That is accurate internally, but it
  makes the first public macro sound replacement-specific instead of a general explicit Java
  argument builder.
- Proposed cleanup: Rewrite the public docs around explicit argument lists for method calls and
  original calls. If the macro remains mostly replacement-oriented after API cleanup, consider moving
  the documentation focus to the replacement module instead of the crate root.
- Verification: Documentation review in the final docs pass; no build needed unless examples are
  added.
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

- Status: Discovered
- Area: `src/value.rs`, `src/java/args.rs`, `src/replacement/api.rs`
- Kind: Merge | Document
- Why it matters: Reference arguments can be represented as `JavaValue::Null` or as
  `JavaValue::Object(RawJavaObject)` containing a null JNI handle through
  `JavaValue::object_raw(ptr::null_mut())`. Both validate as reference arguments, but they report
  different type names (`null` versus `object`) and make raw-reference plumbing harder to explain.
- Proposed cleanup: Choose one primary public spelling for Java null in argument lists, and keep
  raw-null construction documented as an advanced compatibility path only if it remains necessary
  for replacement/original-call internals.
- Verification: Host unit tests for argument validation and error messages; `cargo ndk -t
  arm64-v8a clippy --all-features` if public names or match arms change.
- Links: `HARDENING_AUDIT.md` raw JNI/reference boundary finding.

### Finding: primitive array APIs collapse element identity into `ArrayRef`

- Status: Discovered
- Area: `src/env/arrays.rs`, `src/refs.rs`
- Kind: Rename | Simplify
- Why it matters: `new_int_array`, `new_boolean_array`, and the other primitive constructors all
  return the same `ArrayRef`, while region getters and setters accept any object-like reference.
  This keeps the low-level surface compact, but it means users and maintainers must remember the
  intended primitive element type outside the type name once the array is returned.
- Proposed cleanup: Either keep this as an intentionally raw JNI-style surface and document that
  primitive array element identity is caller-tracked, or introduce lightweight typed aliases/wrappers
  if primitive arrays become a common safe-env user path.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; focused unit coverage for any new
  wrapper/type aliases if introduced.
- Links: `HARDENING_AUDIT.md` primitive array region validation finding.

### Finding: macro-generated primitive Env methods are hard to audit locally

- Status: Discovered
- Area: `src/env/macros.rs`, `src/env/calls.rs`, `src/env/fields.rs`, `src/env/arrays.rs`
- Kind: Simplify | Document
- Why it matters: The primitive call, field, and array methods are generated from large macros that
  mix public method names, JNI slots, raw function pointer types, conversion closures, and operation
  labels. The generated code is regular, but reviewing exception handling or slot correctness
  requires expanding the macro mentally across many cases.
- Proposed cleanup: Keep the macro approach if it remains the smallest surface, but add a local
  maintainer note or table-style structure that makes slot/type/operation invariants easy to review.
  If one family changes for hardening, consider replacing that family with clearer helper tables or
  explicit small functions.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; existing unit tests for primitive
  method/field/array helpers.
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

- Status: Discovered
- Area: `src/java/wrapper.rs`
- Kind: Merge | Rename
- Why it matters: `JavaClass` exposes selector traits plus `method`, `static_method`, `overload`,
  `static_overload`, `method_overload`, `method_overload_by_name`,
  `static_method_overload`, `static_method_overload_by_name`, `call_overload`, and constructor
  variants. Several names describe the same task with different levels of descriptor parsing and
  static/instance specificity, so users must learn the API matrix instead of one clear selection
  story.
- Proposed cleanup: Pick one primary public path for selecting by name and one for selecting by
  argument types, with explicit static/instance/constructor entry points. Keep helper traits or
  parsing variants private unless they materially improve call-site ergonomics.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; compile
  `examples/frida_js_ergonomics_probe.rs`; update app-process call sites if public names change.
- Links: `DOCUMENTATION_PASS.md` Java facade docs.

### Finding: class-level replacement hides static versus instance selection

- Status: Discovered
- Area: `src/java/wrapper.rs`, `src/replacement/api.rs`
- Kind: Simplify | Rename
- Why it matters: `JavaClass::replace` and `replace_overload` merge inherited instance methods and
  declared static methods before selecting a hook target, while ordinary calls use separate
  `method()` and `static_method()` paths. The replacement surface therefore has a different mental
  model from the rest of the facade and can produce ambiguity at the most runtime-sensitive API.
- Proposed cleanup: Prefer replacement through an already selected `JavaMethod`, or split class-level
  convenience helpers into explicit instance/static spellings that mirror method selection.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; app-process replacement harness if
  replacement selection behavior changes.
- Links: `HARDENING_AUDIT.md` replacement lifecycle findings.

### Finding: `Java` and `JavaScope` duplicate forwarding surfaces

- Status: Discovered
- Area: `src/java/handle.rs`
- Kind: Simplify
- Why it matters: `JavaScope` repeats many `Java` methods with small attachment-aware differences,
  including loader selection, class lookup, array creation, enumeration, and scheduling helpers.
  This is understandable for ergonomics, but duplicated bodies increase drift risk around loader
  behavior and error semantics.
- Proposed cleanup: Keep the ergonomic `JavaScope` surface, but centralize shared behavior in
  attached helper functions where practical. Document the few intentional differences, especially
  methods that reuse the current `Env` instead of attaching again.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; app-process tests only if loader or
  `perform()` behavior changes.
- Links: `CURRENT_BEHAVIOR.md` app-loader and `perform()` sections.

### ART Internals

Files: `src/art/`.

Look for:

- Layout, symbol, and version probes duplicated across backend modules.
- Internal helpers that mix capability probing, mutation, and runtime action.
- Unsupported-feature reasons that are built ad hoc instead of consistently reported.
- Direct ART mutation paths that should have narrower module visibility.

Findings:

### Finding: ART module root owns too many backend details

- Status: Discovered
- Area: `src/art/mod.rs`, `src/art/backend.rs`, `src/art/support.rs`
- Kind: Move | Simplify
- Why it matters: `src/art/mod.rs` currently holds feature names, ART symbol names, function
  typedefs, process-global replacement state, constants for access flags/layout probing, and the
  backend struct shape. That makes every ART submodule look coupled to one large namespace, even
  when a constant or symbol only belongs to deoptimization, enumeration, runnable-thread transition,
  or replacement.
- Proposed cleanup: Move feature-local symbol names, typedefs, and constants closer to the module
  that owns the behavior. Keep only genuinely shared ART vocabulary in `mod.rs`, and prefer small
  module-local imports over `use super::*` for areas touched by later cleanup.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; host ART unit tests if constants or
  helper visibility move.
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

- Status: Discovered
- Area: `src/replacement/api.rs`, `src/java/returns.rs`
- Kind: Rename | Move | Document
- Why it matters: `JavaHookReturn` is publicly described as the raw-reference specialization of
  `JavaReturn`, and several safe original-call helpers return it directly. Normal replacement users
  should think in terms of returning `()`, primitives, strings, objects, arrays, or typed
  original-call results, not in terms of raw reference lanes.
- Proposed cleanup: Keep raw hook-return construction available only where needed, but make typed
  helpers the primary public path. Consider renaming or moving raw-return APIs so storing raw
  callback-local references feels visibly advanced.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; replacement app-process harness if
  return APIs change.
- Links: `HARDENING_AUDIT.md` callback-local raw return finding; `DOCUMENTATION_PASS.md`
  replacement docs.

### Finding: replacement lifecycle helpers expose backend diagnostics as first-class API

- Status: Discovered
- Area: `src/replacement/api.rs`
- Kind: Document | Simplify
- Why it matters: `JavaHookGuard::debug_summary()` exposes backend-oriented diagnostics next to the
  lifecycle methods users need (`revert`, `on_error`, `last_error`). This may be useful while
  stabilizing ART replacement, but it teaches backend state as part of the public guard model.
- Proposed cleanup: Decide whether `debug_summary()` is temporary/internal diagnostics or a stable
  public troubleshooting hook. If it remains public, document it as diagnostic-only and keep normal
  lifecycle docs centered on guard ownership and failure observation.
- Verification: Documentation review; no runtime test needed unless the method is moved or removed.
- Links: `DOCUMENTATION_PASS.md` replacement docs.

### Finding: hook-set batch revert stops at the first restore error

- Status: Discovered
- Area: `src/replacement/api.rs`
- Kind: Simplify | Document
- Why it matters: `JavaHookSet::revert_all()` iterates guards in reverse but returns immediately on
  the first restore error. That keeps the first error visible, but remaining hooks are not attempted,
  which is surprising for a batch lifecycle helper.
- Proposed cleanup: Either rename/document the fail-fast behavior clearly, or change the helper in a
  hardening sprint to attempt every revert and report combined restore failures.
- Verification: Unit coverage for batch revert behavior if changed; app-process lifecycle harness
  for real restore behavior.
- Links: `HARDENING_AUDIT.md` replacement lifecycle findings.

### Harnesses, Fixtures, And Examples

Files: `src/app_process_test.rs`, `src/app_process_test/`, `src/apk_perform_test.rs`,
`src/bin/art_test.rs`, `examples/`, `test-fixtures/`.

Look for:

- Tests asserting implementation detail instead of behavior.
- Harness helpers duplicated across app-process, APK, and native bootstrap paths.
- Example names or snippets that teach old API shapes.

Findings:

### Finding: live harness asserts replacement backend debug strings

- Status: Discovered
- Area: `src/app_process_test/assertions.rs`, `src/app_process_test/replacement_checks.rs`,
  `src/app_process_test/replacement_lifecycle.rs`
- Kind: Simplify | Document
- Why it matters: the app-process replacement harness checks `JavaHookGuard::debug_summary()` for
  strings such as `backend=clone-active`, `original_patched=`, and `clone_patched=`. That confirms
  the current backend during stabilization, but it makes live behavior tests depend on diagnostic
  text and reinforces `debug_summary()` as user-facing API.
- Proposed cleanup: Keep live replacement tests focused on observable Java behavior, restore
  behavior, active-callback draining, and explicit unsupported reasons. Move backend-summary
  assertions to host ART diagnostics tests if `debug_summary()` remains, or remove them when the
  diagnostic API is demoted.
- Verification: app-process replacement harness after changing assertions; host ART tests for
  summary formatting if kept.
- Links: `CLEANUP_AUDIT.md` finding "replacement lifecycle helpers expose backend diagnostics as
  first-class API".

### Finding: ergonomics probe intentionally preserves old API pressure

- Status: Discovered
- Area: `examples/frida_js_ergonomics_probe.rs`
- Kind: Document | Rename
- Why it matters: the probe compiles many representative Frida JS snippets against the Rust facade,
  including old or transitional selector names such as `replace_overload`,
  `method_overload_by_name`, and `static_method_overload_by_name`. This is useful pressure during
  cleanup, but after selector cleanup it can accidentally keep compatibility names alive.
- Proposed cleanup: During implementation cleanup, update the probe to the preferred final spellings
  and leave comments only for intentionally retained aliases. Treat compile failures here as API
  design feedback rather than as a requirement to preserve every old name.
- Verification: compile `examples/frida_js_ergonomics_probe.rs`; `cargo ndk -t arm64-v8a clippy
  --all-features` if public selector APIs change.
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
