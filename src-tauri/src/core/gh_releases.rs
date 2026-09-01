//! T-020 — GitHub Releases client.
//!
//! Fetches llama.cpp releases from the GitHub API, parses asset names into
//! `(build_tag, Backend)`, and exposes the newest release per backend.
//! Nothing is downloaded, extracted or installed here (that is T-022), and
//! nothing is rendered (T-024).
//!
//! Endpoint: `GET /repos/ggml-org/llama.cpp/releases?per_page=100` against
//! `https://api.github.com`. One page on purpose: "newest per backend"
//! only needs recent releases, and walking the full release history is a
//! different, unbounded job no current task names.
//!
//! Design notes, recorded where a future reader would otherwise re-derive
//! them the expensive way:
//!
//! - **Pre-releases are included.** Every recent llama.cpp release carries
//!   `prerelease: true` (verified against the live API, 2026-08-31 — all
//!   ten of the newest releases do), so filtering them out would produce an
//!   empty catalogue. The task's contract is "newest per backend", not
//!   "newest stable".
//! - **Only `x64` Windows assets parse.** `rust-toolchain.toml` pins the
//!   app to a single target, `x86_64-pc-windows-msvc`; upstream publishes
//!   real `-arm64` Windows assets, which this app cannot install, so they
//!   are rejected like any other non-matching name. That rejection is by
//!   platform/architecture, not by pattern — the arm64 assets do match the
//!   naming grammar, this app just cannot use them.
//! - **`sha256` is always `None` today.** llama.cpp releases publish no
//!   per-asset checksums (no `sha`-named assets, no `sha256` in release
//!   bodies — verified across the release set fetched 2026-08-31). The
//!   field stays `Option` for T-022's verification path; there is nothing
//!   to populate it from yet.
//! - **Rate limiting.** GitHub's rate-limit responses carry
//!   `X-RateLimit-Reset` (a UNIX timestamp) and/or `Retry-After`
//!   (seconds). `AppError::RateLimited` carries a *duration*
//!   (`retry_after_seconds`), and `docs/TASKS.md` T-020's acceptance text
//!   says the error should carry "the reset time" — the two do not agree,
//!   recorded as D-012 in `PROGRESS.md`. The duration is computed from
//!   whatever the response actually provides: `Retry-After` when present,
//!   otherwise `X-RateLimit-Reset` minus now, otherwise a 60 s floor.
//! - **Malformed JSON is a typed `AppError::Network`, never a panic.** The
//!   `AppError` enum is a T-002/T-004 contract with no generic "bad
//!   payload" variant; `Network` is the closest fit (upstream problem, not
//!   app bug) and its remediation ("check your connection and retry") is
//!   the right action either way. A deserialization failure of a
//!   successful response is reported through it with a message that names
//!   the malformed payload.
//!
//! The parser is `PLAN.md` §2.13's open-enum case: a CUDA major the code
//! has no branch for parses into `Backend::Cuda { major }` rather than
//! failing — the enum is open on purpose.

use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::core::types::{AppError, AvailableRelease, Backend};

/// The GitHub API the client talks to. Tests point at a local stub by
/// passing a different base URL to [`check_for_updates`]; nothing here is
/// hardcoded to one or the other.
pub const GITHUB_API_BASE: &str = "https://api.github.com";

/// One page of the releases list, newest first. 100 releases covers many
/// weeks of llama.cpp's publishing cadence — plenty for "newest per
/// backend" (see the module doc). Only used inside this module; the public
/// surface is [`check_for_updates`].
const RELEASES_ENDPOINT: &str = "/repos/ggml-org/llama.cpp/releases?per_page=100";

/// Used only when a rate-limit response carries neither `Retry-After` nor
/// `X-RateLimit-Reset`. 60 s is a floor, never worse than the documented
/// unauthenticated primary-limit window (60 requests/hour).
const RATE_LIMIT_FALLBACK_SECONDS: u64 = 60;

