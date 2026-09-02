// GENERATED FILE — DO NOT EDIT.
//
// Produced by `npm run generate-types` (scripts/generate-types.mjs) from
// the Rust contract types in src-tauri/src/core/types.rs, which are
// themselves transcribed from docs/CONTRACTS.md §1. AGENTS.md invariant 9:
// this file is generated, never hand-edited. CI fails if it is stale.

// ---- AppError ----
/**
 * Full behaviour (`code()`, `message()`, `remediation()`) defined in T-004.
 * Every IPC command returns `Result<T, AppError>`.
 */
export type AppError = { "kind": "InvalidTransition", "detail": { from: string, command: string, } } | { "kind": "NotFound", "detail": { what: string, } } | { "kind": "Io", "detail": { message: string, } } | { "kind": "Database", "detail": { message: string, } } | { "kind": "Network", "detail": { message: string, } } | { "kind": "RateLimited", "detail": { retry_after_seconds: bigint, } } | { "kind": "ChecksumMismatch", "detail": { expected: string, actual: string, } } | { "kind": "UnsafeArchiveEntry", "detail": { entry: string, } } | { "kind": "InvalidPath", "detail": { path: string, reason: string, } } | { "kind": "GgufParse", "detail": { message: string, } } | { "kind": "PortInUse", "detail": { port: number, } } | { "kind": "UpstreamUnavailable", "detail": { state: string, } } | { "kind": "Unauthorized" } | { "kind": "SchemaTooNew", "detail": { found: number, known: number, } } | { "kind": "Internal", "detail": { message: string, } };

// ---- AppSettings ----
export type AppSettings = { 
/**
 * "dark" | "light" | "system"
 */
theme: string, launch_at_startup: boolean, default_picker_folder: string | null, 
/**
 * "off" | "notify" | "auto"
 */
auto_update_policy: string, log_retention_days: number, };

// ---- AvailableRelease ----
export type AvailableRelease = { build_tag: string, backend: Backend, asset_name: string, asset_url: string, 
/**
 * `None` when the release omits checksums.
 */
sha256: string | null, size_bytes: bigint, published_at: string, release_notes_url: string, };

// ---- Backend ----
/**
 * Open on purpose: a CUDA major the binary has never heard of must be
 * representable, not a parse failure. See `PLAN.md` §2.13.
 */
export type Backend = { "kind": "Cuda", major: number, } | { "kind": "Vulkan" } | { "kind": "Cpu" };

// ---- BackendSelection ----
/**
 * The result of [`select_backend`]: the chosen release plus an optional
 * warning. The warning is `Some` for Vulkan and CPU selections (performance
 * will be poor) and `None` for CUDA selections.
 *
 * `#[ts(export)]` so the frontend (T-024, the version-management UI) can
 * display the selection and its warning; `AvailableRelease` is already
 * exported, and this is a thin wrapper around it.
 */
export type BackendSelection = { release: AvailableRelease, warning: string | null, };

// ---- CheckStatus ----
export type CheckStatus = "Pass" | "Warn" | "Fail";

// ---- Compatibility ----
export type Compatibility = "Supported" | { "SupportedWithWarnings": Array<CompatibilityNote> } | { "Unsupported": string };

// ---- CompatibilityNote ----
export type CompatibilityNote = { text: string, 
/**
 * NVFP4 sets this; see `PLAN.md` §2.6.
 */
experimental: boolean, };

// ---- Diagnosis ----
export type Diagnosis = { 
/**
 * "cuda_dll_missing" | "oom_vram" | "port_in_use" | ...
 */
code: string, message: string, remediation: string, };

// ---- EndpointState ----
/**
 * The listener's lifecycle is independent of `ServerState` — that
 * independence is the point of `PLAN.md` §2.7. The endpoint stays bound
 * while llama-server is stopped, so clients get a structured 503 rather
 * than a refused connection.
 */
export type EndpointState = "Unbound" | { "Bound": { address: string, port: number, } } | { "BindFailed": { address: string, port: number, error: AppError, } };

