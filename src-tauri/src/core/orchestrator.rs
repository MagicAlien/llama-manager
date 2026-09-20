//! `core::orchestrator` — T-041: the app's own control channel to the router.
//!
//! `docs/CONTRACTS.md` §5 is the specification. `ModelOrchestrator` is the seam
//! (`PLAN.md` §2.3 keeps the `llama-swap` fallback contained behind it) and
//! [`RouterOrchestrator`] is the v1 implementation: poll `GET /models`, explicit
//! load and unload, read `/props`, back off on errors.
//!
//! **These are the app's control calls and they go straight to the upstream.**
//! They do not travel through the endpoint listener (T-042), which carries
//! client traffic only — the same distinction §5 draws.
//!
//! # The endpoints, and where each one was established
//!
//! Nothing below is written from memory. Every path and payload shape was read
//! off the **installed build** (`b10883-91f6a6cf3`, the active runtime in this
//! machine's catalogue): its route table at its own commit
//! (`tools/server/server.cpp`, the router block installing `/models`,
//! `/models/load`, `/models/unload`, `/models/sse` and `DELETE /models` over the
//! normal `/models`, `/v1/models` and `/props` routes), its handler bodies
//! (`tools/server/server-models.cpp`) and the fenced live responses `PROGRESS.md`
//! F-021 records. The shapes that matter:
//!
//! | Call | Request | Answer |
//! |---|---|---|
//! | [`MODELS_PATH`] | `GET /models` | `{"data":[{ id, status:{ value, args, failed?, exit_code? }, … }]}` — the load state lives in `status.value` |
//! | [`LOAD_PATH`] | `POST /models/load` `{"model":"<name>"}` | `{"success":true}`, or the error object below |
//! | [`UNLOAD_PATH`] | `POST /models/unload` `{"model":"<name>"}` | `{"success":true}`, or the error object below |
//! | [`PROPS_PATH`] | `GET /props` | `{"role":"router","max_instances":4,"build_info":"b10883-91f6a6cf3",…}` |
//!
//! # `/models` is not `/v1/models`, and the difference is intent
//!
//! On this build the two answer **byte-identical payloads** — measured, 955
//! bytes each (F-020, re-confirmed in T-041's own probe). What differs is what
//! they are for: `/v1/models` is the OpenAI-compatible listing clients are
//! allowed to read, while `/models` is the router's own management view, whose
//! entries carry `status`, `source` and `can_remove`. The app therefore reads
//! its model state from `/models` and never from `/v1/models` — asserted by a
//! test that inspects the paths the stub upstream was actually asked for, so the
//! distinction survives a future build where the two payloads diverge.
//!
//! # A model's name is the preset section name
//!
//! `POST /models/load` addresses a model by the name the router knows it under,
//! which is the section name in the generated preset (`PROGRESS.md` F-015:
//! `[test-external]` registers as `test-external`). That is the catalogue's
//! `served_name` — [`OrchestratedModel::name`] — and it is passed through
//! unchanged, like the absolute path recorded for the load.
//!
//! # Errors: what each failure becomes
//!
//! `AppError` (§1) has no variant for "the upstream refused", and adding one
//! would be a contract change, so the mapping is explicit and one-way:
//!
//! - **No server to talk to** — the port source reports no upstream (Stopped,
//!   Stopping, Crashed): [`AppError::UpstreamUnavailable`], carrying the state
//!   that made the call impossible.
//! - **The upstream answered non-2xx** — [`AppError::Network`] whose message
//!   carries the status, the error type and the server's own `message`, e.g.
//!   `POST /models/load answered 400 invalid_request_error: model is not
//!   running`. The server's words are never replaced by the app's.
//! - **A model that failed to load** — the same `Network` shape, carrying the
//!   message the status block produced (`the server reported the load failed
//!   (exit code 5)`), because on this build a failed load is not a state of its
//!   own: the model returns to `unloaded` with `status.failed = true` and the
//!   child's `exit_code` (F-020).
//! - **A load that never concludes** — bounded by [`Tuning::load_timeout`], so
//!   it surfaces as a typed error rather than hanging a caller.
//! - **The app's own catalogue** — `NotFound` for an id it does not hold, which
//!   is a different failure from the server not knowing the model.
//!
//! # `launch_history` — the write that makes calibration possible
//!
//! §3 is explicit that this table is functional, not bookkeeping, and that
//! "without the writer the calibration path is dead code". [`LaunchHistorySink`]
//! is that writer: one row per **load attempt**, success or failure, keyed by
//! the model's absolute path, carrying `succeeded`, `load_seconds`,
//! `error_message` and `actual_vram_bytes`.
//!
//! `actual_vram_bytes` is `None` on this build, and that is a measurement, not
//! an omission: the router's `/models` reports no VRAM figure for a loaded
//! model (the loaded entry merges the child's `meta` block — `n_ctx`, `n_vram`-
//! free: `vocab_type`, `n_ctx`, `n_ctx_train`, `n_embd`, `n_params`, `size`,
//! `ftype`) and the child's own `/props` does not carry one either. Recording a
//! guessed figure would poison the estimator's calibration with a number nobody
//! measured, so the column stays null until a task that can actually measure it
//! (T-060's telemetry, or a NVML delta) fills it.
//!
//! The row is written **when the attempt concludes**, not when it starts. The
//! alternative — insert a placeholder and update it — leaves a row reading
//! `succeeded = false, error_message = NULL` if the app is killed mid-load,
//! which is a failure that did not happen; one row per concluded attempt is the
//! honest shape.
//!
//! # Everything external is a seam
//!
//! The HTTP calls, the catalogue, the upstream port, the history sink and the
//! event sink are five traits. The state machine is therefore exercised against
//! scripted doubles and a **real local stub HTTP server** (`AGENTS.md` §3: no
//! criterion needs the target hardware), while the production implementations
//! are separately driven against the installed build.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::core::supervisor;
use crate::core::types::{
    AppError, LaunchParams, LaunchRecord, LoadOutcome, LoadedModelState, ModelLoadState,
    ServerProps, ServerState,
};

