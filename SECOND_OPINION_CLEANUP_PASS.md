# Second-Opinion Cleanup Pass

This file is a self-contained brief for an independent cleanup review after the first cleanup
discovery and implementation passes. The reviewer does not need to have participated in those
passes and should not treat the existing cleanup audit as the source of truth.

The goal is to find remaining simplification, naming, organization, and teachability issues before
the crate becomes a first usable private version. This is not a feature sprint, compatibility
freeze, or hardening implementation sprint.

## Project Posture

This crate is a private, pre-user Rust implementation of the useful Android ART behavior from
`frida-java-bridge`. It intentionally does not promise stable exported Rust APIs, stable module
names, or line-by-line JavaScript compatibility. Rename, move, or delete exposed APIs when that
makes normal Java work clearer or ART-specific behavior safer.

Keep the project boundary sharp:

- Android ART only.
- Rust-native API design, not GumJS API cloning.
- Safe APIs for normal Java work.
- Explicit `unsafe` APIs for raw JNI, raw ART mutation, or caller-owned runtime guarantees.
- Unsupported behavior should return a clear unsupported reason instead of guessing or falling back
  silently.

## Primary Reference Points

Use these files and repositories as background, but keep your own findings in
`SECOND_OPINION_CLEANUP_AUDIT.md`:

- `ROADMAP.md`: current priorities, verification gates, and design principles.
- `FINALIZATION_PLAN.md`: overall stabilization sequence.
- `CURRENT_BEHAVIOR.md`: current user-visible behavior notes.
- `FEATURE_PROGRESS.md`: feature/status matrix aligned with upstream docs.
- `CLEANUP_AUDIT.md`: first cleanup pass findings and outcomes. Read this only after you have
  completed an initial code-first review of a module family.
- `HARDENING_AUDIT.md`: known safety/correctness concerns. Link to it when a cleanup finding is
  really a hardening issue.
- `DOCUMENTATION_PASS.md`: public documentation rewrite rules.
- `../frida-java-bridge`: upstream behavior reference.
- `../frida-java-bridge/lib/android.js`: upstream ART internals reference.
- `../frida-gum` and `../frida-rust/frida-gum`: process/module discovery and Rust Gum bindings.
- `~/work/android/art` and `~/work/android/base`: Android/ART source references.

## Independence Rules

The second-opinion pass should produce a fresh view, not a restatement of the first audit.

1. Start each module family from the code, not from `CLEANUP_AUDIT.md`.
2. Record your own findings in `SECOND_OPINION_CLEANUP_AUDIT.md`.
3. After recording findings for a module family, compare with `CLEANUP_AUDIT.md` and mark whether
   each finding is new, previously fixed, previously rejected, or still unresolved.
4. Do not modify `CLEANUP_AUDIT.md` unless you are intentionally correcting a factual error there.
5. Do not implement cleanup while doing discovery. Write the finding first.
6. Do not bury safety bugs as cleanup. Link to `HARDENING_AUDIT.md` and record only the cleanup
   angle in this pass.
7. Do not preserve names, modules, or wrappers only because examples or harnesses currently use
   them.

## What Counts As Cleanup

Cleanup means making the crate easier to understand, smaller where possible, and better organized
without weakening behavior.

Classify findings as:

- Delete: unused code, stale aliases, compatibility shims, helpers, or constants that no longer
  earn their place.
- Merge: duplicate concepts or near-synonyms that should become one concept.
- Move: code living in the wrong module or abstraction layer.
- Rename: names that expose internal vocabulary, imply unsupported scope, or make ordinary Java work
  harder to learn.
- Simplify: control flow or type shape that can be made more direct without losing safety or
  capabilities.
- Document: missing or misleading maintainer comments. Public API documentation belongs in
  `DOCUMENTATION_PASS.md`.
- Reject Previous: a first-pass cleanup decision that appears to have made the design worse or
  needs reconsideration.
- Other: any cleanup issue that does not fit the categories above.

## What Not To Do

- Do not add features unless a feature is required to remove a broken or misleading shape.
- Do not hide runtime failures behind feature gates or best-effort fallbacks.
- Do not move ART-specific unsafety into safe public APIs for prettier call sites.
- Do not broaden harness responsibilities. App-process behavior stays in the app-process harness,
  APK startup behavior stays in the APK harness, and native bootstrap behavior stays in
  `src/bin/art_test.rs`.
