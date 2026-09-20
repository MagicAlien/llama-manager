//! T-041's acceptance suite (`docs/TASKS.md` T-041).
//!
//! "Against a stub HTTP server" is taken literally: every test below drives the
//! real [`RouterHttp`](super::orchestrator::RouterHttp) client and the real
//! state machine against a **local stub upstream** — a `TcpListener` on an
//! ephemeral loopback port that answers the four routes the installed build
//! installed (`/models`, `/models/load`, `/models/unload`, `/props`), records
//! every request it was asked for, and can be told to answer slowly, to fail, or
//! to report a failed load. Nothing here needs a GPU, a model file, or the
//! target machine (`AGENTS.md` §3).
//!
//! The two double-free sockets of the suite — the events sink and the history
//! sink — are recording doubles, and one test uses the **real** database (a
//! temporary file opened through `db::open`, so the row is read back through the
//! same queries the app uses).

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::orchestrator::{
    backoff_interval, launch_records_at, DbLaunchHistory, LaunchAttempt, LaunchHistorySink,
    ModelDirectory, ModelOrchestrator, NoopOrchestrator, OrchestratedModel, OrchestratorDeps,
    OrchestratorEvents, RouterHttp, RouterOrchestrator, Tuning, UpstreamPort, LOAD_PATH,
    MODELS_PATH, PROPS_PATH, UNLOAD_PATH,
};
use crate::core::types::{
    AppError, LaunchParams, LoadOutcome, LoadedModelState, ModelLoadState, ServerProps,
};

// ─── The stub upstream ──────────────────────────────────────────

#[derive(Clone, Debug)]
struct StubModel {
    name: String,
    status: &'static str,
    /// `Some(exit_code)`: the router's own way of reporting a failed load —
    /// `unloaded` **plus** `failed: true` and the child's exit code (F-020).
    failed: Option<i32>,
}

impl StubModel {
    fn unloaded(name: &str) -> Self {
        StubModel {
            name: name.to_string(),
            status: "unloaded",
            failed: None,
        }
    }

    fn loading(name: &str) -> Self {
        StubModel {
            name: name.to_string(),
            status: "loading",
            failed: None,
        }
    }

    fn loaded(name: &str) -> Self {
        StubModel {
            name: name.to_string(),
            status: "loaded",
            failed: None,
        }
    }

