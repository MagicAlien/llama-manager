//! Model registry — import, list, remove, rescan models.
//!
//! Identity is the absolute path (`PLAN.md` §2.13). Importing the same path
//! twice updates the existing entry. Different paths with the same `sha256_head`
//! are flagged as duplicates.
//!
//! Created for T-031.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, info};

use crate::core::gguf;
use crate::core::installer::database_path;
use crate::core::types::{
    AppError, Compatibility, FlashAttn, ImportJobId, ImportProgress, LaunchParams,
    ModelAvailability, ModelEntry, SamplingDefaults, WatchedFolder,
};
use crate::db::{self, queries};

/// Generate reasonable default launch parameters for a newly imported model.
/// Called when there are no retained settings to restore.
/// `block_count` is the model's total layers (from GGUF metadata).
fn generate_default_launch_params(block_count: u32) -> LaunchParams {
    let threads = std::thread::available_parallelism()
        .map(|n| n.get() as u32)
        .unwrap_or(8);
    LaunchParams {
        gpu_layers: Some(block_count), // Max GPU offload by default
        ctx_size: Some(8000),          // User-specified default
        batch_size: Some(512),
        ubatch_size: Some(128),
        flash_attn: Some(FlashAttn::Auto),
        cache_type_k: Some("f16".to_string()), // llama.cpp default
        cache_type_v: Some("f16".to_string()), // llama.cpp default
        n_cpu_moe: Some(threads),              // Same as main threads
        tensor_split: None,
        main_gpu: Some(0), // First GPU by default
        no_mmap: Some(false),
        mlock: Some(false),
        threads: Some(threads),
        chat_template: None, // Auto-detected from GGUF by llama.cpp
        mmproj_path: None,
        extra_args: vec![],
    }
}

/// Generate reasonable default sampling parameters for a newly imported model.
/// Based on llama.cpp's documented defaults.
fn generate_default_sampling() -> SamplingDefaults {
    SamplingDefaults {
        temperature: Some(0.8),
        top_p: Some(0.9),
        top_k: Some(40),
        min_p: Some(0.05),
        repeat_penalty: Some(1.0),
        presence_penalty: Some(0.0),
        frequency_penalty: Some(0.0),
        seed: None,
    }
}

/// Import a list of model files into the registry.
pub async fn import_models(
    paths: Vec<PathBuf>,
    job_id: ImportJobId,
    cancelled: Arc<AtomicBool>,
    progress_callback: Option<std::sync::Arc<dyn Fn(ImportProgress) + Send + Sync>>,
) -> Result<ImportProgress, AppError> {
    let total = paths.len() as u32;

    // Cloned into the `emit` closure so the future stays `Send` (Tauri
    // requires it): the closure owns its copy and the original stays
    // usable below.
    let emit = {
        let job_id = job_id.clone();
        move |event: ImportProgress| {
            if let Some(cb) = &progress_callback {
                cb(event.clone());
            }
            debug!(job_id = %job_id, event = ?event, "import progress");
        }
    };

    emit(ImportProgress::Queued {
        job_id: job_id.clone(),
        total_files: total,
    });

    let mut imported = 0u32;
    let skipped = 0u32;
    let mut failed = 0u32;

    for (i, path) in paths.iter().enumerate() {
        let index = i as u32;

        emit(ImportProgress::FileStarted {
            job_id: job_id.clone(),
            path: path.clone(),
            index,
            total,
        });

        if cancelled.load(Ordering::Relaxed) {
            emit(ImportProgress::Cancelled {
                job_id: job_id.clone(),
                completed: imported + skipped + failed,
            });
            return Ok(ImportProgress::Cancelled {
                job_id,
                completed: imported + skipped + failed,
            });
        }

        match import_model_file(path, index, total, &job_id).await {
            Ok(entry) => {
                imported += 1;
                emit(ImportProgress::FileDone {
                    job_id: job_id.clone(),
                    entry: Box::new(entry),
                    index,
                    total,
                });
            }
            Err(e) => {
                failed += 1;
                emit(ImportProgress::FileFailed {
                    job_id: job_id.clone(),
                    path: path.clone(),
                    error: e.clone(),
                });
            }
        }
    }

    let result = ImportProgress::Finished {
        job_id: job_id.clone(),
        imported,
        skipped,
        failed,
    };
    emit(result.clone());
    Ok(result)
}

