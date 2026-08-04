# Contracts — types, state machine, schema, IPC

Defined once in Rust with `serde`; TypeScript definitions are generated (T-002). Never hand-write a duplicate.

---

## 1. Types

```rust
// core/src/types.rs

// ─── Errors ─────────────────────────────────────────────────────

/// Defined in full in T-004. Every IPC command returns Result<T, AppError>.
#[derive(Serialize, Deserialize, Clone, Debug, thiserror::Error)]
#[serde(tag = "kind", content = "detail")]
pub enum AppError {
    InvalidTransition { from: String, command: String },
    NotFound { what: String },
    Io { message: String },
    Database { message: String },
    Network { message: String },
    RateLimited { retry_after_seconds: u64 },
    ChecksumMismatch { expected: String, actual: String },
    UnsafeArchiveEntry { entry: String },
    InvalidPath { path: String, reason: String },
    GgufParse { message: String },
    PortInUse { port: u16 },
    UpstreamUnavailable { state: String },
    Unauthorized,
    SchemaTooNew { found: u32, known: u32 },
    Internal { message: String },
}

impl AppError {
    pub fn code(&self) -> &'static str;          // stable, machine-readable
    pub fn message(&self) -> String;             // user-facing, never empty
    pub fn remediation(&self) -> Option<String>;
}

// ─── Environment ────────────────────────────────────────────────

/// Open on purpose: a CUDA major the binary has never heard of must be
/// representable, not a parse failure. See PLAN.md §2.13.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind")]
pub enum Backend {
    Cuda { major: u8 },
    Vulkan,
    Cpu,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum CheckStatus { Pass, Warn, Fail }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GpuInfo {
    pub index: u32,
    pub name: String,
    pub compute_capability: (u32, u32),   // Blackwell = (12, 0) or (10, x)
    pub vram_total_bytes: u64,
    pub vram_free_bytes: u64,
    pub driver_version: String,
    pub cuda_version: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct HealthCheck {
    pub id: String,               // "gpu_present" | "driver_ok" | "cuda13_ok"
                                  // | "disk_space" | "endpoint_bindable"
    pub status: CheckStatus,
    pub message: String,          // user-facing, actionable
    pub remediation: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EnvironmentReport {
    pub gpus: Vec<GpuInfo>,
    pub system_ram_bytes: u64,
    pub free_disk_bytes: u64,
    pub os_build: String,
    pub checks: Vec<HealthCheck>,
}

// ─── Runtime ────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VerifiedFlag {
    pub name: String,                        // "--flash-attn"
    pub takes_value: bool,
    pub allowed_values: Option<Vec<String>>, // when the help text enumerates them
}

/// How this build lets a model enter the router's registry.
/// Settled by parsing --help in T-023; see PLAN.md §2.1.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum RegistrationChannel {
    /// --models-preset entries may declare a model by absolute path.
    PresetDeclaresPath,
    /// Only --models-dir scanning registers models; the app links into a
    /// directory it owns.
    ScanOnly,
    /// --help did not answer the question. T-033 must not proceed on a guess.
    Undetermined,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RuntimeBuild {
    pub build_tag: String,        // "b9196"
    pub backend: Backend,
    pub install_path: PathBuf,
    pub is_active: bool,
    pub installed_at: DateTime<Utc>,
    pub verified_flags: Vec<VerifiedFlag>,
    pub registration_channel: RegistrationChannel,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AvailableRelease {
    pub build_tag: String,
    pub backend: Backend,
    pub asset_name: String,
    pub asset_url: String,
    pub sha256: Option<String>,   // None when the release omits checksums
    pub size_bytes: u64,
    pub published_at: DateTime<Utc>,
    pub release_notes_url: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum InstallProgress {
    Resolving,
    Downloading { received_bytes: u64, total_bytes: u64, bytes_per_sec: u64 },
    Verifying,
    Extracting { entries_done: u32, entries_total: u32 },
    Registering,
    Done { build: RuntimeBuild },
    Failed { error: AppError },
}

// ─── Models ─────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GgufMetadata {
    pub architecture: String,     // "llama" | "qwen3" | "deepseek2" | ...
    pub param_count: Option<u64>,
    pub quantization: String,     // "Q4_K_M" | "MXFP4" | "NVFP4" | "F16" | ...
    pub block_count: u32,         // layer count — drives -ngl
    pub context_length: Option<u32>,
    pub embedding_length: Option<u32>,
    /// Both are required by the KV cache term of the estimator. With Grouped
    /// Query Attention the ratio between them reaches 8:1, so ignoring
    /// `attention_head_count_kv` misestimates the cache by nearly an order of
    /// magnitude. When either is absent the estimator says so in its notes
    /// rather than assuming they are equal.
    pub attention_head_count: Option<u32>,
    pub attention_head_count_kv: Option<u32>,
    pub has_chat_template: bool,
    pub is_moe: bool,
    pub expert_count: Option<u32>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CompatibilityNote {
    pub text: String,
    pub experimental: bool,       // NVFP4 sets this; see PLAN.md §2.6
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Compatibility {
    Supported,
    SupportedWithWarnings(Vec<CompatibilityNote>),
    Unsupported(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ModelAvailability {
    Present,
    Missing,          // path no longer resolves: file renamed, drive disconnected
    Unreadable,       // path resolves but the file cannot be opened
}

/// What linking primitive this machine can actually use, established by
/// attempting one in a scratch directory rather than by reading a privilege.
/// The three are not interchangeable on Windows: a junction cannot link an
/// individual file, and a hard link cannot cross volumes. See PLAN.md §2.1.
/// Only reached under the ScanOnly registration channel.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum LinkCapability {
    /// File symlinks work: exact, per-file, any volume. The preferred fallback.
    Symlink,
    /// No symlink privilege; hard links work for targets on the app's volume.
    HardLinkOnly,
    /// Neither. Under ScanOnly this machine cannot register a model at all,
    /// and the app says so plainly rather than linking a whole folder and hoping.
    None,
}

/// Tri-state. Recent builds accept on|off|auto rather than a boolean.
/// VERIFY the type against docs/verified-flags.md before use.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum FlashAttn { On, Off, Auto }

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
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
    /// See AGENTS.md §1.
    pub extra_args: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WatchedFolder {
    pub path: PathBuf,
    pub model_count: u32,
    pub reachable: bool,
    pub last_scan_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ModelEntry {
    pub id: String,               // stable slug derived from the absolute path
    pub display_name: String,
    pub served_name: String,      // what /v1/models exposes; unique across the catalogue
    pub file_path: PathBuf,       // absolute, wherever the user keeps it — never moved
    pub shard_paths: Vec<PathBuf>,
    pub size_bytes: u64,
    pub sha256_head: String,      // hash of first 1 MiB + size — duplicate *signal*, not identity
    pub metadata: GgufMetadata,
    pub compatibility: Compatibility,
    pub availability: ModelAvailability,
    pub duplicate_of: Option<String>,  // another entry with the same sha256_head
    pub launch_params: LaunchParams,
    pub sampling_defaults: SamplingDefaults,
    pub preload: bool,            // loaded at server start rather than on demand
    pub pinned: bool,             // exempt from LRU eviction
    pub added_at: DateTime<Utc>,
}

/// Survives removal of the catalogue entry, keyed by the model's absolute path.
/// Re-importing the same file restores it. See PLAN.md §2.9.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RetainedModelSettings {
    pub file_path: PathBuf,
    pub launch_params: LaunchParams,
    pub sampling_defaults: SamplingDefaults,
    pub preload: bool,
    pub pinned: bool,
    pub retained_at: DateTime<Utc>,
}

pub type ImportJobId = String;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ImportProgress {
    Queued { job_id: ImportJobId, total_files: u32 },
    FileStarted { job_id: ImportJobId, path: PathBuf, index: u32, total: u32 },
    FileDone { job_id: ImportJobId, entry: ModelEntry, index: u32, total: u32 },
    FileFailed { job_id: ImportJobId, path: PathBuf, error: AppError },
    Cancelled { job_id: ImportJobId, completed: u32 },
    Finished { job_id: ImportJobId, imported: u32, skipped: u32, failed: u32 },
}

// ─── Estimation ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EstimateConfidence { Calibrated, Heuristic }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LaunchRecord {
    /// Absolute path, not the entry id: history outlives the catalogue entry.
    pub file_path: PathBuf,
    pub launched_at: DateTime<Utc>,
    pub params: LaunchParams,
    pub succeeded: bool,
    pub actual_vram_bytes: Option<u64>,
    pub load_seconds: Option<f64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EstimateInputs {
    pub metadata: GgufMetadata,
    pub file_size_bytes: u64,
    pub params: LaunchParams,
    pub vram_free_bytes: u64,
    pub ram_free_bytes: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct VramEstimate {
    pub recommended_gpu_layers: u32,
    pub estimated_vram_bytes: u64,
    pub estimated_ram_bytes: u64,
    pub kv_cache_bytes: u64,
    pub fits_fully: bool,
    pub confidence: EstimateConfidence,
    pub notes: Vec<String>,
}

/// The estimator is pure. History is an argument, never a database read.
/// See AGENTS.md invariant 5.
///
/// Calibration contract: `history` is pre-filtered by the caller to records
/// matching this model's absolute path. A record *matches* when it succeeded, carries
/// `actual_vram_bytes`, and its `gpu_layers`, `ctx_size` and cache types equal
/// those in `inputs.params`. One or more matches ⇒ `Calibrated`; the most recent
/// three are averaged. Zero matches ⇒ `Heuristic`, and non-matching records
/// still shift the estimate via a bias term.
pub fn estimate(inputs: &EstimateInputs, history: &[LaunchRecord]) -> VramEstimate;
```

