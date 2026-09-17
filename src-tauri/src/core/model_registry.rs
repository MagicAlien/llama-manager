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

use crate::core::capabilities;
use crate::core::gguf;
use crate::core::installer::database_path;
use crate::core::types::{
    AppError, Compatibility, FlashAttn, GgufMetadata, ImportJobId, ImportProgress, LaunchParams,
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
        // T-036: a freshly imported model carries no speculative-decoding
        // configuration. An MTP model still drafts with its own heads —
        // `spec_type_for_entry` derives that from the header, so nothing has
        // to be written here for it to work.
        speculative: None,
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
    let metadata: GgufMetadata =
        serde_json::from_str(&row.metadata_json).map_err(|e| AppError::Internal {
            message: format!("deserialize metadata: {e}"),
        })?;
    let compatibility =
        serde_json::from_str(&row.compatibility_json).map_err(|e| AppError::Internal {
            message: format!("deserialize compatibility: {e}"),
        })?;
    let launch_params: LaunchParams =
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
    let capability_tags = capabilities::tags_for(&metadata, &launch_params);
    Ok(ModelEntry {
        id: row.id,
        display_name: row.display_name,
        served_name: row.served_name,
        file_path: row.file_path.clone().into(),
        shard_paths,
        size_bytes: row.size_bytes as u64,
        sha256_head: row.sha256_head,
        metadata,
        capability_tags,
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

    // T-037 — the tags a model earns come from its header (Thinking / MTP /
    // Tool use) and from its own file set (Vision, via the projector resolved
    // above), so they are derived here from the same two values the entry
    // stores. One derivation, both screens.
    let capability_tags = capabilities::tags_for(&metadata, &retained_launch_params);

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
        capability_tags,
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

/// T-037 — repair catalogue rows whose stored metadata predates the capability
/// fields.
///
/// The two capability answers (Thinking, Tool use) live inside the header, and
/// T-037 reads them once at import. Rows imported earlier therefore carry a
/// `metadata_json` without `supports_tools` / `supports_thinking`, and
/// `#[serde(default)]` would faithfully deserialize them as `false` — the tag
/// row would stay empty for a model that does have the capability. Nothing
/// re-derives stored metadata on read (re-parsing every model's header on every
/// list call would read a tokenizer table per model per screen load), so the
/// correction happens once, here, at startup.
///
/// **Minimal and idempotent.** Only the two missing keys are added: every other
/// field of the stored JSON — including anything a user set or a later task
/// derived — is carried through untouched. A row that already carries both keys
/// is skipped without touching the disk. A model whose file has gone away, or
/// whose header cannot be parsed, keeps its stored value and is logged: a data
/// anomaly is never upgraded into a plausible answer.
///
/// Mirrors `installer::repair_registration_channels_on` (T-039): the same
/// class of one-off correction, for the same reason.
pub fn backfill_capability_metadata_on(conn: &rusqlite::Connection) -> Result<u32, AppError> {
    let rows = queries::list_models(conn)?;
    let mut corrected = 0u32;

    for row in &rows {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&row.metadata_json) else {
            tracing::warn!(
                "model {}: stored metadata is not valid JSON; capability fields left as stored",
                row.id
            );
            continue;
        };
        let Some(object) = value.as_object_mut() else {
            tracing::warn!(
                "model {}: stored metadata is not a JSON object; capability fields left as stored",
                row.id
            );
            continue;
        };
        if object.contains_key("supports_tools") && object.contains_key("supports_thinking") {
            continue;
        }

        let path = Path::new(&row.file_path);
        if !path.exists() {
            tracing::debug!(
                "model {}: {} is not on disk; capability fields left as stored",
                row.id,
                row.file_path
            );
            continue;
        }
        let parsed = match gguf::parse_file(path) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    "model {}: header could not be re-read ({err}); capability fields left as stored",
                    row.id
                );
                continue;
            }
        };

        object.insert(
            "supports_tools".to_string(),
            serde_json::Value::Bool(parsed.supports_tools),
        );
        object.insert(
            "supports_thinking".to_string(),
            serde_json::Value::Bool(parsed.supports_thinking),
        );
        let json = serde_json::to_string(&value).map_err(|e| AppError::Internal {
            message: format!("serialize metadata: {e}"),
        })?;
        queries::set_model_metadata(conn, &row.id, &json)?;
        info!(
            "backfilled capability metadata for {}: tools={}, thinking={}",
            row.display_name, parsed.supports_tools, parsed.supports_thinking
        );
        corrected += 1;
    }

    Ok(corrected)
}

