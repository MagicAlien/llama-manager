//! `core::speculative` — T-036: speculative-decoding roles.
//!
//! Two different things are called "speculative decoding" in this project and
//! conflating them is the failure this module exists to prevent:
//!
//! - **MTP heads** — a *complete* model that carries Multi-Token-Prediction
//!   heads (`{arch}.nextn_predict_layers` in the GGUF header). It launches
//!   alone and drafts with its own heads: `--spec-type draft-mtp`. It stays in
//!   the catalogue and is launchable like any other model
//!   (`core::gguf::has_mtp_heads`).
//! - **Draft companions** — small models whose *architecture* is a draft
//!   architecture (`dflash`, `dspark`, `eagle`, …). They are never
//!   standalone-launchable; they exist only next to a main model as
//!   `--model-draft` with a matching `--spec-type`
//!   (`core::gguf::is_draft_architecture`). The registry rejects them at
//!   import (`core::model_registry::import_paths`).
//!
//! Everything here is either a pure decision function or a stat-level check on
//! a path — no database access, no launch.
//!
//! # Where the `--spec-type` values come from
//!
//! The values below are not invented: they are the enum the pinned build's own
//! `--help` prints for `--spec-type`, captured from a real
//! `llama-server.exe --help` run (build b10883, this machine — the same
//! capture `docs/verified-flags.md` is generated from). Re-check them when the
//! pinned build changes, per `AGENTS.md` §1: a build whose verified flag list
//! declares `allowed_values` for `--spec-type` is authoritative, and
//! `allowed_spec_type` below prefers it over this table.

use std::path::Path;

use crate::core::gguf;
use crate::core::types::{AppError, LaunchParams, ModelEntry, SpeculativeParams};

/// `--spec-type` value for a model that drafts with its own MTP heads.
pub const SPEC_TYPE_MTP: &str = "draft-mtp";

/// Fallback for a draft companion whose architecture has no dedicated
/// `--spec-type` value in this build.
pub const SPEC_TYPE_GENERIC_DRAFT: &str = "draft-simple";

/// The `--spec-type` values build b10883 lists (see the module header).
pub const SPEC_TYPE_VALUES: &[&str] = &[
    "none",
    "draft-simple",
    "draft-eagle3",
    "draft-mtp",
    "draft-dflash",
    "draft-dspark",
    "ngram-simple",
    "ngram-map-k",
    "ngram-map-k4v",
    "ngram-mod",
    "ngram-cache",
];

/// The `--spec-type` value that drives a draft companion of this
/// architecture. The EAGLE family maps onto the single `draft-eagle3` value
/// the build offers; any other draft architecture falls back to the generic
/// `draft-simple`, which is what it is: a draft model consumed as a draft.
pub fn spec_type_for_draft_architecture(architecture: &str) -> &'static str {
    match architecture.to_lowercase().as_str() {
        "dflash" | "dflash2" => "draft-dflash",
        "dspark" | "dspark2" => "draft-dspark",
        "eagle" | "eagle2" | "eagle3" | "eagle4" => "draft-eagle3",
        _ => SPEC_TYPE_GENERIC_DRAFT,
    }
}

/// The `--spec-type` a model launches with, or `None` when it launches
/// without speculative decoding.
///
/// Precedence is deliberate: an explicit draft companion wins over the
/// model's own MTP heads. Drafting from the companion is the user's choice and
/// a single `--spec-type` cannot express both.
///
/// The companion is re-validated on every call rather than trusted from the
/// row: a companion deleted after it was saved must surface as a typed error,
/// not as a preset that names a file that is no longer there.
pub fn spec_type_for_entry(entry: &ModelEntry) -> Result<Option<String>, AppError> {
    if let Some(path) = companion_of(&entry.launch_params) {
        let info = validate_draft_companion(path)?;
        return Ok(Some(info.spec_type));
    }
    if entry.metadata.has_mtp_heads {
        return Ok(Some(SPEC_TYPE_MTP.to_string()));
    }
    Ok(None)
}

/// The companion file a launch configuration names, if any.
pub fn companion_of(params: &LaunchParams) -> Option<&std::path::PathBuf> {
    params
        .speculative
        .as_ref()
        .and_then(|spec| spec.draft_companion.as_ref())
}