// ---- EnvironmentReport ----
export type EnvironmentReport = { gpus: Array<GpuInfo>, system_ram_bytes: bigint, free_disk_bytes: bigint, os_build: string, checks: Array<HealthCheck>, };

// ---- EstimateConfidence ----
export type EstimateConfidence = "Calibrated" | "Heuristic";

// ---- EstimateInputs ----
export type EstimateInputs = { metadata: GgufMetadata, file_size_bytes: bigint, params: LaunchParams, vram_free_bytes: bigint, ram_free_bytes: bigint, };

// ---- FlashAttn ----
/**
 * Tri-state. Recent builds accept `on|off|auto` rather than a boolean.
 * VERIFY the type against `docs/verified-flags.md` before use.
 */
export type FlashAttn = "On" | "Off" | "Auto";

// ---- GgufMetadata ----
export type GgufMetadata = { 
/**
 * "llama" | "qwen3" | "deepseek2" | ...
 */
architecture: string, param_count: bigint | null, 
/**
 * "Q4_K_M" | "MXFP4" | "NVFP4" | "F16" | ...
 */
quantization: string, 
/**
 * Layer count — drives `-ngl`.
 */
block_count: number, context_length: number | null, embedding_length: number | null, 
/**
 * Both are required by the KV cache term of the estimator. With Grouped
 * Query Attention the ratio between them reaches 8:1, so ignoring
 * `attention_head_count_kv` misestimates the cache by nearly an order
 * of magnitude. When either is absent the estimator says so in its
 * notes rather than assuming they are equal.
 */
attention_head_count: number | null, attention_head_count_kv: number | null, has_chat_template: boolean, is_moe: boolean, expert_count: number | null, };

// ---- GpuInfo ----
export type GpuInfo = { index: number, name: string, 
/**
 * Blackwell = (12, 0) or (10, x).
 */
compute_capability: [number, number], vram_total_bytes: bigint, vram_free_bytes: bigint, driver_version: string, cuda_version: string | null, };

// ---- GpuTelemetry ----
export type GpuTelemetry = { index: number, vram_used_bytes: bigint, vram_total_bytes: bigint, utilization_percent: number, temperature_c: number | null, };

// ---- HealthCheck ----
export type HealthCheck = { 
/**
 * "gpu_present" | "driver_ok" | "cuda13_ok" | "disk_space" | "endpoint_bindable"
 */
id: string, status: CheckStatus, message: string, remediation: string | null, };

// ---- ImportProgress ----
export type ImportProgress = { "Queued": { job_id: string, total_files: number, } } | { "FileStarted": { job_id: string, path: string, index: number, total: number, } } | { "FileDone": { job_id: string, entry: ModelEntry, index: number, total: number, } } | { "FileFailed": { job_id: string, path: string, error: AppError, } } | { "Cancelled": { job_id: string, completed: number, } } | { "Finished": { job_id: string, imported: number, skipped: number, failed: number, } };

// ---- InstallProgress ----
export type InstallProgress = "Resolving" | { "Downloading": { received_bytes: bigint, total_bytes: bigint, bytes_per_sec: bigint, } } | "Verifying" | { "Extracting": { entries_done: number, entries_total: number, } } | "Registering" | { "Done": { build: RuntimeBuild, } } | { "Failed": { error: AppError, } };

// ---- LaunchParams ----
export type LaunchParams = { gpu_layers: number | null, ctx_size: number | null, batch_size: number | null, ubatch_size: number | null, flash_attn: FlashAttn | null, cache_type_k: string | null, cache_type_v: string | null, n_cpu_moe: number | null, tensor_split: Array<number> | null, main_gpu: number | null, no_mmap: boolean | null, mlock: boolean | null, threads: number | null, chat_template: string | null, mmproj_path: string | null, 
/**
 * Unvalidated passthrough. User-entered only — never set by app logic.
 * See `AGENTS.md` §1.
 */
extra_args: Array<string>, };

// ---- LaunchRecord ----
export type LaunchRecord = { 
/**
 * Absolute path, not the entry id: history outlives the catalogue entry.
 */
file_path: string, launched_at: string, params: LaunchParams, succeeded: boolean, actual_vram_bytes: bigint | null, load_seconds: number | null, };

