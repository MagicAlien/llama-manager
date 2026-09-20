//! `core::supervisor` — T-040: the actor that owns `ServerState`.
//!
//! `docs/CONTRACTS.md` §2 is the specification and this module is its only
//! implementation. The shape is the one the contract permits and no other:
//! **one actor task owns the state**, commands arrive over an `mpsc` channel
//! and await a reply, every change is broadcast as `server-state-changed`. No
//! shared `Mutex<ServerState>` (`AGENTS.md` invariant 2).
//!
//! # Why the loop never blocks on the process
//!
//! A supervisor that sat inside `start_server` until `Running` would be unable
//! to answer `get_server_state` during the only minutes the user is watching,
//! and unable to honour `stop_server` while a start was in flight. So the actor
//! is an event loop with deadlines, not a sequence of calls: a command is
//! applied as soon as it is read and the reply carries the state *after* the
//! application, while the long work — waiting for a coordinator to answer, for
//! preloads to finish, for a child to die — is done by deadline checks in the
//! loop. `start_server` therefore answers with `Starting { WaitingForProcess }`
//! in the time it takes to write the preset and spawn the process, and the rest
//! of the startup arrives as state changes.
//!
//! # Phases, timeouts and the 3-strike rule
//!
//! Startup has two phases with two timeouts (§2): the coordinator answering is
//! seconds (`process_timeout_seconds`, default 120), preloading a 65 GB model
//! is minutes (`preload_timeout_seconds`, default 900). One timeout would have
//! to be uselessly loose for the first. The preload set is the catalogue's
//! `preload` flag; when it is empty, the coordinator answering takes the state
//! **straight to `Running`** — the shortcut §2 names.
//!
//! A failed probe is not a death. While `WaitingForProcess` every probe fails
//! until the coordinator starts answering: that is what the phase *means*, and
//! it is bounded by `process_timeout_seconds`, not by a strike count. Once the
//! coordinator has answered, a probe that fails three times in a row is the
//! only signal left that a server which is still in the process table has
//! stopped doing its job — including probes that exceed their own 5 s timeout.
//! Three consecutive failures reach `Crashed`; one dropped response does not
//! (§2, "What \"the health check passes\" means").
//!
//! # Everything external is a seam
//!
//! The database, the process launch, the HTTP probes and the event sink are
//! four traits, so the whole state machine is exercised against scripted
//! doubles (`AGENTS.md` §3: no criterion may need the target hardware), while
//! the real implementations are separately driven against a real process tree
//! and — env-gated — against the installed build. The tests live in
//! `core/supervisor_tests.rs` and drive the actor through its public handle,
//! which is also how the IPC layer drives it.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::core::installer;
use crate::core::model_paths;
use crate::core::model_registry;
use crate::core::preset_generator::{self, PresetWarning};
use crate::core::process::{
    self, LogLine, ManagedChild, PortAllocator, ProcessLauncher, ServerLaunch, SystemLauncher,
    SystemPortAllocator,
};
use crate::core::types::{
    AppError, Diagnosis, LoadedModelState, ModelEntry, ModelLoadState, RegistrationChannel,
    RuntimeBuild, ServerConfig, ServerState, StartupPhase,
};

/// The directory, under the app's data root, that the `ScanOnly` registration
/// channel points the router at (`PLAN.md` §2.1: links live under the app's own
/// data directory). The `PresetDeclaresPath` channel never uses it.
pub const ROUTER_MODELS_DIR: &str = "router-models";

/// The file the router is told to read presets from, under the app's data root.
pub const PRESET_FILE: &str = "presets.ini";

/// The `settings` key T-050 will write and this module reads.
pub const SERVER_CONFIG_KEY: &str = "server_config";

/// Every timer this component has, in one place, with the values
/// `docs/CONTRACTS.md` §2 fixes. Poll intervals, the probe timeout and the stop
/// grace are the contract's, not this module's invention; the fields are public
/// so tests can shrink them, and nothing user-facing can (`ServerConfig`
/// carries no poll interval — a screen must not be able to turn the health
/// check into a hot loop).
#[derive(Clone, Debug)]
pub struct Tuning {
    /// §2: 500 ms while `WaitingForProcess` — fast enough that start feedback
    /// stays inside the 2-second accuracy the Definition of Done asks for.
    pub waiting_probe_interval: Duration,
    /// §2: 2 s while `Preloading`.
    pub preloading_probe_interval: Duration,
    /// Health-check interval while `Running`: §2 fixes the two above, and this
    /// is the same 2 s the preload poll uses, for the same reason.
    pub running_probe_interval: Duration,
    /// §2: a probe's own timeout, counted as a failure when it expires.
    pub probe_timeout: Duration,
    /// §2: the `Stopping` grace window, shared with T-047's quit sequence.
    pub stop_grace: Duration,
    /// How long to wait for a process to die *after* the tree kill before
    /// concluding the stop while reporting anything still standing.
    pub kill_wait: Duration,
    /// `ServerState::Crashed.last_log` carries this many lines of tail.
    pub log_ring_capacity: usize,
    /// How long the actor keeps reading a child's output once its exit has been
    /// observed, before the tail is frozen into `Crashed`.
    ///
    /// A process writes its last words and *then* dies, so the lines that
    /// explain a crash are in flight exactly when the exit becomes visible —
    /// both in the actor (the loop drains before it looks, `check_exit` looks
    /// after it drains) and on the real pipe (the reader thread delivers the
    /// final bytes asynchronously). Without this window the crash state carries
    /// the app's own lines and none of the child's, which is the tail T-043
    /// reads a diagnosis from.
    pub final_drain: Duration,
}

impl Default for Tuning {
    fn default() -> Self {
        Tuning {
            waiting_probe_interval: Duration::from_millis(500),
            preloading_probe_interval: Duration::from_secs(2),
            running_probe_interval: Duration::from_secs(2),
            probe_timeout: Duration::from_secs(5),
            stop_grace: Duration::from_secs(10),
            kill_wait: Duration::from_secs(5),
            log_ring_capacity: 500,
            final_drain: Duration::from_millis(50),
        }
    }
}

/// The two startup deadlines, derived from stored configuration so that a
/// change through `set_server_config` (T-050) takes effect at the next start,
/// and injectable so a test can prove they are honoured independently without
/// waiting fifteen minutes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeouts {
    pub process: Duration,
    pub preload: Duration,
}

impl Timeouts {
    pub fn from_config(config: &ServerConfig) -> Self {
        Timeouts {
            process: Duration::from_secs(u64::from(config.process_timeout_seconds)),
            preload: Duration::from_secs(u64::from(config.preload_timeout_seconds)),
        }
    }
}

