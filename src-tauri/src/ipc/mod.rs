//! `ipc/` — Tauri command handlers. `AGENTS.md` invariant 1: this module
//! holds no logic; it deserializes arguments, calls `core/`, and
//! serializes the result. `core/` never imports `tauri`; this is the one
//! module allowed to depend on both.
//!
//! Created here for the first time (T-010). `docs/CONTRACTS.md` §4's
//! "Implemented by" column names the task that creates each command, and
//! nothing before `probe_environment` (T-010) existed to justify this
//! module existing at all — `main.rs`'s own comment said as much: nothing
//! in `docs/CONTRACTS.md` §4 is implemented until the task named in its
//! "Implemented by" column runs. `PROGRESS.md`'s Observations section
//! already flagged that this module owes `core/mod.rs`'s
//! `#![deny(clippy::unwrap_used, clippy::expect_used)]` line the moment it
//! exists (`AGENTS.md` invariant 6 names both `core/` and `ipc/`) — copied
//! verbatim below.

#![deny(clippy::unwrap_used, clippy::expect_used)]
// Same rationale as the identical allow in `core/types.rs`, `db/mod.rs`
// and `db/queries.rs`: in a `bin` crate (this one; no `src-tauri/src/lib.rs`),
// `pub` alone does not exempt an item from `dead_code`, and nothing calls
// `probe_environment` yet — `main.rs` deliberately does not register it with
// `tauri::generate_handler!` in this task (see the comment there). Whichever
// task wires the first `invoke_handler` should remove this allow.
#![allow(dead_code)]

use crate::core::env_probe;
use crate::core::types::{AppError, EnvironmentReport};

/// `docs/CONTRACTS.md` §4: `probe_environment | — | EnvironmentReport | T-010`.
///
/// No arguments to deserialize. `core::env_probe::probe` never fails on
/// its own account — every internal failure (no GPU, an NVML error, an
/// unreadable disk or registry value) degrades into a `HealthCheck` entry
/// rather than propagating — so this always returns `Ok`. The `Result`
/// wrapper is kept anyway because `docs/CONTRACTS.md` §4's own preamble
/// states every IPC command returns `Result<T, AppError>`; this signature
/// does not need to change if a future provider swap introduces a failure
/// mode `core::env_probe::probe` itself cannot absorb.
#[tauri::command]
pub fn probe_environment() -> Result<EnvironmentReport, AppError> {
    let provider = env_probe::RealNvmlProvider::new();
    // `ServerConfig::listen_port` (`docs/CONTRACTS.md` ~line 397) defaults
    // to 8080. `ServerConfig` is not wired to real persisted settings until
    // T-050, so there is no `ServerConfig::default()` to read here yet.
    Ok(env_probe::probe(&provider, env_probe::DEFAULT_LISTEN_PORT))
}
