//! `core::endpoint` — T-042: the app's own HTTP listener (`PLAN.md` §2.7).
//!
//! The app binds the port clients are configured for and forwards every
//! request to the single upstream process T-040 allocates. `AGENTS.md`
//! invariant 3 is the whole specification of what this module may do with a
//! request: forward method, path, query, headers and body **unchanged**, and
//! never look inside a body. Model routing belongs to llama-server's router
//! mode; a proxy that starts reading bodies has become a scheduler, and
//! `PLAN.md` §2.3 explains why that is the way this project grows out of
//! control.
//!
//! # What is and is not here
//!
//! This task is the **transport**. How the endpoint answers when the upstream
//! is not available — the hold window during `Starting { WaitingForProcess }`,
//! the per-state `503` bodies, the `Diagnosis` in a `Crashed` body — is T-044,
//! and authentication and the request log are T-051. The split is deliberate
//! (`docs/TASKS.md` T-042): "the bytes arrive intact" and "the app behaves
//! correctly when the server is down" are different properties.
//!
//! So the fallback here is the single honest one the transport can offer
//! without inventing T-044's table: when no upstream is allocated, or the
//! upstream is not answering, the client gets `503` in OpenAI error shape
//! built from [`AppError::UpstreamUnavailable`] — the same body §2 describes
//! for the not-available case, minus the state-specific text T-044 owns.
//! Those two call sites are marked [`forward`]'s `unavailable`.
//!
//! # Two lifecycles, one owner
//!
//! The listener binds when the app starts and stays bound while llama-server
//! is stopped or gone; that independence is the point of owning the socket
//! (§2, "The endpoint is a separate lifecycle"). [`EndpointState`] is what
//! the listener reports about itself, and per §2 it is **owned by the
//! supervisor actor** — not by a second object with its own lock. This module
//! therefore never stores that state: it binds, reports the result through
//! [`EndpointStateSink`] (the supervisor's adapter pushes it into the actor,
//! which broadcasts `endpoint-state-changed`), and remembers nothing.
//!
//! # Where the upstream address comes from
//!
//! Only one thing about the server is needed on the data path: the loopback
//! port the app itself allocated. [`UpstreamSlot`] holds exactly that, and is
//! fed by the supervisor's own `server-state-changed` broadcasts — the actor
//! stays the single owner of the state, and the listener reads a published
//! fact instead of asking the actor a question per request. A `RwLock` around
//! a `SocketAddr` is not a `Mutex<ServerState>` (`AGENTS.md` invariant 2): the
//! state is unreachable from here, and what this holds is the answer to one
//! question the actor already answered in public.
//!
//! # The listener owns its runtime
//!
//! `core/` never imports `tauri` (`AGENTS.md` invariant 1), so it cannot reach
//! the app's async runtime — and the endpoint must be up before the first
//! screen exists. [`start`] therefore runs the server on its own named thread
//! with its own multi-thread runtime, the same pattern the supervisor's probe
//! worker and `HttpCoordinator` already use: the listener's lifetime is a
//! property of the app, not of whoever called `start`.

mod forward;
#[cfg(test)]
mod tests;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;

use axum::Router;
use tokio::net::TcpListener;
use tokio::runtime::Builder as RuntimeBuilder;
use tokio::sync::Semaphore;

use crate::core::preset_generator::PresetWarning;
use crate::core::process::LogLine;
use crate::core::supervisor::SupervisorEvents;
use crate::core::types::{AppError, EndpointState, ServerState};

/// Worker threads for the listener's runtime. Two would serve the acceptance
/// suite; four keeps the proxy from being the slow part of a pipeline the user
/// is watching, at a cost of a few idle threads.
const WORKER_THREADS: usize = 4;

/// The one timeout the client-facing side needs, and the reason it is written
/// down instead of inherited.
///
/// A first request against an unloaded 65 GB model holds the connection for
/// minutes (`PLAN.md` §2.7, "Long-held requests"), so whatever a library's
/// default is, it is either too short to serve the product's own use case or
/// an undocumented accident. [`Timeouts::default`] is what `docs/TASKS.md`
/// T-042 requires to be explicit: a response that begins after three minutes
/// must complete, and the transport must not be able to hang forever either.
#[derive(Clone, Copy, Debug)]
pub struct Timeouts {
    /// The whole upstream exchange: connect, request, and the response body to
    /// its last chunk. Bounds a streaming response too, which is what makes it
    /// the honest ceil for "this request will never finish".
    pub upstream_response: Duration,
    /// How long a connection attempt to the loopback upstream may take.
    pub connect: Duration,
}