### The estimation model

Written down because the structural properties T-032 asserts — determinism, monotonicity, bounds — are satisfied by many different functions. Without a stated model, an empirical error of 40% cannot be attributed: the model may be wrong, or the implementation may not be following it. The `insta` snapshots in T-032 freeze this model, so changing a coefficient shows up as a reviewable diff rather than as drift.

Coefficients are a starting point, not truth. They are the owner's to correct from `docs/owner-verification.md`, and correcting them is expected.

**Terms.**

```
head_dim        = embedding_length / attention_head_count

weights_gpu     = file_size_bytes × (gpu_layers / block_count)
weights_cpu     = file_size_bytes − weights_gpu

kv_bytes        = 2                         (one K, one V)
                × ctx_size
                × min(gpu_layers, block_count)
                × attention_head_count_kv
                × head_dim
                × bytes_per_element(cache_type)

compute_buffer  = ubatch_size × embedding_length × 4 bytes × C_compute
context_overhead = C_context                (CUDA context, allocator, cuBLAS workspaces)

estimated_vram  = (weights_gpu + kv_bytes + compute_buffer + context_overhead) × (1 + margin)
estimated_ram   = weights_cpu + C_host
```

`bytes_per_element` is 2 for `f16`, 1 for `q8_0`, 0.5625 for `q4_0` — quantizing the cache is the single largest lever on `kv_bytes` at long context, which is why T-032 asserts the direction of that effect rather than only its determinism.

