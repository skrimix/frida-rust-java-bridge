# Finalization Plan

This file owns the last pre-user stabilization push. The goal is to turn the current implementation
into a smaller, safer, and more teachable first usable version.

The project is still private and pre-user. It is okay to rename exported APIs, move modules, delete
weak abstractions, and rewrite documentation when that makes normal Java work clearer or ART-specific
behavior safer. Do not preserve accidental shapes just because they exist today.

## Current Status

Cleanup implementation is active. The current bounded sprint is Java display placement: moving
formatting impls and display helpers next to the Java facade types that own them, without changing
public APIs or output strings. Gum accessor cleanup, replacement module naming, error enum grouping,
modifier constants, and hardening findings remain deferred to later sprints.

## Working Model

Move in limited sprints. Each sprint should have a narrow target, a written discovery note, a bounded
cleanup or hardening patch, and a verification note.

Use this sequence:

1. Cleanup discovery plus a lightweight hardening inventory.
2. Cleanup implementation.
3. Hardening discovery, including a fresh read of areas changed during cleanup.
4. Hardening implementation.
5. Documentation rewrite.
6. Final verification and behavior-status sync.

Discovery phases write down findings before changing code. Implementation phases may update the
audit files as facts change, but should not silently skip or bury issues discovered earlier.
Discovery is complete when every relevant audit section has concrete findings or an explicit
`Reviewed: no issues found` note.

## Sprint Size

Prefer sprints that cover one module family at a time:

- public facade: `src/java/`, `src/replacement/api.rs`, public re-exports
- low-level JNI surface: `src/env/`, `src/jni.rs`, `src/refs.rs`, `src/value.rs`
- ART internals: `src/art/`, `src/replacement/`, `src/runtime.rs`, `src/vm.rs`
- behavior and status docs: `ROADMAP.md`, `CURRENT_BEHAVIOR.md`, `FEATURE_PROGRESS.md`
- harnesses and examples: `src/app_process_test*`, `src/apk_perform_test.rs`, `src/bin/art_test.rs`,
  `examples/`, `test-fixtures/`

If a sprint starts crossing several of these boundaries, stop and write the dependency down instead
of letting the patch sprawl.

## What To Optimize For

- A small number of concepts users can remember.
- Safe APIs for normal Java work.
- Explicit `unsafe` APIs for raw JNI handles, ART mutation, or caller-owned runtime guarantees.
- Clear unsupported outcomes with reasons.
- Module placement that matches responsibility.
- Tests that explain intended behavior rather than internal implementation tricks.
- Documentation that speaks to someone learning this library and only casually familiar with Java
  internals.

## What Not To Do

- Do not add new features during cleanup unless they are required to remove a broken or misleading
  shape.
- Do not hide failing runtime behavior behind new feature gates.
- Do not keep wrappers, aliases, or helper traits whose only purpose is preserving old local names.
- Do not move ART-specific unsafety into public safe APIs to make call sites look nicer.
- Do not make public documentation teach trampolines, JNI argument frames, vtable slots, or ART/crate
  internal mechanics unless the API is explicitly an unsafe/raw interface.
- Do not call everything unsafe or advanced an "escape hatch" in the documentation.
- Do not broaden the test harness responsibilities: app-process behavior stays in the app-process
  harness, APK startup behavior stays in the APK harness, and native bootstrap behavior stays in
  `src/bin/art_test.rs`.

## Work Files

- `CLEANUP_AUDIT.md`: discovery and implementation tracking for simplifying modules and concepts.
- `HARDENING_AUDIT.md`: discovery and implementation tracking for lifetimes, unsafety, and bugs.
- `DOCUMENTATION_PASS.md`: public-doc rewrite rules and checklist.

## Done Criteria

- Cleanup findings are either fixed, intentionally deferred with a reason, or moved to `ROADMAP.md`.
- If the second-opinion cleanup pass is run, its findings are either fixed, intentionally deferred
  with a reason, rejected with a design reason, or moved to the hardening/documentation trackers.
- Hardening findings are either fixed, intentionally unsupported with a documented reason, or covered
  by explicit `unsafe` boundaries.
- Every `_None recorded yet._` placeholder in `CLEANUP_AUDIT.md` and `HARDENING_AUDIT.md` has been
  replaced with either concrete findings or an explicit `Reviewed: no issues found` note.
- Public documentation explains behavior first and internals only when the caller opted into a raw or
  unsafe layer.
- `CURRENT_BEHAVIOR.md` and `FEATURE_PROGRESS.md` match the code after refactors.
- Required verification from `ROADMAP.md` has been run or the reason it could not be run is written
  down.
