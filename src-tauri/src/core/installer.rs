//! T-022 — Runtime installer.
//!
//! Downloads a release asset with progress events, verifies its SHA-256,
//! extracts it (zip-slip-safe), and registers it in the `runtimes` table.
//! Atomic: any failure leaves no partial install and no database row.
//!
//! ## Pipeline and rollback points
//!
//! ```text
//! pre_install   runtimes check + stale-state cleanup      (sync, DB)
//! download      -> <root>/.partial-<tag>-<dir>.zip       (R1: interrupted or
//!                                                          failed download ->
//!                                                          delete temp file)
//! verify        -> SHA-256 of the temp file               (R2: mismatch ->
//!                                                          delete temp; nothing
//!                                                          extracted yet)
//! extract       -> <root>/.extract-<tag>-<dir>-<pid>/    (R3: unsafe entry or
//!                                                          any write failure ->
//!                                                          delete temp dir +
//!                                                          temp file)
//! move          -> <root>/<tag>-<dir>/                    (R4: rename failure
//!                                                          -> same as R3)
//! register      -> runtimes row                           (R5: insert failure
//!                                                          -> delete the moved
//!                                                          dir + temp file)
//! ```
//!
//! "Disk full during extraction rolls back" is testable without a real full
//! disk: the extraction writes through ordinary `std::fs` calls, so a test
//! pre-creates one of the entry's parent paths as a *file*, which makes that
//! entry's write fail with an I/O error. The rollback path is identical to a
//! genuine ENOSPC — see `disk_full_during_extraction_rolls_back`.
//!
//! ## Shape: why three functions instead of one
//!
//! `rusqlite::Connection` is not `Sync`, so it cannot live inside the future
//! of an async Tauri command (the future must be `Send`). The pipeline is
//! therefore split at the database boundaries: [`pre_install_on`] and
//! [`register_runtime_on`] are synchronous and own all DB access;
//! [`download_verify_extract`] is the only async stage and touches no
//! database. `ipc::install_runtime` orchestrates the three, which keeps every
//! piece of logic in this module — the command itself is one line per stage.
//!
//! ## Decisions recorded here (see the T-022 PR description)
//!
//! - **Install root.** `%LOCALAPPDATA%\LlamaManager\runtimes` — the app's data
//!   root is already `%LOCALAPPDATA%\LlamaManager\` (T-001's logs live in
//!   `<root>\logs`), matching `PLAN.md` §2.1's "app-owned directories under
//!   `%LOCALAPPDATA%\LlamaManager\`". Not configurable in T-022: no settings
//!   plumbing exists before T-050, and a hardcoded root is the only source of
//!   truth this task can have. `C:\ProgramData` (the path used by an old test
//!   fixture) is *not* used: it needs elevation to create on standard Windows
//!   11 installs, which the app does not request — verified empirically on a
//!   non-elevated session where `%LOCALAPPDATA%\LlamaManager` writes succeed.
//! - **Per-backend subdirectory.** `install_path` is `<root>/<build_tag>-<dir>`,
//!   where `<dir>` is the compact backend form with `:` replaced by `_`
//!   (`b9196-cuda_13`, `b9196-vulkan`, `b9196-cpu`). Two backends of one build
//!   tag are two different archives with overlapping file names, so they must
//!   not share a directory; the colon is replaced because it is reserved for
//!   alternate data streams in Windows path syntax.
//! - **No-op re-install.** "Existing build" = a `runtimes` row for
//!   `(build_tag, backend)` *and* the files at its `install_path` still exist:
//!   return the stored `RuntimeBuild` without downloading. If the row exists
//!   but the directory is gone (hand-deleted, or a crash whose temp cleanup ran
//!   before registration), the stale row is deleted and the install proceeds
//!   fresh — a row pointing at nothing is corrupt state, not an install. If
//!   the download then fails, the stale row is already gone; the user retries.
//! - **Orphaned target directory.** A target directory with no matching row is
//!   leftover debris from a crashed install (files moved into place, DB write
//!   lost). The directory is app-owned, so it is removed and the install
//!   proceeds fresh.
//! - **Missing checksum.** `AvailableRelease.sha256` is `None` for every real
//!   llama.cpp release today (T-020's research: no checksum assets are
//!   published). The installer then *cannot* verify — it emits a
//!   `tracing::warn!` line (visible in the app log) and proceeds, rather than
//!   skipping verification silently. There is no UI yet (T-024) and
//!   `InstallProgress` has no warning variant (contract type, not extended
//!   here), so the app log is the only surface this task can use.
//! - **Zip-slip.** The zip crate's `extract()` sanitizes rather than rejects
//!   (absolute paths are relocated into the destination, not refused —
//!   confirmed against the vendored 2.4.2 source and the crate's issue
//!   history), so this module validates every entry name itself, in a first
//!   pass over *all* entries before any write: `..` components, absolute paths,
//!   drive letters, and symlink entries each abort the whole extraction with
//!   `AppError::UnsafeArchiveEntry`. Because validation precedes writing, a
//!   rejected archive leaves nothing on disk.
//! - **Progress without tauri.** `core/` never imports tauri (invariant 1):
//!   [`download_verify_extract`] takes a `FnMut(InstallProgress)` callback; the
//!   `ipc::install_runtime` command bridges it to the `install-progress` event.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