**Constants.** `C_compute = 2.0`, `C_context = 512 MiB`, `C_host = 256 MiB`, `margin = 0.12`.

**The margin is not symmetric and is stated deliberately.** Overestimating costs performance — the user offloads fewer layers than they could. Underestimating costs an out-of-memory failure partway through loading a 65 GB file, minutes in, with nothing to show for the wait. 12% biases toward the recoverable failure, and `VramEstimate.notes` says the figure is conservative so the user is not left wondering why the reported number exceeds what they observe.

**Missing metadata degrades, it does not guess.** When `attention_head_count_kv` is absent the estimator does **not** assume it equals `attention_head_count` — with Grouped Query Attention that is wrong by up to 8×, in the dangerous direction. It falls back to a per-architecture default where one is known, notes which value was assumed, and widens the margin to 25% for that estimate.

**The other two fields the model divides by degrade the same way.** `head_dim` needs `embedding_length` and `attention_head_count`, and both are `Option`:

- Either absent, with a per-architecture default known ⇒ apply it, name it in `notes`, widen the margin to 25%.
- Either absent with no default known ⇒ `kv_bytes` cannot be computed. Do **not** substitute zero: a KV term of zero underestimates in the dangerous direction at long context. Return an estimate whose `kv_cache_bytes` is the whole-file upper bound implied by `ctx_size` and `block_count` alone, mark `confidence: Heuristic`, widen the margin to 25%, and say in `notes` that the cache term is bounded rather than modelled.

