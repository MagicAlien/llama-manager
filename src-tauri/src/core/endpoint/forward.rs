//! The forwarding handler: the whole data path, and the only place a client
//! request is touched.
//!
//! Two rules hold everywhere below, and both are properties the acceptance
//! suite asserts rather than comments:
//!
//! 1. **The body is never read.** It is moved from the incoming request into
//!    the outgoing one as a stream. There is no `Deserialize` in this file, no
//!    `serde_json::from_*`, and no type in this module that could hold a
//!    parsed request — a body that is not JSON at all forwards successfully,
//!    because nothing here would notice.
//! 2. **Nothing accumulates.** Request and response bodies are forwarded as
//!    frames arrive; the response is handed back as a stream, never collected.
//!    An SSE answer that is buffered anywhere becomes one delayed blob, which
//!    is the failure `PLAN.md` §2.7 names as the most likely to ship
//!    unnoticed.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::http::header::{HeaderMap, HeaderValue, CONNECTION, RETRY_AFTER};
use axum::http::{Response, StatusCode};
use futures_util::Stream;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::UpstreamSource;
use crate::core::types::AppError;

/// Seconds advertised in `Retry-After` when the app refuses a request of its
/// own accord. One second is honest for a limit that clears as soon as an
/// in-flight response finishes and is not a promise about the upstream.
const RETRY_AFTER_SECONDS: u64 = 1;

/// Everything the handler needs, built once by [`super::start`].
pub(super) struct ProxyContext {
    /// One client, reused: connection pooling to a single loopback upstream is
    /// what keeps the added latency in the microsecond range.
    pub(super) client: reqwest::Client,
    pub(super) upstream: Arc<dyn UpstreamSource>,
    /// `None` — `ServerConfig.max_concurrent_requests` unset — means no limit
    /// of the app's own. When set, the app refuses the surplus **immediately**
    /// rather than holding it: queueing is a scheduling decision, and
    /// scheduling is not this app's job (`AGENTS.md` invariant 3).
    pub(super) limit: Option<Arc<Semaphore>>,
}

/// `Any` method, any path: the listener has one route.
pub(super) async fn forward(
    State(context): State<Arc<ProxyContext>>,
    request: Request,
) -> Response<Body> {
    // The permit is taken before anything else so that a refused request costs
    // the upstream nothing, and it travels into the response body so it is
    // released when the last chunk is written — not when the handler returns,
    // which for a stream is the same moment the response *starts*.
    let permit = match &context.limit {
        None => None,
        Some(limit) => match Arc::clone(limit).try_acquire_owned() {
            Ok(permit) => Some(permit),
            Err(_) => return refused(),
        },
    };

    let Some(upstream) = context.upstream.upstream() else {
        // No upstream allocated: the server is not running. T-044 owns the
        // per-state answer (including the hold window while the process is
        // starting); the transport's honest answer is that there is nothing to
        // forward to.
        return unavailable(&AppError::UpstreamUnavailable {
            state: "no upstream is allocated — the llama.cpp server is not running".to_string(),
        });
    };

    let (parts, body) = request.into_parts();

    let uri = match upstream_uri(upstream, &parts.uri) {
        Some(uri) => uri,
        None => {
            return unavailable(&AppError::Internal {
                message: format!("could not forward the request path {}", parts.uri.path()),
            })
        }
    };

    let outgoing = context
        .client
        .request(parts.method.clone(), uri)
        .headers(forward_headers(&parts.headers))
        // The body crosses as a stream of frames, exactly as it arrived. Its
        // framing (chunked vs. a declared length) is hop-by-hop and legitimately
        // re-derived by hyper on the way out; the octets are not touched.
        .body(reqwest::Body::wrap_stream(body.into_data_stream()));

    let response = match outgoing.send().await {
        Ok(response) => response,
        Err(err) => {
            return unavailable(&AppError::UpstreamUnavailable {
                state: format!("the upstream at {upstream} did not answer: {err}"),
            })
        }
    };

    relay(response, permit)
}

