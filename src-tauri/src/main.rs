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
    // Headless subcommands (T-023): run before the GUI so the same binary can
    // install/verify a build and render `docs/verified-flags.md` without a
    // window. `AGENTS.md` invariant 1 keeps the logic in `core/`; this only
    // parses arguments and calls `core::installer`, then exits.
    if let Some(code) = run_headless() {
        std::process::exit(code);
    }

    // Held for the process lifetime: dropping it early would stop the
    // non-blocking writer from flushing to app.log.
    let _tracing_guard = init_tracing();

    tracing::info!("llama-manager starting");

    // T-039 — correct `runtimes` rows written before the observed registration
    // channel was recorded, so an installed build becomes usable without a
    // reinstall. Best-effort: a database that cannot be opened is a problem for
    // the screens that need it, never a reason to refuse to launch.
    match crate::core::installer::repair_registration_channels() {
        Ok(0) => {}
        Ok(n) => tracing::info!("corrected the registration channel of {n} installed build(s)"),
        Err(err) => {
            tracing::warn!("could not check the installed builds' registration channels: {err}")
        }
    }

    // T-037 — fill in the capability fields on catalogue rows written before
    // the model reader started answering the Thinking / Tool-use questions, so
    // an already-imported model shows its tags without being re-imported.
    // Best-effort for the same reason as the repair above.
    match crate::core::model_registry::backfill_capability_metadata() {
        Ok(0) => {}
        Ok(n) => tracing::info!("backfilled capability metadata for {n} model(s)"),
        Err(err) => tracing::warn!("could not backfill model capability metadata: {err}"),
    }

    // T-038 — fill in the header-derived geometry (attention interval, per-head
    // dimensions, recurrent-state keys, MTP layer count) on catalogue rows
    // written before the estimator read them, so an already-imported hybrid
    // model stops projecting a KV cache for all of its layers. Same recipe,
    // same best-effort posture.
    match crate::core::model_registry::backfill_vram_metadata() {
        Ok(0) => {}
        Ok(n) => tracing::info!("backfilled VRAM geometry for {n} model(s)"),
        Err(err) => tracing::warn!("could not backfill model VRAM geometry: {err}"),
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // T-040 — the process supervisor owns `ServerState` from here on.
            // It is started in `setup`, not before: the actor reports through
            // Tauri events, so it needs the app handle, and starting it after
            // the window exists cannot lose a state change to an event emitted
            // into the void. Every IPC command that touches the server fails
            // with a clear error rather than a fake "Stopped" if the handle is
            // ever missing, which is why this is a hard failure to log and not
            // a silent one.
            if let Err(err) = ipc::start_supervisor(app.handle().clone()) {
                tracing::error!("could not start the server supervisor: {err}");
            }
            // T-041 — the model orchestrator's poller. It reads the upstream
            // port from the supervisor above, so it is started after it. A
            // failure to start is logged and not fatal: the screens that show
            // loaded models (T-045) fail with a clear error rather than a fake
            // empty list.
            if let Err(err) = ipc::start_orchestrator(app.handle().clone()) {
                tracing::error!("could not start the model orchestrator: {err}");
            }
            // T-042 — the app's own listener (`PLAN.md` §2.7). Bound at app
            // start and independently of `ServerState`, because an address that
            // stays put is the whole reason the app owns the socket: it is up
            // while llama-server is stopped, starting, or gone. It reports into
            // the supervisor above, so it is started after it. A bind failure is
            // a reported state, not a fatal error — the app is usable, the
            // address is not, and T-050's screen is where it gets fixed.
            match ipc::start_endpoint() {
                Ok(state) => tracing::info!("endpoint state at startup: {state:?}"),
                Err(err) => tracing::error!("could not start the endpoint listener: {err}"),
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::probe_environment,
            ipc::check_for_updates,
            ipc::install_runtime,
            ipc::list_runtimes,
            ipc::activate_runtime,
            ipc::remove_runtime,
            ipc::get_active_runtime,
            ipc::get_server_state,
            ipc::get_endpoint_state,
            ipc::start_server,
            ipc::stop_server,
            ipc::dismiss_crash,
            ipc::get_loaded_models,
            ipc::load_model,
            ipc::unload_model,
            ipc::import_models,
            ipc::get_import_status,
            ipc::cancel_import,
            ipc::list_models,
            ipc::get_model,
            ipc::update_model_params,
            ipc::remove_model,
            ipc::set_model_preload,
            ipc::set_model_pinned,
            ipc::add_watched_folder,
            ipc::remove_watched_folder,
            ipc::list_watched_folders,
            ipc::rescan_models,
            ipc::estimate_vram,
            ipc::preview_preset,
            ipc::preview_preset_params,
            ipc::validate_draft_companion
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Dispatch the headless (no-GUI) subcommands. Returns `Some(code)` when a
/// subcommand was handled (and `main` must exit with it), or `None` to launch
/// the normal GUI.
///
/// - `install-verify <zip> <tag> <backend>` — extract a local archive, register
///   the build, and run `llama-server.exe --help` to persist its verified
///   flags (T-023). Keeps the pinned build registered for T-025/T-043.
/// - `export-verified-flags <tag> <backend> <out>` — render
///   `docs/verified-flags.md` for a build from the database (T-023).
fn run_headless() -> Option<i32> {
    let args: Vec<String> = std::env::args().collect();
    let sub = args.get(1).map(String::as_str);

    match sub {
        Some("install-verify") => {
            let (zip, tag, backend) = match (args.get(2), args.get(3), args.get(4)) {
                (Some(zip), Some(tag), Some(backend)) => {
                    (zip.as_str(), tag.as_str(), backend.as_str())
                }
                _ => {
                    eprintln!("usage: llama-manager install-verify <zip> <tag> <backend>");
                    return Some(2);
                }
            };
            let backend = match backend.parse::<crate::core::types::Backend>() {
                Ok(backend) => backend,
                Err(_) => {
                    eprintln!("unknown backend {backend:?} (expected vulkan, cpu, or cuda:N)");
                    return Some(2);
                }
            };
            match crate::core::installer::install_verify_local(
                std::path::Path::new(zip),
                tag,
                &backend,
            ) {
                Ok(build) => {
                    println!(
                        "verified {}/{}: {} flags, health endpoint {:?}, channel {:?}",
                        build.build_tag,
                        build.backend,
                        build.verified_flags.len(),
                        build.health_endpoint,
                        build.registration_channel
                    );
                    Some(0)
                }
                Err(err) => {
                    eprintln!("install-verify failed: {err}");
                    Some(1)
                }
            }
        }
        Some("export-verified-flags") => {
            let (tag, backend, out) = match (args.get(2), args.get(3), args.get(4)) {
                (Some(tag), Some(backend), Some(out)) => {
                    (tag.as_str(), backend.as_str(), out.as_str())
                }
                _ => {
                    eprintln!("usage: llama-manager export-verified-flags <tag> <backend> <out>");
                    return Some(2);
                }
            };
            let backend = match backend.parse::<crate::core::types::Backend>() {
                Ok(backend) => backend,
                Err(_) => {
                    eprintln!("unknown backend {backend:?} (expected vulkan, cpu, or cuda:N)");
                    return Some(2);
                }
            };
            match crate::core::installer::export_verified_flags_md(
                tag,
                &backend,
                std::path::Path::new(out),
            ) {
                Ok(()) => {
                    println!("wrote {out}");
                    Some(0)
                }
                Err(err) => {
                    eprintln!("export-verified-flags failed: {err}");
                    Some(1)
                }
            }
        }
        _ => None,
    }
}