// ─── The trait (`docs/CONTRACTS.md` §5) ─────────────────────────

/// The boxed future every [`ModelOrchestrator`] method returns.
///
/// §5 writes the trait with `#[async_trait]`. This crate has no `async-trait`
/// dependency (`PLAN.md` §3's approved list does not carry one) and the
/// hand-written equivalent — a `Pin<Box<dyn Future + Send>>` alias — is what
/// that macro expands to: the same signatures, the same semantics, and still
/// `dyn`-compatible, which the macro form would have made it by boxing anyway.
/// No dependency is added for a spelling.
pub type OrchestratorFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// The app's own calls to the coordinator. `docs/CONTRACTS.md` §5.
///
/// A second implementation exists ([`NoopOrchestrator`]) and a test drives both
/// through this trait, because a trait with one implementor is a description of
/// that implementor rather than a seam.
///
/// **`dead_code` here is the seam, not an oversight.** No production caller
/// drives these futures: `ipc/` calls the synchronous methods, because a
/// future that blocks on the upstream must never be polled *on* Tauri's
/// runtime (the HTTP client drives its own). Their consumers today are the
/// tests, which drive both implementations through this trait, and — when
/// the fallback of `PLAN.md` §2.3 lands — whatever swaps the no-op in.
#[allow(dead_code)]
pub trait ModelOrchestrator: Send + Sync {
    /// What the upstream currently holds, with each model's load state.
    fn list_models(&self) -> OrchestratorFuture<'_, Vec<LoadedModelState>>;

    /// Load `model_id` and wait until the server reports it ready, failed, or
    /// past [`Tuning::load_timeout`].
    fn load<'a>(
        &'a self,
        model_id: &'a str,
    ) -> OrchestratorFuture<'a, crate::core::types::LoadOutcome>;

    /// Unload `model_id`, waiting until the server reports it no longer loaded.
    fn unload<'a>(&'a self, model_id: &'a str) -> OrchestratorFuture<'a, ()>;

    /// The upstream's `/props`, as [`ServerProps`].
    fn props(&self) -> OrchestratorFuture<'_, ServerProps>;
}

// ─── Endpoints ──────────────────────────────────────────────────

/// The router's management view. Not `/v1/models` — see the module docs.
pub const MODELS_PATH: &str = "/models";
/// Explicit load. Body: `{"model": "<served name>"}`.
pub const LOAD_PATH: &str = "/models/load";
/// Explicit unload. Body: `{"model": "<served name>"}`.
pub const UNLOAD_PATH: &str = "/models/unload";
/// The router's own identity: `role`, `max_instances`, `build_info`.
pub const PROPS_PATH: &str = "/props";

// ─── The catalogue and the history sink ─────────────────────────

/// A model as the orchestrator needs it: the name the router knows it by, the
/// absolute path recorded in `launch_history`, and the parameters the load was
/// attempted with.
#[derive(Clone, Debug, PartialEq)]
pub struct OrchestratedModel {
    /// The catalogue's id — what an IPC caller passes in.
    pub model_id: String,
    /// `served_name`: the preset section name, i.e. the router's own name for
    /// this model. Passed to the server **unchanged**.
    pub name: String,
    /// Absolute path, passed through unchanged — it is the `launch_history` key
    /// and it may live on any volume.
    pub file_path: PathBuf,
    pub params: LaunchParams,
}

/// What the orchestrator needs of the catalogue. One method, so a test can
/// drive the load path with no database at all.
pub trait ModelDirectory: Send + Sync {
    fn find(&self, model_id: &str) -> Result<OrchestratedModel, AppError>;
}

/// The production catalogue: the registry's own rows.
pub struct RegistryDirectory;

impl ModelDirectory for RegistryDirectory {
    fn find(&self, model_id: &str) -> Result<OrchestratedModel, AppError> {
        let entry = crate::core::model_registry::get_model(model_id)?.ok_or_else(|| {
            AppError::NotFound {
                what: format!("model {model_id}"),
            }
        })?;
        Ok(OrchestratedModel {
            model_id: entry.id,
            name: entry.served_name,
            file_path: entry.file_path,
            params: entry.launch_params,
        })
    }
}

/// One concluded load attempt, as `launch_history` stores it (`docs/CONTRACTS.md`
/// §3). Built by [`RouterOrchestrator`] and written by a [`LaunchHistorySink`].
#[derive(Clone, Debug, PartialEq)]
pub struct LaunchAttempt<'a> {
    pub file_path: &'a Path,
    /// The parameters the attempt was made with, serialized.
    pub params_json: String,
    pub attempted_at: chrono::DateTime<Utc>,
    pub succeeded: bool,
    pub load_seconds: Option<f64>,
    pub actual_vram_bytes: Option<u64>,
    pub error_message: Option<&'a str>,
}

/// Where a load attempt is recorded. `docs/CONTRACTS.md` §3, T-041.
pub trait LaunchHistorySink: Send + Sync {
    fn record(&self, attempt: &LaunchAttempt<'_>) -> Result<(), AppError>;
}

/// The production sink: the app's own database.
pub struct DbLaunchHistory {
    db_path: PathBuf,
}

impl DbLaunchHistory {
    pub fn at(db_path: PathBuf) -> Self {
        DbLaunchHistory { db_path }
    }