    fn failed(name: &str, exit_code: i32) -> Self {
        StubModel {
            name: name.to_string(),
            status: "unloaded",
            failed: Some(exit_code),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        let mut status = serde_json::json!({
            "value": self.status,
            "args": ["llama-server.exe", "--port", "0", "--model", "C:/models/x.gguf"],
        });
        if let Some(code) = self.failed {
            status["exit_code"] = serde_json::json!(code);
            status["failed"] = serde_json::json!(true);
        }
        serde_json::json!({
            "id": self.name,
            "aliases": [],
            "tags": [],
            "object": "model",
            "owned_by": "llamacpp",
            "created": 1789917063_u64,
            "status": status,
            "architecture": {
                "input_modalities": ["text"],
                "output_modalities": ["text"],
            },
            "source": "preset",
            "can_remove": false,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Request {
    method: String,
    path: String,
    body: String,
}

struct StubState {
    models: Vec<StubModel>,
    /// Applied immediately when a load/unload POST is accepted.
    after_post: Option<Vec<StubModel>>,
    /// Applied after each `/models` answer: the next state the server moves to,
    /// which is how a load that takes several polls is scripted.
    script: VecDeque<Vec<StubModel>>,
    load_answer: (u16, String),
    unload_answer: (u16, String),
    /// Statuses served by the next `/models` calls, before the models answer.
    poll_errors: VecDeque<u16>,
    /// After this many *successful* `/models` answers, every later one fails —
    /// how "the server stopped answering while a load was in flight" is
    /// scripted without a sleep.
    fail_after: Option<usize>,
    successes: usize,
    requests: Vec<Request>,
    props: String,
}

/// A handle on the running stub: the test mutates what the server says, then
/// asserts on what the app did.
struct Stub {
    port: u16,
    state: Arc<Mutex<StubState>>,
}

impl Stub {
    fn set_models(&self, models: Vec<StubModel>) {
        self.lock().models = models;
    }

    fn set_after_post(&self, models: Vec<StubModel>) {
        self.lock().after_post = Some(models);
    }

    fn set_script(&self, steps: Vec<Vec<StubModel>>) {
        self.lock().script = steps.into_iter().collect();
    }

    fn set_load_answer(&self, status: u16, body: &str) {
        self.lock().load_answer = (status, body.to_string());
    }

    fn set_unload_answer(&self, status: u16, body: &str) {
        self.lock().unload_answer = (status, body.to_string());
    }

    fn fail_polls(&self, statuses: Vec<u16>) {
        self.lock().poll_errors = statuses.into_iter().collect();
    }

    /// Answer normally for `after` polls, then fail every later one.
    fn fail_after_successes(&self, after: usize) {
        self.lock().fail_after = Some(after);
    }

    fn requests(&self) -> Vec<Request> {
        self.lock().requests.clone()
    }

    fn requests_to(&self, method: &str, path: &str) -> Vec<Request> {
        self.requests()
            .into_iter()
            .filter(|request| request.method == method && request.path == path)
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, StubState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// The router's own error shape, verbatim from the installed build's answers
/// (`{"error":{"code":400,"message":"model is not running","type":"invalid_request_error"}}`).
fn error_body(message: &str, kind: &str, code: u16) -> String {
    serde_json::json!({"error": {"message": message, "type": kind, "code": code}}).to_string()
}

fn models_body(models: &[StubModel]) -> String {
    serde_json::json!({
        "data": models.iter().map(StubModel::to_json).collect::<Vec<_>>(),
        "object": "list",
    })
    .to_string()
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}

/// The `build_info` the installed build answers, so the props assertion is a
/// real value rather than a placeholder.
const STUB_PROPS: &str = r#"{"role":"router","max_instances":4,"models_autoload":true,"model_alias":"llama-server","model_path":"none","default_generation_settings":{"params":null,"n_ctx":0},"ui_settings":{},"build_info":"b10883-91f6a6cf3","cors_proxy_enabled":false}"#;

/// Start the stub on an ephemeral loopback port. It lives for the process
/// lifetime: the tests own the port and never need to shut it down.
fn start_stub() -> Stub {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    let port = listener.local_addr().expect("the stub's address").port();
    let state = Arc::new(Mutex::new(StubState {
        models: Vec::new(),
        after_post: None,
        script: VecDeque::new(),
        load_answer: (200, r#"{"success":true}"#.to_string()),
        unload_answer: (200, r#"{"success":true}"#.to_string()),
        poll_errors: VecDeque::new(),
        fail_after: None,
        successes: 0,
        requests: Vec::new(),
        props: STUB_PROPS.to_string(),
    }));
    let worker_state = Arc::clone(&state);

    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else {
                continue;
            };
            let Some((method, path, body)) = read_request(&mut socket) else {
                continue;
            };

            let mut state = match worker_state.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            state.requests.push(Request {
                method: method.clone(),
                path: path.clone(),
                body,
            });

            let (status, body) = match (method.as_str(), path.as_str()) {
                ("GET", MODELS_PATH) => {
                    let dead = matches!(state.fail_after, Some(after) if state.successes >= after);
                    match state.poll_errors.pop_front().or(dead.then_some(503)) {
                        Some(status) => (
                            status,
                            error_body(
                                "the stub was told to fail this poll",
                                "server_error",
                                status,
                            ),
                        ),
                        None => {
                            state.successes += 1;
                            let body = models_body(&state.models);
                            // The state the server moves to *after* this answer, so a
                            // load that takes several polls is deterministic.
                            if let Some(next) = state.script.pop_front() {
                                state.models = next;
                            }
                            (200, body)
                        }
                    }
                }
                // A build that answers this route exists — the app must never
                // call it for its own management (see the module docs).
                ("GET", "/v1/models") => (200, models_body(&state.models)),
                ("GET", PROPS_PATH) => (200, state.props.clone()),
                ("POST", LOAD_PATH) => {
                    let (status, body) = state.load_answer.clone();
                    if (200..300).contains(&status) {
                        if let Some(next) = state.after_post.take() {
                            state.models = next;
                        }
                    }
                    (status, body)
                }
                ("POST", UNLOAD_PATH) => {
                    let (status, body) = state.unload_answer.clone();
                    if (200..300).contains(&status) {
                        if let Some(next) = state.after_post.take() {
                            state.models = next;
                        }
                    }
                    (status, body)
                }
                _ => (
                    404,
                    error_body(&format!("no route {method} {path}"), "not_found_error", 404),
                ),
            };

            let reply = format!(
                "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status_text(status),
                body.len(),
                body
            );
            let _ = socket.write_all(reply.as_bytes());
        }
    });

    Stub { port, state }
}

/// Read one request: the request line, the headers, and exactly
/// `Content-Length` bytes of body.
fn read_request(socket: &mut std::net::TcpStream) -> Option<(String, String, String)> {
    let mut raw: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    while !raw.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = socket.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        raw.extend_from_slice(&chunk[..read]);
        if raw.len() > 65536 {
            return None;
        }
    }

    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)?;
    let headers = String::from_utf8_lossy(&raw[..header_end]).to_string();
    let mut lines = headers.lines();
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") {
                value.trim().parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let mut body = raw[header_end..].to_vec();
    while body.len() < content_length {
        let read = socket.read(&mut chunk).ok()?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
    }

    Some((method, path, String::from_utf8_lossy(&body).to_string()))
}

// ─── The other doubles ──────────────────────────────────────────

/// A catalogue with no database behind it.
struct FixedDirectory(Vec<OrchestratedModel>);

impl ModelDirectory for FixedDirectory {
    fn find(&self, model_id: &str) -> Result<OrchestratedModel, AppError> {
        self.0
            .iter()
            .find(|model| model.model_id == model_id)
            .cloned()
            .ok_or_else(|| AppError::NotFound {
                what: format!("model {model_id}"),
            })
    }
}

struct FixedPort(Option<u16>);

impl UpstreamPort for FixedPort {
    fn upstream_port(&self) -> Result<Option<u16>, AppError> {
        Ok(self.0)
    }
}

/// One recorded `model-load-state-changed` payload.
#[derive(Default)]
struct RecordingEvents {
    snapshots: Mutex<Vec<Vec<LoadedModelState>>>,
}

impl RecordingEvents {
    fn all(&self) -> Vec<Vec<LoadedModelState>> {
        self.snapshots.lock().map(|s| s.clone()).unwrap_or_default()
    }

    fn count(&self) -> usize {
        self.snapshots.lock().map(|s| s.len()).unwrap_or_default()
    }

    /// Did any published snapshot show `name` in `state`?
    fn saw(&self, name: &str, state: ModelLoadState) -> bool {
        self.all().iter().any(|snapshot| {
            snapshot
                .iter()
                .any(|model| model.model_id == name && model.state == state)
        })
    }
}

impl OrchestratorEvents for RecordingEvents {
    fn model_load_state_changed(&self, models: &[LoadedModelState]) {
        if let Ok(mut snapshots) = self.snapshots.lock() {
            snapshots.push(models.to_vec());
        }
    }
}

#[derive(Clone, Debug)]
struct RecordedAttempt {
    file_path: PathBuf,
    params_json: String,
    succeeded: bool,
    load_seconds: Option<f64>,
    actual_vram_bytes: Option<u64>,
    error_message: Option<String>,
}

#[derive(Default)]
struct RecordingHistory {
    attempts: Mutex<Vec<RecordedAttempt>>,
}

impl RecordingHistory {
    fn all(&self) -> Vec<RecordedAttempt> {
        self.attempts.lock().map(|a| a.clone()).unwrap_or_default()
    }
}

impl LaunchHistorySink for RecordingHistory {
    fn record(&self, attempt: &LaunchAttempt<'_>) -> Result<(), AppError> {
        if let Ok(mut attempts) = self.attempts.lock() {
            attempts.push(RecordedAttempt {
                file_path: attempt.file_path.to_path_buf(),
                params_json: attempt.params_json.clone(),
                succeeded: attempt.succeeded,
                load_seconds: attempt.load_seconds,
                actual_vram_bytes: attempt.actual_vram_bytes,
                error_message: attempt.error_message.map(str::to_string),
            });
        }
        Ok(())
    }
}

// ─── Fixtures ───────────────────────────────────────────────────

fn orchestrated(model_id: &str, name: &str, path: &str) -> OrchestratedModel {
    OrchestratedModel {
        model_id: model_id.to_string(),
        name: name.to_string(),
        file_path: PathBuf::from(path),
        params: LaunchParams::default(),
    }
}

fn fixtures() -> (OrchestratedModel, OrchestratedModel) {
    (
        orchestrated("m1", "first-model", "C:/models/gguf/first-model.gguf"),
        orchestrated(
            "m2",
            "second-model",
            "D:/other-volume/gguf/second-model.gguf",
        ),
    )
}

fn test_tuning() -> Tuning {
    Tuning {
        poll_interval: Duration::from_millis(100),
        load_poll_interval: Duration::from_millis(20),
        load_timeout: Duration::from_secs(10),
        unload_timeout: Duration::from_secs(5),
        backoff_ceiling: Duration::from_millis(400),
        probe_strikes: 3,
        probe_timeout: Duration::from_millis(500),
    }
}

struct Harness {
    stub: Stub,
    events: Arc<RecordingEvents>,
    history: Arc<RecordingHistory>,
    deps: Arc<OrchestratorDeps>,
    orchestrator: RouterOrchestrator,
}

impl Harness {
    fn new(directory: Vec<OrchestratedModel>, tuning: Tuning) -> Self {
        let stub = start_stub();
        let events = Arc::new(RecordingEvents::default());
        let history = Arc::new(RecordingHistory::default());
        let deps = Arc::new(OrchestratorDeps::new(
            Arc::new(RouterHttp::new(tuning.probe_timeout)),
            Arc::new(FixedDirectory(directory)),
            Arc::clone(&history) as Arc<dyn LaunchHistorySink>,
            Arc::new(FixedPort(Some(stub.port))),
            Arc::clone(&events) as Arc<dyn OrchestratorEvents>,
            tuning,
        ));
        Harness {
            stub,
            events,
            history,
            orchestrator: RouterOrchestrator::new(Arc::clone(&deps)),
            deps,
        }
    }
}

/// Poll `check` until it is true or `timeout` elapses. Returns whether it became
/// true, so a failing assertion can say what it waited for.
fn wait_until(timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if check() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

// ─── Polling: the state the screen reads ────────────────────────

/// "Load state syncs within one poll interval."
#[test]
fn load_state_syncs_within_one_poll_interval() {
    let (first, _) = fixtures();
    let tuning = test_tuning();
    let harness = Harness::new(vec![first], tuning.clone());
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);

    let handle = super::orchestrator::spawn(Arc::clone(&harness.deps));
    assert!(
        wait_until(Duration::from_secs(3), || {
            handle.snapshot().iter().any(|model| {
                model.model_id == "first-model" && model.state == ModelLoadState::Registered
            })
        }),
        "the poller never published the unloaded model; snapshot: {:?}",
        handle.snapshot()
    );

    // The server loads the model behind the app's back: the next poll must see it.
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);
    let started = Instant::now();
    assert!(
        wait_until(Duration::from_secs(3), || {
            handle.snapshot().iter().any(|model| {
                model.model_id == "first-model" && model.state == ModelLoadState::Loaded
            })
        }),
        "the load state never synced"
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "a state change must sync within a poll interval or two, took {elapsed:?}"
    );

    // And it was announced, not only cached.
    assert!(
        harness.events.saw("first-model", ModelLoadState::Loaded),
        "no model-load-state-changed carried the loaded state: {:?}",
        harness.events.all()
    );
    handle.shutdown();
}

/// The event stream is a sequence of *changes*: an unchanged poll emits nothing.
#[test]
fn an_unchanged_poll_emits_nothing() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);

    let handle = super::orchestrator::spawn(Arc::clone(&harness.deps));
    assert!(
        wait_until(Duration::from_secs(3), || harness.events.count() == 1),
        "the first poll publishes the state it found: {:?}",
        harness.events.all()
    );
    // Several more polls with the upstream unchanged must publish nothing.
    thread::sleep(Duration::from_millis(400));
    assert_eq!(
        harness.events.count(),
        1,
        "an unchanged poll published again: {:?}",
        harness.events.all()
    );

    // A change is still a new event, through the same poller.
    harness
        .stub
        .set_models(vec![StubModel::loading("first-model")]);
    assert!(
        wait_until(Duration::from_secs(3), || harness.events.count() == 2),
        "a changed poll must publish: {:?}",
        harness.events.all()
    );
    handle.shutdown();
    assert!(wait_until(Duration::from_secs(2), || !handle.poller_running()));
}

/// "Repeated failures back off with a bounded ceiling."
#[test]
fn repeated_failures_back_off_with_a_bounded_ceiling() {
    let (first, _) = fixtures();
    let mut tuning = test_tuning();
    tuning.poll_interval = Duration::from_millis(50);
    tuning.backoff_ceiling = Duration::from_millis(200);
    let harness = Harness::new(vec![first], tuning);
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);
    // Every poll fails: 50 ms, 100, 200, 200, 200, …
    harness.stub.fail_polls(vec![500; 64]);

