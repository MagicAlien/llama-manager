//! CRUD per table, against connections handed out by [`super::open`] /
//! [`super::open_in_memory`] — both already have `PRAGMA foreign_keys = ON`
//! set and every migration applied (T-003, `docs/CONTRACTS.md` §3).
//!
//! JSON columns (`*_json`) are stored and returned as opaque `String`s.
//! Parsing them into the domain types `docs/CONTRACTS.md` §1 defines
//! (`ModelEntry`, `LaunchParams`, ...) is a later task's job — this module
//! does not import `core::types` for anything beyond `AppError` and
//! `Backend`, and `Backend` only because `runtimes.backend` is not JSON at
//! all (see the `Display`/`FromStr` note on `Backend` in `core/types.rs`).
//!
//! No row here is ever constructed from unchecked application logic where
//! the database itself is the thing meant to reject it — `insert_runtime`
//! does not pre-check `is_active` uniqueness in Rust; `idx_runtimes_single_active`
//! does that, and the test for it asserts the error comes back as a
//! constraint violation, not an early return from this module.

// Consumed by T-010 onward, per `docs/TASKS.md` — nothing outside
// `#[cfg(test)]` calls most of this module yet. Same rationale as the
// identical allow in `db/mod.rs` and `core/types.rs`: `pub` alone does not
// exempt an item from `dead_code` in a `bin` crate, and this file is loaded
// as a separate module (`mod queries;` in `db/mod.rs`), so it carries its
// own allow rather than relying on inheritance from the parent module.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension};

use crate::core::types::{AppError, Backend};
use crate::db::db_err;

fn bool_to_i64(b: bool) -> i64 {
    if b {
        1
    } else {
        0
    }
}

fn i64_to_bool(v: i64) -> bool {
    v != 0
}

fn fmt_ts(dt: DateTime<Utc>) -> String {
    dt.to_rfc3339()
}

fn parse_ts(s: &str) -> Result<DateTime<Utc>, AppError> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|err| AppError::Database {
            message: format!("stored timestamp {s:?} is not valid RFC 3339: {err}"),
        })
}

// ─── models ─────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ModelRow {
    pub id: String,
    pub display_name: String,
    pub served_name: String,
    pub file_path: String,
    pub shard_paths_json: String,
    pub size_bytes: i64,
    pub sha256_head: String,
    pub metadata_json: String,
    pub compatibility_json: String,
    pub availability: String,
    pub launch_params_json: String,
    pub sampling_json: String,
    pub preload: bool,
    pub pinned: bool,
    pub added_at: DateTime<Utc>,
    pub last_launched_at: Option<DateTime<Utc>>,
}

// clippy::type_complexity: factored into a named type per the lint's own
// suggestion, rather than suppressed — `model_from_row` and
// `model_row_to_struct` share this exact shape, so one definition also
// keeps them from silently drifting apart.
type ModelRowTuple = (
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    i64,
    String,
    Option<String>,
);

fn model_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ModelRowTuple> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
    ))
}

const MODEL_COLUMNS: &str = "id, display_name, served_name, file_path, shard_paths, size_bytes, \
    sha256_head, metadata_json, compatibility_json, availability, launch_params_json, \
    sampling_json, preload, pinned, added_at, last_launched_at";

fn model_row_to_struct(t: ModelRowTuple) -> Result<ModelRow, AppError> {
    Ok(ModelRow {
        id: t.0,
        display_name: t.1,
        served_name: t.2,
        file_path: t.3,
        shard_paths_json: t.4,
        size_bytes: t.5,
        sha256_head: t.6,
        metadata_json: t.7,
        compatibility_json: t.8,
        availability: t.9,
        launch_params_json: t.10,
        sampling_json: t.11,
        preload: i64_to_bool(t.12),
        pinned: i64_to_bool(t.13),
        added_at: parse_ts(&t.14)?,
        last_launched_at: t.15.as_deref().map(parse_ts).transpose()?,
    })
}

