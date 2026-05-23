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

- _None recorded yet._

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

- _None recorded yet._

### High-Level Java Facade

Files: `src/java/`.

Look for:

- Too many entry points for the same user task.
- Wrapper, object, class, method, and field concepts that can be consolidated.
- Names or docs that leak loader, descriptor, or JNI vocabulary into ordinary usage.
- Internal dispatch flows that can be made linear or moved behind selected handles.
- Cache ownership that is harder to explain than the behavior requires.

Findings:

- _None recorded yet._

### ART Internals

Files: `src/art/`.

Look for:

- Layout, symbol, and version probes duplicated across backend modules.
- Internal helpers that mix capability probing, mutation, and runtime action.
- Unsupported-feature reasons that are built ad hoc instead of consistently reported.
- Direct ART mutation paths that should have narrower module visibility.

Findings:

- _None recorded yet._

### Replacement Facade And Backend

Files: `src/replacement/`.

Look for:

- Public callback concepts that expose trampoline or JNI-frame implementation details.
- Duplicated original-call argument builders or return conversion paths.
- Backend adapters whose boundaries are unclear.
- Lifecycle names that make the guard model harder to remember.
- Internal raw replacement pieces that can be hidden or renamed after facade stabilization.

Findings:

- _None recorded yet._

### Harnesses, Fixtures, And Examples

Files: `src/app_process_test.rs`, `src/app_process_test/`, `src/apk_perform_test.rs`,
`src/bin/art_test.rs`, `examples/`, `test-fixtures/`.

Look for:

- Tests asserting implementation detail instead of behavior.
- Harness helpers duplicated across app-process, APK, and native bootstrap paths.
- Example names or snippets that teach old API shapes.

Findings:

- _None recorded yet._

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
