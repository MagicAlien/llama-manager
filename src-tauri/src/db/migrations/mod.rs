//! Forward-only schema migrations (T-003, `docs/CONTRACTS.md` §3).
//!
//! `run` is idempotent: it reads `schema_version` (absent on a brand new
//! database — that reads as "no migrations applied yet", not an error),
//! applies every migration whose version is greater than what is recorded,
//! and updates `schema_version` after each one. Calling it again with
//! nothing new to apply is a no-op, not an error — that is what "run
//! idempotently on an already-migrated database" (T-003 acceptance) means
//! here: the migration *files* are not written to tolerate re-execution
//! (`CREATE TABLE` with no `IF NOT EXISTS`, matching `docs/CONTRACTS.md` §3
//! verbatim); idempotency is a property of this runner, which never
//! re-applies a version it has already recorded.
//!
//! **Never migrates backwards.** If `schema_version.version` on disk is
//! already greater than [`known_schema_version`], `run` returns
//! `AppError::SchemaTooNew` immediately, before touching anything else —
//! per `docs/CONTRACTS.md` §3's "Migrations are forward-only" — a binary
//! older than the database it opens must refuse to start, not guess.

// See the identical allow in `db/queries.rs` for why this file (loaded via
// `mod migrations;` in `db/mod.rs`) carries its own rather than relying on
// inheritance.
#![allow(dead_code)]

use rusqlite::{Connection, OptionalExtension};

use crate::core::types::AppError;
use crate::db::db_err;

/// One entry per migration file, in order. `apply` walks this list and
/// skips anything at or below the version already recorded in
/// `schema_version`. Add a migration by appending here, never by editing
/// an existing entry.
const MIGRATIONS: &[(u32, &str)] = &[(1, include_str!("0001_init.sql"))];

/// The highest schema version this binary knows how to run against.
/// A database recording anything higher is refused, not migrated backward.
pub fn known_schema_version() -> u32 {
    MIGRATIONS
        .iter()
        .map(|(version, _)| *version)
        .max()
        .expect("MIGRATIONS is never empty")
}

fn schema_version_table_exists(conn: &Connection) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'schema_version'",
        [],
        |_| Ok(()),
    )
    .optional()
    .map(|found| found.is_some())
    .map_err(db_err)
}

/// `None` means no migration has ever run against this connection's
/// database (the `schema_version` table itself does not exist yet).
fn recorded_version(conn: &Connection) -> Result<Option<u32>, AppError> {
    if !schema_version_table_exists(conn)? {
        return Ok(None);
    }
    conn.query_row(
        "SELECT version FROM schema_version WHERE id = 1",
        [],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|v| v.map(|v| v as u32))
    .map_err(db_err)
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn apply_migration(
    conn: &Connection,
    version: u32,
    sql: &str,
    first: bool,
) -> Result<(), AppError> {
    conn.execute_batch(sql).map_err(db_err)?;
    if first {
        conn.execute(
            "INSERT INTO schema_version (id, version, applied_at) VALUES (1, ?1, ?2)",
            rusqlite::params![version, now_rfc3339()],
        )
    } else {
        conn.execute(
            "UPDATE schema_version SET version = ?1, applied_at = ?2 WHERE id = 1",
            rusqlite::params![version, now_rfc3339()],
        )
    }
    .map_err(db_err)?;
    Ok(())
}

/// Runs every migration the connection's database has not yet recorded.
/// Safe to call on a fresh database and safe to call again on one that is
/// already fully migrated (T-003 acceptance: idempotent in both cases).
pub fn run(conn: &Connection) -> Result<(), AppError> {
    let known = known_schema_version();
    // Mutable: updated after each migration this call applies, so that if
    // a single `run` ever needs to walk a fresh database through more than
    // one migration (not exercised today — `MIGRATIONS` has one entry —
    // but true the moment a 0002_*.sql is added), the second migration
    // sees the first one's version rather than re-reading the pre-`run`
    // snapshot and wrongly deciding `schema_version`'s row still doesn't
    // exist.
    let mut current = recorded_version(conn)?;

    if let Some(v) = current {
        if v > known {
            return Err(AppError::SchemaTooNew { found: v, known });
        }
    }

    for (version, sql) in MIGRATIONS {
        let already_applied = current.is_some_and(|c| c >= *version);
        if already_applied {
            continue;
        }
        // `first` distinguishes "insert the one allowed schema_version row"
        // (none applied yet in this call and none recorded before it —
        // this is the migration that creates the table) from "update it"
        // (every later migration, whether recorded before this call or
        // applied earlier within it).
        let first = current.is_none();
        apply_migration(conn, *version, sql, first)?;
        current = Some(*version);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_conn() -> Connection {
        Connection::open_in_memory().expect("in-memory sqlite connection")
    }

    #[test]
    fn runs_idempotently_on_a_fresh_database() {
        let conn = fresh_conn();
        run(&conn).expect("first run applies migration 1");

        let version: i64 = conn
            .query_row("SELECT version FROM schema_version WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 1);

        let table_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'models'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 1);
    }

    #[test]
    fn runs_idempotently_on_an_already_migrated_database() {
        let conn = fresh_conn();
        run(&conn).expect("first run");
        // Second call against the same, already-migrated connection must
        // not error and must not attempt to re-run 0001_init.sql (which
        // would fail on the second `CREATE TABLE`, since it has no
        // `IF NOT EXISTS`).
        run(&conn).expect("second run must be a no-op, not an error");

        let version: i64 = conn
            .query_row("SELECT version FROM schema_version WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 1, "version must be unchanged by the no-op run");
    }

    #[test]
    fn schema_too_new_refuses_to_start_without_crashing_or_migrating_backward() {
        let conn = fresh_conn();
        run(&conn).expect("first run");

        // Simulate a database written by a future binary: bump the
        // recorded version past what this binary knows.
        conn.execute("UPDATE schema_version SET version = 99 WHERE id = 1", [])
            .unwrap();

        let result = run(&conn);
        match result {
            Err(AppError::SchemaTooNew { found, known }) => {
                assert_eq!(found, 99);
                assert_eq!(known, known_schema_version());
            }
            other => panic!("expected AppError::SchemaTooNew, got {other:?}"),
        }

        // Refusal must be clean: the recorded version is untouched (no
        // backward migration was attempted), and the schema is exactly
        // what migration 1 left it as.
        let version: i64 = conn
            .query_row("SELECT version FROM schema_version WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            version, 99,
            "must not rewrite the version it refused to run against"
        );
    }

    #[test]
    fn schema_too_new_via_the_public_open_entry_point() {
        // The end-to-end path a real startup takes: open a file-backed
        // database, tamper with it out of band (as a future binary would
        // have left it), then reopen through `db::open` and confirm the
        // refusal happens at the connection-open boundary, not just inside
        // the migration runner directly.
        let path = std::env::temp_dir().join(format!(
            "llama-manager-t003-schema-too-new-{}-{}.sqlite3",
            std::process::id(),
            now_rfc3339().replace(':', "-").replace('.', "-")
        ));
        {
            let conn = super::super::open(&path).expect("first open runs migration 1");
            conn.execute("UPDATE schema_version SET version = 99 WHERE id = 1", [])
                .unwrap();
        }

        let reopened = super::super::open(&path);
        assert!(
            matches!(reopened, Err(AppError::SchemaTooNew { found: 99, .. })),
            "reopening a too-new database must refuse to start, got {reopened:?}"
        );

        let _ = std::fs::remove_file(&path);
    }
}