pub fn insert_model(conn: &rusqlite::Connection, m: &ModelRow) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO models (id, display_name, served_name, file_path, shard_paths, size_bytes, \
         sha256_head, metadata_json, compatibility_json, availability, launch_params_json, \
         sampling_json, preload, pinned, added_at, last_launched_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            m.id,
            m.display_name,
            m.served_name,
            m.file_path,
            m.shard_paths_json,
            m.size_bytes,
            m.sha256_head,
            m.metadata_json,
            m.compatibility_json,
            m.availability,
            m.launch_params_json,
            m.sampling_json,
            bool_to_i64(m.preload),
            bool_to_i64(m.pinned),
            fmt_ts(m.added_at),
            m.last_launched_at.map(fmt_ts),
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn get_model(conn: &rusqlite::Connection, id: &str) -> Result<Option<ModelRow>, AppError> {
    let sql = format!("SELECT {MODEL_COLUMNS} FROM models WHERE id = ?1");
    let found = conn
        .query_row(&sql, params![id], model_from_row)
        .optional()
        .map_err(db_err)?;
    found.map(model_row_to_struct).transpose()
}

pub fn list_models(conn: &rusqlite::Connection) -> Result<Vec<ModelRow>, AppError> {
    let sql = format!("SELECT {MODEL_COLUMNS} FROM models ORDER BY added_at");
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map([], model_from_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter().map(model_row_to_struct).collect()
}

/// The catalogue entry for an absolute model path, if one exists. Model
/// identity is the path (`PLAN.md` §2.13) — this is the lookup the import
/// upsert and the re-import-retains-settings behaviour both key on.
pub fn find_model_by_path(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Result<Option<ModelRow>, AppError> {
    let sql = format!("SELECT {MODEL_COLUMNS} FROM models WHERE file_path = ?1");
    let found = conn
        .query_row(&sql, params![file_path], model_from_row)
        .optional()
        .map_err(db_err)?;
    found.map(model_row_to_struct).transpose()
}

/// The mutable half of a model's catalogue entry a user actually toggles
/// day to day. Other columns (`display_name`, `metadata_json`, ...) belong
/// to whichever later task re-imports or re-scans the file.
pub fn update_model_pinned_and_preload(
    conn: &rusqlite::Connection,
    id: &str,
    pinned: bool,
    preload: bool,
) -> Result<(), AppError> {
    let changed = conn
        .execute(
            "UPDATE models SET pinned = ?1, preload = ?2 WHERE id = ?3",
            params![bool_to_i64(pinned), bool_to_i64(preload), id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound {
            what: format!("model {id}"),
        });
    }
    Ok(())
}

/// The configuration half of a model's catalogue entry: launch params and
/// sampling defaults, serialized the way every other JSON column is.
/// T-035's model-detail screen saves through this; identity columns
/// (`file_path`, `metadata_json`, ...) are never touched here.
pub fn update_model_params(
    conn: &rusqlite::Connection,
    id: &str,
    launch_params_json: &str,
    sampling_json: &str,
) -> Result<(), AppError> {
    let changed = conn
        .execute(
            "UPDATE models SET launch_params_json = ?1, sampling_json = ?2 WHERE id = ?3",
            params![launch_params_json, sampling_json, id],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound {
            what: format!("model {id}"),
        });
    }
    Ok(())
}

pub fn delete_model(conn: &rusqlite::Connection, id: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM models WHERE id = ?1", params![id])
        .map_err(db_err)?;
    Ok(())
}

// ─── watched_folders ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct WatchedFolderRow {
    pub path: String,
    pub added_at: DateTime<Utc>,
    pub last_scan_at: Option<DateTime<Utc>>,
}

pub fn insert_watched_folder(
    conn: &rusqlite::Connection,
    row: &WatchedFolderRow,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO watched_folders (path, added_at, last_scan_at) VALUES (?1, ?2, ?3)",
        params![row.path, fmt_ts(row.added_at), row.last_scan_at.map(fmt_ts)],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn get_watched_folder(
    conn: &rusqlite::Connection,
    path: &str,
) -> Result<Option<WatchedFolderRow>, AppError> {
    conn.query_row(
        "SELECT path, added_at, last_scan_at FROM watched_folders WHERE path = ?1",
        params![path],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .optional()
    .map_err(db_err)?
    .map(|(path, added_at, last_scan_at)| {
        Ok(WatchedFolderRow {
            path,
            added_at: parse_ts(&added_at)?,
            last_scan_at: last_scan_at.as_deref().map(parse_ts).transpose()?,
        })
    })
    .transpose()
}

pub fn list_watched_folders(
    conn: &rusqlite::Connection,
) -> Result<Vec<WatchedFolderRow>, AppError> {
    let mut stmt = conn
        .prepare("SELECT path, added_at, last_scan_at FROM watched_folders ORDER BY path")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter()
        .map(|(path, added_at, last_scan_at)| {
            Ok(WatchedFolderRow {
                path,
                added_at: parse_ts(&added_at)?,
                last_scan_at: last_scan_at.as_deref().map(parse_ts).transpose()?,
            })
        })
        .collect()
}

pub fn update_watched_folder_last_scan(
    conn: &rusqlite::Connection,
    path: &str,
    last_scan_at: DateTime<Utc>,
) -> Result<(), AppError> {
    let changed = conn
        .execute(
            "UPDATE watched_folders SET last_scan_at = ?1 WHERE path = ?2",
            params![fmt_ts(last_scan_at), path],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound {
            what: format!("watched folder {path}"),
        });
    }
    Ok(())
}

pub fn delete_watched_folder(conn: &rusqlite::Connection, path: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM watched_folders WHERE path = ?1", params![path])
        .map_err(db_err)?;
    Ok(())
}

// ─── runtimes ───────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeRow {
    pub build_tag: String,
    pub backend: Backend,
    pub install_path: String,
    pub is_active: bool,
    pub installed_at: DateTime<Utc>,
    pub verified_flags_json: String,
    /// Raw column value. Not parsed against `core::types::RegistrationChannel`
    /// here — that enum has no `Display`/`FromStr` of its own and T-003 was
    /// only asked to round-trip `Backend` (`docs/CONTRACTS.md` §3's comment
    /// on the `backend` column). Kept as the exact stored string.
    pub registration_channel: String,
}

pub fn insert_runtime(conn: &rusqlite::Connection, r: &RuntimeRow) -> Result<(), AppError> {
    // No pre-check of `is_active` uniqueness here on purpose: T-003
    // requires the *database* to reject a second active runtime via
    // `idx_runtimes_single_active`, not application code getting there
    // first.
    conn.execute(
        "INSERT INTO runtimes (build_tag, backend, install_path, is_active, installed_at, \
         verified_flags_json, registration_channel) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            r.build_tag,
            r.backend.to_string(),
            r.install_path,
            bool_to_i64(r.is_active),
            fmt_ts(r.installed_at),
            r.verified_flags_json,
            r.registration_channel,
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

fn runtime_row_from_tuple(
    t: (String, String, String, i64, String, String, String),
) -> Result<RuntimeRow, AppError> {
    let backend = t.1.parse::<Backend>().map_err(|e| AppError::Database {
        message: format!(
            "stored runtimes.backend {:?} does not round-trip: {e:?}",
            t.1
        ),
    })?;
    Ok(RuntimeRow {
        build_tag: t.0,
        backend,
        install_path: t.2,
        is_active: i64_to_bool(t.3),
        installed_at: parse_ts(&t.4)?,
        verified_flags_json: t.5,
        registration_channel: t.6,
    })
}

const RUNTIME_COLUMNS: &str =
    "build_tag, backend, install_path, is_active, installed_at, verified_flags_json, registration_channel";

pub fn get_runtime(
    conn: &rusqlite::Connection,
    build_tag: &str,
    backend: &Backend,
) -> Result<Option<RuntimeRow>, AppError> {
    let sql =
        format!("SELECT {RUNTIME_COLUMNS} FROM runtimes WHERE build_tag = ?1 AND backend = ?2");
    let found = conn
        .query_row(&sql, params![build_tag, backend.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .optional()
        .map_err(db_err)?;
    found.map(runtime_row_from_tuple).transpose()
}

pub fn list_runtimes(conn: &rusqlite::Connection) -> Result<Vec<RuntimeRow>, AppError> {
    let sql = format!("SELECT {RUNTIME_COLUMNS} FROM runtimes ORDER BY installed_at");
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
            ))
        })
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter().map(runtime_row_from_tuple).collect()
}

/// Activating a runtime (`is_active = true`) goes through the same unique
/// index as `insert_runtime` — a second simultaneously-active runtime is
/// rejected by `idx_runtimes_single_active`, not by this function checking
/// first.
pub fn set_runtime_active(
    conn: &rusqlite::Connection,
    build_tag: &str,
    backend: &Backend,
    is_active: bool,
) -> Result<(), AppError> {
    let changed = conn
        .execute(
            "UPDATE runtimes SET is_active = ?1 WHERE build_tag = ?2 AND backend = ?3",
            params![bool_to_i64(is_active), build_tag, backend.to_string()],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound {
            what: format!("runtime {build_tag}/{backend}"),
        });
    }
    Ok(())
}

