//! Contract types — transcribed from `docs/CONTRACTS.md` §1.
//!
//! `AGENTS.md` invariant 9: this file is Rust, not generated. What *is*
//! generated is `src/lib/types.ts`, produced from these definitions by
//! `npm run generate-types` (`scripts/generate-types.mjs`), which runs the
//! `#[ts(export)]` tests below via `cargo test` and collects their output.
//! Never hand-edit `src/lib/types.ts` itself.
//!
//! Two corrections to the literal text of `docs/CONTRACTS.md` §1, both
//! recorded in `PROGRESS.md` rather than applied silently:
//!
//! - `FlashAttn` and `AppError` gained `PartialEq`: `LaunchParams` and
//!   `EndpointState` derive `PartialEq` and contain them, which does not
//!   compile otherwise. See D-009.
//! - `AppError` keeps the `thiserror::Error` derive CONTRACTS.md shows, with
//!   `#[error("...")]` messages added (thiserror requires one per variant).
//!   The `code()` / `message()` / `remediation()` methods themselves are
//!   T-004's, per the doc comment on the enum below — only the derive lives
//!   here.

// Nothing outside `#[cfg(test)]` constructs these types yet — the
// modules that will (T-003 onward, per `docs/TASKS.md`) haven't landed.
// In a `bin` crate (this one; no `src-tauri/src/lib.rs`), `pub` alone
// does not exempt an item from `dead_code`, the way it would in a
// library — nothing external can ever "use" a binary's public items,
// so rustc has no reason to assume they're reachable. Suppressed here,
// narrowly, rather than at the crate level; each consuming task should
// remove the corresponding allow as it wires its module in, not leave
// this blanket for the life of the project.
#![allow(dead_code)]

use std::net::IpAddr;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ─── Errors ─────────────────────────────────────────────────────

/// Full behaviour (`code()`, `message()`, `remediation()`) defined in T-004.
/// Every IPC command returns `Result<T, AppError>`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, thiserror::Error, TS)]
#[serde(tag = "kind", content = "detail")]
#[ts(export)]
pub enum AppError {
    #[error("invalid transition: cannot run `{command}` while in `{from}`")]
    InvalidTransition { from: String, command: String },
    #[error("not found: {what}")]
    NotFound { what: String },
    #[error("I/O error: {message}")]
    Io { message: String },
    #[error("database error: {message}")]
    Database { message: String },
    #[error("network error: {message}")]
    Network { message: String },
    #[error("rate limited; retry after {retry_after_seconds}s")]
    RateLimited { retry_after_seconds: u64 },
    #[error("checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("unsafe archive entry: {entry}")]
    UnsafeArchiveEntry { entry: String },
    #[error("invalid path `{path}`: {reason}")]
    InvalidPath { path: String, reason: String },
    #[error("GGUF parse error: {message}")]
    GgufParse { message: String },
    #[error("port {port} is already in use")]
    PortInUse { port: u16 },
    #[error("upstream unavailable (state: {state})")]
    UpstreamUnavailable { state: String },
    #[error("unauthorized")]
    Unauthorized,
    #[error("database schema {found} is newer than supported {known}")]
    SchemaTooNew { found: u32, known: u32 },
    #[error("internal error: {message}")]
    Internal { message: String },
}

// ─── Environment ────────────────────────────────────────────────