use chrono::Utc;
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use tracing::warn;
use zip::ZipArchive;

use crate::core::types::{
    AppError, AvailableRelease, Backend, InstallProgress, RegistrationChannel, RuntimeBuild,
    VerifiedFlag,
};
use crate::db::queries::{delete_runtime, get_runtime, insert_runtime, RuntimeRow};

/// The app's data root: `%LOCALAPPDATA%\LlamaManager`. T-001 already creates
/// `<root>\logs` here; runtimes and the database live alongside it.
pub fn data_root() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| PathBuf::from(base).join("LlamaManager"))
}

/// Where runtime builds are installed: `<data root>\runtimes`.
pub fn install_root() -> Option<PathBuf> {
    data_root().map(|root| root.join("runtimes"))
}

/// The app database file: `<data root>\llama-manager.db`. No document names a
/// DB location yet; this follows the data-root decision above and is recorded
/// in the T-022 PR description.
pub fn database_path() -> Option<PathBuf> {
    data_root().map(|root| root.join("llama-manager.db"))
}

/// The on-disk directory name for a backend: the compact form (`cuda:13`) with
/// `:` replaced by `_` (colons are reserved in Windows path syntax).
fn backend_dir(backend: &Backend) -> String {
    backend.to_string().replace(':', "_")
}

/// The install target directory: `<root>/<build_tag>-<dir>`.
pub fn target_dir(root: &Path, tag: &str, backend: &Backend) -> PathBuf {
    root.join(format!("{tag}-{}", backend_dir(backend)))
}

/// The extraction scratch directory. Suffixes with the process id so a crash
/// cannot leave a half-extracted tree that the next run mistakes for stale
/// state it must delete (and so two builds in one process never collide).
pub fn extract_dir(root: &Path, tag: &str, backend: &Backend) -> PathBuf {
    root.join(format!(
        ".extract-{tag}-{}-{}",
        backend_dir(backend),
        std::process::id()
    ))
}

/// The partial-download temp file. Deterministic on purpose: a crashed
/// previous attempt of the *same* install is exactly what gets cleaned up.
pub fn partial_file(root: &Path, tag: &str, backend: &Backend) -> PathBuf {
    root.join(format!(".partial-{tag}-{}.zip", backend_dir(backend)))
}

/// Outcome of the pre-install check (see [`pre_install_on`]).
pub enum PreCheckResult {
    /// A `runtimes` row for `(build_tag, backend)` exists *and* its files are
    /// still on disk: return the stored build, emit `Done`, download nothing.
    Existing(RuntimeBuild),
    /// Fresh install (or a stale row whose files were lost — it has already
    /// been deleted).
    Proceed,
}

/// The no-op check plus all stale-state cleanup, against an explicit
/// connection (tests point it at an in-memory database; production callers
/// open [`database_path()`]). Synchronous on purpose: `Connection` is not
/// `Sync`, so this must never run inside an async future.
pub fn pre_install_on(
    conn: &Connection,
    tag: &str,
    backend: &Backend,
    root: &Path,
) -> Result<PreCheckResult, AppError> {
    match get_runtime(conn, tag, backend)? {
        Some(row) if Path::new(&row.install_path).exists() => {
            Ok(PreCheckResult::Existing(row_to_build(&row)))
        }
        Some(_row) => {
            // Row without files: corrupt state (hand-deleted directory, or a
            // crash that lost the registration step). Drop the stale row and
            // let the caller reinstall.
            delete_runtime(conn, tag, backend)?;
            Ok(PreCheckResult::Proceed)
        }
        None => {
            let target = target_dir(root, tag, backend);
            if target.exists() {
                // App-owned directory with no row: debris from a crashed
                // install. Remove it so the fresh install starts clean.
                std::fs::remove_dir_all(&target).map_err(io_err)?;
            }
            Ok(PreCheckResult::Proceed)
        }
    }
}

/// The result of [`download_verify_extract`]: files in place at `target`, and
/// the still-on-disk partial file (deleted by [`register_runtime_on`] once the
/// row lands — deleting it earlier would leave a crash window with files but
/// no row).
pub struct ExtractedInstall {
    pub target: PathBuf,
    pub partial: PathBuf,
}