**What produces `recommended_gpu_layers`.** The estimator is asked two questions at once and they must not be confused. `inputs.params.gpu_layers` is the configuration being *evaluated* — `estimated_vram` is what that configuration would cost, which is why T-032 asserts monotonicity in it. `recommended_gpu_layers` is a separate answer, computed by the same formula run over candidate layer counts:

- Take the largest `n` in `0..=block_count` for which `estimated_vram(n) ≤ vram_free_bytes`, evaluated with the margin already applied. The search is a scan, not a solve: `block_count` is at most a few hundred and the function is cheap and monotone, so a bisection would buy nothing and cost a correctness argument.
- When `inputs.params.gpu_layers` is `None` — the user has not chosen — `estimated_vram` is reported for the recommended `n` rather than for an arbitrary one, and `notes` says which value it describes.
- `recommended_gpu_layers` never exceeds `block_count`, and is `0` when no `n` fits.

**`estimated_ram` omits the KV cache of CPU-resident layers, deliberately and probably wrongly.** `kv_bytes` counts `min(gpu_layers, block_count)` layers and is charged entirely to VRAM; the layers running on CPU carry a cache too, and this model does not bill it. It is left out because its size on the host side is the term the owner is least able to isolate from a single measurement, and adding an unvalidated term would make the whole host figure harder to attribute. **This is the first thing to revisit if `docs/owner-verification.md` reports host RAM consistently under-predicted for partial-offload configurations.**

**Calibration.** With matching history, `estimated_vram` is replaced by the mean of the most recent three matching `actual_vram_bytes`, still with the margin applied, and confidence becomes `Calibrated`. With history for the same model but different parameters, the ratio between predicted and actual on those records is applied as a multiplicative bias to the heuristic, and confidence stays `Heuristic` — a measurement of a different configuration is evidence, not a substitute.