/// Client for production use: points at the real API and bounds the
/// request so a hung connection cannot pin the IPC command.
///
/// reqwest 0.13 has no `ClientBuilder::base_url` (removed in 0.12), so the
/// base URL is a parameter of [`check_for_updates`], not of the client.
/// The relative [`RELEASES_ENDPOINT`] is joined onto it by that function.
pub fn default_client() -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent("llama-manager")
        .timeout(std::time::Duration::from_secs(30))
        .build()
}

/// Fetch the releases list and return the newest release per backend.
///
/// `client` and `base_url` are parameters rather than globals so tests can
/// point the client at a local stub: `check_for_updates(&client,
/// "http://127.0.0.1:N")`. The production base is always
/// [`GITHUB_API_BASE`].
pub async fn check_for_updates(
    client: &reqwest::Client,
    base_url: &str,
) -> Result<Vec<AvailableRelease>, AppError> {
    let url = format!("{base_url}{RELEASES_ENDPOINT}");
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|err| AppError::Network {
            message: format!("could not reach the GitHub releases endpoint (offline?): {err}"),
        })?;

    let status = response.status();
    if status.as_u16() == 429 || (status.as_u16() == 403 && is_rate_limited_response(&response)) {
        let retry_after_seconds = rate_limit_retry_after(&response);
        return Err(AppError::RateLimited {
            retry_after_seconds,
        });
    }

    if !status.is_success() {
        let body = response.text().await.map_err(|err| AppError::Network {
            message: format!("GitHub API error {status}: {err}"),
        })?;
        return Err(AppError::Network {
            message: format!("GitHub API error {status}: {body}"),
        });
    }

    let releases: Vec<ReleaseJson> = response.json().await.map_err(|err| AppError::Network {
        message: format!("malformed JSON from the releases endpoint: {err}"),
    })?;

    Ok(newest_per_backend(releases))
}

/// A 403 is only treated as a rate limit when it carries
/// `X-RateLimit-Remaining: 0` — a 403 without that header is an ordinary
/// API error (e.g. a bad credential) and stays `Network`. A 429 is always
/// a rate limit.
fn is_rate_limited_response(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|value| value.to_str().ok())
        == Some("0")
}

/// Compute the `RateLimited` payload from whatever the response actually
/// provides. `Retry-After` (seconds, absolute duration) is preferred;
/// `X-RateLimit-Reset` (UNIX timestamp) is converted to a duration; neither
/// present, the floor.
fn rate_limit_retry_after(response: &reqwest::Response) -> u64 {
    let headers = response.headers();

    if let Some(value) = headers
        .get(reqwest::header::HeaderName::from_static("retry-after"))
        .and_then(|value| value.to_str().ok())
    {
        if let Ok(seconds) = value.trim().parse::<u64>() {
            return seconds;
        }
    }

    if let Some(value) = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
    {
        if let Ok(reset_unix) = value.trim().parse::<i64>() {
            let delta = reset_unix.saturating_sub(Utc::now().timestamp()).max(1);
            return delta as u64;
        }
    }

    RATE_LIMIT_FALLBACK_SECONDS
}

/// Group by backend, keeping only the asset from the newest release that
/// has one. Releases are sorted newest-`published_at` first, so "first
/// seen wins" is exactly "newest wins" regardless of the order the API
/// happened to return them in. Within a single release, if two assets ever
/// parse to the same backend (e.g. `cuda-13.3` and `cuda-13.4`, both
/// major 13) the API's own asset order breaks the tie — deterministic for
/// a given response body.
///
/// Module-private: the public surface is [`check_for_updates`], which owns
/// the fetch and calls this.
fn newest_per_backend(releases: Vec<ReleaseJson>) -> Vec<AvailableRelease> {
    let mut releases = releases;
    releases.sort_by(|a, b| b.published_at.cmp(&a.published_at));

    let mut seen: Vec<BackendKey> = Vec::new();
    let mut newest: Vec<AvailableRelease> = Vec::new();

    for release in &releases {
        for asset in &release.assets {
            let Some((build_tag, backend)) = parse_asset_name(&asset.name) else {
                continue;
            };
            let key = backend_key(&backend);
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            newest.push(AvailableRelease {
                build_tag,
                backend,
                asset_name: asset.name.clone(),
                asset_url: asset.browser_download_url.clone(),
                // llama.cpp releases publish no per-asset checksums (see
                // the module doc); the field stays for T-022's path.
                sha256: None,
                size_bytes: asset.size,
                published_at: release.published_at,
                release_notes_url: release.html_url.clone(),
            });
        }
    }

    newest
}