/// What the supervisor needs from the database, and the one place it writes to
/// disk on the server's behalf.
///
/// The supervisor never opens the database itself: it asks for a decision's
/// inputs, which is what lets every state-machine test run with no database at
/// all.
pub trait ServerContext: Send + Sync {
    fn server_config(&self) -> Result<ServerConfig, AppError>;
    /// The build `start_server` will spawn. `NotFound` when none is active —
    /// starting with no build installed is not a transition to reject, it is a
    /// precondition the user can see in the UI.
    fn active_build(&self) -> Result<RuntimeBuild, AppError>;
    fn models(&self) -> Result<Vec<ModelEntry>, AppError>;
    /// Write the registration the chosen channel needs and report what the
    /// generator left out, so the supervisor can emit `preset-warning`.
    fn prepare_registration(
        &self,
        build: &RuntimeBuild,
        models: &[ModelEntry],
    ) -> Result<Registration, AppError>;
}

/// The result of `prepare_registration`. Both paths are `Option` because which
/// one is used is the registration channel's business, not the supervisor's:
/// `router_arguments` picks the one it needs and refuses when it is absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub preset_path: Option<PathBuf>,
    pub models_dir: Option<PathBuf>,
    pub warnings: Vec<PresetWarning>,
}

/// The app's own calls to the coordinator. Not the endpoint listener's path:
/// this is the control channel (`docs/CONTRACTS.md` §5's distinction).
///
/// **An implementation must bound its own call** by [`Tuning::probe_timeout`]
/// and return `Err` — blocking is a failure the supervisor cannot otherwise
/// see. The actor also times each probe out on its own side, so a stuck
/// implementation degrades into counted failures instead of a dead supervisor.
pub trait Coordinator: Send + Sync {
    /// `endpoint` is the one T-023 recorded for the active build (`/health`
    /// where it exists, otherwise `/props`); success is any 2xx.
    fn probe_health(&self, port: u16, endpoint: &str) -> Result<(), AppError>;
    /// The router's `/models` — richer than `/v1/models`, and the only place a
    /// model's load state is visible.
    fn loaded_models(&self, port: u16) -> Result<Vec<LoadedModelState>, AppError>;
}

/// The real coordinator: HTTP over loopback, with the probe timeout built into
/// the client.
///
/// It owns a single-threaded Tokio runtime because it is driven from the probe
/// worker thread, which is a plain `std::thread` and not on Tauri's runtime.
/// `block_on` from inside another runtime would panic, which is why this type is
/// only ever called from that worker.
pub struct HttpCoordinator {
    runtime: tokio::runtime::Runtime,
    client: reqwest::Client,
}

impl HttpCoordinator {
    pub fn new(probe_timeout: Duration) -> Result<Self, AppError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| AppError::Internal {
                message: format!("could not start the supervisor's HTTP runtime: {err}"),
            })?;
        let client = reqwest::Client::builder()
            .timeout(probe_timeout)
            .build()
            .map_err(|err| AppError::Internal {
                message: format!("could not build the supervisor's HTTP client: {err}"),
            })?;
        Ok(HttpCoordinator { runtime, client })
    }

    fn url(port: u16, path: &str) -> String {
        let path = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("/{path}")
        };
        format!("http://127.0.0.1:{port}{path}")
    }
}

impl Coordinator for HttpCoordinator {
    fn probe_health(&self, port: u16, endpoint: &str) -> Result<(), AppError> {
        let url = Self::url(port, endpoint);
        self.runtime.block_on(async {
            let response = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|err| AppError::Network {
                    message: format!("{url}: {err}"),
                })?;
            if response.status().is_success() {
                Ok(())
            } else {
                Err(AppError::Network {
                    message: format!("{url} answered {}", response.status()),
                })
            }
        })
    }

    fn loaded_models(&self, port: u16) -> Result<Vec<LoadedModelState>, AppError> {
        let url = Self::url(port, "/models");
        self.runtime.block_on(async {
            let response = self
                .client
                .get(&url)
                .send()
                .await
                .map_err(|err| AppError::Network {
                    message: format!("{url}: {err}"),
                })?;
            let status = response.status();
            let body = response.text().await.map_err(|err| AppError::Network {
                message: format!("{url}: {err}"),
            })?;
            if !status.is_success() {
                return Err(AppError::Network {
                    message: format!("{url} answered {status}"),
                });
            }
            parse_router_models(&body)
        })
    }
}

/// Read the router's `/models` payload into the contract's `LoadedModelState`.
///
/// The shape is not guessed: it was captured from the installed b10883 build
/// (`PROGRESS.md` F-020) and cross-checked against the source at that build's
/// own commit (`tools/server/server-models.h`,
/// `server_model_status_to_string` and `server_model_meta::is_failed`), which is
/// where the two facts that matter come from — the status vocabulary is
/// `downloading | downloaded | unloaded | loading | loaded | sleeping`, and a
/// **failed** load is not a status of its own: the model goes back to
/// `unloaded` and carries `"failed": true` with the child's `exit_code`.
///
/// An unrecognised status is reported as `Registered`: not loaded (so nothing is
/// counted as ready on the strength of a string this app does not know) and not
/// failed (so a writer's new vocabulary cannot fake a failure either).
/// `sleeping` counts as `Loaded` because the weights are still resident — the
/// router's own `is_ready_or_sleep()` draws the line in the same place.
pub fn parse_router_models(body: &str) -> Result<Vec<LoadedModelState>, AppError> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|err| AppError::Network {
        message: format!("the router's /models response was not JSON: {err}"),
    })?;
    let entries = value
        .get("data")
        .and_then(|data| data.as_array())
        .ok_or_else(|| AppError::Network {
            message: "the router's /models response has no `data` array".to_string(),
        })?;

    let mut models = Vec::with_capacity(entries.len());
    for entry in entries {
        let Some(id) = entry.get("id").and_then(|id| id.as_str()) else {
            continue;
        };
        let status = entry.get("status").cloned().unwrap_or_default();
        let value_str = status
            .get("value")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");
        let failed = status
            .get("failed")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false);
        let exit_code = status.get("exit_code").and_then(|code| code.as_i64());

        let (state, error) = if failed {
            (
                ModelLoadState::Failed,
                Some(match exit_code {
                    Some(code) => format!("the server reported the load failed (exit code {code})"),
                    None => "the server reported the load failed".to_string(),
                }),
            )
        } else {
            match value_str {
                "loaded" | "sleeping" => (ModelLoadState::Loaded, None),
                "loading" | "downloading" | "downloaded" => (ModelLoadState::Loading, None),
                "unloaded" => (ModelLoadState::Registered, None),
                other => {
                    tracing::warn!(
                        "model {id} reports an unknown load status {other:?}; \
                         treating it as not loaded"
                    );
                    (ModelLoadState::Registered, None)
                }
            }
        };

        models.push(LoadedModelState {
            model_id: id.to_string(),
            state,
            vram_bytes: status
                .get("vram_bytes")
                .and_then(|bytes| bytes.as_u64())
                .or_else(|| entry.get("vram_bytes").and_then(|bytes| bytes.as_u64())),
            last_used: None,
            error,
        });
    }
    Ok(models)
}

