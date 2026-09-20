//! T-040's state-machine tests (`docs/TASKS.md`), driven through the public
//! handle exactly as the IPC layer drives the actor.
//!
//! Every acceptance criterion about *behaviour* is asserted here against
//! scripted doubles — a child that can be told to hang, exit, or emit output,
//! and a coordinator whose answers are a script. The criteria about the
//! *machine* (a real process tree reaped, no orphan left behind, log capture
//! under volume) are asserted against real processes in `core::process`'s own
//! tests and, below, against a real `cmd.exe` tree; the installed build is
//! driven end-to-end by the env-gated test at the end.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;

use crate::core::preset_generator::PresetWarning;
use crate::core::process::{
    self, LogLine, ManagedChild, PortAllocator, ProcessLauncher, ServerLaunch, SystemLauncher,
};
use crate::core::supervisor::{
    default_server_config, parse_router_models, plan_launch, reject_unless_stopped, spawn,
    state_label, Coordinator, Registration, ServerContext, SupervisorDeps, SupervisorEvents,
    SupervisorHandle, Timeouts, Tuning,
};
use crate::core::types::{
    AppError, Backend, Compatibility, EndpointState, GgufMetadata, LaunchParams, LoadedModelState,
    ModelAvailability, ModelEntry, ModelLoadState, RegistrationChannel, RuntimeBuild,
    SamplingDefaults, ServerConfig, ServerState, StartupPhase, VerifiedFlag,
};

// ─── Test doubles ───────────────────────────────────────────────

/// A pid that cannot be a real process, so the OS-level liveness check the
/// supervisor logs from is never accidentally true for a scripted child.
const FAKE_PID: u32 = u32::MAX - 3;

#[derive(Debug, Default)]
struct ChildState {
    /// `Some(code)` once the process has exited.
    exit_code: Option<i32>,
    /// Whether the polite stop request is honoured. The installed llama-server
    /// refuses it (measured), so `false` is the realistic default.
    grace_accepts: bool,
    kill_tree_calls: u32,
    /// Lines the child "produced", drained by the actor.
    logs: Vec<LogLine>,
    /// Lines the child writes *as it dies*, handed over the first time its exit
    /// status is observed. A real process writes its last words and then exits,
    /// so the actor can only read them after it has noticed the exit — the
    /// interleaving that loses the tail if the crash is built on the exit
    /// status alone. Scripted rather than timed, so the test is deterministic.
    dying_words: Vec<LogLine>,
}

/// A child whose whole observable behaviour is scripted.
struct FakeChild {
    pid: u32,
    state: Arc<Mutex<ChildState>>,
}

impl ManagedChild for FakeChild {
    fn pid(&self) -> u32 {
        self.pid
    }

    fn try_wait(&mut self) -> Result<Option<i32>, AppError> {
        let mut state = self.state.lock().unwrap();
        if state.exit_code.is_some() && !state.dying_words.is_empty() {
            let words = std::mem::take(&mut state.dying_words);
            state.logs.extend(words);
        }
        Ok(state.exit_code)
    }

    fn request_graceful_stop(&mut self) -> Result<(), AppError> {
        let mut state = self.state.lock().unwrap();
        if state.grace_accepts {
            state.exit_code = Some(0);
            Ok(())
        } else {
            Err(AppError::Io {
                message: "the process can only be terminated forcefully".to_string(),
            })
        }
    }

    fn kill_tree(&mut self) -> Result<Vec<u32>, AppError> {
        let mut state = self.state.lock().unwrap();
        state.kill_tree_calls += 1;
        if state.exit_code.is_some() {
            // Already gone: the real tree walk finds nothing left to terminate
            // and reports an empty list, so a sweep over a dead root is not
            // evidence that anything was killed.
            return Ok(Vec::new());
        }
        state.exit_code = Some(1);
        Ok(vec![self.pid])
    }

    fn drain_logs(&mut self, sink: &mut dyn FnMut(LogLine)) {
        let mut state = self.state.lock().unwrap();
        for line in state.logs.drain(..) {
            sink(line);
        }
    }
}

#[derive(Default)]
struct ScriptedLauncher {
    pending: Mutex<VecDeque<Arc<Mutex<ChildState>>>>,
    spawns: Mutex<Vec<ServerLaunch>>,
    failure: Mutex<Option<AppError>>,
}

impl ScriptedLauncher {
    fn new() -> Arc<Self> {
        Arc::new(ScriptedLauncher::default())
    }

    /// Queue a child and hand back its state so the test can drive it.
    fn push(&self, state: ChildState) -> Arc<Mutex<ChildState>> {
        let state = Arc::new(Mutex::new(state));
        self.pending.lock().unwrap().push_back(state.clone());
        state
    }

    fn fail_with(&self, error: AppError) {
        *self.failure.lock().unwrap() = Some(error);
    }

    fn spawn_count(&self) -> usize {
        self.spawns.lock().unwrap().len()
    }

    fn last_launch(&self) -> Option<ServerLaunch> {
        self.spawns.lock().unwrap().last().cloned()
    }
}

impl ProcessLauncher for ScriptedLauncher {
    fn spawn(&self, launch: &ServerLaunch) -> Result<Box<dyn ManagedChild>, AppError> {
        if let Some(error) = self.failure.lock().unwrap().clone() {
            return Err(error);
        }
        self.spawns.lock().unwrap().push(launch.clone());
        let state = self
            .pending
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| Arc::new(Mutex::new(ChildState::default())));
        Ok(Box::new(FakeChild {
            pid: FAKE_PID,
            state,
        }))
    }
}

/// The real launcher with the program and arguments swapped for a stand-in
/// command. Used where the criterion is about the supervisor's behaviour toward
/// a *real* process tree, which a scripted child cannot demonstrate.
struct StandInLauncher {
    program: PathBuf,
    args: Vec<String>,
    inner: SystemLauncher,
}

impl ProcessLauncher for StandInLauncher {
    fn spawn(&self, launch: &ServerLaunch) -> Result<Box<dyn ManagedChild>, AppError> {
        self.inner.spawn(&ServerLaunch {
            program: self.program.clone(),
            args: self.args.clone(),
            cwd: launch.cwd.clone(),
        })
    }
}

struct ScriptedCoordinator {
    health: Mutex<VecDeque<Result<(), AppError>>>,
    health_default: Mutex<Result<(), AppError>>,
    health_delay: Mutex<Duration>,
    models: Mutex<VecDeque<Result<Vec<LoadedModelState>, AppError>>>,
    models_default: Mutex<Result<Vec<LoadedModelState>, AppError>>,
    health_calls: AtomicUsize,
    models_calls: AtomicUsize,
    endpoints: Mutex<Vec<String>>,
}

impl Default for ScriptedCoordinator {
    fn default() -> Self {
        ScriptedCoordinator {
            health: Mutex::new(VecDeque::new()),
            // Default: the coordinator answers. Tests that want silence script
            // failures.
            health_default: Mutex::new(Ok(())),
            health_delay: Mutex::new(Duration::ZERO),
            models: Mutex::new(VecDeque::new()),
            models_default: Mutex::new(Ok(Vec::new())),
            health_calls: AtomicUsize::new(0),
            models_calls: AtomicUsize::new(0),
            endpoints: Mutex::new(Vec::new()),
        }
    }
}

impl ScriptedCoordinator {
    fn new() -> Arc<Self> {
        Arc::new(ScriptedCoordinator::default())
    }