    /// `<data root>\llama-manager.db`, the file every other component uses.
    pub fn production() -> Result<Self, AppError> {
        let path = crate::core::installer::database_path().ok_or_else(|| AppError::Internal {
            message: "LOCALAPPDATA is not set; cannot locate the app database".to_string(),
        })?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AppError::Io {
                message: format!("could not create {}: {err}", parent.display()),
            })?;
        }
        Ok(DbLaunchHistory::at(path))
    }
}

impl LaunchHistorySink for DbLaunchHistory {
    fn record(&self, attempt: &LaunchAttempt<'_>) -> Result<(), AppError> {
        let conn = crate::db::open(&self.db_path)?;
        crate::db::queries::insert_launch_record(
            &conn,
            &crate::db::queries::NewLaunchRecord {
                file_path: attempt.file_path.display().to_string(),
                launched_at: attempt.attempted_at,
                params_json: attempt.params_json.clone(),
                succeeded: attempt.succeeded,
                actual_vram_bytes: attempt.actual_vram_bytes.map(|bytes| bytes as i64),
                load_seconds: attempt.load_seconds,
                error_message: attempt.error_message.map(str::to_string),
            },
        )?;
        Ok(())
    }
}

// ─── The router's HTTP face ─────────────────────────────────────

/// The router's HTTP endpoints, one method per call.
///
/// Synchronous on purpose: every caller is a blocking thread — Tauri's
/// `spawn_blocking` pool through `ipc/`, and the poller thread — never async
/// code, and this crate's async runtime is Tauri's. Each implementation bounds
/// its own call by the tuning's `probe_timeout` and returns `Err` rather than
/// blocking.
pub trait RouterApi: Send + Sync {
    fn models(&self, port: u16) -> Result<Vec<LoadedModelState>, AppError>;
    fn load(&self, port: u16, model: &str) -> Result<(), AppError>;
    fn unload(&self, port: u16, model: &str) -> Result<(), AppError>;
    fn props(&self, port: u16) -> Result<ServerProps, AppError>;
}

/// The real one: HTTP over loopback against the port T-040 allocated.
///
/// A fresh single-threaded Tokio runtime and client per call. The calls are
/// occasional (one poll every couple of seconds, one load, one unload), the
/// endpoint is plain HTTP on loopback so there is no TLS context to build, and a
/// per-call runtime cannot be raced by a second caller — which matters because
/// Tauri's blocking pool will happily run two of these at once, and a
/// `current_thread` runtime driven by two threads at the same time is not a
/// state worth leaving to chance.
pub struct RouterHttp {
    timeout: Duration,
}

impl RouterHttp {
    pub fn new(timeout: Duration) -> Self {
        RouterHttp { timeout }
    }

    fn url(port: u16, path: &str) -> String {
        format!("http://127.0.0.1:{port}{path}")
    }

    /// One request. Returns the status and the body verbatim: interpreting a
    /// non-2xx is the caller's job, because the message the app shows must be
    /// the server's, not this helper's summary of it.
    fn call(
        &self,
        port: u16,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(u16, String), AppError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| AppError::Internal {
                message: format!("could not start the orchestrator's HTTP runtime: {err}"),
            })?;
        let client = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|err| AppError::Internal {
                message: format!("could not build the orchestrator's HTTP client: {err}"),
            })?;
        let url = Self::url(port, path);
        let method_name = method.as_str().to_string();

        runtime.block_on(async move {
            let mut request = client.request(method, &url);
            if let Some(body) = body {
                request = request.json(&body);
            }
            let response = request.send().await.map_err(|err| AppError::Network {
                message: format!("{method_name} {url}: {err}"),
            })?;
            let status = response.status().as_u16();
            let text = response.text().await.map_err(|err| AppError::Network {
                message: format!("{method_name} {url}: {err}"),
            })?;
            Ok((status, text))
        })
    }

    /// Turn a non-2xx answer into the typed error: the server's own `message`
    /// is what the caller sees.
    fn expect_success(method: &str, path: &str, status: u16, body: &str) -> Result<(), AppError> {
        if (200..300).contains(&status) {
            return Ok(());
        }
        Err(server_error(method, path, status, body))
    }
}

/// The error body is the OpenAI shape — `{"error":{"message","type","code"}}`
/// as the installed build answered during T-041's probe:
/// `400 {"error":{"code":400,"message":"model is not running","type":"invalid_request_error"}}`.
/// A body that does not parse is reported verbatim (truncated) rather than
/// replaced by a generic message, since an unrecognised body is itself the
/// finding.
fn server_error(method: &str, path: &str, status: u16, body: &str) -> AppError {
    let parsed = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            let error = value.get("error")?;
            let message = error.get("message")?.as_str()?.to_string();
            let kind = error
                .get("type")
                .and_then(|kind| kind.as_str())
                .unwrap_or("error")
                .to_string();
            Some((kind, message))
        });

    match parsed {
        Some((kind, message)) => AppError::Network {
            message: format!("{method} {path} answered {status} {kind}: {message}"),
        },
        None => AppError::Network {
            message: format!("{method} {path} answered {status}: {}", truncate_body(body)),
        },
    }
}

fn truncate_body(body: &str) -> String {
    const LIMIT: usize = 200;
    let trimmed = body.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_string();
    }
    let head: String = trimmed.chars().take(LIMIT).collect();
    format!("{head}…")
}