/// Open on purpose: a CUDA major the binary has never heard of must be
/// representable, not a parse failure. See `PLAN.md` §2.13.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Backend {
    Cuda { major: u8 },
    Vulkan,
    Cpu,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct GpuInfo {
    pub index: u32,
    pub name: String,
    /// Blackwell = (12, 0) or (10, x).
    pub compute_capability: (u32, u32),
    pub vram_total_bytes: u64,
    pub vram_free_bytes: u64,
    pub driver_version: String,
    pub cuda_version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct HealthCheck {
    /// "gpu_present" | "driver_ok" | "cuda13_ok" | "disk_space" | "endpoint_bindable"
    pub id: String,
    pub status: CheckStatus,
    pub message: String,
    pub remediation: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct EnvironmentReport {
    pub gpus: Vec<GpuInfo>,
    pub system_ram_bytes: u64,
    pub free_disk_bytes: u64,
    pub os_build: String,
    pub checks: Vec<HealthCheck>,
}

// ─── Runtime ────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct VerifiedFlag {
    /// "--flash-attn"
    pub name: String,
    pub takes_value: bool,
    pub allowed_values: Option<Vec<String>>,
}

/// How this build lets a model enter the router's registry.
/// Settled by parsing `--help` in T-023; see `PLAN.md` §2.1.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum RegistrationChannel {
    /// `--models-preset` entries may declare a model by absolute path.
    PresetDeclaresPath,
    /// Only `--models-dir` scanning registers models; the app links into a
    /// directory it owns.
    ScanOnly,
    /// `--help` did not answer the question. T-033 must not proceed on a guess.
    Undetermined,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct RuntimeBuild {
    /// "b9196"
    pub build_tag: String,
    pub backend: Backend,
    pub install_path: PathBuf,
    pub is_active: bool,
    pub installed_at: DateTime<Utc>,
    pub verified_flags: Vec<VerifiedFlag>,
    pub registration_channel: RegistrationChannel,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct AvailableRelease {
    pub build_tag: String,
    pub backend: Backend,
    pub asset_name: String,
    pub asset_url: String,
    /// `None` when the release omits checksums.
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub published_at: DateTime<Utc>,
    pub release_notes_url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub enum InstallProgress {
    Resolving,
    Downloading {
        received_bytes: u64,
        total_bytes: u64,
        bytes_per_sec: u64,
    },
    Verifying,
    Extracting {
        entries_done: u32,
        entries_total: u32,
    },
    Registering,
    Done {
        build: RuntimeBuild,
    },
    Failed {
        error: AppError,
    },
}

// ─── Models ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct GgufMetadata {
    /// "llama" | "qwen3" | "deepseek2" | ...
    pub architecture: String,
    pub param_count: Option<u64>,
    /// "Q4_K_M" | "MXFP4" | "NVFP4" | "F16" | ...
    pub quantization: String,
    /// Layer count — drives `-ngl`.
    pub block_count: u32,
    pub context_length: Option<u32>,
    pub embedding_length: Option<u32>,
    /// Both are required by the KV cache term of the estimator. With Grouped
    /// Query Attention the ratio between them reaches 8:1, so ignoring
    /// `attention_head_count_kv` misestimates the cache by nearly an order
    /// of magnitude. When either is absent the estimator says so in its
    /// notes rather than assuming they are equal.
    pub attention_head_count: Option<u32>,
    pub attention_head_count_kv: Option<u32>,
    pub has_chat_template: bool,
    pub is_moe: bool,
    pub expert_count: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct CompatibilityNote {
    pub text: String,
    /// NVFP4 sets this; see `PLAN.md` §2.6.
    pub experimental: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub enum Compatibility {
    Supported,
    SupportedWithWarnings(Vec<CompatibilityNote>),
    Unsupported(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum ModelAvailability {
    Present,
    /// Path no longer resolves: file renamed, drive disconnected.
    Missing,
    /// Path resolves but the file cannot be opened.
    Unreadable,
}

/// What linking primitive this machine can actually use, established by
/// attempting one in a scratch directory rather than by reading a privilege.
/// The three are not interchangeable on Windows: a junction cannot link an
/// individual file, and a hard link cannot cross volumes. See `PLAN.md` §2.1.
/// Only reached under the `ScanOnly` registration channel.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum LinkCapability {
    /// File symlinks work: exact, per-file, any volume. The preferred fallback.
    Symlink,
    /// No symlink privilege; hard links work for targets on the app's volume.
    HardLinkOnly,
    /// Neither. Under `ScanOnly` this machine cannot register a model at
    /// all, and the app says so plainly rather than linking a whole folder
    /// and hoping.
    None,
}

/// Tri-state. Recent builds accept `on|off|auto` rather than a boolean.
/// VERIFY the type against `docs/verified-flags.md` before use.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum FlashAttn {
    On,
    Off,
    Auto,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, TS)]
#[ts(export)]
pub struct LaunchParams {
    pub gpu_layers: Option<u32>,
    pub ctx_size: Option<u32>,
    pub batch_size: Option<u32>,
    pub ubatch_size: Option<u32>,
    pub flash_attn: Option<FlashAttn>,
    pub cache_type_k: Option<String>,
    pub cache_type_v: Option<String>,
    pub n_cpu_moe: Option<u32>,
    pub tensor_split: Option<Vec<f32>>,
    pub main_gpu: Option<u32>,
    pub no_mmap: Option<bool>,
    pub mlock: Option<bool>,
    pub threads: Option<u32>,
    pub chat_template: Option<String>,
    pub mmproj_path: Option<PathBuf>,
    /// Unvalidated passthrough. User-entered only — never set by app logic.
    /// See `AGENTS.md` §1.
    pub extra_args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, TS)]
#[ts(export)]
pub struct SamplingDefaults {
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub min_p: Option<f32>,
    pub repeat_penalty: Option<f32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub seed: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct WatchedFolder {
    pub path: PathBuf,
    pub model_count: u32,
    pub reachable: bool,
    pub last_scan_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct ModelEntry {
    /// Stable slug derived from the absolute path.
    pub id: String,
    pub display_name: String,
    /// What `/v1/models` exposes; unique across the catalogue.
    pub served_name: String,
    /// Absolute, wherever the user keeps it — never moved.
    pub file_path: PathBuf,
    pub shard_paths: Vec<PathBuf>,
    pub size_bytes: u64,
    /// Hash of first 1 MiB + size — duplicate *signal*, not identity.
    pub sha256_head: String,
    pub metadata: GgufMetadata,
    pub compatibility: Compatibility,
    pub availability: ModelAvailability,
    /// Another entry with the same `sha256_head`.
    pub duplicate_of: Option<String>,
    pub launch_params: LaunchParams,
    pub sampling_defaults: SamplingDefaults,
    /// Loaded at server start rather than on demand.
    pub preload: bool,
    /// Exempt from LRU eviction.
    pub pinned: bool,
    pub added_at: DateTime<Utc>,
}

/// Survives removal of the catalogue entry, keyed by the model's absolute
/// path. Re-importing the same file restores it. See `PLAN.md` §2.9.
#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct RetainedModelSettings {
    pub file_path: PathBuf,
    pub launch_params: LaunchParams,
    pub sampling_defaults: SamplingDefaults,
    pub preload: bool,
    pub pinned: bool,
    pub retained_at: DateTime<Utc>,
}

pub type ImportJobId = String;

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub enum ImportProgress {
    Queued {
        job_id: ImportJobId,
        total_files: u32,
    },
    FileStarted {
        job_id: ImportJobId,
        path: PathBuf,
        index: u32,
        total: u32,
    },
    FileDone {
        job_id: ImportJobId,
        entry: ModelEntry,
        index: u32,
        total: u32,
    },
    FileFailed {
        job_id: ImportJobId,
        path: PathBuf,
        error: AppError,
    },
    Cancelled {
        job_id: ImportJobId,
        completed: u32,
    },
    Finished {
        job_id: ImportJobId,
        imported: u32,
        skipped: u32,
        failed: u32,
    },
}

// ─── Estimation ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum EstimateConfidence {
    Calibrated,
    Heuristic,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct LaunchRecord {
    /// Absolute path, not the entry id: history outlives the catalogue entry.
    pub file_path: PathBuf,
    pub launched_at: DateTime<Utc>,
    pub params: LaunchParams,
    pub succeeded: bool,
    pub actual_vram_bytes: Option<u64>,
    pub load_seconds: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct EstimateInputs {
    pub metadata: GgufMetadata,
    pub file_size_bytes: u64,
    pub params: LaunchParams,
    pub vram_free_bytes: u64,
    pub ram_free_bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct VramEstimate {
    pub recommended_gpu_layers: u32,
    pub estimated_vram_bytes: u64,
    pub estimated_ram_bytes: u64,
    pub kv_cache_bytes: u64,
    pub fits_fully: bool,
    pub confidence: EstimateConfidence,
    pub notes: Vec<String>,
}

// `pub fn estimate(...)` (docs/CONTRACTS.md §1) is logic, not a type — T-032's
// scope, not exported here.

// ─── Endpoint (PLAN.md §2.7) ────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct ServerConfig {
    /// The address clients configure. Owned by the app, stable across restarts.
    pub listen_address: IpAddr,
    /// Default 8080; never auto-incremented.
    pub listen_port: u16,
    /// Range the app draws the upstream loopback port from. Internal.
    /// Default `(49500, 49999)`.
    pub upstream_port_range: (u16, u16),
    /// How long a request is held while the coordinator is not yet
    /// answering. Default 120.
    pub startup_hold_seconds: u32,
    /// How long the coordinator itself may take to answer before Crashed.
    /// Default 120.
    pub process_timeout_seconds: u32,
    /// How long preloading may take before Crashed. Separate from the above
    /// because a 65 GB preload is minutes, not seconds, and one timeout
    /// cannot serve both without being uselessly loose for the first.
    /// Default 900.
    pub preload_timeout_seconds: u32,
    /// `--models-max`; 1 = strict hot-swap.
    pub models_max: u32,
    /// `--models-autoload`.
    pub autoload: bool,
    /// **No flag is known to implement this.** It is modelled here and
    /// shown in T-050, but `docs/LLAMACPP.md` §1 lists it as `[assumed]`
    /// with no observed spelling. T-023 confirms it against the capture;
    /// until then it is a setting without a mechanism, and T-050 hides the
    /// control rather than offering one that does nothing. See
    /// `PROGRESS.md` D-003.
    pub idle_unload_seconds: Option<u32>,
    /// Never the key itself.
    pub api_key_set: bool,
    /// Default true; in-memory only.
    pub request_log_enabled: bool,
    /// Concurrent in-flight requests the listener will admit. `None` — the
    /// default — means the app imposes no limit of its own and lets the
    /// upstream decide, which is the transparent behaviour. When set,
    /// requests over the limit are refused immediately with 503 and
    /// `Retry-After`. **The app never queues**: holding a request behind
    /// others is a scheduling decision, and scheduling is not the app's job
    /// (`AGENTS.md` invariant 3).
    pub max_concurrent_requests: Option<u32>,
}

/// The listener's lifecycle is independent of `ServerState` — that
/// independence is the point of `PLAN.md` §2.7. The endpoint stays bound
/// while llama-server is stopped, so clients get a structured 503 rather
/// than a refused connection.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum EndpointState {
    Unbound,
    Bound { address: IpAddr, port: u16 },
    BindFailed { address: IpAddr, port: u16, error: AppError },
}

/// Never contains a body, a header value, or the model name. See `PLAN.md` §6.
#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct RequestLogEntry {
    pub at: DateTime<Utc>,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub duration_ms: u64,
    pub request_bytes: u64,
    pub response_bytes: u64,
    pub streamed: bool,
    pub authenticated: bool,
}

// ─── Server ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct Diagnosis {
    /// "cuda_dll_missing" | "oom_vram" | "port_in_use" | ...
    pub code: String,
    pub message: String,
    pub remediation: String,
}

/// Startup has two phases with very different durations: the coordinator
/// answering takes seconds, preloading a 65 GB model takes minutes.
/// Collapsing them into one opaque `Starting` gives the user a progress bar
/// that appears frozen. See `PLAN.md` §2.12.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum StartupPhase {
    /// Process spawned, coordinator not yet answering.
    WaitingForProcess,
    /// Coordinator is answering; models marked `preload` are being loaded.
    Preloading {
        done: u32,
        total: u32,
        current: Option<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub enum ServerState {
    Stopped,
    Starting {
        since: DateTime<Utc>,
        upstream_port: u16,
        phase: StartupPhase,
    },
    Running {
        pid: u32,
        upstream_port: u16,
        since: DateTime<Utc>,
        config_dirty: bool,
    },
    Stopping,
    Crashed {
        exit_code: Option<i32>,
        diagnosis: Option<Diagnosis>,
        last_log: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct ServerProps {
    pub build_tag: Option<String>,
    pub models_max: Option<u32>,
    /// `/props` is not a stable contract; keep the original.
    pub raw: serde_json::Value,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, TS)]
#[ts(export)]
pub enum ModelLoadState {
    Registered,
    Loading,
    Loaded,
    Unloading,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct LoadedModelState {
    pub model_id: String,
    pub state: ModelLoadState,
    pub vram_bytes: Option<u64>,
    pub last_used: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct TelemetrySnapshot {
    pub sampled_at: DateTime<Utc>,
    pub gpus: Vec<GpuTelemetry>,
    pub loaded_models: Vec<LoadedModelState>,
    /// Counted at the app's listener. Available because the app owns the
    /// endpoint — under direct binding these had no source.
    pub active_requests: u32,
    pub requests_last_minute: u32,
    /// Parsed from the server's own stderr, never from response bodies.
    pub tokens_per_sec: Option<f32>,
    pub uptime_seconds: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct GpuTelemetry {
    pub index: u32,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
    pub utilization_percent: u32,
    pub temperature_c: Option<u32>,
}

// ─── Settings ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, TS)]
#[ts(export)]
pub struct AppSettings {
    /// "dark" | "light" | "system"
    pub theme: String,
    pub launch_at_startup: bool,
    pub default_picker_folder: Option<PathBuf>,
    /// "off" | "notify" | "auto"
    pub auto_update_policy: String,
    pub log_retention_days: u32,
}

// ─── Enum round-trip tests (T-002 acceptance criterion) ──────────────────
//
// For each of the fourteen named enums: serialize a real value with
// `serde_json` and check the *actual* emitted shape — not the shape the
// `#[derive]` list merely suggests. Where a field's own wire format isn't
// the point of the test (chrono's `DateTime<Utc>`, `PathBuf`, `IpAddr`),
// the expected JSON is built by serializing that sub-value directly rather
// than hand-typing its string form, so the test doesn't silently assume a
// timestamp or path format it was never asked to verify.
//
// Each test also greps the *committed* `src/lib/types.ts` (whitespace-
// insensitive) for a fragment that could only be present if the generated
// type reflects the same shape. This is deliberately a looser check than
// the JSON assertion above it — its job is to catch the generated file
// drifting from the Rust definition, not to re-derive ts-rs's exact
// formatting by hand.
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn generated_types() -> String {
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../src/lib/types.ts"))
            .expect(
                "src/lib/types.ts must exist — run `npm run generate-types` before `cargo test`",
            )
    }

    fn normalize(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    fn assert_ts_contains(ts: &str, needle: &str) {
        let hay = normalize(ts);
        let n = normalize(needle);
        assert!(
            hay.contains(&n),
            "generated src/lib/types.ts is missing an expected fragment.\n\
             looked for (whitespace-insensitive): {needle}"
        );
    }

    fn sample_runtime_build() -> RuntimeBuild {
        RuntimeBuild {
            build_tag: "b9196".into(),
            backend: Backend::Cuda { major: 13 },
            install_path: PathBuf::from("C:/ProgramData/LlamaManager/runtimes/b9196"),
            is_active: true,
            installed_at: Utc.with_ymd_and_hms(2026, 8, 1, 9, 0, 0).unwrap(),
            verified_flags: vec![VerifiedFlag {
                name: "--flash-attn".into(),
                takes_value: true,
                allowed_values: Some(vec!["on".into(), "off".into(), "auto".into()]),
            }],
            registration_channel: RegistrationChannel::Undetermined,
        }
    }

    fn sample_model_entry() -> ModelEntry {
        ModelEntry {
            id: "abc123".into(),
            display_name: "Test Model".into(),
            served_name: "test-model".into(),
            file_path: PathBuf::from("D:/models/test-model.gguf"),
            shard_paths: vec![],
            size_bytes: 4_294_967_296,
            sha256_head: "deadbeef".into(),
            metadata: GgufMetadata {
                architecture: "llama".into(),
                param_count: Some(7_000_000_000),
                quantization: "Q4_K_M".into(),
                block_count: 32,
                context_length: Some(8192),
                embedding_length: Some(4096),
                attention_head_count: Some(32),
                attention_head_count_kv: Some(8),
                has_chat_template: true,
                is_moe: false,
                expert_count: None,
            },
            compatibility: Compatibility::Supported,
            availability: ModelAvailability::Present,
            duplicate_of: None,
            launch_params: LaunchParams::default(),
            sampling_defaults: SamplingDefaults::default(),
            preload: false,
            pinned: false,
            added_at: Utc.with_ymd_and_hms(2026, 8, 1, 9, 0, 0).unwrap(),
        }
    }

    #[test]
    fn flash_attn_round_trips() {
        assert_eq!(serde_json::to_value(FlashAttn::On).unwrap(), json!("On"));
        assert_eq!(serde_json::to_value(FlashAttn::Off).unwrap(), json!("Off"));
        assert_eq!(serde_json::to_value(FlashAttn::Auto).unwrap(), json!("Auto"));
        assert_eq!(
            serde_json::from_value::<FlashAttn>(json!("Auto")).unwrap(),
            FlashAttn::Auto
        );

        let ts = generated_types();
        assert_ts_contains(&ts, r#""On""#);
        assert_ts_contains(&ts, r#""Off""#);
        assert_ts_contains(&ts, r#""Auto""#);
    }

    #[test]
    fn compatibility_round_trips() {
        assert_eq!(
            serde_json::to_value(Compatibility::Supported).unwrap(),
            json!("Supported")
        );

        let warn = Compatibility::SupportedWithWarnings(vec![CompatibilityNote {
            text: "NVFP4 is experimental".into(),
            experimental: true,
        }]);
        assert_eq!(
            serde_json::to_value(&warn).unwrap(),
            json!({"SupportedWithWarnings": [{"text": "NVFP4 is experimental", "experimental": true}]})
        );

        let unsupported = Compatibility::Unsupported("architecture not recognized".into());
        assert_eq!(
            serde_json::to_value(&unsupported).unwrap(),
            json!({"Unsupported": "architecture not recognized"})
        );

        let ts = generated_types();
        assert_ts_contains(&ts, "SupportedWithWarnings");
        assert_ts_contains(&ts, "Unsupported");
    }

    #[test]
    fn server_state_round_trips() {
        assert_eq!(
            serde_json::to_value(ServerState::Stopped).unwrap(),
            json!("Stopped")
        );
        assert_eq!(
            serde_json::to_value(ServerState::Stopping).unwrap(),
            json!("Stopping")
        );

        let since = Utc.with_ymd_and_hms(2026, 8, 5, 12, 0, 0).unwrap();
        let since_json = serde_json::to_value(since).unwrap();

        let starting = ServerState::Starting {
            since,
            upstream_port: 49500,
            phase: StartupPhase::WaitingForProcess,
        };
        assert_eq!(
            serde_json::to_value(&starting).unwrap(),
            json!({"Starting": {"since": since_json, "upstream_port": 49500, "phase": "WaitingForProcess"}})
        );

        let running = ServerState::Running {
            pid: 4242,
            upstream_port: 49500,
            since,
            config_dirty: true,
        };
        assert_eq!(
            serde_json::to_value(&running).unwrap(),
            json!({"Running": {"pid": 4242, "upstream_port": 49500, "since": since_json, "config_dirty": true}})
        );

        let crashed = ServerState::Crashed {
            exit_code: Some(-1),
            diagnosis: None,
            last_log: vec!["line one".into()],
        };
        assert_eq!(
            serde_json::to_value(&crashed).unwrap(),
            json!({"Crashed": {"exit_code": -1, "diagnosis": null, "last_log": ["line one"]}})
        );

        let ts = generated_types();
        assert_ts_contains(&ts, "Starting");
        assert_ts_contains(&ts, "Running");
        assert_ts_contains(&ts, "Crashed");
    }

    #[test]
    fn endpoint_state_round_trips() {
        assert_eq!(
            serde_json::to_value(EndpointState::Unbound).unwrap(),
            json!("Unbound")
        );

        let addr: IpAddr = "127.0.0.1".parse().unwrap();
        let addr_json = serde_json::to_value(addr).unwrap();

        let bound = EndpointState::Bound { address: addr, port: 8080 };
        assert_eq!(
            serde_json::to_value(&bound).unwrap(),
            json!({"Bound": {"address": addr_json, "port": 8080}})
        );

        let err = AppError::PortInUse { port: 8080 };
        let err_json = serde_json::to_value(&err).unwrap();
        assert_eq!(err_json, json!({"kind": "PortInUse", "detail": {"port": 8080}}));

        let bind_failed = EndpointState::BindFailed {
            address: addr,
            port: 8080,
            error: err,
        };
        assert_eq!(
            serde_json::to_value(&bind_failed).unwrap(),
            json!({"BindFailed": {"address": addr_json, "port": 8080, "error": err_json}})
        );

        // Round trip, not just one-way serialize.
        let back: EndpointState = serde_json::from_value(serde_json::to_value(&bound).unwrap()).unwrap();
        assert_eq!(back, bound);

        let ts = generated_types();
        assert_ts_contains(&ts, "Unbound");
        assert_ts_contains(&ts, "Bound");
        assert_ts_contains(&ts, "BindFailed");
    }

    #[test]
    fn install_progress_round_trips() {
        assert_eq!(
            serde_json::to_value(InstallProgress::Resolving).unwrap(),
            json!("Resolving")
        );
        assert_eq!(
            serde_json::to_value(InstallProgress::Verifying).unwrap(),
            json!("Verifying")
        );
        assert_eq!(
            serde_json::to_value(InstallProgress::Registering).unwrap(),
            json!("Registering")
        );

        let downloading = InstallProgress::Downloading {
            received_bytes: 1024,
            total_bytes: 4096,
            bytes_per_sec: 512,
        };
        assert_eq!(
            serde_json::to_value(&downloading).unwrap(),
            json!({"Downloading": {"received_bytes": 1024, "total_bytes": 4096, "bytes_per_sec": 512}})
        );

        let build = sample_runtime_build();
        let build_json = serde_json::to_value(&build).unwrap();
        let done = InstallProgress::Done { build: build.clone() };
        assert_eq!(serde_json::to_value(&done).unwrap(), json!({"Done": {"build": build_json}}));

        let err = AppError::ChecksumMismatch {
            expected: "aaa".into(),
            actual: "bbb".into(),
        };
        let err_json = serde_json::to_value(&err).unwrap();
        let failed = InstallProgress::Failed { error: err };
        assert_eq!(serde_json::to_value(&failed).unwrap(), json!({"Failed": {"error": err_json}}));

        let ts = generated_types();
        assert_ts_contains(&ts, "Downloading");
        assert_ts_contains(&ts, "Done");
        assert_ts_contains(&ts, "Failed");
    }

    #[test]
    fn import_progress_round_trips() {
        let queued = ImportProgress::Queued {
            job_id: "job-1".into(),
            total_files: 3,
        };
        assert_eq!(
            serde_json::to_value(&queued).unwrap(),
            json!({"Queued": {"job_id": "job-1", "total_files": 3}})
        );

        let entry = sample_model_entry();
        let entry_json = serde_json::to_value(&entry).unwrap();
        let file_done = ImportProgress::FileDone {
            job_id: "job-1".into(),
            entry: entry.clone(),
            index: 0,
            total: 3,
        };
        assert_eq!(
            serde_json::to_value(&file_done).unwrap(),
            json!({"FileDone": {"job_id": "job-1", "entry": entry_json, "index": 0, "total": 3}})
        );

        let finished = ImportProgress::Finished {
            job_id: "job-1".into(),
            imported: 2,
            skipped: 1,
            failed: 0,
        };
        assert_eq!(
            serde_json::to_value(&finished).unwrap(),
            json!({"Finished": {"job_id": "job-1", "imported": 2, "skipped": 1, "failed": 0}})
        );

        let ts = generated_types();
        assert_ts_contains(&ts, "Queued");
        assert_ts_contains(&ts, "FileDone");
        assert_ts_contains(&ts, "Finished");
    }

    #[test]
    fn model_availability_round_trips() {
        assert_eq!(
            serde_json::to_value(ModelAvailability::Present).unwrap(),
            json!("Present")
        );
        assert_eq!(
            serde_json::to_value(ModelAvailability::Missing).unwrap(),
            json!("Missing")
        );
        assert_eq!(
            serde_json::to_value(ModelAvailability::Unreadable).unwrap(),
            json!("Unreadable")
        );

        let ts = generated_types();
        assert_ts_contains(&ts, r#""Present""#);
        assert_ts_contains(&ts, r#""Missing""#);
        assert_ts_contains(&ts, r#""Unreadable""#);
    }

    #[test]
    fn link_capability_round_trips() {
        assert_eq!(
            serde_json::to_value(LinkCapability::Symlink).unwrap(),
            json!("Symlink")
        );
        assert_eq!(
            serde_json::to_value(LinkCapability::HardLinkOnly).unwrap(),
            json!("HardLinkOnly")
        );
        // `LinkCapability::None` — a variant literally named `None`, distinct
        // from `Option::None`. Confirms the derive doesn't do anything odd
        // with the name collision.
        assert_eq!(
            serde_json::to_value(LinkCapability::None).unwrap(),
            json!("None")
        );

        let ts = generated_types();
        assert_ts_contains(&ts, "Symlink");
        assert_ts_contains(&ts, "HardLinkOnly");
    }

    #[test]
    fn backend_round_trips() {
        assert_eq!(serde_json::to_value(Backend::Vulkan).unwrap(), json!({"kind": "Vulkan"}));
        assert_eq!(serde_json::to_value(Backend::Cpu).unwrap(), json!({"kind": "Cpu"}));

        // The point of this case: `major` the code has no branch for today
        // (no real GPU reports major 200) still round-trips as plain data,
        // because `Backend::Cuda` is `{ major: u8 }`, not a closed enum of
        // known majors (PLAN.md §2.13: "Backend is open").
        let unknown_major = Backend::Cuda { major: 200 };
        assert_eq!(
            serde_json::to_value(&unknown_major).unwrap(),
            json!({"kind": "Cuda", "major": 200})
        );
        let back: Backend = serde_json::from_value(json!({"kind": "Cuda", "major": 200})).unwrap();
        assert_eq!(back, unknown_major);

        let ts = generated_types();
        assert_ts_contains(&ts, "Cuda");
        assert_ts_contains(&ts, "Vulkan");
        assert_ts_contains(&ts, "Cpu");
    }

    #[test]
    fn registration_channel_round_trips() {
        assert_eq!(
            serde_json::to_value(RegistrationChannel::PresetDeclaresPath).unwrap(),
            json!("PresetDeclaresPath")
        );
        assert_eq!(
            serde_json::to_value(RegistrationChannel::ScanOnly).unwrap(),
            json!("ScanOnly")
        );
        assert_eq!(
            serde_json::to_value(RegistrationChannel::Undetermined).unwrap(),
            json!("Undetermined")
        );

        let ts = generated_types();
        assert_ts_contains(&ts, "PresetDeclaresPath");
        assert_ts_contains(&ts, "ScanOnly");
        assert_ts_contains(&ts, "Undetermined");
    }

    #[test]
    fn check_status_round_trips() {
        assert_eq!(serde_json::to_value(CheckStatus::Pass).unwrap(), json!("Pass"));
        assert_eq!(serde_json::to_value(CheckStatus::Warn).unwrap(), json!("Warn"));
        assert_eq!(serde_json::to_value(CheckStatus::Fail).unwrap(), json!("Fail"));

        let ts = generated_types();
        assert_ts_contains(&ts, r#""Pass""#);
        assert_ts_contains(&ts, r#""Warn""#);
        assert_ts_contains(&ts, r#""Fail""#);
    }

    #[test]
    fn model_load_state_round_trips() {
        for (variant, name) in [
            (ModelLoadState::Registered, "Registered"),
            (ModelLoadState::Loading, "Loading"),
            (ModelLoadState::Loaded, "Loaded"),
            (ModelLoadState::Unloading, "Unloading"),
            (ModelLoadState::Failed, "Failed"),
        ] {
            assert_eq!(serde_json::to_value(&variant).unwrap(), json!(name));
        }

        let ts = generated_types();
        assert_ts_contains(&ts, "Registered");
        assert_ts_contains(&ts, "Unloading");
    }

    #[test]
    fn estimate_confidence_round_trips() {
        assert_eq!(
            serde_json::to_value(EstimateConfidence::Calibrated).unwrap(),
            json!("Calibrated")
        );
        assert_eq!(
            serde_json::to_value(EstimateConfidence::Heuristic).unwrap(),
            json!("Heuristic")
        );

        let ts = generated_types();
        assert_ts_contains(&ts, "Calibrated");
        assert_ts_contains(&ts, "Heuristic");
    }

    #[test]
    fn app_error_round_trips() {
        // Unit variant under adjacent tagging (`tag = "kind", content =
        // "detail"`): no `detail` key at all when there's no content.
        assert_eq!(
            serde_json::to_value(AppError::Unauthorized).unwrap(),
            json!({"kind": "Unauthorized"})
        );

        // Struct variant with one field.
        assert_eq!(
            serde_json::to_value(AppError::PortInUse { port: 8080 }).unwrap(),
            json!({"kind": "PortInUse", "detail": {"port": 8080}})
        );

        // Struct variant with two fields, to confirm `detail` wraps every
        // field rather than only the first.
        let invalid = AppError::InvalidTransition {
            from: "Stopped".into(),
            command: "stop_server".into(),
        };
        assert_eq!(
            serde_json::to_value(&invalid).unwrap(),
            json!({"kind": "InvalidTransition", "detail": {"from": "Stopped", "command": "stop_server"}})
        );

        let back: AppError = serde_json::from_value(serde_json::to_value(&invalid).unwrap()).unwrap();
        assert_eq!(back, invalid);

        let ts = generated_types();
        assert_ts_contains(&ts, "Unauthorized");
        assert_ts_contains(&ts, "PortInUse");
        assert_ts_contains(&ts, "InvalidTransition");
    }
}