    fn script_health(&self, replies: Vec<Result<(), AppError>>) {
        self.health.lock().unwrap().extend(replies);
    }

    /// The default for *every* remaining call — what a silent coordinator is.
    fn set_health_default(&self, reply: Result<(), AppError>) {
        *self.health_default.lock().unwrap() = reply;
    }

    fn set_health_delay(&self, delay: Duration) {
        *self.health_delay.lock().unwrap() = delay;
    }

    fn script_models(&self, replies: Vec<Result<Vec<LoadedModelState>, AppError>>) {
        self.models.lock().unwrap().extend(replies);
    }

    fn set_models_default(&self, reply: Result<Vec<LoadedModelState>, AppError>) {
        *self.models_default.lock().unwrap() = reply;
    }

    fn health_calls(&self) -> usize {
        self.health_calls.load(Ordering::SeqCst)
    }

    fn models_calls(&self) -> usize {
        self.models_calls.load(Ordering::SeqCst)
    }

    fn probed_endpoints(&self) -> Vec<String> {
        self.endpoints.lock().unwrap().clone()
    }
}

fn network_error() -> AppError {
    AppError::Network {
        message: "connection refused".to_string(),
    }
}

impl Coordinator for ScriptedCoordinator {
    fn probe_health(&self, _port: u16, endpoint: &str) -> Result<(), AppError> {
        self.health_calls.fetch_add(1, Ordering::SeqCst);
        self.endpoints.lock().unwrap().push(endpoint.to_string());
        let delay = *self.health_delay.lock().unwrap();
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        match self.health.lock().unwrap().pop_front() {
            Some(reply) => reply,
            None => self.health_default.lock().unwrap().clone(),
        }
    }

    fn loaded_models(&self, _port: u16) -> Result<Vec<LoadedModelState>, AppError> {
        self.models_calls.fetch_add(1, Ordering::SeqCst);
        match self.models.lock().unwrap().pop_front() {
            Some(reply) => reply,
            None => self.models_default.lock().unwrap().clone(),
        }
    }
}

#[derive(Default)]
struct Recorder {
    states: Mutex<Vec<ServerState>>,
    logs: Mutex<Vec<LogLine>>,
    warnings: Mutex<Vec<PresetWarning>>,
    /// T-042: the listener's own lifecycle, recorded separately from
    /// `ServerState` because §2 keeps the two apart.
    endpoints: Mutex<Vec<EndpointState>>,
}

impl Recorder {
    fn new() -> Arc<Self> {
        Arc::new(Recorder::default())
    }

    fn labels(&self) -> Vec<String> {
        self.states
            .lock()
            .unwrap()
            .iter()
            .map(|state| state_label(state).to_string())
            .collect()
    }

    fn phases(&self) -> Vec<String> {
        self.states
            .lock()
            .unwrap()
            .iter()
            .filter_map(|state| match state {
                ServerState::Starting { phase, .. } => Some(match phase {
                    StartupPhase::WaitingForProcess => "WaitingForProcess".to_string(),
                    StartupPhase::Preloading {
                        done,
                        total,
                        current,
                    } => format!(
                        "Preloading({done}/{total},{})",
                        current.clone().unwrap_or_else(|| "-".to_string())
                    ),
                }),
                _ => None,
            })
            .collect()
    }

    fn lines(&self) -> Vec<String> {
        self.logs
            .lock()
            .unwrap()
            .iter()
            .map(|line| line.line.clone())
            .collect()
    }

    /// T-042: the listener's own lifecycle, in the order it was reported.
    fn endpoints(&self) -> Vec<EndpointState> {
        self.endpoints.lock().unwrap().clone()
    }
}

impl SupervisorEvents for Recorder {
    fn state_changed(&self, state: &ServerState) {
        self.states.lock().unwrap().push(state.clone());
    }

    fn log_line(&self, line: &LogLine) {
        self.logs.lock().unwrap().push(line.clone());
    }

    fn preset_warning(&self, warning: &PresetWarning) {
        self.warnings.lock().unwrap().push(warning.clone());
    }

    fn endpoint_state_changed(&self, state: &EndpointState) {
        self.endpoints.lock().unwrap().push(state.clone());
    }
}

struct FakeContext {
    config: ServerConfig,
    build: Result<RuntimeBuild, AppError>,
    models: Vec<ModelEntry>,
    warnings: Vec<PresetWarning>,
    registration_calls: AtomicUsize,
}

impl FakeContext {
    fn new(build: RuntimeBuild, models: Vec<ModelEntry>) -> Arc<Self> {
        Arc::new(FakeContext {
            config: default_config(),
            build: Ok(build),
            models,
            warnings: Vec::new(),
            registration_calls: AtomicUsize::new(0),
        })
    }

    fn registration_calls(&self) -> usize {
        self.registration_calls.load(Ordering::SeqCst)
    }
}

impl ServerContext for FakeContext {
    fn server_config(&self) -> Result<ServerConfig, AppError> {
        Ok(self.config.clone())
    }

    fn active_build(&self) -> Result<RuntimeBuild, AppError> {
        self.build.clone()
    }

    fn models(&self) -> Result<Vec<ModelEntry>, AppError> {
        Ok(self.models.clone())
    }

    fn prepare_registration(
        &self,
        _build: &RuntimeBuild,
        _models: &[ModelEntry],
    ) -> Result<Registration, AppError> {
        self.registration_calls.fetch_add(1, Ordering::SeqCst);
        Ok(Registration {
            preset_path: Some(PathBuf::from("C:/tmp/presets.ini")),
            models_dir: None,
            warnings: self.warnings.clone(),
        })
    }
}

struct ScriptedPorts {
    ports: Mutex<VecDeque<Result<u16, AppError>>>,
    fallback: u16,
}

impl ScriptedPorts {
    fn new() -> Arc<Self> {
        Arc::new(ScriptedPorts {
            ports: Mutex::new(VecDeque::new()),
            fallback: 49700,
        })
    }

    fn fail_with(&self, error: AppError) {
        self.ports.lock().unwrap().push_back(Err(error));
    }
}

impl PortAllocator for ScriptedPorts {
    fn allocate(&self, _range: (u16, u16)) -> Result<u16, AppError> {
        match self.ports.lock().unwrap().pop_front() {
            Some(reply) => reply,
            None => Ok(self.fallback),
        }
    }
}

// ─── Fixtures ───────────────────────────────────────────────────

/// Every timer shrunk so the state machine runs in milliseconds. The values
/// under test are the *relations* between them, and the contract's own numbers
/// are asserted separately (`the_two_timeouts_come_from_the_stored_configuration`).
fn fast_tuning() -> Tuning {
    Tuning {
        waiting_probe_interval: Duration::from_millis(10),
        preloading_probe_interval: Duration::from_millis(10),
        running_probe_interval: Duration::from_millis(10),
        probe_timeout: Duration::from_millis(300),
        stop_grace: Duration::from_millis(150),
        kill_wait: Duration::from_millis(150),
        log_ring_capacity: 4,
        // Bounded and short: the scripted child hands its dying words over
        // immediately, so two empty reads end the drain in about a millisecond.
        final_drain: Duration::from_millis(5),
    }
}

fn default_timeouts() -> Timeouts {
    Timeouts {
        process: Duration::from_millis(400),
        preload: Duration::from_millis(1200),
    }
}

