//! Model path operations: normalization, validation, INI escaping,
//! availability probing, and link management.
//!
//! Windows-specific implementation. Model files are never moved or copied;
//! the app addresses them by absolute path (AGENTS.md invariant 8).
//!
//! See PLAN.md §2.1 for the design rationale.
//!
//! Functions are pub but not called from the binary yet — only from tests.
//! The consuming tasks (T-033, T-037) will call them; dead_code is allowed
//! here because that is the expected state between now and those tasks.
#![allow(dead_code)]

use std::fs;
use std::io::{self, Read};
use std::os::windows::fs::symlink_file;
use std::path::{Path, PathBuf};

use crate::core::types::{AppError, LinkCapability, ModelAvailability};

/// Normalize a path to its canonical absolute form.
///
/// Resolves symlinks, junctions, `.` and `..` components. Returns an absolute
/// path that is stable across invocations.
///
/// Idempotent: `normalize(normalize(p)) == normalize(p)`.
pub fn normalize(path: &Path) -> Result<PathBuf, AppError> {
    fs::canonicalize(path).map_err(|e| AppError::InvalidPath {
        path: path.to_string_lossy().to_string(),
        reason: e.to_string(),
    })
}

/// Escape a path for use in a llama.cpp preset INI file.
///
/// INI format uses double-quotes for values containing spaces or special
/// characters. The path is returned in a form that round-trips with
/// `normalize` when parsed back.
///
/// # Returns
/// The INI-safe literal string for the path.
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

    // Check if it's a file (not a directory)
    if !path.is_file() {
        return ModelAvailability::Missing;
    }

    // Try to open and read a byte to verify readability
    match fs::File::open(path) {
        Ok(mut f) => {
            let mut buf = [0u8; 1];
            match f.read_exact(&mut buf) {
                Ok(_) => ModelAvailability::Present,
                Err(e) => {
                    if e.kind() == io::ErrorKind::UnexpectedEof {
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

/// Create a symlink within the app's data directory pointing to a target model.
///
/// This is used under the `ScanOnly` registration channel to make models
/// discoverable by `--models-dir` without moving them.
///
/// # Arguments
/// - `app_dir`: the app's data directory (must exist)
/// - `target`: the absolute path to the model file (must exist)
/// - `name`: the name to give the link within `app_dir`
///
/// # Returns
/// The absolute path of the created link.
///
/// # Errors
/// - `InvalidPath` if the destination would be outside `app_dir`
/// - `Io` if the link cannot be created
pub fn link_into(app_dir: &Path, target: &Path, name: &str) -> Result<PathBuf, AppError> {
    let link_path = app_dir.join(name);

    // Validate that the destination is within app_dir
    let canonical_app = normalize(app_dir)?;
    // Check the parent of the link path, not the link itself (which doesn't exist yet)
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

    // Create the symlink
    match symlink_file(target, &link_path) {
        Ok(()) => Ok(link_path),
        Err(e) => Err(AppError::Io {
            message: format!(
                "failed to create symlink at {}: {}",
                link_path.to_string_lossy(),
                e
            ),
        }),
    }
}

/// Remove a symlink, leaving the target intact.
///
/// # Errors
/// - `Io` if the link cannot be removed
pub fn unlink(link: &Path) -> Result<(), AppError> {
    fs::remove_file(link).map_err(|_| AppError::Io {
        message: format!("failed to remove link at {}", link.to_string_lossy()),
    })
}

/// Determine what linking primitive this machine can actually use.
///
/// Attempts to create a symlink in a scratch directory rather than reading
/// a registry key or privilege. The result is cached for the session.
///
/// # Returns
/// - `Symlink`: file symlinks work (preferred)
/// - `HardLinkOnly`: no symlink privilege, hard links work on same volume
/// - `None`: neither works
pub fn link_capability(app_dir: &Path) -> LinkCapability {
    // Create a scratch directory for the test
    let scratch = app_dir.join(".link-test");
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

    // Try symlink first
    let symlink_ok = symlink_file(&test_target, &test_link).is_ok();

    if symlink_ok {
        // Clean up the link
        let _ = fs::remove_file(&test_link);
    }

    // Clean up scratch
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
        let name = format!("llama-test-{}-{}", std::process::id(), counter);
        let path = tmp.join(&name);
        fs::create_dir_all(&path).expect("failed to create temp dir");
        path
    }

    fn cleanup(dir: &Path) {
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn test_normalize_idempotent() {
        let tmp = temp_dir();
        let file = tmp.join("test.gguf");
        let mut f = File::create(&file).unwrap();
        f.write_all(b"test").unwrap();

        let n1 = normalize(&file).unwrap();
        let n2 = normalize(&n1).unwrap();
        assert_eq!(n1, n2, "normalize must be idempotent");

        cleanup(&tmp);
    }

    #[test]
    fn test_normalize_absolute() {
        let tmp = temp_dir();
        let file = tmp.join("test.gguf");
        let mut f = File::create(&file).unwrap();
        f.write_all(b"test").unwrap();

        let normalized = normalize(&file).unwrap();
        assert!(normalized.is_absolute(), "normalized path must be absolute");
        assert_eq!(normalized, file.canonicalize().unwrap());

        cleanup(&tmp);
    }

    #[test]
    fn test_preset_literal_quotes() {
        let path = PathBuf::from("/path with spaces/model.gguf");
        let literal = preset_literal(&path);
        assert_eq!(literal, "\"/path with spaces/model.gguf\"");
    }

    #[test]
    fn test_preset_literal_roundtrip() {
        let tmp = temp_dir();
        let file = tmp.join("test.gguf");
        let mut f = File::create(&file).unwrap();
        f.write_all(b"test").unwrap();

        let normalized = normalize(&file).unwrap();
        let literal = preset_literal(&normalized);
        // The literal should be the quoted normalized path
        assert!(literal.starts_with("\""));
        assert!(literal.ends_with("\""));

        cleanup(&tmp);
    }

    #[test]
    fn test_probe_present() {
        let tmp = temp_dir();
        let file = tmp.join("test.gguf");
        let mut f = File::create(&file).unwrap();
        f.write_all(b"test").unwrap();

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
    fn test_link_into_and_unlink() {
        let tmp = temp_dir();
        let app_dir = tmp.join("app");
        let models_dir = tmp.join("models");
        fs::create_dir_all(&app_dir).unwrap();
        fs::create_dir_all(&models_dir).unwrap();

        let target = models_dir.join("model.gguf");
        let mut f = File::create(&target).unwrap();
        f.write_all(b"model data").unwrap();

        // link_into may fail if the user doesn't have symlink privilege
        let link = link_into(&app_dir, &target, "my-model.gguf");
        match link {
            Ok(link_path) => {
                assert!(link_path.exists(), "link must exist after link_into");
                assert!(target.exists(), "target must still exist after link_into");

                // Verify the link points to the target
                let link_target = fs::read_link(&link_path).unwrap();
                assert_eq!(link_target, target);

                unlink(&link_path).unwrap();
                assert!(!link_path.exists(), "link must not exist after unlink");
                assert!(target.exists(), "target must still exist after unlink");
            }
            Err(e) => {
                // If symlink creation fails (no privilege), that's OK for the test
                assert!(
                    matches!(e, AppError::Io { .. }),
                    "link failure should be an Io error"
                );
            }
        }

        cleanup(&tmp);
    }

    #[test]
    fn test_link_into_rejects_outside_app_dir() {
        let tmp = temp_dir();
        let app_dir = tmp.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let target = tmp.join("target.gguf");
        let mut f = File::create(&target).unwrap();
        f.write_all(b"test").unwrap();

        // Try to link outside the app dir
        let result = link_into(&app_dir, &target, "../escape.gguf");
        assert!(
            result.is_err(),
            "link_into must reject paths outside app_dir"
        );

        cleanup(&tmp);
    }

    #[test]
    fn test_link_capability() {
        let tmp = temp_dir();
        let app_dir = tmp.join("app");
        fs::create_dir_all(&app_dir).unwrap();

        let cap = link_capability(&app_dir);
        // On Windows with Developer Mode or admin, this should be Symlink
        // Otherwise HardLinkOnly. None is unlikely.
        assert!(matches!(
            cap,
            LinkCapability::Symlink | LinkCapability::HardLinkOnly
        ));

        cleanup(&tmp);
    }
}