/// Where state changes and output lines go. Implemented in `main.rs` as Tauri
/// events; `core/` must not know about Tauri (`AGENTS.md` invariant 1).
pub trait SupervisorEvents: Send + Sync {
    fn state_changed(&self, state: &ServerState);
    fn log_line(&self, line: &LogLine);
    /// The runtime half of `AGENTS.md` §1: a flag the active build does not
    /// carry is omitted from the preset *and named*, so the omission is visible.
    fn preset_warning(&self, warning: &PresetWarning);
}

// ─── Commands and replies ───────────────────────────────────────

type Reply<T> = Sender<Result<T, AppError>>;

enum SupervisorCommand {
    Start(Reply<ServerState>),
    Stop(Reply<ServerState>),
    DismissCrash(Reply<ServerState>),
    GetState(Reply<ServerState>),
    MarkConfigDirty(Reply<()>),
}

/// The handle IPC holds. Every method blocks until the actor has applied the
/// command and replies — microseconds, except that `start` also writes the
/// preset and spawns the process before it answers.
///
/// The `Sender` lives behind a `Mutex` only because `std::sync::mpsc::Sender` is
/// `Send` but not `Sync`: IPC calls arrive on several threads at once, and the
/// lock is released before the call waits for its reply, so concurrent commands
/// are genuinely concurrent. The state is still reachable only through the
/// channel (`AGENTS.md` invariant 2).
pub struct SupervisorHandle {
    tx: Mutex<Sender<SupervisorCommand>>,
}

impl SupervisorHandle {
    /// `Stopped` → `Starting { WaitingForProcess }`, or from `Crashed`.
    /// `InvalidTransition` for every other state.
    pub fn start(&self) -> Result<ServerState, AppError> {
        self.call(SupervisorCommand::Start)
    }

    /// Legal from `Starting` (kills the partial process) and from `Running`.
    /// The reply is `Stopping`: the process may take the grace window to die and
    /// the caller is not made to wait for it.
    pub fn stop(&self) -> Result<ServerState, AppError> {
        self.call(SupervisorCommand::Stop)
    }

    /// `Crashed` → `Stopped`.
    pub fn dismiss_crash(&self) -> Result<ServerState, AppError> {
        self.call(SupervisorCommand::DismissCrash)
    }

    pub fn state(&self) -> Result<ServerState, AppError> {
        self.call(SupervisorCommand::GetState)
    }

    /// Marks `Running.config_dirty` — the restart banner's input. Its callers
    /// are the edits that change what a restart would regenerate
    /// (`update_model_params` in T-031, `set_server_config` in T-050); neither
    /// calls it yet, so it is `dead_code` until then. It lives here rather than
    /// in a screen because §2 puts the flag on the state the actor owns.
    #[allow(dead_code)]
    pub fn mark_config_dirty(&self) -> Result<(), AppError> {
        let (tx, rx) = mpsc::channel();
        self.send(SupervisorCommand::MarkConfigDirty(tx))?;
        rx.recv().map_err(|_| actor_gone())?
    }

    fn send(&self, command: SupervisorCommand) -> Result<(), AppError> {
        let tx = self.tx.lock().map_err(|_| actor_gone())?;
        tx.send(command).map_err(|_| actor_gone())
    }

    fn call<F>(&self, make: F) -> Result<ServerState, AppError>
    where
        F: FnOnce(Reply<ServerState>) -> SupervisorCommand,
    {
        let (tx, rx) = mpsc::channel();
        self.send(make(tx))?;
        // Outside the lock: two commands may be waiting for their replies at
        // the same time without either of them holding anything the other needs.
        rx.recv().map_err(|_| actor_gone())?
    }
}

fn actor_gone() -> AppError {
    AppError::Internal {
        message: "the server supervisor is not running".to_string(),
    }
}

/// The single supervisor instance the app uses. A `OnceLock<Sender<..>>` is not
/// a shared state object: the state itself lives in the actor's own thread and
/// is unreachable except by sending it a command (`AGENTS.md` invariant 2).
static SUPERVISOR: OnceLock<SupervisorHandle> = OnceLock::new();

/// Install the handle produced by [`spawn`]. Called once, from `main.rs`.
pub fn install(handle: SupervisorHandle) -> Result<(), AppError> {
    SUPERVISOR.set(handle).map_err(|_| AppError::Internal {
        message: "the server supervisor was installed twice".to_string(),
    })
}

/// The handle IPC talks to. Fails loudly rather than defaulting to a fake
/// `Stopped`: a screen that shows "stopped" because the supervisor is missing is
/// worse than an error the user can report.
pub fn handle() -> Result<&'static SupervisorHandle, AppError> {
    SUPERVISOR.get().ok_or_else(actor_gone)
}

/// Reject *this* command while the server is not `Stopped` (`docs/CONTRACTS.md`
/// §2, "Rejected commands"): activating or removing a runtime changes the files
/// the running process was started from.
pub fn reject_unless_stopped(state: &ServerState, command: &str) -> Result<(), AppError> {
    if matches!(state, ServerState::Stopped) {
        Ok(())
    } else {
        Err(AppError::InvalidTransition {
            from: state_label(state).to_string(),
            command: command.to_string(),
        })
    }
}

/// A short, stable name for a state — what `InvalidTransition.from` carries.
/// The state's own `Debug` includes timestamps and pids, which is not something
/// a user-facing message should read out.
pub fn state_label(state: &ServerState) -> &'static str {
    match state {
        ServerState::Stopped => "Stopped",
        ServerState::Starting { .. } => "Starting",
        ServerState::Running { .. } => "Running",
        ServerState::Stopping => "Stopping",
        ServerState::Crashed { .. } => "Crashed",
    }
}

// ─── Wiring the actor ───────────────────────────────────────────

pub struct SupervisorDeps {
    pub context: Arc<dyn ServerContext>,
    pub launcher: Arc<dyn ProcessLauncher>,
    pub coordinator: Arc<dyn Coordinator>,
    pub ports: Arc<dyn PortAllocator>,
    pub events: Arc<dyn SupervisorEvents>,
    pub tuning: Tuning,
    /// `None` in production (the deadlines come from `ServerConfig`), `Some` in
    /// tests so the two timeouts can be measured without waiting for them.
    pub timeouts_override: Option<Timeouts>,
}

/// Start the actor thread and hand back its handle.
pub fn spawn(deps: SupervisorDeps) -> SupervisorHandle {
    let (tx, rx) = mpsc::channel();
    let actor = Actor::new(deps);
    if let Err(err) = thread::Builder::new()
        .name("supervisor".to_string())
        .spawn(move || actor.run(rx))
    {
        tracing::error!("could not start the supervisor thread: {err}");
    }
    SupervisorHandle { tx: Mutex::new(tx) }
}

/// The default production wiring: the real launcher, the real HTTP coordinator,
/// the real port allocator, and the database behind `ServerContext`.
pub fn production_deps(
    events: Arc<dyn SupervisorEvents>,
    tuning: Tuning,
) -> Result<SupervisorDeps, AppError> {
    Ok(SupervisorDeps {
        context: Arc::new(DatabaseContext),
        launcher: Arc::new(SystemLauncher::new()),
        coordinator: Arc::new(HttpCoordinator::new(tuning.probe_timeout)?),
        ports: Arc::new(SystemPortAllocator),
        events,
        tuning,
        timeouts_override: None,
    })
}