// ---- LinkCapability ----
/**
 * What linking primitive this machine can actually use, established by
 * attempting one in a scratch directory rather than by reading a privilege.
 * The three are not interchangeable on Windows: a junction cannot link an
 * individual file, and a hard link cannot cross volumes. See `PLAN.md` §2.1.
 * Only reached under the `ScanOnly` registration channel.
 */
export type LinkCapability = "Symlink" | "HardLinkOnly" | "None";

// ---- LoadedModelState ----
export type LoadedModelState = { model_id: string, state: ModelLoadState, vram_bytes: bigint | null, last_used: string | null, error: string | null, };

// ---- ModelAvailability ----
export type ModelAvailability = "Present" | "Missing" | "Unreadable";

// ---- ModelEntry ----
export type ModelEntry = { 
/**
 * Stable slug derived from the absolute path.
 */
id: string, display_name: string, 
/**
 * What `/v1/models` exposes; unique across the catalogue.
 */
served_name: string, 
/**
 * Absolute, wherever the user keeps it — never moved.
 */
file_path: string, shard_paths: Array<string>, size_bytes: bigint, 
/**
 * Hash of first 1 MiB + size — duplicate *signal*, not identity.
 */
sha256_head: string, metadata: GgufMetadata, compatibility: Compatibility, availability: ModelAvailability, 
/**
 * Another entry with the same `sha256_head`.
 */
duplicate_of: string | null, launch_params: LaunchParams, sampling_defaults: SamplingDefaults, 
/**
 * Loaded at server start rather than on demand.
 */
preload: boolean, 
/**
 * Exempt from LRU eviction.
 */
pinned: boolean, added_at: string, };

// ---- ModelLoadState ----
export type ModelLoadState = "Registered" | "Loading" | "Loaded" | "Unloading" | "Failed";

// ---- RegistrationChannel ----
/**
 * How this build lets a model enter the router's registry.
 * Settled by parsing `--help` in T-023; see `PLAN.md` §2.1.
 */
export type RegistrationChannel = "PresetDeclaresPath" | "ScanOnly" | "Undetermined";

// ---- RequestLogEntry ----
/**
 * Never contains a body, a header value, or the model name. See `PLAN.md` §6.
 */
export type RequestLogEntry = { at: string, method: string, path: string, status: number, duration_ms: bigint, request_bytes: bigint, response_bytes: bigint, streamed: boolean, authenticated: boolean, };

// ---- RetainedModelSettings ----
/**
 * Survives removal of the catalogue entry, keyed by the model's absolute
 * path. Re-importing the same file restores it. See `PLAN.md` §2.9.
 */
export type RetainedModelSettings = { file_path: string, launch_params: LaunchParams, sampling_defaults: SamplingDefaults, preload: boolean, pinned: boolean, retained_at: string, };

// ---- RuntimeBuild ----
export type RuntimeBuild = { 
/**
 * "b9196"
 */
build_tag: string, backend: Backend, install_path: string, is_active: boolean, installed_at: string, verified_flags: Array<VerifiedFlag>, registration_channel: RegistrationChannel, };

// ---- SamplingDefaults ----
export type SamplingDefaults = { temperature: number | null, top_p: number | null, top_k: number | null, min_p: number | null, repeat_penalty: number | null, presence_penalty: number | null, frequency_penalty: number | null, seed: bigint | null, };