/// Open (creating if absent) the app database. Every registry call opens a
/// short-lived connection: `rusqlite::Connection` is not `Sync` and the
/// import job runs on its own thread, so a shared connection would need a
/// mutex for no real gain — call frequency is human-scale.
fn open_db() -> Result<rusqlite::Connection, AppError> {
    let path = database_path().ok_or_else(|| AppError::Internal {
        message: "LOCALAPPDATA is not set; cannot locate the app database".to_string(),
    })?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::Io {
            message: format!("could not create data directory {}: {e}", parent.display()),
        })?;
    }
    db::open(&path)
}

/// Serialize a catalogue entry's JSON-backed columns (`metadata_json`,
/// `compatibility_json`, ...) the way `db::queries` stores them.
fn model_row_from_entry(
    entry: &ModelEntry,
    last_launched_at: Option<chrono::DateTime<Utc>>,
) -> Result<queries::ModelRow, AppError> {
    Ok(queries::ModelRow {
        id: entry.id.clone(),
        display_name: entry.display_name.clone(),
        served_name: entry.served_name.clone(),
        file_path: entry.file_path.display().to_string(),
        shard_paths_json: serde_json::to_string(&entry.shard_paths).map_err(|e| {
            AppError::Internal {
                message: format!("serialize shard_paths: {e}"),
            }
        })?,
        size_bytes: entry.size_bytes as i64,
        sha256_head: entry.sha256_head.clone(),
        metadata_json: serde_json::to_string(&entry.metadata).map_err(|e| AppError::Internal {
            message: format!("serialize metadata: {e}"),
        })?,
        compatibility_json: serde_json::to_string(&entry.compatibility).map_err(|e| {
            AppError::Internal {
                message: format!("serialize compatibility: {e}"),
            }
        })?,
        availability: match entry.availability {
            ModelAvailability::Present => "Present".to_string(),
            ModelAvailability::Missing => "Missing".to_string(),
            ModelAvailability::Unreadable => "Unreadable".to_string(),
        },
        launch_params_json: serde_json::to_string(&entry.launch_params).map_err(|e| {
            AppError::Internal {
                message: format!("serialize launch_params: {e}"),
            }
        })?,
        sampling_json: serde_json::to_string(&entry.sampling_defaults).map_err(|e| {
            AppError::Internal {
                message: format!("serialize sampling_defaults: {e}"),
            }
        })?,
        preload: entry.preload,
        pinned: entry.pinned,
        added_at: entry.added_at,
        last_launched_at,
    })
}

fn model_entry_from_row(row: queries::ModelRow) -> Result<ModelEntry, AppError> {
    let metadata = serde_json::from_str(&row.metadata_json).map_err(|e| AppError::Internal {
        message: format!("deserialize metadata: {e}"),
    })?;
    let compatibility =
        serde_json::from_str(&row.compatibility_json).map_err(|e| AppError::Internal {
            message: format!("deserialize compatibility: {e}"),
        })?;
    let launch_params =
        serde_json::from_str(&row.launch_params_json).map_err(|e| AppError::Internal {
            message: format!("deserialize launch_params: {e}"),
        })?;
    let sampling_defaults =
        serde_json::from_str(&row.sampling_json).map_err(|e| AppError::Internal {
            message: format!("deserialize sampling_defaults: {e}"),
        })?;
    let shard_paths =
        serde_json::from_str(&row.shard_paths_json).map_err(|e| AppError::Internal {
            message: format!("deserialize shard_paths: {e}"),
        })?;
    Ok(ModelEntry {
        id: row.id,
        display_name: row.display_name,
        served_name: row.served_name,
        file_path: row.file_path.clone().into(),
        shard_paths,
        size_bytes: row.size_bytes as u64,
        sha256_head: row.sha256_head,
        metadata,
        compatibility,
        // Availability is re-probed on every read, not trusted from the row:
        // a model removed from disk must surface as Missing without waiting
        // for a rescan (`docs/TASKS.md` T-031 — drive disconnected marks its
        // models Missing). probe() is a stat-level check per file. The row's
        // stored `availability` is written at import time and can go stale.
        availability: crate::core::model_paths::probe(Path::new(&row.file_path)),
        duplicate_of: None,
        launch_params,
        sampling_defaults,
        preload: row.preload,
        pinned: row.pinned,
        added_at: row.added_at,
    })
}

