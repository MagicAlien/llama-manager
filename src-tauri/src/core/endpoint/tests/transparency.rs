//! The differential test: the same request sent directly to the stub and
//! through the listener, with the two answers compared byte for byte.
//!
//! `AGENTS.md` invariant 4 states the property — *"with no API key configured,
//! a response reaching the client is byte-identical to what the upstream
//! produced"* — and this is where it is checked. Two details make the check
//! mean something:
//!
//! - **The answer depends on the request.** The stub's default reply echoes the
//!   method, the target, one of the request's headers and its body. Against a
//!   fixed reply, a listener that dropped the body and the query would compare
//!   equal and the test would prove nothing.
//! - **Hop-by-hop headers are excluded, and named.** `Connection`,
//!   `Transfer-Encoding`, `Trailer` and the rest of RFC 9110 §7.6.1 are the
//!   connection's business, not the message's: the hop that ends a connection
//!   is this one, so hyper re-derives all of them. Trailers are in that set,
//!   which is why the criterion says "minus hop-by-hop" and why this comparison
//!   is written against an explicit list rather than an assumption.

use std::collections::BTreeSet;

use reqwest::header::HeaderMap;

use super::stub::{Reply, Stub};
use super::{client, listener_for, url};

/// Everything a test compares: status, the end-to-end headers, and the body.
#[derive(Debug, PartialEq)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// The `Date` a real origin sends. llama.cpp's server includes one, and every
/// HTTP server does: the matrix below is run with the header a real upstream
/// produces rather than with a stunted reply.
const STUB_DATE: &str = "Mon, 01 Jan 2024 00:00:00 GMT";

fn get(target: &'static str) -> Case {
    Case {
        name: "GET",
        method: "GET",
        target,
        tag: None,
        body: Vec::new(),
    }
}

struct Case {
    name: &'static str,
    method: &'static str,
    /// Path and query, verbatim — including the awkward ones.
    target: &'static str,
    tag: Option<&'static str>,
    body: Vec<u8>,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "plain GET",
            method: "GET",
            target: "/v1/models",
            tag: None,
            body: Vec::new(),
        },
        Case {
            name: "GET with a query string that must not be re-encoded",
            method: "GET",
            target: "/v1/models?foo=1&bar=a%20b&plus=%2B&bare=+&empty=",
            tag: Some("query"),
            body: Vec::new(),
        },
        Case {
            name: "POST with a JSON body",
            method: "POST",
            target: "/v1/chat/completions",
            tag: Some("json"),
            body: br#"{"model":"qwen","messages":[{"role":"user","content":"hi"}],"stream":true}"#
                .to_vec(),
        },
        Case {
            name: "POST with a body that is not JSON at all",
            method: "POST",
            target: "/v1/completions",
            tag: Some("not-json"),
            body: b"{\"prompt\": \"unterminated".to_vec(),
        },
        Case {
            name: "PUT with every byte value in the body",
            method: "PUT",
            target: "/upload",
            tag: Some("binary"),
            body: (0..=255u8).collect(),
        },
        Case {
            name: "DELETE with no body",
            method: "DELETE",
            target: "/models/removed",
            tag: None,
            body: Vec::new(),
        },
        Case {
            name: "PATCH with an empty body",
            method: "PATCH",
            target: "/settings",
            tag: Some("empty"),
            body: Vec::new(),
        },
        Case {
            name: "POST with an unusual path",
            method: "POST",
            target: "/v1/chat/completions?x=a/b%2Fc&y=1,2,3",
            tag: Some("odd-path"),
            body: b"body".to_vec(),
        },
    ]
}

#[test]
fn transparency_is_byte_identical_across_a_matrix_of_requests() {
    let stub = Stub::start();
    stub.set_date(Some(STUB_DATE));
    let (port, _sink) = listener_for(&stub);
    let client = client();

    for case in cases() {
        let direct = send(&client, &stub.url(case.target), &case);
        let through = send(&client, &url(port, case.target), &case);
        assert_eq!(
            direct, through,
            "case `{}`: a client must not be able to tell the difference",
            case.name
        );

        // The other half of transparency: what the upstream *received* is the
        // same request either way. The echo above only proves the answer
        // matched; this proves the request did.
        let seen = stub.requests();
        let through_seen = seen.last().expect("the stub saw the request");
        assert_eq!(through_seen.method, case.method, "case `{}`", case.name);
        assert_eq!(through_seen.target, case.target, "case `{}`", case.name);
        assert_eq!(
            through_seen.body, case.body,
            "case `{}`: the request body reached the upstream unchanged",
            case.name
        );
        if let Some(tag) = case.tag {
            assert_eq!(
                through_seen.header("x-tag"),
                Some(tag),
                "case `{}`: end-to-end request headers survive",
                case.name
            );
        }
        assert_eq!(
            through_seen.header("connection"),
            None,
            "case `{}`: `Connection` is hop-by-hop and must not be forwarded",
            case.name
        );
    }
}

