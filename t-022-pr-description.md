## Task

`T-022` — Runtime installer

## What changed

New module `core/installer.rs` (registered in `core/mod.rs`) implementing the runtime install pipeline: download with progress events → SHA-256 verification → zip-slip-safe extraction → `runtimes` registration. Atomic: any failure leaves no partial install, no temp artifact, and no database row. Plus the thin IPC command `ipc::install_runtime` (registered in `main.rs` via `generate_handler!`), and the crates `zip` (2.4.2), `sha2`, `tempfile` (dev) per `PLAN.md` §3.

**Shape — three functions, not one.** `rusqlite::Connection` is not `Sync`, so it cannot live inside the future of an async Tauri command (the future must be `Send`). The pipeline is split at the database boundaries: `pre_install_on` (sync, all no-op/stale-state logic) and `register_runtime_on` (sync, the DB insert) own every DB access; `download_verify_extract` is the only async stage and touches no database. `ipc::install_runtime` orchestrates the three. `core/` never imports `tauri` (invariant 1): the async stage takes a `FnMut(InstallProgress)` callback; the command bridges it to the `install-progress` event.

**Pipeline and rollback points** (documented in the module doc):

```text
pre_install   runtimes check + stale-state cleanup      (sync, DB)
download      -> <root>/.partial-<tag>-<dir>.zip       (R1: interrupted/failed -> delete temp file)
verify        -> SHA-256 of the temp file               (R2: mismatch -> delete temp; nothing extracted yet)
extract       -> <root>/.extract-<tag>-<dir>-<pid>/    (R3: unsafe entry or write failure -> delete temp dir + temp file)
move          -> <root>/<tag>-<dir>/                    (R4: rename failure -> same as R3)
register      -> runtimes row                           (R5: insert failure -> delete moved dir + temp file)
```

A drop-guard struct deletes whatever is still unconsumed on any failure path; after a successful rename it stops touching the target, and after `Ok` it stops entirely.

**Decisions recorded here** (the documents never name the install root; the task says to decide and record):

