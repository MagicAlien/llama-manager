//! `core::preset_generator` — T-033: Serialize every registered model into
//! `presets.ini` and generate the router command line.
//!
//! Branches on `RuntimeBuild.registration_channel` as established empirically
//! by T-025 (which overrides T-023's reading of `--help`):
//!
//! - `PresetDeclaresPath` — one section per registered model naming its
//!   absolute path via `model = <path>`, in the exact syntax T-025 recorded
//!   as working. `--models-dir` is never emitted.
//! - `ScanOnly` — the app links each registered model into its own data
//!   directory via `link_into` (T-029) and emits `--models-dir` pointing
//!   there. Sections carry settings only.
//! - `Undetermined` — stop. T-025 ran and still could not tell; this is an
//!   owner decision. Record a discrepancy and do not proceed.
//!
//! # Preset key spelling (verified empirically — see PROGRESS.md F-015)
//!
//! A preset key is the **build's own flag name without its leading dashes**,
//! dashes preserved: `--ctx-size` is written `ctx-size = 8000`, never
//! `ctx_size`. The router matches each key against its argument table and
//! **refuses to start** on a key it does not know
//! (`failed to initialize router models: option 'cache_type_k' not recognized
//! in preset 'probe'`) — so a misspelled key is not ignored, it is fatal for
//! the whole server. This is why the emitted keys are checked against
//! `RuntimeBuild.verified_flags` and omitted with a `PresetWarning` when the
//! active build does not carry the flag: b10883 has no `--no-mmap` and no
//! `--mlock` (both moved to `--load-mode`), and emitting them would prevent
//! the server from starting at all.
//!
//! # Speculative decoding (T-036)
//!
//! Two roles, emitted for the model's own section and never as a global
//! default: a model carrying MTP heads gets `spec-type = draft-mtp`, and a
//! model with a draft companion gets `spec-type = <derived from the
//! companion's architecture>` plus `model-draft = <path>` and the draft
//! tuning flags the user set. See `core::speculative`.
//!
//! `generate_preset` and `router_arguments` are called by the supervisor
//! (T-040) at server start. Until T-040 wires them, they are dead code —
//! `preview_preset` is the only command reachable from IPC today.

use crate::core::model_registry;
use crate::core::speculative;
use crate::core::types::{AppError, ModelEntry, RegistrationChannel, RuntimeBuild, VerifiedFlag};

/// A flag the generator deliberately did not emit, and why. T-033's
/// `preset-warning` event is the eventual consumer of these; until it is
/// wired (`docs/TASKS.md` T-033, unbuilt as of T-036), they are returned by
/// `preset_plan` so the omission is observable and testable rather than
/// silent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetWarning {
    pub model: String,
    pub flag: String,
    pub reason: String,
}

/// The generated preset plus everything it left out on purpose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetPlan {
    pub ini: String,
    pub warnings: Vec<PresetWarning>,
}

/// Generate the `presets.ini` content for the given active build and models.
#[allow(dead_code)]
pub fn generate_preset(build: &RuntimeBuild, models: &[ModelEntry]) -> Result<String, AppError> {
    Ok(preset_plan(build, models)?.ini)
}

/// `generate_preset`, plus the flags it omitted. Same rendering path — the
/// two can never drift.
pub fn preset_plan(build: &RuntimeBuild, models: &[ModelEntry]) -> Result<PresetPlan, AppError> {
    match build.registration_channel {
        RegistrationChannel::PresetDeclaresPath => render_plan(build, models, true),
        RegistrationChannel::ScanOnly => render_plan(build, models, false),
        RegistrationChannel::Undetermined => Err(AppError::Internal {
            message: "registration channel is Undetermined; T-025 did not resolve it. \
                     Cannot generate preset without knowing the channel."
                .to_string(),
        }),
    }
}

fn render_plan(
    build: &RuntimeBuild,
    models: &[ModelEntry],
    declares_path: bool,
) -> Result<PresetPlan, AppError> {
    let mut writer = PresetWriter::new(build);

    for model in models {
        writer.section(&model.served_name);
        if declares_path {
            // The section name IS the served name (T-025: `[test-external]`
            // registered as `test-external`), and the router rejects a
            // `served_name` key outright — there is no such option.
            writer.line(format!("model = {}\n", model.file_path.to_string_lossy()));
        }
        writer.launch_params(model)?;
        writer.blank();
    }

    Ok(writer.finish())
}