```rust

// ─── Endpoint (PLAN.md §2.7) ────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerConfig {
    /// The address clients configure. Owned by the app, stable across restarts.
    pub listen_address: IpAddr,          // default 127.0.0.1
    pub listen_port: u16,                // default 8080; never auto-incremented
    /// Range the app draws the upstream loopback port from. Internal.
    pub upstream_port_range: (u16, u16), // default (49500, 49999)
    /// How long a request is held while the coordinator is not yet answering.
    pub startup_hold_seconds: u32,       // default 120
    /// How long the coordinator itself may take to answer before Crashed.
    pub process_timeout_seconds: u32,    // default 120
    /// How long preloading may take before Crashed. Separate from the above
    /// because a 65 GB preload is minutes, not seconds, and one timeout cannot
    /// serve both without being uselessly loose for the first.
    pub preload_timeout_seconds: u32,    // default 900
    pub models_max: u32,                 // --models-max; 1 = strict hot-swap
    pub autoload: bool,                  // --models-autoload
    /// **No flag is known to implement this.** It is modelled here and shown in
    /// T-050, but `docs/LLAMACPP.md` §1 lists it as [assumed] with no observed
    /// spelling. T-023 confirms it against the capture; until then it is a
    /// setting without a mechanism, and T-050 hides the control rather than
    /// offering one that does nothing. See PROGRESS.md D-003.
    pub idle_unload_seconds: Option<u32>,
    pub api_key_set: bool,               // never the key itself
    pub request_log_enabled: bool,       // default true; in-memory only
    /// Concurrent in-flight requests the listener will admit. `None` — the
    /// default — means the app imposes no limit of its own and lets the
    /// upstream decide, which is the transparent behaviour. When set, requests
    /// over the limit are refused immediately with 503 and `Retry-After`.
    /// **The app never queues**: holding a request behind others is a
    /// scheduling decision, and scheduling is not the app's job (AGENTS.md
    /// invariant 3).
    pub max_concurrent_requests: Option<u32>,
}

/// The listener's lifecycle is independent of ServerState — that independence
/// is the point of PLAN.md §2.7. The endpoint stays bound while llama-server
/// is stopped, so clients get a structured 503 rather than a refused connection.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum EndpointState {
    Unbound,
    Bound { address: IpAddr, port: u16 },
    BindFailed { address: IpAddr, port: u16, error: AppError },
}

/// Never contains a body, a header value, or the model name.
/// See PLAN.md §6.
#[derive(Serialize, Deserialize, Clone, Debug)]
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Diagnosis {
    pub code: String,             // "cuda_dll_missing" | "oom_vram" | "port_in_use" | ...
    pub message: String,
    pub remediation: String,
}

/// Startup has two phases with very different durations: the coordinator
/// answering takes seconds, preloading a 65 GB model takes minutes. Collapsing
/// them into one opaque `Starting` gives the user a progress bar that appears
/// frozen. See PLAN.md §2.12.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum StartupPhase {
    /// Process spawned, coordinator not yet answering.
    WaitingForProcess,
    /// Coordinator is answering; models marked `preload` are being loaded.
    Preloading { done: u32, total: u32, current: Option<String> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ServerState {
    Stopped,
    Starting { since: DateTime<Utc>, upstream_port: u16, phase: StartupPhase },
    Running { pid: u32, upstream_port: u16, since: DateTime<Utc>, config_dirty: bool },
    Stopping,
    Crashed { exit_code: Option<i32>, diagnosis: Option<Diagnosis>, last_log: Vec<String> },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerProps {
    pub build_tag: Option<String>,
    pub models_max: Option<u32>,
    pub raw: serde_json::Value,   // /props is not a stable contract; keep the original
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum ModelLoadState { Registered, Loading, Loaded, Unloading, Failed }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LoadedModelState {
    pub model_id: String,
    pub state: ModelLoadState,
    pub vram_bytes: Option<u64>,
    pub last_used: Option<DateTime<Utc>>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
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

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct GpuTelemetry {
    pub index: u32,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
    pub utilization_percent: u32,
    pub temperature_c: Option<u32>,
}

// ─── Settings ───────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AppSettings {
    pub theme: String,                    // "dark" | "light" | "system"
    pub launch_at_startup: bool,
    pub default_picker_folder: Option<PathBuf>,
    pub auto_update_policy: String,       // "off" | "notify" | "auto"
    pub log_retention_days: u32,
}
```

---

## 2. Server state machine

`ServerState` is owned by a single `Supervisor` actor task. IPC commands send messages over an `mpsc` channel and await a reply. Changes are broadcast as `server-state-changed` events. **No shared `Mutex<ServerState>`** — this is the only permitted design for this component.

### Legal transitions

| From | Trigger | To | Notes |
|---|---|---|---|
| Stopped | `start_server` | Starting `{ WaitingForProcess }` | regenerates `presets.ini`; allocates an upstream port |
| Starting `{ WaitingForProcess }` | coordinator answers the health endpoint | Starting `{ Preloading }` | skipped straight to Running when nothing is marked `preload` |
| Starting `{ Preloading }` | every `preload` model reports `Loaded` | Running | |
| Starting `{ WaitingForProcess }` | process exits, or `process_timeout_seconds` | Crashed | |
| Starting `{ Preloading }` | a preload fails, or `preload_timeout_seconds` | Crashed | the diagnosis names the model that failed |
| Starting | `stop_server` | Stopping | must kill the partially started process |
| Running | `stop_server` | Stopping | |
| Running | process exits unexpectedly | Crashed | |
| Stopping | process exited | Stopped | |
| Stopping | 10 s grace timeout | Stopped | forced kill of the whole process tree |
| Crashed | `start_server` | Starting | |
| Crashed | `dismiss_crash` | Stopped | |