- **Install root: `%LOCALAPPDATA%\LlamaManager\runtimes`** — the app's data root is already `%LOCALAPPDATA%\LlamaManager\` (T-001's logs live in `<root>\logs`), matching `PLAN.md` §2.1's "app-owned directories under `%LOCALAPPDATA%\LlamaManager\`". Not configurable in T-022: no settings plumbing exists before T-050. `C:\ProgramData` (the path in the old `db/queries.rs` test fixture) is *not* used: it needs elevation to create on standard Windows 11 installs, which the app does not request. The fixture was corrected to the decided layout (see below). Verified empirically on this machine: a non-elevated session *can* create a directory in `C:\ProgramData` here — but that is not standard Windows 11 behavior, so the decision does not depend on it.
- **Per-backend subdirectory.** `install_path` is `<root>/<build_tag>-<dir>`, where `<dir>` is the compact backend form with `:` replaced by `_` (`b9196-cuda_13`, `b9196-vulkan`, `b9196-cpu`). Two backends of one build tag are two different archives with overlapping file names, so they must not share a directory; the colon is replaced because it is reserved for alternate data streams in Windows path syntax.
- **No-op re-install.** "Existing build" = a `runtimes` row for `(build_tag, backend)` *and* the files at its `install_path` still exist: return the stored `RuntimeBuild` and emit only `Done` — no network, no extraction. If the row exists but the directory is gone (hand-deleted, or a crash whose cleanup ran before registration), the stale row is deleted and the install proceeds fresh — a row pointing at nothing is corrupt state, not an install. A target directory with no matching row (debris from a crashed install) is removed the same way.
- **Missing checksum.** `AvailableRelease.sha256` is `None` for every real llama.cpp release today (F-008: no checksum assets are published). The installer then *cannot* verify — it emits a `tracing::warn!` line (visible in the app log) and proceeds, rather than skipping verification silently. There is no UI yet (T-024) and `InstallProgress` has no warning variant (contract type, not extended here), so the app log is the only surface this task can use.
- **Zip-slip.** The zip crate's `extract()` sanitizes rather than rejects (absolute paths are relocated into the destination, not refused — confirmed against the vendored 2.4.2 source: `safe_prepare_path` is `pub(crate)` and strips leading absolute prefixes). This module therefore validates every entry name itself, in a first pass over *all* entries before any write: `..` components, absolute paths, drive letters, and symlink entries each abort the whole extraction with `AppError::UnsafeArchiveEntry`. Because validation precedes writing, a rejected archive leaves nothing on disk.
- **Progress without tauri.** `core/` never imports tauri (invariant 1): `download_verify_extract` takes a `FnMut(InstallProgress)` callback; `ipc::install_runtime` bridges it to the `install-progress` event. The `Resolving` event is emitted by the command (resolution happens there, before core is reached); core emits `Downloading` … `Extracting`; the command emits `Registering` and `Done` (and `Failed { error }` on any failure, in addition to returning the error — the event keeps the UI's progress state consistent, the returned error is what an `invoke` caller catches).
- **Database file: `<data root>\llama-manager.db`.** No document names a DB location; this follows the data-root decision above. The command creates the parent directory (`rusqlite` opens files but never creates directories).

## Acceptance criteria

- [x] **An interrupted download leaves no partial install and reports a clean error.** `interrupted_download_leaves_no_partial_install`: the test server declares the full `Content-Length` but sends half the body and closes — `reqwest` ends the stream early, the installer's length check catches it, the error is `AppError::Network` naming the truncation, and no target directory, no temp file, no row exist afterward.
- [x] **Checksum mismatch aborts before extraction (`AppError::ChecksumMismatch`).** `checksum_mismatch_aborts_before_extraction`: a wrong published digest produces `ChecksumMismatch { expected, actual }` with the real 64-hex digest as `actual`; no extract directory, no target, no row, and the `Verifying` event proves the download completed first.
- [x] **Re-installing an existing build is a no-op.** `reinstalling_an_existing_build_is_a_no_op`: a registered build whose files exist returns the stored row (same `installed_at`), emits only `Done`, and performs no network I/O (the asset URL points at a closed port — success proves nothing was fetched). `row_without_files_reinstalls_fresh` covers the recorded decision: a stale row without files is deleted and the build reinstalled fresh.
- [x] **Disk-full during extraction rolls back.** `disk_full_during_extraction_rolls_back` — simulated without a real full disk: the fixture archive contains `bin/llama-server.exe`; pre-creating `<extract>/bin` as a *file* makes that entry's parent-directory creation fail with an I/O error, the same failure class ENOSPC produces. The rollback is identical: no target, no extract directory, no temp file, no row.
- [x] **Zip-slip rejected: an entry whose normalized destination escapes the target (`../`, absolute path, drive letter, or a symlink entry) aborts the entire extraction with `AppError::UnsafeArchiveEntry` — covered by a malicious fixture archive.** `zip_slip_entries_are_rejected_and_write_nothing` builds four real archives with the zip crate's own writer (hostile names are stored verbatim — the writer performs no sanitization): `../evil.txt`, `/abs-evil.txt`, `C:\evil-drive.txt`, and a symlink entry `evil-link → ../outside-target`. Each aborts with `UnsafeArchiveEntry` and writes nothing — not even the safe entries that precede the offender, because validation is a first pass over all entries. `validate_entry_name_table` unit-tests the validator directly (8 unsafe names, 4 safe).
- [x] **A release without a published checksum surfaces a warning rather than skipping verification silently.** `missing_checksum_warns_and_installs`: `sha256: None` proceeds with a `tracing::warn!` line (the common case today, F-008) and installs normally.

## Invariants (`AGENTS.md` §6)

- [x] `ipc/` contains no logic; `core/` does not import `tauri` — `core/installer.rs`'s only non-stdlib imports are `chrono`, `rusqlite`, `sha2`, `tracing`, `zip`, and `crate::core::types`/`crate::db::queries`; `ipc::install_runtime` deserializes `(tag, backend)`, resolves the release via T-020, opens the DB, and orchestrates the three core functions.
- [x] No shared `Mutex<ServerState>` introduced — the command takes an injected `AppHandle` (Tauri's standard mechanism, no global state).
- [x] Estimator remains pure (history passed as an argument) — n/a, this PR does not touch the estimator.
- [x] No `unwrap()` / `expect()` in `core/` or `ipc/` outside tests — `core/` carries `#![deny(clippy::unwrap_used, clippy::expect_used)]` and the module compiles clean under it (verified by grep: zero `unwrap`/`expect` outside the `#[cfg(test)]` module).
- [x] No string literals in JSX text positions — n/a, no frontend code touched (`InstallProgress` and `RuntimeBuild` are existing contract types; `types.ts` was not regenerated because this PR exports no new type).
- [x] No model file copied, moved without consent, or deleted
- [x] Removal retains parameters and launch history keyed by path (`PLAN.md` §2.9`) — n/a.
- [x] `src/lib/types.ts` not hand-edited — untouched.

## Endpoint invariants (`AGENTS.md` invariants 3–4, `PLAN.md` §2.7)

- [x] n/a — this PR does not touch the request path

## Flags used

| Flag | In verified list? | Value type matches? |
|---|---|---|
| — | — | — |

- [x] No flag was used that is absent from the verified list (except `extra_args`) — n/a, this PR emits no llama.cpp flags.
- [x] Any omitted flag emits a `preset-warning` event — n/a.

## Correction task (`T-1xx` only)

n/a — this is an ordinary task.

## Dependencies

- [x] No new dependency, **or** the new dependency is listed in `PLAN.md` §3
- New dependency and justification: `zip` (archive extraction), `sha2` (verification), `tempfile` (dev-dependency, test scratch directories). All three are named in `PLAN.md` §3's approved-crate list; none is re-justified here. `zip` resolves to 2.4.2, which is past the symlink-traversal fix of CVE-2025-29787 (fixed in 2.3.0).

## Excluded scope check (`AGENTS.md` §2)

- [x] No chat UI, message composer, or user-facing inference
- [x] No Hugging Face downloading
- [x] No cloud sync, accounts, or telemetry upload

## Security

- [x] No secret, absolute path from my machine, or model file committed — the test fixture uses the representative placeholder `C:/Users/test/AppData/Local/LlamaManager/...` (no real user name).
- [x] If this PR touches the API key: it is never logged, never sent upstream, never returned by an IPC command — n/a.
- [x] If this PR touches the request log: no body, header value, or model name is recorded — n/a.

## Empirical questions raised

- **The zip crate's `extract()` sanitizes rather than rejects** (confirmed against the vendored 2.4.2 source and the crate's issue history; the symlink-traversal CVE-2025-29787 was fixed in 2.3.0, so 2.4.2 is out of range). Absolute paths are relocated into the destination, not refused — which is why this module validates entry names itself instead of delegating to `extract()`. Recorded in Facts established.
- **`Path::is_absolute()` is `false` for POSIX-style `/foo` on Windows** (it is rooted at the current drive, which is exactly what makes it dangerous: joining it onto the extraction directory would escape it). The entry-name validator therefore checks the raw leading byte (`/` or `\`) explicitly, plus drive letters and `..` components — the platform-dependent `is_absolute()` alone would let `/abs-evil.txt` through.
- **`reqwest` 0.13 `Response::chunk()` returns `Err` (not `None`) when the stream closes early** (verified empirically against a local stub server that declares the full `Content-Length` and then closes). The installer's length check (`received != total`) covers the `None` case; both paths abort with `AppError::Network`.
- **Windows elevation on this machine is non-standard**: a non-elevated session can create a directory in `C:\ProgramData` (verified with `mkdir`). Standard Windows 11 installs deny this, which is why the install-root decision does not depend on it — `%LOCALAPPDATA%` is writable by the user in every case.
- **`C:\ProgramData` vs `%LOCALAPPDATA%`** — the `db/queries.rs` test fixture used `C:/ProgramData/LlamaManager/runtimes/b9196`; the decision above moves it to the `%LOCALAPPDATA%` layout and the fixture was corrected to `C:/Users/test/AppData/Local/LlamaManager/runtimes/b9196-cuda_13` (including the per-backend directory form).

## PROGRESS.md

- [ ] Task moved to `Done` with its PR number and date — **filled after the merge is confirmed** (this PR is still open).
- [x] Any discrepancy or ambiguity recorded in Discrepancies (not worked around) — the install root is not named in any document; the decision (and the elevation finding) are recorded here and in the module doc, not silently picked.
- [x] Any out-of-scope problem noticed recorded in Observations (not fixed here) — the T-021 entry's note "the `#![allow(dead_code)]` is removed when T-022 wires the caller" is stale: `ipc::install_runtime` takes `(tag, backend)` from the caller (per `CONTRACTS.md` §4) and resolves the release directly via T-020 — it does not call `select_backend`, whose consumer is the UI (T-024). The `allow` stays.
- [x] Any discovery a later task depends on recorded in Facts established — the install root and DB file location (T-024/T-025/T-03x consume them), the zip-crate sanitization behavior, the Windows `Path::is_absolute()` gap, and `reqwest`'s `chunk()` early-close behavior.
- [x] If this is T-023: … — n/a.
- [x] If this is T-025: … — n/a.
