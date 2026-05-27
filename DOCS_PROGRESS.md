# Documentation Progress

This file tracks the documentation rewrite described in `DOCUMENTATION_PASS.md`.
`DOCUMENTATION_PASS.md` owns the writing rules; this file owns sprint state, findings, and
verification notes.

## Current Sprint

- Sprint: supporting public rustdoc for errors, capabilities, Android version, and VM attachment
- Overall status: Verified
- Next step: use this tracker for future documentation findings and keep behavior/status docs synced
  when API names or runtime behavior change.

## Surface Status

| Surface | Status | Notes |
| --- | --- | --- |
| Crate-level docs and `README.md` | Verified | README and crate `//!` docs added with the first `Java::obtain()` / `perform()` path. |
| Java facade docs | Verified | Public rustdoc now leads with `perform()`, loader scope, wrapper calls, object/array ownership, and scheduling behavior. |
| Replacement facade docs | Verified | Guard ownership, original calls, constructor initialization, callback errors, and return conversion are documented in user terms. |
| Low-level JNI docs | Verified | Raw JNI module docs now state attachment, local-reference lifetime, and unsafe caller guarantees. |
| Runtime, capability, and error docs | Verified | `Error`, `JavaThrowable`, `AndroidVersion`, `JavaCapabilities`, `FeatureSupport`, and `Vm` now explain the user-visible contract and diagnostic boundaries. |
| ART/internal docs | Verified | Internal ART module docs describe maintainer invariants and supported/unsupported reporting boundaries. |
| Behavior docs | Verified | `ROADMAP.md`, `CURRENT_BEHAVIOR.md`, and `FEATURE_PROGRESS.md` remain the authoritative behavior/status split. |

## Findings

### Finding: crate-level entry point was missing

- Status: Rewritten
- Area: `README.md`, `src/lib.rs`
- Audience: normal user
- Problem: The crate exposed many Android-gated modules without a top-level explanation of what to
  use first.
- Proposed rewrite: Add a README and crate-level rustdoc that introduce `Java::obtain()`,
  `perform()`, wrapper calls, replacement, and raw JNI boundaries in that order.
- Verification: `cargo fmt --check`, `just check`, `just host-test`, and Android rustdoc attempt.
- Links: `DOCUMENTATION_PASS.md` crate-level docs target.

### Finding: Java facade docs mixed loader behavior with implementation shape

- Status: Rewritten
- Area: `src/java/`, especially `src/java/mod.rs` and `src/java/handle.rs`
- Audience: normal user
- Problem: Public docs described some cache and scope details before explaining the everyday choice
  between `perform()`, `perform_now()`, and `attach()`.
- Proposed rewrite: Put the decision guide first: use `perform()` for app classes and startup
  deferral, `perform_now()` for immediate scoped work, and `attach()` when code needs to hold a
  scope or access `Env`.
- Verification: Rustdoc build and review for behavior-first wording.
- Links: `DOCUMENTATION_PASS.md` Java facade docs target.

### Finding: replacement docs needed a user contract before callback internals

- Status: Rewritten
- Area: `src/replacement/api.rs`, `src/replacement/mod.rs`
- Audience: normal user
- Problem: Several comments mentioned raw callback machinery before explaining guard ownership,
  original calls, constructor initialization, and callback-local returns.
- Proposed rewrite: Describe installed callbacks as later Java invocations, explain `JavaHookGuard`
  ownership, document safe constructor initialization tokens, and keep raw JNI returns behind
  explicit unsafe APIs.
- Verification: Rustdoc build and app-process verification when replacement behavior changes.
- Links: `DOCUMENTATION_PASS.md` replacement docs target.

### Finding: low-level modules needed one clear raw boundary

- Status: Rewritten
- Area: `src/env/`, `src/refs.rs`, `src/value.rs`, `src/signature.rs`, `src/jni.rs`
- Audience: advanced JNI user
- Problem: The raw layer had accurate safety comments on individual methods but lacked a short
  module-level explanation of when to use it and what lifetime rules apply.
- Proposed rewrite: Add module docs covering thread attachment, local/global references, raw handle
  validity, descriptor parsing, and the relationship to high-level wrappers.
- Verification: Rustdoc build and review for precise `# Safety` sections.
- Links: `DOCUMENTATION_PASS.md` low-level JNI docs target.

### Finding: ART docs should stay maintainer-oriented

- Status: Rewritten
- Area: `src/art/`, internal replacement backend
- Audience: maintainer
- Problem: Internal ART docs needed a clearer boundary between public behavior and runtime mutation
  invariants.
- Proposed rewrite: Keep ART terminology in internal docs, state that unsupported runtime shapes
  must be reported structurally, and keep direct mutation behind guarded internal paths.
- Verification: Rustdoc build plus existing Android ART test matrix when internals change.
- Links: `DOCUMENTATION_PASS.md` ART/internal docs target.

### Finding: support types needed behavior-first public docs

- Status: Rewritten
- Area: `src/error.rs`, `src/android.rs`, `src/runtime.rs`, `src/vm.rs`
- Audience: normal user | advanced JNI user
- Problem: The supporting public types around errors, Android version reporting, runtime
  capabilities, and VM attachment were accurate but sparse, making users infer what was diagnostic,
  what was recoverable, and when `Vm` was appropriate instead of the high-level `Java` facade.
- Proposed rewrite: Add rustdoc that explains structured error matching, Java exception retention,
  honest unsupported capability reasons, Android property provenance, and explicit attachment guard
  behavior.
- Verification: `cargo fmt --check`; `git diff --check`; `just host-test`; `just check`;
  `cargo ndk -t arm64-v8a doc --no-deps --all-features`.
- Links: `DOCUMENTATION_PASS.md` public API doc rules; `CLEANUP_AUDIT.md` error grouping finding.

## Verification Notes

- `cargo fmt --check`: passed.
- `git diff --check`: passed.
- `just check`: passed.
- `just host-test`: passed, 22 tests.
- `cargo ndk -t arm64-v8a doc --no-deps --all-features`: passed after fixing rustdoc link
  warnings for `env::Env`, `mod@env`, and `refs::LocalRef`.
- Follow-up support-doc pass: `cargo fmt --check`, `git diff --check`, `just host-test`,
  `just check`, and
  `cargo ndk -t arm64-v8a doc --no-deps --all-features` passed.