### Rejected commands

Return `AppError::InvalidTransition { from, command }` and leave state unchanged:

- `start_server` while Starting, Running, or Stopping
- `stop_server` while Stopped or Stopping
- `activate_runtime`, `remove_runtime` while state is not Stopped
- `remove_model` for a model whose `ModelLoadState` is Loading or Loaded
- `set_server_config` changing `listen_address` or `listen_port` while `EndpointState` is `Bound` and the server is not Stopped

### What "the health check passes" means

The transition out of `Starting` was the only one in this table without a definition, which meant a test could assert it against a stub that decided for itself when to answer.

- **Endpoint.** The coordinator's health endpoint for the active build, resolved in T-023 and recorded with the verified flag list. `/health` where it exists; otherwise `/props`, whose success is equally a proof of life. The app does not invent one.
- **Ready means preloaded.** `Running` is reached only once every model marked `preload` reports `Loaded`. A server that answers while a 65 GB preload is still in flight is not yet doing the job it was configured to do, and reporting it as Running would make the dashboard lie in the one situation where the user is watching it.
- **Availability does not wait for it.** While `Preloading`, the coordinator answers, so the endpoint **forwards** rather than holding. The app is never less available than the server it manages; `Running` is a statement about configuration being fully applied, not a gate on traffic.
- **Poll interval.** 500 ms while `WaitingForProcess`, 2 s while `Preloading`. Neither is a hot loop, and the first is fast enough that start feedback stays inside the 2-second accuracy the Definition of Done asks for.
- **A single failed probe is not a death.** Three consecutive failures, or a probe that exceeds its own 5-second timeout three times, count as a failure. One dropped response is a dropped response.

### The endpoint is a separate lifecycle

`EndpointState` is **not** part of `ServerState` and does not transition with it. The listener binds when the app starts and stays bound. It is owned by the same actor, so the two are consistent, but they are reported and reasoned about separately.

How the endpoint answers, per `ServerState`:

| `ServerState` | Response to a forwarded request |
|---|---|
| Running | forwarded to the upstream, unmodified |
| Starting `{ WaitingForProcess }` | held up to `startup_hold_seconds`, then forwarded if the coordinator is answering, else `503` |
| Starting `{ Preloading }` | forwarded — the coordinator is already answering |
| Stopped, Stopping | `503` immediately, body naming the state |
| Crashed | `503` immediately, body carrying the `Diagnosis` code and message |

A `503` body is JSON in OpenAI error shape so that standard clients surface it as a server error rather than a parse failure.

### Configuration changes while running

Editing model parameters, preload flags, or router settings while Running **does not** restart the server and **does not** rewrite the live `presets.ini`. The edit is persisted and `Running.config_dirty` becomes `true`; the UI shows a persistent restart banner. Restart regenerates the preset from the database. No hot-reload in v1.

Endpoint settings behave differently, because they are the app's own: `request_log_enabled` and `startup_hold_seconds` apply immediately. `listen_address` and `listen_port` require the server to be Stopped, and are rejected otherwise — moving the address clients are using is never done implicitly.

---

## 3. SQLite schema — `0001_init.sql`

`PRAGMA foreign_keys = ON` is set on every connection. It is off by default in SQLite, so the `REFERENCES` clause below is decorative without it.

```sql
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
```

**Migrations are forward-only.** On startup, if `schema_version.version` exceeds the version the binary knows, the app **refuses to start** with `AppError::SchemaTooNew` — it never migrates backwards and never crashes on an unknown schema.

**Removal is not amnesia.** `launch_history` and `retained_model_settings` are keyed by absolute path and have no foreign key to `models`, so removing an entry keeps its tuning and its calibration. Re-importing the same file restores both.