    let handle = super::orchestrator::spawn(Arc::clone(&harness.deps));
    thread::sleep(Duration::from_millis(1000));
    let polls = harness.stub.requests_to("GET", MODELS_PATH).len();
    handle.shutdown();

    // Without backoff a 50 ms interval would ask ~20 times in one second; with
    // the 200 ms ceiling it is ~5-7. The lower bound proves the failures were
    // retried at all, the upper bound proves the ceiling bounds them.
    assert!(
        (3..=9).contains(&polls),
        "expected the poll rate to back off to the 200 ms ceiling, saw {polls} polls in 1 s"
    );

    // The doubling itself, as a table: it is a pure function.
    let base = Duration::from_millis(50);
    let ceiling = Duration::from_millis(200);
    assert_eq!(backoff_interval(base, 1, ceiling), base);
    assert_eq!(
        backoff_interval(base, 2, ceiling),
        Duration::from_millis(100)
    );
    assert_eq!(backoff_interval(base, 3, ceiling), ceiling);
    assert_eq!(backoff_interval(base, 40, ceiling), ceiling);
}

/// A poll that fails a few times and then succeeds returns to the base interval
/// — the backoff is not sticky.
#[test]
fn a_successful_poll_resets_the_backoff() {
    let (first, _) = fixtures();
    let mut tuning = test_tuning();
    tuning.poll_interval = Duration::from_millis(50);
    tuning.backoff_ceiling = Duration::from_millis(400);
    let harness = Harness::new(vec![first], tuning);
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);
    harness.stub.fail_polls(vec![500, 500, 500]);

    let handle = super::orchestrator::spawn(Arc::clone(&harness.deps));
    assert!(
        wait_until(Duration::from_secs(5), || !handle.snapshot().is_empty()),
        "the poller never recovered"
    );
    let after_recovery = harness.stub.requests_to("GET", MODELS_PATH).len();
    thread::sleep(Duration::from_millis(400));
    let later = harness.stub.requests_to("GET", MODELS_PATH).len();
    handle.shutdown();
    assert!(
        later - after_recovery >= 4,
        "after a successful poll the interval must be back to 50 ms: {after_recovery} → {later}"
    );
}

