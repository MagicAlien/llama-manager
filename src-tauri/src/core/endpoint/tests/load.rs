//! Load: the numbers T-042 puts on "transparent" so it cannot be renegotiated
//! downwards one commit at a time, and the one limit the app imposes of its own
//! accord.
//!
//! The latency measurement runs in CI against the stub on a shared runner, so
//! the statistic is specified rather than left to chance: **five sequential
//! batches of 200, the p99 taken per batch, and the median of the five compared
//! against the threshold**. A single batch over budget is noise; three of five
//! over budget is the regression. Both numbers are printed on every run, pass or
//! fail, so drift is visible before it crosses.

use std::time::{Duration, Instant};

use futures_util::StreamExt;

use super::stub::{sse_reply, Stub};
use super::{async_io, client, fixed_upstream, listener, listener_for, url, Timeouts};

const BATCH: usize = 200;
const BATCHES: usize = 5;
/// Non-streaming: the p99 of (through the listener − direct to the stub).
const P99_BUDGET: Duration = Duration::from_millis(5);
/// Streaming: the added time to first byte.
const TTFB_BUDGET: Duration = Duration::from_millis(15);
const TTFB_SAMPLES: usize = 50;

async fn timed(client: &reqwest::Client, url: &str) -> Duration {
    let start = Instant::now();
    let response = client
        .get(url)
        .send()
        .await
        .expect("the request is answered");
    let _ = response.bytes().await.expect("the body is readable");
    start.elapsed()
}

async fn batch(client: &reqwest::Client, url: &str) -> Vec<Duration> {
    let mut samples = Vec::with_capacity(BATCH);
    for _ in 0..BATCH {
        samples.push(timed(client, url).await);
    }
    samples
}

/// Nearest-rank p99: the value at or below which 99% of the samples fall.
fn p99(samples: &mut [Duration]) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    samples.sort();
    let rank = ((samples.len() as f64) * 0.99).ceil() as usize;
    samples[rank.saturating_sub(1).min(samples.len() - 1)]
}

fn median(values: &mut [Duration]) -> Duration {
    if values.is_empty() {
        return Duration::ZERO;
    }
    values.sort();
    values[values.len() / 2]
}

async fn to_head(client: &reqwest::Client, url: &str) -> Duration {
    let start = Instant::now();
    let response = client
        .get(url)
        .send()
        .await
        .expect("the request is answered");
    let elapsed = start.elapsed();
    // The response is dropped without being read: this measures the head, and
    // closing early is the point.
    drop(response);
    elapsed
}

async fn drain(response: reqwest::Response) -> Result<usize, reqwest::Error> {
    let mut stream = response.bytes_stream();
    let mut frames = 0;
    while let Some(item) = stream.next().await {
        item?;
        frames += 1;
    }
    Ok(frames)
}

#[test]
fn the_added_latency_stays_under_the_budget_on_five_batches_of_two_hundred() {
    let stub = Stub::start();
    let (port, _sink) = listener_for(&stub);
    let client = client();
    let direct_url = stub.url("/latency");
    let through_url = url(port, "/latency");

    // Warm both paths: the first request on each pays for a connection setup,
    // and neither of them is what "added latency" means.
    async_io(async {
        let _ = timed(&client, &direct_url).await;
        let _ = timed(&client, &through_url).await;
    });

    let mut deltas = Vec::with_capacity(BATCHES);
    for batch_index in 0..BATCHES {
        let direct = async_io(batch(&client, &direct_url));
        let through = async_io(batch(&client, &through_url));
        let mut direct = direct;
        let mut through = through;
        let direct_p99 = p99(&mut direct);
        let through_p99 = p99(&mut through);
        let delta = through_p99.saturating_sub(direct_p99);
        println!(
            "latency batch {}: direct p99 {direct_p99:?}, through p99 {through_p99:?}, delta {delta:?}",
            batch_index + 1
        );
        deltas.push(delta);
    }

    let mut sorted = deltas.clone();
    let median_delta = median(&mut sorted);
    println!(
        "latency: median of {BATCHES} p99 deltas = {median_delta:?} (budget {P99_BUDGET:?}); \
         all deltas: {deltas:?}"
    );

    let over_budget = deltas.iter().filter(|delta| **delta >= P99_BUDGET).count();
    assert!(
        median_delta < P99_BUDGET,
        "the median p99 delta is {median_delta:?}, at or over the {P99_BUDGET:?} budget \
         ({over_budget} of {BATCHES} batches over). Fix the measurement or the listener — \
         never the threshold: a number that can be negotiated downward one commit at a time \
         is not a number."
    );
}

#[test]
fn the_added_time_to_first_byte_stays_under_the_streaming_budget() {
    let stub = Stub::start();
    stub.set(
        "GET",
        "/ttfb",
        sse_reply(
            super::stub::sse_chunks(3),
            Duration::ZERO,
            Duration::from_millis(20),
        ),
    );
    let (port, _sink) = listener_for(&stub);
    let client = client();

    // Warm-ups, as above.
    async_io(async {
        let _ = to_head(&client, &stub.url("/ttfb")).await;
        let _ = to_head(&client, &url(port, "/ttfb")).await;
    });

    let mut deltas = Vec::with_capacity(TTFB_SAMPLES);
    for _ in 0..TTFB_SAMPLES {
        let direct = async_io(to_head(&client, &stub.url("/ttfb")));
        let through = async_io(to_head(&client, &url(port, "/ttfb")));
        deltas.push(through.saturating_sub(direct));
    }
    let mut sorted = deltas.clone();
    let median_delta = median(&mut sorted);
    println!(
        "time to first byte: median delta {median_delta:?} over {TTFB_SAMPLES} samples \
         (budget {TTFB_BUDGET:?}); worst {:?}, best {:?}",
        sorted.last().copied().unwrap_or_default(),
        sorted.first().copied().unwrap_or_default()
    );
    assert!(
        median_delta < TTFB_BUDGET,
        "the added time to first byte is {median_delta:?}, at or over the {TTFB_BUDGET:?} budget"
    );
}