/// Download `release.asset_url`, verify its SHA-256, and extract it into the
/// install target. No database access (see the module doc's "Shape" note).
///
/// `root` is a parameter (not the process-global one) so tests point the whole
/// install at a scratch directory; production callers pass [`install_root()`].
/// The progress callback receives `Downloading` … `Extracting` events in order.
pub async fn download_verify_extract(
    client: &reqwest::Client,
    release: &AvailableRelease,
    root: &Path,
    progress: &mut impl FnMut(InstallProgress),
) -> Result<ExtractedInstall, AppError> {
    let tag = &release.build_tag;
    let backend = &release.backend;

    std::fs::create_dir_all(root).map_err(io_err)?;

    let partial = partial_file(root, tag, backend);
    // Stale leftover from a previous crashed attempt of the *same* install.
    let _ = std::fs::remove_file(&partial);

    // Deletes whatever has not been consumed yet on any failure path (R1–R4 in
    // the module doc). After a successful rename `extract` is set to `None`;
    // after the function returns `Ok`, `done` stops the drop from touching
    // anything at all.
    let mut cleanup = InstallCleanup {
        partial: Some(partial.clone()),
        extract: None,
        done: false,
    };

    // ── R1: download ────────────────────────────────────────────────────
    let mut response =
        client
            .get(&release.asset_url)
            .send()
            .await
            .map_err(|err| AppError::Network {
                message: format!("could not download {}: {err}", release.asset_name),
            })?;
    if !response.status().is_success() {
        return Err(AppError::Network {
            message: format!(
                "download of {} failed with status {}",
                release.asset_name,
                response.status()
            ),
        });
    }

    let total = response.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&partial).map_err(io_err)?;
    let mut received: u64 = 0;
    let start = Instant::now();
    let mut last_emit = Instant::now();
    progress(InstallProgress::Downloading {
        received_bytes: 0,
        total_bytes: total,
        bytes_per_sec: 0,
    });

    loop {
        let chunk = response.chunk().await.map_err(|err| AppError::Network {
            message: format!("download of {} interrupted: {err}", release.asset_name),
        })?;
        let Some(chunk) = chunk else {
            break;
        };
        file.write_all(&chunk).map_err(io_err)?;
        received += chunk.len() as u64;
        let now = Instant::now();
        if now.duration_since(last_emit).as_millis() >= 200 {
            progress(InstallProgress::Downloading {
                received_bytes: received,
                total_bytes: total,
                bytes_per_sec: bytes_per_sec(received, start),
            });
            last_emit = now;
        }
    }
    // A server that closes the connection before the declared Content-Length is
    // satisfied ends the stream early: `chunk()` returns `None` (or errored
    // above). Neither may be treated as a complete download.
    if total > 0 && received != total {
        return Err(AppError::Network {
            message: format!(
                "download of {} incomplete: received {received} of {total} bytes",
                release.asset_name
            ),
        });
    }
    progress(InstallProgress::Downloading {
        received_bytes: received,
        total_bytes: total,
        bytes_per_sec: bytes_per_sec(received, start),
    });

    // ── R2: verify ──────────────────────────────────────────────────────
    progress(InstallProgress::Verifying);
    match &release.sha256 {
        Some(expected) => {
            let actual = sha256_file(&partial).map_err(io_err)?;
            if !expected.eq_ignore_ascii_case(&actual) {
                return Err(AppError::ChecksumMismatch {
                    expected: expected.clone(),
                    actual,
                });
            }
        }
        // The common case today (T-020: llama.cpp publishes no checksums).
        // Warn rather than skip silently — there is no UI surface yet (T-024)
        // and InstallProgress has no warning variant, so the app log carries it.
        None => warn!(
            "release {} publishes no SHA-256 checksum; installing without verification",
            release.build_tag
        ),
    }

    // ── R3: extract (zip-slip-safe, two passes) ────────────────────────
    progress(InstallProgress::Extracting {
        entries_done: 0,
        entries_total: 0,
    });
    let zip_file = std::fs::File::open(&partial).map_err(io_err)?;
    let mut archive = ZipArchive::new(zip_file).map_err(zip_err)?;
    let total_entries = archive.len() as u32;

    // Pass 1: validate *every* entry before writing anything, so a rejected
    // archive leaves nothing on disk — not even the entries that happened to
    // precede the offender.
    let mut names: Vec<String> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(zip_err)?;
        if entry.is_symlink() {
            return Err(AppError::UnsafeArchiveEntry {
                entry: entry.name().to_string(),
            });
        }
        validate_entry_name(entry.name())?;
        names.push(entry.name().to_string());
    }

    let extract = extract_dir(root, tag, backend);
    cleanup.extract = Some(extract.clone());

    progress(InstallProgress::Extracting {
        entries_done: 0,
        entries_total: total_entries,
    });
    for (i, name) in names.iter().enumerate() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        if entry.is_dir() {
            std::fs::create_dir_all(extract.join(name)).map_err(io_err)?;
        } else {
            let destination = extract.join(name);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).map_err(io_err)?;
            }
            let mut out = std::fs::File::create(&destination).map_err(io_err)?;
            std::io::copy(&mut entry, &mut out).map_err(io_err)?;
        }
        progress(InstallProgress::Extracting {
            entries_done: (i + 1) as u32,
            entries_total: total_entries,
        });
    }

    // ── R4: move into place ─────────────────────────────────────────────
    let target = target_dir(root, tag, backend);
    std::fs::rename(&extract, &target).map_err(|err| AppError::Io {
        message: format!(
            "could not move the extracted runtime into place at {}: {err}",
            target.display()
        ),
    })?;
    cleanup.extract = None;

    // The partial file stays on disk until registration succeeds (see
    // `ExtractedInstall`); the drop must not touch it.
    cleanup.done = true;

    Ok(ExtractedInstall {
        target,
        partial: partial.clone(),
    })
}