/// The database side of the supervisor.
pub struct DatabaseContext;

fn database_path() -> Result<PathBuf, AppError> {
    installer::database_path().ok_or_else(|| AppError::Internal {
        message: "LOCALAPPDATA is not set; cannot locate the app database".to_string(),
    })
}

impl ServerContext for DatabaseContext {
    fn server_config(&self) -> Result<ServerConfig, AppError> {
        let path = database_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| AppError::Io {
                message: format!("could not create {}: {err}", parent.display()),
            })?;
        }
        let conn = crate::db::open(&path)?;
        match crate::db::queries::get_setting(&conn, SERVER_CONFIG_KEY)? {
            Some(json) => serde_json::from_str(&json).map_err(|err| AppError::Internal {
                message: format!(
                    "the stored server configuration is not readable ({err}); \
                     save it again from the API screen"
                ),
            }),
            // No row yet: the documented defaults, so a first start works before
            // the API screen (T-050) has ever been opened.
            None => Ok(default_server_config()),
        }
    }

    fn active_build(&self) -> Result<RuntimeBuild, AppError> {
        let conn = crate::db::open(&database_path()?)?;
        let rows = crate::db::queries::list_runtimes(&conn)?;
        rows.into_iter()
            .find(|row| row.is_active)
            .map(|row| installer::row_to_build(&row))
            .ok_or_else(|| AppError::NotFound {
                what: "an active runtime build — install one in Runtime settings first".to_string(),
            })
    }

    fn models(&self) -> Result<Vec<ModelEntry>, AppError> {
        model_registry::list_models()
    }

    fn prepare_registration(
        &self,
        build: &RuntimeBuild,
        models: &[ModelEntry],
    ) -> Result<Registration, AppError> {
        let root = installer::data_root().ok_or_else(|| AppError::Internal {
            message: "LOCALAPPDATA is not set; cannot locate the app data directory".to_string(),
        })?;
        std::fs::create_dir_all(&root).map_err(|err| AppError::Io {
            message: format!("could not create {}: {err}", root.display()),
        })?;

        let plan = preset_generator::preset_plan(build, models)?;
        let preset_path = root.join(PRESET_FILE);
        std::fs::write(&preset_path, plan.ini.as_bytes()).map_err(|err| AppError::Io {
            message: format!("could not write {}: {err}", preset_path.display()),
        })?;

        let models_dir = match build.registration_channel {
            RegistrationChannel::ScanOnly => {
                let dir = root.join(ROUTER_MODELS_DIR);
                std::fs::create_dir_all(&dir).map_err(|err| AppError::Io {
                    message: format!("could not create {}: {err}", dir.display()),
                })?;
                for model in models {
                    let name = model
                        .file_path
                        .file_name()
                        .map(|name| name.to_string_lossy().to_string())
                        .ok_or_else(|| AppError::InvalidPath {
                            path: model.file_path.display().to_string(),
                            reason: "a model path with no file name cannot be linked".to_string(),
                        })?;
                    model_paths::link_into(&dir, &model.file_path, &name)?;
                }
                Some(dir)
            }
            // The preset channel names each model's absolute path in its own
            // section; a scan directory would register the same models twice.
            RegistrationChannel::PresetDeclaresPath | RegistrationChannel::Undetermined => None,
        };

        Ok(Registration {
            preset_path: Some(preset_path),
            models_dir,
            warnings: plan.warnings,
        })
    }
}

/// `ServerConfig`'s documented defaults, used until T-050's screen has stored
/// anything. Every value is the one `docs/CONTRACTS.md` §1 annotates.
pub fn default_server_config() -> ServerConfig {
    ServerConfig {
        listen_address: std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
        listen_port: 8080,
        upstream_port_range: (49500, 49999),
        startup_hold_seconds: 120,
        process_timeout_seconds: 120,
        preload_timeout_seconds: 900,
        models_max: 1,
        autoload: false,
        idle_unload_seconds: None,
        api_key_set: false,
        request_log_enabled: true,
        max_concurrent_requests: None,
    }
}

/// The argument vector a start will spawn, as a value.
///
/// `--host 127.0.0.1` is not negotiable and not configurable: the upstream is the
/// app's own private process on loopback (`PLAN.md` §6), while
/// `ServerConfig.listen_address` is the *client-facing* address the app's own
/// listener binds — a different thing entirely. Building the vector as a
/// function is what makes that a property a test can assert rather than a
/// comment.
pub fn plan_launch(build: &RuntimeBuild, args: Vec<String>) -> Result<ServerLaunch, AppError> {
    let program = build.install_path.join("llama-server.exe");
    if !program.exists() {
        return Err(AppError::NotFound {
            what: format!(
                "llama-server.exe in {} — the installed build's files are missing; \
                 reinstall it from Runtime settings",
                build.install_path.display()
            ),
        });
    }
    Ok(ServerLaunch {
        program,
        args,
        cwd: Some(build.install_path.clone()),
    })
}

/// The probe kinds the actor can have in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProbeKind {
    Health,
    Models,
}

enum ProbeJob {
    Health {
        tag: u64,
        port: u16,
        endpoint: String,
    },
    Models {
        tag: u64,
        port: u16,
    },
}

enum ProbeOutcome {
    Health {
        tag: u64,
        result: Result<(), AppError>,
    },
    Models {
        tag: u64,
        result: Result<Vec<LoadedModelState>, AppError>,
    },
}

impl ProbeOutcome {
    fn kind(&self) -> ProbeKind {
        match self {
            ProbeOutcome::Health { .. } => ProbeKind::Health,
            ProbeOutcome::Models { .. } => ProbeKind::Models,
        }
    }

    /// The probe this result belongs to. A result whose tag is not the one
    /// currently in flight is a straggler — the actor had already given up on
    /// it — and must never be applied to the probe that replaced it, or a
    /// coordinator that answers late would keep resetting the failure count of
    /// a server that is not answering at all.
    fn tag(&self) -> u64 {
        match self {
            ProbeOutcome::Health { tag, .. } | ProbeOutcome::Models { tag, .. } => *tag,
        }
    }
}

/// Probes run on their own thread so a coordinator that hangs cannot freeze the
/// actor: the actor's own deadline notices, and the next probe is issued while
/// the stuck one is still out. The worker owns the coordinator, which owns its
/// runtime — the reason `HttpCoordinator` may block at all.
struct ProbeWorker {
    jobs: Sender<ProbeJob>,
    results: Receiver<ProbeOutcome>,
}