/// The path and query **as a string**, appended to the upstream's authority.
///
/// `path_and_query` is taken verbatim rather than re-encoded from parts, so a
/// query string reaches llama-server byte-for-byte: no re-ordering, no
/// re-escaping, nothing that a client could notice.
fn upstream_uri(upstream: std::net::SocketAddr, uri: &axum::http::Uri) -> Option<reqwest::Url> {
    let path_and_query = uri.path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    reqwest::Url::parse(&format!("http://{upstream}{path_and_query}")).ok()
}

/// Turn an upstream response into the client's response, forwarding the body
/// as it arrives.
fn relay(response: reqwest::Response, permit: Option<OwnedSemaphorePermit>) -> Response<Body> {
    let status = response.status();
    let headers = forward_headers(response.headers());
    let stream = permit_stream(response.bytes_stream(), permit);

    let mut relayed = Response::new(Body::from_stream(stream));
    *relayed.status_mut() = status;
    *relayed.headers_mut() = headers;
    relayed
}

/// The upstream's frames, with the concurrency permit held for their lifetime.
struct PermitStream<S> {
    inner: S,
    _permit: Option<OwnedSemaphorePermit>,
}

impl<S: Stream + Unpin> Stream for PermitStream<S> {
    type Item = S::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.get_mut().inner).poll_next(cx)
    }
}

fn permit_stream<S>(stream: S, permit: Option<OwnedSemaphorePermit>) -> PermitStream<BoxStream>
where
    S: Stream<Item = reqwest::Result<Bytes>> + Send + 'static,
{
    // Boxed so the wrapper is `Unpin` without a pin projection: one allocation
    // per response, next to which the HTTP work it protects is enormous, and
    // no `unsafe` in the crate's data path.
    let inner: BoxStream = Box::pin(stream);
    PermitStream {
        inner,
        _permit: permit,
    }
}

type BoxStream = Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>;

/// The headers that are the connection's business, not the message's
/// (RFC 9110 §7.6.1 — the same set RFC 7230 §6.1 listed).
///
/// `Trailer`, `Transfer-Encoding` and `Connection` cannot be forwarded: the
/// hop that ends a connection is this one, and hyper re-derives all three. The
/// `Connection` header can also *name* further hop-by-hop headers, which are
/// therefore dropped too — that is the rule the acceptance suite's "minus
/// hop-by-hop" comparison is written against, and trailers are part of it.
const HOP_BY_HOP: [&str; 8] = [
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

fn forward_headers(source: &HeaderMap) -> HeaderMap {
    let named = named_by_connection(source);
    let mut forwarded = HeaderMap::new();
    for (name, value) in source {
        let lower = name.as_str();
        if HOP_BY_HOP.contains(&lower) || named.iter().any(|named| named == lower) {
            continue;
        }
        forwarded.append(name.clone(), value.clone());
    }
    forwarded
}

/// The header names a `Connection` header lists as hop-by-hop.
fn named_by_connection(source: &HeaderMap) -> Vec<String> {
    source
        .get_all(CONNECTION)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect()
}

/// `503` in OpenAI error shape, so a standard client surfaces it as a server
/// error rather than a parse failure (`docs/CONTRACTS.md` §2).
fn unavailable(error: &AppError) -> Response<Body> {
    let body = serde_json::json!({
        "error": {
            "message": error.message(),
            "type": "server_error",
            "code": error.code(),
        }
    })
    .to_string();
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// The app refusing a request of its own accord — over the concurrency limit.
///
/// `503` with `Retry-After`, as `docs/TASKS.md` T-042 specifies. The body's
/// own code is `rate_limited`: the app is saying it refused this itself, which
/// is a different fact from the upstream having failed.
fn refused() -> Response<Body> {
    let mut response = unavailable(&AppError::RateLimited {
        retry_after_seconds: RETRY_AFTER_SECONDS,
    });
    response.headers_mut().insert(
        RETRY_AFTER,
        HeaderValue::from_str(&RETRY_AFTER_SECONDS.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("1")),
    );
    response
}