/// Insert the `runtimes` row for a build whose files are already at `target`,
/// then delete the partial file. If the insert fails, both the moved directory
/// and the partial file are removed (R5 in the module doc) — a failed
/// registration leaves no orphaned directory and no temp file. Synchronous on
/// purpose: `Connection` is not `Sync`.
pub fn register_runtime_on(
    conn: &Connection,
    tag: &str,
    backend: &Backend,
    target: &Path,
    partial: &Path,
) -> Result<RuntimeBuild, AppError> {
    let installed_at = Utc::now();
    match insert_runtime(
        conn,
        &RuntimeRow {
            build_tag: tag.to_string(),
            backend: backend.clone(),
            install_path: target.to_string_lossy().into_owned(),
            is_active: false,
            installed_at,
            // T-023 fills both of these; the schema defaults are the honest
            // values until then.
            verified_flags_json: "[]".to_string(),
            registration_channel: "Undetermined".to_string(),
        },
    ) {
        Ok(()) => {
            let _ = std::fs::remove_file(partial);
            Ok(RuntimeBuild {
                build_tag: tag.to_string(),
                backend: backend.clone(),
                install_path: target.to_path_buf(),
                is_active: false,
                installed_at,
                verified_flags: Vec::new(),
                registration_channel: RegistrationChannel::Undetermined,
            })
        }
        Err(err) => {
            let _ = std::fs::remove_dir_all(target);
            let _ = std::fs::remove_file(partial);
            Err(err)
        }
    }
}

/// A `runtimes` row back into the contract type. `verified_flags_json` is
/// decoded with a lenient fallback (a corrupt value degrades to "unverified",
/// never an error — T-023 owns that column's semantics). The stored
/// `registration_channel` string maps onto the enum by variant name; any
/// unrecognized value degrades to `Undetermined` with a warning rather than
/// guessing.
fn row_to_build(row: &RuntimeRow) -> RuntimeBuild {
    let verified_flags: Vec<VerifiedFlag> =
        serde_json::from_str(&row.verified_flags_json).unwrap_or_default();
    RuntimeBuild {
        build_tag: row.build_tag.clone(),
        backend: row.backend.clone(),
        install_path: PathBuf::from(&row.install_path),
        is_active: row.is_active,
        installed_at: row.installed_at,
        verified_flags,
        registration_channel: channel_from_stored(&row.registration_channel),
    }
}

fn channel_from_stored(raw: &str) -> RegistrationChannel {
    match raw {
        "PresetDeclaresPath" => RegistrationChannel::PresetDeclaresPath,
        "ScanOnly" => RegistrationChannel::ScanOnly,
        // "Undetermined" and anything unrecognized: the enum's own default.
        _ => {
            if raw != "Undetermined" {
                warn!("unrecognized stored registration channel {raw:?}; treating as Undetermined");
            }
            RegistrationChannel::Undetermined
        }
    }
}

/// Reject an archive entry whose destination would escape the extraction
/// directory: `..` components, absolute paths, and drive letters. Symlink
/// entries are rejected by the caller (they are a separate, equally unsafe
/// class). Checked against the *raw stored name* — the zip crate stores
/// names verbatim, so what is validated here is exactly what would be joined
/// onto the destination.
fn validate_entry_name(name: &str) -> Result<(), AppError> {
    let reject = || AppError::UnsafeArchiveEntry {
        entry: name.to_string(),
    };

    if name.is_empty() {
        return Err(reject());
    }
    let bytes = name.as_bytes();
    // Rooted or absolute. On Windows `Path::is_absolute()` is true only for
    // drive-letter (`C:\`), rooted (`\foo`) and UNC (`\\server\share`) paths —
    // a POSIX-style `/foo` reports *relative* (it is rooted at the current
    // drive, which is exactly what makes it dangerous: joining it onto the
    // extraction directory would escape it). So the leading-slash case is
    // checked explicitly against the raw bytes.
    if bytes[0] == b'/' || bytes[0] == b'\\' {
        return Err(reject());
    }
    // Drive letter (`C:` / `c:`), checked explicitly so the rule does not
    // depend on which platform the check runs on.
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(reject());
    }
    for component in Path::new(name).components() {
        if matches!(component, Component::ParentDir) {
            return Err(reject());
        }
    }
    Ok(())
}

/// Streaming SHA-256 of a file, lowercase hex.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn bytes_per_sec(received: u64, start: Instant) -> u64 {
    let elapsed = start.elapsed().as_secs_f64();
    if elapsed <= 0.0 {
        0
    } else {
        (received as f64 / elapsed) as u64
    }
}

fn io_err(err: std::io::Error) -> AppError {
    AppError::Io {
        message: err.to_string(),
    }
}

/// A `zip`-crate failure (corrupt or malformed archive). The contract's
/// `AppError` has no "malformed archive" variant; `Io` is the closest fit —
/// it is a local-file problem, and its remediation is none ("internal"),
/// which is honest: a corrupt download is a bug in the source, not a user
/// action.
fn zip_err(err: zip::result::ZipError) -> AppError {
    AppError::Io {
        message: format!("archive error: {err}"),
    }
}

/// Deletes the temp artifacts that any still-unconsumed failure path must not
/// leave behind (rollback points R1–R4 in the module doc). Path-only fields:
/// `Send`-safe to hold across an `await`.
struct InstallCleanup {
    partial: Option<PathBuf>,
    extract: Option<PathBuf>,
    done: bool,
}

