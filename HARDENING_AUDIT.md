# Hardening Audit

This file is the discovery notebook and implementation tracker for the hardening pass.

Hardening means finding places where the code can be wrong, unsound, misleadingly safe, racy,
version-fragile, or too trusting of ART/JNI behavior. It is broader than Rust `unsafe`: lifetime
shape, thread ownership, exception state, loader identity, callback failure, and runtime capability
reporting all count.

## Process

Use two phases.

### Phase 1: Discovery And Documentation

Read module families with a safety and correctness lens. Record findings before changing code.
Include the expected failure mode, the caller-visible consequence, and the boundary that should own
the guarantee.

Start with the lightweight inventory captured before cleanup implementation, then re-read areas that
cleanup patches touched. Cleanup can move a lifetime, raw-handle, or callback boundary even when it
does not intend to change behavior.

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

Look at `src/refs.rs`, `src/env/references.rs`, `src/java/object.rs`, `src/java/array.rs`,
`src/replacement/api.rs`, and callback-local reference views.

Questions:

- Can a local reference escape its attached scope?
- Can a borrowed hook argument be stored past callback return?
- Are global references clearly owned and released?
- Are null references represented distinctly from non-null objects where behavior requires it?
- Are casts and declared object returns binding references to the right loader/class context?

Findings:

### Finding: callback-local raw returns can escape without a lifetime

- Status: Discovered
- Area: `src/replacement/api.rs`, `src/java/returns.rs`, `src/value.rs`
- Kind: Lifetime | Raw handle
- Failure mode: Safe helpers such as `JavaHookContext::call_original_current()` and
  `JavaHookContext::call_original_return()` can return `JavaHookReturn`, whose object and array
  lanes carry `RawJavaObject` without a Rust lifetime tying the reference to the active replacement
  callback.
- User-visible consequence: A caller can store a raw object/array return after the callback-local
  JNI reference is no longer valid, then later feed it back through unsafe or raw-return APIs and
  observe use-after-lifetime behavior at the JNI/ART boundary.
- Proposed hardening: Make callback-local object/array original-call results lifetime-bound by
  default, or make raw-return extraction/original-call paths explicitly unsafe with a contract that
  they must not escape the callback. Keep typed helpers such as `call_original_object()` and
  `call_original_array()` as the safe path.
- Verification: Unit compile assertions for non-escaping local return types if the type shape
  changes; app-process replacement coverage for object/array original calls.
- Links: `CLEANUP_AUDIT.md` finding "raw hook return alias is a public user concept".

### Finding: global-reference drop can leak when thread attachment fails

- Status: Discovered
- Area: `src/refs.rs`, `src/env/references.rs`, `src/vm.rs`
- Kind: Lifetime | Threading
- Failure mode: `GlobalRef::drop()` attempts to attach the current thread before deleting the JNI
  global reference, but silently skips deletion if attachment fails. Because `GlobalRef` is
  `Send + Sync`, the final drop can happen on any Rust thread that still has the value.
- User-visible consequence: A global JNI reference can remain live for the rest of the process
  without a visible error if the last owner is dropped from a thread or runtime state where
  attachment is unavailable.
- Proposed hardening: Decide whether global references require an explicit close/release path that
  reports deletion failure, a VM-owned cleanup queue, or a documented best-effort drop contract.
  Keep `Drop` non-panicking, but avoid making deletion failure invisible to callers who care.
- Verification: Unit coverage for any explicit release state machine; app-process coverage only if
  deletion behavior changes in live ART.
- Links: `CLEANUP_AUDIT.md` low-level JNI reference findings.

### Hidden Unsafety

Look at all `unsafe` blocks and any safe functions that call raw JNI/ART helpers.

Questions:

- Does the public boundary expose the required caller guarantee?
- Is each unsafe block close to the invariant that justifies it?
- Are raw handles accepted only from crate-owned wrappers unless the API is unsafe?
- Are architecture assumptions checked before use?

Findings:

### Finding: raw JNI/reference surface needs one explicit public boundary

- Status: Discovered
- Area: `src/lib.rs`, `src/jni.rs`, `src/refs.rs`, `src/env/`, `src/vm.rs`, `src/value.rs`
- Kind: Unsafe boundary | Raw handle
- Failure mode: Raw JNI definitions and low-level reference/value types are publicly reachable
  beside high-level Java APIs. Most raw constructors and extractors are marked `unsafe`, but the
  crate does not yet present one cohesive public boundary that tells callers which raw handles may
  be forged, borrowed, retained, or moved across threads.