fn default_config() -> ServerConfig {
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

/// A directory that looks like an installed build: `plan_launch` refuses a build
/// whose files are gone, and that refusal is itself a criterion.
fn install_dir_with_server(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("t040-build-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("llama-server.exe"), b"stand-in").unwrap();
    dir
}

fn build_fixture(install_path: PathBuf) -> RuntimeBuild {
    RuntimeBuild {
        build_tag: "b10883".to_string(),
        backend: Backend::Cpu,
        install_path,
        is_active: true,
        installed_at: Utc::now(),
        verified_flags: vec![
            VerifiedFlag {
                name: "--models-preset".to_string(),
                takes_value: true,
                allowed_values: None,
            },
            VerifiedFlag {
                name: "--ctx-size".to_string(),
                takes_value: true,
                allowed_values: None,
            },
        ],
        health_endpoint: Some("/props".to_string()),
        registration_channel: RegistrationChannel::PresetDeclaresPath,
    }
}

fn metadata_fixture() -> GgufMetadata {
    GgufMetadata {
        architecture: "qwen3".to_string(),
        param_count: Some(27_000_000_000),
        quantization: "NVFP4".to_string(),
        block_count: 65,
        context_length: Some(32768),
        embedding_length: Some(5120),
        attention_head_count: Some(40),
        attention_head_count_kv: Some(8),
        attention_key_length: Some(128),
        attention_value_length: Some(128),
        full_attention_interval: Some(4),
        ssm_state_size: None,
        ssm_inner_size: None,
        ssm_group_count: None,
        ssm_conv_kernel: None,
        has_chat_template: true,
        is_moe: false,
        expert_count: None,
        is_draft_model: false,
        has_mtp_heads: false,
        mtp_layer_count: None,
        supports_tools: false,
        supports_thinking: false,
    }
}

fn model_fixture(served_name: &str, preload: bool) -> ModelEntry {
    ModelEntry {
        id: format!("model-{served_name}"),
        display_name: served_name.to_string(),
        served_name: served_name.to_string(),
        file_path: PathBuf::from(format!("C:/models/{served_name}.gguf")),
        shard_paths: Vec::new(),
        size_bytes: 1024,
        sha256_head: "deadbeef".to_string(),
        metadata: metadata_fixture(),
        capability_tags: Vec::new(),
        compatibility: Compatibility::Supported,
        availability: ModelAvailability::Present,
        duplicate_of: None,
        launch_params: LaunchParams::default(),
        sampling_defaults: SamplingDefaults::default(),
        preload,
        pinned: false,
        added_at: Utc::now(),
    }
}

fn loaded(name: &str) -> LoadedModelState {
    LoadedModelState {
        model_id: name.to_string(),
        state: ModelLoadState::Loaded,
        vram_bytes: None,
        last_used: None,
        error: None,
    }
}

fn unloaded(name: &str) -> LoadedModelState {
    LoadedModelState {
        model_id: name.to_string(),
        state: ModelLoadState::Registered,
        vram_bytes: None,
        last_used: None,
        error: None,
    }
}

fn failed(name: &str, error: &str) -> LoadedModelState {
    LoadedModelState {
        model_id: name.to_string(),
        state: ModelLoadState::Failed,
        vram_bytes: None,
        last_used: None,
        error: Some(error.to_string()),
    }
}

// ─── The harness ────────────────────────────────────────────────

struct Harness {
    handle: SupervisorHandle,
    launcher: Arc<ScriptedLauncher>,
    coordinator: Arc<ScriptedCoordinator>,
    events: Arc<Recorder>,
    context: Arc<FakeContext>,
    ports: Arc<ScriptedPorts>,
}

impl Harness {
    fn new(build: RuntimeBuild, models: Vec<ModelEntry>, timeouts: Timeouts) -> Harness {
        let launcher = ScriptedLauncher::new();
        let coordinator = ScriptedCoordinator::new();
        let events = Recorder::new();
        let context = FakeContext::new(build, models);
        let ports = ScriptedPorts::new();
        let handle = spawn(SupervisorDeps {
            context: context.clone(),
            launcher: launcher.clone(),
            coordinator: coordinator.clone(),
            ports: ports.clone(),
            events: events.clone(),
            tuning: fast_tuning(),
            timeouts_override: Some(timeouts),
        });
        Harness {
            handle,
            launcher,
            coordinator,
            events,
            context,
            ports,
        }
    }

    fn state(&self) -> ServerState {
        self.handle.state().unwrap()
    }

    fn is_preloading(&self) -> bool {
        matches!(
            self.state(),
            ServerState::Starting {
                phase: StartupPhase::Preloading { .. },
                ..
            }
        )
    }

    /// Poll the actor until `predicate` holds, then return the state. Panics
    /// with the recorded transition list, which is what makes a failure here
    /// readable instead of a bare timeout.
    fn wait_for<F>(&self, what: &str, predicate: F) -> ServerState
    where
        F: Fn(&ServerState) -> bool,
    {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let state = self.state();
            if predicate(&state) {
                return state;
            }
            if Instant::now() > deadline {
                panic!(
                    "timed out waiting for {what}; state is {state:?}; transitions were {:?}",
                    self.events.labels()
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_running(&self) -> ServerState {
        self.wait_for("Running", |state| {
            matches!(state, ServerState::Running { .. })
        })
    }

    fn wait_stopped(&self) -> ServerState {
        self.wait_for("Stopped", |state| matches!(state, ServerState::Stopped))
    }

    fn wait_crashed(&self) -> ServerState {
        self.wait_for("Crashed", |state| {
            matches!(state, ServerState::Crashed { .. })
        })
    }

    fn wait_preloading(&self) -> ServerState {
        self.wait_for("Preloading", |state| {
            matches!(
                state,
                ServerState::Starting {
                    phase: StartupPhase::Preloading { .. },
                    ..
                }
            )
        })
    }
}

// ─── The legal transitions ──────────────────────────────────────

#[test]
fn a_start_with_no_preload_goes_straight_from_waiting_to_running() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("shortcut")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );

    let state = harness.handle.start().unwrap();
    assert!(
        matches!(
            state,
            ServerState::Starting {
                phase: StartupPhase::WaitingForProcess,
                ..
            }
        ),
        "start must answer with the first phase, got {state:?}"
    );

    harness.wait_running();
    assert_eq!(
        harness.events.phases(),
        vec!["WaitingForProcess".to_string()],
        "with nothing marked preload the shortcut must skip Preloading entirely"
    );
    assert_eq!(
        harness.events.labels(),
        vec!["Starting", "Running"],
        "exactly one legal path from Stopped to Running"
    );
}

#[test]
fn a_coordinator_that_answers_during_a_preload_is_preloading_and_not_running() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("inflight")),
        vec![model_fixture("big", true)],
        default_timeouts(),
    );
    // The coordinator is up; the model is not loaded and stays that way — the
    // case §2 says must NOT read as Running.
    harness
        .coordinator
        .set_models_default(Ok(vec![unloaded("big")]));

    harness.handle.start().unwrap();
    harness.wait_preloading();
    let polls_before = harness.coordinator.models_calls();

    // Give the actor several poll intervals to make the wrong transition if it
    // were going to.
    std::thread::sleep(Duration::from_millis(120));
    assert!(
        harness.coordinator.models_calls() > polls_before,
        "the preload must actually be observed, not assumed"
    );
    assert!(harness.is_preloading(), "state is {:?}", harness.state());
    assert!(
        !harness.events.labels().contains(&"Running".to_string()),
        "Running must not be reported before the preload finishes: {:?}",
        harness.events.labels()
    );
}