impl RouterApi for RouterHttp {
    fn models(&self, port: u16) -> Result<Vec<LoadedModelState>, AppError> {
        let (status, body) = self.call(port, reqwest::Method::GET, MODELS_PATH, None)?;
        Self::expect_success("GET", MODELS_PATH, status, &body)?;
        // The payload shape and its mapping to `LoadedModelState` are T-040's,
        // already read off this build and tested there: a failed load is
        // `unloaded` plus `failed: true` and the child's exit code, and the load
        // state lives in `status.value`.
        supervisor::parse_router_models(&body)
    }

    fn load(&self, port: u16, model: &str) -> Result<(), AppError> {
        let (status, body) = self.call(
            port,
            reqwest::Method::POST,
            LOAD_PATH,
            Some(serde_json::json!({ "model": model })),
        )?;
        Self::expect_success("POST", LOAD_PATH, status, &body)
    }

    fn unload(&self, port: u16, model: &str) -> Result<(), AppError> {
        let (status, body) = self.call(
            port,
            reqwest::Method::POST,
            UNLOAD_PATH,
            Some(serde_json::json!({ "model": model })),
        )?;
        Self::expect_success("POST", UNLOAD_PATH, status, &body)
    }

    fn props(&self, port: u16) -> Result<ServerProps, AppError> {
        let (status, body) = self.call(port, reqwest::Method::GET, PROPS_PATH, None)?;
        Self::expect_success("GET", PROPS_PATH, status, &body)?;
        let raw: serde_json::Value =
            serde_json::from_str(&body).map_err(|err| AppError::Network {
                message: format!("GET {PROPS_PATH} did not answer JSON: {err}"),
            })?;
        Ok(ServerProps {
            build_tag: raw
                .get("build_info")
                .and_then(|value| value.as_str())
                .map(str::to_string),
            models_max: raw
                .get("max_instances")
                .and_then(|value| value.as_u64())
                .map(|value| value as u32),
            raw,
        })
    }
}

// ─── Where the upstream port comes from ─────────────────────────

/// The port the coordinator is listening on, as the supervisor knows it.
/// `None` when there is no server: Stopped, Stopping, or Crashed.
pub trait UpstreamPort: Send + Sync {
    fn upstream_port(&self) -> Result<Option<u16>, AppError>;
}

/// The production source: the actor that owns `ServerState` (invariant 2 — the
/// port is asked for over the channel, never read from shared memory).
pub struct SupervisorPort;

impl UpstreamPort for SupervisorPort {
    fn upstream_port(&self) -> Result<Option<u16>, AppError> {
        Ok(match supervisor::handle()?.state()? {
            ServerState::Starting { upstream_port, .. }
            | ServerState::Running { upstream_port, .. } => Some(upstream_port),
            ServerState::Stopped | ServerState::Stopping | ServerState::Crashed { .. } => None,
        })
    }
}

// ─── Events ─────────────────────────────────────────────────────

/// Where the orchestrator's state changes go. `docs/CONTRACTS.md` §4:
/// `model-load-state-changed` carries the whole `Vec<LoadedModelState>`, so a
/// screen renders a coherent snapshot rather than replaying deltas.
/// Implemented in `ipc/` as the Tauri event; `core/` never imports `tauri`.
pub trait OrchestratorEvents: Send + Sync {
    fn model_load_state_changed(&self, models: &[LoadedModelState]);
}

// ─── Tuning ─────────────────────────────────────────────────────

/// Every timer this component has, in one place.
#[derive(Clone, Debug)]
pub struct Tuning {
    /// Poll interval while a coordinator is answering — the same 2 s
    /// `docs/CONTRACTS.md` §2 uses for its own probes.
    pub poll_interval: Duration,
    /// Poll interval while a load is being waited on. 500 ms: a load that takes
    /// minutes still reports progress, and a load that takes a second is not
    /// reported late.
    pub load_poll_interval: Duration,
    /// A load attempt is abandoned after this long, with a typed error. The same
    /// budget as §2's `preload_timeout_seconds` (900 s): on this machine a 65 GB
    /// preload is minutes, not seconds.
    pub load_timeout: Duration,
    /// An unload that does not clear its model in this window is reported as a
    /// failure rather than waited on forever.
    pub unload_timeout: Duration,
    /// Backoff ceiling for repeated poll failures.
    pub backoff_ceiling: Duration,
    /// The ceiling is the `probe_strikes`-th failure's interval; see
    /// [`backoff_interval`]. The same 3 as §2's rule that one dropped response is
    /// not a death.
    pub probe_strikes: u32,
    /// One HTTP call's own timeout.
    pub probe_timeout: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            poll_interval: Duration::from_secs(2),
            load_poll_interval: Duration::from_millis(500),
            load_timeout: Duration::from_secs(900),
            unload_timeout: Duration::from_secs(60),
            backoff_ceiling: Duration::from_secs(60),
            probe_strikes: 3,
            probe_timeout: Duration::from_secs(5),
        }
    }
}

/// Interval after `failures` consecutive poll failures: the base doubled per
/// failure, clamped at the ceiling. Bounded by construction — the ceiling is a
/// value in [`Tuning`], not a comment.
pub(crate) fn backoff_interval(base: Duration, failures: u32, ceiling: Duration) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    let scaled = base.saturating_mul(1u32 << shift);
    scaled.min(ceiling)
}

// ─── The router implementation ──────────────────────────────────