// ─── Loading ────────────────────────────────────────────────────

/// "A load failure surfaces as a typed error carrying the server's message,
/// never a hang."
#[test]
fn a_load_failure_surfaces_the_servers_message() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);
    harness.stub.set_load_answer(
        400,
        &error_body("model is not running", "invalid_request_error", 400),
    );

    let started = Instant::now();
    let err = harness
        .orchestrator
        .load_blocking("m1")
        .expect_err("a refused load must be an error");
    let elapsed = started.elapsed();

    match &err {
        AppError::Network { message } => {
            assert!(
                message.contains("model is not running"),
                "the server's own words must survive: {message}"
            );
            assert!(
                message.contains("invalid_request_error") && message.contains("400"),
                "the type and status must be in the message: {message}"
            );
            assert!(
                message.contains(LOAD_PATH),
                "the message must name the call that failed: {message}"
            );
        }
        other => panic!("expected a typed Network error, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(2),
        "a refused load must not wait on anything: {elapsed:?}"
    );

    // The attempt is recorded even though it never started.
    let attempts = harness.history.all();
    assert_eq!(attempts.len(), 1, "one attempt, one row: {attempts:?}");
    assert!(!attempts[0].succeeded);
    assert!(
        attempts[0]
            .error_message
            .as_ref()
            .is_some_and(|message| message.contains("model is not running")),
        "the row carries the reason: {attempts:?}"
    );
}