- User-visible consequence: Advanced callers may combine raw values from the wrong VM, thread,
  callback, or local-reference scope and only discover the mistake as a JNI/ART crash or corrupted
  exception state.
- Proposed hardening: During cleanup, group raw JNI/reference APIs under an explicitly advanced or
  unsafe public surface and audit every raw-handle constructor/extractor for a precise caller
  contract. Keep normal Java object work on safe wrapper APIs.
- Verification: `cargo ndk -t arm64-v8a clippy --all-features`; documentation review for every
  remaining public `unsafe fn` in the raw layer.
- Links: `CLEANUP_AUDIT.md` finding "top-level exports mix normal Java work with raw internals";
  `DOCUMENTATION_PASS.md` low-level JNI docs.

### Finding: method and field IDs are not bound to their declaring class

- Status: Discovered
- Area: `src/env/ids.rs`, `src/env/members.rs`, `src/env/calls.rs`, `src/env/fields.rs`,
  `src/metadata.rs`
- Kind: Unsafe boundary | Raw handle
- Failure mode: `MethodId`, `FieldId`, and public metadata IDs carry kind and descriptor/type
  information, but not the class or VM identity that produced the ID. Safe `Env` call/field helpers
  accept an object or class separately from the ID, so a caller can accidentally combine an ID with
  the wrong receiver/class in a safe call path. After the method/field facade rework,
  `JavaMethodMetadata::id` and `JavaFieldMetadata::id` also remain public raw `jmethodID` /
  `jfieldID` values, so callers can copy opaque IDs out of reflection metadata without any
  declaring-class or VM boundary.
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

### Finding: selected method and field handles accept unchecked receivers

- Status: Discovered
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
- Proposed hardening: Add app-process negative coverage for wrong-receiver method calls and field
  access first. Then either validate receivers with `IsInstanceOf` before JNI access in the safe
  selected-handle path, or move detached unchecked receiver calls behind a clearer advanced or
  unsafe boundary while steering normal users to `JavaObject` / `JavaBoundObject` calls.
- Verification: App-process negative tests for wrong receiver method call, wrong receiver field
  get, and wrong receiver field set; `cargo ndk -t arm64-v8a clippy --all-features`.
- Links: `CLEANUP_AUDIT.md` Java facade findings.

### Threading And Attachment

Look at `src/vm.rs`, `src/java/perform.rs`, `src/java/main_thread.rs`, `src/art/runnable_thread.rs`,
and `src/art/runnable_thread/arm64.rs`.

Questions:

- Can attached env values cross threads?
- Does deferred `perform()` preserve loader scope and callback lifetime correctly?
- Does main-thread scheduling behave predictably when the main looper is absent?
- Are runnable-thread and architecture-specific pieces separated cleanly enough to audit?

Findings:

- Reviewed during public-facade sprint: no issues found in the inspected `JavaScope`, `Env`, and
  callback-local object thread-affinity boundaries. This is not the full hardening pass; re-read the
  complete threading and attachment modules after cleanup changes.

### Exceptions And JNI Call State

Look at `src/env/calls.rs`, `src/env/fields.rs`, `src/env/members.rs`, `src/env/exceptions.rs`,
`src/java/dispatch.rs`, and replacement original-call paths.

Questions:

- Does each JNI call path handle pending exceptions consistently?
- Can a pending exception poison later helper calls?
- Are Java exceptions surfaced as Rust errors where users expect them?
- Are diagnostic calls like `toString()` careful about exception state?

Findings:

Reviewed during low-level JNI sprint: normal `Env` call, field, member lookup, reference, string,
and array helpers check pending Java exceptions after JNI calls that can produce caller-visible
Java failures. Exception summary helpers intentionally clear secondary `Throwable.toString()`
failures and preserve the original exception where the replacement path requires it. Re-read
replacement original-call paths during the replacement lifecycle hardening sprint.

### Finding: empty primitive array regions skip JNI validation

- Status: Discovered
- Area: `src/env/arrays.rs`
- Kind: Exception state
- Failure mode: `get_primitive_array_region()` and `set_primitive_array_region()` return `Ok(())`
  immediately for empty output/input slices without calling JNI. That avoids zero-length JNI calls,
  but also bypasses ART validation of the array reference and start index.
- User-visible consequence: Safe low-level calls can report success for an invalid array reference,
  wrong array kind, or invalid start index when the requested region length is zero, while otherwise
  equivalent non-empty calls would surface a Java exception.
- Proposed hardening: Preserve the fast path only when the API explicitly treats empty regions as
  no-ops, or perform a side-effect-light validation such as `GetArrayLength` before returning. The
  chosen behavior should be documented and covered by host or app-process tests as appropriate.