/// Everything [`RouterOrchestrator`] talks to, in one place so the actor and the
/// handle share one owner of the seams.
pub struct OrchestratorDeps {
    pub api: Arc<dyn RouterApi>,
    pub directory: Arc<dyn ModelDirectory>,
    pub history: Arc<dyn LaunchHistorySink>,
    pub ports: Arc<dyn UpstreamPort>,
    pub events: Arc<dyn OrchestratorEvents>,
    pub tuning: Tuning,
    /// The last snapshot that was published to `events`, so a poll that changes
    /// nothing emits nothing. Not `ServerState`: invariant 2 is about the state
    /// the supervisor owns, and this is a cache of what the upstream said.
    pub published: Mutex<Vec<LoadedModelState>>,
}

impl OrchestratorDeps {
    pub fn new(
        api: Arc<dyn RouterApi>,
        directory: Arc<dyn ModelDirectory>,
        history: Arc<dyn LaunchHistorySink>,
        ports: Arc<dyn UpstreamPort>,
        events: Arc<dyn OrchestratorEvents>,
        tuning: Tuning,
    ) -> Self {
        OrchestratorDeps {
            api,
            directory,
            history,
            ports,
            events,
            tuning,
            published: Mutex::new(Vec::new()),
        }
    }
}

/// The v1 implementation of [`ModelOrchestrator`] (`docs/CONTRACTS.md` §5).
pub struct RouterOrchestrator {
    deps: Arc<OrchestratorDeps>,
}

impl RouterOrchestrator {
    pub fn new(deps: Arc<OrchestratorDeps>) -> Self {
        RouterOrchestrator { deps }
    }

    /// The current upstream port, or `UpstreamUnavailable` naming the state that
    /// made the call impossible.
    fn port(&self) -> Result<u16, AppError> {
        match self.deps.ports.upstream_port()? {
            Some(port) => Ok(port),
            None => {
                let state = supervisor::handle()
                    .and_then(|handle| handle.state())
                    .map(|state| supervisor::state_label(&state).to_string())
                    .unwrap_or_else(|_| "no server".to_string());
                Err(AppError::UpstreamUnavailable { state })
            }
        }
    }

    /// Publish `models` as the truth if it differs from the last thing
    /// published. Every poll and every load/unload step goes through here, so
    /// the screen sees one coherent sequence and no duplicate events.
    fn publish(&self, models: Vec<LoadedModelState>) {
        let Ok(mut published) = self.deps.published.lock() else {
            tracing::warn!("the orchestrator's snapshot lock is poisoned; skipping this publish");
            return;
        };
        if !states_differ(&published, &models) {
            return;
        }
        *published = models.clone();
        self.deps.events.model_load_state_changed(&models);
    }

    /// The last published snapshot. Never blocks on the network: this is what a
    /// screen reads, and the poller keeps it within one interval of the server.
    pub fn snapshot(&self) -> Vec<LoadedModelState> {
        self.deps
            .published
            .lock()
            .map(|published| published.clone())
            .unwrap_or_default()
    }

    /// One poll: read the upstream's model list and publish it. With no server
    /// running this publishes an empty list — nothing is loaded, because there
    /// is nothing to load — and is not an error; a *failing* upstream is.
    pub fn sync_once(&self) -> Result<(), AppError> {
        let Some(port) = self.deps.ports.upstream_port()? else {
            self.publish(Vec::new());
            return Ok(());
        };
        let models = self.deps.api.models(port)?;
        self.publish(models);
        Ok(())
    }

    /// Read `/props` once per upstream and log what the server says it is.
    /// Written into the app's log rather than a screen: it is how the app's own
    /// log answers "which build is this, and is it really a router", and T-045's
    /// dashboard is the screen that will render it.
    fn observe_props(&self, port: u16) {
        match self.deps.api.props(port) {
            Ok(props) => {
                let role = props
                    .raw
                    .get("role")
                    .and_then(|role| role.as_str())
                    .unwrap_or("(none)");
                if role == "router" {
                    tracing::info!(
                        port,
                        build_tag = props.build_tag.as_deref().unwrap_or("(unknown)"),
                        models_max = props.models_max.unwrap_or(0),
                        "upstream is a router"
                    );
                } else {
                    tracing::warn!(
                        port,
                        role,
                        "the upstream does not report the router role; model load state may not be per-model"
                    );
                }
            }
            Err(err) => tracing::debug!(port, "could not read /props: {err}"),
        }
    }

    fn record(&self, attempt: &LaunchAttempt<'_>) {
        // A history write that fails must not turn a successful load into a
        // failed one: the server is running and the user can use it. The failure
        // is logged, and the calibration path simply does without the row.
        if let Err(err) = self.deps.history.record(attempt) {
            tracing::warn!(
                path = %attempt.file_path.display(),
                "could not write the launch_history row for this attempt: {err}"
            );
        }
    }

    fn params_json(model: &OrchestratedModel) -> String {
        serde_json::to_string(&model.params).unwrap_or_else(|err| {
            // `LaunchParams` is a plain serde struct; if it ever stops being
            // serializable, `{}` keeps the row readable instead of losing it.
            tracing::warn!("could not serialize the launch parameters: {err}");
            "{}".to_string()
        })
    }