/// What the UI shows for the validation command: the facts the picker needs
/// to accept or reject a candidate companion, and the role it will play.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, ts_rs::TS)]
#[ts(export)]
pub struct DraftCompanionInfo {
    pub path: std::path::PathBuf,
    /// The GGUF's own `general.architecture` value.
    pub architecture: String,
    /// The `--spec-type` value this companion will be driven with.
    pub spec_type: String,
    pub size_bytes: u64,
    pub block_count: u32,
    pub quantization: String,
}

/// Validate a candidate draft companion: it must exist, parse as GGUF, and
/// carry a draft architecture. Every failure is a typed error naming the file
/// — never a warning, and never a silent acceptance (`docs/TASKS.md` T-036).
pub fn validate_draft_companion(path: &Path) -> Result<DraftCompanionInfo, AppError> {
    if !path.exists() {
        return Err(AppError::NotFound {
            what: format!("draft companion file: {}", path.display()),
        });
    }

    let metadata = gguf::parse_file(path).map_err(|err| AppError::GgufParse {
        message: format!(
            "draft companion {} is not a readable GGUF file: {err}",
            path.display()
        ),
    })?;

    if !metadata.is_draft_model {
        return Err(AppError::GgufParse {
            message: format!(
                "draft companion {} has architecture '{}', which is not a draft architecture \
                 (expected one of dflash, dspark, eagle and successors); a complete model is \
                 launched as a main model, not as a draft companion",
                path.display(),
                metadata.architecture
            ),
        });
    }

    let size_bytes = std::fs::metadata(path)
        .map_err(|err| AppError::Io {
            message: format!("failed to stat {}: {err}", path.display()),
        })?
        .len();

    Ok(DraftCompanionInfo {
        path: path.to_path_buf(),
        spec_type: spec_type_for_draft_architecture(&metadata.architecture).to_string(),
        architecture: metadata.architecture,
        size_bytes,
        block_count: metadata.block_count,
        quantization: metadata.quantization,
    })
}

/// Validate the companion a launch configuration carries, if any — used on
/// the write path so a bad path is rejected before it reaches the database
/// (`model_registry::update_model_params`).
pub fn validate_companion_of(params: &LaunchParams) -> Result<(), AppError> {
    if let Some(path) = companion_of(params) {
        validate_draft_companion(path)?;
    }
    Ok(())
}

/// The draft-tuning fields that are set, as `(flag, value)` pairs.
///
/// Only the fields the user actually set are returned, so an unset field
/// leaves the build's own default in place. The flag names are spelled as the
/// build spells them; the preset generator maps them onto preset keys.
///
/// They apply to both roles: a model drafting with its own MTP heads tunes the
/// same draft stage as one driving a companion.
pub fn draft_tuning_flags(companion: &SpeculativeParams) -> Vec<(&'static str, String)> {
    let mut flags = Vec::new();
    if let Some(n_max) = companion.n_max {
        flags.push(("--spec-draft-n-max", n_max.to_string()));
    }
    if let Some(n_min) = companion.n_min {
        flags.push(("--spec-draft-n-min", n_min.to_string()));
    }
    if let Some(p_min) = companion.p_min {
        flags.push(("--spec-draft-p-min", p_min.to_string()));
    }
    if let Some(threads) = companion.threads {
        flags.push(("--spec-draft-threads", threads.to_string()));
    }
    if let Some(ref cache_type_k) = companion.cache_type_k {
        flags.push(("--spec-draft-type-k", cache_type_k.clone()));
    }
    if let Some(ref cache_type_v) = companion.cache_type_v {
        flags.push(("--spec-draft-type-v", cache_type_v.clone()));
    }
    flags
}