pub fn delete_runtime(
    conn: &rusqlite::Connection,
    build_tag: &str,
    backend: &Backend,
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM runtimes WHERE build_tag = ?1 AND backend = ?2",
        params![build_tag, backend.to_string()],
    )
    .map_err(db_err)?;
    Ok(())
}

/// T-024 — deactivate all runtimes. Used before activating a new one, since
/// the unique index `idx_runtimes_single_active` rejects two simultaneously
/// active rows.
pub fn deactivate_all_runtimes(conn: &rusqlite::Connection) -> Result<(), AppError> {
    conn.execute("UPDATE runtimes SET is_active = 0", [])
        .map_err(db_err)?;
    Ok(())
}

/// T-024 — count the total number of installed runtimes.
pub fn count_runtimes(conn: &rusqlite::Connection) -> Result<u32, AppError> {
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM runtimes", [], |row| row.get(0))
        .map_err(db_err)?;
    Ok(count as u32)
}

/// T-023 fills the two verification columns after it has parsed a build's
/// `llama-server.exe --help` output. `verified_flags_json` holds the verified
/// flag list *and* the health endpoint as one JSON object (the column's
/// default stays the honest unverified value, `"[]"`); `registration_channel`
/// is the enum's variant name string. A build that has never been verified
/// simply keeps its defaults — this function is only called with real values.
///
/// Like `set_runtime_active`, the row is addressed by (build_tag, backend) and
/// a missing row is a `NotFound`, not a silent no-op: verification of a build
/// that is not registered is a caller bug.
pub fn update_runtime_verification(
    conn: &rusqlite::Connection,
    build_tag: &str,
    backend: &Backend,
    verified_flags_json: &str,
    registration_channel: &str,
) -> Result<(), AppError> {
    let changed = conn
        .execute(
            "UPDATE runtimes SET verified_flags_json = ?1, registration_channel = ?2 \
             WHERE build_tag = ?3 AND backend = ?4",
            params![
                verified_flags_json,
                registration_channel,
                build_tag,
                backend.to_string()
            ],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound {
            what: format!("runtime {build_tag}/{backend}"),
        });
    }
    Ok(())
}