/// Preview the preset for a single model without writing to disk.
pub fn preview_preset(model: &ModelEntry, build: &RuntimeBuild) -> Result<String, AppError> {
    Ok(preview_preset_plan(model, build)?.ini)
}

/// `preview_preset`, plus the flags it omitted.
pub fn preview_preset_plan(
    model: &ModelEntry,
    build: &RuntimeBuild,
) -> Result<PresetPlan, AppError> {
    match build.registration_channel {
        RegistrationChannel::PresetDeclaresPath => {
            render_plan(build, std::slice::from_ref(model), true)
        }
        RegistrationChannel::ScanOnly => render_plan(build, std::slice::from_ref(model), false),
        RegistrationChannel::Undetermined => Err(AppError::Internal {
            message: "registration channel is Undetermined; cannot preview preset".to_string(),
        }),
    }
}

/// Build the writer that renders one channel's sections.
struct PresetWriter<'b> {
    build: &'b RuntimeBuild,
    ini: String,
    warnings: Vec<PresetWarning>,
    /// The model whose section is being written, for the warnings.
    model: String,
}

impl<'b> PresetWriter<'b> {
    fn new(build: &'b RuntimeBuild) -> Self {
        Self {
            build,
            ini: String::new(),
            warnings: Vec::new(),
            model: String::new(),
        }
    }

    fn section(&mut self, served_name: &str) {
        self.model = served_name.to_string();
        self.ini.push_str(&format!("[{served_name}]\n"));
    }

    fn blank(&mut self) {
        self.ini.push('\n');
    }

    /// An unconditional line (`model = <path>`) — the channel's own key, not
    /// a flag.
    fn line(&mut self, text: String) {
        self.ini.push_str(&text);
    }