/// A `Backend` the app knows how to compare. `Backend` derives `PartialEq`
/// but not `Eq`/`Hash` (T-002's contract shape, not touched here), so the
/// grouping key is a local value type rather than a new derive on the
/// contract type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BackendKey {
    Cuda(u8),
    Vulkan,
    Cpu,
}

fn backend_key(backend: &Backend) -> BackendKey {
    match backend {
        Backend::Cuda { major } => BackendKey::Cuda(*major),
        Backend::Vulkan => BackendKey::Vulkan,
        Backend::Cpu => BackendKey::Cpu,
    }
}

/// Parse a llama.cpp release asset name into `(build_tag, Backend)`.
///
/// The real-name corpus (pulled from `github.com/ggml-org/llama.cpp/releases`,
/// 2026-08-31) includes `llama-b10726-bin-win-cuda-13.3-x64.zip`,
/// `llama-b10726-bin-win-cpu-x64.zip`, `llama-b10726-bin-win-vulkan-x64.zip`,
/// `llama-b10726-bin-ubuntu-rocm-7.14-x64.tar.gz`,
/// `cudart-llama-bin-win-cuda-12.4-x64.zip`, `llama-b10726-ui.tar.gz` and
/// `llama-b10726-xcframework.zip`.
///
/// Accepted shape: `llama-b<digits>-bin-win-<variant>-x64.<ext>` where
/// `<variant>` is `cpu`, `vulkan` or `cuda-<major>.<minor>`. A CUDA major
/// the code has no branch for parses into `Backend::Cuda { major }` — the
/// enum is open (`PLAN.md` §2.13); the parser fails on a major outside
/// `u8`, never on an unfamiliar one.
///
/// Everything else is `None`, never a panic: other platforms (`ubuntu`,
/// `macos`, `android`, …), other backends (`rocm`, `sycl`, `opencl`,
/// `openvino`, …), non-`x64` architectures (the app is `x86_64`-only), the
/// `cudart-` prefix, and non-binary assets (`-ui`, `-xcframework`).
pub fn parse_asset_name(name: &str) -> Option<(String, Backend)> {
    let stem = name
        .strip_suffix(".tar.gz")
        .or_else(|| name.strip_suffix(".zip"))
        .unwrap_or(name);

    let rest = stem.strip_prefix("llama-b")?;
    let (digits, rest) = rest.split_once('-')?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let build_tag = format!("b{digits}");

    let rest = rest.strip_prefix("bin-")?;
    let (platform, rest) = rest.split_once('-')?;
    if platform != "win" {
        return None;
    }
    let (variant, arch) = rest.rsplit_once('-')?;
    if arch != "x64" {
        return None;
    }

    match variant {
        "cpu" => Some((build_tag, Backend::Cpu)),
        "vulkan" => Some((build_tag, Backend::Vulkan)),
        _ => variant
            .strip_prefix("cuda-")
            .and_then(|version| version.split_once('.'))
            .and_then(|(major, _minor)| major.parse::<u8>().ok())
            .map(|major| (build_tag, Backend::Cuda { major })),
    }
}

/// One page of the GitHub releases API as this client consumes it. Only the
/// fields the client needs are declared, and unknown fields are ignored on
/// purpose — the API is free to add fields, and a client that rejects
/// unknown fields is one that breaks on the next API change.
#[derive(Clone, Debug, Deserialize)]
struct ReleaseJson {
    /// ISO 8601, e.g. `2026-08-31T19:41:30Z`.
    published_at: DateTime<Utc>,
    /// The release page — becomes `AvailableRelease.release_notes_url`.
    html_url: String,
    assets: Vec<AssetJson>,
}