fn spawn_probe_worker(coordinator: Arc<dyn Coordinator>) -> ProbeWorker {
    let (job_tx, job_rx) = mpsc::channel::<ProbeJob>();
    let (result_tx, result_rx) = mpsc::channel::<ProbeOutcome>();
    if let Err(err) = thread::Builder::new()
        .name("supervisor-probe".to_string())
        .spawn(move || {
            while let Ok(job) = job_rx.recv() {
                let outcome = match job {
                    ProbeJob::Health {
                        tag,
                        port,
                        endpoint,
                    } => ProbeOutcome::Health {
                        tag,
                        result: coordinator.probe_health(port, &endpoint),
                    },
                    ProbeJob::Models { tag, port } => ProbeOutcome::Models {
                        tag,
                        result: coordinator.loaded_models(port),
                    },
                };
                if result_tx.send(outcome).is_err() {
                    break;
                }
            }
        })
    {
        tracing::error!("could not start the supervisor's probe thread: {err}");
    }
    ProbeWorker {
        jobs: job_tx,
        results: result_rx,
    }
}

/// The state of a named model in a `/models` reading, if it is there at all.
fn load_state_of(models: &[LoadedModelState], name: &str) -> Option<ModelLoadState> {
    models
        .iter()
        .find(|model| model.model_id == name)
        .map(|model| model.state.clone())
}

struct Actor {
    deps: SupervisorDeps,
    worker: Option<ProbeWorker>,

    state: ServerState,
    child: Option<Box<dyn ManagedChild>>,
    /// Bumped on every transition: a probe whose result belongs to a state we
    /// have left is discarded rather than applied to the new one.
    generation: u64,
    probe_in_flight: Option<(u64, ProbeKind)>,
    /// When the in-flight probe stops being believable.
    probe_deadline: Option<Instant>,
    /// Identifies the probe that is out. Every job carries its tag and every
    /// result must match the tag in flight, so a straggler from a probe the
    /// actor already gave up on cannot be mistaken for the current one's
    /// answer (`0` matches nothing).
    in_flight_tag: u64,
    next_probe_tag: u64,
    failures: u32,
    phase_started: Instant,
    /// The startup deadlines, read once per start (the database is not on the
    /// loop's path).
    start_timeouts: Timeouts,
    next_probe_at: Instant,
    stopping_since: Option<Instant>,
    killed_at: Option<Instant>,
    graceful_attempted: bool,
    /// The build's health endpoint, resolved at start. `None` means no server is
    /// starting.
    health_endpoint: Option<String>,
    upstream_port: Option<u16>,
    /// The served names of the models marked `preload`, in catalogue order.
    preload_targets: Vec<String>,
    /// The tail a crash reports.
    recent_log: Vec<String>,
}

impl Actor {
    fn new(deps: SupervisorDeps) -> Self {
        let now = Instant::now();
        Actor {
            deps,
            worker: None,
            state: ServerState::Stopped,
            child: None,
            generation: 0,
            probe_in_flight: None,
            probe_deadline: None,
            in_flight_tag: 0,
            next_probe_tag: 0,
            failures: 0,
            phase_started: now,
            start_timeouts: Timeouts {
                process: Duration::from_secs(120),
                preload: Duration::from_secs(900),
            },
            next_probe_at: now,
            stopping_since: None,
            killed_at: None,
            graceful_attempted: false,
            health_endpoint: None,
            upstream_port: None,
            preload_targets: Vec::new(),
            recent_log: Vec::new(),
        }
    }

