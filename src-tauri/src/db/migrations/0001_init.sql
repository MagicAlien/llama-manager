-- T-003 — docs/CONTRACTS.md §3, transcribed verbatim from that section.
--
-- Forward-only: this file is never edited once merged. A future schema
-- change is a new numbered file (0002_*.sql etc.) plus an entry in
-- `MIGRATIONS` in `db/migrations/mod.rs`; `db::migrations::run` tracks which
-- version has already been applied via `schema_version` and never re-runs
-- a file whose version is <= the recorded one.
--
-- `PRAGMA foreign_keys = ON` is set by `db::init_connection` on every
-- connection, not in this file — a `PRAGMA` is per-connection state in
-- SQLite, not part of the persisted schema, so it has to be re-applied
-- every time a connection opens rather than baked into a migration.
--
-- `launch_history` and `retained_model_settings` deliberately carry no
-- `FOREIGN KEY` to `models`: they are keyed by absolute path and survive
-- deletion of the corresponding `models` row (`PLAN.md` §2.9, "Removal is
-- not amnesia"). See PROGRESS.md D-010 for the discrepancy this resolves
-- against — `docs/TASKS.md` T-003's acceptance text and `docs/CONTRACTS.md`
-- §3's intro line both read, in isolation, as if a FOREIGN KEY / cascade
-- exists here; it deliberately does not, per CONTRACTS' own design
-- commentary, which is the only version of this with a stated rationale.

CREATE TABLE schema_version (
  id          INTEGER PRIMARY KEY CHECK (id = 1),   -- exactly one row, always
  version     INTEGER NOT NULL,
  applied_at  TEXT NOT NULL
);

CREATE TABLE models (
  id                 TEXT PRIMARY KEY,
  display_name       TEXT NOT NULL,
  served_name        TEXT NOT NULL UNIQUE,
  file_path          TEXT NOT NULL UNIQUE,   -- absolute; the model's identity
  shard_paths        TEXT NOT NULL DEFAULT '[]',   -- JSON array
  size_bytes         INTEGER NOT NULL,
  sha256_head        TEXT NOT NULL,          -- duplicate signal; deliberately not unique
  metadata_json      TEXT NOT NULL,
  compatibility_json TEXT NOT NULL,
  availability       TEXT NOT NULL DEFAULT 'Present',
  launch_params_json TEXT NOT NULL DEFAULT '{}',
  sampling_json      TEXT NOT NULL DEFAULT '{}',
  preload            INTEGER NOT NULL DEFAULT 0,
  pinned             INTEGER NOT NULL DEFAULT 0,
  added_at           TEXT NOT NULL,
  last_launched_at   TEXT
);

CREATE INDEX idx_models_sha_head ON models(sha256_head);
-- ModelEntry.duplicate_of has no column: it is computed at query time from this
-- index. Storing it would need updating on every insert and delete, and would go
-- stale exactly when it matters.

CREATE TABLE watched_folders (
  path         TEXT PRIMARY KEY,   -- absolute, user-chosen; rescanned for new models
  added_at     TEXT NOT NULL,
  last_scan_at TEXT
);

CREATE TABLE runtimes (
  build_tag            TEXT NOT NULL,
  backend              TEXT NOT NULL,   -- compact form of Backend: 'cuda:13', 'vulkan', 'cpu'
                                        -- (Display/FromStr, not the serde tagged form;
                                        --  it is half a primary key, so it must be short
                                        --  and stable. Round-trip asserted in T-003.)
  install_path         TEXT NOT NULL,
  is_active            INTEGER NOT NULL DEFAULT 0,
  installed_at         TEXT NOT NULL,
  verified_flags_json  TEXT NOT NULL DEFAULT '[]',
  registration_channel TEXT NOT NULL DEFAULT 'Undetermined',
  PRIMARY KEY (build_tag, backend)
);

-- At most one active runtime, enforced by the database rather than by care.
CREATE UNIQUE INDEX idx_runtimes_single_active
  ON runtimes(is_active) WHERE is_active = 1;

CREATE TABLE settings (
  key    TEXT PRIMARY KEY,
  value  TEXT NOT NULL          -- api_key holds DPAPI ciphertext, never plaintext
);

-- Keyed by absolute path, not by entry id, and with no foreign key: calibration
-- outlives the catalogue entry, so removing and re-adding a file does not reset
-- the estimator to Heuristic. See PLAN.md §2.9.
CREATE TABLE launch_history (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  file_path         TEXT NOT NULL,
  launched_at       TEXT NOT NULL,
  params_json       TEXT NOT NULL,
  succeeded         INTEGER NOT NULL,
  actual_vram_bytes INTEGER,
  load_seconds      REAL,
  error_message     TEXT
);

CREATE INDEX idx_launch_history_path ON launch_history(file_path, launched_at DESC);

-- Parameters and sampling defaults survive removal the same way, so that
-- "remove it and re-add it later" costs nothing but the import.
CREATE TABLE retained_model_settings (
  file_path          TEXT PRIMARY KEY,
  launch_params_json TEXT NOT NULL,
  sampling_json      TEXT NOT NULL,
  preload            INTEGER NOT NULL DEFAULT 0,
  pinned             INTEGER NOT NULL DEFAULT 0,
  retained_at        TEXT NOT NULL
);