#[test]
fn running_is_reached_only_after_every_preload_reports_loaded() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("two-preloads")),
        vec![model_fixture("first", true), model_fixture("second", true)],
        default_timeouts(),
    );
    harness.coordinator.script_models(vec![
        Ok(vec![unloaded("first"), unloaded("second")]),
        Ok(vec![loaded("first"), unloaded("second")]),
        Ok(vec![loaded("first"), loaded("second")]),
    ]);

    harness.handle.start().unwrap();
    harness.wait_running();

    let phases = harness.events.phases();
    assert!(
        phases.contains(&"Preloading(0/2,first)".to_string()),
        "the phase must report how far the preload has got, got {phases:?}"
    );
    assert!(
        phases.contains(&"Preloading(1/2,second)".to_string()),
        "one of two loaded must report the model still to come, got {phases:?}"
    );
    assert_eq!(
        harness.coordinator.models_calls(),
        3,
        "Running required the third reading — the one that reported both models loaded; \
         the first two said otherwise and the machine kept waiting"
    );
    let labels = harness.events.labels();
    assert_eq!(
        labels.iter().filter(|label| *label == "Running").count(),
        1,
        "Running is reached once, after the preloads, got {labels:?}"
    );
    assert_eq!(labels.last().map(String::as_str), Some("Running"));
    let last_preloading = labels
        .iter()
        .rposition(|label| label == "Starting")
        .unwrap();
    assert!(
        last_preloading < labels.len() - 1,
        "Running must come after the last Starting, got {labels:?}"
    );
}

#[test]
fn a_preload_failure_crashes_with_a_diagnosis_naming_the_model() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("broken-preload")),
        vec![model_fixture("broken", true)],
        default_timeouts(),
    );
    harness
        .coordinator
        .set_models_default(Ok(vec![failed("broken", "exit code 7")]));

    harness.handle.start().unwrap();
    let state = harness.wait_crashed();

    let ServerState::Crashed { diagnosis, .. } = state else {
        panic!("expected Crashed");
    };
    let diagnosis = diagnosis.expect("a preload failure must carry a diagnosis");
    assert!(
        diagnosis.message.contains("broken"),
        "the diagnosis must name the model that failed: {}",
        diagnosis.message
    );
    assert!(
        diagnosis.message.contains("exit code 7"),
        "and must carry what the server said: {}",
        diagnosis.message
    );
    assert!(
        !diagnosis.remediation.is_empty(),
        "every diagnosis has a remediation"
    );
}

#[test]
fn a_silent_coordinator_crashes_on_the_process_timeout() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("silent")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    harness.coordinator.set_health_default(Err(network_error()));

    let started = Instant::now();
    harness.handle.start().unwrap();
    let state = harness.wait_crashed();
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(350),
        "the process timeout must actually be waited out, took {elapsed:?}"
    );
    let ServerState::Crashed { diagnosis, .. } = state else {
        panic!("expected Crashed");
    };
    assert_eq!(
        diagnosis
            .expect("a startup timeout carries a diagnosis")
            .code,
        "startup_timeout"
    );
}

#[test]
fn a_preload_that_never_finishes_crashes_on_the_preload_timeout_not_the_process_one() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("slow-preload")),
        vec![model_fixture("slow", true)],
        Timeouts {
            process: Duration::from_millis(300),
            preload: Duration::from_millis(900),
        },
    );
    // The coordinator answers, so the process timeout is satisfied and must not
    // fire: only the preload deadline can end this start.
    harness
        .coordinator
        .set_models_default(Ok(vec![unloaded("slow")]));

    let started = Instant::now();
    harness.handle.start().unwrap();
    let state = harness.wait_crashed();
    let elapsed = started.elapsed();

    assert!(
        elapsed >= Duration::from_millis(850),
        "the preload timeout is the one that must fire here, took {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(2500),
        "and nothing else may be waited for first, took {elapsed:?}"
    );
    let ServerState::Crashed { diagnosis, .. } = state else {
        panic!("expected Crashed");
    };
    let diagnosis = diagnosis.expect("a preload timeout carries a diagnosis");
    assert_eq!(diagnosis.code, "preload_timeout");
    assert!(
        diagnosis.message.contains("slow"),
        "the timeout must name the model still loading: {}",
        diagnosis.message
    );
}

#[test]
fn the_two_timeouts_come_from_the_stored_configuration() {
    let config = ServerConfig {
        process_timeout_seconds: 33,
        preload_timeout_seconds: 77,
        ..default_config()
    };
    let timeouts = Timeouts::from_config(&config);
    assert_eq!(timeouts.process, Duration::from_secs(33));
    assert_eq!(timeouts.preload, Duration::from_secs(77));
    // And the documented defaults are the ones §2 names.
    let defaults = Timeouts::from_config(&default_config());
    assert_eq!(defaults.process, Duration::from_secs(120));
    assert_eq!(defaults.preload, Duration::from_secs(900));
}

// ─── The 3-strike rule ──────────────────────────────────────────

#[test]
fn one_dropped_response_is_not_a_death_but_three_consecutive_are() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("drops")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    // Running, then: one drop, two answers, then three drops.
    harness.coordinator.script_health(vec![
        Ok(()),               // -> Running
        Err(network_error()), // the dropped response
        Ok(()),
        Ok(()),
        Err(network_error()),
        Err(network_error()),
        Err(network_error()),
    ]);

    harness.handle.start().unwrap();
    harness.wait_running();

    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.coordinator.health_calls() < 4 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        matches!(harness.state(), ServerState::Running { .. }),
        "one dropped probe is a dropped probe, not a death: {:?}",
        harness.state()
    );

    let state = harness.wait_crashed();
    let ServerState::Crashed { diagnosis, .. } = state else {
        panic!("expected Crashed");
    };
    assert_eq!(
        diagnosis
            .expect("a silent-but-alive server carries a diagnosis")
            .code,
        "upstream_not_answering"
    );
    assert!(
        harness
            .events
            .lines()
            .iter()
            .any(|line| line.contains("(3/3 consecutive)")),
        "the third failure must be visible in the log: {:?}",
        harness.events.lines()
    );
}

#[test]
fn a_probe_that_exceeds_its_own_timeout_counts_as_a_failure() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("stuck-probe")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    // Answering normally reaches Running; then the coordinator starts blocking
    // for longer than the probe timeout, so the actor must count each probe as
    // failed rather than wait for it forever.
    harness.handle.start().unwrap();
    harness.wait_running();
    harness
        .coordinator
        .set_health_delay(Duration::from_millis(400));

    let state = harness.wait_crashed();
    let ServerState::Crashed { diagnosis, .. } = state else {
        panic!("expected Crashed");
    };
    assert_eq!(
        diagnosis.expect("a timed-out probe is a failure").code,
        "upstream_not_answering"
    );
    assert!(
        harness
            .events
            .lines()
            .iter()
            .any(|line| line.contains("within its own timeout")),
        "the log must name the reason: {:?}",
        harness.events.lines()
    );
}

// ─── Rejected commands ──────────────────────────────────────────