impl Default for Timeouts {
    fn default() -> Self {
        // 30 minutes: an order of magnitude above the 3-minute case T-042
        // requires to complete, and finite so a wedged upstream releases the
        // connection instead of pinning a client forever.
        Timeouts {
            upstream_response: Duration::from_secs(1800),
            connect: Duration::from_secs(5),
        }
    }
}

/// Where the listener forwards to.
///
/// One method, because the transport needs one fact. It deliberately cannot
/// say *which* model should serve a request, or whether a model should load:
/// there is nothing here to ask that of.
pub trait UpstreamSource: Send + Sync {
    /// The upstream the app allocated, or `None` when there is none.
    fn upstream(&self) -> Option<SocketAddr>;
}

/// The live upstream address, published by the supervisor's state changes.
#[derive(Debug, Default)]
pub struct UpstreamSlot {
    address: RwLock<Option<SocketAddr>>,
}

impl UpstreamSlot {
    pub fn new() -> Arc<UpstreamSlot> {
        Arc::new(UpstreamSlot::default())
    }

    pub fn set(&self, address: Option<SocketAddr>) {
        match self.address.write() {
            Ok(mut slot) => *slot = address,
            // A poisoned lock here means a previous reader panicked while
            // reading a `SocketAddr`. The value is not worth a crash: the
            // reader recovering the guard has the same field, unchanged.
            Err(poisoned) => *poisoned.into_inner() = address,
        }
    }
}