This is deliberate and it is the reason per-model enable/disable was dropped (`PLAN.md` §2.9): removing a model is now cheap enough to be the same gesture. If the two tables were cascaded away with the entry, "remove and re-add" would silently discard half an hour of tuning on a 65 GB model and reset the estimator to `Heuristic`.

The risk this trades against is a path being reused by a *different* file. `sha256_head` is recorded with the retained settings and compared on restore; a mismatch discards them rather than applying one model's tuning to another.

**`launch_history` is functional, not bookkeeping.** T-041 writes a record on every load attempt; T-032 reads them (as an argument) to calibrate. Without the writer the calibration path is dead code.

**The request log is not in this schema.** It is an in-memory ring buffer, discarded on exit (`PLAN.md` §6, §8).

---

## 4. IPC surface

Every command returns `Result<T, AppError>`. Typed wrappers in `src/lib/ipc.ts`.

**The `Implemented by` column is binding.** Before v6.3 this table listed the surface without saying who built it, and thirty-seven of these commands were named by no task at all — an agent could have closed every task in `docs/TASKS.md` with most of the IPC surface missing, and nothing would have caught it before T-065. A command is part of its owning task's scope even where that task's prose does not spell it out, and T-006 check 5 now verifies that every row names a task that exists.

| Command | Args | Returns | Implemented by |
|---|---|---|---|
| `probe_environment` | — | `EnvironmentReport` | T-010 |
| `list_runtimes` | — | `Vec<RuntimeBuild>` | T-024 |
| `check_for_updates` | — | `Vec<AvailableRelease>` | T-020 |
| `install_runtime` | `tag, backend` | `()` + `install-progress` events | T-022 |
| `activate_runtime` | `tag, backend` | `RuntimeBuild` | T-024 |
| `remove_runtime` | `tag, backend` | `()` | T-024 |
| `add_watched_folder` | `path` | `Vec<ModelEntry>` (scans, registers; files stay put) | T-031 |
| `remove_watched_folder` | `path` | `()` (unregisters its models, never touches files) | T-031 |
| `list_watched_folders` | — | `Vec<WatchedFolder>` | T-031 |
| `import_models` | `paths` | `ImportJobId` + `import-progress` events | T-031 |
| `cancel_import` | `job_id` | `()` | T-031 |
| `list_models` | — | `Vec<ModelEntry>` | T-031 |
| `rescan_models` | — | `Vec<ModelEntry>` | T-031 |
| `update_model_params` | `id, LaunchParams, SamplingDefaults` | `ModelEntry` | T-031 |
| `set_model_preload` | `id, bool` | `ModelEntry` | T-031 |
| `set_model_pinned` | `id, bool` | `ModelEntry` | T-031 |
| `estimate_vram` | `id, LaunchParams` | `VramEstimate` | T-032 |
| `preview_preset` | `id` | `String` | T-033 |
| `remove_model` | `id` | `()` (settings and history retained by path) | T-031 |
| `list_retained_settings` | — | `Vec<RetainedModelSettings>` | T-063 |
| `forget_retained_settings` | `file_path` | `()` | T-063 |
| `get_server_config` | — | `ServerConfig` | T-050 |
| `set_server_config` | `ServerConfig` | `ServerConfig` | T-050 |
| `start_server` | — | `ServerState` | T-040 |
| `stop_server` | — | `ServerState` | T-040 |
| `dismiss_crash` | — | `ServerState` | T-040 |
| `get_server_state` | — | `ServerState` | T-040 |
| `get_endpoint_state` | — | `EndpointState` | T-042 |
| `rebind_endpoint` | — | `EndpointState` (retry after a `BindFailed`) | T-044 |
| `get_request_log` | `limit` | `Vec<RequestLogEntry>` | T-051 |
| `clear_request_log` | — | `()` | T-051 |
| `get_loaded_models` | — | `Vec<LoadedModelState>` | T-041 |
| `load_model` | `id` | `()` | T-041 |
| `unload_model` | `id` | `()` | T-041 |
| `get_telemetry` | — | `TelemetrySnapshot` | T-060 |
| `set_api_key` | `key` | `()` (DPAPI-encrypted; never returned) | T-051 |
| `clear_api_key` | — | `()` | T-051 |
| `get_settings` | — | `AppSettings` | T-063 |
| `set_settings` | `AppSettings` | `AppSettings` | T-063 |
| `export_logs` | `path, filter` | `()` | T-046 |
| `create_crash_bundle` | `path` | `()` | T-062 |