#[test]
fn every_rejected_command_returns_invalid_transition_and_changes_nothing() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("rejects")),
        vec![model_fixture("big", true)],
        default_timeouts(),
    );
    harness
        .coordinator
        .set_models_default(Ok(vec![unloaded("big")]));

    // Stopped: stop and dismiss are both illegal.
    assert_invalid(
        &harness.handle.stop().unwrap_err(),
        "Stopped",
        "stop_server",
    );
    assert_invalid(
        &harness.handle.dismiss_crash().unwrap_err(),
        "Stopped",
        "dismiss_crash",
    );

    // Starting: start and dismiss are illegal.
    harness.handle.start().unwrap();
    harness.wait_preloading();
    let before = snapshot(&harness.state());
    assert_invalid(
        &harness.handle.start().unwrap_err(),
        "Starting",
        "start_server",
    );
    assert_invalid(
        &harness.handle.dismiss_crash().unwrap_err(),
        "Starting",
        "dismiss_crash",
    );
    assert_eq!(
        snapshot(&harness.state()),
        before,
        "a rejected command must leave the state alone"
    );

    // Stopping: start is illegal, and stop is illegal once it is already there.
    harness.handle.stop().unwrap();
    assert_invalid(
        &harness.handle.start().unwrap_err(),
        "Stopping",
        "start_server",
    );
    assert_invalid(
        &harness.handle.stop().unwrap_err(),
        "Stopping",
        "stop_server",
    );
    harness.wait_stopped();
}

fn assert_invalid(error: &AppError, from: &str, command: &str) {
    match error {
        AppError::InvalidTransition {
            from: actual_from,
            command: actual_command,
        } => {
            assert_eq!(actual_from, from, "wrong `from` in {error:?}");
            assert_eq!(actual_command, command, "wrong `command` in {error:?}");
        }
        other => panic!("expected InvalidTransition, got {other:?}"),
    }
}

fn snapshot(state: &ServerState) -> serde_json::Value {
    serde_json::to_value(state).unwrap()
}

#[test]
fn activating_or_removing_a_runtime_is_rejected_while_the_server_is_not_stopped() {
    assert!(reject_unless_stopped(&ServerState::Stopped, "activate_runtime").is_ok());
    let running = ServerState::Running {
        pid: 1,
        upstream_port: 49700,
        since: Utc::now(),
        config_dirty: false,
    };
    assert_invalid(
        &reject_unless_stopped(&running, "activate_runtime").unwrap_err(),
        "Running",
        "activate_runtime",
    );
    assert_invalid(
        &reject_unless_stopped(&running, "remove_runtime").unwrap_err(),
        "Running",
        "remove_runtime",
    );
    let crashed = ServerState::Crashed {
        exit_code: Some(1),
        diagnosis: None,
        last_log: Vec::new(),
    };
    assert!(
        reject_unless_stopped(&crashed, "remove_runtime").is_err(),
        "§2 rejects these while the state is not Stopped, Crashed included"
    );
}

// ─── Concurrency, stopping, crashing ────────────────────────────

#[test]
fn concurrent_start_calls_resolve_with_exactly_one_winner() {
    let launcher = ScriptedLauncher::new();
    let coordinator = ScriptedCoordinator::new();
    coordinator.set_models_default(Ok(vec![unloaded("big")]));
    let events = Recorder::new();
    let handle = Arc::new(spawn(SupervisorDeps {
        context: FakeContext::new(
            build_fixture(install_dir_with_server("concurrent")),
            vec![model_fixture("big", true)],
        ),
        launcher: launcher.clone(),
        coordinator,
        ports: ScriptedPorts::new(),
        events: events.clone(),
        tuning: fast_tuning(),
        timeouts_override: Some(default_timeouts()),
    }));

    let barrier = Arc::new(std::sync::Barrier::new(8));
    let mut threads = Vec::new();
    for _ in 0..8 {
        let handle = handle.clone();
        let barrier = barrier.clone();
        threads.push(std::thread::spawn(move || {
            barrier.wait();
            handle.start()
        }));
    }
    let results: Vec<Result<ServerState, AppError>> = threads
        .into_iter()
        .map(|thread| thread.join().unwrap())
        .collect();

    let winners = results.iter().filter(|result| result.is_ok()).count();
    let losers = results
        .iter()
        .filter(|result| matches!(result, Err(AppError::InvalidTransition { .. })))
        .count();
    assert_eq!(winners, 1, "exactly one start may win: {results:?}");
    assert_eq!(
        losers, 7,
        "the rest must be rejected as transitions: {results:?}"
    );
    assert_eq!(
        launcher.spawn_count(),
        1,
        "one winner means exactly one process spawned"
    );
}

#[test]
fn stopping_during_startup_kills_the_partial_process() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("stop-partial")),
        vec![model_fixture("big", true)],
        default_timeouts(),
    );
    // A child that refuses the polite request, and a coordinator that never
    // answers: the start is still in its first phase when the stop arrives.
    let child = harness.launcher.push(ChildState::default());
    harness.coordinator.set_health_default(Err(network_error()));

    harness.handle.start().unwrap();
    harness.wait_for("WaitingForProcess", |state| {
        matches!(
            state,
            ServerState::Starting {
                phase: StartupPhase::WaitingForProcess,
                ..
            }
        )
    });
    let reply = harness.handle.stop().unwrap();
    assert!(
        matches!(reply, ServerState::Stopping),
        "stop answers immediately with Stopping, got {reply:?}"
    );
    harness.wait_stopped();

    assert_eq!(
        child.lock().unwrap().kill_tree_calls,
        1,
        "the partially started process must be killed, not left running"
    );
}

#[test]
fn the_grace_timeout_escalates_to_a_tree_kill() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("grace")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    // Refuses the polite request (as the real router does) and never exits.
    let child = harness.launcher.push(ChildState::default());

    harness.handle.start().unwrap();
    harness.wait_running();
    let sent = Instant::now();
    harness.handle.stop().unwrap();
    harness.wait_stopped();
    let elapsed = sent.elapsed();

    assert!(
        elapsed >= Duration::from_millis(140),
        "the grace window must be honoured before the kill, took {elapsed:?}"
    );
    assert_eq!(child.lock().unwrap().kill_tree_calls, 1);
    assert!(
        harness
            .events
            .lines()
            .iter()
            .any(|line| line.contains("refused the graceful stop request")),
        "a refused polite request must be visible: {:?}",
        harness.events.lines()
    );
    assert!(
        harness
            .events
            .lines()
            .iter()
            .any(|line| line.contains("grace window")),
        "the escalation must be visible: {:?}",
        harness.events.lines()
    );
}

#[test]
fn a_child_that_honours_the_stop_request_is_not_tree_killed() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("polite")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    let child = harness.launcher.push(ChildState {
        grace_accepts: true,
        ..ChildState::default()
    });

    harness.handle.start().unwrap();
    harness.wait_running();
    harness.handle.stop().unwrap();
    harness.wait_stopped();

    let lines = harness.events.lines();
    assert!(
        !lines.iter().any(|line| line.contains("grace window")),
        "a child that left on request must not reach the escalation: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line.contains("terminated ")),
        "and the sweep over the dead root must find nothing to terminate: {lines:?}"
    );
    assert_eq!(
        child.lock().unwrap().exit_code,
        Some(0),
        "the exit status is the child's own, not one this app imposed"
    );
}