// ─── settings ───────────────────────────────────────────────────

/// `key -> value` upsert. Covers both "create" (key did not exist) and
/// "update" (key already had a value) — `settings` has no meaningful
/// distinction between the two, per `docs/CONTRACTS.md` §3.
pub fn set_setting(conn: &rusqlite::Connection, key: &str, value: &str) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO settings (key, value) VALUES (?1, ?2) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn get_setting(conn: &rusqlite::Connection, key: &str) -> Result<Option<String>, AppError> {
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        params![key],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .map_err(db_err)
}

pub fn list_settings(conn: &rusqlite::Connection) -> Result<Vec<(String, String)>, AppError> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings ORDER BY key")
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

pub fn delete_setting(conn: &rusqlite::Connection, key: &str) -> Result<(), AppError> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
        .map_err(db_err)?;
    Ok(())
}

// ─── launch_history ─────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct NewLaunchRecord {
    pub file_path: String,
    pub launched_at: DateTime<Utc>,
    pub params_json: String,
    pub succeeded: bool,
    pub actual_vram_bytes: Option<i64>,
    pub load_seconds: Option<f64>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LaunchHistoryRow {
    pub id: i64,
    pub file_path: String,
    pub launched_at: DateTime<Utc>,
    pub params_json: String,
    pub succeeded: bool,
    pub actual_vram_bytes: Option<i64>,
    pub load_seconds: Option<f64>,
    pub error_message: Option<String>,
}

const LAUNCH_HISTORY_COLUMNS: &str =
    "id, file_path, launched_at, params_json, succeeded, actual_vram_bytes, load_seconds, error_message";

// Same rationale as `ModelRowTuple` above: a named type instead of an
// `#[allow(clippy::type_complexity)]` suppression, shared by both
// functions that need this exact shape.
type LaunchHistoryTuple = (
    i64,
    String,
    String,
    String,
    i64,
    Option<i64>,
    Option<f64>,
    Option<String>,
);

fn launch_history_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<LaunchHistoryTuple> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        r.get(2)?,
        r.get(3)?,
        r.get(4)?,
        r.get(5)?,
        r.get(6)?,
        r.get(7)?,
    ))
}

fn launch_history_row_to_struct(t: LaunchHistoryTuple) -> Result<LaunchHistoryRow, AppError> {
    Ok(LaunchHistoryRow {
        id: t.0,
        file_path: t.1,
        launched_at: parse_ts(&t.2)?,
        params_json: t.3,
        succeeded: i64_to_bool(t.4),
        actual_vram_bytes: t.5,
        load_seconds: t.6,
        error_message: t.7,
    })
}

/// Returns the new row's `id` (`AUTOINCREMENT`). This is the write T-041
/// makes on every load attempt (`docs/CONTRACTS.md` §3: "`launch_history`
/// is functional, not bookkeeping") — not implemented by this task, but the
/// column it writes through is.
pub fn insert_launch_record(
    conn: &rusqlite::Connection,
    rec: &NewLaunchRecord,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO launch_history (file_path, launched_at, params_json, succeeded, \
         actual_vram_bytes, load_seconds, error_message) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            rec.file_path,
            fmt_ts(rec.launched_at),
            rec.params_json,
            bool_to_i64(rec.succeeded),
            rec.actual_vram_bytes,
            rec.load_seconds,
            rec.error_message,
        ],
    )
    .map_err(db_err)?;
    Ok(conn.last_insert_rowid())
}

pub fn get_launch_record(
    conn: &rusqlite::Connection,
    id: i64,
) -> Result<Option<LaunchHistoryRow>, AppError> {
    let sql = format!("SELECT {LAUNCH_HISTORY_COLUMNS} FROM launch_history WHERE id = ?1");
    let found = conn
        .query_row(&sql, params![id], launch_history_from_row)
        .optional()
        .map_err(db_err)?;
    found.map(launch_history_row_to_struct).transpose()
}