    /// The synchronous core of [`ModelOrchestrator::load`]: everything the
    /// boxed-future method does, with nothing to await.
    pub fn load_blocking(
        &self,
        model_id: &str,
    ) -> Result<crate::core::types::LoadOutcome, AppError> {
        let model = self.deps.directory.find(model_id)?;
        let port = self.port()?;

        // A model the server already holds needs no attempt and gets no history
        // row: `POST /models/load` answers `400 model is already running`, and
        // recording a launch that never happened would put a row in the
        // calibration set that no load produced.
        let before = self.deps.api.models(port)?;
        self.publish(before.clone());
        if let Some(state) = find_model(&before, &model.name) {
            if state.state == ModelLoadState::Loaded {
                tracing::info!(model = %model.name, "model is already loaded; nothing to do");
                return Ok(crate::core::types::LoadOutcome {
                    load_seconds: 0.0,
                    vram_bytes: state.vram_bytes,
                });
            }
        }

        let started = Instant::now();
        let attempted_at = Utc::now();
        let params_json = Self::params_json(&model);

        if let Err(err) = self.deps.api.load(port, &model.name) {
            let message = err.to_string();
            self.record(&LaunchAttempt {
                file_path: &model.file_path,
                params_json,
                attempted_at,
                succeeded: false,
                load_seconds: Some(started.elapsed().as_secs_f64()),
                actual_vram_bytes: None,
                error_message: Some(message.as_str()),
            });
            return Err(err);
        }

        let deadline = Instant::now() + self.deps.tuning.load_timeout;
        let mut consecutive_probe_errors: u32 = 0;

        loop {
            if Instant::now() >= deadline {
                let message = format!(
                    "the model {} did not finish loading within {}s",
                    model.name,
                    self.deps.tuning.load_timeout.as_secs()
                );
                self.record_failure(&model, &params_json, attempted_at, started, &message);
                return Err(AppError::Network { message });
            }
            thread::sleep(self.deps.tuning.load_poll_interval);

            let models = match self.deps.api.models(port) {
                Ok(models) => {
                    consecutive_probe_errors = 0;
                    models
                }
                Err(err) => {
                    consecutive_probe_errors += 1;
                    if consecutive_probe_errors >= self.deps.tuning.probe_strikes {
                        let message = format!(
                            "the server stopped answering while the model {} was loading: {err}",
                            model.name
                        );
                        self.record_failure(&model, &params_json, attempted_at, started, &message);
                        return Err(AppError::Network { message });
                    }
                    continue;
                }
            };

            // Publishing here is what "a slow load emits progress rather than
            // blocking" means: every state the server reports reaches the UI as
            // it happens, from a call that has not returned yet.
            self.publish(models.clone());

            let Some(state) = find_model(&models, &model.name) else {
                consecutive_probe_errors += 1;
                if consecutive_probe_errors >= self.deps.tuning.probe_strikes {
                    let message = format!(
                        "the server stopped listing the model {} while it was loading",
                        model.name
                    );
                    self.record_failure(&model, &params_json, attempted_at, started, &message);
                    return Err(AppError::Network { message });
                }
                continue;
            };

            match state.state {
                ModelLoadState::Loaded => {
                    let load_seconds = started.elapsed().as_secs_f64();
                    self.record(&LaunchAttempt {
                        file_path: &model.file_path,
                        params_json,
                        attempted_at,
                        succeeded: true,
                        load_seconds: Some(load_seconds),
                        actual_vram_bytes: state.vram_bytes,
                        error_message: None,
                    });
                    tracing::info!(
                        model = %model.name,
                        load_seconds,
                        "model loaded"
                    );
                    return Ok(crate::core::types::LoadOutcome {
                        load_seconds,
                        vram_bytes: state.vram_bytes,
                    });
                }
                ModelLoadState::Failed => {
                    // The server's own message, never a canned one: on this build
                    // a failed load returns the model to `unloaded` and carries
                    // the child's exit code, which `parse_router_models` renders
                    // as the error string.
                    let reason = state.error.clone().unwrap_or_else(|| {
                        format!("the server reported the load of {} failed", model.name)
                    });
                    self.record_failure(&model, &params_json, attempted_at, started, &reason);
                    return Err(AppError::Network { message: reason });
                }
                // Loading, or anything else the build answers: keep waiting.
                _ => {}
            }
        }
    }

    fn record_failure(
        &self,
        model: &OrchestratedModel,
        params_json: &str,
        attempted_at: chrono::DateTime<Utc>,
        started: Instant,
        message: &str,
    ) {
        self.record(&LaunchAttempt {
            file_path: &model.file_path,
            params_json: params_json.to_string(),
            attempted_at,
            succeeded: false,
            load_seconds: Some(started.elapsed().as_secs_f64()),
            actual_vram_bytes: None,
            error_message: Some(message),
        });
    }

    /// The synchronous core of [`ModelOrchestrator::props`].
    pub fn props_blocking(&self) -> Result<ServerProps, AppError> {
        let port = self.port()?;
        self.deps.api.props(port)
    }

    /// The synchronous core of [`ModelOrchestrator::unload`].
    pub fn unload_blocking(&self, model_id: &str) -> Result<(), AppError> {
        let model = self.deps.directory.find(model_id)?;
        let port = self.port()?;

        let before = self.deps.api.models(port)?;
        self.publish(before.clone());
        let Some(state) = find_model(&before, &model.name) else {
            return Err(AppError::NotFound {
                what: format!(
                    "model {} on the running server — reload the server so the preset includes it",
                    model.name
                ),
            });
        };
        if state.state != ModelLoadState::Loaded {
            tracing::info!(model = %model.name, "model is not loaded; nothing to unload");
            return Ok(());
        }

        if let Err(err) = self.deps.api.unload(port, &model.name) {
            // The server may have raced us — its own state is the authority, so
            // the refusal is only real if it still says the model is loaded.
            // Matching on the message string ("model is not running") would break
            // the moment the wording changes; asking the server does not.
            let now = self.deps.api.models(port)?;
            let still_loaded = find_model(&now, &model.name)
                .map(|state| state.state == ModelLoadState::Loaded)
                .unwrap_or(false);
            self.publish(now);
            if still_loaded {
                return Err(err);
            }
            return Ok(());
        }

        let deadline = Instant::now() + self.deps.tuning.unload_timeout;
        loop {
            if Instant::now() >= deadline {
                return Err(AppError::Network {
                    message: format!(
                        "the model {} was still loaded {}s after the server accepted the unload",
                        model.name,
                        self.deps.tuning.unload_timeout.as_secs()
                    ),
                });
            }
            thread::sleep(self.deps.tuning.load_poll_interval);
            let models = self.deps.api.models(port)?;
            let loaded = find_model(&models, &model.name)
                .map(|state| state.state == ModelLoadState::Loaded)
                .unwrap_or(false);
            self.publish(models);
            if !loaded {
                tracing::info!(model = %model.name, "model unloaded");
                return Ok(());
            }
        }
    }
}

