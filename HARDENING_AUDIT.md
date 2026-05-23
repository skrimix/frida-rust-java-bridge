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

- _None recorded yet._

### Hidden Unsafety

Look at all `unsafe` blocks and any safe functions that call raw JNI/ART helpers.

Questions:

- Does the public boundary expose the required caller guarantee?
- Is each unsafe block close to the invariant that justifies it?
- Are raw handles accepted only from crate-owned wrappers unless the API is unsafe?
- Are architecture assumptions checked before use?

Findings:

- _None recorded yet._

### Threading And Attachment

Look at `src/vm.rs`, `src/java/perform.rs`, `src/java/main_thread.rs`, `src/art/runnable_thread.rs`,
and `src/art/runnable_thread/arm64.rs`.

Questions:

- Can attached env values cross threads?
- Does deferred `perform()` preserve loader scope and callback lifetime correctly?
- Does main-thread scheduling behave predictably when the main looper is absent?
- Are runnable-thread and architecture-specific pieces separated cleanly enough to audit?

Findings:

- _None recorded yet._

### Exceptions And JNI Call State

Look at `src/env/calls.rs`, `src/env/fields.rs`, `src/env/members.rs`, `src/env/exceptions.rs`,
`src/java/dispatch.rs`, and replacement original-call paths.

Questions:

- Does each JNI call path handle pending exceptions consistently?
- Can a pending exception poison later helper calls?
- Are Java exceptions surfaced as Rust errors where users expect them?
- Are diagnostic calls like `toString()` careful about exception state?

Findings:

- _None recorded yet._

### ART Layouts, Symbols, And Mutation

Look at `src/art/layout.rs`, `src/art/support.rs`, `src/art/backend.rs`, `src/art/replacement.rs`,
`src/art/enumeration.rs`, and `src/art/deoptimization.rs`.

Questions:

- Does every layout probe validate enough before reading or writing?
- Are unsupported Android versions, ABIs, or symbol sets reported with clear reasons?
- Are mutation operations isolated from pure capability probing?
- Can failed restore or partial install leave ART state inconsistent?

Findings:

- _None recorded yet._

### Replacement Callback Lifecycle

Look at `src/replacement/closure.rs`, `src/replacement/trampoline.rs`, `src/replacement/original.rs`,
`src/replacement/original_call.rs`, and `src/replacement/backend.rs`.

Questions:

- Are panics contained before returning to Java?
- Is callback-local state removed on all exit paths?
- Can original-call handles outlive the active replacement/thread scope?
- Are wrong return kinds and assignability failures handled before JNI sees invalid data?

Findings:

- _None recorded yet._

### Test Matrix

Questions:

- Which risky behavior has only compile coverage?
- Which host-testable logic is only exercised through device tests?
- Which app startup behavior requires APK coverage?
- Which native ART bootstrap assumption belongs in `art_test` only?

Findings:

- _None recorded yet._

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
