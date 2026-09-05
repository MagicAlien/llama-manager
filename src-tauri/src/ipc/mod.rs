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
//!
//! `main.rs` registers `probe_environment` with `tauri::generate_handler!`
//! as of T-011 (docs/TASKS.md T-011), which is why the `#[allow(dead_code)]`
//! T-010 left here is gone: the command now has a real caller reachable
//! from the binary's own entry point, not just a compiling-but-unrooted
//! function.

#![deny(clippy::unwrap_used, clippy::expect_used)]

use crate::core::env_probe;
use crate::core::gh_releases;
use crate::core::installer;
use crate::core::types::{AppError, AvailableRelease, Backend, EnvironmentReport, InstallProgress};
use tauri::Emitter;

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

/// `docs/CONTRACTS.md` §4: `list_runtimes | — | Vec<RuntimeBuild> | T-024`.
///
/// Returns all installed runtime builds, including their verification
/// status. The UI uses this to render the version management screen.
#[tauri::command]
pub fn list_runtimes() -> Result<Vec<crate::core::types::RuntimeBuild>, AppError> {
    let db_path = installer::database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    let conn = crate::db::open(&db_path)?;
    let rows = crate::db::queries::list_runtimes(&conn)?;
    Ok(rows
        .into_iter()
        .map(|row| installer::row_to_build(&row))
        .collect())
}

/// `docs/CONTRACTS.md` §4: `activate_runtime | tag, backend | RuntimeBuild | T-024`.
///
/// Sets the specified build as the active runtime. The active build is used
/// on the next server start. Multiple active builds are rejected by the
/// database's unique index, so this command first deactivates all others.
#[tauri::command]
pub fn activate_runtime(
    tag: String,
    backend: Backend,
) -> Result<crate::core::types::RuntimeBuild, AppError> {
    let db_path = installer::database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    let conn = crate::db::open(&db_path)?;
    installer::activate_runtime_on(&conn, &tag, &backend)
}

/// `docs/CONTRACTS.md` §4: `remove_runtime | tag, backend | () | T-024`.
///
/// Removes a runtime build from the database and deletes its files.
/// Cannot remove the last remaining build or an active build while the
/// server is running (checked by the caller).
#[tauri::command]
pub fn remove_runtime(tag: String, backend: Backend) -> Result<(), AppError> {
    let db_path = installer::database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    let Some(root) = installer::install_root() else {
        return Err(AppError::Internal {
            message: "LOCALAPPDATA is not set; the install root cannot be determined".into(),
        });
    };
    let conn = crate::db::open(&db_path)?;
    installer::remove_runtime_on(&conn, &tag, &backend, &root)?;
    Ok(())
}

/// `docs/CONTRACTS.md` §4: `get_active_runtime | — | RuntimeBuild | T-024`.
///
/// Returns the currently active runtime build, if any.
#[tauri::command]
pub fn get_active_runtime() -> Result<Option<crate::core::types::RuntimeBuild>, AppError> {
    let db_path = installer::database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    let conn = crate::db::open(&db_path)?;
    let rows = crate::db::queries::list_runtimes(&conn)?;
    Ok(rows
        .into_iter()
        .find(|row| row.is_active)
        .map(|row| installer::row_to_build(&row)))
}

/// `docs/CONTRACTS.md` §4: `install_runtime | tag, backend | () + install-progress events | T-022`.
///
/// Thin by design (invariant 1): deserialize `(tag, backend)`, resolve the
/// matching release through T-020's client, open the app database and the
/// install root, and bridge core's progress callback to the `install-progress`
/// event. All pipeline logic lives in `core::installer`.
///
/// The `Resolving` event is emitted here — not by core — because resolution
/// (fetching the releases list) happens in this handler; core emits from
/// `Downloading` onward, and a no-op re-install emits only `Done`. On any
/// failure the command emits `Failed { error }` *and* returns the error: the
/// event keeps the UI's progress state consistent while the returned error is
/// what an `invoke` caller can catch.
///
/// `AppHandle` is injected by Tauri (it is not a frontend argument): it is how
/// this handler reaches the event emitter without any global state
/// (invariant 2). The database file lives at `<data root>\llama-manager.db`
/// (`core::installer::database_path`); its parent directory is created here,
/// since `rusqlite` opens files but never creates directories.
#[tauri::command]
pub async fn install_runtime(
    tag: String,
    backend: Backend,
    app: tauri::AppHandle,
) -> Result<(), AppError> {
    let emit = |event: InstallProgress| {
        let _ = app.emit("install-progress", event);
    };

    emit(InstallProgress::Resolving);

    let client = gh_releases::default_client().map_err(|err| AppError::Network {
        message: format!("could not build the HTTP client: {err}"),
    })?;
    let releases = gh_releases::check_for_updates(&client, gh_releases::GITHUB_API_BASE).await?;

    let release = releases
        .into_iter()
        .find(|r| r.build_tag == tag && r.backend == backend)
        .ok_or_else(|| AppError::NotFound {
            what: format!("release {tag} for backend {backend}"),
        })?;

    let Some(data_root) = installer::data_root() else {
        return Err(AppError::Internal {
            message: "LOCALAPPDATA is not set; the app data directory cannot be determined".into(),
        });
    };
    let db_path = installer::database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    std::fs::create_dir_all(&data_root).map_err(|err| AppError::Io {
        message: format!("could not create {}: {err}", data_root.display()),
    })?;
    let conn = crate::db::open(&db_path)?;

    let Some(root) = installer::install_root() else {
        return Err(AppError::Internal {
            message: "LOCALAPPDATA is not set; the install root cannot be determined".into(),
        });
    };

    // Stage 1 (sync, DB): no-op check + stale-state cleanup.
    match installer::pre_install_on(&conn, &tag, &backend, &root)? {
        installer::PreCheckResult::Existing(build) => {
            emit(InstallProgress::Done { build });
            return Ok(());
        }
        installer::PreCheckResult::Proceed => {}
    }

    let mut progress = |event: InstallProgress| emit(event);

    // Stage 2 (async, no DB): download -> verify -> extract -> move.
    match installer::download_verify_extract(&client, &release, &root, &mut progress).await {
        Ok(extracted) => {
            // Stage 3 (sync, DB): register + delete the partial file.
            emit(InstallProgress::Registering);
            let build = installer::register_runtime_on(
                &conn,
                &tag,
                &backend,
                &extracted.target,
                &extracted.partial,
            )?;
            // Stage 4 (sync, DB): run `llama-server.exe --help` and persist the
            // verified flags + health endpoint (T-023). A failure here leaves
            // the build registered but unverified — it does not roll back the
            // install (see `installer::verify_runtime_on`).
            let build = installer::verify_runtime_on(&conn, &build)?;
            emit(InstallProgress::Done { build });
            Ok(())
        }
        Err(err) => {
            emit(InstallProgress::Failed { error: err.clone() });
            Err(err)
        }
    }
}

/// `docs/CONTRACTS.md` §4: `list_runtimes | — | Vec<RuntimeBuild> | T-024`.
#[tauri::command]
pub async fn check_for_updates() -> Result<Vec<AvailableRelease>, AppError> {
    let client = gh_releases::default_client().map_err(|err| AppError::Network {
        message: format!("could not build the HTTP client: {err}"),
    })?;
    gh_releases::check_for_updates(&client, gh_releases::GITHUB_API_BASE).await
}