#[derive(Clone, Debug, Deserialize)]
struct AssetJson {
    name: String,
    size: u64,
    browser_download_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    // ── Asset-name parser ──────────────────────────────────────────

    /// Table-driven over real asset names pulled from
    /// `github.com/ggml-org/llama.cpp/releases` (the b10726 release page,
    /// fetched 2026-08-31), plus one hypothetical name that demonstrates
    /// the `PLAN.md` §2.13 open-enum case.
    #[test]
    fn parse_asset_name_table_driven() {
        let rows: &[(&str, Option<(&str, Backend)>)] = &[
            // Accepted — Windows x64, one per backend kind.
            (
                "llama-b10726-bin-win-cpu-x64.zip",
                Some(("b10726", Backend::Cpu)),
            ),
            (
                "llama-b10726-bin-win-cuda-12.4-x64.zip",
                Some(("b10726", Backend::Cuda { major: 12 })),
            ),
            (
                "llama-b10726-bin-win-cuda-13.3-x64.zip",
                Some(("b10726", Backend::Cuda { major: 13 })),
            ),
            (
                "llama-b10726-bin-win-vulkan-x64.zip",
                Some(("b10726", Backend::Vulkan)),
            ),
            // Open-enum case (PLAN.md §2.13): a CUDA major the code has no
            // branch for parses into Backend::Cuda { major } rather than
            // failing.
            (
                "llama-b10726-bin-win-cuda-99.9-x64.zip",
                Some(("b10726", Backend::Cuda { major: 99 })),
            ),
            // Rejected — real names that match no known pattern.
            ("llama-b10726-bin-win-rocm-7.14-x64.zip", None),
            ("llama-b10726-bin-win-sycl-x64.zip", None),
            ("llama-b10726-bin-win-opencl-adreno-arm64.zip", None),
            ("llama-b10726-bin-win-openvino-2026.3.1-x64.zip", None),
            ("cudart-llama-bin-win-cuda-12.4-x64.zip", None),
            ("llama-b10726-ui.tar.gz", None),
            ("llama-b10726-xcframework.zip", None),
            // Rejected — the right pattern, the wrong platform or
            // architecture for this app.
            ("llama-b10726-bin-win-cpu-arm64.zip", None),
            ("llama-b10726-bin-win-cuda-13.4-arm64.zip", None),
            ("llama-b10726-bin-ubuntu-x64.tar.gz", None),
            ("llama-b10726-bin-ubuntu-vulkan-x64.tar.gz", None),
            ("llama-b10726-bin-macos-arm64.tar.gz", None),
            ("llama-b10726-bin-android-arm64.tar.gz", None),
        ];

        assert!(
            rows.len() >= 12,
            "T-020 acceptance: at least 12 real asset names"
        );
        for (name, expected) in rows {
            let got = parse_asset_name(name);
            let matches = match (got.clone(), expected) {
                (Some((tag, backend)), Some((want_tag, want_backend))) => {
                    tag == *want_tag && backend == *want_backend
                }
                (None, None) => true,
                _ => false,
            };
            assert!(
                matches,
                "asset name: {name}: got {got:?}, expected {expected:?}"
            );
        }
    }

    // ── Newest-per-backend selection ───────────────────────────────