/// Import a single model file.
async fn import_model_file(
    path: &Path,
    _index: u32,
    _total: u32,
    _job_id: &ImportJobId,
) -> Result<ModelEntry, AppError> {
    import_paths(&[path.to_path_buf()], None).await
}

/// Import one model SET as a single catalogue entry (`docs/TASKS.md` T-031
/// acceptance: "A multi-file model imports as a single entry"). `files` is
/// the shard list in order (or the lone file); `projector` is the
/// `mmproj-` companion discovered by naming convention, never picked by the
/// user. Identity is the head file's absolute path (`PLAN.md` §2.13).
async fn import_paths(
    files: &[PathBuf],
    projector: Option<PathBuf>,
) -> Result<ModelEntry, AppError> {
    let Some(head) = files.first() else {
        return Err(AppError::NotFound {
            what: "empty model set".to_string(),
        });
    };
    let normalized = head.clone();

    if !normalized.exists() {
        return Err(AppError::NotFound {
            what: format!("model file: {}", normalized.display()),
        });
    }

    let metadata = gguf::parse_file(&normalized)?;

    // Reject speculative-decoding draft models (DFlash, EAGLE, DSpark, ...).
    // These are companion models used with `-md`/`--spec-type`, not
    // standalone-launchable. They must not enter the catalogue as models.
    if metadata.is_draft_model {
        return Err(AppError::GgufParse {
            message: format!(
                "draft model detected (architecture: {}); not importable as a standalone model",
                metadata.architecture
            ),
        });
    }

    let sha256_head = sha256_head(&normalized)?;

    // size_bytes is the WHOLE set: sum of the shard files (the projector
    // is not part of the model's weight footprint on the preset channel).
    let mut size_bytes = 0u64;
    for f in files {
        size_bytes += std::fs::metadata(f)
            .map_err(|e| AppError::Io {
                message: format!("failed to stat {}: {e}", f.display()),
            })?
            .len();
    }

    // Retained settings (PLAN.md §2.9): a previous entry removed from the
    // catalogue left its params/sampling/preload/pinned behind keyed by
    // path — restore them on re-import. Open caveat: T-031's acceptance
    // also requires DISCARDING them when the file at the path changed
    // (sha256_head mismatch), but `retained_model_settings` has no hash
    // column to compare against — that needs a schema change and is
    // recorded in PROGRESS.md as an open gap, not silently approximated.
    let conn = open_db()?;
    let path_str = normalized.display().to_string();
    let retained = queries::get_retained_model_settings(&conn, &path_str)?;
    let (mut retained_launch_params, retained_sampling, retained_preload, retained_pinned): (
        LaunchParams,
        SamplingDefaults,
        bool,
        bool,
    ) = match retained {
        Some(r) => (
            serde_json::from_str(&r.launch_params_json)
                .unwrap_or_else(|_| generate_default_launch_params(metadata.block_count)),
            serde_json::from_str(&r.sampling_json).unwrap_or_else(|_| generate_default_sampling()),
            r.preload,
            r.pinned,
        ),
        None => (
            generate_default_launch_params(metadata.block_count),
            generate_default_sampling(),
            false,
            false,
        ),
    };
    // The projector travels in LaunchParams.mmproj_path (docs/CONTRACTS.md
    // §1). Set it only when discovered now or retained earlier.
    if let Some(proj) = projector {
        retained_launch_params.mmproj_path = Some(proj);
    }

    let entry = ModelEntry {
        id: format!("model-{ts}", ts = Utc::now().timestamp_millis()),
        display_name: normalized
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string(),
        served_name: normalized
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string(),
        file_path: normalized.clone(),
        shard_paths: files.iter().skip(1).cloned().collect(),
        size_bytes,
        sha256_head,
        metadata,
        compatibility: Compatibility::Supported,
        availability: ModelAvailability::Present,
        duplicate_of: None,
        launch_params: retained_launch_params,
        sampling_defaults: retained_sampling,
        preload: retained_preload,
        pinned: retained_pinned,
        added_at: Utc::now(),
    };

    // Persist. Upsert by absolute path: if a row for this file already
    // exists (re-import), update it in place rather than inserting a
    // second row — file_path is UNIQUE in the schema.
    let row = model_row_from_entry(&entry, None)?;
    let existing = queries::find_model_by_path(&conn, &path_str)?;
    match existing {
        Some(prev) => {
            // Keep the original id and added_at; refresh the derived data.
            let mut row = row;
            let prev_id = prev.id.clone();
            row.id = prev.id;
            row.added_at = prev.added_at;
            queries::delete_model(&conn, &prev_id)?;
            queries::insert_model(&conn, &row)?;
        }
        None => queries::insert_model(&conn, &row)?,
    }
    info!(path = %normalized.display(), "model imported");

    Ok(entry)
}

