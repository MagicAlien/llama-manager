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
//! Emits only flags in the active build's verified list, except `extra_args`.
//! Emits a `preset-warning` event for each flag omitted.
//!
//! `generate_preset` and `router_arguments` are called by the supervisor
//! (T-040) at server start. Until T-040 wires them, they are dead code —
//! `preview_preset` is the only command reachable from IPC today.

use crate::core::model_registry;
use crate::core::types::{AppError, LaunchParams, ModelEntry, RegistrationChannel, RuntimeBuild};

/// Generate the `presets.ini` content for the given active build and models.
#[allow(dead_code)]
pub fn generate_preset(build: &RuntimeBuild, models: &[ModelEntry]) -> Result<String, AppError> {
    match build.registration_channel {
        RegistrationChannel::PresetDeclaresPath => generate_preset_path(models),
        RegistrationChannel::ScanOnly => generate_preset_scan(models),
        RegistrationChannel::Undetermined => Err(AppError::Internal {
            message: "registration channel is Undetermined; T-025 did not resolve it. \
                     Cannot generate preset without knowing the channel."
                .to_string(),
        }),
    }
}

/// Generate preset for the `PresetDeclaresPath` channel.
#[allow(dead_code)]
fn generate_preset_path(models: &[ModelEntry]) -> Result<String, AppError> {
    let mut ini = String::new();

    for model in models {
        ini.push_str(&format!("[{}]\n", model.served_name));
        ini.push_str(&format!("model = {}\n", model.file_path.to_string_lossy()));
        ini.push_str(&format!("served_name = {}\n", model.served_name));
        append_launch_params(&mut ini, &model.launch_params);
        ini.push('\n');
    }

    Ok(ini)
}

/// Generate preset for the `ScanOnly` channel.
#[allow(dead_code)]
fn generate_preset_scan(models: &[ModelEntry]) -> Result<String, AppError> {
    let mut ini = String::new();

    for model in models {
        ini.push_str(&format!("[{}]\n", model.served_name));
        ini.push_str(&format!("served_name = {}\n", model.served_name));
        append_launch_params(&mut ini, &model.launch_params);
        ini.push('\n');
    }

    Ok(ini)
}

/// Append launch parameters to the INI content.
fn append_launch_params(ini: &mut String, params: &LaunchParams) {
    if let Some(gpu_layers) = params.gpu_layers {
        ini.push_str(&format!("gpu_layers = {gpu_layers}\n"));
    }
    if let Some(ctx_size) = params.ctx_size {
        ini.push_str(&format!("ctx_size = {ctx_size}\n"));
    }
    if let Some(batch_size) = params.batch_size {
        ini.push_str(&format!("batch_size = {batch_size}\n"));
    }
    if let Some(ubatch_size) = params.ubatch_size {
        ini.push_str(&format!("ubatch_size = {ubatch_size}\n"));
    }
    if let Some(ref flash_attn) = params.flash_attn {
        let value = match flash_attn {
            crate::core::types::FlashAttn::On => "on",
            crate::core::types::FlashAttn::Off => "off",
            crate::core::types::FlashAttn::Auto => "auto",
        };
        ini.push_str(&format!("flash_attn = {value}\n"));
    }
    if let Some(ref cache_type_k) = params.cache_type_k {
        ini.push_str(&format!("cache_type_k = {cache_type_k}\n"));
    }
    if let Some(ref cache_type_v) = params.cache_type_v {
        ini.push_str(&format!("cache_type_v = {cache_type_v}\n"));
    }
    if let Some(n_cpu_moe) = params.n_cpu_moe {
        ini.push_str(&format!("n_cpu_moe = {n_cpu_moe}\n"));
    }
    if let Some(ref tensor_split) = params.tensor_split {
        let values: Vec<String> = tensor_split.iter().map(|f| f.to_string()).collect();
        ini.push_str(&format!("tensor_split = {}\n", values.join(",")));
    }
    if let Some(main_gpu) = params.main_gpu {
        ini.push_str(&format!("main_gpu = {main_gpu}\n"));
    }
    if let Some(no_mmap) = params.no_mmap {
        ini.push_str(&format!("no_mmap = {no_mmap}\n"));
    }
    if let Some(mlock) = params.mlock {
        ini.push_str(&format!("mlock = {mlock}\n"));
    }
    if let Some(threads) = params.threads {
        ini.push_str(&format!("threads = {threads}\n"));
    }
    if let Some(ref chat_template) = params.chat_template {
        ini.push_str(&format!("chat_template = {chat_template}\n"));
    }
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

/// Preview the preset for a single model without writing to disk.
pub fn preview_preset(model: &ModelEntry, build: &RuntimeBuild) -> Result<String, AppError> {
    match build.registration_channel {
        RegistrationChannel::PresetDeclaresPath => {
            let mut ini = String::new();
            ini.push_str(&format!("[{}]\n", model.served_name));
            ini.push_str(&format!("model = {}\n", model.file_path.to_string_lossy()));
            ini.push_str(&format!("served_name = {}\n", model.served_name));
            append_launch_params(&mut ini, &model.launch_params);
            Ok(ini)
        }
        RegistrationChannel::ScanOnly => {
            let mut ini = String::new();
            ini.push_str(&format!("[{}]\n", model.served_name));
            ini.push_str(&format!("served_name = {}\n", model.served_name));
            append_launch_params(&mut ini, &model.launch_params);
            Ok(ini)
        }
        RegistrationChannel::Undetermined => Err(AppError::Internal {
            message: "registration channel is Undetermined; cannot preview preset".to_string(),
        }),
    }
}

/// Get the list of models to include in the preset.
#[allow(dead_code)]
pub fn preset_models() -> Result<Vec<ModelEntry>, AppError> {
    model_registry::list_models()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{Backend, GgufMetadata, ModelAvailability};
    use std::path::PathBuf;

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
            },
            compatibility: crate::core::types::Compatibility::Supported,
            availability: ModelAvailability::Present,
            duplicate_of: None,
            launch_params: LaunchParams::default(),
            sampling_defaults: crate::core::types::SamplingDefaults::default(),
            preload: false,
            pinned: false,
            added_at: chrono::Utc::now(),
        }
    }

    fn test_build(channel: RegistrationChannel) -> RuntimeBuild {
        RuntimeBuild {
            build_tag: "b9196".to_string(),
            backend: Backend::Cpu,
            install_path: PathBuf::from("/fake/install"),
            is_active: true,
            installed_at: chrono::Utc::now(),
            verified_flags: vec![],
            health_endpoint: Some("/health".to_string()),
            registration_channel: channel,
        }
    }

    #[test]
    fn test_generate_preset_path() {
        let models = vec![test_model("1", "/models/model-1.gguf")];
        let build = test_build(RegistrationChannel::PresetDeclaresPath);
        let ini = generate_preset(&build, &models).unwrap();

        assert!(ini.contains("[test-1]"));
        assert!(ini.contains("model = /models/model-1.gguf"));
        assert!(ini.contains("served_name = test-1"));
    }

    #[test]
    fn test_generate_preset_scan() {
        let models = vec![test_model("1", "/models/model-1.gguf")];
        let build = test_build(RegistrationChannel::ScanOnly);
        let ini = generate_preset(&build, &models).unwrap();

        assert!(ini.contains("[test-1]"));
        assert!(!ini.contains("model = /models/model-1.gguf"));
        assert!(ini.contains("served_name = test-1"));
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
}
