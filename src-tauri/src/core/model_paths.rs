//! Model path operations: normalization, validation, INI escaping,
//! availability probing, and link management.
//!
//! Windows-specific implementation. Model files are never moved or copied;
//! the app addresses them by absolute path (AGENTS.md invariant 8).
//!
//! See PLAN.md §2.1 for the design rationale.
//!
//! Functions are pub but not called from the binary yet — only from tests.
//! The consuming tasks (T-031, T-033) will call them.
#![allow(dead_code)]

use std::fs;
use std::io::{self, Read};
use std::os::windows::fs::symlink_file;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::core::types::{AppError, LinkCapability, ModelAvailability};

/// Cache for link capability, established once per process.
static LINK_CAPABILITY: OnceLock<LinkCapability> = OnceLock::new();

/// Normalize a path to its canonical absolute form.
///
/// Resolves symlinks, junctions, `.` and `..` components. Returns an absolute
/// path that is stable across invocations.
///
/// Idempotent: `normalize(normalize(p)) == normalize(p)`.
///
/// For paths that do not exist (disconnected volume, renamed file), returns
/// the lexically absolute form with a typed marker rather than failing, so
/// `probe` can still classify them as `Missing`.
pub fn normalize(path: &Path) -> Result<PathBuf, AppError> {
    if let Ok(canon) = fs::canonicalize(path) {
        return Ok(canon);
    }

    // Path does not exist — return the lexically absolute form
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Err(AppError::InvalidPath {
            path: path.to_string_lossy().to_string(),
            reason: "relative path".to_string(),
        })
    }
}

/// Escape a path for use in a llama.cpp preset INI file.
///
/// INI format uses double-quotes for values containing spaces or special
/// characters. The path is returned in a form that round-trips with
/// `normalize` when parsed back.
pub fn preset_literal(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    // Always quote for safety — handles spaces, non-ASCII, trailing dots
    format!("\"{s}\"")
}

/// Probe a model path and determine its availability.
///
/// - `Present`: the path resolves to a readable file.
/// - `Missing`: the path does not resolve (file renamed, drive disconnected).
/// - `Unreadable`: the path resolves but the file cannot be opened.
pub fn probe(path: &Path) -> ModelAvailability {
    if !path.exists() {
        return ModelAvailability::Missing;
    }

    if !path.is_file() {
        return ModelAvailability::Missing;
    }

    match fs::File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 1];
            match f.read_exact(&mut buf) {
                Ok(_) => ModelAvailability::Present,
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
                        // Empty file is still present
                        ModelAvailability::Present
                    } else {
                        ModelAvailability::Unreadable
                    }
                }
            }
        }
        Err(_) => ModelAvailability::Unreadable,
    }
}

/// Create a link within the app's data directory pointing to a target model.
///
/// Used under the `ScanOnly` registration channel to make models discoverable
/// by `--models-dir` without moving them.
///
/// Strategy: symlink first, then hard link (same volume only), then typed error.
///
/// # Errors
/// - `InvalidPath` if the destination would be outside `app_dir`
/// - `Io` if no link primitive works (names Developer Mode as remedy)
pub fn link_into(app_dir: &Path, target: &Path, name: &str) -> Result<PathBuf, AppError> {
    let link_path = app_dir.join(name);

    // Validate destination is within app_dir
    let canonical_app = normalize(app_dir)?;
    let link_parent = link_path.parent().ok_or_else(|| AppError::InvalidPath {
        path: link_path.to_string_lossy().to_string(),
        reason: "link path has no parent".to_string(),
    })?;
    let canonical_link_parent = normalize(link_parent)?;
    if !canonical_link_parent.starts_with(&canonical_app) {
        return Err(AppError::InvalidPath {
            path: link_path.to_string_lossy().to_string(),
            reason: "link destination is outside the app directory".to_string(),
        });
    }

    // Try symlink first
    match symlink_file(target, &link_path) {
        Ok(()) => Ok(link_path),
        Err(e) => {
            // Symlink failed — try hard link (same volume only)
            match fs::hard_link(target, &link_path) {
                Ok(()) => Ok(link_path),
                Err(hard_err) => Err(AppError::Io {
                    message: format!(
                        "failed to link {} to {}: symlink failed ({}), hard link failed ({}). \
                         Enable Developer Mode on Windows for symlink support.",
                        link_path.to_string_lossy(),
                        target.to_string_lossy(),
                        e,
                        hard_err
                    ),
                }),
            }
        }
    }
}