/// Compute sha256 of first 1 MiB of file.
fn sha256_head(path: &Path) -> Result<String, AppError> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open(path).map_err(|e| AppError::Io {
        message: format!("failed to open {}: {}", path.display(), e),
    })?;

    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    let bytes_read = file.read(&mut buffer).map_err(|e| AppError::Io {
        message: format!("failed to read {}: {}", path.display(), e),
    })?;

    hasher.update(&buffer[..bytes_read]);
    let result = hasher.finalize();
    let hex: String = result.iter().map(|b| format!("{b:02x}")).collect();
    Ok(hex)
}

/// List all imported models.
pub fn list_models() -> Result<Vec<ModelEntry>, AppError> {
    let conn = open_db()?;
    let rows = queries::list_models(&conn)?;
    rows.into_iter().map(model_entry_from_row).collect()
}

/// The single catalogue entry for an id, if it exists (`docs/CONTRACTS.md`
/// §4 `get_model`; T-035's detail screen loads through it).
pub fn get_model(id: &str) -> Result<Option<ModelEntry>, AppError> {
    let conn = open_db()?;
    let row = queries::get_model(&conn, id)?;
    row.map(model_entry_from_row).transpose()
}

/// Save a model's launch params and sampling defaults (`docs/CONTRACTS.md`
/// §4 `update_model_params`; T-035's detail screen persists through it).
/// The identity columns (path, metadata, compatibility) are untouched:
/// they belong to the importer, not to the settings editor.
pub fn update_model_params(
    id: &str,
    launch_params: &LaunchParams,
    sampling_defaults: &SamplingDefaults,
) -> Result<ModelEntry, AppError> {
    let conn = open_db()?;
    let launch_params_json =
        serde_json::to_string(launch_params).map_err(|e| AppError::Internal {
            message: format!("serialize launch_params: {e}"),
        })?;
    let sampling_json =
        serde_json::to_string(sampling_defaults).map_err(|e| AppError::Internal {
            message: format!("serialize sampling_defaults: {e}"),
        })?;
    queries::update_model_params(&conn, id, &launch_params_json, &sampling_json)?;
    // Re-read through the standard mapper so the returned entry carries a
    // fresh availability probe, not the stale stored value.
    let row = queries::get_model(&conn, id)?.ok_or_else(|| AppError::Internal {
        message: format!("model {id} vanished during update"),
    })?;
    model_entry_from_row(row)
}

/// Live import jobs by id, each with its cancellation flag. The frontend's
/// `cancel_import(jobId)` (docs/CONTRACTS.md §4) flips the flag; the
/// awaiting `import_models` command observes it between files. Entries are
/// removed when the command returns, so this map is bounded by in-flight
/// work only.
static IMPORT_JOBS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<ImportJobId, Arc<AtomicBool>>>,
> = std::sync::OnceLock::new();

