// GENERATED FILE — DO NOT EDIT.
//
// Produced by `npm run generate-types` (scripts/generate-types.mjs) from
// the Rust contract types in src-tauri/src/core/types.rs, which are
// themselves transcribed from docs/CONTRACTS.md §1. AGENTS.md invariant 9:
// this file is generated, never hand-edited. CI fails if it is stale.
//
// This particular commit was produced without a local Rust toolchain (see
// PROGRESS.md) — hand-transcribed to match what `ts-rs` 12.x should emit for
// the types in core/types.rs, not captured from a real `cargo test` run.
// CI runs the real generator and the freshness check; if it disagrees with
// this file, pull its output back rather than hand-editing this one further
// (docs/WORKFLOW.md §4 — the same pattern T-001 used for Cargo.lock).

// ---- AppError ----
export type AppError =
  | { kind: "InvalidTransition", detail: { from: string, command: string } }
  | { kind: "NotFound", detail: { what: string } }
  | { kind: "Io", detail: { message: string } }
  | { kind: "Database", detail: { message: string } }
  | { kind: "Network", detail: { message: string } }
  | { kind: "RateLimited", detail: { retry_after_seconds: bigint } }
  | { kind: "ChecksumMismatch", detail: { expected: string, actual: string } }
  | { kind: "UnsafeArchiveEntry", detail: { entry: string } }
  | { kind: "InvalidPath", detail: { path: string, reason: string } }
  | { kind: "GgufParse", detail: { message: string } }
  | { kind: "PortInUse", detail: { port: number } }
  | { kind: "UpstreamUnavailable", detail: { state: string } }
  | { kind: "Unauthorized" }
  | { kind: "SchemaTooNew", detail: { found: number, known: number } }
  | { kind: "Internal", detail: { message: string } };

// ---- AppSettings ----
export type AppSettings = { theme: string, launch_at_startup: boolean, default_picker_folder: string | null, auto_update_policy: string, log_retention_days: number, };

// ---- AvailableRelease ----
export type AvailableRelease = { build_tag: string, backend: Backend, asset_name: string, asset_url: string, sha256: string | null, size_bytes: bigint, published_at: string, release_notes_url: string, };

// ---- Backend ----
export type Backend = { kind: "Cuda", major: number } | { kind: "Vulkan" } | { kind: "Cpu" };

// ---- CheckStatus ----
export type CheckStatus = "Pass" | "Warn" | "Fail";

// ---- Compatibility ----
export type Compatibility = "Supported" | { SupportedWithWarnings: Array<CompatibilityNote> } | { Unsupported: string };

// ---- CompatibilityNote ----
export type CompatibilityNote = { text: string, experimental: boolean, };

// ---- Diagnosis ----
export type Diagnosis = { code: string, message: string, remediation: string, };

// ---- EndpointState ----
export type EndpointState = "Unbound" | { Bound: { address: string, port: number } } | { BindFailed: { address: string, port: number, error: AppError } };

// ---- EnvironmentReport ----
export type EnvironmentReport = { gpus: Array<GpuInfo>, system_ram_bytes: bigint, free_disk_bytes: bigint, os_build: string, checks: Array<HealthCheck>, };

// ---- EstimateConfidence ----
export type EstimateConfidence = "Calibrated" | "Heuristic";

// ---- EstimateInputs ----
export type EstimateInputs = { metadata: GgufMetadata, file_size_bytes: bigint, params: LaunchParams, vram_free_bytes: bigint, ram_free_bytes: bigint, };

// ---- FlashAttn ----
export type FlashAttn = "On" | "Off" | "Auto";

// ---- GgufMetadata ----
export type GgufMetadata = { architecture: string, param_count: bigint | null, quantization: string, block_count: number, context_length: number | null, embedding_length: number | null, attention_head_count: number | null, attention_head_count_kv: number | null, has_chat_template: boolean, is_moe: boolean, expert_count: number | null, };

// ---- GpuInfo ----
export type GpuInfo = { index: number, name: string, compute_capability: [number, number], vram_total_bytes: bigint, vram_free_bytes: bigint, driver_version: string, cuda_version: string | null, };

// ---- GpuTelemetry ----
export type GpuTelemetry = { index: number, vram_used_bytes: bigint, vram_total_bytes: bigint, utilization_percent: number, temperature_c: number | null, };

// ---- HealthCheck ----
export type HealthCheck = { id: string, status: CheckStatus, message: string, remediation: string | null, };

// ---- ImportProgress ----
export type ImportProgress =
  | { Queued: { job_id: string, total_files: number } }
  | { FileStarted: { job_id: string, path: string, index: number, total: number } }
  | { FileDone: { job_id: string, entry: ModelEntry, index: number, total: number } }
  | { FileFailed: { job_id: string, path: string, error: AppError } }
  | { Cancelled: { job_id: string, completed: number } }
  | { Finished: { job_id: string, imported: number, skipped: number, failed: number } };

