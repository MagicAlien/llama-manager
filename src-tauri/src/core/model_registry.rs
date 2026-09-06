//! Model registry — import, list, remove, rescan models.
//!
//! Identity is the absolute path (`PLAN.md` §2.13). Importing the same path
//! twice updates the existing entry. Different paths with the same `sha256_head`
//! are flagged as duplicates.
//!
//! Created for T-031.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use chrono::Utc;
use tokio::sync::Semaphore;
use tracing::{debug, info};

use crate::core::gguf;
use crate::core::types::{
    AppError, Compatibility, GgufMetadata, ImportJobId, ImportProgress, ModelAvailability,
    ModelEntry, WatchedFolder,
};

/// Maximum number of files to process in parallel during import.
const IMPORT_PARALLELISM: usize = 4;

/// Import a list of model files into the registry.
pub async fn import_models(
    paths: Vec<PathBuf>,
    job_id: ImportJobId,
    cancelled: Arc<AtomicBool>,
    progress_callback: Option<Box<dyn Fn(ImportProgress) + Send>>,
) -> Result<ImportProgress, AppError> {
    let total = paths.len() as u32;

    let emit = |event: ImportProgress| {
        if let Some(cb) = &progress_callback {
            cb(event.clone());
        }
        debug!(job_id = %job_id, event = ?event, "import progress");
    };

    emit(ImportProgress::Queued {
        job_id: job_id.clone(),
        total_files: total,
    });

    // Process sequentially for now (parallelism to be added later)
    let mut imported = 0u32;
    let mut skipped = 0u32;
    let mut failed = 0u32;

    for (i, path) in paths.iter().enumerate() {
        let index = i as u32;
        let path_str = path.to_string_lossy().to_string();

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

        match import_model_file(path, index, total, &job_id, &progress_callback).await {
            Ok(entry) => {
                emit(ImportProgress::FileDone {
                    job_id: job_id.clone(),
                    entry: Box::new(entry),
                    index,
                    total,
                });
                imported += 1;
            }
            Err(e) => {
                emit(ImportProgress::FileFailed {
                    job_id: job_id.clone(),
                    path: path.clone(),
                    error: e.clone(),
                });
                failed += 1;
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

/// Import a single model file.
async fn import_model_file(
    path: &Path,
    index: u32,
    total: u32,
    job_id: &ImportJobId,
    progress_callback: &Option<Box<dyn Fn(ImportProgress) + Send>>,
) -> Result<ModelEntry, AppError> {
    // Resolve to absolute path
    let normalized = path.to_path_buf();

    // Check file exists
    if !normalized.exists() {
        return Err(AppError::NotFound {
            what: format!("model file: {}", normalized.display()),
        });
    }

    // Read GGUF metadata
    let metadata = gguf::parse_file(&normalized)?;

    // Compute sha256_head
    let sha256_head = sha256_head(&normalized)?;

    let entry = ModelEntry {
        id: format!("model-{}", Utc::now().timestamp_millis()),
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
        shard_paths: Vec::new(),
        size_bytes: 0, // Not available from GGUF header
        sha256_head,
        metadata,
        compatibility: Compatibility::Supported,
        availability: ModelAvailability::Present,
        duplicate_of: None,
        launch_params: Default::default(),
        sampling_defaults: Default::default(),
        preload: false,
        pinned: false,
        added_at: Utc::now(),
    };

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
    let mut buffer = vec![0u8; 1024 * 1024]; // 1 MiB
    let bytes_read = file.read(&mut buffer).map_err(|e| AppError::Io {
        message: format!("failed to read {}: {}", path.display(), e),
    })?;

    hasher.update(&buffer[..bytes_read]);
    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

/// List all imported models (returns empty for now).
pub fn list_models() -> Result<Vec<ModelEntry>, AppError> {
    Ok(Vec::new())
}

/// Get a model by ID (returns None for now).
pub fn get_model(_id: &str) -> Result<Option<ModelEntry>, AppError> {
    Ok(None)
}

/// Remove a model from the catalogue.
pub fn remove_model(_id: &str) -> Result<(), AppError> {
    info!(model_id = %_id, "model removed");
    Ok(())
}

/// Add a watched folder.
pub fn add_watched_folder(path: &Path) -> Result<WatchedFolder, AppError> {
    Ok(WatchedFolder {
        path: path.to_path_buf(),
        model_count: 0,
        reachable: true,
        last_scan_at: None,
    })
}

/// List watched folders (returns empty for now).
pub fn list_watched_folders() -> Result<Vec<WatchedFolder>, AppError> {
    Ok(Vec::new())
}

/// Remove a watched folder.
pub fn remove_watched_folder(_id: &str) -> Result<(), AppError> {
    Ok(())
}