/// A load the server accepts and then fails: on this build that is not a state
/// of its own — `unloaded` plus `failed: true` and the child's exit code.
#[test]
fn a_failed_load_is_reported_from_the_status_block() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);
    // Accepted, then the server reports the failure on the next poll: the state
    // it moves to is not `failed` (this build's status vocabulary has no such
    // value) but `unloaded` with `failed: true` and the child's exit code.
    harness.stub.set_script(vec![
        vec![StubModel::loading("first-model")],
        vec![StubModel::failed("first-model", 5)],
    ]);

    let err = harness
        .orchestrator
        .load_blocking("m1")
        .expect_err("a failed load must be an error");
    match &err {
        AppError::Network { message } => assert!(
            message.contains("exit code 5"),
            "the child's exit code is what the server reports: {message}"
        ),
        other => panic!("expected a typed Network error, got {other:?}"),
    }

    let attempts = harness.history.all();
    assert_eq!(attempts.len(), 1);
    assert!(!attempts[0].succeeded);
    assert!(
        attempts[0].load_seconds.is_some(),
        "even a failed attempt records how long it took: {attempts:?}"
    );
}

/// "A slow load emits progress rather than blocking."
#[test]
fn a_slow_load_emits_progress_rather_than_blocking() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);
    // Accepted, then several polls of `loading` before `loaded`.
    harness
        .stub
        .set_after_post(vec![StubModel::loading("first-model")]);
    harness.stub.set_script(vec![
        vec![StubModel::loading("first-model")],
        vec![StubModel::loading("first-model")],
        vec![StubModel::loading("first-model")],
        vec![StubModel::loaded("first-model")],
    ]);

    let deps = Arc::clone(&harness.deps);
    let worker = thread::spawn(move || {
        let orchestrator = RouterOrchestrator::new(deps);
        orchestrator.load_blocking("m1")
    });

    // The caller has not returned yet, and the state is already visible.
    assert!(
        wait_until(Duration::from_secs(5), || harness
            .events
            .saw("first-model", ModelLoadState::Loading)),
        "a load in flight must report progress: {:?}",
        harness.events.all()
    );
    assert!(
        !worker.is_finished(),
        "the load was still running while progress was already published"
    );

    let outcome = worker.join().expect("the load thread");
    let outcome = outcome.expect("the load succeeds");
    assert!(
        outcome.load_seconds > 0.0,
        "a load that took polls must report its duration: {outcome:?}"
    );
    assert_eq!(
        outcome.vram_bytes, None,
        "this build reports no VRAM figure, so the field stays None rather than guessed"
    );
    assert!(harness.events.saw("first-model", ModelLoadState::Loaded));

    let attempts = harness.history.all();
    assert_eq!(attempts.len(), 1);
    assert!(attempts[0].succeeded);
    assert!(attempts[0]
        .load_seconds
        .is_some_and(|seconds| seconds > 0.0));
    assert!(attempts[0].error_message.is_none());
    assert!(attempts[0].actual_vram_bytes.is_none());
}

/// A model the server already holds is not re-posted and gets no history row.
#[test]
fn a_model_that_is_already_loaded_is_not_loaded_again() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);

    let outcome = harness
        .orchestrator
        .load_blocking("m1")
        .expect("no-op load");
    assert_eq!(
        outcome,
        LoadOutcome {
            load_seconds: 0.0,
            vram_bytes: None
        }
    );
    assert!(
        harness.stub.requests_to("POST", LOAD_PATH).is_empty(),
        "the router refuses a second load of a running model; the app must not ask: {:?}",
        harness.stub.requests()
    );
    assert!(
        harness.history.all().is_empty(),
        "no load happened, so no launch_history row: {:?}",
        harness.history.all()
    );
}

/// "A load failure never hangs": an accepted load that never concludes is
/// bounded by its own timeout.
#[test]
fn a_load_that_never_concludes_is_bounded() {
    let (first, _) = fixtures();
    let mut tuning = test_tuning();
    tuning.load_timeout = Duration::from_millis(200);
    tuning.load_poll_interval = Duration::from_millis(50);
    let harness = Harness::new(vec![first], tuning);
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);
    harness
        .stub
        .set_after_post(vec![StubModel::loading("first-model")]);

    let started = Instant::now();
    let err = harness
        .orchestrator
        .load_blocking("m1")
        .expect_err("a load that never concludes must fail");
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "the wait must be bounded by load_timeout: {elapsed:?}"
    );
    match &err {
        AppError::Network { message } => assert!(
            message.contains("did not finish loading"),
            "the timeout must name itself: {message}"
        ),
        other => panic!("expected a typed Network error, got {other:?}"),
    }
    let attempts = harness.history.all();
    assert_eq!(attempts.len(), 1, "a timed-out attempt is still an attempt");
    assert!(!attempts[0].succeeded);
}