impl Drop for InstallCleanup {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        if let Some(path) = &self.partial {
            let _ = std::fs::remove_file(path);
        }
        if let Some(dir) = &self.extract {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use chrono::DateTime;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;
    use zip::write::{FileOptions, ZipWriter};

    // ── Fixture helpers ─────────────────────────────────────────────────

    /// A fixture archive with a realistic llama.cpp layout (real asset name
    /// from T-020's corpus: `llama-b9196-bin-win-cuda-13.3-x64.zip`).
    fn fixture_zip() -> Vec<u8> {
        make_zip(
            &[
                ("bin/llama-server.exe", b"EXE-BYTES"),
                ("README.md", b"# llama.cpp"),
            ],
            &[],
        )
    }

    /// Build a zip in memory. `symlinks` are added with the crate's own
    /// `add_symlink` (which stores the entry as a symlink — exactly what a
    /// malicious archive would contain).
    fn make_zip(entries: &[(&str, &[u8])], symlinks: &[(&str, &str)]) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(&mut buffer);
        for (name, data) in entries {
            writer
                .start_file(name, FileOptions::<()>::default())
                .unwrap();
            writer.write_all(data).unwrap();
        }
        for (name, target) in symlinks {
            writer
                .add_symlink(name, target, FileOptions::<()>::default())
                .unwrap();
        }
        writer.finish().unwrap();
        buffer.into_inner()
    }

