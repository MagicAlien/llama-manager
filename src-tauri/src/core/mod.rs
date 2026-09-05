//! `core/` holds application logic. `AGENTS.md` invariant 1: this module and
//! everything under it never imports `tauri`. `ipc/` (created in T-010) is the
//! only thing allowed to depend on both `core/` and `tauri`.

// AGENTS.md invariant 6: no `unwrap()` or `expect()` in `core/` or `ipc/`
// outside tests. `deny` (not `warn`) so this fails `cargo clippy` on its
// own, independent of the `-D warnings` CI passes — an in-source lint-level
// attribute takes precedence over a command-line group flag for the same
// lint, so relying on `-D warnings` alone would not have been enough here.
// Scoped to this module (propagates to every submodule, including
// `core::types`) rather than crate-wide: `db/` and `main.rs` are
// deliberately outside invariant 6's scope, and `T-004`'s job is `core/`
// and `ipc/` only. `ipc/mod.rs` carries the identical attribute as of
// T-010, which is also the task that creates the module (see PROGRESS.md
// Observations, closed by this task).
// "Outside tests" is `../clippy.toml`'s job (`allow-unwrap-in-tests`,
// `allow-expect-in-tests`), not this attribute's — `deny` alone would also
// reject every `.unwrap()` inside `#[cfg(test)]` code.
#![deny(clippy::unwrap_used, clippy::expect_used)]

pub mod backend_selection;
pub mod env_probe;
pub mod flag_verify;
pub mod gh_releases;
pub mod installer;
pub mod types;