#[test]
fn a_hundred_simultaneous_streams_are_all_served() {
    let stub = Stub::start();
    let chunks = super::stub::sse_chunks(20);
    stub.set(
        "GET",
        "/many",
        sse_reply(chunks.clone(), Duration::ZERO, Duration::from_millis(5)),
    );
    // `max_concurrent_requests` unset: the app imposes no limit of its own.
    let (port, _sink) = listener_for(&stub);
    let client = client();

    let results = async_io(async move {
        let mut tasks = Vec::new();
        for _ in 0..100 {
            let client = client.clone();
            let url = url(port, "/many");
            tasks.push(tokio::spawn(async move {
                let response = client.get(&url).send().await?;
                let status = response.status().as_u16();
                let frames = drain(response).await?;
                Ok::<_, reqwest::Error>((status, frames))
            }));
        }
        let mut out = Vec::new();
        for task in tasks {
            out.push(task.await.expect("no task may panic"));
        }
        out
    });

    assert_eq!(results.len(), 100);
    for (index, result) in results.iter().enumerate() {
        let (status, frames) = result.as_ref().unwrap_or_else(|err| {
            panic!("stream {index} failed: {err}");
        });
        assert_eq!(*status, 200, "stream {index}");
        assert_eq!(
            *frames,
            chunks.len(),
            "stream {index} lost frames — 100 simultaneous streams must all be served in full"
        );
    }
    assert_eq!(stub.request_count(), 100);
}

#[test]
fn with_a_limit_the_surplus_is_refused_immediately_and_not_queued() {
    let stub = Stub::start();
    let chunks = super::stub::sse_chunks(20);
    // ~800 ms of streaming, so the slot is held for long enough to observe.
    stub.set(
        "GET",
        "/slow",
        sse_reply(chunks.clone(), Duration::ZERO, Duration::from_millis(40)),
    );
    let (port, _sink) = listener(fixed_upstream(stub.address()), Some(1), Timeouts::default());
    let client = client();

    let (status, retry_after, body, elapsed, held_frames) = async_io(async {
        let slow_client = client.clone();
        let slow_url = url(port, "/slow");
        let slow = tokio::spawn(async move {
            let response = slow_client.get(&slow_url).send().await?;
            let frames = drain(response).await?;
            Ok::<usize, reqwest::Error>(frames)
        });

        // The permit is taken before anything reaches the upstream, so once the
        // stub has seen the request the slot is certainly held.
        while stub.request_count() == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        let start = Instant::now();
        let refused = client
            .get(url(port, "/slow"))
            .send()
            .await
            .expect("the surplus request is answered, not queued");
        let status = refused.status().as_u16();
        let retry_after = refused
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let body = refused.text().await.expect("a body");
        let elapsed = start.elapsed();

        let held_frames = slow
            .await
            .expect("the held stream must complete")
            .expect("the held stream must be served");
        (status, retry_after, body, elapsed, held_frames)
    });

    assert_eq!(status, 503, "body: {body}");
    assert_eq!(
        retry_after.as_deref(),
        Some("1"),
        "a limit that clears as soon as a response finishes advertises when to come back"
    );
    assert!(
        elapsed < Duration::from_millis(100),
        "the surplus is refused, not queued — queuing would be the app scheduling traffic \
         (`AGENTS.md` invariant 3): it took {elapsed:?}"
    );
    assert_eq!(
        held_frames,
        chunks.len(),
        "the request holding the only slot must still be served in full"
    );
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("JSON body");
    assert_eq!(parsed["error"]["code"], "rate_limited");

    // The permit is released when the stream ends, not when the handler
    // returned: the next request is served again.
    let after = async_io(async {
        let response = client.get(url(port, "/slow")).send().await.expect("served");
        let status = response.status().as_u16();
        let frames = drain(response).await.expect("the body");
        (status, frames)
    });
    assert_eq!(after.0, 200, "the slot is free again");
    assert_eq!(after.1, chunks.len());
}

#[test]
fn a_body_that_is_not_json_reaches_the_upstream_and_is_answered() {
    // T-042's fallback form of the no-parsing invariant, and the one a reviewer
    // can see running: a body no `Deserialize` could accept must forward
    // successfully, because nothing on this path looks at it.
    let stub = Stub::start();
    let (port, _sink) = listener_for(&stub);
    let client = client();
    let nonsense: Vec<u8> = b"{\"broken\": [1, 2, \x00\xff not json at all".to_vec();

    let (status, body) = async_io(async {
        let response = client
            .post(url(port, "/v1/chat/completions"))
            .header("content-type", "application/json")
            .body(nonsense.clone())
            .send()
            .await
            .expect("a body that is not JSON must still be forwarded");
        let status = response.status().as_u16();
        let body = response.text().await.expect("a body");
        (status, body)
    });

    assert_eq!(status, 200);
    let seen = stub.requests();
    let request = seen.last().expect("the stub saw the request");
    assert_eq!(
        request.body, nonsense,
        "the octets reached the upstream untouched"
    );
    assert!(
        body.contains("not json at all"),
        "the upstream answered using the body it received: {body}"
    );
    println!(
        "forwarded request framing: content-length {:?}, transfer-encoding {:?}",
        request.header("content-length"),
        request.header("transfer-encoding")
    );
}