#[test]
fn a_child_that_exits_on_its_own_crashes_with_its_exit_code() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("self-exit")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    let child = harness.launcher.push(ChildState::default());
    harness.coordinator.set_health_default(Err(network_error()));

    harness.handle.start().unwrap();
    harness.wait_for("WaitingForProcess", |state| {
        matches!(state, ServerState::Starting { .. })
    });
    std::thread::sleep(Duration::from_millis(60));
    {
        let mut state = child.lock().unwrap();
        state.exit_code = Some(9);
        state.logs.push(LogLine {
            at: Utc::now(),
            level: "error".to_string(),
            line: "0.00.001.000 E srv  llama_server: could not load model".to_string(),
        });
        // ...and the line it writes as it dies, handed over only once the exit
        // status is observed. A crash built on the exit alone loses this one,
        // and it is the line a diagnosis reads.
        state.dying_words.push(LogLine {
            at: Utc::now(),
            level: "error".to_string(),
            line: "0.00.001.100 E srv  llama_server: failed to load model, exiting".to_string(),
        });
    }

    let state = harness.wait_crashed();
    let ServerState::Crashed {
        exit_code,
        last_log,
        diagnosis,
    } = state
    else {
        panic!("expected Crashed");
    };
    assert_eq!(exit_code, Some(9));
    assert!(
        diagnosis.is_none(),
        "an unexplained exit carries no invented cause until T-043 maps stderr"
    );
    assert!(
        last_log
            .iter()
            .any(|line| line.contains("could not load model")),
        "the crash must carry the child's own last words: {last_log:?}"
    );
    assert!(
        last_log
            .iter()
            .any(|line| line.contains("failed to load model, exiting")),
        "the tail must carry what the process wrote as it died, not only what \
         happened to be drained before the exit was seen: {last_log:?}"
    );
    assert!(
        last_log.len() <= 4,
        "the tail is capped by the ring buffer: {last_log:?}"
    );
}

#[test]
fn dismiss_crash_returns_to_stopped_and_a_crash_can_also_just_be_restarted() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("dismiss")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    harness.launcher.push(ChildState {
        exit_code: Some(1),
        ..ChildState::default()
    });

    harness.handle.start().unwrap();
    harness.wait_crashed();
    assert!(matches!(
        harness.handle.dismiss_crash().unwrap(),
        ServerState::Stopped
    ));
    assert_invalid(
        &harness.handle.dismiss_crash().unwrap_err(),
        "Stopped",
        "dismiss_crash",
    );

    // Crashed is also a legal starting point without a dismissal.
    let harness2 = Harness::new(
        build_fixture(install_dir_with_server("dismiss2")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    harness2.launcher.push(ChildState {
        exit_code: Some(1),
        ..ChildState::default()
    });
    harness2.handle.start().unwrap();
    harness2.wait_crashed();
    assert!(
        harness2.handle.start().is_ok(),
        "start_server from Crashed is legal"
    );
}

// ─── The launch plan and the failures that precede it ───────────

#[test]
fn the_upstream_is_always_loopback_and_the_client_port_never_reaches_it() {
    let mut build = build_fixture(install_dir_with_server("loopback"));
    // A configuration that points the *client-facing* listener elsewhere must
    // not move the upstream.
    let config = ServerConfig {
        listen_address: "0.0.0.0".parse().unwrap(),
        listen_port: 8099,
        ..default_config()
    };
    let args = crate::core::preset_generator::router_arguments(
        &build,
        49777,
        Some(std::path::Path::new("C:/tmp/presets.ini")),
        None,
    )
    .unwrap();
    let launch = plan_launch(&build, args).unwrap();

    assert_eq!(launch.program.file_name().unwrap(), "llama-server.exe");
    assert_eq!(launch.cwd.as_deref(), Some(build.install_path.as_path()));
    let joined = launch.args.join(" ");
    assert!(
        joined.contains("--host 127.0.0.1"),
        "the upstream is loopback, always: {joined}"
    );
    assert!(
        joined.contains("--port 49777"),
        "the upstream port is the allocated one: {joined}"
    );
    assert!(
        joined.contains("--no-webui"),
        "no UI on the upstream: {joined}"
    );
    assert!(
        !joined.contains("0.0.0.0") && !joined.contains(&config.listen_port.to_string()),
        "the client-facing address must not leak into the upstream: {joined}"
    );
    assert!(
        !joined.contains("8080"),
        "nor the default client port: {joined}"
    );

    // A build whose files are missing must say so rather than fail obscurely.
    build.install_path = PathBuf::from("C:/definitely/not/here");
    let error = plan_launch(&build, Vec::new()).unwrap_err();
    assert!(matches!(error, AppError::NotFound { .. }), "got {error:?}");
}

#[test]
fn an_unverified_build_refuses_to_start_rather_than_guessing_the_endpoint() {
    let mut build = build_fixture(install_dir_with_server("unverified"));
    build.health_endpoint = None;
    let harness = Harness::new(
        build,
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );

    let error = harness.handle.start().unwrap_err();
    assert!(
        matches!(error, AppError::Internal { .. }) && error.message().contains("health endpoint"),
        "got {error:?}"
    );
    assert!(matches!(harness.state(), ServerState::Stopped));
    assert_eq!(harness.launcher.spawn_count(), 0);
}

#[test]
fn the_recorded_health_endpoint_is_the_one_probed() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("endpoint")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );

    harness.handle.start().unwrap();
    harness.wait_running();
    assert_eq!(
        harness.coordinator.probed_endpoints(),
        vec!["/props".to_string()],
        "the app probes the endpoint T-023 recorded, never an invented one"
    );
}

#[test]
fn a_port_that_cannot_be_allocated_leaves_the_state_alone() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("no-port")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    harness.ports.fail_with(AppError::PortInUse { port: 49999 });

    let error = harness.handle.start().unwrap_err();
    assert_eq!(error, AppError::PortInUse { port: 49999 });
    assert!(matches!(harness.state(), ServerState::Stopped));
    assert_eq!(
        harness.launcher.spawn_count(),
        0,
        "no process may be spawned without a port"
    );
    assert!(!error.remediation().unwrap_or_default().is_empty());
}

#[test]
fn a_start_with_no_active_build_reports_not_found_and_changes_nothing() {
    let launcher = ScriptedLauncher::new();
    let context = Arc::new(FakeContext {
        config: default_config(),
        build: Err(AppError::NotFound {
            what: "an active runtime build".to_string(),
        }),
        models: Vec::new(),
        warnings: Vec::new(),
        registration_calls: AtomicUsize::new(0),
    });
    let handle = spawn(SupervisorDeps {
        context,
        launcher: launcher.clone(),
        coordinator: ScriptedCoordinator::new(),
        ports: ScriptedPorts::new(),
        events: Recorder::new(),
        tuning: fast_tuning(),
        timeouts_override: Some(default_timeouts()),
    });

    let error = handle.start().unwrap_err();
    assert!(matches!(error, AppError::NotFound { .. }), "got {error:?}");
    assert!(matches!(handle.state().unwrap(), ServerState::Stopped));
    assert_eq!(launcher.spawn_count(), 0);
}