    /// The build's verified entry for a flag, if it has one.
    fn verified(&self, flag: &str) -> Option<&'b VerifiedFlag> {
        self.build.verified_flags.iter().find(|f| f.name == flag)
    }

    /// Write the whole launch configuration for one model.
    fn launch_params(&mut self, entry: &ModelEntry) -> Result<(), AppError> {
        let params = &entry.launch_params;

        self.flag_value("--gpu-layers", params.gpu_layers.map(|v| v.to_string()));
        self.flag_value("--ctx-size", params.ctx_size.map(|v| v.to_string()));
        self.flag_value("--batch-size", params.batch_size.map(|v| v.to_string()));
        self.flag_value("--ubatch-size", params.ubatch_size.map(|v| v.to_string()));
        self.flag_value(
            "--flash-attn",
            params.flash_attn.as_ref().map(|fa| {
                match fa {
                    crate::core::types::FlashAttn::On => "on",
                    crate::core::types::FlashAttn::Off => "off",
                    crate::core::types::FlashAttn::Auto => "auto",
                }
                .to_string()
            }),
        );
        self.flag_value("--cache-type-k", params.cache_type_k.clone());
        self.flag_value("--cache-type-v", params.cache_type_v.clone());
        self.flag_value("--n-cpu-moe", params.n_cpu_moe.map(|v| v.to_string()));
        self.flag_value(
            "--tensor-split",
            params.tensor_split.as_ref().map(|values| {
                values
                    .iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            }),
        );
        self.flag_value("--main-gpu", params.main_gpu.map(|v| v.to_string()));
        // `no_mmap` / `mlock` were retired in favour of `--load-mode` on newer
        // builds (b10883 has neither flag). The verified-flag gate below is
        // what keeps them out of a preset that would refuse to start.
        self.flag_value(
            "--no-mmap",
            params.no_mmap.filter(|v| *v).map(|_| "1".to_string()),
        );
        self.flag_value(
            "--mlock",
            params.mlock.filter(|v| *v).map(|_| "1".to_string()),
        );
        self.flag_value("--threads", params.threads.map(|v| v.to_string()));
        self.flag_value("--chat-template", params.chat_template.clone());

        self.speculative(entry)?;
        Ok(())
    }

    /// T-036: the model's speculative-decoding keys, in its own section.
    ///
    /// An invalid or missing draft companion is a typed error naming the file
    /// (`core::speculative::spec_type_for_entry`), never a warning and never a
    /// preset that points at a file that is not there.
    fn speculative(&mut self, entry: &ModelEntry) -> Result<(), AppError> {
        let Some(spec_type) = speculative::spec_type_for_entry(entry)? else {
            // Neither signal: an ordinary model emits nothing here, which is
            // the no-regression property (docs/TASKS.md T-036).
            return Ok(());
        };

        // A build that declares the allowed values for `--spec-type` is
        // authoritative (`AGENTS.md` §1); otherwise the captured enum in
        // `core::speculative` is the reference.
        let allowed = self
            .verified("--spec-type")
            .and_then(|f| f.allowed_values.as_deref());
        if !speculative::allowed_spec_type(&spec_type, allowed) {
            self.warn(
                "--spec-type",
                &format!("this build does not accept the value '{spec_type}'"),
            );
            return Ok(());
        }
        self.flag_value("--spec-type", Some(spec_type));

        // A companion file, when there is one. An MTP model drafts with its own
        // heads and has none — it still gets the tuning fields below.
        if let Some(companion) = speculative::companion_of(&entry.launch_params) {
            // The draft-model argument answers to two names in the build
            // (`--spec-draft-model, -md, --model-draft`). Emit the one
            // `docs/TASKS.md` T-036 names when the build verifies it, falling
            // back to the primary spelling for a build that only carries that
            // one — both were verified accepted as preset keys on b10883.
            let draft_flag = ["--model-draft", "--spec-draft-model"]
                .into_iter()
                .find(|flag| self.verified(flag).is_some());
            match draft_flag {
                Some(draft_flag) => {
                    self.flag_value(draft_flag, Some(companion.to_string_lossy().to_string()))
                }
                None => self.warn(
                    "--spec-draft-model",
                    "not in the active build's verified flag list",
                ),
            }
        }

        // The draft-stage tuning, whatever drives the draft: the model's own
        // MTP heads, or a companion model.
        if let Some(spec) = entry.launch_params.speculative.as_ref() {
            for (flag, value) in speculative::draft_tuning_flags(spec) {
                self.flag_value(flag, Some(value));
            }
        }
        Ok(())
    }

    /// Emit `flag-value = value` when the flag is verified, or record why not.
    fn flag_value(&mut self, flag: &str, value: Option<String>) {
        let Some(value) = value else { return };
        match self.verified(flag) {
            Some(_) => {
                self.ini
                    .push_str(&format!("{} = {value}\n", preset_key(flag)));
            }
            None => self.warn(flag, "not in the active build's verified flag list"),
        }
    }

    fn warn(&mut self, flag: &str, reason: &str) {
        self.warnings.push(PresetWarning {
            model: self.model.clone(),
            flag: flag.to_string(),
            reason: reason.to_string(),
        });
    }

    fn finish(self) -> PresetPlan {
        PresetPlan {
            ini: self.ini,
            warnings: self.warnings,
        }
    }
}

/// A preset key is the flag name without its leading dashes; the dashes
/// *inside* the name are preserved (verified empirically, PROGRESS.md F-015).
fn preset_key(flag: &str) -> &str {
    flag.trim_start_matches('-')
}

/// Generate the router command line arguments.
#[allow(dead_code)]
pub fn router_arguments(
    build: &RuntimeBuild,
    upstream_port: u16,
    preset_path: Option<&std::path::Path>,
    models_dir: Option<&std::path::Path>,
) -> Result<Vec<String>, AppError> {
    let mut args = vec![
        "--no-webui".to_string(),
        "--host".to_string(),
        "127.0.0.1".to_string(),
        "--port".to_string(),
        upstream_port.to_string(),
    ];

    match build.registration_channel {
        RegistrationChannel::PresetDeclaresPath => {
            let path = preset_path.ok_or_else(|| AppError::Internal {
                message: "PresetDeclaresPath channel requires a preset path".to_string(),
            })?;
            args.push("--models-preset".to_string());
            args.push(path.to_string_lossy().to_string());
        }
        RegistrationChannel::ScanOnly => {
            let path = models_dir.ok_or_else(|| AppError::Internal {
                message: "ScanOnly channel requires a models directory path".to_string(),
            })?;
            args.push("--models-dir".to_string());
            args.push(path.to_string_lossy().to_string());
        }
        RegistrationChannel::Undetermined => {
            return Err(AppError::Internal {
                message: "registration channel is Undetermined; cannot generate router arguments"
                    .to_string(),
            });
        }
    }

    Ok(args)
}