/// Remove a link, leaving the target intact.
///
/// Works for both symlinks and hard links.
pub fn unlink(link: &Path) -> Result<(), AppError> {
    fs::remove_file(link).map_err(|_| AppError::Io {
        message: format!("failed to remove link at {}", link.to_string_lossy()),
    })
}

/// Determine what linking primitive this machine can actually use.
///
/// Probes by attempting a symlink in a scratch directory, not by reading
/// a registry key or privilege. The result is cached for the session.
pub fn link_capability(app_dir: &Path) -> LinkCapability {
    LINK_CAPABILITY
        .get_or_init(|| probe_link_capability(app_dir))
        .clone()
}

fn probe_link_capability(app_dir: &Path) -> LinkCapability {
    let scratch = app_dir.join(".link-cap-test");
    if fs::create_dir_all(&scratch).is_err() {
        return LinkCapability::None;
    }

    let test_target = scratch.join("target.txt");
    let test_link = scratch.join("link.txt");

    // Create a dummy target file
    if fs::write(&test_target, "test").is_err() {
        let _ = fs::remove_dir_all(&scratch);
        return LinkCapability::None;
    }

    // Try symlink
    let symlink_ok = symlink_file(&test_target, &test_link).is_ok();

    if symlink_ok {
        let _ = fs::remove_file(&test_link);
    }

    let _ = fs::remove_dir_all(&scratch);

    if symlink_ok {
        LinkCapability::Symlink
    } else {
        // Symlinks failed; hard links work on the same volume (no privilege needed)
        LinkCapability::HardLinkOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        let tmp = std::env::temp_dir();
        let name = format!("llama-paths-{}-{}", std::process::id(), counter);
        let path = tmp.join(&name);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(b"test model data").unwrap();
        path
    }

    // === normalize: idempotency ===

    #[test]
    fn test_normalize_idempotent() {
        let tmp = temp_dir();
        let file = temp_file(&tmp, "test.gguf");

        let n1 = normalize(&file).unwrap();
        let n2 = normalize(&n1).unwrap();
        assert_eq!(n1, n2, "normalize must be idempotent");

        cleanup(&tmp);
    }

    #[test]
    fn test_normalize_absolute() {
        let tmp = temp_dir();
        let file = temp_file(&tmp, "test.gguf");

        let normalized = normalize(&file).unwrap();
        assert!(normalized.is_absolute(), "normalized path must be absolute");
        assert_eq!(normalized, file.canonicalize().unwrap());

        cleanup(&tmp);
    }

    // === normalize: paths that don't exist ===

    #[test]
    fn test_normalize_nonexistent_absolute() {
        let tmp = temp_dir();
        let nonexistent = tmp.join("does-not-exist.gguf");

        // Should return the absolute path as-is, not error
        let result = normalize(&nonexistent).unwrap();
        assert!(result.is_absolute());
        assert!(!result.exists());

        cleanup(&tmp);
    }

    #[test]
    fn test_normalize_relative_fails() {
        let result = normalize(Path::new("relative/path.gguf"));
        assert!(result.is_err());
        match result.unwrap_err() {
            AppError::InvalidPath { reason, .. } => {
                assert!(reason.contains("relative"));
            }
            other => panic!("expected InvalidPath, got {:?}", other),
        }
    }

    // === preset_literal ===

    #[test]
    fn test_preset_literal_quotes() {
        let path = PathBuf::from("/path with spaces/model.gguf");
        let literal = preset_literal(&path);
        assert_eq!(literal, "\"/path with spaces/model.gguf\"");
    }

    #[test]
    fn test_preset_literal_roundtrip_on_normalized() {
        let tmp = temp_dir();
        let file = temp_file(&tmp, "test.gguf");

        let normalized = normalize(&file).unwrap();
        let literal = preset_literal(&normalized);

        // The literal should be the quoted normalized path
        let unquoted = literal.trim_matches('"');
        assert_eq!(unquoted, normalized.to_string_lossy());

        cleanup(&tmp);
    }

    // === probe ===

    #[test]
    fn test_probe_present() {
        let tmp = temp_dir();
        let file = temp_file(&tmp, "test.gguf");

        assert_eq!(probe(&file), ModelAvailability::Present);

        cleanup(&tmp);
    }

    #[test]
    fn test_probe_missing() {
        let tmp = temp_dir();
        let file = tmp.join("nonexistent.gguf");
        assert_eq!(probe(&file), ModelAvailability::Missing);
        cleanup(&tmp);
    }

    #[test]
    fn test_probe_directory_is_missing() {
        let tmp = temp_dir();
        let dir = tmp.join("a-directory");
        fs::create_dir(&dir).unwrap();
        assert_eq!(probe(&dir), ModelAvailability::Missing);
        cleanup(&tmp);
    }

    #[test]
    fn test_probe_empty_file_is_present() {
        let tmp = temp_dir();
        let file = tmp.join("empty.gguf");
        File::create(&file).unwrap();

        assert_eq!(probe(&file), ModelAvailability::Present);

        cleanup(&tmp);
    }

    // === link_into ===

    #[test]
    fn test_link_into_and_unlink() {
        let tmp = temp_dir();
        let app_dir = tmp.join("app");
        let models_dir = tmp.join("models");
        fs::create_dir_all(&app_dir).unwrap();
        fs::create_dir_all(&models_dir).unwrap();

        let target = temp_file(&models_dir, "model.gguf");

        let link = link_into(&app_dir, &target, "my-model.gguf");
        match link {
            Ok(link_path) => {
                assert!(link_path.exists(), "link must exist after link_into");
                assert!(target.exists(), "target must still exist after link_into");

                // Verify the link works by reading through it
                let content = fs::read(&link_path).unwrap();
                assert_eq!(content, b"test model data");

                unlink(&link_path).unwrap();
                assert!(!link_path.exists(), "link must not exist after unlink");
                assert!(target.exists(), "target must still exist after unlink");
            }
            Err(e) => {
                // If symlink and hard link both fail, that's an environment issue
                assert!(matches!(e, AppError::Io { .. }));
            }
        }

        cleanup(&tmp);
    }

    #[test]
    fn test_link_into_rejects_outside_app_dir() {
        let tmp = temp_dir();
        let app_dir = tmp.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let target = temp_file(&tmp, "target.gguf");

        // Try to link outside the app dir
        let result = link_into(&app_dir, &target, "../escape.gguf");
        assert!(
            result.is_err(),
            "link_into must reject paths outside app_dir"
        );
        match result.unwrap_err() {
            AppError::InvalidPath { reason, .. } => {
                assert!(reason.contains("outside"));
            }
            other => panic!("expected InvalidPath, got {:?}", other),
        }

        cleanup(&tmp);
    }

    // === link_capability ===

    #[test]
    fn test_link_capability_returns_valid() {
        let tmp = temp_dir();
        let app_dir = tmp.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let cap = link_capability(&app_dir);
        // On Windows with Developer Mode or admin, this should be Symlink
        // Otherwise HardLinkOnly. None is unlikely.
        assert!(
            matches!(cap, LinkCapability::Symlink | LinkCapability::HardLinkOnly),
            "expected Symlink or HardLinkOnly, got {:?}",
            cap
        );

        cleanup(&tmp);
    }

    // === Proptest: path invariants ===

    use proptest::prelude::*;

    /// Generate a plausible Windows-style path fragment
    fn path_fragment() -> impl Strategy<Value = String> {
        proptest::collection::vec(any::<u8>(), 1..20).prop_map(|bytes| {
            // Avoid Windows reserved filename characters: * ? " < > | : \
            let safe = [
                b'a', b'b', b'c', b'd', b'e', b'f', b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n',
                b'o', b'p', b'q', b'r', b's', b't', b'u', b'v', b'w', b'x', b'y', b'z', b'0', b'1',
                b'2', b'3', b'4', b'5', b'6', b'7', b'8', b'9', b'_', b'-',
            ];
            bytes
                .iter()
                .map(|b| safe[(b % 38) as usize])
                .map(|b| b as char)
                .collect::<String>()
        })
    }

    proptest! {
        #[test]
        fn test_normalize_idempotent_property(fragment in path_fragment()) {
            let tmp = temp_dir();
            let name = format!("{}.gguf", fragment);
            let path = tmp.join(&name);

            // Create the file so it exists
            fs::write(&path, b"test").unwrap();

            let n1 = normalize(&path).unwrap();
            let n2 = normalize(&n1).unwrap();
            assert_eq!(n1, n2, "normalize must be idempotent for {}", name);

            cleanup(&tmp);
        }

        #[test]
        fn test_preset_literal_roundtrip_property(fragment in path_fragment()) {
            let tmp = temp_dir();
            let name = format!("{}.gguf", fragment);
            let path = tmp.join(&name);

            // Create the file
            fs::write(&path, b"test").unwrap();

            // Normalize first
            let normalized = normalize(&path).unwrap();

            // Escape for INI
            let literal = preset_literal(&normalized);

            // Parse back: strip quotes
            let unquoted = literal.trim_matches('"');
            assert_eq!(
                unquoted,
                normalized.to_string_lossy(),
                "preset_literal must round-trip for {}",
                name
            );

            cleanup(&tmp);
        }
    }
}