`start_server` takes no argument: configuration comes from the database via `set_server_config`. This removes the class of bug where a UI screen holds a stale config and reapplies it on start.

**Events to the UI** (Tauri channel — the renderer never polls):

| Event | Payload | Emitted by |
|---|---|---|
| `server-state-changed` | `ServerState` | T-040 |
| `endpoint-state-changed` | `EndpointState` | T-042 |
| `server-log-line` | `{ level, line, at }` | T-040 |
| `install-progress` | `InstallProgress` | T-022 |
| `import-progress` | `ImportProgress` | T-031 |
| `telemetry-tick` | `TelemetrySnapshot` | T-060 |
| `model-load-state-changed` | `Vec<LoadedModelState>` | T-041 |
| `request-logged` | `RequestLogEntry` | T-051 |
| `preset-warning` | `{ model_id, flag, reason }` | T-033 |

`preset-warning` is the runtime half of `AGENTS.md` §1: when a modelled flag is absent from the active build's verified list, T-033 omits it and emits this. Without the event the omission would be invisible.

---

## 5. Orchestrator trait

Router mode is the v1 implementation; the trait keeps the `llama-swap` fallback contained (`PLAN.md` §2.3).

These are the app's **own** control calls to the upstream. They do not travel through the endpoint listener — that path carries client traffic only.

```rust
#[async_trait]
pub trait ModelOrchestrator: Send + Sync {
    async fn list_models(&self) -> Result<Vec<LoadedModelState>, AppError>;
    async fn load(&self, model_id: &str) -> Result<LoadOutcome, AppError>;
    async fn unload(&self, model_id: &str) -> Result<(), AppError>;
    async fn props(&self) -> Result<ServerProps, AppError>;
}

pub struct LoadOutcome {
    pub load_seconds: f64,
    pub vram_bytes: Option<u64>,
}
```

`LoadOutcome` is what T-041 writes into `launch_history`.

**The command line is not on this trait.** An earlier draft put `launch_arguments` here, which made the supervisor (T-040) depend on the orchestrator (T-041) while T-041 depends on T-040 — a cycle. The router command line is generated by the preset generator (T-033), which already owns the other half of the same configuration:

```rust
// core/src/preset_generator.rs — T-033
pub fn router_arguments(cfg: &ServerConfig, upstream_port: u16) -> Vec<String>;
```

It receives the upstream port because the app allocates it at spawn time, not from stored config. The returned vector always contains `--no-webui`, `--host 127.0.0.1` and `--port <upstream_port>`; `PLAN.md` §6 makes the loopback bind non-negotiable.

## 6. Provider traits

Two seams exist so that hardware-dependent code is testable on a machine without the hardware. Both have a mock implementation used throughout the test suite.

```rust
/// GPU access. Every task that reads GPU state goes through this,
/// which is why a GPU-less machine can take T-010, T-060 and T-061 to green.
pub trait NvmlProvider: Send + Sync {
    fn device_count(&self) -> Result<u32, AppError>;
    fn device_info(&self, index: u32) -> Result<GpuInfo, AppError>;
    fn device_telemetry(&self, index: u32) -> Result<GpuTelemetry, AppError>;
    fn driver_version(&self) -> Result<String, AppError>;
}

/// Windows registry access, so launch-at-startup is testable (T-063).
pub trait RegistryProvider: Send + Sync {
    fn set_run_entry(&self, name: &str, command: &str) -> Result<(), AppError>;
    fn remove_run_entry(&self, name: &str) -> Result<(), AppError>;
    fn get_run_entry(&self, name: &str) -> Result<Option<String>, AppError>;
}
```

A failure from either degrades to an absent value or a typed error. Neither ever panics, and neither is allowed to stall the UI.
