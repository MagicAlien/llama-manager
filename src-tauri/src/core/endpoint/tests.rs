//! T-042's acceptance suite (`docs/TASKS.md` T-042).
//!
//! Every test below drives the **real** listener — the same `start()`, the same
//! axum router, the same reqwest client the app uses — against the stub
//! upstream in `stub.rs`. Nothing here needs a GPU, a model file, or the target
//! machine (`AGENTS.md` §3).
//!
//! The three properties the task is split around are asserted separately, on
//! purpose: the bytes arrive intact (`transparency`), a stream is not
//! accumulated (`streaming`), and the added latency and the concurrency limit
//! stay inside their stated budgets (`load`). A single "it proxies" test would
//! pass with any two of the three broken.

mod load;
mod streaming;
mod stub;
mod transparency;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use super::{start, EndpointConfig, EndpointState, EndpointStateSink, Timeouts, UpstreamSource};
use crate::core::types::{AppError, ServerState};

// ─── Test doubles ───────────────────────────────────────────────

/// The upstream the listener forwards to, held fixed: the tests are about the
/// transport, so the address is a constant rather than a state machine.
struct FixedUpstream(Option<SocketAddr>);

impl UpstreamSource for FixedUpstream {
    fn upstream(&self) -> Option<SocketAddr> {
        self.0
    }
}

fn fixed_upstream(address: SocketAddr) -> Arc<dyn UpstreamSource> {
    Arc::new(FixedUpstream(Some(address)))
}

fn no_upstream() -> Arc<dyn UpstreamSource> {
    Arc::new(FixedUpstream(None))
}

/// Records what the listener reported about itself, so "the bind failure is
/// reported as `BindFailed`" is checked without a database or an app handle.
#[derive(Default)]
struct RecordingSink {
    reports: Mutex<Vec<EndpointState>>,
}

impl RecordingSink {
    fn reports(&self) -> Vec<EndpointState> {
        match self.reports.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn last(&self) -> Option<EndpointState> {
        self.reports().pop()
    }
}

impl EndpointStateSink for RecordingSink {
    fn report(&self, state: &EndpointState) {
        match self.reports.lock() {
            Ok(mut guard) => guard.push(state.clone()),
            Err(poisoned) => poisoned.into_inner().push(state.clone()),
        }
    }
}

/// A bound listener on an ephemeral port, plus the sink it reported to.
fn listener(
    upstream: Arc<dyn UpstreamSource>,
    limit: Option<u32>,
    timeouts: Timeouts,
) -> (u16, Arc<RecordingSink>) {
    let sink = Arc::new(RecordingSink::default());
    let state = start(EndpointConfig {
        address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        // Port 0: the OS picks, so two tests never collide, and the returned
        // state must name the port actually bound (asserted in
        // `the_reported_address_is_the_one_actually_bound`).
        port: 0,
        max_concurrent_requests: limit,
        upstream,
        state_sink: Arc::clone(&sink) as Arc<dyn EndpointStateSink>,
        timeouts,
    })
    .expect("the listener must start");
    match state {
        EndpointState::Bound { port, .. } => (port, sink),
        other => panic!("expected a bound listener, got {other:?}"),
    }
}

/// A listener with production timeouts, against `stub`.
fn listener_for(stub: &stub::Stub) -> (u16, Arc<RecordingSink>) {
    listener(fixed_upstream(stub.address()), None, Timeouts::default())
}

fn url(port: u16, target: &str) -> String {
    format!("http://127.0.0.1:{port}{target}")
}

/// The client both sides of a differential test use. No overall timeout: the
/// suite deliberately asks one response to begin after a long delay, and the
/// listener's own timeout is what that test is about.
fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .http1_only()
        .build()
        .expect("the test client must build")
}

// ─── The listener's own lifecycle ───────────────────────────────

