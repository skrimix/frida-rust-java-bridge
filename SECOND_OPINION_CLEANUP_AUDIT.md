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

_Not reviewed yet._

## Safe JNI Environment And Values

Files: `src/env/`, `src/jni.rs`, `src/refs.rs`, `src/value.rs`, `src/signature.rs`,
`src/metadata.rs`, `src/modifiers.rs`.

Findings:

_Not reviewed yet._

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
