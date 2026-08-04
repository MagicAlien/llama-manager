# ADR-002 — The app owns the client-facing endpoint

**Status:** Accepted · 1 August 2026
**Supersedes:** the direct-binding arrangement assumed up to plan v5.0
**Affects:** `AGENTS.md` invariant 3, `PLAN.md` §2.7 and §2.12, `ServerConfig`, `ServerState`, T-042, T-044, T-050, T-051, T-052, T-047

---

## Context

Up to v5.0, `llama-server` bound `127.0.0.1:8080` itself and external clients connected to it directly. The app generated its configuration, spawned it, and never saw a request.

That is the simpler arrangement and it has one defect, which happens to sit on the product's main promise: **the address does not stay put.**

A user configures an agent, an IDE plugin, a script, a `.env`. Then they update the llama.cpp build, or the server crashes and restarts, or they change a router setting and restart. Each of those is a window during which the configured address refuses connections. The client reports `ECONNREFUSED` as its own failure, and the user is left correlating two applications' states by hand.

Three further things were unreachable under direct binding, and only became visible once the first problem was taken seriously:

- Authentication was llama-server's, so the API key had to be handed to it and written where it could be read.
- Exposure was llama-server's: binding to the LAN was its setting, subject to its confirmation, which is to say none.
- Request counts had no source at all. `TelemetrySnapshot.active_requests` was in the contracts with nothing behind it.

## Decision

**The app binds the client-facing socket. `llama-server` binds an ephemeral port on `127.0.0.1`, chosen by the app at spawn time and invisible to the user. The app forwards every request to that single upstream, unchanged.**

The listener binds when the app starts and stays bound, **independently of whether llama-server is running**. A request arriving when there is no server receives a structured `503` naming the state, not a refused connection.

## What this explicitly does not authorise

This is the part that matters, because a reverse proxy and a model router differ by about four commits.

The app **must not**: read a request body, choose which model serves a request, run more than one server process, allocate a port per model, implement eviction or per-model queuing, buffer a streaming response, or alter status codes and headers except to reject an unauthenticated request.

`AGENTS.md` invariant 3 states this as a rule and the PR template checks it. The distinction is: **owning a socket and forwarding to one fixed address is transport; choosing where to forward based on content, and managing what sits behind each destination, is `llama-swap`.** `PLAN.md` §2.3 explains at length why rebuilding `llama-swap` is the one thing that would sink this project.

## Consequences

**Gained.**

- The configured address survives restarts, crashes and build switches. This is now a Definition of Done item and is verified end to end in T-065.
- The API key is checked at the app's edge, in constant time, and never reaches llama-server or `presets.ini`.
- `llama-server` is permanently on loopback and cannot be exposed to the LAN even by mistake. Exposure is the app's decision, with the app's confirmation dialog.
- The request log exists, and with it the telemetry that had no source.
- Swapping the orchestrator — the `llama-swap` contingency — no longer changes the address clients use. It becomes an internal substitution.

**Paid.**

- **A proxy that can break streaming.** SSE must pass through unbuffered; any accumulation turns token-by-token output into one delayed blob, and a test comparing concatenated bodies will not notice. T-042 asserts chunk boundaries and inter-chunk timing instead.
- **Long-held connections.** A first request against an unloaded 65 GB model holds for minutes. Timeouts on both sides are configured deliberately rather than inherited.
- **Header and status fidelity** must survive untouched, or clients switching on them misbehave in ways that look like server bugs.
- **One more hop to debug.** "Is it the app or the server?" is now a question. The request log exists partly to answer it.
- **Three dependencies**: `axum`, `futures-util`, `subtle`.
- **An Electron fallback (`PLAN.md` §2.5) got more expensive**: the listener would move too.

**Structural.**

- `ServerConfig` separates `listen_port` (the contract with clients, never auto-incremented) from `upstream_port_range` (internal, auto-incremented on collision — the one place that is correct).
- `EndpointState` is separate from `ServerState` and does not transition with it. That separation is the decision, expressed in types.
- The endpoint forwards during `Starting { Preloading }` (§2.12). `Running` is a statement about configuration being fully applied; availability does not wait for it. **The app is never less available than the server it manages.**

## Alternatives considered

**Keep direct binding, take the counters from a metrics endpoint.** Would have recovered telemetry, if such an endpoint exists on the pinned build. Recovers nothing else — not address stability, not authentication, not exposure control. The counters were the symptom, not the problem.

**Optional gateway on the same port, off by default (the v5.0 plan).** The worst option: enabling or disabling it moves the address under clients that are already configured. If a pass-through layer is optional, it must be on a different port, and then the address is unstable by construction.

**Full router in the app.** Rejected in `PLAN.md` §2.3, unchanged by this decision, and now guarded by an invariant instead of an absence.

## Revisiting

Reconsider only if the streaming pass-through proves unfixable on this stack — which would be a surprise, since one fixed upstream is the simplest case a proxy can have. Reverting is mechanical: bind llama-server to `listen_port`, drop `core/endpoint/`, and accept back every problem listed under Context.