impl UpstreamSource for UpstreamSlot {
    fn upstream(&self) -> Option<SocketAddr> {
        match self.address.read() {
            Ok(slot) => *slot,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

/// The upstream a state makes available, or `None`.
///
/// `Starting { WaitingForProcess }` already has a port — the process was
/// spawned with it — and `Starting { Preloading }` certainly has one: the
/// coordinator is answering. Publishing the port in both is what lets a
/// request arriving during a preload be forwarded rather than refused, which
/// §2 states as a rule ("Availability does not wait for it").
pub fn upstream_from_state(state: &ServerState) -> Option<SocketAddr> {
    match state {
        ServerState::Starting { upstream_port, .. }
        | ServerState::Running { upstream_port, .. } => Some(loopback(*upstream_port)),
        ServerState::Stopped | ServerState::Stopping | ServerState::Crashed { .. } => None,
    }
}

/// llama-server is always on loopback and never reachable otherwise
/// (`PLAN.md` §6): the only way in is through this listener.
pub fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

impl SupervisorEvents for UpstreamSlot {
    /// The whole of this sink: the port the state makes available, and
    /// nothing else. It is a subscriber to the actor's broadcasts, not a
    /// second copy of the state.
    fn state_changed(&self, state: &ServerState) {
        self.set(upstream_from_state(state));
    }

    fn log_line(&self, _line: &LogLine) {}

    fn preset_warning(&self, _warning: &PresetWarning) {}

    fn endpoint_state_changed(&self, _state: &EndpointState) {}
}

/// Reports the listener's own lifecycle to whoever owns it.
///
/// A trait rather than a direct call into the supervisor: the acceptance tests
/// bind real listeners and assert what was reported, and a sink that is a
/// recording double is how "the bind failure is reported as `BindFailed`" is
/// checked without a database or an app handle.
pub trait EndpointStateSink: Send + Sync {
    fn report(&self, state: &EndpointState);
}

/// Everything [`start`] needs. A struct rather than seven arguments, and the
/// seam the tests drive: a fixed [`UpstreamSource`] double replaces the slot,
/// and a recording [`EndpointStateSink`] replaces the actor.
pub struct EndpointConfig {
    pub address: IpAddr,
    pub port: u16,
    /// `ServerConfig.max_concurrent_requests` verbatim: `None` means the app
    /// imposes no limit of its own and lets the upstream decide, which is the
    /// transparent behaviour (§1).
    pub max_concurrent_requests: Option<u32>,
    pub upstream: Arc<dyn UpstreamSource>,
    pub state_sink: Arc<dyn EndpointStateSink>,
    pub timeouts: Timeouts,
}

/// Bind the client-facing socket and serve on it.
///
/// # The two failures are not the same failure
///
/// - **The port is taken** → `Ok(EndpointState::BindFailed { .. })`, reported
///   through the sink. The app keeps running: the address is the app's
///   contract with configured clients, so it is never silently replaced by
///   another port (`PLAN.md` §2.7), and the user fixes it in T-050 and
///   rebinds (T-044). A bind failure is therefore a *state*, not an error the
///   caller has to handle to keep the app alive.
/// - **There is no listener to have** → `Err(..)`, and nothing is reported,
///   because `EndpointState::Unbound` is the truth: the runtime or the HTTP
///   client could not be created, so no socket was ever involved.
pub fn start(config: EndpointConfig) -> Result<EndpointState, AppError> {
    let address = SocketAddr::new(config.address, config.port);

    let client = reqwest::Client::builder()
        .timeout(config.timeouts.upstream_response)
        .connect_timeout(config.timeouts.connect)
        // One upstream, spoken to over loopback; HTTP/1.1 is what every
        // llama.cpp build serves and what SSE is documented against.
        .http1_only()
        .build()
        .map_err(|err| AppError::Internal {
            message: format!("could not create the endpoint's HTTP client: {err}"),
        })?;

    let listener = match std::net::TcpListener::bind(address) {
        Ok(listener) => listener,
        Err(err) => {
            let state = EndpointState::BindFailed {
                address: config.address,
                port: config.port,
                error: bind_error(address, &err),
            };
            tracing::warn!("the endpoint could not bind {address}: {err}");
            config.state_sink.report(&state);
            return Ok(state);
        }
    };

    if let Err(err) = listener.set_nonblocking(true) {
        let state = EndpointState::BindFailed {
            address: config.address,
            port: config.port,
            error: AppError::Io {
                message: format!("could not put the listening socket in non-blocking mode: {err}"),
            },
        };
        tracing::warn!("the endpoint could not serve {address}: {err}");
        config.state_sink.report(&state);
        return Ok(state);
    }

    let context = Arc::new(forward::ProxyContext {
        client,
        upstream: config.upstream,
        limit: config
            .max_concurrent_requests
            .filter(|limit| *limit > 0)
            .map(|limit| Arc::new(Semaphore::new(limit as usize))),
    });

    // The *bound* address, not the requested one: with port `0` the OS picks,
    // and a state that reported the request instead of the socket would name a
    // port nobody is listening on. In production the two are the same. Read
    // before the socket is handed to the thread that owns it.
    let bound = listener_local_address(&listener, address);

    serve_on_its_own_thread(listener, context);
    let state = EndpointState::Bound {
        address: bound.ip(),
        port: bound.port(),
    };
    // The app's own log answers "is the endpoint up, and where?" without a
    // screen existing yet (the Dashboard is T-045), the same way the
    // supervisor and the orchestrator each log one line at startup.
    tracing::info!("endpoint listening on {bound}");
    config.state_sink.report(&state);
    Ok(state)
}

fn listener_local_address(listener: &std::net::TcpListener, requested: SocketAddr) -> SocketAddr {
    match listener.local_addr() {
        Ok(address) => address,
        // A socket that cannot report its own address is still bound; saying
        // so with the requested address is better than refusing to serve.
        Err(_) => requested,
    }
}

/// A port that is taken is `PortInUse`; anything else about a bind is an
/// `Io` error and says so. Naming a permission failure "port in use" would
/// send a user to look for a conflict that does not exist.
fn bind_error(address: SocketAddr, err: &std::io::Error) -> AppError {
    if err.kind() == std::io::ErrorKind::AddrInUse {
        AppError::PortInUse {
            port: address.port(),
        }
    } else {
        AppError::Io {
            message: format!("could not bind {address}: {err}"),
        }
    }
}

/// Hand the bound socket to its own thread and runtime.
///
/// The socket is already bound here, so this cannot fail in a way the caller
/// could report as a state — a failure to start the thread or the runtime
/// leaves a socket bound by nobody, which is logged as the error it is.
fn serve_on_its_own_thread(listener: std::net::TcpListener, context: Arc<forward::ProxyContext>) {
    let spawned = thread::Builder::new()
        .name("endpoint".to_string())
        .spawn(move || {
            let runtime = match RuntimeBuilder::new_multi_thread()
                .worker_threads(WORKER_THREADS)
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(err) => {
                    tracing::error!("could not start the endpoint's runtime: {err}");
                    return;
                }
            };

            runtime.block_on(async move {
                // `from_std` needs a runtime in scope, which is why the
                // non-blocking `std` listener crosses the thread boundary and
                // becomes a tokio one here.
                let listener = match TcpListener::from_std(listener) {
                    Ok(listener) => listener,
                    Err(err) => {
                        tracing::error!("the endpoint could not adopt its socket: {err}");
                        return;
                    }
                };

                // A fallback rather than a route table: every method and every
                // path reach the same handler, because deciding anything from
                // the path is exactly the job this module does not have.
                let app = Router::new()
                    .fallback(forward::forward)
                    .with_state(Arc::clone(&context));

                if let Err(err) = axum::serve(listener, app).await {
                    tracing::error!("the endpoint listener stopped: {err}");
                }
            });
        });

    if let Err(err) = spawned {
        tracing::error!("could not start the endpoint's thread: {err}");
    }
}
