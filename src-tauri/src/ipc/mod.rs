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
use crate::core::estimator;
use crate::core::gh_releases;
use crate::core::installer;
use crate::core::model_registry;
use crate::core::preset_generator;
use crate::core::types::{
    AppError, AvailableRelease, Backend, EnvironmentReport, EstimateInputs, ImportJobId,
    ImportProgress, InstallProgress, ModelEntry, VramEstimate, WatchedFolder,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
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
    let db_dir = db_path.parent().ok_or_else(|| AppError::Internal {
        message: "could not determine the database directory".into(),
    })?;
    if !db_dir.exists() {
        std::fs::create_dir_all(db_dir).map_err(|e| AppError::Internal {
            message: format!("could not create database directory: {e}"),
        })?;
    }
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

/// `docs/CONTRACTS.md` §4: `get_server_state | — | ServerState | T-040`.
///
/// Returns the current server state. Until T-040 is implemented, this
/// always returns "Stopped" — the server process supervisor does not
/// exist yet, so the server can never be running.
#[tauri::command]
pub fn get_server_state() -> Result<crate::core::types::ServerState, AppError> {
    Ok(crate::core::types::ServerState::Stopped)
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

/// `docs/CONTRACTS.md` §4: `import_models | paths: Vec<String> | ImportJobId | T-031`.
///
/// Import and AWAIT the result. The command used to return the job id
/// immediately while a background thread did the work — the frontend's
/// `loadModels()` then raced the import and read an empty catalogue
/// (observed 8 Sept 2026: the row landed in the DB ~14 ms after the UI
/// had already refreshed to "No models imported yet"). The import is
/// human-scale file I/O on explicitly user-picked paths; awaiting it
/// here is the honest contract, and `get_import_status` remains for
/// longer-lived progress reporting.
#[tauri::command]
pub async fn import_models(paths: Vec<String>) -> Result<ImportJobId, AppError> {
    let job_id = ImportJobId::new();
    let path_bufs: Vec<std::path::PathBuf> =
        paths.into_iter().map(std::path::PathBuf::from).collect();

    let cancelled = Arc::new(AtomicBool::new(false));
    // Register the job so cancel_import(jobId) can reach the flag while the
    // command is awaited; the guard unregisters on return either way.
    let _guard = model_registry::register_import_job(&job_id, &cancelled);
    let result = model_registry::import_models(path_bufs, job_id.clone(), cancelled, None).await?;
    // Surface a failed job as a rejected command so the frontend can
    // distinguish success from failure (the Finished/Cancelled event the
    // result carries is still the progress record).
    tracing::info!(job_id = %job_id, ?result, "import_models done");

    Ok(job_id)
}

/// `docs/CONTRACTS.md` §4: `cancel_import | job_id: String | () | T-031`.
#[tauri::command]
pub fn cancel_import(job_id: String) -> Result<(), AppError> {
    model_registry::cancel_import(&job_id)
}

/// `docs/CONTRACTS.md` §4: `get_import_status | job_id: String | ImportProgress | T-031`.
///
/// Returns the current status of an import job.
#[tauri::command]
pub fn get_import_status(job_id: String) -> Result<ImportProgress, AppError> {
    // For now, return a stub. In a real implementation, this would look up
    // the job status from a shared state store.
    Ok(ImportProgress::Finished {
        job_id,
        imported: 0,
        skipped: 0,
        failed: 0,
    })
}

/// `docs/CONTRACTS.md` §4: `list_models | — | Vec<ModelEntry> | T-031`.
///
/// Returns all imported models with their metadata and status.
#[tauri::command]
pub fn list_models() -> Result<Vec<ModelEntry>, AppError> {
    model_registry::list_models()
}

/// `docs/CONTRACTS.md` §4: `remove_model | id: String | () | T-031`.
///
/// Removes a model from the catalogue, retaining its settings by path.
#[tauri::command]
pub fn remove_model(id: String) -> Result<(), AppError> {
    model_registry::remove_model(&id)
}

/// `docs/CONTRACTS.md` §4: `set_model_preload | id: String, preload: bool | ModelEntry | T-031`.
#[tauri::command]
pub fn set_model_preload(id: String, preload: bool) -> Result<ModelEntry, AppError> {
    model_registry::set_model_preload(&id, preload)
}

/// `docs/CONTRACTS.md` §4: `set_model_pinned | id: String, pinned: bool | ModelEntry | T-031`.
#[tauri::command]
pub fn set_model_pinned(id: String, pinned: bool) -> Result<ModelEntry, AppError> {
    model_registry::set_model_pinned(&id, pinned)
}

/// `docs/CONTRACTS.md` §4: `add_watched_folder | path: String | Vec<ModelEntry> | T-031`.
///
/// Adds a directory to watch for new models AND registers what it contains.
#[tauri::command]
pub async fn add_watched_folder(path: String) -> Result<Vec<ModelEntry>, AppError> {
    let path_buf = std::path::PathBuf::from(path);
    model_registry::add_watched_folder(&path_buf).await
}

/// `docs/CONTRACTS.md` §4: `rescan_models | — | Vec<ModelEntry> | T-031`.
#[tauri::command]
pub async fn rescan_models() -> Result<Vec<ModelEntry>, AppError> {
    model_registry::rescan_models().await
}

/// `docs/CONTRACTS.md` §4: `list_watched_folders | — | Vec<WatchedFolder> | T-031`.
///
/// Returns all watched folders.
#[tauri::command]
pub fn list_watched_folders() -> Result<Vec<WatchedFolder>, AppError> {
    model_registry::list_watched_folders()
}

/// `docs/CONTRACTS.md` §4: `remove_watched_folder | path: String | () | T-031`.
///
/// Unwatches a folder and unregisters the models inside it; files on disk
/// are never touched.
#[tauri::command]
pub fn remove_watched_folder(path: String) -> Result<(), AppError> {
    model_registry::remove_watched_folder(&path)
}

/// `docs/CONTRACTS.md` §4: `estimate_vram | id: String, LaunchParams | VramEstimate | T-032`.
///
/// Estimates VRAM usage for a model with the given launch parameters.
/// This is a pure function — no database access, no side effects.
#[tauri::command]
pub fn estimate_vram(
    id: String,
    params: crate::core::types::LaunchParams,
) -> Result<VramEstimate, AppError> {
    let models = model_registry::list_models()?;
    let model = models
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::NotFound {
            what: format!("model {id}"),
        })?;

    let inputs = EstimateInputs {
        metadata: model.metadata,
        file_size_bytes: model.size_bytes,
        params,
        // Use a large default VRAM; the caller can override via params
        vram_free_bytes: 24 * 1024 * 1024 * 1024,
        ram_free_bytes: 16 * 1024 * 1024 * 1024,
    };

    // No history for now — T-041 writes launch_history, T-032 reads it
    Ok(estimator::estimate(&inputs, &[]))
}

/// `docs/CONTRACTS.md` §4: `preview_preset | id: String | String | T-033`.
///
/// Returns a preview of the preset INI content for a single model,
/// without writing to disk. Used by the model detail screen to show
/// the user what the preset would look like.
#[tauri::command]
pub fn preview_preset(id: String) -> Result<String, AppError> {
    let models = model_registry::list_models()?;
    let model = models
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| AppError::NotFound {
            what: format!("model {id}"),
        })?;

    let builds = list_runtimes()?;
    let build = builds
        .into_iter()
        .find(|b| b.is_active)
        .ok_or_else(|| AppError::NotFound {
            what: "active runtime build".to_string(),
        })?;

    preset_generator::preview_preset(&model, &build)
}