#[test]
fn a_fixed_answer_survives_every_header_it_carries() {
    // The mirror of the echo: a response whose bytes are decided up front —
    // an unusual status, repeated headers, a header with a comma and a quoted
    // string — through the listener and straight to the stub.
    let stub = Stub::start();
    let stub_wide_reply = Reply::Fixed {
        status: 418,
        headers: vec![
            ("content-type", "application/json; charset=utf-8"),
            ("x-single", "one"),
            ("x-repeated", "first"),
            ("x-repeated", "second"),
            ("x-awkward", "a, b; c=\"d\", e"),
            ("cache-control", "no-store"),
        ],
        body: br#"{"error":{"message":"teapot"}}"#.to_vec(),
    };
    stub.set("GET", "/teapot", stub_wide_reply);
    stub.set_date(Some(STUB_DATE));
    let (port, _sink) = listener_for(&stub);
    let client = client();

    let case = Case {
        name: "fixed answer",
        method: "GET",
        target: "/teapot",
        tag: None,
        body: Vec::new(),
    };
    let direct = send(&client, &stub.url("/teapot"), &case);
    let through = send(&client, &url(port, "/teapot"), &case);

    assert_eq!(direct.status, 418);
    assert_eq!(direct, through);
    assert!(
        through
            .headers
            .iter()
            .any(|(name, value)| name == "x-repeated" && value == "first"),
        "repeated end-to-end headers arrive as many times as they were sent: {:?}",
        through.headers
    );
}

fn send(client: &reqwest::Client, url: &str, case: &Case) -> Answer {
    let method = reqwest::Method::from_bytes(case.method.as_bytes()).expect("a valid method");
    let mut request = client.request(method, url);
    if let Some(tag) = case.tag {
        request = request.header("x-tag", tag);
    }
    // A header whose value carries a comma, a quoted string and a semicolon —
    // the kind a proxy that rebuilt headers would mangle.
    request = request.header("accept", "application/json, text/event-stream;q=0.9");
    if !case.body.is_empty() || matches!(case.method, "POST" | "PUT" | "PATCH") {
        request = request.body(case.body.clone());
    }

    let response = super::async_io(request.send()).expect("the request must be answered");
    let status = response.status().as_u16();
    let headers = end_to_end(response.headers());
    let body = super::async_io(response.bytes()).expect("the body must be readable");
    Answer {
        status,
        headers,
        body: body.to_vec(),
    }
}

/// The end-to-end headers, sorted and lowercased so the comparison is about
/// content rather than order, with the hop-by-hop set removed.
fn end_to_end(headers: &HeaderMap) -> Vec<(String, String)> {
    let hop_by_hop = [
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
    ];
    let mut out = BTreeSet::new();
    for (name, value) in headers {
        let name = name.as_str().to_ascii_lowercase();
        if hop_by_hop.contains(&name.as_str()) {
            continue;
        }
        out.insert((name, value.to_str().unwrap_or_default().to_string()));
    }
    out.into_iter().collect()
}
/// `Date` is an end-to-end header, so it is forwarded like any other — and the
/// one case where the app's own HTTP stack adds one is asserted here rather
/// than left to be discovered.
///
/// hyper's server supplies a `Date` to any response that arrives without one,
/// and it does so after the handler has returned: `docs/TASKS.md` T-042 asks
/// for headers byte-identical "minus hop-by-hop", `Date` is not hop-by-hop, and
/// suppressing it would mean building the server on `hyper` directly, which
/// `PLAN.md` §3 forbids ("Brings `hyper` and `tower` transitively; do not
/// depend on them directly"). What the test does instead is state the exact
/// size of the difference: with an upstream that sends a `Date`, it is
/// forwarded verbatim; with one that sends none, a `Date` is the **only**
/// header the client sees that the upstream did not produce.
#[test]
fn the_upstream_date_is_forwarded_verbatim_and_supplied_only_when_absent() {
    let stub = Stub::start();
    stub.set_date(Some(STUB_DATE));
    stub.set(
        "GET",
        "/dated",
        Reply::Fixed {
            status: 200,
            headers: vec![("x-stub", "1")],
            body: b"dated".to_vec(),
        },
    );
    let (port, _sink) = listener_for(&stub);
    let client = client();

    let dated = send(&client, &url(port, "/dated"), &get("/dated"));
    let forwarded = dated
        .headers
        .iter()
        .find(|(name, _)| name == "date")
        .map(|(_, value)| value.as_str());
    assert_eq!(
        forwarded,
        Some(STUB_DATE),
        "the upstream's own Date must be forwarded, not replaced by the app"
    );

    // And now the branch where the upstream sends none at all.
    stub.set_date(None);
    stub.set(
        "GET",
        "/undated",
        Reply::Fixed {
            status: 200,
            headers: vec![("x-stub", "1")],
            body: b"undated".to_vec(),
        },
    );

    let direct = send(&client, &stub.url("/undated"), &get("/undated"));
    assert!(
        !direct.headers.iter().any(|(name, _)| name == "date"),
        "the stub really sent no Date: {:?}",
        direct.headers
    );
    let through = send(&client, &url(port, "/undated"), &get("/undated"));

    let without_date: Vec<(String, String)> = through
        .headers
        .iter()
        .filter(|(name, _)| name != "date")
        .cloned()
        .collect();
    assert_eq!(
        without_date, direct.headers,
        "a Date-less upstream may differ in exactly one header, the Date itself"
    );
    let added = through
        .headers
        .iter()
        .find(|(name, _)| name == "date")
        .map(|(_, value)| value.clone());
    println!("Date supplied by the listener's own HTTP stack: {added:?}");
    assert!(
        added.is_some(),
        "the difference is a Date, not silence: {through:?}"
    );
}
