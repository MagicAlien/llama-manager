//! `core/` holds application logic. `AGENTS.md` invariant 1: this module and
//! everything under it never imports `tauri`. `ipc/` (not yet created) is the
//! only thing allowed to depend on both `core/` and `tauri`.

// AGENTS.md invariant 6: no `unwrap()` or `expect()` in `core/` or `ipc/`
// outside tests. `deny` (not `warn`) so this fails `cargo clippy` on its
// own, independent of the `-D warnings` CI passes — an in-source lint-level
// attribute takes precedence over a command-line group flag for the same
// lint, so relying on `-D warnings` alone would not have been enough here.
// Scoped to this module (propagates to every submodule, including
// `core::types`) rather than crate-wide: `db/` and `main.rs` are
// deliberately outside invariant 6's scope, and `T-004`'s job is `core/`
// and `ipc/` only. `ipc/mod.rs` must carry the identical attribute when
// that module is created — nothing enforces invariant 6 there until it
// does, since the module does not exist yet (see PROGRESS.md).
// "Outside tests" is `../clippy.toml`'s job (`allow-unwrap-in-tests`,
// `allow-expect-in-tests`), not this attribute's — `deny` alone would also
// reject every `.unwrap()` inside `#[cfg(test)]` code.
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod types;