#[test]
fn omitted_flags_surface_as_preset_warnings() {
    let build = build_fixture(install_dir_with_server("warnings"));
    let context = Arc::new(FakeContext::new(build, vec![model_fixture("plain", false)]));
    let events = Recorder::new();
    let launcher = ScriptedLauncher::new();
    let handle = spawn(SupervisorDeps {
        context: Arc::new(FakeContext {
            config: default_config(),
            build: context.build.clone(),
            models: vec![model_fixture("plain", false)],
            warnings: vec![PresetWarning {
                model: "plain".to_string(),
                flag: "--no-mmap".to_string(),
                reason: "the build does not carry this flag".to_string(),
            }],
            registration_calls: AtomicUsize::new(0),
        }),
        launcher: launcher.clone(),
        coordinator: ScriptedCoordinator::new(),
        ports: ScriptedPorts::new(),
        events: events.clone(),
        tuning: fast_tuning(),
        timeouts_override: Some(default_timeouts()),
    });

    handle.start().unwrap();
    let warnings = events.warnings.lock().unwrap().clone();
    assert_eq!(
        warnings.len(),
        1,
        "an omitted flag must be reported, not silently dropped"
    );
    assert_eq!(warnings[0].flag, "--no-mmap");
    assert!(
        handle.stop().is_ok(),
        "the harness must stop what it started"
    );
    assert!(launcher.spawn_count() == 1);
}

#[test]
fn every_line_the_child_writes_reaches_the_events() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("logs")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    let child = harness.launcher.push(ChildState::default());

    harness.handle.start().unwrap();
    harness.wait_running();
    assert!(
        harness.launcher.last_launch().is_some(),
        "the launch plan reaches the launcher"
    );
    assert_eq!(
        harness.context.registration_calls(),
        1,
        "the preset is written once per start"
    );

    for index in 0..10 {
        child.lock().unwrap().logs.push(LogLine {
            at: Utc::now(),
            level: "info".to_string(),
            line: format!("0.00.000.001 I srv line-{index}"),
        });
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.events.lines().len() < 10 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let lines = harness.events.lines();
    assert!(
        lines.iter().any(|line| line.contains("line-9")),
        "every line the child writes must be emitted: {lines:?}"
    );
}

#[test]
fn config_dirty_is_only_set_while_running() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("dirty")),
        vec![model_fixture("plain", false)],
        default_timeouts(),
    );
    harness.handle.mark_config_dirty().unwrap();
    assert!(matches!(harness.state(), ServerState::Stopped));

    harness.handle.start().unwrap();
    harness.wait_running();
    harness.handle.mark_config_dirty().unwrap();
    match harness.state() {
        ServerState::Running { config_dirty, .. } => assert!(config_dirty),
        other => panic!("expected Running, got {other:?}"),
    }
}

// ─── Parsing the router's own report ────────────────────────────

#[test]
fn the_captured_models_payload_is_read_as_the_contract_states() {
    // Verbatim from the installed b10883 build (PROGRESS.md F-020), including
    // the shape a failed preload takes.
    let body = r#"{"data":[
        {"id":"t040-probe","aliases":[],"tags":[],"object":"model","owned_by":"llamacpp",
         "created":1789908330,
         "status":{"value":"unloaded","args":["llama-server.exe","--port","60647"],
                   "preset":"[t040-probe]\n","exit_code":1,"failed":true},
         "architecture":{"input_modalities":["text"],"output_modalities":["text"]},
         "source":"preset","can_remove":false},
        {"id":"idle","status":{"value":"unloaded"},"source":"preset"},
        {"id":"warm","status":{"value":"loading"},"source":"preset"},
        {"id":"ready","status":{"value":"loaded"},"source":"preset"},
        {"id":"asleep","status":{"value":"sleeping"},"source":"preset"},
        {"id":"futuristic","status":{"value":"quantum"},"source":"preset"}
      ],"object":"list"}"#;

    let models = parse_router_models(body).unwrap();
    let state_of = |name: &str| {
        models
            .iter()
            .find(|model| model.model_id == name)
            .map(|model| model.state.clone())
    };
    assert_eq!(state_of("t040-probe"), Some(ModelLoadState::Failed));
    assert!(
        models
            .iter()
            .find(|model| model.model_id == "t040-probe")
            .and_then(|model| model.error.clone())
            .unwrap_or_default()
            .contains("exit code 1"),
        "the child's exit code must survive into the error"
    );
    assert_eq!(state_of("idle"), Some(ModelLoadState::Registered));
    assert_eq!(state_of("warm"), Some(ModelLoadState::Loading));
    assert_eq!(state_of("ready"), Some(ModelLoadState::Loaded));
    assert_eq!(state_of("asleep"), Some(ModelLoadState::Loaded));
    assert_eq!(
        state_of("futuristic"),
        Some(ModelLoadState::Registered),
        "an unknown status is counted as neither loaded nor failed"
    );

    // Malformed input is a typed error, never a panic.
    assert!(matches!(
        parse_router_models("not json").unwrap_err(),
        AppError::Network { .. }
    ));
    assert!(matches!(
        parse_router_models("{}").unwrap_err(),
        AppError::Network { .. }
    ));
}

// ─── Real processes ─────────────────────────────────────────────

/// The stand-in is a real `cmd.exe` that spawns a real child (`ping`) and never
/// answers anything: the supervisor must notice on its own deadline and leave
/// no process of the tree behind.
#[cfg(windows)]
fn stand_in_launcher() -> Arc<StandInLauncher> {
    Arc::new(StandInLauncher {
        program: PathBuf::from("cmd.exe"),
        args: vec![
            "/C".to_string(),
            "cmd /C ping -n 120 127.0.0.1 >NUL".to_string(),
        ],
        inner: SystemLauncher::new(),
    })
}

/// The pid the supervisor logged for the child it spawned — read back rather
/// than assumed, since it is the handle the whole stop sequence works from.
#[cfg(windows)]
fn spawned_pid(events: &Arc<Recorder>) -> u32 {
    events
        .lines()
        .iter()
        .find_map(|line| {
            let marker = "(pid ";
            let start = line.find(marker)? + marker.len();
            let rest = &line[start..];
            let end = rest.find(')')?;
            rest[..end].parse().ok()
        })
        .unwrap_or(0)
}

