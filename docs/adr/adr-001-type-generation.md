# ADR-001 — TypeScript type generation: `ts-rs`, not `specta`

**Status:** Accepted · 30 July 2026
**Affects:** T-002, `src/lib/types.ts`, `src/lib/ipc.ts`, the Rust MSRV

---

## Context

Types are defined once, in Rust, in `docs/CONTRACTS.md` §1. The renderer needs TypeScript definitions for the same types. Hand-writing them is not an option: two declarations of one contract drift, and the drift is silent until a field is read as `undefined` at runtime.

Two libraries generate TypeScript from Rust types.

**`specta`** is the one most Tauri material points at. It integrates with Tauri's command macros, so it can generate not just types but typed command bindings, which is genuinely more than `ts-rs` offers.

**`ts-rs`** is a derive macro and nothing else. It emits a `.ts` file per type, or into a shared file, and knows nothing about Tauri.

## Decision

**`ts-rs` 12.x.** `src/lib/types.ts` is generated. `src/lib/ipc.ts` is hand-written, imports those types, and contains one thin wrapper per IPC command.

## Reasoning

**Specta 2.0 has been in release candidate since 2023.** The 1.x line does not do what is needed. Depending on an RC for the type system of an entire renderer means depending on something whose author has explicitly not committed to its shape.

**The integration that makes specta attractive is also what makes it fragile.** Typed command bindings require specta, `tauri-specta` and `tauri` to agree on versions three ways. That coupling has already broken against stable Tauri releases. The failure lands where it is most expensive — an upgrade that should be routine turns into an unbounded investigation — and it lands on a project with one developer.

**What we give up is smaller than it looks.** Specta would generate the command wrappers; with `ts-rs` we write them by hand. There are roughly forty commands, each wrapper is three lines, and they change only when the IPC surface changes. That is an afternoon, once, against a permanent three-way version constraint.

**`ts-rs` is boring in the way a foundation should be.** A derive macro that emits text has almost no surface to break. When it does break, it breaks at compile time.

## Consequences

- **Rust MSRV is 1.88**, the MSRV of `ts-rs` 12.x. This is why `rust-toolchain.toml` pins 1.88 and why `docs/DEV-SETUP.md` states it as a requirement rather than a suggestion.
- **`TS_RS_EXPORT_DIR` must be set** in `.cargo/config.toml`, or generated files land in `bindings/` instead of `src/lib/`.
- **`src/lib/types.ts` is generated and never hand-edited.** This is invariant 9 in `AGENTS.md`.
- **`src/lib/ipc.ts` is hand-written but contains wrappers only.** It imports types; it never redeclares them. A lint rule enforces this, because an inline type declaration in a wrapper is exactly the drift this decision exists to prevent.
- **CI fails when `src/lib/types.ts` is stale.** Generation is not a step someone remembers to run.
- **One open question, resolved in T-002 either way.** Whether directing many types to one shared `export_to` target produces a single clean file is not certain. If it does not, a collector script concatenates the per-type output and the freshness check covers the result. Either path satisfies the requirement; the requirement is one file with no duplicate declarations, not a particular mechanism.

## Revisiting

Reconsider if specta 2.0 ships stable **and** has been stable across at least two Tauri minor releases. Migrating later is cheap: the generated file is an output, and the hand-written wrappers are the only thing that would be replaced.

Do not reconsider because a tutorial uses specta. That is the reason this ADR exists.
