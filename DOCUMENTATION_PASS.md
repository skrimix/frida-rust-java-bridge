# Documentation Pass

This file owns the final documentation rewrite.

The reader is a Rust user who wants to inspect or change Java behavior in an Android app. Assume they
know ordinary programming and may know Java, but do not assume deep knowledge of JNI, ART internals,
Frida GumJS implementation details, trampolines, vtables, method layouts, or Android startup
internals.

## Documentation Goals

- Explain what the user can do.
- Explain what values they must keep alive.
- Explain when work runs immediately, later, or on another thread.
- Explain what errors mean and how to react to unsupported features.
- Put normal Java work first and raw/unsafe work later.
- Keep public docs behavior-oriented.

## Voice

Write to a human learning the library:

- Prefer: "Use this for building long argument lists for method calls."
- Avoid: "This is useful for long hook original-call lists where Rust tuple support intentionally
  stops at the common small arities."

Prefer:

- "Runs the closure while the current thread is attached to Java."
- "Keeps the replacement installed until the guard is dropped or reverted."
- "Returns an error if the app class loader is not available yet."
- "Use `perform()` when you need app classes during startup."

Avoid public-facing explanations centered on:

- trampolines
- JNI argument frames
- vtable slots
- ART method layouts
- quick entrypoints
- cloned method internals
- internal crate implementation details
- register and stack argument capture

Those details may appear in private module comments or unsafe/raw API docs when they are the actual
caller contract.

## Public API Doc Rules

- Start with the behavior, not the implementation.
- Name the common use case before advanced variants.
- Say what owns a reference, guard, or callback result.
- Say whether a method uses the app loader, an explicit loader, or bootstrap lookup when that affects
  user behavior.
- Say whether a callback may run later.
- Say what happens on Java exceptions.
- Say when `unsafe` is required and what the caller must guarantee.
- Keep examples small and realistic.
- Do not promise API stability.
- Do not call everything unsafe or advanced an "escape hatch".
- Use "raw JNI layer", "unsafe API", or a specific contract when those are more precise than
  "advanced".

## Concept Budget

The public docs should help users remember a small set of concepts:

- `Java`: the handle for Java work in this process.
- `perform()`: use app classes, including during startup.
- `attach()` / `perform_now()`: run synchronous work on the current thread.
- `use_class()`: get a high-level class wrapper.
- `JavaClass` / `JavaObject`: call methods, access fields, create objects, and cast values.
- loader scope: only when choosing a non-default class loader matters.
- `JavaHookGuard`: keeps a method or constructor replacement active.
- `Env` and raw JNI: advanced/raw JNI layer.

If public docs need more concepts than this for ordinary usage, record a cleanup finding in
`CLEANUP_AUDIT.md`.

## Documentation Targets

### Crate-Level Docs

Files: `src/lib.rs`, new `README.md` file.

Needed:

- A short "what this crate is" paragraph.
- A first example using `Java::obtain()` and `perform()`.
- A brief map of normal APIs versus advanced/raw APIs.
- A clear Android ART-only statement, with arm64/API-level limits called out only for the features
  that currently depend on that runtime support, such as ART mutation, enumeration, deoptimization,
  or replacement.
- A high level list of upstream `frida-java-bridge` features and their implementation status in this
  crate.

### Java Facade Docs

Files: `src/java/`.

Needed:

- When to use `perform()`, `perform_now()`, and `attach()`.
- Include a short decision guide: use `perform()` for app classes and startup deferral,
  `perform_now()` for immediate work in the current loader scope, and `attach()` when a lexical Java
  scope should be held for several operations.
- How `use_class()` behaves with the default app loader.
- Calling methods and constructors with overloads.
- Fields, arrays, strings, casts, and object retention.
- Main-thread scheduling behavior and result reporting.

### Replacement Docs

Files: `src/replacement/api.rs`, public items re-exported from `src/replacement/mod.rs`.

Needed:

- How to replace a method.
- How to call the original implementation.
- How constructor replacement differs from method replacement.
- What the guard owns.
- How callback errors are reported.
- How to return strings, objects, arrays, primitives, null, and void.

Do not make users learn closure trampoline internals to use this API.

### Low-Level JNI Docs

Files: `src/env/`, `src/refs.rs`, `src/value.rs`, `src/signature.rs`, `src/jni.rs`.

Needed:

- Explain that this is the advanced layer.
- Explain attachment and local-reference lifetime at the API boundary.
- Keep raw handles and unsafe constructors precise about caller guarantees.
- Link common users back to `Java::use_class()` and friends where appropriate.

### ART/Internal Docs

Files: `src/art/`, internal replacement backend files.

Needed:

- Explain invariants for maintainers.
- Keep unsupported reasons and Android-version assumptions easy to audit.
- Document why unsafe blocks are valid near the code using them.

Internal docs may use ART/JNI vocabulary, but should not leak that vocabulary into high-level public
API docs.

Distinguish doc surfaces:

- `///` public docs follow the public voice and concept budget.
- `//` internal comments may use precise ART/JNI vocabulary when that helps maintainers.
- `// SAFETY:` comments on unsafe blocks should state the local invariant or caller contract, not
  just that a check happened.

### Behavior Docs

Files: `ROADMAP.md`, `CURRENT_BEHAVIOR.md`, `FEATURE_PROGRESS.md`.

Needed:

- Sync after cleanup and hardening.
- Keep `CURRENT_BEHAVIOR.md` about current behavior, not future wishes.
- Keep `FEATURE_PROGRESS.md` as a status matrix.
- Keep `ROADMAP.md` focused on remaining sequencing and deliberate scope choices.

## Rewrite Checklist

- [ ] Crate-level docs give a first usable example.
- [ ] Public high-level docs avoid JNI/ART internals unless required.
- [ ] Raw and unsafe docs state caller guarantees.
- [ ] Guard and reference lifetimes are explained in user terms.
- [ ] Loader behavior is explained without requiring Android framework internals.
- [ ] Replacement docs explain behavior, original calls, and guard ownership.
- [ ] Unsupported features and runtime errors point users toward next steps.
- [ ] Examples compile or are clearly marked as illustrative.
- [ ] `CURRENT_BEHAVIOR.md` and `FEATURE_PROGRESS.md` match the final API names.

## Finding Template

```md
### Finding: short title

- Status: Discovered | Rewritten | Deferred | Rejected
- Area: module or file path
- Audience: normal user | advanced JNI user | maintainer
- Problem:
- Proposed rewrite:
- Verification:
- Links:
```