impl ModelOrchestrator for RouterOrchestrator {
    fn list_models(&self) -> OrchestratorFuture<'_, Vec<LoadedModelState>> {
        Box::pin(async move {
            let port = self.port()?;
            let models = self.deps.api.models(port)?;
            self.publish(models.clone());
            Ok(models)
        })
    }

    fn load<'a>(&'a self, model_id: &'a str) -> OrchestratorFuture<'a, LoadOutcome> {
        Box::pin(async move { self.load_blocking(model_id) })
    }

    fn unload<'a>(&'a self, model_id: &'a str) -> OrchestratorFuture<'a, ()> {
        Box::pin(async move { self.unload_blocking(model_id) })
    }

    fn props(&self) -> OrchestratorFuture<'_, ServerProps> {
        Box::pin(async move { self.props_blocking() })
    }
}

/// The no-op implementation: the seam `PLAN.md` §2.3 keeps for the `llama-swap`
/// fallback, and the proof that [`ModelOrchestrator`] is a trait rather than a
/// description of [`RouterOrchestrator`].
///
/// Under an orchestrator that has no per-model control, every model the server
/// was started with is available and none can be loaded or unloaded on demand —
/// so `list_models` answers nothing (the app's own catalogue is the list that
/// matters there) and the two commands are accepted no-ops rather than errors:
/// a fallback that refused them would make a screen fail for a server that is
/// working exactly as configured.
#[derive(Default)]
pub struct NoopOrchestrator;

impl ModelOrchestrator for NoopOrchestrator {
    fn list_models(&self) -> OrchestratorFuture<'_, Vec<LoadedModelState>> {
        Box::pin(async { Ok(Vec::new()) })
    }

    fn load<'a>(
        &'a self,
        _model_id: &'a str,
    ) -> OrchestratorFuture<'a, crate::core::types::LoadOutcome> {
        Box::pin(async {
            Ok(LoadOutcome {
                load_seconds: 0.0,
                vram_bytes: None,
            })
        })
    }

    fn unload<'a>(&'a self, _model_id: &'a str) -> OrchestratorFuture<'a, ()> {
        Box::pin(async { Ok(()) })
    }

    fn props(&self) -> OrchestratorFuture<'_, ServerProps> {
        Box::pin(async {
            Ok(ServerProps {
                build_tag: None,
                models_max: None,
                raw: serde_json::Value::Null,
            })
        })
    }
}

// ─── The poller and the handle ──────────────────────────────────

/// The handle IPC holds.
///
/// Unlike the supervisor this is not an actor with a command channel: the state
/// it holds is a cache of what the upstream said, not `ServerState`, and every
/// method either reads that cache or blocks on the server. Commands from several
/// Tauri threads are therefore concurrent by construction — the only shared
/// value is the published snapshot, behind its own mutex.
pub struct OrchestratorHandle {
    inner: Arc<RouterOrchestrator>,
    /// The poller's stop flag. Read by the poller thread — which `spawn` clones
    /// it into — and by `shutdown`. No production caller shuts the poller down
    /// yet: the app's boundary is process exit, and T-047's quit sequence is the
    /// caller that will drain it before the process ends. Unused-in-production
    /// is why it carries the allow.
    #[allow(dead_code)]
    stop: Arc<AtomicBool>,
}

impl OrchestratorHandle {
    /// The last published snapshot — no HTTP, no blocking.
    pub fn snapshot(&self) -> Vec<LoadedModelState> {
        self.inner.snapshot()
    }

    pub fn load(&self, model_id: &str) -> Result<crate::core::types::LoadOutcome, AppError> {
        self.inner.load_blocking(model_id)
    }

    pub fn unload(&self, model_id: &str) -> Result<(), AppError> {
        self.inner.unload_blocking(model_id)
    }

    /// Ask the poller thread to finish. Called by the tests today; the app's own
    /// process exit is the production boundary, and T-047's quit sequence is the
    /// caller that will use this for real.
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// True while the poller is still running — used by the tests that assert
    /// the shutdown actually lands.
    #[allow(dead_code)]
    pub fn poller_running(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
    }
}

/// Start the poller thread and hand back its handle.
pub fn spawn(deps: Arc<OrchestratorDeps>) -> OrchestratorHandle {
    let orchestrator = Arc::new(RouterOrchestrator::new(deps));
    let stop = Arc::new(AtomicBool::new(false));
    let worker = Arc::clone(&orchestrator);
    let worker_stop = Arc::clone(&stop);
    if let Err(err) = thread::Builder::new()
        .name("orchestrator".to_string())
        .spawn(move || poll(worker, worker_stop))
    {
        tracing::error!("could not start the orchestrator poller thread: {err}");
    }
    OrchestratorHandle {
        inner: orchestrator,
        stop,
    }
}

