// Entry point only. `AGENTS.md` invariant 1: `core/` and `ipc/` hold the
// logic. `ipc/` exists as of T-010 (`probe_environment` only, so far) —
// nothing in `docs/CONTRACTS.md` §4 is implemented until the task named in
// its "Implemented by" column runs. `invoke_handler` now registers
// `ipc::probe_environment` (T-011, docs/TASKS.md T-011) — T-010 deliberately
// left this out of its own scope (`core/` and `ipc/` only) since it was the
// first screen-side task that needed a real command to call, per
// PROGRESS.md's Observation naming this task. This line sits slightly
// outside T-005's "frontend only" precedent for a screen task, noted as a
// plain note in PROGRESS.md rather than a Discrepancy: it's necessary
// plumbing no earlier task had to do, not reality diverging from a
// document.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod db;
mod ipc;

use std::path::PathBuf;

use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

/// `%LOCALAPPDATA%\LlamaManager\logs`, per T-001's acceptance criteria.
/// `LOCALAPPDATA` is a Windows environment variable (`AGENTS.md` targets
/// Windows 11 exclusively); if it is ever absent, logging falls back to
/// stderr only rather than panicking — there is no `core/` or `ipc/`
/// invariant against `expect()` here, but a missing log directory is not a
/// reason to fail to launch.
fn log_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .map(|base| PathBuf::from(base).join("LlamaManager").join("logs"))
}

/// Initializes tracing with a daily-rotating file appender plus stderr.
/// Returns the `WorkerGuard` that must stay alive for the process lifetime
/// so buffered log lines are flushed on shutdown.
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let Some(dir) = log_dir() else {
        eprintln!("LOCALAPPDATA not set; logging to stderr only, no log file this run");
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
        return None;
    };

    if let Err(err) = std::fs::create_dir_all(&dir) {
        eprintln!(
            "could not create log directory {}: {err}; logging to stderr only",
            dir.display()
        );
        tracing_subscriber::registry()
            .with(env_filter)
            .with(fmt::layer().with_writer(std::io::stderr))
            .init();
        return None;
    }

    let file_appender = RollingFileAppender::new(Rotation::DAILY, &dir, "app.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt::layer().with_writer(std::io::stderr))
        .with(fmt::layer().with_writer(non_blocking).with_ansi(false))
        .init();

    Some(guard)
}

fn main() {
    // Held for the process lifetime: dropping it early would stop the
    // non-blocking writer from flushing to app.log.
    let _tracing_guard = init_tracing();

    tracing::info!("llama-manager starting");

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ipc::probe_environment])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