- Verification: App-process array coverage for null/wrong-kind/empty-region behavior if the safe
  Env surface changes; host tests for any helper-level argument policy.
- Links: `CLEANUP_AUDIT.md` primitive array API finding.

### ART Layouts, Symbols, And Mutation

Look at `src/art/layout.rs`, `src/art/support.rs`, `src/art/backend.rs`, `src/art/replacement.rs`,
`src/art/enumeration.rs`, and `src/art/deoptimization.rs`.

Questions:

- Does every layout probe validate enough before reading or writing?
- Are unsupported Android versions, ABIs, or symbol sets reported with clear reasons?
- Are mutation operations isolated from pure capability probing?
- Can failed restore or partial install leave ART state inconsistent?

Findings:

### Finding: general runtime layout scan reads candidate fields without memory-range validation

- Status: Discovered
- Area: `src/art/support.rs`, `src/art/backend.rs`
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

- Status: Discovered
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

### Replacement Callback Lifecycle

Look at `src/replacement/closure.rs`, `src/replacement/trampoline.rs`, `src/replacement/original.rs`,
`src/replacement/original_call.rs`, and `src/replacement/backend.rs`.

Questions:

- Are panics contained before returning to Java?
- Is callback-local state removed on all exit paths?
- Can original-call handles outlive the active replacement/thread scope?
- Are wrong return kinds and assignability failures handled before JNI sees invalid data?

Findings:

### Finding: hook-set batch revert can leave later guards active after one restore failure

- Status: Discovered
- Area: `src/replacement/api.rs`
- Kind: Callback failure
- Failure mode: `JavaHookSet::revert_all()` returns on the first `JavaHookGuard::revert()` error
  while iterating in reverse order, so older guards that have not yet been visited remain active.
- User-visible consequence: A caller using `JavaHookSet` as a lifecycle owner may believe teardown
  has been attempted for the whole set, while some hooks were never asked to restore after an
  unrelated restore failure.
- Proposed hardening: Attempt every guard restore during batch teardown and return a combined error,
  or rename/document the helper as fail-fast. Prefer the all-attempting behavior if `JavaHookSet`
  remains the public batch lifecycle type.
- Verification: Add focused unit coverage with a fake guard backend if possible; otherwise extend
  app-process replacement lifecycle coverage after implementation.
- Links: `CLEANUP_AUDIT.md` finding "hook-set batch revert stops at the first restore error".

### Test Matrix

Questions:

- Which risky behavior has only compile coverage?
- Which host-testable logic is only exercised through device tests?
- Which app startup behavior requires APK coverage?
- Which native ART bootstrap assumption belongs in `art_test` only?

Findings:

### Finding: batch hook teardown failure has no focused non-device test

- Status: Discovered
- Area: `src/replacement/api.rs`, `src/replacement/closure.rs`
- Kind: Test gap
- Failure mode: the known `JavaHookSet::revert_all()` fail-fast behavior is only documented in the
  audit and exercised indirectly through successful app-process lifecycle cases. There is no narrow
  test that simulates one guard failing to restore and verifies whether later guards are attempted.
- User-visible consequence: a future cleanup could preserve or change batch teardown semantics
  without an immediate test explaining the intended behavior.
- Proposed hardening: When implementing the `JavaHookSet` cleanup/hardening, add a fake or test-only
  guard backend so failure aggregation/fail-fast behavior can be tested without a live ART process.
- Verification: focused host unit test for batch teardown semantics; app-process lifecycle harness
  only for live restore behavior.
- Links: `HARDENING_AUDIT.md` replacement lifecycle finding for `JavaHookSet::revert_all()`.

### Finding: empty primitive array region policy lacks runtime coverage

- Status: Discovered
- Area: `src/env/arrays.rs`, `src/app_process_test/checks.rs`
- Kind: Test gap
- Failure mode: primitive array region tests cover normal non-empty get/set calls, but the
  zero-length get/set fast path identified earlier has no app-process case for null arrays,
  wrong-kind arrays, invalid starts, or an explicitly accepted no-op policy.
- User-visible consequence: the low-level safe Env API can keep reporting success for invalid empty
  regions, or a later fix can change that policy, without a device test documenting the expected JNI
  behavior.
- Proposed hardening: Add targeted app-process checks once the desired empty-region behavior is
  chosen: either validation through `GetArrayLength` or a documented no-op policy for empty slices.
- Verification: app-process low-level JNI array checks.
- Links: `HARDENING_AUDIT.md` finding "empty primitive array regions skip JNI validation".

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