#[cfg(windows)]
fn wait_for_state<F>(handle: &SupervisorHandle, what: &str, predicate: F) -> ServerState
where
    F: Fn(&ServerState) -> bool,
{
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let state = handle.state().unwrap();
        if predicate(&state) {
            return state;
        }
        if Instant::now() > deadline {
            panic!("timed out waiting for {what}; state is {state:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(windows)]
#[test]
fn a_real_child_that_never_answers_crashes_on_the_timeout_and_leaves_nothing_behind() {
    let events = Recorder::new();
    let coordinator = ScriptedCoordinator::new();
    coordinator.set_health_default(Err(network_error()));
    let handle = spawn(SupervisorDeps {
        context: FakeContext::new(
            build_fixture(install_dir_with_server("real-crash")),
            vec![model_fixture("plain", false)],
        ),
        launcher: stand_in_launcher(),
        coordinator,
        ports: ScriptedPorts::new(),
        events: events.clone(),
        tuning: fast_tuning(),
        timeouts_override: Some(Timeouts {
            process: Duration::from_millis(700),
            preload: Duration::from_millis(2000),
        }),
    });

    handle.start().unwrap();
    let pid = {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut pid = spawned_pid(&events);
        while pid == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            pid = spawned_pid(&events);
        }
        pid
    };
    assert!(
        pid != 0,
        "the supervisor must log the pid of the child it spawned"
    );
    assert!(
        process::tree::pid_alive(pid),
        "the stand-in child ({pid}) must be alive while the supervisor waits"
    );

    let state = wait_for_state(&handle, "Crashed", |state| {
        matches!(state, ServerState::Crashed { .. })
    });
    let ServerState::Crashed { diagnosis, .. } = state else {
        panic!("expected Crashed, got {state:?}");
    };
    assert_eq!(
        diagnosis
            .map(|diagnosis| diagnosis.code)
            .unwrap_or_default(),
        "startup_timeout"
    );
    assert!(
        !process::tree::pid_alive(pid),
        "the crashed child {pid} must be reaped, not orphaned"
    );
}

#[cfg(windows)]
#[test]
fn stopping_a_real_child_during_startup_leaves_no_process_in_the_tree() {
    let events = Recorder::new();
    let coordinator = ScriptedCoordinator::new();
    coordinator.set_health_default(Err(network_error()));
    let handle = spawn(SupervisorDeps {
        context: FakeContext::new(
            build_fixture(install_dir_with_server("real-stop")),
            vec![model_fixture("plain", false)],
        ),
        launcher: stand_in_launcher(),
        coordinator,
        ports: ScriptedPorts::new(),
        events: events.clone(),
        tuning: Tuning {
            // Short grace so the escalation runs inside the test, long enough
            // that the polite request is genuinely attempted first.
            stop_grace: Duration::from_millis(300),
            ..fast_tuning()
        },
        timeouts_override: Some(Timeouts {
            process: Duration::from_secs(30),
            preload: Duration::from_secs(30),
        }),
    });

    handle.start().unwrap();
    let pid = {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut pid = spawned_pid(&events);
        while pid == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
            pid = spawned_pid(&events);
        }
        pid
    };
    assert!(pid != 0, "the supervisor must log the pid it spawned");

    // Let the stand-in spawn its own child, so the tree has two levels to reap.
    let children: Vec<u32> = {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let found = process::tree::descendants(pid).unwrap_or_default();
            if !found.is_empty() || Instant::now() > deadline {
                break found;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert!(
        !children.is_empty(),
        "the stand-in must have a child process for a tree kill to mean anything"
    );

    handle.stop().unwrap();
    let stopped = wait_for_state(&handle, "Stopped", |state| {
        matches!(state, ServerState::Stopped)
    });
    assert_eq!(state_label(&stopped), "Stopped");
    assert!(
        !process::tree::pid_alive(pid),
        "the root {pid} must not survive the stop"
    );
    for child in &children {
        assert!(
            !process::tree::pid_alive(*child),
            "child {child} was orphaned by the stop"
        );
    }
}

// ─── The installed build, end to end (env-gated) ────────────────

/// Drives the **real** pipeline — the database's own active build and catalogue,
/// the real preset writer, the real `llama-server.exe`, the real HTTP
/// coordinator — far enough to prove the start path works on this machine.
///
/// Gated on `LLAMA_MANAGER_SUPERVISOR_E2E=1` because it needs an installed
/// build; `AGENTS.md` §3 keeps such a criterion out of the acceptance list for
/// exactly that reason, and this is run by hand when a hand-over build is
/// prepared. It writes `presets.ini` where a real start would, which is the
/// app's own file.
#[cfg(windows)]
#[test]
fn the_installed_build_starts_and_stops_without_leaving_a_process_behind() {
    if std::env::var("LLAMA_MANAGER_SUPERVISOR_E2E").as_deref() != Ok("1") {
        return;
    }
    let events = Recorder::new();
    let deps = crate::core::supervisor::production_deps(events.clone(), Tuning::default())
        .expect("production deps");
    let handle = spawn(deps);

    let started = handle.start().expect("the installed build must start");
    println!("start replied: {started:?}");
    let running = wait_for_state(&handle, "Running", |state| {
        matches!(state, ServerState::Running { .. })
    });
    println!("reached: {running:?}");
    let ServerState::Running {
        pid, upstream_port, ..
    } = running
    else {
        panic!("expected Running");
    };
    assert!(
        process::tree::pid_alive(pid),
        "the server process {pid} must be alive while Running"
    );
    println!("upstream port {upstream_port}, pid {pid}");

    let stopping = handle.stop().expect("stop must be accepted");
    println!("stop replied: {stopping:?}");
    let stopped = wait_for_state(&handle, "Stopped", |state| {
        matches!(state, ServerState::Stopped)
    });
    println!("reached: {stopped:?}");
    assert!(
        !process::tree::pid_alive(pid),
        "the server process {pid} must be gone after the stop"
    );
    for line in events.lines() {
        println!("  log: {line}");
    }
}
// ─── The listener's lifecycle (T-042) ───────────────────────────

/// `docs/CONTRACTS.md` §2: **"`EndpointState` is not part of `ServerState` and
/// does not transition with it"** — but it *is* owned by the same actor, so a
/// `get_server_state` and a `get_endpoint_state` are a coherent pair rather
/// than two snapshots taken at different times. This drives that ownership
/// through the public handle, which is how the IPC layer reaches it.
#[test]
fn the_endpoint_state_is_owned_by_the_actor_and_broadcast_when_it_changes() {
    let harness = Harness::new(
        build_fixture(install_dir_with_server("endpoint")),
        Vec::new(),
        default_timeouts(),
    );

    // Nothing has tried to bind yet. That is a different fact from a bind that
    // failed, and T-045's dashboard renders them differently.
    assert_eq!(
        harness.handle.endpoint_state().unwrap(),
        EndpointState::Unbound
    );
    assert!(harness.events.endpoints().is_empty());

    let address = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
    let bound = EndpointState::Bound {
        address,
        port: 8080,
    };
    assert_eq!(
        harness.handle.report_endpoint_state(bound.clone()).unwrap(),
        bound
    );
    assert_eq!(harness.handle.endpoint_state().unwrap(), bound);
    assert_eq!(harness.events.endpoints(), vec![bound.clone()]);

    // A repeat is not a change: one report, one event, or the renderer redraws
    // for nothing.
    harness.handle.report_endpoint_state(bound).unwrap();
    assert_eq!(harness.events.endpoints().len(), 1);

    let failed = EndpointState::BindFailed {
        address,
        port: 8080,
        error: AppError::PortInUse { port: 8080 },
    };
    harness
        .handle
        .report_endpoint_state(failed.clone())
        .unwrap();
    assert_eq!(harness.handle.endpoint_state().unwrap(), failed);
    assert_eq!(harness.events.endpoints().len(), 2);

    // And the server did not move: the two lifecycles are separate values with
    // one owner, which is the whole point of §2's split.
    assert!(matches!(harness.state(), ServerState::Stopped));
}

/// `PLAN.md` §2.7 and §6, and `docs/CONTRACTS.md` §1, all state that the app's
/// listener defaults to `127.0.0.1`; binding `0.0.0.0` "exposes an inference
/// server to the LAN and requires explicit confirmation naming that
/// consequence", and with no API key configured (T-051) it also raises a
/// persistent warning. T-042 is the task that makes the stored address
/// operational — it is what the listener binds — so the default is pinned here
/// rather than left to a value nobody reads.
#[test]
fn the_default_listen_address_is_loopback() {
    let config = default_server_config();
    assert_eq!(
        config.listen_address,
        std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
    );
    // The two values T-042 reads besides the address: the port is the app's
    // contract with configured clients, and an unset limit means the app
    // imposes none of its own (`max_concurrent_requests`).
    assert_eq!(config.listen_port, 8080);
    assert_eq!(config.max_concurrent_requests, None);
}