    /// Serve `body` once per connection on an ephemeral loopback port. When
    /// `truncate_after` is set, the response declares the full length in
    /// `Content-Length` but sends only that many bytes before closing — a
    /// mid-transfer interruption, not a short-but-complete file.
    fn start_byte_server(body: Vec<u8>, truncate_after: Option<usize>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else {
                    continue;
                };
                let mut headers: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 512];
                while !headers.ends_with(b"\r\n\r\n") {
                    let Ok(n) = socket.read(&mut tmp) else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    headers.extend_from_slice(&tmp[..n]);
                    if headers.len() > 65536 {
                        break;
                    }
                }
                let send_len = truncate_after.unwrap_or(body.len()).min(body.len());
                // Raw bytes, not a String round-trip: the body is a zip file
                // (arbitrary binary), and `String::from_utf8_lossy` would
                // replace invalid sequences with U+FFFD and change the length.
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(header.as_bytes()).unwrap();
                let _ = socket.write_all(&body[..send_len]);
            }
        });
        port
    }

    fn client() -> reqwest::Client {
        reqwest::Client::builder().build().unwrap()
    }

    /// A release pointing at `url`, with the realistic b9196 cuda-13.3 asset
    /// name from T-020's corpus.
    fn release(url: &str, sha256: Option<String>) -> AvailableRelease {
        AvailableRelease {
            build_tag: "b9196".to_string(),
            backend: Backend::Cuda { major: 13 },
            asset_name: "llama-b9196-bin-win-cuda-13.3-x64.zip".to_string(),
            asset_url: url.to_string(),
            sha256,
            size_bytes: 0,
            published_at: "2026-08-31T19:41:30Z".parse().unwrap(),
            release_notes_url: "https://github.com/ggml-org/llama.cpp/releases/tag/b9196"
                .to_string(),
        }
    }

    fn backend() -> Backend {
        Backend::Cuda { major: 13 }
    }

    /// The full pipeline against a scratch root and an in-memory database —
    /// exactly what `ipc::install_runtime` orchestrates.
    async fn install(
        client: &reqwest::Client,
        rel: &AvailableRelease,
        conn: &Connection,
        root: &Path,
        events: &mut Vec<InstallProgress>,
    ) -> Result<RuntimeBuild, AppError> {
        let mut progress = |p: InstallProgress| events.push(p);

        match pre_install_on(conn, &rel.build_tag, &rel.backend, root)? {
            PreCheckResult::Existing(build) => {
                progress(InstallProgress::Done {
                    build: build.clone(),
                });
                return Ok(build);
            }
            PreCheckResult::Proceed => {}
        }

        let extracted = download_verify_extract(client, rel, root, &mut progress).await?;
        progress(InstallProgress::Registering);
        let build = register_runtime_on(
            conn,
            &rel.build_tag,
            &rel.backend,
            &extracted.target,
            &extracted.partial,
        )?;
        progress(InstallProgress::Done {
            build: build.clone(),
        });
        Ok(build)
    }

    // ── Happy path ──────────────────────────────────────────────────────

    #[tokio::test]
    async fn happy_path_download_verify_extract_register() {
        let body = fixture_zip();
        let digest = format!("{:x}", Sha256::digest(&body));
        let port = start_byte_server(body.clone(), None);
        let rel = release(
            &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
            Some(digest),
        );

        let root = tempfile::tempdir().unwrap();
        let conn = db::open_in_memory().unwrap();
        let mut events: Vec<InstallProgress> = Vec::new();

        let build = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .unwrap_or_else(|e| panic!("install failed: {e:?}"));

        assert_eq!(build.build_tag, "b9196");
        assert_eq!(build.backend, backend());
        assert!(!build.is_active);
        let target = root.path().join("b9196-cuda_13");
        assert_eq!(build.install_path, target);

        // Files actually landed.
        assert_eq!(
            std::fs::read(target.join("bin/llama-server.exe")).unwrap(),
            b"EXE-BYTES"
        );
        assert!(target.join("README.md").exists());

        // The row landed with the schema defaults (T-023 fills the rest).
        let row = get_runtime(&conn, "b9196", &backend()).unwrap().unwrap();
        assert_eq!(row.install_path, target.to_string_lossy());
        assert!(!row.is_active);
        assert_eq!(row.verified_flags_json, "[]");
        assert_eq!(row.registration_channel, "Undetermined");

        // No temp artifacts survive: only the target directory remains.
        let leftovers: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            leftovers,
            vec!["b9196-cuda_13"],
            "only the target dir may remain"
        );

        // The event sequence: Downloading … Verifying, Extracting(2),
        // Registering (emitted by the caller of this test's pipeline — see
        // `install`), Done.
        let kinds: Vec<&str> = events
            .iter()
            .map(|e| match e {
                InstallProgress::Resolving => "Resolving",
                InstallProgress::Downloading { .. } => "Downloading",
                InstallProgress::Verifying => "Verifying",
                InstallProgress::Extracting { .. } => "Extracting",
                InstallProgress::Registering => "Registering",
                InstallProgress::Done { .. } => "Done",
                InstallProgress::Failed { .. } => "Failed",
            })
            .collect();
        assert!(kinds.contains(&"Downloading"), "saw {kinds:?}");
        assert!(kinds.contains(&"Verifying"), "saw {kinds:?}");
        assert!(
            matches!(events.last(), Some(InstallProgress::Done { .. })),
            "saw {kinds:?}"
        );
        let extracting: Vec<&InstallProgress> = events
            .iter()
            .filter(|e| matches!(e, InstallProgress::Extracting { .. }))
            .collect();
        assert!(
            matches!(
                extracting.last(),
                Some(InstallProgress::Extracting {
                    entries_done: 2,
                    entries_total: 2
                })
            ),
            "final Extracting event must count both entries: {extracting:?}"
        );

        // The last Downloading event carries the full size.
        let last_downloading = events
            .iter()
            .rev()
            .find_map(|e| match e {
                InstallProgress::Downloading { received_bytes, .. } => Some(*received_bytes),
                _ => None,
            })
            .unwrap();
        assert_eq!(last_downloading as usize, body.len());
    }

    // ── Interrupted download (R1) ───────────────────────────────────────

    #[tokio::test]
    async fn interrupted_download_leaves_no_partial_install() {
        let body = fixture_zip();
        // Declare the full length, send only half, close: a mid-transfer cut.
        let port = start_byte_server(body.clone(), Some(body.len() / 2));
        let rel = release(
            &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
            None,
        );

        let root = tempfile::tempdir().unwrap();
        let conn = db::open_in_memory().unwrap();
        let mut events: Vec<InstallProgress> = Vec::new();

        let err = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .expect_err("a truncated stream must not install");

        match &err {
            AppError::Network { message } => {
                assert!(
                    message.contains("interrupted") || message.contains("incomplete"),
                    "the error must name the truncation, got: {message}"
                );
            }
            other => panic!("expected a Network error for a truncated download, got {other:?}"),
        }

        // No partial install: no target dir, no temp file, no row.
        assert!(!root.path().join("b9196-cuda_13").exists());
        assert!(
            !partial_file(root.path(), "b9196", &backend()).exists(),
            "the partial download must be cleaned up"
        );
        assert_eq!(get_runtime(&conn, "b9196", &backend()).unwrap(), None);
    }

    // ── Checksum mismatch (R2) ──────────────────────────────────────────

    #[tokio::test]
    async fn checksum_mismatch_aborts_before_extraction() {
        let body = fixture_zip();
        let port = start_byte_server(body.clone(), None);
        let wrong = "0".repeat(64);
        let rel = release(
            &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
            Some(wrong.clone()),
        );

        let root = tempfile::tempdir().unwrap();
        let conn = db::open_in_memory().unwrap();
        let mut events: Vec<InstallProgress> = Vec::new();

        let err = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .expect_err("a mismatched checksum must abort");

        match err {
            AppError::ChecksumMismatch { expected, actual } => {
                assert_eq!(expected, wrong);
                assert_ne!(actual, wrong, "the actual digest must be the real one");
                assert_eq!(actual.len(), 64);
            }
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }

        // Aborted *before* extraction: no extract dir, no target, no row, and
        // the temp file is cleaned up.
        assert!(!extract_dir(root.path(), "b9196", &backend()).exists());
        assert!(!root.path().join("b9196-cuda_13").exists());
        assert!(get_runtime(&conn, "b9196", &backend()).unwrap().is_none());
        // The download itself completed (the mismatch is a verification-stage
        // failure, not a network one).
        assert!(events
            .iter()
            .any(|e| matches!(e, InstallProgress::Verifying)));
    }

    // ── No-op re-install ────────────────────────────────────────────────

    #[tokio::test]
    async fn reinstalling_an_existing_build_is_a_no_op() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("b9196-cuda_13");
        std::fs::create_dir_all(target.join("bin")).unwrap();
        std::fs::write(target.join("bin/llama-server.exe"), b"EXE-BYTES").unwrap();

        let conn = db::open_in_memory().unwrap();
        let installed_at: DateTime<Utc> = "2026-08-01T00:00:00Z".parse().unwrap();
        insert_runtime(
            &conn,
            &RuntimeRow {
                build_tag: "b9196".to_string(),
                backend: backend(),
                install_path: target.to_string_lossy().into_owned(),
                is_active: false,
                installed_at,
                verified_flags_json: "[]".to_string(),
                registration_channel: "Undetermined".to_string(),
            },
        )
        .unwrap();

        // The asset URL points at a closed port: if the code tried to
        // download, this would fail — success proves no network I/O happened.
        let rel = release("http://127.0.0.1:9/never-downloaded.zip", None);
        let mut events: Vec<InstallProgress> = Vec::new();

        let build = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .unwrap_or_else(|e| panic!("no-op must not fail: {e:?}"));

        assert_eq!(build.install_path, target);
        assert_eq!(
            build.installed_at, installed_at,
            "the stored row must be returned, not a new one"
        );
        assert_eq!(events.len(), 1, "a no-op emits only Done: {events:?}");
        assert!(matches!(&events[0], InstallProgress::Done { .. }));

        // The row is untouched.
        let row = get_runtime(&conn, "b9196", &backend()).unwrap().unwrap();
        assert_eq!(row.installed_at, installed_at);
    }

    /// Recorded decision: a row whose files are gone is corrupt state — the
    /// stale row is deleted and the build is reinstalled fresh.
    #[tokio::test]
    async fn row_without_files_reinstalls_fresh() {
        let body = fixture_zip();
        let digest = format!("{:x}", Sha256::digest(&body));
        let port = start_byte_server(body, None);

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("b9196-cuda_13");
        // The directory does NOT exist — that is the corrupt state.
        assert!(!target.exists());

        let conn = db::open_in_memory().unwrap();
        let stale_at: DateTime<Utc> = "2026-07-01T00:00:00Z".parse().unwrap();
        insert_runtime(
            &conn,
            &RuntimeRow {
                build_tag: "b9196".to_string(),
                backend: backend(),
                install_path: target.to_string_lossy().into_owned(),
                is_active: false,
                installed_at: stale_at,
                verified_flags_json: "[]".to_string(),
                registration_channel: "Undetermined".to_string(),
            },
        )
        .unwrap();

        let rel = release(
            &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
            Some(digest),
        );
        let mut events: Vec<InstallProgress> = Vec::new();

        let build = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .unwrap_or_else(|e| panic!("reinstall must succeed: {e:?}"));

        assert_eq!(build.install_path, target);
        assert!(target.join("bin/llama-server.exe").exists());
        assert_ne!(
            build.installed_at, stale_at,
            "the fresh install gets a new timestamp"
        );
        let rows = crate::db::queries::list_runtimes(&conn).unwrap();
        assert_eq!(rows.len(), 1, "the stale row is replaced, not duplicated");
    }

    // ── Disk-full during extraction (R3) ────────────────────────────────

    /// Simulated without a real full disk: the fixture archive contains
    /// `bin/llama-server.exe`; pre-creating `<extract>/bin` as a *file* makes
    /// that entry's parent-directory creation fail with an I/O error — the
    /// same failure class ENOSPC produces. The rollback must be identical.
    #[tokio::test]
    async fn disk_full_during_extraction_rolls_back() {
        let body = fixture_zip();
        let digest = format!("{:x}", Sha256::digest(&body));
        let port = start_byte_server(body, None);

        let root = tempfile::tempdir().unwrap();
        // The obstacle: a file where the extraction expects to create a dir.
        let extract = extract_dir(root.path(), "b9196", &backend());
        std::fs::create_dir_all(&extract).unwrap();
        std::fs::write(extract.join("bin"), b"obstacle").unwrap();

        let conn = db::open_in_memory().unwrap();
        let rel = release(
            &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
            Some(digest),
        );
        let mut events: Vec<InstallProgress> = Vec::new();

        let err = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .expect_err("a write failure mid-extraction must abort");

        match &err {
            AppError::Io { .. } => {}
            other => panic!("expected an Io error for a failed entry write, got {other:?}"),
        }

        // Rolled back: no target dir, no extract dir (the obstacle's parent),
        // no temp file, no row.
        assert!(!root.path().join("b9196-cuda_13").exists());
        assert!(!extract.exists());
        assert!(!partial_file(root.path(), "b9196", &backend()).exists());
        assert_eq!(get_runtime(&conn, "b9196", &backend()).unwrap(), None);
    }

    // ── Zip-slip (R3) ───────────────────────────────────────────────────

    /// Each escape kind, as a real archive entry built with the zip crate's
    /// own writer (hostile names are stored verbatim — the writer performs no
    /// sanitization). All four must abort with `UnsafeArchiveEntry` and leave
    /// nothing on disk.
    #[tokio::test]
    async fn zip_slip_entries_are_rejected_and_write_nothing() {
        let cases: &[(&str, Vec<u8>)] = &[
            (
                "parent-dir escape",
                make_zip(&[("../evil.txt", b"escaped")], &[]),
            ),
            (
                "absolute path",
                make_zip(&[("/abs-evil.txt", b"escaped")], &[]),
            ),
            (
                "drive letter",
                make_zip(&[("C:\\evil-drive.txt", b"escaped")], &[]),
            ),
            (
                "symlink entry",
                make_zip(&[], &[("evil-link", "../outside-target")]),
            ),
        ];

        for (label, body) in cases {
            let port = start_byte_server(body.clone(), None);
            let root = tempfile::tempdir().unwrap();
            let conn = db::open_in_memory().unwrap();
            let rel = release(
                &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
                None,
            );

            let mut events: Vec<InstallProgress> = Vec::new();
            let err = install(&client(), &rel, &conn, root.path(), &mut events)
                .await
                .expect_err("a zip-slip entry must abort");

            match &err {
                AppError::UnsafeArchiveEntry { .. } => {}
                other => panic!("[{label}] expected UnsafeArchiveEntry, got {other:?}"),
            }

            // Nothing written: no extract dir (validation precedes writing),
            // no target dir, no row.
            assert!(
                !extract_dir(root.path(), "b9196", &backend()).exists(),
                "[{label}]"
            );
            assert!(!root.path().join("b9196-cuda_13").exists(), "[{label}]");
            assert!(get_runtime(&conn, "b9196", &backend()).unwrap().is_none());

            // And specifically nothing *outside* the extraction directory.
            assert!(!root.path().join("evil.txt").exists(), "[{label}]");
            assert!(!root.path().join("abs-evil.txt").exists(), "[{label}]");
            assert!(!root.path().join("evil-link").exists(), "[{label}]");
        }
    }

    // ── Missing checksum ────────────────────────────────────────────────

    /// The common case today: no published checksum. The install proceeds
    /// (the warning goes to the app log — `tracing::warn!` — there is no UI
    /// surface yet) and must not be treated as an error.
    #[tokio::test]
    async fn missing_checksum_warns_and_installs() {
        let body = fixture_zip();
        let port = start_byte_server(body, None);
        let root = tempfile::tempdir().unwrap();
        let conn = db::open_in_memory().unwrap();
        let rel = release(
            &format!("http://127.0.0.1:{port}/llama-b9196-bin-win-cuda-13.3-x64.zip"),
            None,
        );
        let mut events: Vec<InstallProgress> = Vec::new();

        let build = install(&client(), &rel, &conn, root.path(), &mut events)
            .await
            .unwrap_or_else(|e| panic!("install without a checksum must succeed: {e:?}"));

        assert!(build.install_path.join("bin/llama-server.exe").exists());
        assert!(get_runtime(&conn, "b9196", &backend()).unwrap().is_some());
    }

    // ── Entry-name validation (unit) ────────────────────────────────────

    #[test]
    fn validate_entry_name_table() {
        let safe = [
            "bin/llama-server.exe",
            "a/b/c.txt",
            "README.md",
            "tools\\helper.exe",
        ];
        for name in &safe {
            assert!(validate_entry_name(name).is_ok(), "{name} must be accepted");
        }

        let unsafe_names = [
            "../evil.txt",
            "../../etc/passwd",
            "a/../../evil.txt",
            "/abs-evil.txt",
            "C:\\evil-drive.txt",
            "d:/x/y.txt",
            "\\\\server\\share\\file.txt",
            "",
        ];
        for name in &unsafe_names {
            match validate_entry_name(name) {
                Err(AppError::UnsafeArchiveEntry { entry }) => assert_eq!(entry, *name),
                other => panic!("{name:?} must be rejected with UnsafeArchiveEntry, got {other:?}"),
            }
        }
    }

    // ── Row -> RuntimeBuild mapping ─────────────────────────────────────

    #[test]
    fn row_to_build_maps_defaults_and_stored_values() {
        let row = RuntimeRow {
            build_tag: "b9196".to_string(),
            backend: Backend::Vulkan,
            install_path: "C:/x/runtimes/b9196-vulkan".to_string(),
            is_active: true,
            installed_at: "2026-08-01T00:00:00Z".parse().unwrap(),
            verified_flags_json:
                r#"[{"name":"--flash-attn","takes_value":false,"allowed_values":null}]"#.to_string(),
            registration_channel: "ScanOnly".to_string(),
        };

        let build = row_to_build(&row);
        assert_eq!(build.backend, Backend::Vulkan);
        assert!(build.is_active);
        assert_eq!(build.registration_channel, RegistrationChannel::ScanOnly);
        assert_eq!(build.verified_flags.len(), 1);
        assert_eq!(build.verified_flags[0].name, "--flash-attn");

        // A corrupt flags column degrades to "unverified", never an error.
        let mut corrupt = row.clone();
        corrupt.verified_flags_json = "not-json".to_string();
        assert!(row_to_build(&corrupt).verified_flags.is_empty());

        // An unrecognized channel string degrades to Undetermined.
        let mut unknown = row.clone();
        unknown.registration_channel = "SomethingNew".to_string();
        assert_eq!(
            row_to_build(&unknown).registration_channel,
            RegistrationChannel::Undetermined
        );
    }
}