    fn run(mut self, rx: Receiver<SupervisorCommand>) {
        self.worker = Some(spawn_probe_worker(self.deps.coordinator.clone()));
        loop {
            self.drain_logs();
            self.drain_probe_results();
            self.advance();
            match self.next_deadline() {
                Some(when) => {
                    let wait = when
                        .saturating_duration_since(Instant::now())
                        .max(Duration::from_millis(1));
                    match rx.recv_timeout(wait) {
                        Ok(command) => self.apply(command),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                // Nothing scheduled: sleep on the channel instead of waking up
                // every second to find out there is nothing to do.
                None => match rx.recv() {
                    Ok(command) => self.apply(command),
                    Err(_) => return,
                },
            }
        }
    }

    /// The earliest moment the actor has something to do, or `None` when it has
    /// nothing scheduled at all.
    fn next_deadline(&self) -> Option<Instant> {
        if matches!(
            &self.state,
            ServerState::Stopped | ServerState::Crashed { .. }
        ) {
            return None;
        }
        let mut next = self.next_probe_at;
        if let Some(deadline) = self.probe_deadline {
            next = next.min(deadline);
        }
        if matches!(&self.state, ServerState::Starting { .. }) {
            next = next.min(self.phase_started + self.phase_limit());
        }
        if let Some(since) = self.stopping_since {
            next = next.min(since + self.deps.tuning.stop_grace);
        }
        if let Some(killed_at) = self.killed_at {
            next = next.min(killed_at + self.deps.tuning.kill_wait);
        }
        Some(next)
    }

    /// The deadline that bounds the phase the actor is in.
    fn phase_limit(&self) -> Duration {
        match &self.state {
            ServerState::Starting {
                phase: StartupPhase::Preloading { .. },
                ..
            } => self.start_timeouts.preload,
            _ => self.start_timeouts.process,
        }
    }

    fn is_waiting_for_process(&self) -> bool {
        matches!(
            &self.state,
            ServerState::Starting {
                phase: StartupPhase::WaitingForProcess,
                ..
            }
        )
    }

    fn is_preloading(&self) -> bool {
        matches!(
            &self.state,
            ServerState::Starting {
                phase: StartupPhase::Preloading { .. },
                ..
            }
        )
    }

    fn is_running(&self) -> bool {
        matches!(&self.state, ServerState::Running { .. })
    }

    // ─── Commands ───────────────────────────────────────────────

    fn apply(&mut self, command: SupervisorCommand) {
        match command {
            SupervisorCommand::Start(reply) => {
                let result = self.cmd_start();
                let _ = reply.send(result);
            }
            SupervisorCommand::Stop(reply) => {
                let result = self.cmd_stop();
                let _ = reply.send(result);
            }
            SupervisorCommand::DismissCrash(reply) => {
                let result = self.cmd_dismiss_crash();
                let _ = reply.send(result);
            }
            SupervisorCommand::GetState(reply) => {
                let _ = reply.send(Ok(self.state.clone()));
            }
            SupervisorCommand::MarkConfigDirty(reply) => {
                let _ = reply.send(self.cmd_mark_config_dirty());
            }
        }
    }

    fn cmd_start(&mut self) -> Result<ServerState, AppError> {
        match self.state {
            ServerState::Stopped | ServerState::Crashed { .. } => {}
            _ => return Err(self.invalid("start_server")),
        }

        let build = self.deps.context.active_build()?;
        let models = self.deps.context.models()?;
        // The endpoint is recorded by T-023 and may be absent for a build that
        // was never verified. §2 says the app does not invent one, so starting
        // stops here instead of probing a guessed path for two minutes.
        let endpoint = build
            .health_endpoint
            .clone()
            .ok_or_else(|| AppError::Internal {
                message: format!(
                    "{} has no recorded health endpoint, so the app cannot tell when it is up. \
                     Install or re-verify the build in Runtime settings.",
                    build.build_tag
                ),
            })?;

        let registration = self.deps.context.prepare_registration(&build, &models)?;
        for warning in &registration.warnings {
            self.deps.events.preset_warning(warning);
        }

        let config = self.deps.context.server_config()?;
        let timeouts = match self.deps.timeouts_override {
            Some(override_timeouts) => override_timeouts,
            None => Timeouts::from_config(&config),
        };
        let port = self.deps.ports.allocate(config.upstream_port_range)?;
        let args = preset_generator::router_arguments(
            &build,
            port,
            registration.preset_path.as_deref(),
            registration.models_dir.as_deref(),
        )?;
        let launch = plan_launch(&build, args)?;
        let child = self.deps.launcher.spawn(&launch)?;
        let pid = child.pid();

        self.preload_targets = models
            .iter()
            .filter(|model| model.preload)
            .map(|model| model.served_name.clone())
            .collect();
        self.child = Some(child);
        self.start_timeouts = timeouts;
        self.health_endpoint = Some(endpoint);
        self.upstream_port = Some(port);
        self.failures = 0;
        self.stopping_since = None;
        self.killed_at = None;
        self.graceful_attempted = false;
        self.phase_started = Instant::now();
        self.next_probe_at = self.phase_started + self.deps.tuning.waiting_probe_interval;
        self.probe_in_flight = None;
        self.probe_deadline = None;
        self.in_flight_tag = 0;
        self.push_log(format!(
            "starting {} on 127.0.0.1:{port} (pid {pid})",
            build.build_tag
        ));
        // `set_state` bumps the generation, so it comes last: everything the new
        // state's probes rely on is already in place.
        self.set_state(ServerState::Starting {
            since: Utc::now(),
            upstream_port: port,
            phase: StartupPhase::WaitingForProcess,
        });
        Ok(self.state.clone())
    }

    fn cmd_stop(&mut self) -> Result<ServerState, AppError> {
        match self.state {
            ServerState::Starting { .. } | ServerState::Running { .. } => {}
            _ => return Err(self.invalid("stop_server")),
        }
        self.begin_stop();
        Ok(self.state.clone())
    }

    fn cmd_dismiss_crash(&mut self) -> Result<ServerState, AppError> {
        match self.state {
            ServerState::Crashed { .. } => {}
            _ => return Err(self.invalid("dismiss_crash")),
        }
        self.clear_run_context();
        self.set_state(ServerState::Stopped);
        Ok(self.state.clone())
    }

    fn cmd_mark_config_dirty(&mut self) -> Result<(), AppError> {
        if let ServerState::Running {
            pid,
            upstream_port,
            since,
            config_dirty: false,
        } = self.state
        {
            self.set_state(ServerState::Running {
                pid,
                upstream_port,
                since,
                config_dirty: true,
            });
        }
        // Already dirty, or not running: the flag describes a running server,
        // and marking a stopped one dirty would show a banner for a restart that
        // is not needed.
        Ok(())
    }

    fn invalid(&self, command: &str) -> AppError {
        AppError::InvalidTransition {
            from: state_label(&self.state).to_string(),
            command: command.to_string(),
        }
    }

    // ─── State transitions ──────────────────────────────────────

    fn set_state(&mut self, state: ServerState) {
        self.generation += 1;
        self.state = state;
        self.deps.events.state_changed(&self.state);
    }

    fn clear_run_context(&mut self) {
        self.generation += 1;
        self.probe_in_flight = None;
        self.probe_deadline = None;
        self.in_flight_tag = 0;
        self.health_endpoint = None;
        self.upstream_port = None;
        self.preload_targets.clear();
        self.failures = 0;
    }

    fn begin_stop(&mut self) {
        self.stopping_since = Some(Instant::now());
        self.killed_at = None;
        self.graceful_attempted = false;
        // A probe issued before the stop describes a server the user has just
        // asked to end: its result must not move the new state.
        self.probe_in_flight = None;
        self.probe_deadline = None;
        self.in_flight_tag = 0;
        self.failures = 0;
        self.push_log("stopping the server".to_string());
        self.set_state(ServerState::Stopping);
    }

    fn finish_stop(&mut self) {
        let root = self.child.as_ref().map(|child| child.pid());
        // A root that exited on its own can still have left children behind —
        // a router that dies does not necessarily take its per-model processes
        // with it, and those hold VRAM. When the tree kill already ran, the
        // sweep would be a second kill of processes that are already gone.
        if self.killed_at.is_none() {
            self.reap_tree(root, "while stopping");
        }
        self.child = None;
        self.stopping_since = None;
        self.killed_at = None;
        self.graceful_attempted = false;
        self.push_log("server stopped".to_string());
        self.clear_run_context();
        self.set_state(ServerState::Stopped);
    }

    fn crash(&mut self, exit_code: Option<i32>, diagnosis: Option<Diagnosis>) {
        // Before the tail is frozen: read the child's last words. Every crash
        // path arrives here (an observed exit, an expired phase, three failed
        // probes), and in all of them the lines that explain the failure are the
        // ones written just before it — in flight, not yet drained.
        self.final_drain();
        let root = self.child.as_ref().map(|child| child.pid());
        self.reap_tree(root, "after the crash");
        self.child = None;
        self.clear_run_context();
        let last_log = std::mem::take(&mut self.recent_log);
        self.set_state(ServerState::Crashed {
            exit_code,
            diagnosis,
            last_log,
        });
    }

    /// Kill whatever is left of a tree and say so when the OS disagrees.
    ///
    /// The sweep runs even when the root has already exited: a router that dies
    /// does not take its per-model child processes with it, and those hold VRAM.
    /// `pid_alive` asks the OS, so this is evidence rather than a belief.
    fn reap_tree(&mut self, root: Option<u32>, when: &str) {
        let Some(root) = root else { return };
        let outcome = self
            .child
            .as_mut()
            .map(|child| child.kill_tree())
            .unwrap_or(Ok(Vec::new()));
        match outcome {
            Ok(killed) if !killed.is_empty() => {
                let count = killed.len();
                self.push_log(format!("terminated {count} process(es) {when}: {killed:?}"));
            }
            Ok(_) => {}
            Err(err) => self.push_log(format!("could not terminate the process tree: {err}")),
        }
        if process::tree::pid_alive(root) {
            self.push_log(format!(
                "process {root} is still alive {when}; it may need to be ended manually"
            ));
        }
    }

    // ─── The loop's own work ────────────────────────────────────

    /// Read whatever the child has produced since the last call, and report how
    /// many lines that was.
    fn drain_logs(&mut self) -> usize {
        let mut lines: Vec<LogLine> = Vec::new();
        if let Some(child) = self.child.as_mut() {
            child.drain_logs(&mut |line| lines.push(line));
        }
        let drained = lines.len();
        for line in lines {
            self.record_log(&line);
        }
        drained
    }

    /// Keep reading the child's output for a bounded moment after its exit was
    /// observed, so its last words reach the tail instead of being dropped.
    ///
    /// The loop's own drain runs *before* the state machine looks at the child,
    /// so a line written between the two is missed by a single read — and on a
    /// real pipe the reader thread delivers the final bytes a moment after the
    /// process ends. Both are the same loss: a crash whose tail cannot say why.
    /// Two consecutive empty reads mean the writer is gone and the drain stops
    /// immediately, so a scripted child in a test costs about a millisecond.
    fn final_drain(&mut self) {
        let deadline = Instant::now() + self.deps.tuning.final_drain;
        let mut empty_reads = 0;
        loop {
            if self.drain_logs() > 0 {
                empty_reads = 0;
            } else {
                empty_reads += 1;
                if empty_reads >= 2 {
                    return;
                }
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    fn record_log(&mut self, line: &LogLine) {
        let cap = self.deps.tuning.log_ring_capacity.max(1);
        while self.recent_log.len() >= cap {
            self.recent_log.remove(0);
        }
        self.recent_log.push(line.line.clone());
        self.deps.events.log_line(line);
    }

    /// An actor-side line (a decision, a refusal) joins the same stream as the
    /// server's own output: the Logs screen (T-046) shows why the app acted and
    /// not only what the server said.
    fn push_log(&mut self, message: String) {
        let line = LogLine {
            at: Utc::now(),
            level: "info".to_string(),
            line: format!("supervisor: {message}"),
        };
        self.record_log(&line);
    }

    fn drain_probe_results(&mut self) {
        let Some(worker) = self.worker.as_ref() else {
            return;
        };
        let mut outcomes = Vec::new();
        while let Ok(outcome) = worker.results.try_recv() {
            outcomes.push(outcome);
        }
        for outcome in outcomes {
            self.apply_probe(outcome);
        }
    }

    fn apply_probe(&mut self, outcome: ProbeOutcome) {
        let kind = outcome.kind();
        // A result that belongs to a state we have left — or to a probe the
        // actor already timed out and replaced — is dropped: applying it would
        // move the new state on the strength of an old observation.
        let matches_in_flight = matches!(
            self.probe_in_flight,
            Some((generation, in_flight)) if generation == self.generation && in_flight == kind
        );
        if !matches_in_flight || outcome.tag() != self.in_flight_tag {
            return;
        }
        self.probe_in_flight = None;
        self.probe_deadline = None;
        self.in_flight_tag = 0;

        match outcome {
            ProbeOutcome::Health { result, .. } => self.on_health_probe(result),
            ProbeOutcome::Models { result, .. } => self.on_models_probe(result),
        }
    }

    fn on_health_probe(&mut self, result: Result<(), AppError>) {
        if self.is_waiting_for_process() {
            if result.is_ok() {
                if self.preload_targets.is_empty() {
                    self.enter_running();
                } else if let Some(port) = self.upstream_port {
                    self.enter_preloading(port);
                }
            }
            // A failing probe here is the phase's own definition, not a death:
            // the coordinator is expected to be quiet until it is up, and
            // `process_timeout_seconds` bounds how long that lasts.
            return;
        }
        if self.is_running() || self.is_preloading() {
            match result {
                Ok(()) => self.failures = 0,
                Err(err) => self.note_probe_failure(format!(
                    "the coordinator did not answer the health check: {err}"
                )),
            }
        }
    }

    fn on_models_probe(&mut self, result: Result<Vec<LoadedModelState>, AppError>) {
        if !self.is_preloading() {
            return;
        }
        let Some(port) = self.upstream_port else {
            return;
        };

        let models = match result {
            Ok(models) => {
                self.failures = 0;
                models
            }
            Err(err) => {
                self.note_probe_failure(format!(
                    "the coordinator did not answer while preloading: {err}"
                ));
                return;
            }
        };

        // A failed preload ends the start rather than sitting in a phase: §2
        // makes the diagnosis name the model that failed.
        let failed = self
            .preload_targets
            .iter()
            .find(|name| load_state_of(&models, name) == Some(ModelLoadState::Failed))
            .cloned();
        if let Some(failed) = failed {
            let detail = models
                .iter()
                .find(|model| model.model_id == failed)
                .and_then(|model| model.error.clone())
                .unwrap_or_default();
            let message = if detail.is_empty() {
                format!("the model {failed} failed to load")
            } else {
                format!("the model {failed} failed to load: {detail}")
            };
            self.crash(
                None,
                Some(Diagnosis {
                    code: "preload_failed".to_string(),
                    message,
                    remediation: format!(
                        "Open {failed} in the Models screen and check its launch parameters \
                         against this build's verified flags, then start the server again."
                    ),
                }),
            );
            return;
        }

        let total = self.preload_targets.len() as u32;
        let done = self
            .preload_targets
            .iter()
            .filter(|name| load_state_of(&models, name) == Some(ModelLoadState::Loaded))
            .count() as u32;
        if done == total {
            self.enter_running();
            return;
        }

        let current = self
            .preload_targets
            .iter()
            .find(|name| load_state_of(&models, name) != Some(ModelLoadState::Loaded))
            .cloned();
        self.enter_preloading_with(
            port,
            StartupPhase::Preloading {
                done,
                total,
                current,
            },
        );
    }

    /// A failure of a probe issued *after* the coordinator has answered once.
    /// Three in a row is the death of a server that is still in the process
    /// table; one is a dropped response.
    fn note_probe_failure(&mut self, detail: String) {
        self.failures += 1;
        let seen = self.failures.min(3);
        self.push_log(format!("{detail} ({seen}/3 consecutive)"));
        if self.failures >= 3 {
            self.crash(
                None,
                Some(Diagnosis {
                    code: "upstream_not_answering".to_string(),
                    message: format!("the server is still running but stopped answering: {detail}"),
                    remediation: "Start the server again from the Runtime screen. If it keeps \
                                  happening, look at the logs for the last thing it said before \
                                  going quiet."
                        .to_string(),
                }),
            );
        }
    }

    fn enter_preloading(&mut self, port: u16) {
        let phase = StartupPhase::Preloading {
            done: 0,
            total: self.preload_targets.len() as u32,
            current: self.preload_targets.first().cloned(),
        };
        self.enter_preloading_with(port, phase);
    }

    fn enter_preloading_with(&mut self, port: u16, phase: StartupPhase) {
        let (since, current_port, current_phase) = match &self.state {
            ServerState::Starting {
                since,
                upstream_port,
                phase,
            } => (*since, *upstream_port, phase.clone()),
            _ => return,
        };
        // Progress updates arrive on every poll; only a change is an event.
        if current_phase == phase && current_port == port {
            return;
        }
        let now = Instant::now();
        if !matches!(current_phase, StartupPhase::Preloading { .. }) {
            // Entering the phase starts its own timeout.
            self.phase_started = now;
        }
        self.next_probe_at = now + self.deps.tuning.preloading_probe_interval;
        self.set_state(ServerState::Starting {
            since,
            upstream_port: port,
            phase,
        });
    }

    fn enter_running(&mut self) {
        let Some(pid) = self.child.as_ref().map(|child| child.pid()) else {
            return;
        };
        let Some(port) = self.upstream_port else {
            return;
        };
        self.failures = 0;
        self.next_probe_at = Instant::now() + self.deps.tuning.running_probe_interval;
        self.push_log(format!("server is up (pid {pid}) on 127.0.0.1:{port}"));
        self.set_state(ServerState::Running {
            pid,
            upstream_port: port,
            since: Utc::now(),
            config_dirty: false,
        });
    }

    fn advance(&mut self) {
        let now = Instant::now();

        // A probe that never comes back is a failure like any other, and it must
        // not block the next one: the worker's result, when it finally arrives,
        // is dropped because nothing is in flight any more.
        if let Some(deadline) = self.probe_deadline {
            if now >= deadline {
                self.probe_in_flight = None;
                self.probe_deadline = None;
                // Nothing matches this probe any more: its result, if it ever
                // arrives, is a straggler and gets dropped.
                self.in_flight_tag = 0;
                if self.is_running() || self.is_preloading() {
                    self.note_probe_failure(
                        "a probe did not answer within its own timeout".to_string(),
                    );
                }
            }
        }

        self.check_exit();

        if self.is_waiting_for_process() || self.is_preloading() {
            let limit = self.phase_limit();
            let waiting = self.is_waiting_for_process();
            if now.saturating_duration_since(self.phase_started) >= limit {
                let seconds = limit.as_secs();
                if waiting {
                    self.crash(
                        None,
                        Some(Diagnosis {
                            code: "startup_timeout".to_string(),
                            message: format!(
                                "the server did not answer its health endpoint within {seconds} s"
                            ),
                            remediation: "Check the log tail below for what the process said, \
                                          then start it again. Raise the process timeout in API \
                                          settings if this machine is simply slow to start."
                                .to_string(),
                        }),
                    );
                } else {
                    let model = self
                        .current_preload()
                        .unwrap_or_else(|| "a preloaded model".to_string());
                    self.crash(
                        None,
                        Some(Diagnosis {
                            code: "preload_timeout".to_string(),
                            message: format!("{model} did not finish loading within {seconds} s"),
                            remediation: format!(
                                "Check that {model} fits in VRAM at its current parameters, or \
                                 untick preload on it to load it on demand instead."
                            ),
                        }),
                    );
                }
                return;
            }
            if let Some(port) = self.upstream_port {
                let kind = if waiting {
                    ProbeKind::Health
                } else {
                    ProbeKind::Models
                };
                self.maybe_probe(now, kind, port);
            }
            return;
        }

        if let ServerState::Running { upstream_port, .. } = self.state {
            self.maybe_probe(now, ProbeKind::Health, upstream_port);
            return;
        }

        if matches!(&self.state, ServerState::Stopping) {
            self.advance_stopping(now);
        }
    }

    fn advance_stopping(&mut self, now: Instant) {
        if !self.graceful_attempted {
            self.graceful_attempted = true;
            let outcome = self
                .child
                .as_mut()
                .map(|child| child.request_graceful_stop());
            match outcome {
                Some(Ok(())) => self.push_log(
                    "the server accepted the stop request; waiting for it to exit".to_string(),
                ),
                // Measured on the installed b10883 router: it refuses the
                // request outright, so this path — not the polite one — is what
                // actually stops a server today. Say so rather than pretending
                // the request worked. `taskkill` answers in the console's own
                // code page and with embedded newlines, so its message is
                // flattened before it becomes a log *line*.
                Some(Err(err)) => {
                    let detail = err
                        .message()
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    self.push_log(format!(
                        "the server refused the graceful stop request ({detail}); \
                         the grace window decides"
                    ));
                }
                None => {}
            }
        }

        let exited = matches!(
            self.child.as_mut().map(|child| child.try_wait()),
            Some(Ok(Some(_))) | Some(Err(_))
        );
        if exited {
            self.finish_stop();
            return;
        }

        let since = self.stopping_since.unwrap_or(now);
        if self.killed_at.is_none()
            && now.saturating_duration_since(since) >= self.deps.tuning.stop_grace
        {
            let grace = self.deps.tuning.stop_grace.as_secs();
            self.push_log(format!(
                "the grace window ({grace} s) expired; killing the process tree"
            ));
            self.killed_at = Some(now);
            let outcome = self
                .child
                .as_mut()
                .map(|child| child.kill_tree())
                .unwrap_or(Ok(Vec::new()));
            match outcome {
                Ok(killed) if !killed.is_empty() => {
                    let count = killed.len();
                    self.push_log(format!("terminated {count} process(es): {killed:?}"));
                }
                Ok(_) => {}
                Err(err) => self.push_log(format!("could not kill the process tree: {err}")),
            }
        }

        if let Some(killed_at) = self.killed_at {
            if now.saturating_duration_since(killed_at) >= self.deps.tuning.kill_wait {
                if let Some(child) = self.child.as_ref() {
                    let pid = child.pid();
                    if process::tree::pid_alive(pid) {
                        self.push_log(format!(
                            "process {pid} survived the tree kill; it may need to be ended manually"
                        ));
                    }
                }
                self.finish_stop();
            }
        }
    }

    /// A child that has exited on its own.
    fn check_exit(&mut self) {
        if self.child.is_none() {
            return;
        }
        if matches!(&self.state, ServerState::Stopping) {
            // `advance_stopping` owns this case: it must sweep the tree before
            // declaring the stop finished.
            return;
        }
        let code = match self.child.as_mut().map(|child| child.try_wait()) {
            Some(Ok(Some(code))) => code,
            _ => return,
        };
        self.push_log(format!("the server process exited with code {code}"));
        // T-043 maps stderr patterns to a diagnosis. Until then a crash carries
        // its log tail and no invented cause: an exit code alone does not name
        // one.
        self.crash(Some(code), None);
    }

    fn current_preload(&self) -> Option<String> {
        match &self.state {
            ServerState::Starting {
                phase:
                    StartupPhase::Preloading {
                        current: Some(current),
                        ..
                    },
                ..
            } => Some(current.clone()),
            _ => self.preload_targets.first().cloned(),
        }
    }

    fn maybe_probe(&mut self, now: Instant, kind: ProbeKind, port: u16) {
        if self.probe_in_flight.is_some() || now < self.next_probe_at {
            return;
        }
        let Some(endpoint) = self.health_endpoint.clone() else {
            return;
        };
        self.next_probe_tag += 1;
        let tag = self.next_probe_tag;
        let job = match kind {
            ProbeKind::Health => ProbeJob::Health {
                tag,
                port,
                endpoint,
            },
            ProbeKind::Models => ProbeJob::Models { tag, port },
        };
        let sent = match self.worker.as_ref() {
            Some(worker) => worker.jobs.send(job).is_ok(),
            None => false,
        };
        if !sent {
            return;
        }
        let interval = match kind {
            ProbeKind::Health => {
                if self.is_running() {
                    self.deps.tuning.running_probe_interval
                } else {
                    self.deps.tuning.waiting_probe_interval
                }
            }
            ProbeKind::Models => self.deps.tuning.preloading_probe_interval,
        };
        self.probe_in_flight = Some((self.generation, kind));
        self.in_flight_tag = tag;
        self.probe_deadline = Some(now + self.deps.tuning.probe_timeout);
        self.next_probe_at = now + interval;
    }
}
