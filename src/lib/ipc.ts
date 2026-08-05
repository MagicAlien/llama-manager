// IPC wrappers — hand-written, per docs/CONTRACTS.md §4 (T-002).
//
// AGENTS.md invariant 9 / docs/adr/adr-001-type-generation.md: this file
// imports its types from the generated ./types — it never redeclares them.
// A lint rule (eslint.config.js) enforces that for this file specifically.
//
// Every command below returns `Promise<T>`, matching the `T` in the Rust
// side's `Result<T, AppError>` — a rejected command surfaces as a rejected
// Promise carrying the serialized `AppError`. None of these commands has a
// Rust-side handler yet (see docs/CONTRACTS.md §4's "Implemented by"
// column); calling one before its owning task lands will reject with
// Tauri's own "command not found" error, not an `AppError`.

import { invoke } from "@tauri-apps/api/core";

import type {
  AppSettings,
  AvailableRelease,
  Backend,
  EndpointState,
  EnvironmentReport,
  LaunchParams,
  LoadedModelState,
  ModelEntry,
  RequestLogEntry,
  RetainedModelSettings,
  RuntimeBuild,
  SamplingDefaults,
  ServerConfig,
  ServerState,
  TelemetrySnapshot,
  VramEstimate,
  WatchedFolder,
} from "./types";

// T-010
export function probeEnvironment(): Promise<EnvironmentReport> {
  return invoke<EnvironmentReport>("probe_environment");
}

// T-024
export function listRuntimes(): Promise<RuntimeBuild[]> {
  return invoke<RuntimeBuild[]>("list_runtimes");
}

// T-020
export function checkForUpdates(): Promise<AvailableRelease[]> {
  return invoke<AvailableRelease[]>("check_for_updates");
}

// T-022 — progress reported via the `install-progress` event, not the
// command's return value.
export function installRuntime(tag: string, backend: Backend): Promise<void> {
  return invoke<void>("install_runtime", { tag, backend });
}

// T-024
export function activateRuntime(tag: string, backend: Backend): Promise<RuntimeBuild> {
  return invoke<RuntimeBuild>("activate_runtime", { tag, backend });
}

// T-024
export function removeRuntime(tag: string, backend: Backend): Promise<void> {
  return invoke<void>("remove_runtime", { tag, backend });
}

// T-031 — scans and registers; files stay put.
export function addWatchedFolder(path: string): Promise<ModelEntry[]> {
  return invoke<ModelEntry[]>("add_watched_folder", { path });
}

// T-031 — unregisters its models, never touches files.
export function removeWatchedFolder(path: string): Promise<void> {
  return invoke<void>("remove_watched_folder", { path });
}

// T-031
export function listWatchedFolders(): Promise<WatchedFolder[]> {
  return invoke<WatchedFolder[]>("list_watched_folders");
}

// T-031 — progress reported via the `import-progress` event.
export function importModels(paths: string[]): Promise<string /* ImportJobId */> {
  return invoke<string>("import_models", { paths });
}

// T-031
export function cancelImport(jobId: string /* ImportJobId */): Promise<void> {
  return invoke<void>("cancel_import", { jobId });
}

// T-031
export function listModels(): Promise<ModelEntry[]> {
  return invoke<ModelEntry[]>("list_models");
}

// T-031
export function rescanModels(): Promise<ModelEntry[]> {
  return invoke<ModelEntry[]>("rescan_models");
}

// T-031
export function updateModelParams(
  id: string,
  launchParams: LaunchParams,
  samplingDefaults: SamplingDefaults,
): Promise<ModelEntry> {
  return invoke<ModelEntry>("update_model_params", { id, launchParams, samplingDefaults });
}

// T-031
export function setModelPreload(id: string, preload: boolean): Promise<ModelEntry> {
  return invoke<ModelEntry>("set_model_preload", { id, preload });
}

// T-031
export function setModelPinned(id: string, pinned: boolean): Promise<ModelEntry> {
  return invoke<ModelEntry>("set_model_pinned", { id, pinned });
}

// T-032
export function estimateVram(id: string, launchParams: LaunchParams): Promise<VramEstimate> {
  return invoke<VramEstimate>("estimate_vram", { id, launchParams });
}

// T-033
export function previewPreset(id: string): Promise<string> {
  return invoke<string>("preview_preset", { id });
}

// T-031 — settings and history retained by path.
export function removeModel(id: string): Promise<void> {
  return invoke<void>("remove_model", { id });
}

// T-063
export function listRetainedSettings(): Promise<RetainedModelSettings[]> {
  return invoke<RetainedModelSettings[]>("list_retained_settings");
}

// T-063
export function forgetRetainedSettings(filePath: string): Promise<void> {
  return invoke<void>("forget_retained_settings", { filePath });
}

// T-050
export function getServerConfig(): Promise<ServerConfig> {
  return invoke<ServerConfig>("get_server_config");
}

// T-050
export function setServerConfig(config: ServerConfig): Promise<ServerConfig> {
  return invoke<ServerConfig>("set_server_config", { config });
}

// T-040 — no argument: configuration comes from the database via
// `setServerConfig` (docs/CONTRACTS.md §4).
export function startServer(): Promise<ServerState> {
  return invoke<ServerState>("start_server");
}

// T-040
export function stopServer(): Promise<ServerState> {
  return invoke<ServerState>("stop_server");
}

// T-040
export function dismissCrash(): Promise<ServerState> {
  return invoke<ServerState>("dismiss_crash");
}

// T-040
export function getServerState(): Promise<ServerState> {
  return invoke<ServerState>("get_server_state");
}

// T-042
export function getEndpointState(): Promise<EndpointState> {
  return invoke<EndpointState>("get_endpoint_state");
}

// T-044 — retry after a `BindFailed`.
export function rebindEndpoint(): Promise<EndpointState> {
  return invoke<EndpointState>("rebind_endpoint");
}

// T-051
export function getRequestLog(limit: number): Promise<RequestLogEntry[]> {
  return invoke<RequestLogEntry[]>("get_request_log", { limit });
}

// T-051
export function clearRequestLog(): Promise<void> {
  return invoke<void>("clear_request_log");
}

// T-041
export function getLoadedModels(): Promise<LoadedModelState[]> {
  return invoke<LoadedModelState[]>("get_loaded_models");
}

// T-041
export function loadModel(id: string): Promise<void> {
  return invoke<void>("load_model", { id });
}

// T-041
export function unloadModel(id: string): Promise<void> {
  return invoke<void>("unload_model", { id });
}

// T-060
export function getTelemetry(): Promise<TelemetrySnapshot> {
  return invoke<TelemetrySnapshot>("get_telemetry");
}

// T-051 — DPAPI-encrypted; never returned.
export function setApiKey(key: string): Promise<void> {
  return invoke<void>("set_api_key", { key });
}

// T-051
export function clearApiKey(): Promise<void> {
  return invoke<void>("clear_api_key");
}

// T-063
export function getSettings(): Promise<AppSettings> {
  return invoke<AppSettings>("get_settings");
}

// T-063
export function setSettings(settings: AppSettings): Promise<AppSettings> {
  return invoke<AppSettings>("set_settings", { settings });
}

// T-046
export function exportLogs(path: string, filter: string): Promise<void> {
  return invoke<void>("export_logs", { path, filter });
}

// T-062
export function createCrashBundle(path: string): Promise<void> {
  return invoke<void>("create_crash_bundle", { path });
}