/// The startup entry point for [`backfill_capability_metadata_on`]: resolves the
/// database path, ensures its parent directory exists (the database is created
/// on first open, and `db::open` cannot create its own parent), opens the
/// database and runs the repair.
///
/// **Best-effort by design**, like the T-039 repair beside it: the caller logs
/// a failure and starts anyway.
pub fn backfill_capability_metadata() -> Result<u32, AppError> {
    let db_path = database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    if let Some(dir) = db_path.parent() {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::Io {
                message: format!("create database directory {}: {e}", dir.display()),
            })?;
        }
    }
    let conn = db::open(&db_path)?;
    backfill_capability_metadata_on(&conn)
}

/// The header-derived geometry fields T-038 added to `GgufMetadata`, paired with
/// the JSON key each is stored under. A table, not eight inline inserts: the
/// next header-derived field is one line here plus its fixture.
fn vram_geometry_fields(
    parsed: &crate::core::types::GgufMetadata,
) -> [(&'static str, serde_json::Value); 8] {
    fn opt(value: Option<u32>) -> serde_json::Value {
        match value {
            Some(n) => serde_json::Value::from(n),
            None => serde_json::Value::Null,
        }
    }

    [
        ("attention_key_length", opt(parsed.attention_key_length)),
        ("attention_value_length", opt(parsed.attention_value_length)),
        (
            "full_attention_interval",
            opt(parsed.full_attention_interval),
        ),
        ("ssm_state_size", opt(parsed.ssm_state_size)),
        ("ssm_inner_size", opt(parsed.ssm_inner_size)),
        ("ssm_group_count", opt(parsed.ssm_group_count)),
        ("ssm_conv_kernel", opt(parsed.ssm_conv_kernel)),
        ("mtp_layer_count", opt(parsed.mtp_layer_count)),
    ]
}

/// T-038 — repair rows imported before the header-derived geometry fields
/// existed, exactly as [`backfill_capability_metadata_on`] does for T-037's two
/// capability flags, and for the same reason.
///
/// `metadata_json` is written once at import and never re-derived on read (a
/// tokenizer table per model per screen is not affordable), so a row that
/// predates T-038 carries neither `full_attention_interval` nor the per-head
/// dimensions. `#[serde(default)]` then reports them as `None` — and `None`
/// means "every layer holds a KV cache", which is the overestimate this task
/// exists to remove. Re-reading every header on every read is not the fix; this
/// one-off repair is.
///
/// Additive and idempotent: only the missing keys are inserted, a file that is
/// gone or unreadable leaves its row **exactly as stored** (an anomaly must not
/// be upgraded into a plausible answer), and a second run returns 0.
pub fn backfill_vram_metadata_on(conn: &rusqlite::Connection) -> Result<u32, AppError> {
    let rows = queries::list_models(conn)?;
    let mut corrected = 0u32;

    for row in &rows {
        let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&row.metadata_json) else {
            tracing::warn!(
                "model {}: stored metadata is not valid JSON; VRAM geometry left as stored",
                row.id
            );
            continue;
        };
        let Some(object) = value.as_object_mut() else {
            tracing::warn!(
                "model {}: stored metadata is not a JSON object; VRAM geometry left as stored",
                row.id
            );
            continue;
        };
        if object.contains_key("full_attention_interval") && object.contains_key("mtp_layer_count")
        {
            continue;
        }

        let path = Path::new(&row.file_path);
        if !path.exists() {
            tracing::debug!(
                "model {}: {} is not on disk; VRAM geometry left as stored",
                row.id,
                row.file_path
            );
            continue;
        }
        let parsed = match gguf::parse_file(path) {
            Ok(parsed) => parsed,
            Err(err) => {
                tracing::warn!(
                    "model {}: header could not be re-read ({err}); VRAM geometry left as stored",
                    row.id
                );
                continue;
            }
        };

        let mut inserted: Vec<&str> = Vec::new();
        for (key, field) in vram_geometry_fields(&parsed) {
            if !object.contains_key(key) {
                object.insert(key.to_string(), field);
                inserted.push(key);
            }
        }
        if inserted.is_empty() {
            continue;
        }

        let json = serde_json::to_string(&value).map_err(|e| AppError::Internal {
            message: format!("serialize metadata: {e}"),
        })?;
        queries::set_model_metadata(conn, &row.id, &json)?;
        info!(
            "backfilled VRAM geometry for {}: {}",
            row.display_name,
            inserted.join(", ")
        );
        corrected += 1;
    }

    Ok(corrected)
}