/// Newest first, matching `idx_launch_history_path`'s sort order.
pub fn list_launch_history_for_path(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Result<Vec<LaunchHistoryRow>, AppError> {
    let sql = format!(
        "SELECT {LAUNCH_HISTORY_COLUMNS} FROM launch_history WHERE file_path = ?1 ORDER BY launched_at DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let rows = stmt
        .query_map(params![file_path], launch_history_from_row)
        .map_err(db_err)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(db_err)?;
    rows.into_iter().map(launch_history_row_to_struct).collect()
}

pub fn update_launch_record_outcome(
    conn: &rusqlite::Connection,
    id: i64,
    succeeded: bool,
    actual_vram_bytes: Option<i64>,
    load_seconds: Option<f64>,
    error_message: Option<&str>,
) -> Result<(), AppError> {
    let changed = conn
        .execute(
            "UPDATE launch_history SET succeeded = ?1, actual_vram_bytes = ?2, load_seconds = ?3, \
             error_message = ?4 WHERE id = ?5",
            params![
                bool_to_i64(succeeded),
                actual_vram_bytes,
                load_seconds,
                error_message,
                id
            ],
        )
        .map_err(db_err)?;
    if changed == 0 {
        return Err(AppError::NotFound {
            what: format!("launch_history row {id}"),
        });
    }
    Ok(())
}

pub fn delete_launch_record(conn: &rusqlite::Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM launch_history WHERE id = ?1", params![id])
        .map_err(db_err)?;
    Ok(())
}

/// Every `launch_history` row for `file_path`. Used directly by the T-003
/// test asserting these rows survive deletion of the `models` row they
/// happen to share a path with — there being no foreign key is exactly
/// what makes this query still return them afterward.
pub fn count_launch_history_for_path(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Result<i64, AppError> {
    conn.query_row(
        "SELECT count(*) FROM launch_history WHERE file_path = ?1",
        params![file_path],
        |r| r.get::<_, i64>(0),
    )
    .map_err(db_err)
}

// ─── retained_model_settings ────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct RetainedModelSettingsRow {
    pub file_path: String,
    pub launch_params_json: String,
    pub sampling_json: String,
    pub preload: bool,
    pub pinned: bool,
    pub retained_at: DateTime<Utc>,
}

/// "Retain" is inherently an upsert: re-removing a model that already has
/// retained settings replaces them with the current ones rather than
/// erroring, matching `docs/CONTRACTS.md` §3's "remove it and re-add it
/// later costs nothing but the import".
pub fn upsert_retained_model_settings(
    conn: &rusqlite::Connection,
    row: &RetainedModelSettingsRow,
) -> Result<(), AppError> {
    conn.execute(
        "INSERT INTO retained_model_settings (file_path, launch_params_json, sampling_json, \
         preload, pinned, retained_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(file_path) DO UPDATE SET \
           launch_params_json = excluded.launch_params_json, \
           sampling_json = excluded.sampling_json, \
           preload = excluded.preload, \
           pinned = excluded.pinned, \
           retained_at = excluded.retained_at",
        params![
            row.file_path,
            row.launch_params_json,
            row.sampling_json,
            bool_to_i64(row.preload),
            bool_to_i64(row.pinned),
            fmt_ts(row.retained_at),
        ],
    )
    .map_err(db_err)?;
    Ok(())
}

pub fn get_retained_model_settings(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Result<Option<RetainedModelSettingsRow>, AppError> {
    conn.query_row(
        "SELECT file_path, launch_params_json, sampling_json, preload, pinned, retained_at \
         FROM retained_model_settings WHERE file_path = ?1",
        params![file_path],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        },
    )
    .optional()
    .map_err(db_err)?
    .map(
        |(file_path, launch_params_json, sampling_json, preload, pinned, retained_at)| {
            Ok(RetainedModelSettingsRow {
                file_path,
                launch_params_json,
                sampling_json,
                preload: i64_to_bool(preload),
                pinned: i64_to_bool(pinned),
                retained_at: parse_ts(&retained_at)?,
            })
        },
    )
    .transpose()
}

pub fn delete_retained_model_settings(
    conn: &rusqlite::Connection,
    file_path: &str,
) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM retained_model_settings WHERE file_path = ?1",
        params![file_path],
    )
    .map_err(db_err)?;
    Ok(())
}

// ─── schema_version ─────────────────────────────────────────────