/// The poll loop: one tick per interval, doubling on consecutive failures up to
/// the ceiling, and back to the base interval as soon as a poll succeeds.
fn poll(orchestrator: Arc<RouterOrchestrator>, stop: Arc<AtomicBool>) {
    let tuning = orchestrator.deps.tuning.clone();
    let mut failures: u32 = 0;
    let mut last_port: Option<u16> = None;

    while !stop.load(Ordering::Relaxed) {
        let port = orchestrator.deps.ports.upstream_port().ok().flatten();
        if let Some(port) = port {
            if last_port != Some(port) {
                orchestrator.observe_props(port);
            }
        }
        last_port = port;

        let interval = match orchestrator.sync_once() {
            Ok(()) => {
                failures = 0;
                tuning.poll_interval
            }
            Err(err) => {
                failures += 1;
                let interval =
                    backoff_interval(tuning.poll_interval, failures, tuning.backoff_ceiling);
                tracing::warn!(
                    failures,
                    retry_in_ms = interval.as_millis() as u64,
                    "could not read the loaded models from the upstream: {err}"
                );
                interval
            }
        };

        sleep_until(&stop, interval);
    }
}

/// Sleep in short slices so a shutdown does not wait out a whole backoff.
fn sleep_until(stop: &AtomicBool, interval: Duration) {
    let deadline = Instant::now() + interval;
    while !stop.load(Ordering::Relaxed) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
        }
        thread::sleep(remaining.min(Duration::from_millis(50)));
    }
}

fn find_model<'a>(models: &'a [LoadedModelState], name: &str) -> Option<&'a LoadedModelState> {
    models.iter().find(|model| model.model_id == name)
}

/// Do two snapshots differ for the purpose of the event stream? Compared field
/// by field rather than by `==` because `LoadedModelState` (§1) does not derive
/// `PartialEq` and adding the derive to a contract-listed type is not this
/// task's to do.
fn states_differ(left: &[LoadedModelState], right: &[LoadedModelState]) -> bool {
    left.len() != right.len()
        || left.iter().zip(right).any(|(left, right)| {
            left.model_id != right.model_id
                || left.state != right.state
                || left.vram_bytes != right.vram_bytes
                || left.error != right.error
        })
}

// ─── The install-time handle ────────────────────────────────────

static ORCHESTRATOR: OnceLock<OrchestratorHandle> = OnceLock::new();

/// Install the handle produced by [`spawn`]. Called once, from `ipc/`.
pub fn install(handle: OrchestratorHandle) -> Result<(), AppError> {
    ORCHESTRATOR.set(handle).map_err(|_| AppError::Internal {
        message: "the model orchestrator was installed twice".to_string(),
    })
}

/// The handle IPC talks to. Fails loudly rather than answering an empty list:
/// a screen that shows "no models loaded" because the orchestrator is missing is
/// worse than an error the user can report.
pub fn handle() -> Result<&'static OrchestratorHandle, AppError> {
    ORCHESTRATOR.get().ok_or_else(|| AppError::Internal {
        message: "the model orchestrator is not running".to_string(),
    })
}

/// The production wiring: the real HTTP client, the registry's catalogue, the
/// app's database, the supervisor's port and the Tauri event sink.
pub fn production_deps(
    events: Arc<dyn OrchestratorEvents>,
    tuning: Tuning,
) -> Result<Arc<OrchestratorDeps>, AppError> {
    Ok(Arc::new(OrchestratorDeps::new(
        Arc::new(RouterHttp::new(tuning.probe_timeout)),
        Arc::new(RegistryDirectory),
        Arc::new(DbLaunchHistory::production()?),
        Arc::new(SupervisorPort),
        events,
        tuning,
    )))
}

// ─── The read side of `launch_history` ──────────────────────────

/// Every stored load attempt for `file_path`, as the estimator's own input type.
///
/// `docs/CONTRACTS.md` §3: "T-041 writes a record on every load attempt; T-032
/// reads them (as an argument) to calibrate. Without the writer the calibration
/// path is dead code." This is the reader half of that sentence, and it is
/// implemented here rather than in `ipc/` because it is the same table this
/// module owns the writer for.
///
/// A row whose `params_json` cannot be parsed is **skipped with a warning**, not
/// fatal: one unreadable row must not cost the estimate the rest of the history,
/// and it must never be turned into a plausible-looking default.
pub fn launch_records_for(file_path: &str) -> Result<Vec<LaunchRecord>, AppError> {
    let Some(db_path) = crate::core::installer::database_path() else {
        return Ok(Vec::new());
    };
    launch_records_at(&db_path, file_path)
}

/// The same read against a named database. Split out so the tests can prove it
/// against a temporary database rather than the installed one.
pub fn launch_records_at(db_path: &Path, file_path: &str) -> Result<Vec<LaunchRecord>, AppError> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }
    let conn = crate::db::open(db_path)?;
    let rows = crate::db::queries::list_launch_history_for_path(&conn, file_path)?;

    let mut records = Vec::with_capacity(rows.len());
    for row in rows {
        match serde_json::from_str::<LaunchParams>(&row.params_json) {
            Ok(params) => records.push(LaunchRecord {
                file_path: PathBuf::from(&row.file_path),
                launched_at: row.launched_at,
                params,
                succeeded: row.succeeded,
                actual_vram_bytes: row.actual_vram_bytes.map(|bytes| bytes as u64),
                load_seconds: row.load_seconds,
            }),
            Err(err) => tracing::warn!(
                path = %row.file_path,
                row = row.id,
                "skipping an unreadable launch_history row: {err}"
            ),
        }
    }
    // Newest first is the query's order; the estimator takes the most recent
    // matches by iterating in reverse, so the order is preserved as read.
    Ok(records)
}