- Do not make public docs teach JNI vtables, trampolines, cloned methods, or ART layout internals
  unless the API is explicitly raw or unsafe.
- Do not do broad style churn, formatting-only rewrites, or unrelated renames.

## Review Order

Work one module family at a time. Stop and write down dependencies when a finding spans families.

### 1. Public Crate Shape

Files:

- `src/lib.rs`
- `src/error.rs`
- `src/android.rs`
- `src/runtime.rs`
- `src/vm.rs`

### 2. Safe JNI Environment And Values

Files:

- `src/env/`
- `src/jni.rs`
- `src/refs.rs`
- `src/value.rs`
- `src/signature.rs`
- `src/metadata.rs`
- `src/modifiers.rs`

### 3. High-Level Java Facade

Files:

- `src/java/`
- `src/replacement/api.rs` where it is part of the user-facing Java facade

### 4. ART Internals

Files:

- `src/art/`
- `src/runtime.rs`
- `src/vm.rs` where runtime discovery or ART access is involved

### 5. Replacement Facade And Backend

Files:

- `src/replacement/`
- `src/art/replacement.rs`
- `src/java/wrapper.rs` where replacement is exposed from selected handles

### 6. Harnesses, Fixtures, And Examples

Files:

- `src/app_process_test.rs`
- `src/app_process_test/`
- `src/apk_perform_test.rs`
- `src/bin/art_test.rs`
- `examples/`
- `test-fixtures/`
- `justfile`

### 7. Behavior And Status Docs

Files:

- `ROADMAP.md`
- `CURRENT_BEHAVIOR.md`
- `FEATURE_PROGRESS.md`
- `FINALIZATION_PLAN.md`
- `DOCUMENTATION_PASS.md`
- `CLEANUP_AUDIT.md`
- `HARDENING_AUDIT.md`

## Discovery Deliverable

For each module family in `SECOND_OPINION_CLEANUP_AUDIT.md`, record either concrete findings or:

```md
Reviewed: no second-opinion cleanup findings.
```

Each finding should include:

- Status: usually `Discovered` during discovery.
- Area: files or module family.
- Kind: one or more cleanup categories.
- Independent observation: what you saw before comparing with the first audit.
- Why it matters: the teachability, organization, or API-shape cost.
- Proposed cleanup: the likely fix or decision.
- First-pass comparison: new, previously fixed, previously rejected, overlapping unresolved, or
  needs reconciliation.
- Verification: the narrowest likely check after implementation.
- Links: related roadmap, cleanup, hardening, behavior, or documentation notes.

## Implementation Deliverable

If asked to implement the second-opinion findings, move in small patches. After each patch, update
the matching finding status to one of:

- Fixed: code changed and verification noted.
- Deferred: still wanted, with a reason.
- Rejected: intentionally kept, with the design reason.
- Moved To Hardening: the issue is primarily safety/correctness and now belongs in
  `HARDENING_AUDIT.md`.
- Moved To Documentation: the issue belongs in `DOCUMENTATION_PASS.md`.

Prefer implementation batches that stay within one module family. If a patch crosses families, name
the dependency in the finding before changing code.

## Verification Commands

Use `cargo ndk` for build/check/test operations. Prefer the `justfile` recipes:

- `just check`: Android arm64 clippy.
- `just build`: Android arm64 debug build.
- `just unit-test-build`: build Android arm64 library unit tests without running them.
- `just unit-test all`: run Android arm64 unit tests through `cargo-ndk-runner`.
- `just test all`: primary app-process ART harness through `adb`.
- `just apk-perform-test all`: APK startup-agent deferred `Java::perform()` test.
- `just art-test all`: native ART bootstrap test harness.

Use the narrowest gate that matches the change:

- Parser, descriptor, formatting, and selection logic: Android unit-test build or unit tests.
- Live Java calls, wrapper dispatch, loader lookup, metadata, exceptions, replacement, main-thread
  scheduling, and reference ownership: `just test`.
- Early app startup, deferred `Java::perform()`, app loader publication, or real main-looper
  behavior: `just apk-perform-test`.
- Native ART loading, manual VM creation, signal-chain behavior, or bootstrap attachment:
  `just art-test`.

If a required device or tool is unavailable, record the exact command that could not run and the
reason in the finding.