/// The startup entry point for [`backfill_vram_metadata_on`]. Best-effort, like
/// its two neighbours: a failure is logged and the app starts anyway.
pub fn backfill_vram_metadata() -> Result<u32, AppError> {
    let db_path = database_path().ok_or_else(|| AppError::Internal {
        message: "could not determine the database path".into(),
    })?;
    if let Some(dir) = db_path.parent() {
        if !dir.exists() {
            std::fs::create_dir_all(dir).map_err(|e| AppError::Io {
                message: format!("create database directory {}: {e}", dir.display()),
            })?;
        }
    }
    let conn = db::open(&db_path)?;
    backfill_vram_metadata_on(&conn)
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
    // T-036: a draft companion is validated BEFORE it is persisted — it must
    // exist, parse as GGUF, and carry a draft architecture. Each failure is a
    // typed error naming the file (docs/TASKS.md T-036), so a bad path can
    // never reach the database and be discovered at server start instead.
    crate::core::speculative::validate_companion_of(launch_params)?;

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

    /// T-037 — a catalogue row written before the capability fields existed
    /// must gain them at startup: without this, every already-imported model
    /// renders an empty tag row while its header plainly declares the
    /// capabilities (the row is the only thing the screens see).
    #[test]
    fn test_t037_backfill_fills_capability_fields_only_and_is_idempotent() {
        use crate::core::types::CapabilityTag;

        let conn = crate::db::open_in_memory().unwrap();

        let tmp = std::env::temp_dir().join(format!("lm-mgr-t037-backfill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // A real header that declares both template-driven capabilities. The
        // delimiters are spelled as the real template spells them: plain, no
        // backslash (see PROGRESS.md F-017).
        let template = concat!(
            "{%- if tools %}<tool_call></tool_call>{%- endif %}",
            "{%- if enable_thinking is undefined %}",
            "{{- '<think>' + reasoning_content + '</think>' }}",
            "{%- endif %}"
        );

        let declares = tmp.join("declares.gguf");
        std::fs::write(
            &declares,
            minimal_gguf(&[
                (
                    "general.architecture",
                    GgufValue::String("qwen35".to_string()),
                ),
                (
                    "tokenizer.chat_template",
                    GgufValue::String(template.to_string()),
                ),
            ]),
        )
        .unwrap();

        // A header with a template that declares neither.
        let silent = tmp.join("silent.gguf");
        std::fs::write(
            &silent,
            minimal_gguf(&[
                (
                    "general.architecture",
                    GgufValue::String("qwen35".to_string()),
                ),
                (
                    "tokenizer.chat_template",
                    GgufValue::String("{{ }}".to_string()),
                ),
            ]),
        )
        .unwrap();

        // A row whose model file is gone.
        let gone = tmp.join("gone.gguf");

        // The metadata as an earlier version of the app stored it: no
        // `supports_tools`, no `supports_thinking`, and a user-visible field
        // (`context_length` 200000) that must survive the correction.
        let stored_metadata = "{\"architecture\":\"qwen35\",\"param_count\":27000000000,\
\"quantization\":\"NVFP4\",\"block_count\":65,\"context_length\":200000,\
\"embedding_length\":null,\"attention_head_count\":null,\"attention_head_count_kv\":null,\
\"has_chat_template\":true,\"is_moe\":false,\"expert_count\":null,\
\"is_draft_model\":false,\"has_mtp_heads\":true}"
            .to_string();

        let mut row = queries::ModelRow {
            id: "model-declares".to_string(),
            display_name: "declares.gguf".to_string(),
            served_name: "declares.gguf".to_string(),
            file_path: declares.to_string_lossy().to_string(),
            shard_paths_json: "[]".to_string(),
            size_bytes: 1024,
            sha256_head: "abc".to_string(),
            metadata_json: stored_metadata.clone(),
            compatibility_json: "\"Supported\"".to_string(),
            availability: "Present".to_string(),
            launch_params_json: serde_json::to_string(&LaunchParams::default()).unwrap(),
            sampling_json: serde_json::to_string(&SamplingDefaults::default()).unwrap(),
            preload: false,
            pinned: false,
            added_at: Utc::now(),
            last_launched_at: None,
        };
        queries::insert_model(&conn, &row).unwrap();

        row.id = "model-silent".to_string();
        row.served_name = "silent.gguf".to_string();
        row.file_path = silent.to_string_lossy().to_string();
        queries::insert_model(&conn, &row).unwrap();

        row.id = "model-gone".to_string();
        row.served_name = "gone.gguf".to_string();
        row.file_path = gone.to_string_lossy().to_string();
        queries::insert_model(&conn, &row).unwrap();

        let corrected = backfill_capability_metadata_on(&conn).unwrap();
        assert_eq!(
            corrected, 2,
            "both rows whose file could be re-read are corrected; the missing one is not"
        );

        // The declaring model now carries both answers, and everything the row
        // already held is intact — a field the user can see is not collateral.
        let updated = queries::get_model(&conn, "model-declares")
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated.metadata_json).unwrap();
        assert_eq!(value["supports_tools"], serde_json::Value::Bool(true));
        assert_eq!(value["supports_thinking"], serde_json::Value::Bool(true));
        assert_eq!(value["context_length"], serde_json::json!(200000));
        assert_eq!(value["has_mtp_heads"], serde_json::Value::Bool(true));
        assert_eq!(value["quantization"], serde_json::json!("NVFP4"));

        // The reading the screens do now yields the tags.
        let entry = model_entry_from_row(updated).unwrap();
        assert_eq!(
            entry.capability_tags,
            vec![
                CapabilityTag::Thinking,
                CapabilityTag::Mtp,
                CapabilityTag::ToolUse
            ],
            "the row the screens read must now carry the header's answers"
        );

        // A template that declares neither keeps both fields false: the
        // correction reports what the header says, it does not invent a tag.
        let silent_row = queries::get_model(&conn, "model-silent").unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&silent_row.metadata_json).unwrap();
        assert_eq!(value["supports_tools"], serde_json::Value::Bool(false));
        assert_eq!(value["supports_thinking"], serde_json::Value::Bool(false));

        // A model that is not on disk is left exactly as stored, not guessed.
        let gone_row = queries::get_model(&conn, "model-gone").unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&gone_row.metadata_json).unwrap();
        assert!(
            value.get("supports_tools").is_none(),
            "a file that cannot be re-read must not gain a plausible-looking answer"
        );

        // Idempotent: the second pass has nothing left to do.
        assert_eq!(backfill_capability_metadata_on(&conn).unwrap(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// T-038 — a catalogue row written before the geometry fields existed must
    /// gain them at startup: without this, an already-imported hybrid model
    /// keeps `full_attention_interval: null`, which the estimator reads as
    /// "every layer holds a KV cache" — the 4× overestimate T-038 removes. The
    /// installed catalogue is the only thing the screens see, so a code fix
    /// alone would leave it wrong.
    #[test]
    fn test_t038_backfill_repairs_the_stored_geometry_only_and_is_idempotent() {
        let conn = crate::db::open_in_memory().unwrap();

        let tmp = std::env::temp_dir().join(format!("lm-mgr-t038-backfill-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        // The owner's real shape, in miniature: a hybrid attention/SSM model
        // with one MTP layer, all of it arch-prefixed.
        let hybrid = tmp.join("hybrid.gguf");
        std::fs::write(
            &hybrid,
            minimal_gguf(&[
                (
                    "general.architecture",
                    GgufValue::String("qwen35".to_string()),
                ),
                ("qwen35.block_count", GgufValue::UInt32(65)),
                ("qwen35.attention.key_length", GgufValue::UInt32(256)),
                ("qwen35.attention.value_length", GgufValue::UInt32(256)),
                ("qwen35.full_attention_interval", GgufValue::UInt32(4)),
                ("qwen35.ssm.state_size", GgufValue::UInt32(128)),
                ("qwen35.ssm.inner_size", GgufValue::UInt32(6144)),
                ("qwen35.ssm.group_count", GgufValue::UInt32(16)),
                ("qwen35.ssm.conv_kernel", GgufValue::UInt32(4)),
                ("qwen35.nextn_predict_layers", GgufValue::UInt32(1)),
            ]),
        )
        .unwrap();

        // A dense model with none of the geometry: every key is inserted as
        // null, which is the honest reading of a file that declares none.
        let dense = tmp.join("dense.gguf");
        std::fs::write(
            &dense,
            minimal_gguf(&[
                (
                    "general.architecture",
                    GgufValue::String("llama".to_string()),
                ),
                ("llama.block_count", GgufValue::UInt32(32)),
            ]),
        )
        .unwrap();

        let gone = tmp.join("gone.gguf");

        // The metadata as a pre-T-038 import wrote it: no geometry keys, and a
        // user-visible field (`context_length`) that must survive untouched.
        let stored_metadata = "{\"architecture\":\"qwen35\",\"param_count\":27000000000,\
\"quantization\":\"NVFP4\",\"block_count\":65,\"context_length\":200000,\
\"embedding_length\":null,\"attention_head_count\":null,\"attention_head_count_kv\":null,\
\"has_chat_template\":true,\"is_moe\":false,\"expert_count\":null,\
\"is_draft_model\":false,\"has_mtp_heads\":true}"
            .to_string();

        let mut row = queries::ModelRow {
            id: "model-hybrid".to_string(),
            display_name: "hybrid.gguf".to_string(),
            served_name: "hybrid.gguf".to_string(),
            file_path: hybrid.to_string_lossy().to_string(),
            shard_paths_json: "[]".to_string(),
            size_bytes: 1024,
            sha256_head: "abc".to_string(),
            metadata_json: stored_metadata.clone(),
            compatibility_json: "\"Supported\"".to_string(),
            availability: "Present".to_string(),
            launch_params_json: serde_json::to_string(&LaunchParams::default()).unwrap(),
            sampling_json: serde_json::to_string(&SamplingDefaults::default()).unwrap(),
            preload: false,
            pinned: false,
            added_at: Utc::now(),
            last_launched_at: None,
        };
        queries::insert_model(&conn, &row).unwrap();

        row.id = "model-dense".to_string();
        row.served_name = "dense.gguf".to_string();
        row.file_path = dense.to_string_lossy().to_string();
        queries::insert_model(&conn, &row).unwrap();

        row.id = "model-gone".to_string();
        row.served_name = "gone.gguf".to_string();
        row.file_path = gone.to_string_lossy().to_string();
        queries::insert_model(&conn, &row).unwrap();

        let corrected = backfill_vram_metadata_on(&conn).unwrap();
        assert_eq!(
            corrected, 2,
            "both rows whose file could be re-read are corrected; the missing one is not"
        );

        let updated = queries::get_model(&conn, "model-hybrid").unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&updated.metadata_json).unwrap();
        assert_eq!(value["full_attention_interval"], serde_json::json!(4));
        assert_eq!(value["attention_key_length"], serde_json::json!(256));
        assert_eq!(value["attention_value_length"], serde_json::json!(256));
        assert_eq!(value["ssm_state_size"], serde_json::json!(128));
        assert_eq!(value["ssm_inner_size"], serde_json::json!(6144));
        assert_eq!(value["ssm_group_count"], serde_json::json!(16));
        assert_eq!(value["ssm_conv_kernel"], serde_json::json!(4));
        assert_eq!(value["mtp_layer_count"], serde_json::json!(1));
        // Nothing the row already held was rewritten.
        assert_eq!(value["context_length"], serde_json::json!(200000));
        assert_eq!(value["quantization"], serde_json::json!("NVFP4"));

        // The reading the screens do now carries the geometry, so the
        // projection is computed from the file rather than from defaults.
        let entry = model_entry_from_row(updated).unwrap();
        assert_eq!(entry.metadata.full_attention_interval, Some(4));
        assert_eq!(entry.metadata.attention_key_length, Some(256));
        assert_eq!(entry.metadata.mtp_layer_count, Some(1));

        // A model that is not on disk is left exactly as stored, not guessed.
        let gone_row = queries::get_model(&conn, "model-gone").unwrap().unwrap();
        let value: serde_json::Value = serde_json::from_str(&gone_row.metadata_json).unwrap();
        assert!(
            value.get("full_attention_interval").is_none(),
            "a file that cannot be re-read must not gain a plausible-looking answer"
        );

        // Idempotent: the second pass has nothing left to do.
        assert_eq!(backfill_vram_metadata_on(&conn).unwrap(), 0);

        let _ = std::fs::remove_dir_all(&tmp);
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
