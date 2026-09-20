//! Streaming: the property that a buffered proxy would fail while passing
//! every other test in this file.
//!
//! A concatenated-bodies comparison cannot detect accumulation — one delayed
//! blob and fifty prompt chunks concatenate to the same bytes. So the
//! assertions here are about **frame boundaries and their timing**: the number
//! of frames the client sees, and the gaps between them. `PLAN.md` §2.7 names
//! this as "the failure most likely to ship unnoticed".
//!
//! The delays are scaled down from the product's real numbers (a 65 GB preload
//! is minutes, an SSE token is milliseconds) because a suite that waited three
//! minutes could not run in CI. What is *not* scaled is the configuration: the
//! production timeout is asserted against its documented floor in
//! `the_production_timeout_is_explicit_and_permits_a_three_minute_response`,
//! and the enforcement test drives the same code path with a shorter value.

use std::time::{Duration, Instant};

use futures_util::StreamExt;

use super::stub::{sse_chunks, sse_reply, Reply, Stub};
use super::{async_io, client, listener, listener_for, url, Timeouts};

struct Frames {
    status: u16,
    /// Time from sending the request to the response head arriving.
    to_head: Duration,
    /// One entry per frame the client observed, in arrival order.
    chunks: Vec<Vec<u8>>,
    /// The instant each frame arrived, measured from the request being sent.
    at: Vec<Duration>,
}

impl Frames {
    fn gaps(&self) -> Vec<Duration> {
        self.at
            .windows(2)
            .map(|pair| pair[1].saturating_sub(pair[0]))
            .collect()
    }
}

async fn frames(client: &reqwest::Client, url: &str) -> Frames {
    let start = Instant::now();
    let response = client
        .get(url)
        .send()
        .await
        .expect("the request must be sent");
    let status = response.status().as_u16();
    let to_head = start.elapsed();
    let mut stream = response.bytes_stream();
    let mut chunks = Vec::new();
    let mut at = Vec::new();
    while let Some(item) = stream.next().await {
        let bytes = item.expect("every frame must arrive intact");
        chunks.push(bytes.to_vec());
        at.push(start.elapsed());
    }
    Frames {
        status,
        to_head,
        chunks,
        at,
    }
}

fn median(values: &mut [Duration]) -> Duration {
    if values.is_empty() {
        return Duration::ZERO;
    }
    values.sort();
    values[values.len() / 2]
}

#[test]
fn streamed_chunks_keep_their_boundaries_and_their_timing() {
    let stub = Stub::start();
    let chunks = sse_chunks(50);
    let gap = Duration::from_millis(30);
    stub.set(
        "GET",
        "/stream",
        sse_reply(chunks.clone(), Duration::ZERO, gap),
    );
    let (port, _sink) = listener_for(&stub);
    let client = client();

    let direct = async_io(frames(&client, &stub.url("/stream")));
    let through = async_io(frames(&client, &url(port, "/stream")));

    assert_eq!(direct.status, 200);
    assert_eq!(through.status, 200);

    // The head is not held back either: the reply starts as promptly through
    // the listener as it does straight from the upstream.
    assert!(
        through.to_head <= direct.to_head + Duration::from_millis(50),
        "the response head must not be delayed: direct {:?}, through {:?}",
        direct.to_head,
        through.to_head
    );

    // Boundaries: every emitted chunk is its own frame on the client side. A
    // proxy that accumulated the stream would deliver one frame, or a handful.
    assert_eq!(
        direct.chunks, chunks,
        "the stub is the reference: 50 chunks, 50 frames"
    );
    assert_eq!(
        through.chunks.len(),
        chunks.len(),
        "each chunk must reach the client as its own frame, not coalesced: {} frames for {} chunks",
        through.chunks.len(),
        chunks.len()
    );
    for (index, (expected, observed)) in chunks.iter().zip(&through.chunks).enumerate() {
        assert_eq!(expected, observed, "frame {index} differs");
    }

    // Timing: each frame waited for the next one to be written. A buffer that
    // held the stream and released it at the end would produce one long pause
    // and then gaps near zero.
    let through_gaps = through.gaps();
    assert_eq!(through_gaps.len(), chunks.len() - 1);
    let smallest = through_gaps.iter().min().copied().unwrap_or_default();
    assert!(
        smallest >= gap / 2,
        "frames must arrive spread over time, not in a burst at the end: \
         smallest gap {smallest:?} against an emitted gap of {gap:?}\n\
         through-listener gaps: {through_gaps:?}"
    );

    // And the stream is not slowed down on the way: the whole exchange takes
    // about as long as the upstream took to write it.
    let direct_total = *direct.at.last().unwrap_or(&Duration::ZERO);
    let through_total = *through.at.last().unwrap_or(&Duration::ZERO);
    println!(
        "streaming: 50 chunks, {gap:?} apart — direct {direct_total:?}, through the listener {through_total:?}, \
         median gap direct {:?}, median gap through {:?}",
        median(&mut direct.gaps()),
        median(&mut through.gaps()),
    );
    assert!(
        through_total <= direct_total + gap * 3,
        "the listener must not add a delay of its own: direct {direct_total:?}, through {through_total:?}"
    );
}