/// Three consecutive probe failures during a load mean the server has stopped
/// answering — the same 3-strike rule §2 applies to health checks.
#[test]
fn a_server_that_stops_answering_during_a_load_fails_the_attempt() {
    let (first, _) = fixtures();
    let mut tuning = test_tuning();
    tuning.load_poll_interval = Duration::from_millis(20);
    let harness = Harness::new(vec![first], tuning);
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);
    harness
        .stub
        .set_after_post(vec![StubModel::loading("first-model")]);
    // The pre-check read succeeds, then the server stops answering: three
    // consecutive failures are what makes this a failure rather than a hiccup.
    harness.stub.fail_after_successes(1);

    let err = harness
        .orchestrator
        .load_blocking("m1")
        .expect_err("a server that stopped answering is a failure");
    match &err {
        AppError::Network { message } => assert!(
            message.contains("stopped answering"),
            "the reason must be the server, not a timeout: {message}"
        ),
        other => panic!("expected a typed Network error, got {other:?}"),
    }
}

// ─── Unloading ──────────────────────────────────────────────────

#[test]
fn unload_waits_until_the_model_is_no_longer_loaded() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);
    harness.stub.set_script(vec![
        vec![StubModel::loaded("first-model")],
        vec![StubModel::unloaded("first-model")],
    ]);

    harness.orchestrator.unload_blocking("m1").expect("unload");

    let posts = harness.stub.requests_to("POST", UNLOAD_PATH);
    assert_eq!(posts.len(), 1, "one unload, one call: {:?}", posts);
    assert!(
        posts[0].body.contains("first-model"),
        "the unload names the router's own name for the model: {posts:?}"
    );
    assert_eq!(
        harness
            .orchestrator
            .snapshot()
            .iter()
            .filter(|model| model.model_id == "first-model")
            .map(|model| model.state.clone())
            .collect::<Vec<_>>(),
        vec![ModelLoadState::Registered],
        "the published state follows the server"
    );
}

#[test]
fn unloading_an_unloaded_model_is_a_no_op() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::unloaded("first-model")]);

    harness
        .orchestrator
        .unload_blocking("m1")
        .expect("no-op unload");
    assert!(
        harness.stub.requests_to("POST", UNLOAD_PATH).is_empty(),
        "there is nothing to unload, so nothing is asked: {:?}",
        harness.stub.requests()
    );
}

/// The server may refuse an unload it has already performed. Its own state is
/// the authority, so the app does not have to match on an error string.
#[test]
fn a_refused_unload_whose_state_cleared_is_not_an_error() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);
    harness.stub.set_unload_answer(
        400,
        &error_body("model is not running", "invalid_request_error", 400),
    );
    harness
        .stub
        .set_script(vec![vec![StubModel::unloaded("first-model")]]);

    harness
        .orchestrator
        .unload_blocking("m1")
        .expect("the model is not loaded, so the outcome holds");
}

#[test]
fn a_model_the_server_does_not_list_cannot_be_unloaded() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness.stub.set_models(Vec::new());

    let err = harness
        .orchestrator
        .unload_blocking("m1")
        .expect_err("a model the router does not hold cannot be unloaded");
    match &err {
        AppError::NotFound { what } => assert!(
            what.contains("first-model"),
            "the error names the model the server does not have: {what}"
        ),
        other => panic!("expected NotFound, got {other:?}"),
    }
}

// ─── Ports, props and the catalogue ─────────────────────────────

/// With no server there is nothing to control, and that is a typed error — not
/// an empty list that reads like "nothing is loaded".
#[test]
fn with_no_server_running_every_call_is_typed() {
    let (first, _) = fixtures();
    let deps = Arc::new(OrchestratorDeps::new(
        Arc::new(RouterHttp::new(Duration::from_millis(200))),
        Arc::new(FixedDirectory(vec![first])),
        Arc::new(RecordingHistory::default()),
        Arc::new(FixedPort(None)),
        Arc::new(RecordingEvents::default()),
        test_tuning(),
    ));
    let orchestrator = RouterOrchestrator::new(deps);

    for err in [
        orchestrator.load_blocking("m1").expect_err("no upstream"),
        orchestrator.unload_blocking("m1").expect_err("no upstream"),
        orchestrator.props_blocking().expect_err("no upstream"),
    ] {
        match err {
            AppError::UpstreamUnavailable { state } => assert!(
                !state.is_empty(),
                "the error names the state that made the call impossible"
            ),
            other => panic!("expected UpstreamUnavailable, got {other:?}"),
        }
    }
    // A poll with no server is not an error: nothing is loaded because there is
    // nothing to load, and the published snapshot says so.
    orchestrator.sync_once().expect("a poll with no upstream");
    assert!(orchestrator.snapshot().is_empty());
}