    fn asset(name: &str) -> AssetJson {
        AssetJson {
            name: name.to_string(),
            size: 1234,
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    #[test]
    fn newest_per_backend_picks_newest_release_and_drops_rejected() {
        let older = ReleaseJson {
            published_at: "2026-08-30T10:00:00Z".parse().unwrap(),
            html_url: "https://github.com/ggml-org/llama.cpp/releases/tag/b10725".to_string(),
            assets: vec![
                // Shadowed by the newer release's cuda-13.3 asset.
                asset("llama-b10725-bin-win-cuda-13.3-x64.zip"),
                asset("llama-b10725-bin-win-vulkan-x64.zip"),
                // Rejected: non-Windows platform.
                asset("llama-b10725-bin-ubuntu-x64.tar.gz"),
            ],
        };
        let newer = ReleaseJson {
            published_at: "2026-08-31T19:00:00Z".parse().unwrap(),
            html_url: "https://github.com/ggml-org/llama.cpp/releases/tag/b10726".to_string(),
            assets: vec![
                asset("llama-b10726-bin-win-cuda-13.3-x64.zip"),
                asset("llama-b10726-bin-win-cpu-x64.zip"),
                // Rejected: no known backend pattern.
                asset("llama-b10726-bin-win-rocm-7.14-x64.zip"),
            ],
        };

        // Deliberately older-first: the function must sort, not trust the
        // input order.
        let result = newest_per_backend(vec![older, newer]);

        let tags: Vec<(&Backend, &str)> = result
            .iter()
            .map(|release| (&release.backend, release.build_tag.as_str()))
            .collect();
        assert!(
            tags.contains(&(&Backend::Cuda { major: 13 }, "b10726")),
            "cuda-13 must come from the newer release, got {tags:?}"
        );
        assert!(
            tags.contains(&(&Backend::Cpu, "b10726")),
            "cpu must come from the newer release, got {tags:?}"
        );
        assert!(
            tags.contains(&(&Backend::Vulkan, "b10725")),
            "vulkan only exists in the older release, got {tags:?}"
        );
        assert_eq!(result.len(), 3, "rocm/ubuntu must be dropped, got {tags:?}");

        let cuda = result
            .iter()
            .find(|release| release.backend == Backend::Cuda { major: 13 })
            .expect("cuda-13 entry");
        assert_eq!(
            cuda.release_notes_url,
            "https://github.com/ggml-org/llama.cpp/releases/tag/b10726"
        );
        assert_eq!(
            cuda.asset_url,
            "https://example.invalid/llama-b10726-bin-win-cuda-13.3-x64.zip"
        );
        assert_eq!(cuda.sha256, None);
    }

    // ── HTTP behaviour against a local stub ────────────────────────

    /// A single canned HTTP/1.1 response, served once per connection.
    struct Canned {
        status: u16,
        headers: Vec<(String, String)>,
        body: String,
    }

    /// Serve `canned` responses in order (looping the last one), one per
    /// connection, on an ephemeral loopback port. The listener lives for
    /// the process lifetime; tests own the port and never need to shut it
    /// down.
    fn start_stub(canned: Vec<Canned>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            let mut index = 0usize;
            for socket in listener.incoming() {
                let Ok(mut socket) = socket else {
                    continue;
                };
                // Read the request headers (there is no request body).
                let mut buf: Vec<u8> = Vec::new();
                let mut tmp = [0u8; 512];
                while !buf.ends_with(b"\r\n\r\n") {
                    let Ok(n) = socket.read(&mut tmp) else {
                        break;
                    };
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&tmp[..n]);
                    if buf.len() > 65536 {
                        break;
                    }
                }
                let response = &canned[index.min(canned.len() - 1)];
                index += 1;
                let status_text = match response.status {
                    200 => "OK",
                    403 => "Forbidden",
                    429 => "Too Many Requests",
                    _ => "Unknown",
                };
                let mut reply = format!("HTTP/1.1 {} {}\r\n", response.status, status_text);
                for (key, value) in &response.headers {
                    reply.push_str(&format!("{key}: {value}\r\n"));
                }
                reply.push_str(&format!(
                    "Content-Length: {}\r\nConnection: close\r\n\r\n",
                    response.body.len()
                ));
                reply.push_str(&response.body);
                let _ = socket.write_all(reply.as_bytes());
            }
        });
        port
    }

    /// A plain client; the base URL is supplied per call.
    fn client() -> reqwest::Client {
        reqwest::Client::builder().build().unwrap()
    }

    /// Point a client at a local stub for the given port.
    fn base_for(port: u16) -> String {
        format!("http://127.0.0.1:{port}")
    }

    /// Offline: a client pointed at a closed loopback port.
    #[tokio::test]
    async fn offline_returns_typed_network_error() {
        let client = client();
        let err = check_for_updates(&client, "http://127.0.0.1:9")
            .await
            .expect_err("closed port must be an error");
        match err {
            AppError::Network { .. } => {}
            other => panic!("expected Network for an unreachable endpoint, got {other:?}"),
        }
    }

    /// GitHub's documented shape: a rate-limit response carries
    /// `X-RateLimit-Reset` (UNIX timestamp). The error must carry the
    /// derived duration.
    #[tokio::test]
    async fn rate_limit_response_returns_rate_limited_with_reset_duration() {
        let reset = (Utc::now().timestamp() + 90).to_string();
        let port = start_stub(vec![Canned {
            status: 403,
            headers: vec![
                ("x-ratelimit-remaining".to_string(), "0".to_string()),
                ("x-ratelimit-reset".to_string(), reset),
            ],
            body: r#"{"message":"You have exceeded a rate limit"}"#.to_string(),
        }]);

        let err = check_for_updates(&client(), &base_for(port))
            .await
            .expect_err("403 must be an error");
        match err {
            AppError::RateLimited {
                retry_after_seconds,
            } => assert!(
                (80..=100).contains(&retry_after_seconds),
                "expected ~90 s derived from X-RateLimit-Reset, got {retry_after_seconds}"
            ),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// `Retry-After` (seconds, absolute) is preferred over `X-RateLimit-Reset`
    /// when both are present.
    #[tokio::test]
    async fn retry_after_header_is_preferred_over_reset_timestamp() {
        let reset = (Utc::now().timestamp() + 999).to_string();
        let port = start_stub(vec![Canned {
            status: 429,
            headers: vec![
                ("retry-after".to_string(), "123".to_string()),
                ("x-ratelimit-reset".to_string(), reset),
            ],
            body: "{}".to_string(),
        }]);

        let err = check_for_updates(&client(), &base_for(port))
            .await
            .expect_err("429 must be an error");
        match err {
            AppError::RateLimited {
                retry_after_seconds,
            } => assert_eq!(retry_after_seconds, 123),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// Neither header present: the floor, not a guess-shaped panic.
    #[tokio::test]
    async fn rate_limit_without_headers_uses_fallback_duration() {
        let port = start_stub(vec![Canned {
            status: 429,
            headers: Vec::new(),
            body: "{}".to_string(),
        }]);

        let err = check_for_updates(&client(), &base_for(port))
            .await
            .expect_err("429 must be an error");
        match err {
            AppError::RateLimited {
                retry_after_seconds,
            } => assert_eq!(retry_after_seconds, RATE_LIMIT_FALLBACK_SECONDS),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }

    /// A 403 that is *not* a rate limit (no `X-RateLimit-Remaining: 0`)
    /// stays a `Network` error — the discriminator is what keeps an auth
    /// failure from masquerading as "retry later".
    #[tokio::test]
    async fn non_rate_limited_403_stays_network() {
        let port = start_stub(vec![Canned {
            status: 403,
            headers: vec![("x-ratelimit-remaining".to_string(), "52".to_string())],
            body: r#"{"message":"Bad credentials"}"#.to_string(),
        }]);

        let err = check_for_updates(&client(), &base_for(port))
            .await
            .expect_err("403 must be an error");
        match err {
            AppError::Network { message } => assert!(
                message.contains("403"),
                "the message should name the status: {message}"
            ),
            other => panic!("expected Network for a non-rate-limit 403, got {other:?}"),
        }
    }

    /// Malformed JSON on a 200 response: a typed error, never a panic.
    #[tokio::test]
    async fn malformed_json_returns_typed_error() {
        let port = start_stub(vec![Canned {
            status: 200,
            headers: Vec::new(),
            body: r#"{"releases": oops"#.to_string(),
        }]);

        let err = check_for_updates(&client(), &base_for(port))
            .await
            .expect_err("garbage must be an error");
        match err {
            AppError::Network { message } => assert!(
                message.contains("malformed JSON"),
                "the message should name the failure: {message}"
            ),
            other => panic!("expected a Network error naming the malformed payload, got {other:?}"),
        }
    }

    /// A server error stays a typed error carrying the status.
    #[tokio::test]
    async fn server_error_stays_typed() {
        let port = start_stub(vec![Canned {
            status: 500,
            headers: Vec::new(),
            body: r#"{"message":"internal"}"#.to_string(),
        }]);

        let err = check_for_updates(&client(), &base_for(port))
            .await
            .expect_err("500 must be an error");
        match err {
            AppError::Network { message } => assert!(
                message.contains("500"),
                "the message should name the status: {message}"
            ),
            other => panic!("expected Network for a 500, got {other:?}"),
        }
    }

    /// End to end against a stub serving a realistic two-release payload
    /// (field shapes, sizes and timestamps as the live API returned them
    /// on 2026-08-31): newest-per-backend wins, rejected names are
    /// dropped, URLs pass through, checksums stay absent.
    #[tokio::test]
    async fn happy_path_selects_newest_per_backend() {
        let body = r#"[
            {
                "tag_name": "b10724",
                "published_at": "2026-08-31T17:26:21Z",
                "html_url": "https://github.com/ggml-org/llama.cpp/releases/tag/b10724",
                "assets": [
                    {"id": 1, "name": "llama-b10724-bin-win-cuda-13.3-x64.zip",
                     "size": 147194606,
                     "browser_download_url": "https://github.com/ggml-org/llama.cpp/releases/download/b10724/llama-b10724-bin-win-cuda-13.3-x64.zip"},
                    {"id": 2, "name": "llama-b10724-bin-win-vulkan-x64.zip",
                     "size": 34937666,
                     "browser_download_url": "https://github.com/ggml-org/llama.cpp/releases/download/b10724/llama-b10724-bin-win-vulkan-x64.zip"}
                ]
            },
            {
                "tag_name": "b10726",
                "published_at": "2026-08-31T19:41:30Z",
                "html_url": "https://github.com/ggml-org/llama.cpp/releases/tag/b10726",
                "assets": [
                    {"id": 3, "name": "llama-b10726-bin-win-cuda-13.3-x64.zip",
                     "size": 147435371,
                     "browser_download_url": "https://github.com/ggml-org/llama.cpp/releases/download/b10726/llama-b10726-bin-win-cuda-13.3-x64.zip"},
                    {"id": 4, "name": "llama-b10726-bin-win-rocm-7.14-x64.zip",
                     "size": 244241090,
                     "browser_download_url": "https://github.com/ggml-org/llama.cpp/releases/download/b10726/llama-b10726-bin-win-rocm-7.14-x64.zip"}
                ]
            }
        ]"#
        .to_string();

        let port = start_stub(vec![Canned {
            status: 200,
            headers: Vec::new(),
            body,
        }]);

        let releases = check_for_updates(&client(), &base_for(port))
            .await
            .expect("a well-formed 200 must succeed");

        let cuda = releases
            .iter()
            .find(|release| release.backend == Backend::Cuda { major: 13 })
            .expect("cuda-13 entry");
        assert_eq!(cuda.build_tag, "b10726", "the newer release must win");
        assert_eq!(
            cuda.asset_url,
            "https://github.com/ggml-org/llama.cpp/releases/download/b10726/llama-b10726-bin-win-cuda-13.3-x64.zip"
        );
        assert_eq!(
            cuda.release_notes_url,
            "https://github.com/ggml-org/llama.cpp/releases/tag/b10726"
        );
        assert_eq!(cuda.size_bytes, 147435371);
        assert_eq!(cuda.sha256, None);

        let vulkan = releases
            .iter()
            .find(|release| release.backend == Backend::Vulkan)
            .expect("vulkan entry");
        assert_eq!(vulkan.build_tag, "b10724");
        assert_eq!(vulkan.size_bytes, 34937666);

        assert_eq!(
            releases.len(),
            2,
            "cuda-13 and vulkan only; the rocm asset must be dropped"
        );
    }
}