fn jobs() -> std::sync::MutexGuard<'static, std::collections::HashMap<ImportJobId, Arc<AtomicBool>>>
{
    IMPORT_JOBS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Register a job's cancel flag. Called by `ipc::import_models` before it
/// awaits the work; the returned guard's Drop unregisters it.
pub fn register_import_job(job_id: &ImportJobId, cancelled: &Arc<AtomicBool>) -> ImportJobGuard {
    jobs().insert(job_id.clone(), cancelled.clone());
    ImportJobGuard {
        job_id: job_id.clone(),
    }
}

/// Unregisters the job on drop, whatever the import's outcome.
pub struct ImportJobGuard {
    job_id: ImportJobId,
}

impl Drop for ImportJobGuard {
    fn drop(&mut self) {
        jobs().remove(&self.job_id);
    }
}

/// Request cancellation of an in-flight import by job id. The next file in
/// the loop is skipped; the file already in progress is atomic per row
/// (one INSERT at the end), so the catalogue stays consistent — T-031's
/// "cancelling mid-import leaves a consistent catalogue with no partial
/// entries". Returns NotFound for an unknown/finished job.
pub fn cancel_import(job_id: &str) -> Result<(), AppError> {
    let flag = jobs()
        .get(job_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound {
            what: format!("import job {job_id}"),
        })?;
    flag.store(true, Ordering::Relaxed);
    Ok(())
}

/// Remove a model from the catalogue, RETAINING its settings by absolute
/// path (`PLAN.md` §2.9, docs/TASKS.md T-031): a later re-import of the
/// same file restores launch params, sampling defaults, preload and pinned.
pub fn remove_model(id: &str) -> Result<(), AppError> {
    let conn = open_db()?;
    retain_and_delete_model(&conn, id)
}

/// The DB half of [`remove_model`], taking a caller's connection so a
/// batch (watched-folder unregistration) can share one.
fn retain_and_delete_model(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    let row = queries::get_model(conn, id)?.ok_or_else(|| AppError::NotFound {
        what: format!("model {id}"),
    })?;
    let retained = queries::RetainedModelSettingsRow {
        file_path: row.file_path.clone(),
        launch_params_json: row.launch_params_json,
        sampling_json: row.sampling_json,
        preload: row.preload,
        pinned: row.pinned,
        retained_at: Utc::now(),
    };
    queries::upsert_retained_model_settings(conn, &retained)?;
    queries::delete_model(conn, id)?;
    info!(model_id = %id, "model removed (settings retained by path)");
    Ok(())
}

/// True when `model_path` lives under `folder` (absolute prefix match,
/// separator-inclusive so `C:\a\bc` is not under `C:\a\b`).
fn path_under_folder(model_path: &str, folder: &str) -> bool {
    let prefix = folder.trim_end_matches(['\\', '/']);
    match model_path.strip_prefix(prefix) {
        Some(rest) => rest.starts_with('\\') || rest.starts_with('/') || rest.is_empty(),
        None => false,
    }
}

/// Remove a watched folder AND unregister the models that live inside it
/// (`docs/CONTRACTS.md` §4: `remove_watched_folder | path | ()
/// (unregisters its models, never touches files)`; T-031 acceptance:
/// "Removing a watched folder unregisters its models and never touches the
/// files on disk"). The unregistered models keep their retained settings,
/// so re-watching the folder or re-importing a file restores everything.
pub fn remove_watched_folder(path: &str) -> Result<(), AppError> {
    let conn = open_db()?;
    let doomed: Vec<String> = queries::list_models(&conn)?
        .into_iter()
        .filter(|m| path_under_folder(&m.file_path, path))
        .map(|m| m.id)
        .collect();
    for id in &doomed {
        retain_and_delete_model(&conn, id)?;
    }
    queries::delete_watched_folder(&conn, path)?;
    info!(folder = %path, unregistered = doomed.len(), "watched folder removed");
    Ok(())
}

/// Toggle preload for a catalogue entry. Returns the updated entry so the
/// frontend can refresh without a second round-trip (`ipc.ts` contract).
pub fn set_model_preload(id: &str, preload: bool) -> Result<ModelEntry, AppError> {
    let conn = open_db()?;
    let row = queries::get_model(&conn, id)?.ok_or_else(|| AppError::NotFound {
        what: format!("model {id}"),
    })?;
    queries::update_model_pinned_and_preload(&conn, id, row.pinned, preload)?;
    model_entry_from_row(
        queries::get_model(&conn, id)?.ok_or_else(|| AppError::Internal {
            message: format!("model {id} vanished during update"),
        })?,
    )
}

/// Toggle pin (exemption from LRU eviction). See [`set_model_preload`].
pub fn set_model_pinned(id: &str, pinned: bool) -> Result<ModelEntry, AppError> {
    let conn = open_db()?;
    let row = queries::get_model(&conn, id)?.ok_or_else(|| AppError::NotFound {
        what: format!("model {id}"),
    })?;
    queries::update_model_pinned_and_preload(&conn, id, pinned, row.preload)?;
    model_entry_from_row(
        queries::get_model(&conn, id)?.ok_or_else(|| AppError::Internal {
            message: format!("model {id} vanished during update"),
        })?,
    )
}

/// Maximum watched-folder scan depth below the root (root = 0). Four
/// covers the LM Studio layout (`models/<owner>/<repo>/*.gguf`) and the
/// HuggingFace cache layout with room to spare; a hard limit bounds a
/// pathological tree (network mounts, junction loops) the same way the
/// router's own one-level scan does.
const WATCH_SCAN_MAX_DEPTH: u32 = 4;

/// Collect every GGUF model set under `dir`, descending up to
/// `WATCH_SCAN_MAX_DEPTH` levels. Symlinked/junction subdirectories are
/// not followed — a cycle under a watched root would otherwise hang the
/// scan (T-025 F-011 showed the router resolves links; for a catalogue
/// scan the files live in place, links add no models worth the risk).
fn collect_model_sets(dir: &Path, depth: u32, out: &mut Vec<gguf::ModelSet>) {
    if depth > WATCH_SCAN_MAX_DEPTH {
        return;
    }
    if let Ok(sets) = gguf::scan_directory(dir) {
        out.extend(sets);
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_model_sets(&entry.path(), depth + 1, out);
        }
    }
}