// ---- ServerConfig ----
export type ServerConfig = { 
/**
 * The address clients configure. Owned by the app, stable across restarts.
 */
listen_address: string, 
/**
 * Default 8080; never auto-incremented.
 */
listen_port: number, 
/**
 * Range the app draws the upstream loopback port from. Internal.
 * Default `(49500, 49999)`.
 */
upstream_port_range: [number, number], 
/**
 * How long a request is held while the coordinator is not yet
 * answering. Default 120.
 */
startup_hold_seconds: number, 
/**
 * How long the coordinator itself may take to answer before Crashed.
 * Default 120.
 */
process_timeout_seconds: number, 
/**
 * How long preloading may take before Crashed. Separate from the above
 * because a 65 GB preload is minutes, not seconds, and one timeout
 * cannot serve both without being uselessly loose for the first.
 * Default 900.
 */
preload_timeout_seconds: number, 
/**
 * `--models-max`; 1 = strict hot-swap.
 */
models_max: number, 
/**
 * `--models-autoload`.
 */
autoload: boolean, 
/**
 * **No flag is known to implement this.** It is modelled here and
 * shown in T-050, but `docs/LLAMACPP.md` §1 lists it as `[assumed]`
 * with no observed spelling. T-023 confirms it against the capture;
 * until then it is a setting without a mechanism, and T-050 hides the
 * control rather than offering one that does nothing. See
 * `PROGRESS.md` D-003.
 */
idle_unload_seconds: number | null, 
/**
 * Never the key itself.
 */
api_key_set: boolean, 
/**
 * Default true; in-memory only.
 */
request_log_enabled: boolean, 
/**
 * Concurrent in-flight requests the listener will admit. `None` — the
 * default — means the app imposes no limit of its own and lets the
 * upstream decide, which is the transparent behaviour. When set,
 * requests over the limit are refused immediately with 503 and
 * `Retry-After`. **The app never queues**: holding a request behind
 * others is a scheduling decision, and scheduling is not the app's job
 * (`AGENTS.md` invariant 3).
 */
max_concurrent_requests: number | null, };

// ---- ServerProps ----
export type ServerProps = { build_tag: string | null, models_max: number | null, 
/**
 * `/props` is not a stable contract; keep the original.
 *
 * `#[ts(type = "unknown")]`: ts-rs's `serde-json-impl` feature names
 * this `JsonValue` in the generated TS rather than defining it, so a
 * file that used the feature's default would reference an undefined
 * type the moment two or more exported files were concatenated (see
 * PROGRESS.md — caught by the owner's real `npm run generate-types`
 * run, not by this session). Overriding to `unknown` sidesteps needing
 * that definition at all; the field is already documented above as
 * "not a stable contract", so a type a caller must narrow before use
 * is arguably more honest than a named alias would have been anyway.
 */
raw: unknown, };

// ---- ServerState ----
export type ServerState = "Stopped" | { "Starting": { since: string, upstream_port: number, phase: StartupPhase, } } | { "Running": { pid: number, upstream_port: number, since: string, config_dirty: boolean, } } | "Stopping" | { "Crashed": { exit_code: number | null, diagnosis: Diagnosis | null, last_log: Array<string>, } };

// ---- StartupPhase ----
/**
 * Startup has two phases with very different durations: the coordinator
 * answering takes seconds, preloading a 65 GB model takes minutes.
 * Collapsing them into one opaque `Starting` gives the user a progress bar
 * that appears frozen. See `PLAN.md` §2.12.
 */
export type StartupPhase = "WaitingForProcess" | { "Preloading": { done: number, total: number, current: string | null, } };

// ---- TelemetrySnapshot ----
export type TelemetrySnapshot = { sampled_at: string, gpus: Array<GpuTelemetry>, loaded_models: Array<LoadedModelState>, 
/**
 * Counted at the app's listener. Available because the app owns the
 * endpoint — under direct binding these had no source.
 */
active_requests: number, requests_last_minute: number, 
/**
 * Parsed from the server's own stderr, never from response bodies.
 */
tokens_per_sec: number | null, uptime_seconds: bigint, };

// ---- VerifiedFlag ----
export type VerifiedFlag = { 
/**
 * "--flash-attn"
 */
name: string, takes_value: boolean, allowed_values: Array<string> | null, };

// ---- VramEstimate ----
export type VramEstimate = { recommended_gpu_layers: number, estimated_vram_bytes: bigint, estimated_ram_bytes: bigint, kv_cache_bytes: bigint, fits_fully: boolean, confidence: EstimateConfidence, notes: Array<string>, };

// ---- WatchedFolder ----
export type WatchedFolder = { path: string, model_count: number, reachable: boolean, last_scan_at: string | null, };