#[test]
fn the_reported_address_is_the_one_actually_bound() {
    let upstream = stub::Stub::start();
    let (port, sink) = listener_for(&upstream);

    assert!(port > 0, "a bind on port 0 must report the port it got");
    match sink.last() {
        Some(EndpointState::Bound {
            address,
            port: reported,
        }) => {
            assert_eq!(address, IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert_eq!(reported, port);
        }
        other => panic!("expected `Bound`, reported {other:?}"),
    }
    // And the listener really is there: a request through it reaches the stub.
    let response = blocking_get(&url(port, "/alive"));
    assert_eq!(response.status(), 200);
    assert_eq!(upstream.request_count(), 1);
}

#[test]
fn a_taken_port_is_reported_as_port_in_use_and_never_auto_incremented() {
    let upstream = stub::Stub::start();
    // Hold a port, then ask the listener for exactly that one.
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("bind the conflicting socket");
    let taken = held.local_addr().expect("the conflicting address").port();

    let sink = Arc::new(RecordingSink::default());
    let state = start(EndpointConfig {
        address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: taken,
        max_concurrent_requests: None,
        upstream: fixed_upstream(upstream.address()),
        state_sink: Arc::clone(&sink) as Arc<dyn EndpointStateSink>,
        timeouts: Timeouts::default(),
    })
    .expect("a taken port is a state, not an error");

    match state {
        EndpointState::BindFailed {
            address,
            port,
            error,
        } => {
            // The address clients configure is the app's contract: it fails
            // there rather than moving somewhere a configured client cannot
            // find it (`PLAN.md` §2.7).
            assert_eq!(address, IpAddr::V4(Ipv4Addr::LOCALHOST));
            assert_eq!(port, taken, "the failure names the port that was asked for");
            assert_eq!(error, AppError::PortInUse { port: taken });
        }
        other => panic!("expected `BindFailed`, got {other:?}"),
    }
    assert!(
        matches!(sink.last(), Some(EndpointState::BindFailed { .. })),
        "the failure must be reported, not only returned"
    );
}

#[test]
fn the_upstream_slot_follows_the_states_that_carry_a_port() {
    // `Starting` already has a port — the process was spawned with it — which
    // is what lets a request arriving during a preload be forwarded rather
    // than held (`docs/CONTRACTS.md` §2).
    use super::{upstream_from_state, UpstreamSlot};
    use crate::core::supervisor::SupervisorEvents;
    use crate::core::types::StartupPhase;
    use chrono::Utc;

    let starting = ServerState::Starting {
        since: Utc::now(),
        upstream_port: 49500,
        phase: StartupPhase::WaitingForProcess,
    };
    assert_eq!(upstream_from_state(&starting), Some(super::loopback(49500)));

    let preloading = ServerState::Starting {
        since: Utc::now(),
        upstream_port: 49501,
        phase: StartupPhase::Preloading {
            done: 0,
            total: 1,
            current: Some("qwen".to_string()),
        },
    };
    assert_eq!(
        upstream_from_state(&preloading),
        Some(super::loopback(49501)),
        "the coordinator answers during a preload, so there is an upstream to forward to"
    );

    let running = ServerState::Running {
        pid: 1234,
        upstream_port: 49502,
        since: Utc::now(),
        config_dirty: false,
    };
    assert_eq!(upstream_from_state(&running), Some(super::loopback(49502)));

    for unavailable in [
        ServerState::Stopped,
        ServerState::Stopping,
        ServerState::Crashed {
            exit_code: Some(1),
            diagnosis: None,
            last_log: Vec::new(),
        },
    ] {
        assert_eq!(upstream_from_state(&unavailable), None);
    }

    // The slot is the live version of that mapping: it is fed by the same
    // broadcasts the actor sends.
    let slot = UpstreamSlot::new();
    assert_eq!(slot.upstream(), None);
    slot.state_changed(&running);
    assert_eq!(slot.upstream(), Some(super::loopback(49502)));
    slot.state_changed(&ServerState::Stopping);
    assert_eq!(slot.upstream(), None);
}

#[test]
fn with_no_upstream_the_answer_is_503_in_the_openai_error_shape() {
    let (port, _sink) = listener(no_upstream(), None, Timeouts::default());

    let response = blocking_get(&url(port, "/v1/chat/completions"));
    assert_eq!(response.status(), 503);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.starts_with("application/json"),
        "a client must parse this as a server error, not as a body it cannot read: {content_type}"
    );

    let body: serde_json::Value = serde_json::from_str(&response.text).expect("JSON body");
    let error = &body["error"];
    assert_eq!(error["code"], "upstream_unavailable");
    assert_eq!(error["type"], "server_error");
    let message = error["message"].as_str().unwrap_or_default();
    assert!(
        !message.is_empty(),
        "the body names the cause; T-044 replaces it with the per-state table"
    );
}

#[test]
fn the_endpoint_never_parses_a_forwarded_body() {
    // Source-level, as T-042 specifies as the primary form: no type in
    // `core/endpoint/` can hold a parsed request, and no forwarded content is
    // deserialized. Comments are stripped first — the module documents the rule
    // in the same words the check looks for, and a check that failed on its own
    // documentation would be useless.
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("forward.rs", include_str!("forward.rs")),
    ];
    for (name, source) in sources {
        let code = strip_comments(source);
        assert!(
            code.len() > 1000,
            "{name}: the check read something, not an empty file"
        );
        assert!(
            !code.contains("Deserialize"),
            "{name} must not carry a type that can hold a parsed request body"
        );
        assert!(
            !code.contains("serde_json::from_"),
            "{name} must not deserialize forwarded content"
        );
    }
}

/// Everything outside `//` line comments and `/* */` blocks.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut rest = source;
    while let Some(start) = rest.find("/*") {
        out.push_str(&rest[..start]);
        match rest[start..].find("*/") {
            Some(end) => rest = &rest[start + end + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out.lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A blocking GET, so the lifecycle tests stay synchronous and readable. The
/// async parts of the suite (streaming, latency, concurrency) own their own
/// runtime.
fn blocking_get(url: &str) -> SimpleResponse {
    let client = client();
    let response = async_io(client.get(url).send()).expect("the request must reach the listener");
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let text = async_io(response.text()).expect("the body must be readable");
    SimpleResponse {
        status,
        headers,
        text,
    }
}

struct SimpleResponse {
    status: u16,
    headers: reqwest::header::HeaderMap,
    text: String,
}

impl SimpleResponse {
    fn status(&self) -> u16 {
        self.status
    }

    fn headers(&self) -> &reqwest::header::HeaderMap {
        &self.headers
    }
}

/// Run one future to completion on a scratch runtime. Each call is its own
/// runtime: the listener owns its own, and the tests must not share one with
/// it.
fn async_io<T>(future: impl std::future::Future<Output = T>) -> T {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a scratch runtime");
    runtime.block_on(future)
}