/// Whether a `--spec-type` value is one this build accepts. When the build's
/// verified flag list carries `allowed_values` for `--spec-type` those values
/// are authoritative (`AGENTS.md` §1); otherwise the captured enum above is
/// the reference.
pub fn allowed_spec_type(value: &str, verified_allowed: Option<&[String]>) -> bool {
    match verified_allowed {
        Some(allowed) => allowed.iter().any(|a| a == value),
        None => SPEC_TYPE_VALUES.contains(&value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::gguf::GgufValue;
    use crate::core::types::{Compatibility, GgufMetadata, ModelAvailability, SamplingDefaults};
    use std::path::PathBuf;

    /// A minimal, header-only GGUF (magic, version, tensor count, KV pairs) —
    /// the same shape `core::gguf`'s own tests build.
    fn gguf_bytes(architecture: &str, extra: &[(&str, GgufValue)]) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x4655_4747u32.to_le_bytes()); // "GGUF"
        buf.extend_from_slice(&3u32.to_le_bytes()); // version
        buf.extend_from_slice(&0u64.to_le_bytes()); // tensor count

        let mut kv: Vec<(&str, GgufValue)> = vec![(
            "general.architecture",
            GgufValue::String(architecture.to_string()),
        )];
        kv.extend(extra.iter().map(|(k, v)| (*k, v.clone())));
        buf.extend_from_slice(&(kv.len() as u64).to_le_bytes());

        for (key, value) in kv {
            buf.extend_from_slice(&(key.len() as u64).to_le_bytes());
            buf.extend_from_slice(key.as_bytes());
            match value {
                GgufValue::String(s) => {
                    buf.extend_from_slice(&8u32.to_le_bytes());
                    buf.extend_from_slice(&(s.len() as u64).to_le_bytes());
                    buf.extend_from_slice(s.as_bytes());
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

    fn write_fixture(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("write fixture");
        path
    }

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lm-mgr-t036-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        dir
    }

    fn entry(
        architecture: &str,
        has_mtp_heads: bool,
        companion: Option<SpeculativeParams>,
    ) -> ModelEntry {
        ModelEntry {
            id: "m1".to_string(),
            display_name: "main.gguf".to_string(),
            served_name: "main".to_string(),
            file_path: PathBuf::from("E:/models/main.gguf"),
            shard_paths: vec![],
            size_bytes: 1024,
            sha256_head: "abc".to_string(),
            metadata: GgufMetadata {
                architecture: architecture.to_string(),
                param_count: Some(27_000_000_000),
                quantization: "NVFP4".to_string(),
                block_count: 64,
                context_length: Some(32768),
                embedding_length: Some(5120),
                attention_head_count: Some(40),
                attention_head_count_kv: Some(8),
                attention_key_length: None,
                attention_value_length: None,
                full_attention_interval: None,
                ssm_state_size: None,
                ssm_inner_size: None,
                ssm_group_count: None,
                ssm_conv_kernel: None,
                has_chat_template: true,
                is_moe: false,
                expert_count: None,
                is_draft_model: false,
                has_mtp_heads,
                // The helper takes the flag; a file that declares MTP heads
                // declares at least one MTP layer, which is what the
                // estimator prices (T-038).
                mtp_layer_count: if has_mtp_heads { Some(1) } else { None },
                supports_tools: false,
                supports_thinking: false,
            },
            capability_tags: vec![],
            compatibility: Compatibility::Supported,
            availability: ModelAvailability::Present,
            duplicate_of: None,
            launch_params: LaunchParams {
                speculative: companion,
                ..Default::default()
            },
            sampling_defaults: SamplingDefaults::default(),
            preload: false,
            pinned: false,
            added_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn spec_type_mapping_covers_the_draft_families() {
        // Architecture-derived, never name-derived.
        assert_eq!(spec_type_for_draft_architecture("dflash"), "draft-dflash");
        assert_eq!(spec_type_for_draft_architecture("dflash2"), "draft-dflash");
        assert_eq!(spec_type_for_draft_architecture("dspark"), "draft-dspark");
        assert_eq!(spec_type_for_draft_architecture("eagle"), "draft-eagle3");
        assert_eq!(spec_type_for_draft_architecture("EAGLE3"), "draft-eagle3");
        // An architecture the build has no dedicated value for still drafts.
        assert_eq!(
            spec_type_for_draft_architecture("futuredraft"),
            SPEC_TYPE_GENERIC_DRAFT
        );
    }

    #[test]
    fn validation_rejects_a_missing_file_by_name() {
        let dir = tmp_dir("missing");
        let missing = dir.join("not-there.gguf");
        match validate_draft_companion(&missing) {
            Err(AppError::NotFound { what }) => {
                assert!(
                    what.contains("not-there.gguf"),
                    "error must name the file: {what}"
                );
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation_rejects_a_non_gguf_file_by_name() {
        let dir = tmp_dir("nongguf");
        let path = write_fixture(&dir, "draft.gguf", b"this is not a gguf header at all");
        match validate_draft_companion(&path) {
            Err(AppError::GgufParse { message }) => {
                assert!(
                    message.contains("draft.gguf"),
                    "error must name the file: {message}"
                );
                assert!(message.contains("not a readable GGUF"));
            }
            other => panic!("expected GgufParse, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation_rejects_a_complete_model_as_companion_by_name() {
        // The acceptance's "each failure is a typed error naming the file":
        // a normal architecture is not a draft companion.
        let dir = tmp_dir("complete");
        let path = write_fixture(&dir, "main.gguf", &gguf_bytes("qwen35", &[]));
        match validate_draft_companion(&path) {
            Err(AppError::GgufParse { message }) => {
                assert!(message.contains("main.gguf"));
                assert!(message.contains("qwen35"));
                assert!(message.contains("not a draft architecture"));
            }
            other => panic!("expected GgufParse, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn validation_accepts_a_draft_architecture_and_names_the_spec_type() {
        let dir = tmp_dir("accept");
        let path = write_fixture(&dir, "draft-dflash2.gguf", &gguf_bytes("dflash2", &[]));
        let info = validate_draft_companion(&path).expect("a dflash2 file is a draft companion");
        assert_eq!(info.architecture, "dflash2");
        assert_eq!(info.spec_type, "draft-dflash");
        assert!(info.size_bytes > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn spec_type_is_none_for_a_model_with_neither_signal() {
        let plain = entry("llama", false, None);
        assert_eq!(spec_type_for_entry(&plain).expect("no I/O"), None);
    }

    #[test]
    fn spec_type_is_draft_mtp_for_an_mtp_model() {
        let mtp = entry("qwen35", true, None);
        assert_eq!(
            spec_type_for_entry(&mtp).expect("no I/O"),
            Some(SPEC_TYPE_MTP.to_string())
        );
    }

    #[test]
    fn a_draft_companion_wins_over_the_model_s_own_mtp_heads() {
        let dir = tmp_dir("precedence");
        let companion_path = write_fixture(&dir, "draft.gguf", &gguf_bytes("dflash", &[]));
        let mut mtp = entry(
            "qwen35",
            true,
            Some(SpeculativeParams {
                draft_companion: Some(companion_path.clone()),
                ..Default::default()
            }),
        );
        assert_eq!(
            spec_type_for_entry(&mtp).expect("companion parses"),
            Some("draft-dflash".to_string())
        );

        // A companion deleted after it was saved is a typed error naming the
        // file — never a preset that points at nothing.
        std::fs::remove_file(&companion_path).expect("remove companion");
        match spec_type_for_entry(&mtp) {
            Err(AppError::NotFound { what }) => assert!(what.contains("draft.gguf")),
            other => panic!("expected NotFound after deleting the companion, got {other:?}"),
        }

        // …and the write path rejects it too.
        assert!(validate_companion_of(&mtp.launch_params).is_err());
        mtp.launch_params.speculative = None;
        assert!(validate_companion_of(&mtp.launch_params).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_model_with_its_own_mtp_heads_drafts_without_a_companion() {
        // T-036 acceptance refinement (owner, 14 Sept 2026): an MTP model
        // drafts with its own heads, so it has a draft stage to tune and needs
        // no companion file at all.
        let mtp = entry("qwen35", true, None);
        assert_eq!(
            spec_type_for_entry(&mtp).expect("no I/O"),
            Some("draft-mtp".to_string())
        );

        // A model with neither signal has no draft stage: no spec-type, and
        // therefore nothing for the tuning fields to tune.
        let plain = entry("llama", false, None);
        assert_eq!(spec_type_for_entry(&plain).expect("no I/O"), None);
    }

    #[test]
    fn tuning_flags_only_include_what_was_set() {
        let companion = SpeculativeParams {
            draft_companion: Some(PathBuf::from("E:/models/draft.gguf")),
            n_max: Some(4),
            n_min: None,
            p_min: Some(0.5),
            threads: None,
            cache_type_k: Some("f16".to_string()),
            cache_type_v: None,
        };
        let flags = draft_tuning_flags(&companion);
        let names: Vec<&str> = flags.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            vec![
                "--spec-draft-n-max",
                "--spec-draft-p-min",
                "--spec-draft-type-k"
            ]
        );
        assert_eq!(flags[0].1, "4");
        assert_eq!(flags[1].1, "0.5");
    }

    #[test]
    fn allowed_spec_type_prefers_the_build_s_declared_values() {
        // No declared values: the captured enum is the reference.
        assert!(allowed_spec_type("draft-mtp", None));
        assert!(!allowed_spec_type("draft-nonsense", None));
        // A build that declares them is authoritative.
        let declared = vec!["none".to_string(), "draft-simple".to_string()];
        assert!(allowed_spec_type("draft-simple", Some(&declared)));
        assert!(!allowed_spec_type("draft-mtp", Some(&declared)));
    }
}
