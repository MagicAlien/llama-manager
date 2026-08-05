//! `db/` — SQLite persistence (T-003, `docs/CONTRACTS.md` §3).
//!
//! `AGENTS.md` invariant 1 scopes "no logic" to `ipc/` and "never imports
//! `tauri`" to `core/`; this module holds neither restriction, but follows
//! the same shape by convention: it stores and retrieves rows, nothing
//! more. JSON columns (`*_json`) are opaque `String`s here — encoding and
//! decoding the domain types they represent (`docs/CONTRACTS.md` §1) is a
//! later task's job, not this one's.
//!
//! Every connection this module hands out has already had
//! `PRAGMA foreign_keys = ON` applied and every known migration run
//! against it — there is no other way to get a [`rusqlite::Connection`]
//! out of this module.

// Nothing outside `#[cfg(test)]` opens a connection through here yet — the
// modules that will (T-010 onward, per `docs/TASKS.md`) haven't landed.
// See the identical note in `core/types.rs` for why `pub` alone does not
// exempt an item from `dead_code` in a `bin` crate.
#![allow(dead_code)]

pub mod migrations;
pub mod queries;

use std::path::Path;

use rusqlite::Connection;

use crate::core::types::AppError;

pub(crate) fn db_err(err: rusqlite::Error) -> AppError {
    AppError::Database {
        message: err.to_string(),
    }
}

fn init_connection(conn: &Connection) -> Result<(), AppError> {
    // Off by default in SQLite; must be re-applied on every connection,
    // per `docs/CONTRACTS.md` §3's own warning that the schema's
    // `REFERENCES` clauses (where present) are decorative without it.
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(db_err)?;
    migrations::run(conn)?;
    Ok(())
}

/// Opens (creating if absent) a SQLite database at `path`, applies
/// `PRAGMA foreign_keys = ON`, and runs every migration this binary knows.
/// Returns `AppError::SchemaTooNew` — never panics, never migrates
/// backward — if the database's recorded `schema_version` is newer than
/// this binary understands.
pub fn open(path: &Path) -> Result<Connection, AppError> {
    let conn = Connection::open(path).map_err(db_err)?;
    init_connection(&conn)?;
    Ok(conn)
}

/// Same as [`open`], against a private in-memory database. Used by tests
/// and by any caller that needs a scratch database with the real schema.
pub fn open_in_memory() -> Result<Connection, AppError> {
    let conn = Connection::open_in_memory().map_err(db_err)?;
    init_connection(&conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_in_memory_applies_pragma_and_migrations() {
        let conn = open_in_memory().expect("open_in_memory");

        let foreign_keys_on: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys_on, 1, "foreign_keys pragma must be on");

        let version: i64 = conn
            .query_row("SELECT version FROM schema_version WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 1);
    }
}