/// Add a watched folder AND register the models it contains
/// (`docs/CONTRACTS.md` §4: `add_watched_folder | path | Vec<ModelEntry>
/// (scans, registers; files stay put)`). Returns the entries discovered
/// on this first scan; rescans of new files are `rescan_models`'s job.
pub async fn add_watched_folder(path: &Path) -> Result<Vec<ModelEntry>, AppError> {
    if !path.is_dir() {
        return Err(AppError::InvalidPath {
            path: path.display().to_string(),
            reason: "not a directory".to_string(),
        });
    }
    let conn = open_db()?;
    let now = Utc::now();
    let row = queries::WatchedFolderRow {
        path: path.display().to_string(),
        added_at: now,
        last_scan_at: Some(now),
    };
    // Re-watching an existing folder is an update, not an error.
    queries::insert_watched_folder(&conn, &row).or_else(|e| match e {
        AppError::Database { ref message } if message.contains("UNIQUE") => {
            queries::update_watched_folder_last_scan(&conn, &row.path, now)
        }
        other => Err(other),
    })?;

    let mut sets = Vec::new();
    collect_model_sets(path, 0, &mut sets);

    let mut entries = Vec::new();
    for set in sets {
        // A file that is part of a shard set already imported is skipped
        // only when its head is registered: import is idempotent per path
        // (upsert), so a genuine re-scan refreshes instead of erroring.
        match import_paths(&set.model_files, set.projector).await {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                // One unreadable model out of a folder must not lose the
                // rest (T-031: "leaves the rest of the catalogue intact").
                tracing::warn!(path = %set.model_files[0].display(), error = %e, "watched-folder scan: model skipped");
            }
        }
    }
    info!(folder = %path.display(), found = entries.len(), "watched folder registered");
    Ok(entries)
}

/// Rescan every watched folder, registering newly appeared models
/// (`docs/CONTRACTS.md` §4: `rescan_models | — | Vec<ModelEntry>`).
/// Models whose file vanished are marked Missing by the availability
/// probe — the catalogue is not silently edited by a scan.
pub async fn rescan_models() -> Result<Vec<ModelEntry>, AppError> {
    let conn = open_db()?;
    let folders = queries::list_watched_folders(&conn)?;
    let now = Utc::now();
    let mut entries = Vec::new();
    for folder in folders {
        let path = PathBuf::from(&folder.path);
        let mut sets = Vec::new();
        if path.is_dir() {
            collect_model_sets(&path, 0, &mut sets);
        }
        for set in sets {
            match import_paths(&set.model_files, set.projector).await {
                Ok(entry) => entries.push(entry),
                Err(e) => {
                    tracing::warn!(path = %set.model_files[0].display(), error = %e, "rescan: model skipped");
                }
            }
        }
        if let Err(e) = queries::update_watched_folder_last_scan(&conn, &folder.path, now) {
            tracing::warn!(folder = %folder.path, error = %e, "rescan: last_scan_at not updated");
        }
    }
    info!(registered = entries.len(), "rescan_models done");
    Ok(entries)
}