// ---- InstallProgress ----
export type InstallProgress =
  | "Resolving"
  | { Downloading: { received_bytes: bigint, total_bytes: bigint, bytes_per_sec: bigint } }
  | "Verifying"
  | { Extracting: { entries_done: number, entries_total: number } }
  | "Registering"
  | { Done: { build: RuntimeBuild } }
  | { Failed: { error: AppError } };

// ---- LaunchParams ----
export type LaunchParams = { gpu_layers: number | null, ctx_size: number | null, batch_size: number | null, ubatch_size: number | null, flash_attn: FlashAttn | null, cache_type_k: string | null, cache_type_v: string | null, n_cpu_moe: number | null, tensor_split: Array<number> | null, main_gpu: number | null, no_mmap: boolean | null, mlock: boolean | null, threads: number | null, chat_template: string | null, mmproj_path: string | null, extra_args: Array<string>, };

// ---- LaunchRecord ----
export type LaunchRecord = { file_path: string, launched_at: string, params: LaunchParams, succeeded: boolean, actual_vram_bytes: bigint | null, load_seconds: number | null, };

// ---- LinkCapability ----
export type LinkCapability = "Symlink" | "HardLinkOnly" | "None";

// ---- LoadedModelState ----
export type LoadedModelState = { model_id: string, state: ModelLoadState, vram_bytes: bigint | null, last_used: string | null, error: string | null, };

// ---- ModelAvailability ----
export type ModelAvailability = "Present" | "Missing" | "Unreadable";

// ---- ModelEntry ----
export type ModelEntry = { id: string, display_name: string, served_name: string, file_path: string, shard_paths: Array<string>, size_bytes: bigint, sha256_head: string, metadata: GgufMetadata, compatibility: Compatibility, availability: ModelAvailability, duplicate_of: string | null, launch_params: LaunchParams, sampling_defaults: SamplingDefaults, preload: boolean, pinned: boolean, added_at: string, };

// ---- ModelLoadState ----
export type ModelLoadState = "Registered" | "Loading" | "Loaded" | "Unloading" | "Failed";

// ---- RegistrationChannel ----
export type RegistrationChannel = "PresetDeclaresPath" | "ScanOnly" | "Undetermined";

// ---- RequestLogEntry ----
export type RequestLogEntry = { at: string, method: string, path: string, status: number, duration_ms: bigint, request_bytes: bigint, response_bytes: bigint, streamed: boolean, authenticated: boolean, };

// ---- RetainedModelSettings ----
export type RetainedModelSettings = { file_path: string, launch_params: LaunchParams, sampling_defaults: SamplingDefaults, preload: boolean, pinned: boolean, retained_at: string, };

// ---- RuntimeBuild ----
export type RuntimeBuild = { build_tag: string, backend: Backend, install_path: string, is_active: boolean, installed_at: string, verified_flags: Array<VerifiedFlag>, registration_channel: RegistrationChannel, };

// ---- SamplingDefaults ----
export type SamplingDefaults = { temperature: number | null, top_p: number | null, top_k: number | null, min_p: number | null, repeat_penalty: number | null, presence_penalty: number | null, frequency_penalty: number | null, seed: bigint | null, };

// ---- ServerConfig ----
export type ServerConfig = { listen_address: string, listen_port: number, upstream_port_range: [number, number], startup_hold_seconds: number, process_timeout_seconds: number, preload_timeout_seconds: number, models_max: number, autoload: boolean, idle_unload_seconds: number | null, api_key_set: boolean, request_log_enabled: boolean, max_concurrent_requests: number | null, };

// ---- ServerProps ----
export type ServerProps = { build_tag: string | null, models_max: number | null, raw: unknown, };

// ---- ServerState ----
export type ServerState =
  | "Stopped"
  | { Starting: { since: string, upstream_port: number, phase: StartupPhase } }
  | { Running: { pid: number, upstream_port: number, since: string, config_dirty: boolean } }
  | "Stopping"
  | { Crashed: { exit_code: number | null, diagnosis: Diagnosis | null, last_log: Array<string> } };

// ---- StartupPhase ----
export type StartupPhase = "WaitingForProcess" | { Preloading: { done: number, total: number, current: string | null } };

// ---- TelemetrySnapshot ----
export type TelemetrySnapshot = { sampled_at: string, gpus: Array<GpuTelemetry>, loaded_models: Array<LoadedModelState>, active_requests: number, requests_last_minute: number, tokens_per_sec: number | null, uptime_seconds: bigint, };

// ---- VerifiedFlag ----
export type VerifiedFlag = { name: string, takes_value: boolean, allowed_values: Array<string> | null, };

// ---- VramEstimate ----
export type VramEstimate = { recommended_gpu_layers: number, estimated_vram_bytes: bigint, estimated_ram_bytes: bigint, kv_cache_bytes: bigint, fits_fully: boolean, confidence: EstimateConfidence, notes: Array<string>, };

// ---- WatchedFolder ----
export type WatchedFolder = { path: string, model_count: number, reachable: boolean, last_scan_at: string | null, };