/// Read-only from this module's perspective — `db::migrations` owns every
/// write to this table. Exposed for callers (and tests) that just need to
/// know what version a connection is at.
pub fn get_schema_version(conn: &rusqlite::Connection) -> Result<u32, AppError> {
    conn.query_row("SELECT version FROM schema_version WHERE id = 1", [], |r| {
        r.get::<_, i64>(0)
    })
    .map(|v| v as u32)
    .map_err(db_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn conn() -> rusqlite::Connection {
        crate::db::open_in_memory().expect("open_in_memory")
    }

    fn ts(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    fn sample_model(id: &str, file_path: &str) -> ModelRow {
        ModelRow {
            id: id.into(),
            display_name: "Test Model".into(),
            served_name: format!("served-{id}"),
            file_path: file_path.into(),
            shard_paths_json: "[]".into(),
            size_bytes: 4_294_967_296,
            sha256_head: "deadbeef".into(),
            metadata_json: "{}".into(),
            compatibility_json: "{}".into(),
            availability: "Present".into(),
            launch_params_json: "{}".into(),
            sampling_json: "{}".into(),
            preload: false,
            pinned: false,
            added_at: ts(2026, 8, 1),
            last_launched_at: None,
        }
    }

    // ─── models CRUD ────────────────────────────────────────────

    #[test]
    fn models_crud() {
        let c = conn();
        let m = sample_model("m1", "D:/models/test.gguf");

        // Create
        insert_model(&c, &m).unwrap();

        // Read
        let fetched = get_model(&c, "m1").unwrap().expect("model present");
        assert_eq!(fetched, m);
        assert_eq!(list_models(&c).unwrap().len(), 1);
        assert!(get_model(&c, "does-not-exist").unwrap().is_none());

        // Update
        update_model_pinned_and_preload(&c, "m1", true, true).unwrap();
        let updated = get_model(&c, "m1").unwrap().unwrap();
        assert!(updated.pinned);
        assert!(updated.preload);
        assert!(matches!(
            update_model_pinned_and_preload(&c, "missing", true, true),
            Err(AppError::NotFound { .. })
        ));

        // Delete
        delete_model(&c, "m1").unwrap();
        assert!(get_model(&c, "m1").unwrap().is_none());
    }

    #[test]
    fn update_model_params_replaces_only_the_config_columns() {
        let c = conn();
        insert_model(&c, &sample_model("m1", "D:/models/test.gguf")).unwrap();

        update_model_params(&c, "m1", r#"{"gpu_layers":35}"#, r#"{"temperature":0.7}"#).unwrap();

        let updated = get_model(&c, "m1").unwrap().unwrap();
        assert_eq!(updated.launch_params_json, r#"{"gpu_layers":35}"#);
        assert_eq!(updated.sampling_json, r#"{"temperature":0.7}"#);
        // Identity columns untouched.
        assert_eq!(updated.file_path, "D:/models/test.gguf");
        assert_eq!(updated.metadata_json, "{}");
        assert!(!updated.pinned && !updated.preload);

        // Unknown model is a typed NotFound, not a silent no-op.
        assert!(matches!(
            update_model_params(&c, "missing", "{}", "{}"),
            Err(AppError::NotFound { .. })
        ));
    }

    #[test]
    fn models_served_name_and_file_path_are_unique() {
        let c = conn();
        insert_model(&c, &sample_model("m1", "D:/models/a.gguf")).unwrap();
        let dup_path = ModelRow {
            file_path: "D:/models/a.gguf".into(),
            ..sample_model("m2", "D:/models/a.gguf")
        };
        assert!(
            insert_model(&c, &dup_path).is_err(),
            "duplicate file_path must be rejected"
        );
    }

    // ─── watched_folders CRUD ───────────────────────────────────

    #[test]
    fn watched_folders_crud() {
        let c = conn();
        let row = WatchedFolderRow {
            path: "D:/models".into(),
            added_at: ts(2026, 8, 1),
            last_scan_at: None,
        };

        insert_watched_folder(&c, &row).unwrap();
        assert_eq!(get_watched_folder(&c, "D:/models").unwrap().unwrap(), row);
        assert_eq!(list_watched_folders(&c).unwrap().len(), 1);

        update_watched_folder_last_scan(&c, "D:/models", ts(2026, 8, 2)).unwrap();
        let updated = get_watched_folder(&c, "D:/models").unwrap().unwrap();
        assert_eq!(updated.last_scan_at, Some(ts(2026, 8, 2)));
        assert!(matches!(
            update_watched_folder_last_scan(&c, "D:/missing", ts(2026, 8, 2)),
            Err(AppError::NotFound { .. })
        ));

        delete_watched_folder(&c, "D:/models").unwrap();
        assert!(get_watched_folder(&c, "D:/models").unwrap().is_none());
    }

    // ─── runtimes CRUD + Backend round-trip + unique active index ──

    #[test]
    fn backend_display_fromstr_round_trips() {
        for backend in [Backend::Cuda { major: 13 }, Backend::Vulkan, Backend::Cpu] {
            let compact = backend.to_string();
            let parsed: Backend = compact.parse().unwrap_or_else(|e| {
                panic!("Backend {backend:?} -> {compact:?} failed to parse back: {e:?}")
            });
            assert_eq!(
                parsed, backend,
                "round trip must recover the original value"
            );
        }
        assert_eq!(Backend::Cuda { major: 13 }.to_string(), "cuda:13");
        assert_eq!(Backend::Vulkan.to_string(), "vulkan");
        assert_eq!(Backend::Cpu.to_string(), "cpu");
        // Not the serde-tagged JSON form (`docs/CONTRACTS.md` §3's comment
        // on the `backend` column) — confirm the two forms actually differ,
        // so this test would fail if someone "simplified" Display to just
        // call serde_json::to_string.
        assert_ne!(
            Backend::Cuda { major: 13 }.to_string(),
            serde_json::to_string(&Backend::Cuda { major: 13 }).unwrap()
        );
        assert!("not-a-backend".parse::<Backend>().is_err());
        assert!("cuda:not-a-number".parse::<Backend>().is_err());
    }

    fn sample_runtime(build_tag: &str, backend: Backend, is_active: bool) -> RuntimeRow {
        RuntimeRow {
            build_tag: build_tag.into(),
            backend,
            // Representative of the T-022 install layout: the app data root is
            // `%LOCALAPPDATA%\LlamaManager` (not `C:\ProgramData`, which needs
            // elevation), and each build gets a per-backend directory
            // `<build_tag>-<dir>` (T-022 decision, see core/installer.rs).
            install_path: "C:/Users/test/AppData/Local/LlamaManager/runtimes/b9196-cuda_13".into(),
            is_active,
            installed_at: ts(2026, 8, 1),
            verified_flags_json: "[]".into(),
            registration_channel: "Undetermined".into(),
        }
    }

    #[test]
    fn runtimes_crud() {
        let c = conn();
        let r = sample_runtime("b9196", Backend::Cuda { major: 13 }, false);

        insert_runtime(&c, &r).unwrap();
        assert_eq!(get_runtime(&c, "b9196", &r.backend).unwrap().unwrap(), r);
        assert_eq!(list_runtimes(&c).unwrap().len(), 1);

        set_runtime_active(&c, "b9196", &r.backend, true).unwrap();
        assert!(
            get_runtime(&c, "b9196", &r.backend)
                .unwrap()
                .unwrap()
                .is_active
        );

        delete_runtime(&c, "b9196", &r.backend).unwrap();
        assert!(get_runtime(&c, "b9196", &r.backend).unwrap().is_none());
    }

    #[test]
    fn a_second_active_runtime_is_rejected_by_the_unique_index_not_application_code() {
        let c = conn();
        insert_runtime(
            &c,
            &sample_runtime("b9196", Backend::Cuda { major: 13 }, true),
        )
        .unwrap();

        // A different (build_tag, backend) key, also active: must be
        // rejected by idx_runtimes_single_active at INSERT time.
        let second = sample_runtime("b9200", Backend::Vulkan, true);
        let err = insert_runtime(&c, &second).expect_err("second active runtime must be rejected");
        let AppError::Database { message } = err else {
            panic!("expected AppError::Database carrying the constraint violation");
        };
        assert!(
            message.to_lowercase().contains("unique"),
            "error must come from the unique index, got: {message}"
        );

        // Only one row exists — the rejected insert did not partially land.
        assert_eq!(list_runtimes(&c).unwrap().len(), 1);

        // Same story via UPDATE: activate a second, already-inserted-inactive
        // runtime while the first is still active.
        insert_runtime(&c, &sample_runtime("b9201", Backend::Cpu, false)).unwrap();
        let update_err = set_runtime_active(&c, "b9201", &Backend::Cpu, true)
            .expect_err("activating a second runtime via UPDATE must also be rejected");
        assert!(matches!(update_err, AppError::Database { .. }));
    }

    // ─── settings CRUD ──────────────────────────────────────────

    #[test]
    fn settings_crud() {
        let c = conn();
        assert!(get_setting(&c, "theme").unwrap().is_none());

        set_setting(&c, "theme", "dark").unwrap(); // create
        assert_eq!(get_setting(&c, "theme").unwrap().unwrap(), "dark");

        set_setting(&c, "theme", "light").unwrap(); // update (upsert)
        assert_eq!(get_setting(&c, "theme").unwrap().unwrap(), "light");
        assert_eq!(
            list_settings(&c).unwrap(),
            vec![("theme".to_string(), "light".to_string())]
        );

        delete_setting(&c, "theme").unwrap();
        assert!(get_setting(&c, "theme").unwrap().is_none());
    }

    // ─── launch_history CRUD ────────────────────────────────────

    fn sample_launch_record(file_path: &str) -> NewLaunchRecord {
        NewLaunchRecord {
            file_path: file_path.into(),
            launched_at: ts(2026, 8, 1),
            params_json: "{}".into(),
            succeeded: true,
            actual_vram_bytes: Some(4_000_000_000),
            load_seconds: Some(3.5),
            error_message: None,
        }
    }

    #[test]
    fn launch_history_crud() {
        let c = conn();
        let id = insert_launch_record(&c, &sample_launch_record("D:/models/a.gguf")).unwrap();

        let fetched = get_launch_record(&c, id).unwrap().expect("record present");
        assert_eq!(fetched.file_path, "D:/models/a.gguf");
        assert!(fetched.succeeded);

        assert_eq!(
            list_launch_history_for_path(&c, "D:/models/a.gguf")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            count_launch_history_for_path(&c, "D:/models/a.gguf").unwrap(),
            1
        );

        update_launch_record_outcome(&c, id, false, None, None, Some("port in use")).unwrap();
        let updated = get_launch_record(&c, id).unwrap().unwrap();
        assert!(!updated.succeeded);
        assert_eq!(updated.error_message.as_deref(), Some("port in use"));
        assert!(matches!(
            update_launch_record_outcome(&c, 999_999, true, None, None, None),
            Err(AppError::NotFound { .. })
        ));

        delete_launch_record(&c, id).unwrap();
        assert!(get_launch_record(&c, id).unwrap().is_none());
    }

    #[test]
    fn launch_history_orders_newest_first() {
        let c = conn();
        insert_launch_record(
            &c,
            &NewLaunchRecord {
                launched_at: ts(2026, 8, 1),
                ..sample_launch_record("D:/m.gguf")
            },
        )
        .unwrap();
        insert_launch_record(
            &c,
            &NewLaunchRecord {
                launched_at: ts(2026, 8, 3),
                ..sample_launch_record("D:/m.gguf")
            },
        )
        .unwrap();
        insert_launch_record(
            &c,
            &NewLaunchRecord {
                launched_at: ts(2026, 8, 2),
                ..sample_launch_record("D:/m.gguf")
            },
        )
        .unwrap();

        let rows = list_launch_history_for_path(&c, "D:/m.gguf").unwrap();
        let dates: Vec<_> = rows.iter().map(|r| r.launched_at).collect();
        assert_eq!(dates, vec![ts(2026, 8, 3), ts(2026, 8, 2), ts(2026, 8, 1)]);
    }

    // ─── retained_model_settings CRUD ───────────────────────────

    fn sample_retained(file_path: &str) -> RetainedModelSettingsRow {
        RetainedModelSettingsRow {
            file_path: file_path.into(),
            launch_params_json: r#"{"n_gpu_layers":99}"#.into(),
            sampling_json: r#"{"temperature":0.8}"#.into(),
            preload: true,
            pinned: false,
            retained_at: ts(2026, 8, 1),
        }
    }

    #[test]
    fn retained_model_settings_crud() {
        let c = conn();
        let row = sample_retained("D:/models/a.gguf");

        upsert_retained_model_settings(&c, &row).unwrap(); // create
        assert_eq!(
            get_retained_model_settings(&c, "D:/models/a.gguf")
                .unwrap()
                .unwrap(),
            row
        );

        let updated_row = RetainedModelSettingsRow {
            pinned: true,
            retained_at: ts(2026, 8, 2),
            ..row.clone()
        };
        upsert_retained_model_settings(&c, &updated_row).unwrap(); // update (upsert)
        let fetched = get_retained_model_settings(&c, "D:/models/a.gguf")
            .unwrap()
            .unwrap();
        assert!(fetched.pinned);
        assert_eq!(fetched.retained_at, ts(2026, 8, 2));

        delete_retained_model_settings(&c, "D:/models/a.gguf").unwrap();
        assert!(get_retained_model_settings(&c, "D:/models/a.gguf")
            .unwrap()
            .is_none());
    }

    // ─── schema_version ─────────────────────────────────────────

    #[test]
    fn schema_version_cannot_hold_two_rows() {
        let c = conn();
        assert_eq!(get_schema_version(&c).unwrap(), 1);

        // The CHECK (id = 1) constraint, not a uniqueness constraint on a
        // shared value: id = 2 is a different primary key and would not
        // collide on PRIMARY KEY alone.
        let err = c
            .execute(
                "INSERT INTO schema_version (id, version, applied_at) VALUES (2, 1, '2026-08-01T00:00:00Z')",
                [],
            )
            .expect_err("a second schema_version row must be rejected");
        let msg = err.to_string().to_lowercase();
        assert!(
            msg.contains("check"),
            "expected a CHECK constraint violation, got: {msg}"
        );

        // Still exactly one row.
        let count: i64 = c
            .query_row("SELECT count(*) FROM schema_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    // ─── foreign_keys pragma + survives-deletion (see PROGRESS.md D-010) ──

    #[test]
    fn foreign_keys_pragma_is_on() {
        let c = conn();
        let value: i64 = c
            .pragma_query_value(None, "foreign_keys", |r| r.get(0))
            .unwrap();
        assert_eq!(value, 1);
    }

    /// T-003 acceptance (`docs/TASKS.md`): "`launch_history` and
    /// `retained_model_settings` survive deletion of the corresponding
    /// `models` row, asserted directly — they are keyed by path and carry
    /// no foreign key." This is the half of the contradiction in D-010 that
    /// `docs/CONTRACTS.md`'s own design commentary ("Removal is not
    /// amnesia") gives a stated rationale for; the "cascades" half of the
    /// same acceptance paragraph is deliberately not implemented or tested
    /// here — see PROGRESS.md D-010.
    #[test]
    fn launch_history_and_retained_settings_survive_deletion_of_the_model_row() {
        let c = conn();
        let path = "D:/models/a.gguf";

        insert_model(&c, &sample_model("m1", path)).unwrap();
        insert_launch_record(&c, &sample_launch_record(path)).unwrap();
        upsert_retained_model_settings(&c, &sample_retained(path)).unwrap();

        assert!(get_model(&c, "m1").unwrap().is_some());
        assert_eq!(count_launch_history_for_path(&c, path).unwrap(), 1);
        assert!(get_retained_model_settings(&c, path).unwrap().is_some());

        delete_model(&c, "m1").unwrap();

        assert!(
            get_model(&c, "m1").unwrap().is_none(),
            "model row itself is gone"
        );
        assert_eq!(
            count_launch_history_for_path(&c, path).unwrap(),
            1,
            "launch_history must survive deletion of the models row (no FK, per D-010)"
        );
        assert!(
            get_retained_model_settings(&c, path).unwrap().is_some(),
            "retained_model_settings must survive deletion of the models row (no FK, per D-010)"
        );
    }
}