/// List watched folders.
pub fn list_watched_folders() -> Result<Vec<WatchedFolder>, AppError> {
    let conn = open_db()?;
    let rows = queries::list_watched_folders(&conn)?;
    // One pass over the catalogue to count how many registered models live
    // under each watched root (prefix match on the absolute path, separator
    // included so /foo/bar does not swallow /foo/barbaz).
    let model_paths: Vec<String> = queries::list_models(&conn)?
        .into_iter()
        .map(|m| m.file_path)
        .collect();
    Ok(rows
        .into_iter()
        .map(|r| {
            let path_buf = PathBuf::from(&r.path);
            let reachable = path_buf.is_dir();
            let model_count = model_paths
                .iter()
                .filter(|p| path_under_folder(p, &r.path))
                .count() as u32;
            WatchedFolder {
                path: path_buf,
                model_count,
                reachable,
                last_scan_at: r.last_scan_at,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gguf::GgufValue;

    /// Build a minimal valid GGUF byte buffer carrying only the keys given.
    /// Wire types per the GGUF v3 spec: string=8, u32=4.
    fn minimal_gguf(kv: &[(&str, GgufValue)]) -> Vec<u8> {
        const MAGIC: u32 = 0x46554747;
        const VERSION: u32 = 3;
        let mut buf = Vec::new();
        buf.extend_from_slice(&MAGIC.to_le_bytes());
        buf.extend_from_slice(&VERSION.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes()); // tensor count
        buf.extend_from_slice(&(kv.len() as u64).to_le_bytes()); // kv count
        for (key, value) in kv {
            let key_bytes = key.as_bytes();
            buf.extend_from_slice(&(key_bytes.len() as u64).to_le_bytes());
            buf.extend_from_slice(key_bytes);
            match value {
                GgufValue::String(s) => {
                    buf.extend_from_slice(&8u32.to_le_bytes());
                    let s_bytes = s.as_bytes();
                    buf.extend_from_slice(&(s_bytes.len() as u64).to_le_bytes());
                    buf.extend_from_slice(s_bytes);
                }
                GgufValue::UInt32(n) => {
                    buf.extend_from_slice(&4u32.to_le_bytes());
                    buf.extend_from_slice(&n.to_le_bytes());
                }
                other => panic!("unsupported test value: {other:?}"),
            }
        }
        buf
    }

    #[tokio::test]
    async fn test_import_rejects_draft_architecture_before_db_write() {
        // T-036 acceptance: a draft-architecture file is rejected at import
        // with a typed, named error — and the rejection happens BEFORE any
        // row is written, so no DB interaction is required to observe it.
        let tmp = std::env::temp_dir().join(format!("lm-mgr-draft-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("Qwen3.8-27B-DFlash2.gguf");
        // A file NAMED like a DFlash draft whose architecture IS a draft
        // architecture must be rejected on the header, not the name.
        let bytes = minimal_gguf(&[
            (
                "general.architecture",
                GgufValue::String("dflash2".to_string()),
            ),
            ("llama.block_count", GgufValue::UInt32(16)),
        ]);
        std::fs::write(&file, &bytes).unwrap();

        let result = import_paths(&[file.clone()], None).await;
        let _ = std::fs::remove_dir_all(&tmp);

        match result {
            Err(AppError::GgufParse { message }) => {
                assert!(
                    message.contains("draft model detected"),
                    "rejection must name the draft-model reason, got: {message}"
                );
                assert!(
                    message.contains("dflash2"),
                    "rejection must name the architecture"
                );
            }
            other => panic!("expected a draft-model GgufParse error, got: {other:?}"),
        }
    }
}