#[test]
fn a_response_that_begins_after_a_long_delay_completes() {
    // T-042 asks for three minutes, which cannot be spent in CI. The same path
    // is driven here with a delay four orders of magnitude shorter, and the
    // production allowance is pinned separately — the configuration is what the
    // three-minute criterion is about, not the wall clock of the suite.
    let stub = Stub::start();
    let chunks = sse_chunks(4);
    stub.set(
        "GET",
        "/slow-start",
        sse_reply(
            chunks.clone(),
            Duration::from_millis(300),
            Duration::from_millis(5),
        ),
    );
    let (port, _sink) = listener_for(&stub);
    let client = client();

    let start = Instant::now();
    let through = async_io(frames(&client, &url(port, "/slow-start")));
    let elapsed = start.elapsed();

    assert_eq!(through.status, 200);
    assert_eq!(through.chunks, chunks);
    assert!(
        elapsed >= Duration::from_millis(300),
        "the delay really happened: {elapsed:?}"
    );
    println!(
        "long-held request: first byte after {elapsed:?}, {} chunks",
        through.chunks.len()
    );
}

#[test]
fn the_production_timeout_is_explicit_and_permits_a_three_minute_response() {
    // "with the timeout configured explicitly in code and asserted by a test
    // rather than left to a default" (`docs/TASKS.md` T-042). The floor is the
    // task's own three-minute case; the ceiling matters as much — an unbounded
    // wait would pin a client forever behind an upstream that never answers.
    let production = Timeouts::default();
    assert!(
        production.upstream_response >= Duration::from_secs(180),
        "a response that begins after three minutes must complete: {}",
        production.upstream_response.as_secs()
    );
    assert!(
        production.upstream_response <= Duration::from_secs(3600),
        "and it must be finite: {:?}",
        production.upstream_response
    );
    assert!(
        production.connect <= Duration::from_secs(30),
        "connecting to a loopback socket that is not listening fails at once; \
         the ceiling only bounds a pathological case: {:?}",
        production.connect
    );
}

#[test]
fn a_response_that_outlives_the_configured_timeout_is_refused_rather_than_held() {
    let stub = Stub::start();
    let chunks = sse_chunks(2);
    // The response *begins* after 900 ms, against a listener allowed 250 ms:
    // nothing has been sent upstream when the timeout fires, so the listener
    // can still answer with a status of its own.
    stub.set(
        "GET",
        "/too-slow",
        sse_reply(
            chunks.clone(),
            Duration::from_millis(900),
            Duration::from_millis(5),
        ),
    );
    // The control: the same listener, a delay inside its allowance.
    stub.set(
        "GET",
        "/slow-but-inside",
        sse_reply(
            chunks.clone(),
            Duration::from_millis(50),
            Duration::from_millis(5),
        ),
    );

    let timeouts = Timeouts {
        upstream_response: Duration::from_millis(250),
        connect: Duration::from_secs(2),
    };
    let (port, _sink) = listener(
        std::sync::Arc::new(super::FixedUpstream(Some(stub.address()))),
        None,
        timeouts,
    );
    let client = client();

    let start = Instant::now();
    let refused = async_io(async {
        let response = client
            .get(url(port, "/too-slow"))
            .send()
            .await
            .expect("the listener answers instead of hanging");
        let status = response.status().as_u16();
        let body = response.text().await.expect("a body");
        (status, body)
    });
    let elapsed = start.elapsed();

    assert_eq!(refused.0, 503, "body: {}", refused.1);
    let parsed: serde_json::Value = serde_json::from_str(&refused.1).expect("JSON body");
    assert_eq!(parsed["error"]["code"], "upstream_unavailable");
    assert!(
        elapsed < Duration::from_millis(800),
        "the configured 250 ms timeout ended the request, not the upstream's \
         900 ms answer: it took {elapsed:?}"
    );

    // The same listener, with a delay inside its allowance, serves normally:
    // this is what makes the assertion above about the timeout rather than
    // about a listener that refuses everything.
    let served = async_io(frames(&client, &url(port, "/slow-but-inside")));
    assert_eq!(served.status, 200);
    assert_eq!(served.chunks, chunks);
}

#[test]
fn hop_by_hop_headers_named_by_connection_are_not_forwarded() {
    // RFC 9110 §7.6.1: a `Connection` header names further headers that are
    // hop-by-hop for this hop, so they must be dropped with it. Sending
    // `Connection: x-custom` and keeping `x-custom` would leak a header the
    // upstream must not see.
    let stub = Stub::start();
    stub.set(
        "GET",
        "/named",
        Reply::Fixed {
            status: 200,
            headers: vec![("x-stub", "kept")],
            body: b"ok".to_vec(),
        },
    );
    let (port, _sink) = listener_for(&stub);
    let client = client();

    let response = async_io(async {
        client
            .get(url(port, "/named"))
            .header("connection", "x-custom")
            .header("x-custom", "must-not-arrive")
            .header("x-kept", "arrives")
            .send()
            .await
            .expect("the request must be answered")
    });
    assert_eq!(response.status(), 200);

    let seen = stub.requests();
    let request = seen.last().expect("the stub saw the request");
    assert_eq!(request.header("x-custom"), None, "{request:?}");
    assert_eq!(request.header("x-kept"), Some("arrives"));
}