/// `/props` is the router's own identity, read rather than assumed.
#[test]
fn props_reads_the_routers_own_identity() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    let props = harness
        .orchestrator
        .props_blocking()
        .expect("the stub answers /props");
    assert_eq!(props.build_tag.as_deref(), Some("b10883-91f6a6cf3"));
    assert_eq!(props.models_max, Some(4));
    assert_eq!(
        props.raw.get("role").and_then(|role| role.as_str()),
        Some("router")
    );
    assert_eq!(
        harness.stub.requests_to("GET", PROPS_PATH).len(),
        1,
        "one call per read: {:?}",
        harness.stub.requests()
    );
}

/// "The router implementation passes model paths through unchanged, asserted
/// for a path on a second volume" — and it addresses the model by the router's
/// own name, never by a path it composed itself.
#[test]
fn names_and_paths_are_passed_through_unchanged() {
    let (_, second) = fixtures();
    let harness = Harness::new(vec![second], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::unloaded("second-model")]);
    harness
        .stub
        .set_after_post(vec![StubModel::loaded("second-model")]);

    harness.orchestrator.load_blocking("m2").expect("load");

    let posts = harness.stub.requests_to("POST", LOAD_PATH);
    assert_eq!(posts.len(), 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&posts[0].body)
            .expect("the body is the router's JSON"),
        serde_json::json!({"model": "second-model"}),
        "the name the router knows the model by, verbatim"
    );

    let attempts = harness.history.all();
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].file_path,
        PathBuf::from("D:/other-volume/gguf/second-model.gguf"),
        "the path on the second volume is recorded byte-for-byte, not rewritten"
    );
    assert_eq!(
        serde_json::from_str::<LaunchParams>(&attempts[0].params_json)
            .expect("the recorded parameters are the model's own"),
        LaunchParams::default()
    );

    // The management surface is `/models`; `/v1/models` is the clients' view and
    // the app never reads its own state from it.
    let paths: Vec<String> = harness
        .stub
        .requests()
        .into_iter()
        .map(|request| request.path)
        .collect();
    assert!(
        paths.iter().all(|path| path != "/v1/models"),
        "the app must not use the client-facing listing for its own state: {paths:?}"
    );
    assert!(paths.iter().any(|path| path == MODELS_PATH));
    assert!(paths.iter().any(|path| path == LOAD_PATH));
}

/// An id the catalogue does not hold is a different failure from a model the
/// server does not hold.
#[test]
fn an_unknown_catalogue_id_is_not_found() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    let err = harness
        .orchestrator
        .load_blocking("no-such-id")
        .expect_err("an unknown id must fail");
    assert!(matches!(err, AppError::NotFound { .. }), "{err:?}");
}

// ─── The trait as a seam ────────────────────────────────────────

/// "The trait has a second no-op implementation used in tests, proving the
/// abstraction is real rather than router-shaped."
#[test]
fn the_trait_has_two_real_implementations() {
    fn exercise<O: ModelOrchestrator>(
        orchestrator: &O,
    ) -> (Vec<LoadedModelState>, LoadOutcome, ServerProps) {
        let models = block_on(orchestrator.list_models()).expect("list_models");
        let outcome = block_on(orchestrator.load("m1")).expect("load");
        block_on(orchestrator.unload("m1")).expect("unload");
        let props = block_on(orchestrator.props()).expect("props");
        (models, outcome, props)
    }

    // The router implementation against the stub.
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);
    // The unload POST is the only accepted call in this exercise (the load is a
    // no-op on a loaded model), so the state it moves to is the unloaded model.
    harness
        .stub
        .set_after_post(vec![StubModel::unloaded("first-model")]);
    let (models, outcome, props) = exercise(&harness.orchestrator);
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].model_id, "first-model");
    assert_eq!(outcome.load_seconds, 0.0, "already loaded: no attempt");
    assert_eq!(props.build_tag.as_deref(), Some("b10883-91f6a6cf3"));

    // The other implementation, through the same generic call: no upstream is
    // involved at all, which is the whole point of the seam.
    let (models, outcome, props) = exercise(&NoopOrchestrator);
    assert!(models.is_empty());
    assert_eq!(outcome.load_seconds, 0.0);
    assert!(outcome.vram_bytes.is_none());
    assert!(props.build_tag.is_none());
    assert!(props.raw.is_null());
}

/// Drive a boxed future to completion on the current thread. The orchestrator's
/// own implementations never actually suspend — they block — so a minimal
/// executor is enough and keeps the test's shape async, like §5's trait.
fn block_on<T>(future: super::orchestrator::OrchestratorFuture<'_, T>) -> Result<T, AppError> {
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    let waker = Waker::from(Arc::new(Noop));
    let mut context = Context::from_waker(&waker);
    let mut future = future;
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => thread::yield_now(),
        }
    }
}

// ─── launch_history, in the real database ───────────────────────