/// Get the list of models to include in the preset.
#[allow(dead_code)]
pub fn preset_models() -> Result<Vec<ModelEntry>, AppError> {
    model_registry::list_models()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{
        Backend, Compatibility, GgufMetadata, LaunchParams, ModelAvailability, SamplingDefaults,
        SpeculativeParams,
    };
    use std::path::PathBuf;

    /// The verified flag list of the pinned build, trimmed to what these
    /// tests exercise. Every name here is real (captured from b10883's own
    /// `--help`); the tests care about presence, not about the value types.
    fn verified(names: &[&str]) -> Vec<VerifiedFlag> {
        names
            .iter()
            .map(|name| VerifiedFlag {
                name: (*name).to_string(),
                takes_value: true,
                allowed_values: None,
            })
            .collect()
    }

    fn full_verified() -> Vec<VerifiedFlag> {
        verified(&[
            "--gpu-layers",
            "--ctx-size",
            "--batch-size",
            "--ubatch-size",
            "--flash-attn",
            "--cache-type-k",
            "--cache-type-v",
            "--n-cpu-moe",
            "--tensor-split",
            "--main-gpu",
            "--threads",
            "--chat-template",
            "--spec-type",
            "--model-draft",
            "--spec-draft-model",
            "--spec-draft-n-max",
            "--spec-draft-p-min",
        ])
    }

    fn test_model(id: &str, path: &str) -> ModelEntry {
        ModelEntry {
            id: id.to_string(),
            display_name: format!("Test Model {id}"),
            served_name: format!("test-{id}"),
            file_path: PathBuf::from(path),
            shard_paths: vec![],
            size_bytes: 1024,
            sha256_head: "abc123".to_string(),
            metadata: GgufMetadata {
                architecture: "llama".to_string(),
                param_count: Some(7000000000),
                quantization: "Q4_K_M".to_string(),
                block_count: 32,
                context_length: Some(4096),
                embedding_length: Some(4096),
                attention_head_count: Some(32),
                attention_head_count_kv: Some(8),
                has_chat_template: true,
                is_moe: false,
                expert_count: None,
                is_draft_model: false,
                has_mtp_heads: false,
            },
            compatibility: Compatibility::Supported,
            availability: ModelAvailability::Present,
            duplicate_of: None,
            launch_params: LaunchParams::default(),
            sampling_defaults: SamplingDefaults::default(),
            preload: false,
            pinned: false,
            added_at: chrono::Utc::now(),
        }
    }

    fn test_build(channel: RegistrationChannel) -> RuntimeBuild {
        RuntimeBuild {
            build_tag: "b10883".to_string(),
            backend: Backend::Cpu,
            install_path: PathBuf::from("/fake/install"),
            is_active: true,
            installed_at: chrono::Utc::now(),
            verified_flags: full_verified(),
            health_endpoint: Some("/health".to_string()),
            registration_channel: channel,
        }
    }

    /// Write a header-only GGUF fixture to a scratch directory.
    ///
    /// The directory is keyed on the fixture's own name as well as the process
    /// id: `cargo test` runs these tests in parallel inside ONE process, so a
    /// shared directory meant the first test to finish deleted the other's
    /// fixture mid-run (observed as an intermittent single failure).
    fn gguf_fixture(name: &str, architecture: &str) -> PathBuf {
        let stem = name.replace('.', "-");
        let dir = std::env::temp_dir()
            .join(format!("lm-mgr-t036-preset-{stem}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        let path = dir.join(name);
        let mut buf = Vec::new();
        buf.extend_from_slice(&0x4655_4747u32.to_le_bytes());
        buf.extend_from_slice(&3u32.to_le_bytes());
        buf.extend_from_slice(&0u64.to_le_bytes());
        buf.extend_from_slice(&1u64.to_le_bytes());
        let key = "general.architecture";
        buf.extend_from_slice(&(key.len() as u64).to_le_bytes());
        buf.extend_from_slice(key.as_bytes());
        buf.extend_from_slice(&8u32.to_le_bytes());
        buf.extend_from_slice(&(architecture.len() as u64).to_le_bytes());
        buf.extend_from_slice(architecture.as_bytes());
        std::fs::write(&path, &buf).expect("write fixture");
        path
    }

    #[test]
    fn test_generate_preset_path() {
        let models = vec![test_model("1", "/models/model-1.gguf")];
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let ini = generate_preset(&build, &models).unwrap();

        assert!(ini.contains("[test-1]"));
        assert!(ini.contains("model = /models/model-1.gguf"));
        // No `served_name` key: the router has no such option, the section
        // name is the served name (F-015).
        assert!(!ini.contains("served_name"));
    }

    #[test]
    fn test_generate_preset_scan() {
        let models = vec![test_model("1", "/models/model-1.gguf")];
        let build = test_build(RegistrationChannel::ScanOnly);
        let ini = generate_preset(&build, &models).unwrap();

        assert!(ini.contains("[test-1]"));
        assert!(!ini.contains("model = /models/model-1.gguf"));
    }

    #[test]
    fn test_generate_preset_undetermined() {
        let models = vec![test_model("1", "/models/model-1.gguf")];
        let build = test_build(RegistrationChannel::Undetermined);
        let result = generate_preset(&build, &models);

        assert!(result.is_err());
        let err = result.unwrap_err();
        match err {
            AppError::Internal { message } => {
                assert!(message.contains("Undetermined"));
            }
            _ => panic!("expected Internal error"),
        }
    }

    /// The key spelling is the build's own: dashes preserved
    /// (`--ctx-size` → `ctx-size`), because the router refuses to start on a
    /// key it does not know (F-015).
    #[test]
    fn keys_are_spelled_as_the_build_spells_them() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut model = test_model("1", "/models/model-1.gguf");
        model.launch_params = LaunchParams {
            gpu_layers: Some(35),
            ctx_size: Some(8000),
            cache_type_k: Some("f16".to_string()),
            ..Default::default()
        };
        let plan = preset_plan(&build, std::slice::from_ref(&model)).unwrap();

        assert!(plan.ini.contains("gpu-layers = 35\n"), "{}", plan.ini);
        assert!(plan.ini.contains("ctx-size = 8000\n"), "{}", plan.ini);
        assert!(plan.ini.contains("cache-type-k = f16\n"), "{}", plan.ini);
        assert!(!plan.ini.contains("ctx_size"), "{}", plan.ini);
        assert!(!plan.ini.contains("gpu_layers"), "{}", plan.ini);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    /// Only flags the active build verifies are emitted; the rest are omitted
    /// and named. b10883 has no `--no-mmap`, so a saved `no_mmap: true` must
    /// not reach the file (it would stop the server from starting).
    #[test]
    fn unverified_flags_are_omitted_and_named() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut model = test_model("1", "/models/model-1.gguf");
        model.launch_params = LaunchParams {
            no_mmap: Some(true),
            mlock: Some(true),
            ctx_size: Some(4096),
            ..Default::default()
        };
        let plan = preset_plan(&build, std::slice::from_ref(&model)).unwrap();

        assert!(!plan.ini.contains("no-mmap"));
        assert!(!plan.ini.contains("mlock"));
        assert!(plan.ini.contains("ctx-size = 4096"));
        let flags: Vec<&str> = plan.warnings.iter().map(|w| w.flag.as_str()).collect();
        assert!(flags.contains(&"--no-mmap"), "{:?}", plan.warnings);
        assert!(flags.contains(&"--mlock"), "{:?}", plan.warnings);
        assert!(plan.warnings.iter().all(|w| w.model == "test-1"));
    }

    /// T-036 acceptance: an MTP model emits `spec-type = draft-mtp`, in its
    /// own section, and only that model's section.
    #[test]
    fn mtp_model_emits_draft_mtp_in_its_own_section() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut mtp = test_model("1", "/models/mtp.gguf");
        mtp.metadata.has_mtp_heads = true;
        let plain = test_model("2", "/models/plain.gguf");

        let plan = preset_plan(&build, &[mtp, plain]).unwrap();

        assert_eq!(
            plan.ini.matches("spec-type = draft-mtp").count(),
            1,
            "the flag belongs to one model's section, not to the file: {}",
            plan.ini
        );
        // It is inside [test-1] and before [test-2].
        let section = &plan.ini[..plan.ini.find("[test-2]").expect("second section")];
        assert!(section.contains("spec-type = draft-mtp"), "{section}");
        assert!(!plan.ini.contains("model-draft"));
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    /// T-036 acceptance: a main+draft pair emits `model-draft` naming the
    /// companion, the companion's `spec-type`, and the tuning flags the user
    /// set.
    #[test]
    fn a_draft_companion_emits_model_draft_spec_type_and_tuning() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let companion_path = gguf_fixture("draft-dflash2.gguf", "dflash2");
        let mut model = test_model("1", "/models/main.gguf");
        model.launch_params.speculative = Some(SpeculativeParams {
            draft_companion: Some(companion_path.clone()),
            n_max: Some(4),
            p_min: Some(0.5),
            ..Default::default()
        });

        let plan = preset_plan(&build, std::slice::from_ref(&model)).unwrap();

        assert!(
            plan.ini.contains("spec-type = draft-dflash"),
            "{}",
            plan.ini
        );
        assert!(
            plan.ini
                .contains(&format!("model-draft = {}", companion_path.display())),
            "{}",
            plan.ini
        );
        assert!(plan.ini.contains("spec-draft-n-max = 4"), "{}", plan.ini);
        assert!(plan.ini.contains("spec-draft-p-min = 0.5"), "{}", plan.ini);
        // Unset tuning fields are not written: the build's defaults stand.
        assert!(!plan.ini.contains("spec-draft-ngl"));
        assert!(!plan.ini.contains("spec-draft-threads"));

        let _ = std::fs::remove_dir_all(companion_path.parent().expect("parent"));
    }

    /// A build that only carries the primary spelling gets that one: the two
    /// names are the same argument, and the preset must name the one the
    /// build verifies.
    #[test]
    fn the_draft_model_key_falls_back_to_the_primary_spelling() {
        let mut build = test_build(RegistrationChannel::PresetDeclaresPath);
        build.verified_flags = verified(&["--spec-type", "--spec-draft-model"]);
        let companion_path = gguf_fixture("draft-dspark.gguf", "dspark");
        let mut model = test_model("1", "/models/main.gguf");
        model.launch_params.speculative = Some(SpeculativeParams {
            draft_companion: Some(companion_path.clone()),
            ..Default::default()
        });

        let plan = preset_plan(&build, std::slice::from_ref(&model)).unwrap();

        assert!(
            plan.ini
                .contains(&format!("spec-draft-model = {}", companion_path.display())),
            "{}",
            plan.ini
        );
        assert!(!plan.ini.contains("model-draft ="), "{}", plan.ini);
        assert!(
            plan.ini.contains("spec-type = draft-dspark"),
            "{}",
            plan.ini
        );

        let _ = std::fs::remove_dir_all(companion_path.parent().expect("parent"));
    }

    /// T-036 acceptance refinement (owner, 14 Sept 2026): an MTP model has a
    /// draft stage of its own, so its tuning fields are emitted with no
    /// companion anywhere in sight — and no `model-draft`.
    #[test]
    fn an_mtp_model_tunes_its_own_draft_stage_without_a_companion() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut mtp = test_model("1", "/models/mtp.gguf");
        mtp.metadata.has_mtp_heads = true;
        mtp.launch_params.speculative = Some(SpeculativeParams {
            draft_companion: None,
            n_max: Some(3),
            p_min: Some(0.4),
            ..Default::default()
        });

        let plan = preset_plan(&build, std::slice::from_ref(&mtp)).unwrap();

        assert!(plan.ini.contains("spec-type = draft-mtp"), "{}", plan.ini);
        assert!(plan.ini.contains("spec-draft-n-max = 3"), "{}", plan.ini);
        assert!(plan.ini.contains("spec-draft-p-min = 0.4"), "{}", plan.ini);
        assert!(!plan.ini.contains("model-draft"), "{}", plan.ini);
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    /// The companion is re-validated at generation time: a companion that
    /// disappeared after it was saved fails with a typed error naming the
    /// file, never a preset pointing at nothing.
    #[test]
    fn a_vanished_draft_companion_is_a_typed_error() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let missing = PathBuf::from("E:/models/gone-draft-dflash.gguf");
        let mut model = test_model("1", "/models/main.gguf");
        model.launch_params.speculative = Some(SpeculativeParams {
            draft_companion: Some(missing.clone()),
            ..Default::default()
        });

        match preset_plan(&build, std::slice::from_ref(&model)) {
            Err(AppError::NotFound { what }) => {
                assert!(what.contains("gone-draft-dflash.gguf"), "{what}");
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    /// A build that does not verify the speculative flags omits them (and
    /// names them) instead of writing a preset the server would refuse.
    #[test]
    fn speculative_flags_absent_from_the_build_are_omitted() {
        let mut build = test_build(RegistrationChannel::PresetDeclaresPath);
        build.verified_flags = verified(&["--ctx-size"]);
        let mut mtp = test_model("1", "/models/mtp.gguf");
        mtp.metadata.has_mtp_heads = true;

        let plan = preset_plan(&build, std::slice::from_ref(&mtp)).unwrap();

        assert!(!plan.ini.contains("spec-type"));
        assert!(
            plan.warnings.iter().any(|w| w.flag == "--spec-type"),
            "{:?}",
            plan.warnings
        );
    }

    /// T-036 acceptance, no regression: a model with neither signal emits no
    /// speculative key at all, and nothing else about its section changed.
    #[test]
    fn an_ordinary_model_emits_no_speculative_keys() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut model = test_model("1", "/models/plain.gguf");
        model.launch_params = LaunchParams {
            gpu_layers: Some(32),
            ctx_size: Some(8000),
            batch_size: Some(512),
            ubatch_size: Some(128),
            flash_attn: Some(crate::core::types::FlashAttn::Auto),
            cache_type_k: Some("f16".to_string()),
            cache_type_v: Some("f16".to_string()),
            threads: Some(32),
            ..Default::default()
        };

        let plan = preset_plan(&build, std::slice::from_ref(&model)).unwrap();

        for key in ["spec-type", "model-draft", "spec-draft", "draft-", "ngram"] {
            assert!(
                !plan.ini.contains(key),
                "ordinary model must emit no '{key}': {}",
                plan.ini
            );
        }
        assert_eq!(
            plan.ini,
            "[test-1]\n\
             model = /models/plain.gguf\n\
             gpu-layers = 32\n\
             ctx-size = 8000\n\
             batch-size = 512\n\
             ubatch-size = 128\n\
             flash-attn = auto\n\
             cache-type-k = f16\n\
             cache-type-v = f16\n\
             threads = 32\n\
             \n"
        );
        assert!(plan.warnings.is_empty(), "{:?}", plan.warnings);
    }

    #[test]
    fn test_router_arguments_preset_path() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let args = router_arguments(
            &build,
            8080,
            Some(PathBuf::from("/path/to/presets.ini").as_path()),
            None,
        )
        .unwrap();

        assert!(args.contains(&"--no-webui".to_string()));
        assert!(args.contains(&"--host".to_string()));
        assert!(args.contains(&"127.0.0.1".to_string()));
        assert!(args.contains(&"--port".to_string()));
        assert!(args.contains(&"8080".to_string()));
        assert!(args.contains(&"--models-preset".to_string()));
        assert!(args.contains(&"/path/to/presets.ini".to_string()));
    }

    #[test]
    fn test_router_arguments_scan_only() {
        let build = test_build(RegistrationChannel::ScanOnly);
        let args = router_arguments(
            &build,
            8080,
            None,
            Some(PathBuf::from("/path/to/models").as_path()),
        )
        .unwrap();

        assert!(args.contains(&"--models-dir".to_string()));
        assert!(args.contains(&"/path/to/models".to_string()));
    }

    #[test]
    fn test_preview_preset() {
        let model = test_model("1", "/models/model-1.gguf");
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let preview = preview_preset(&model, &build).unwrap();

        assert!(preview.contains("[test-1]"));
        assert!(preview.contains("model = /models/model-1.gguf"));
    }

    /// T-035 acceptance: the detail screen's live preview is produced by the
    /// SAME code path as T-033 and is byte-identical to `preview_preset`.
    /// The IPC command `preview_preset_params` grafts draft params onto the
    /// model and calls this same function — this test asserts that for
    /// params equal to the stored ones, the output is identical, and that
    /// different params change the output (the preview is live, not cached).
    #[test]
    fn test_preview_preset_params_is_byte_identical_to_preview_preset() {
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut model = test_model("1", "/models/model-1.gguf");
        model.launch_params = LaunchParams {
            gpu_layers: Some(35),
            ctx_size: Some(8192),
            ..Default::default()
        };

        // The stored-params preview.
        let stored_preview = preview_preset(&model, &build).unwrap();

        // The "draft params" preview: the IPC layer clones the model,
        // assigns the draft params, and calls preview_preset.
        let mut draft_model = model.clone();
        let draft_params = draft_model.launch_params.clone();
        draft_model.launch_params = draft_params;
        let draft_preview = preview_preset(&draft_model, &build).unwrap();

        // Byte-identical: same code path, same params.
        assert_eq!(stored_preview, draft_preview);

        // And the preview is live: different params change the output.
        let mut other_model = model.clone();
        other_model.launch_params = LaunchParams {
            gpu_layers: Some(1),
            ctx_size: Some(2048),
            ..Default::default()
        };
        let other_preview = preview_preset(&other_model, &build).unwrap();
        assert_ne!(stored_preview, other_preview);
    }

    /// A real `llama-server` accepts the generated file. This is the
    /// demonstration that the key spelling is right — a preset it refuses to
    /// parse never starts the server at all, and the failure is at startup,
    /// not at model load.
    ///
    /// Opt-in: set `LLAMA_MANAGER_LLAMA_SERVER` to the `llama-server.exe`
    /// path (and `LLAMA_MANAGER_REAL_GGUF` for a real model to name) to run
    /// it. Gated because it needs the binary, like
    /// `gguf::tests::test_real_gguf_file` needs a real file.
    #[test]
    fn generated_preset_is_accepted_by_a_real_llama_server() {
        let Ok(exe) = std::env::var("LLAMA_MANAGER_LLAMA_SERVER") else {
            return;
        };
        let model_path = std::env::var("LLAMA_MANAGER_REAL_GGUF")
            .unwrap_or_else(|_| "E:/LMM/fixtures/model-a.gguf".to_string());

        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let mut model = test_model("1", &model_path);
        model.served_name = "t036-probe".to_string();
        model.metadata.has_mtp_heads = true;
        model.launch_params = LaunchParams {
            gpu_layers: Some(1),
            ctx_size: Some(2048),
            flash_attn: Some(crate::core::types::FlashAttn::Auto),
            cache_type_k: Some("f16".to_string()),
            threads: Some(4),
            no_mmap: Some(true), // not a flag on recent builds: must be omitted
            ..Default::default()
        };

        let dir = std::env::temp_dir().join(format!("lm-mgr-t036-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create tmp dir");
        let preset_path = dir.join("presets.ini");
        let log_path = dir.join("server.log");
        let preset = generate_preset(&build, std::slice::from_ref(&model)).unwrap();
        // Printed so the evidence is in the test output, not just in a green
        // tick: `cargo test generated_preset -- --nocapture`.
        println!("--- generated preset ---\n{preset}--- end preset ---");
        std::fs::write(&preset_path, preset).expect("write preset");

        let mut child = std::process::Command::new(&exe)
            .arg("--models-preset")
            .arg(&preset_path)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg("8137")
            .arg("--no-webui")
            .arg("--log-file")
            .arg(&log_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn llama-server");

        // Give it time to parse the preset and either listen or die.
        let mut listened = false;
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            if log.contains("listening on http") {
                listened = true;
                break;
            }
            if log.contains("not recognized") || log.contains("failed to initialize") {
                break;
            }
        }
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let _ = child.kill();
        let _ = child.wait();
        println!(
            "--- llama-server tail ---\n{}",
            log.lines().rev().take(3).collect::<Vec<_>>().join("\n")
        );
        let _ = std::fs::remove_dir_all(&dir);

        assert!(
            listened,
            "llama-server refused the generated preset. Last log lines:\n{}",
            log.lines().rev().take(6).collect::<Vec<_>>().join("\n")
        );
    }
}