/// "A `launch_history` row is written for every load attempt, success or
/// failure, verified against the database."
#[test]
fn every_load_attempt_is_written_to_the_database() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let db_path = dir.path().join("llama-manager.db");
    // Create it through the app's own opener, so the schema is the real one.
    crate::db::open(&db_path).expect("the schema migrates");

    let (first, _) = fixtures();
    let stub = start_stub();
    let history: Arc<dyn LaunchHistorySink> = Arc::new(DbLaunchHistory::at(db_path.clone()));
    let deps = Arc::new(OrchestratorDeps::new(
        Arc::new(RouterHttp::new(Duration::from_millis(500))),
        Arc::new(FixedDirectory(vec![first.clone()])),
        history,
        Arc::new(FixedPort(Some(stub.port))),
        Arc::new(RecordingEvents::default()),
        test_tuning(),
    ));
    let orchestrator = RouterOrchestrator::new(deps);

    // One successful attempt …
    stub.set_models(vec![StubModel::unloaded("first-model")]);
    stub.set_after_post(vec![StubModel::loaded("first-model")]);
    orchestrator.load_blocking("m1").expect("the load succeeds");

    // … and one refused attempt. The model is put back to unloaded first, or the
    // path under test (an already-loaded model is a no-op) would swallow it.
    stub.set_models(vec![StubModel::unloaded("first-model")]);
    stub.set_after_post(vec![StubModel::unloaded("first-model")]);
    stub.set_load_answer(
        400,
        &error_body("model is not running", "invalid_request_error", 400),
    );
    orchestrator
        .load_blocking("m1")
        .expect_err("the second attempt is refused");

    let path = first.file_path.display().to_string();
    let conn = crate::db::open(&db_path).expect("reopen");
    let rows = crate::db::queries::list_launch_history_for_path(&conn, &path)
        .expect("read the history back");

    assert_eq!(rows.len(), 2, "one row per attempt: {rows:?}");
    let succeeded = rows
        .iter()
        .find(|row| row.succeeded)
        .expect("the successful attempt is recorded");
    assert!(
        succeeded.load_seconds.is_some_and(|seconds| seconds >= 0.0),
        "a successful attempt records its duration: {succeeded:?}"
    );
    assert!(succeeded.error_message.is_none());
    assert!(succeeded.actual_vram_bytes.is_none());
    assert_eq!(
        serde_json::from_str::<LaunchParams>(&succeeded.params_json)
            .expect("the parameters round-trip"),
        LaunchParams::default(),
        "the row carries the parameters the attempt was made with"
    );

    let failed = rows
        .iter()
        .find(|row| !row.succeeded)
        .expect("the refused attempt is recorded");
    assert!(
        failed
            .error_message
            .as_ref()
            .is_some_and(|message| message.contains("model is not running")),
        "the row carries the server's own message: {failed:?}"
    );

    // And the reader half: the same rows come back as the estimator's input.
    let records = launch_records_at(&db_path, &path).expect("the estimator's view");
    assert_eq!(records.len(), 2);
    assert_eq!(records.iter().filter(|record| record.succeeded).count(), 1);
    assert!(records
        .iter()
        .all(|record| record.file_path == first.file_path));
}

/// One unreadable row must not cost the estimate the rest of the history, and
/// must never be turned into a plausible-looking default.
#[test]
fn an_unreadable_history_row_is_skipped() {
    let dir = tempfile::tempdir().expect("a scratch directory");
    let db_path = dir.path().join("llama-manager.db");
    let conn = crate::db::open(&db_path).expect("the schema migrates");
    let path = "C:/models/gguf/first-model.gguf";

    crate::db::queries::insert_launch_record(
        &conn,
        &crate::db::queries::NewLaunchRecord {
            file_path: path.to_string(),
            launched_at: chrono::Utc::now(),
            params_json: "not json at all".to_string(),
            succeeded: true,
            actual_vram_bytes: Some(1234),
            load_seconds: Some(1.5),
            error_message: None,
        },
    )
    .expect("insert the unreadable row");
    crate::db::queries::insert_launch_record(
        &conn,
        &crate::db::queries::NewLaunchRecord {
            file_path: path.to_string(),
            launched_at: chrono::Utc::now(),
            params_json: serde_json::to_string(&LaunchParams::default()).expect("serialize"),
            succeeded: true,
            actual_vram_bytes: Some(4096),
            load_seconds: Some(2.5),
            error_message: None,
        },
    )
    .expect("insert the readable row");

    let records = launch_records_at(&db_path, path).expect("read back");
    assert_eq!(
        records.len(),
        1,
        "the readable row survives its neighbour: {records:?}"
    );
    assert_eq!(records[0].actual_vram_bytes, Some(4096));
}

// ─── The poller's own lifecycle ─────────────────────────────────

#[test]
fn the_poller_stops_when_it_is_told_to() {
    let (first, _) = fixtures();
    let harness = Harness::new(vec![first], test_tuning());
    harness
        .stub
        .set_models(vec![StubModel::loaded("first-model")]);

    let handle = super::orchestrator::spawn(Arc::clone(&harness.deps));
    assert!(wait_until(Duration::from_secs(3), || !handle
        .snapshot()
        .is_empty()));
    handle.shutdown();
    assert!(wait_until(Duration::from_secs(2), || !handle.poller_running()));
    let settled = harness.stub.requests_to("GET", MODELS_PATH).len();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        settled,
        harness.stub.requests_to("GET", MODELS_PATH).len(),
        "a stopped poller makes no calls"
    );
}
