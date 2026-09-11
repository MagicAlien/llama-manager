# llama.cpp Manager — Project Plan

**Windows desktop app for managing a llama.cpp server. No in-app inference — control plane, endpoint ownership, and API exposure only.**

Version 6.3 — 2 August 2026

| Document | Contents | Read when |
|---|---|---|
| `AGENTS.md` | Binding rules | Always — loaded every session |
| `PROGRESS.md` | Live state: done, in flight, blocked, discrepancies, facts learned | Every session, first |
| `docs/WORKFLOW.md` | Session protocol: task selection, closing, what to do when stuck | Once, then as needed |
| `PLAN.md` | This file: architecture, decisions, milestones, risks | Orientation, planning |
| `docs/DEV-SETUP.md` | Toolchain, prerequisites, first run, fixtures | Once, before T-000 |
| `docs/CONTRACTS.md` | Types, state machine, schema, IPC surface, provider traits | Implementing anything |
| `docs/TASKS.md` | Task list with acceptance criteria | Working a task |
| `docs/LLAMACPP.md` | llama.cpp flags and endpoints | Touching the server |
| `docs/owner-verification.md` | Empirical checks on real hardware | Owner only — not the agent |
| `docs/verified-flags.md` | Generated flag list per build | Before using any flag |

---

## 1. What this app does

Manages the lifecycle of a llama.cpp server on Windows: detects and installs the binary, tracks versions and updates, maintains a catalogue of GGUF models with per-model settings, generates the server's configuration, supervises the process, **owns the HTTP endpoint that external clients connect to**, and forwards their traffic to the server.

It contains no chat interface and performs no inference of its own.

```
┌───────────────────────────────────────────────────────────────────┐
│  WINDOWS (single machine, no VM, no container)                    │
│                                                                   │
│  ┌────────────────────────┐      ┌─────────────────────────────┐  │
│  │  UI (React/TS)         │◄────►│  Core (Rust/Tauri)          │  │
│  │  renderer process      │ IPC  │  supervisor + services      │  │
│  └────────────────────────┘      │                             │  │
│                                  │  ┌───────────────────────┐  │  │
│                                  │  │ endpoint listener     │◄─┼──┼── external agents
│                                  │  │ 127.0.0.1:8080        │  │  │   (stable address)
│                                  │  │ auth + request log    │  │  │
│                                  │  └──────────┬────────────┘  │  │
│                                  └─────────────┼───────────────┘  │
│         writes presets.ini                     │ transparent      │
│         spawns + monitors      ┌───────────────▼───────────────┐  │
│         polls /models          │  llama-server (router mode)   │  │
│                                │  127.0.0.1:<ephemeral>        │  │
│                                └──────────────┬────────────────┘  │
│                                               │ child processes   │
│                         ┌─────────────────────┼──────────────┐    │
│                         │  model A   │  model B   │   ...     │    │
│                         └─────────────────────┬──────────────┘    │
│                                               ▼                   │
│                             NVIDIA Blackwell GPU (CUDA 13) + RAM  │
└───────────────────────────────────────────────────────────────────┘
```

**Two central design facts:**

1. llama-server's router mode already implements just-in-time model loading and switching. The app does not implement a scheduler. Its job is to manage the binary, maintain the catalogue, generate `presets.ini`, supervise one process, and present state. §2.3 explains what that decision buys.
2. The app owns the client-facing socket and forwards to the server (§2.7). This is a *transport* concern only: the app never reads a request body and never decides which model answers. The distinction between owning the socket and scheduling models is the line `AGENTS.md` invariant 3 draws.

---

## 2. Decisions taken

These were open questions in earlier versions. They are now settled, and the tasks assume them.

**Numbering has gaps.** §2.8 and §2.11 covered decisions removed in the v6.0 review. The remaining sections keep their numbers so that references elsewhere stay valid; the gaps are deliberate and nothing is missing.

### 2.1 Models stay where the user keeps them, addressed by absolute path

**Decision: model files are never moved. The app addresses each model by its absolute path, on whatever volume it lives.**

A user with hundreds of gigabytes of models spread across several drives adds them by pointing at them. No managed folder, no relocation prompt, no models root in Settings, no copying.

`llama-server` takes an absolute path in `-m` — that is its most basic usage and is not in question. The narrow thing to establish is how a model enters the registry **in router mode**, which by definition starts without `-m`:

- **Certain channel:** the directory scan driven by `--models-dir`.
- **To establish:** whether an entry in the `--models-preset` file can introduce a model by path, or only carry settings for a model the scan already found.

**Preliminary evidence, recorded in `PROGRESS.md` F-000, points at the preset being able to declare paths.** Upstream documentation describes the router discovering models from the scan *and* from the preset, and an upstream discussion advises keeping models outside the scanned directory and naming them by absolute path in the preset. That is not confirmation — it is documentation and a forum answer, not the `--help` of the pinned build — but it is enough to say that the scan-only branch below is now the unlikely case rather than the coin flip it was written as. Nothing about the design changes; the order of expectation does.

**This is answered by running the pinned build against prepared fixtures** — no GPU, no model files, no target machine. T-023 captures and parses `--help` during Milestone B; T-025 then runs the same CPU build in router mode against a prepared directory structure and observes actual behaviour. Observed behaviour beats an inference from help text, even when the two agree.

Four questions sit together and are answered in one sitting with a real binary:

1. Does a `--models-preset` entry register a model by absolute path, or only carry settings for one the scan already found?
2. Does the `--models-dir` scan follow reparse points — symlinks and junctions — at all? If it does not, the scan-only fallback does not exist in any form.
3. What is the scan's real depth, and what happens to a subdirectory that is not a model?
4. Under `PresetDeclaresPath`, how is a multimodal model's projector expressed — is the `mmproj` filename convention honoured in a preset entry, or is a key required? T-033 must emit a vision model on both channels and nothing yet says how the preferred one does it.

Only the first is answerable from `--help`, and T-023 answers it there. **The other three are answered by T-025**, which runs the same CPU build against a prepared directory — as is §2.12's separate question of which health endpoint answers before any model is loaded, since it is the same sitting with the same binary. None of the four needs a GPU or a real model file, so none of them is the owner's — see `AGENTS.md` §3. What remains the owner's is what T-025 cannot do on its own hardware, and T-025 reports exactly that rather than assuming.

**Both outcomes are supported, and the decision point is explicit:**

- **Preset can declare paths.** The app declares each registered model directly. `--models-dir` is never emitted. This is the preferred outcome and the one the tasks are written against.
- **Registration is scan-only.** The app creates a directory inside its own data directory, links the user's models into it, and points `--models-dir` there.

  **That directory must be flat, and subdirectories are not free to use.** Under `--models-dir` a subdirectory means "one model made of several files" — multimodal or multi-shard — not "a group of models". The scanner treats every subdirectory as a candidate model and offers no way to exclude one, so a folder created for tidiness becomes a phantom entry. Scan depth is one level. The app therefore lays out its link directory to match that grammar exactly: a link per single-file model at the top, one subdirectory per multi-file model, and nothing else. The user's disk layout is untouched; the links live in `%LOCALAPPDATA%\LlamaManager\`, are created and destroyed by the app, and are never placed anywhere the user chose.

  **The linking primitives are not interchangeable on Windows, and an earlier version of this document was wrong about it.** Three exist and each fails somewhere the others do not:

  | Primitive | Privileges | Works for |
  |---|---|---|
  | Directory junction | none | directories only — cannot link an individual `.gguf` |
  | File symlink | Administrator, or Developer Mode enabled | any file, any volume |
  | Hard link | none | files on the **same volume** only |

  Models live on several drives, which rules out hard links as a general answer; they are single files, which rules out junctions. So the only primitive that always does what is needed is the one that needs a privilege an ordinary user does not have.

  The app therefore probes its own capability at startup — by attempting a symlink in a scratch directory, not by reading a registry key — and reports what it can do:

  - **Symlinks available:** one per model, exact and per-file. The preferred fallback.
  - **Symlinks unavailable, model on the same volume as the app data directory:** hard link, equally exact.
  - **Neither:** the app cannot register that model under scan-only, and says so plainly, with an explanation that enabling Developer Mode resolves it. It does not silently link a whole folder and hope.

T-025 **must record which outcome holds** in the Facts established section of `PROGRESS.md` before T-033 begins. T-033 branches on it; it does not guess.

**If the outcome is `Undetermined`, work stops rather than falling back.** Defaulting to scan-only would look like the safe choice and is not: it carries a privilege dependency and an unverified assumption about reparse points, either of which can fail on a user's machine in a way no test here would catch. The right response is the empirical check, which is minutes with a real binary, not a guess that propagates into Milestone D.

Consequences worth naming:

- **Every registered model is exposed.** There is no per-model enable switch — see §2.9 for why it was dropped and what replaced it.
- **Preload and pinned are independent runtime properties.** Preload means loaded into VRAM at server start rather than on first request; pinned means exempt from LRU eviction. Neither affects registration.
- **Registration is not loading.** A registered model costs no VRAM until a request names it, so a large catalogue is cheap.
- **The app never copies, moves, or deletes a model file.** A link is not a copy and points outward only. If a path stops resolving — drive disconnected, file renamed elsewhere — that model is marked `Missing` and the rest of the catalogue keeps working.
- **Paths must survive round-tripping intact:** spaces, non-ASCII characters, UNC paths, and second volumes are normal input, not edge cases.
- **A multi-file model is a directory, not a file.** Multi-shard models and multimodal models with a projector are addressed as a set. The projector is identified by a filename beginning with `mmproj` — a convention, not a flag — which means the catalogue must recognise it by name rather than expecting the user to point at it. This holds on both registration channels.

### 2.2 Target hardware is Blackwell, on the CUDA 13 build

**Decision: NVIDIA Blackwell (RTX 50xx / RTX PRO), CUDA 13. Never silently accept a CUDA 12 build on Blackwell.**

The reasoning here was wrong in v5.0 and is worth correcting, because the wrong version leads to the right behaviour for a reason that does not survive contact with reality.

It is **not** true that CUDA 12 cannot address Blackwell. CUDA 12.8 and later support compute capability 12.x, and a llama.cpp build compiled with a 12.8+ toolkit runs on an RTX 50-series card.

The real problem is that **the CUDA major version in a release asset's name tells you nothing about which GPU architectures were compiled into it.** llama.cpp's CUDA backend is built for an explicit architecture list; if `120` is absent from that list, the binary loads, finds no usable kernels, and falls back to CPU **silently**. The user sees the app start normally and inference crawl, with no error anywhere. This is not hypothetical — the same omission has shipped in other projects packaging llama.cpp for Blackwell.

So the selection rule stands, for the corrected reason:

- **Blackwell takes the CUDA 13 build or nothing.** "Blackwell present, no CUDA 13 asset available" is a hard failure with a clear message. Not a downgrade, not a warning-and-proceed — because the failure mode being avoided is silent, and a silent failure cannot be recovered from by a user reading the UI.
- Non-Blackwell NVIDIA prefers CUDA 13 and may fall back to CUDA 12.
- Vulkan and CPU builds remain diagnostic fallbacks only, surfaced with an explicit warning that performance will be poor.

If a future build advertises its compiled architecture list in a machine-readable way, this rule can be relaxed to check the list directly. Until then the major version is the only signal available and it is treated conservatively.

### 2.3 Router mode, with the fallback contained

**Decision: build against router mode; isolate it behind a trait.**

#### Why the router exists, and what it does for us

Historically llama-server served exactly one model: started with `-m <path>`, it answered with that model for its whole life. Changing model meant stopping and restarting the process. That is fine for a fixed deployment and useless for desktop use, where the expectation set by LM Studio and Ollama is one endpoint, many models, and the client choosing per request via the `model` field. External tools — `llama-swap` among them — existed to fill that gap. Router mode brings the capability inside llama-server: started without `-m`, it keeps a registry, spawns a dedicated child process for a model when a request names it, and bounds residency with `--models-max` plus LRU eviction.

Two of this project's requirements are satisfied by that alone:

- **Agents receive the model list on connect** — `/v1/models` returns the registry in OpenAI-compatible form, so any standard client reads it without adaptation.
- **JIT model switching, LM Studio-style** — a request naming an unloaded model triggers its load, then is served.

These are the two hardest requirements on the list, and using the router means the app writes no code for either.

#### What abandoning it would cost

Without router mode the app would have to spawn one llama-server per model with its own `-m`, and therefore also: allocate and track a free port per instance; **read the `model` field from each request body and route accordingly**; implement an eviction policy when VRAM runs out; supervise N processes rather than one; and map failures across N instances into sensible HTTP responses.

That is the same work `llama-swap` does. It would be the riskiest component in the product — the most concurrent state, the most ways to fail silently — and ours to maintain permanently.

**Note what is and is not on that list.** Owning a socket and forwarding bytes to one fixed address (§2.7) is a small, testable piece of it. Choosing *where* to forward based on request content, and managing the lifecycle of what sits behind each destination, is the expensive part. The app does the first and not the second, which is exactly what `AGENTS.md` invariant 3 says.

#### The open risk

The open risk is reliability under VRAM pressure: unpredictable eviction, or loads that hang instead of returning an error. The orchestration surface therefore sits behind a `ModelOrchestrator` trait, with the router as the v1 implementation.

`llama-swap` is the contingency, **with its cost stated plainly rather than glossed over.** Swapping the interface is cheap; the capability is not equivalent. The router offers a dynamic budget — `--models-max N` keeps up to N models resident and evicts the least recently used. llama-swap defaults to one model at a time; concurrency comes from its `groups` feature, which is statically authored, loads an entire group together on the first request touching any member, and does not let a model belong to more than one group. Expressing "keep N resident within a VRAM budget, evict LRU" would mean generating a matrix of groups that approximates a policy the router applies on its own.

So the fallback is real but is a downgrade, not a lateral move. That raises the value of confirming router behaviour early rather than treating the fallback as painless insurance.

One thing §2.7 changes here: because the app owns the client-facing socket, **swapping the orchestrator no longer changes the address clients use.** The fallback becomes an internal substitution rather than a migration the user has to notice.

### 2.4 Empirical verification is the owner's

There are no `[human]` tasks. Properties that can only be confirmed on real hardware live in `docs/owner-verification.md` as a checklist for the project owner, outside the task list and outside the agent's Definition of Done. Tasks assert the structural properties that are testable without a GPU.

### 2.5 "LM Studio-style UI" means the visual language, not the stack

**Decision: Tauri + React, matching LM Studio's visual language rather than its implementation.**

The original requirement asked for LM Studio's UI framework. LM Studio is Electron + React — so the literal reading would mean Electron. It is worth being explicit about why we are not taking that reading, because the reasoning is not obvious.

**Matching the stack would not have delivered the look anyway.** LM Studio is closed source. Knowing it runs Electron and React tells us nothing about what actually makes its interface recognizable: its component library, design tokens, spacing scale, typography, or whether any of that is bespoke. None of it is published or derivable from the shipped app. Choosing Electron would buy the same starting point as choosing Tauri — an empty webview — and none of the design work.

**The visual language is the portable part, and it is not framework-bound.** Both Electron and Tauri render a web page in a desktop window. React, Tailwind and a component library behave identically in either. Everything that makes an interface feel like LM Studio — dark-first palette with a single accent, high information density, restrained motion, controls that expose real parameters rather than hiding them — is CSS and component design. That is what T-005 specifies, and it is reachable from either shell.

**What actually differs, and why Tauri wins here:**

- **Backend language.** Tauri's core is Rust, Electron's is Node. This app supervises a child process, reads headers from 65 GB binaries without loading them, talks to NVML, maintains a concurrent state machine, runs an HTTP listener with streaming pass-through, and writes SQLite. Rust's type system and error model earn their keep across all of that.
- **Rendering engine.** Electron bundles its own Chromium, so you know exactly what runs on the user's machine. Tauri uses the system webview — WebView2 on Windows, also Chromium-based, but versioned by Microsoft. For a Windows-only app the gap is small; the residual risk is a WebView2 update regressing something, which you cannot pin.
- **Footprint.** ~10-15 MB versus ~150 MB installed. Real, but not a serious argument for an app that manages 65 GB model files.

**The honest cost.** Electron's desktop ecosystem is more mature, and answers to specific problems are easier to find. ADR-001 is a direct symptom: a well-known Tauri type-generation library has sat in release candidate for three years. Expect to hit more of that.

Electron remains a viable fallback if Rust's ecosystem gaps become a drag. Switching would mean rewriting `src-tauri/` and keeping the entire `src/` renderer, since the architectural boundary (`ipc/` thin, `core/` free of framework imports) already isolates the shell. §2.7 raises the cost of that switch: the endpoint listener would move to Node too.

### 2.6 NVFP4 is recognized, labelled experimental, and not asserted

**Decision: the catalogue understands NVFP4 and says so honestly. The app does not claim support until llama.cpp ships it officially.**

v5.0 called NVFP4 "a first-class v1 requirement". That was ahead of the upstream: Blackwell-native NVFP4 tensor-core dispatch was still an open PR as of spring 2026, and the app has no reliable way to detect from a build whether the fast path is present — it is a property of compiled kernels, not of the CLI surface, so the verified flag list cannot answer it.

What ships in v1:

- The GGUF reader identifies NVFP4 quantization and the catalogue displays it.
- Compatibility is `SupportedWithWarnings` carrying an **`experimental`** label, with a note stating that the memory profile is expected to hold but the speedup depends on the active build and has not been confirmed.
- The model is importable, enablable and launchable. Nothing is blocked.
- **The estimator models the NVFP4 memory profile.** Memory saving follows from the format, not from kernel dispatch, so it is modelled like any other quantization — with a note that the figure is unvalidated.
- The UI never asserts that NVFP4 acceleration is active.

**Quantization label is header-faithful.** The reader derives the label from the header's `general.file_type` (`ftype_label`). For mixed-quantization writers (e.g., unsloth NVFP4), that field encodes the non-backbone tensors' quantization (Q8_0 attention + lm_head) rather than the backbone's (NVFP4), so the label is faithful to the header but does not identify the backbone's quantization. Owner's decision (9 Sept 2026, PROGRESS.md D-013): keep the header-faithful label; do not guess from the filename or `description`. A future task may find a better derivation (per-tensor types, or a writer-specific metadata key).

Promoting NVFP4 to plain `Supported`, with a real capability probe behind it, is a v1.1 item (§8). The probe most likely reads the server's startup output or `/props` rather than the flag list; that is an empirical question for `docs/owner-verification.md`.

### 2.7 The app owns the endpoint

**Decision: external clients connect to the app, not to llama-server. The app binds the public port and forwards every request, unchanged, to a single llama-server instance on an ephemeral loopback port.**

This is the one architectural decision that changed in v6.0, and it deserves its reasoning written down.

#### The problem it solves

Under the previous design llama-server bound `127.0.0.1:8080` directly and the app never saw a request. That is simpler, and it fails on the thing this app exists for: **an address that stays put.**

A user configures an agent, an IDE plugin, a script, a `.env` file. Then they update the llama.cpp build, or the server crashes and restarts, or they change a router setting and restart. Under direct binding, every one of those events is a window during which the configured address refuses connections — and any of them can change the port if the user touches it. The client sees `ECONNREFUSED` and reports it as its own failure. The user is left correlating two applications' states by hand.

When the app owns the socket, the listener is bound for as long as the app runs, **independently of whether llama-server is up**. A request arriving while the server is stopped gets a structured `503` naming the state and what to do about it, not a dropped connection. A request arriving during startup is held, up to a bounded timeout, and then served — which is the behaviour a user actually wants when a 65 GB model is loading.

Three further things come with it, none of which were reachable before:

- **Authentication is the app's.** The API key is checked at the app's edge, in constant time, before anything reaches the server. llama-server never needs the key and it never appears in `presets.ini`.
- **Exposure is the app's.** Binding to `0.0.0.0` is the app's decision, with the app's confirmation dialog. llama-server stays on loopback permanently and cannot be reached from the LAN even by mistake.
- **The request log exists.** Method, path, status, duration, byte counts — the data behind `active_requests` and `requests_last_minute`, and the diagnostic surface of T-051. Under direct binding these numbers had no source at all.

#### The cost, stated plainly

A reverse proxy in the path is a component that can break streaming, and streaming is the product's most visible behaviour. The specific hazards:

- **SSE must pass through unbuffered.** Any accumulation turns token-by-token output into a single delayed blob. This is the failure most likely to ship unnoticed, so T-042 asserts chunk boundaries and inter-chunk timing, not just final content.
- **Long-held requests.** A first request against an unloaded 65 GB model can hold the connection for minutes. Idle timeouts on either side must be configured to permit that, deliberately, rather than inheriting a default.
- **Header and status fidelity.** `Content-Type`, `Transfer-Encoding`, trailers and error bodies must survive untouched, or clients that switch on them misbehave in ways that look like server bugs.
- **One more hop to debug.** When something goes wrong the question "is it the app or the server?" now exists. The request log is partly there to answer it.

These are real, and they are bounded: there is exactly one upstream, at a known address, chosen once at spawn time. That is what keeps this a transport concern and not the `llama-swap` rebuild §2.3 rejects.

#### What the app must never do in the path

Restated from `AGENTS.md` invariant 3, because this is where it gets tested: **the app does not read request bodies.** Not to log the model name, not to count tokens, not to pretty-print an error. The `model` field is llama-server's business. The moment the app parses a body to make a decision, it has started building a router, and the invariant exists to catch that on the first commit rather than the tenth.

`TelemetrySnapshot.tokens_per_sec` therefore continues to come from parsing the server's own stderr output (T-060), not from inspecting responses.

#### Port allocation

- **Public port:** user-configured, default `8080`, default bind `127.0.0.1`. Never auto-incremented. If it is taken, the app fails with a `port_in_use` diagnosis naming the conflict — a server that silently moves breaks configured clients, which is the whole point of this decision.
- **Upstream port:** chosen by the app from an ephemeral range on `127.0.0.1`, passed to llama-server via `--host` and `--port`. Invisible to the user. If it collides, the app retries with a different port; this is internal and needs no user involvement. It is the one place auto-increment is correct.

#### What this does not change

The app still writes `presets.ini`, still supervises one process, still refuses to schedule. `ModelOrchestrator` still talks to the upstream directly for control operations (`/models`, `/props`, explicit load and unload) — those are the app's own calls and do not pass through the listener.

### 2.9 No per-model enable switch; removal is cheap instead

**Decision: a registered model is always exposed. `enabled` is gone. Removing an entry retains its tuning and its calibration, keyed by absolute path, so removing and re-adding is a reversible gesture rather than a destructive one.**

v5.0 carried a per-model enable/disable toggle, listed in §8 as shipping in v1. Two things pushed against it.

The first is §2.1: under scan-only registration the toggle would have to be expressed by which links exist, and the link primitives available without Administrator rights cannot always do that per file. The feature would have been exact on one channel and approximate on the other, with the UI apologising for the difference.

The second is that the toggle was solving the wrong problem. Its real value was never visibility in `/v1/models` — a user who does not want a model listed can remove it. Its value was **not losing the model's settings**: `-ngl`, context size, cache types, MoE offload, sampling defaults, and the launch history the estimator calibrates from. Disabling preserved all of that; removal discarded it, which is why disabling existed.

So the retention is the feature, and the toggle was a workaround for its absence:

- `launch_history` and `retained_model_settings` are keyed by the model's **absolute path** and have no foreign key to the catalogue entry. Removal does not touch them.
- Re-importing the same file restores its parameters, its flags, and its calibration. The estimator does not fall back to `Heuristic`.
- `sha256_head` is stored with the retained settings and checked on restore. A different file at a reused path gets a clean slate, not another model's tuning.
- Settings retained for files no longer in the catalogue are visible and clearable in Settings, so the table is not a place where things accumulate invisibly.

What is genuinely lost: a model cannot be hidden from clients while staying in the app's own list. That is the whole cost, and it is small.

### 2.10 Closing the window leaves the app in the tray

**Decision: closing the window hides it to the system tray, leaving the endpoint and the server running. Quitting is an explicit action from the tray menu, and it stops the server before the process exits.**

This follows directly from §2.7. An endpoint whose value is that it stays put cannot disappear because someone clicked the window's close button.

The hazard is the mirror image: a user who believes they closed the app while a llama-server child holds tens of gigabytes of VRAM and a listening port. Three things address it, and none is optional.

- **The tray icon reports state.** Running, Stopped and Crashed are visually distinct. A mute icon makes the running server invisible, which is exactly the failure being guarded against.
- **The first close explains itself once.** A notification stating that the app continues in the background, shown on the first close and not again.
- **Quit stops the server first.** Graceful stop, then the same **10-second grace timeout** the state machine already uses for `Stopping`, then a process-tree kill, then the listener unbinds, then the process exits. The app never exits while a child is alive: an orphaned llama-server holds VRAM and the port, and on next launch the app would find its own address occupied by its own previous run.

Unloading a 65 GB model is not instant, so quitting shows progress rather than appearing to hang, and the window can be reopened during it.

**Windows session end is the same path.** `WM_QUERYENDSESSION` runs the quit sequence with the shorter grace period the OS allows. Without it, logging off orphans the child — the one case where the user cannot see what was left behind.

**Launch at startup starts minimized to the tray**, not with the window open.

### 2.12 Running means preloaded, but availability does not wait for it

**Decision: the server reaches `Running` only once every model marked `preload` is in VRAM. Meanwhile the endpoint already forwards traffic, because the coordinator is answering.**

The transition out of `Starting` was the one edge of the state machine with no definition, which is a poor place to leave a gap: it decides whether the app says the server is alive.

Two readings were available. Declaring `Running` as soon as the coordinator answers is simple and makes start feel fast, but the dashboard would report a healthy server while a 65 GB preload is still in flight — precisely the moment the user is watching it. Waiting for the preloads is honest but turns `Starting` into a state that can last minutes.

The resolution is that these are two different questions wearing one name:

- **`Running` is a statement about configuration**, not about reachability. It means what the user asked for is in effect. So it waits for the preloads.
- **Reachability is the endpoint's business**, and the endpoint does not wait. Once the coordinator answers, requests are forwarded even though the state is still `Starting { Preloading }`. The app is never less available than the server it manages.

`StartupPhase` makes the wait legible instead of opaque: `WaitingForProcess` for the seconds before the coordinator answers, then `Preloading { done, total, current }` so the dashboard can show "2 of 3, loading Qwen3-72B" rather than a bar that looks stuck.

Two consequences worth stating:

- **Two timeouts, not one.** A coordinator that has not answered in 120 seconds is dead; a preload that has not finished in 120 seconds is normal. One constant covering both would be uselessly loose for the first case, which is the one that catches real failures.
- **The health endpoint is per build.** `/health` where the build has it, `/props` otherwise. T-023 records which, alongside the verified flags. The app does not assume.

### 2.13 Smaller decisions

| Question | Decision |
|---|---|
| TypeScript type generation | `ts-rs`, not `specta` — ADR-001. Specta 2.0 has been in RC since 2023 and requires three-way version pinning that has already broken against stable Tauri |
| Model identity | The absolute path. `sha256_head` detects and reports duplicates; it is not the primary key. Two identical files on different volumes are two models |
| `Backend` representation | `Cuda { major: u8 }`, not a closed `Cuda13 \| Cuda12`. A new CUDA major must not require a schema migration, and an unrecognized major must be representable rather than a parse failure |
| Removing a model while it is loaded | Blocked, with a message telling the user to unload first |
| Removing a model otherwise | Allowed; parameters, sampling defaults and launch history are retained by path (§2.9) |
| Closing the window | Hides to tray; the server keeps running (§2.10) |
| Meaning of `Running` | Preloads are in VRAM; traffic is forwarded before that (§2.12) |
| Endpoint concurrency | No app-imposed limit by default; when set, excess requests are refused, never queued |
| Endpoint latency | Budgeted and regression-tested: p99 overhead under 5 ms non-streaming, 15 ms to first byte |
| Documentation consistency | Linted in CI as a blocking check (T-006) |
| Quitting from the tray | Graceful stop, 10 s grace, tree kill, unbind, exit — the same 10 s the `Stopping` state uses, not a second constant |
| Public port already in use | Fail explicitly with a diagnosis. Never auto-increment |
| Upstream port already in use | Retry with another ephemeral port, silently. Internal detail |
| Requests while the server is down | Structured `503` from the app naming the state; never a refused connection |
| Requests during startup | Held up to `startup_hold_seconds` (default 120), then `503` |
| App downgrade (DB newer than binary) | Refuse to start with a clear message; never crash or migrate backwards |
| Import concurrency | Background queue, parallelism 4, cancellable, per-file progress |
| Config edits while the server runs | Persisted, `config_dirty` flag raised, restart banner. No hot-reload in v1 |
| UI language | English only; all strings centralized in `src/lib/strings.ts` |
| Visual reference | LM Studio's look, reimplemented — no assets, icons, or code from it |

---

## 3. Stack

| Layer | Choice | Version |
|---|---|---|
| Desktop shell | Tauri | 2.x |
| Core language | Rust | **1.88+** (edition 2021) — MSRV of `ts-rs` 12.x |
| UI | React + TypeScript | 18.x / 5.x |
| Styling | Tailwind CSS + shadcn/ui | 3.x |
| UI state | Zustand | 4.x |
| Database | SQLite via `rusqlite` (bundled) | — |
| HTTP server (endpoint listener) | `axum` | 0.7+ |
| HTTP client | `reqwest` (rustls, json, stream) | — |
| Async runtime | `tokio` | — |
| GGUF parsing | Custom header reader, `core/src/gguf/` | — |
| GPU telemetry | `nvml-wrapper` | — |
| Logging | `tracing` + `tracing-subscriber` | — |
| Rust tests | `#[test]` + `insta` snapshots + `proptest` | — |
| Type generation | `ts-rs` (see `docs/adr/001-type-generation.md`) | 12.x |
| UI tests | Vitest + React Testing Library | — |
| E2E | Playwright against the Tauri build | — |

**Approved Rust crates:** `tauri`, `tokio`, `serde`, `serde_json`, `rusqlite`, `axum`, `reqwest`, `futures-util`, `subtle`, `tracing`, `tracing-subscriber`, `thiserror`, `anyhow`, `nvml-wrapper`, `zip`, `sha2`, `semver`, `regex`, `insta`, `proptest`, `tempfile`, `ts-rs`, `windows` (DPAPI, junctions).

Three of those are new in v6.0 and exist only because of §2.7:

- **`axum`** — the endpoint listener. Brings `hyper` and `tower` transitively; do not depend on them directly.
- **`futures-util`** — stream adapters for forwarding a response body without collecting it. The one place where getting this wrong silently breaks SSE.
- **`subtle`** — constant-time comparison for the API key. Hand-rolling this is possible and is the kind of thing that is subtly wrong for years.

**Approved npm packages:** `react`, `react-dom`, `@tauri-apps/api`, `zustand`, `tailwindcss`, `class-variance-authority`, `clsx`, `lucide-react`, `recharts`, `vitest`, `@testing-library/react`, `playwright`.

### Repository layout

```
llama-manager/
├─ AGENTS.md
├─ PROGRESS.md
├─ PLAN.md
├─ src-tauri/
│  ├─ src/
│  │  ├─ main.rs
│  │  ├─ ipc/                    # thin command handlers
│  │  ├─ core/
│  │  │  ├─ env_probe.rs         # T-010
│  │  │  ├─ runtime_manager.rs   # T-020..T-023
│  │  │  ├─ gguf/                # T-030
│  │  │  ├─ model_paths.rs       # T-029  path validation, availability, links
│  │  │  ├─ model_registry.rs    # T-031
│  │  │  ├─ estimator.rs         # T-032  pure
│  │  │  ├─ preset_generator.rs  # T-033
│  │  │  ├─ supervisor.rs        # T-040  actor
│  │  │  ├─ orchestrator/        # T-041  trait + router impl
│  │  │  ├─ endpoint/            # T-042 transport, T-044 state behaviour,
│  │  │  │                        # T-051 auth and request log
│  │  │  ├─ diagnostics.rs       # T-043
│  │  │  ├─ lifecycle.rs         # T-047  tray, quit sequence, session end
│  │  │  └─ telemetry.rs         # T-060
│  │  ├─ db/
│  │  │  ├─ migrations/
│  │  │  └─ queries.rs
│  │  └─ error.rs
│  ├─ fixtures/                  # synthetic GGUF, --help captures, stderr samples
│  ├─ tests/
│  └─ Cargo.toml
├─ src/                          # React renderer
│  ├─ screens/                   # Dashboard, Runtime, Models, ModelDetail, Api, Logs, Settings
│  ├─ components/ui/
│  ├─ lib/{ipc,types,strings}.ts
│  └─ store/
├─ scripts/make-gguf-fixture.py
├─ scripts/export-verified-flags.ps1
├─ scripts/probe-router.ps1        # T-025, rerunnable per build
├─ scripts/lint-docs.py           # T-006, blocking in CI
├─ .cargo/config.toml            # TS_RS_EXPORT_DIR
├─ rust-toolchain.toml           # pinned 1.88, MSVC target
├─ docs/
│  ├─ DEV-SETUP.md
│  ├─ WORKFLOW.md
│  ├─ CONTRACTS.md
│  ├─ TASKS.md
│  ├─ LLAMACPP.md              # claims to be verified — never authoritative
│  ├─ verified-flags.md        # generated from a real capture — authoritative
│  ├─ owner-verification.md    # owner only
│  └─ adr/
│     ├─ 001-type-generation.md
│     └─ 002-endpoint-ownership.md
└─ .github/pull_request_template.md
```

---

## 4. Milestones

**This table is an ordering, not a schedule.** Every task is executed by an agent, one task per branch, in the order the selection rule produces (`docs/WORKFLOW.md` §2). Estimating it in developer-weeks — as versions up to v6.2 did — measured a currency this project does not spend, and implied a parallelism the workflow forbids. What actually paces the work is the review queue: roughly forty PRs, each gated on the owner reading it, plus every discrepancy, which by design **stops the agent and waits for an owner decision** (`docs/WORKFLOW.md` §5). That is the critical path, and it is the owner's to manage.

| Milestone | Tasks | Outcome |
|---|---|---|
| **A — Skeleton** | T-000…T-006 | CI, scaffold, DB, error model, design system, documentation lint |
| **B — Runtime** | T-010…T-025 | Environment probing, binary install, version management, flag verification, empirical router probe |
| **C — Catalogue** | T-029…T-036 | Paths, GGUF reader, registry, estimator, preset generation, model screens, speculative decoding |
| **D — Server** | T-040…T-047 | Supervisor, orchestrator, endpoint transport and behaviour, diagnostics, dashboard, logs, tray and shutdown |
| **E — API** | T-050…T-052 | API screen, auth and request log, client contract tests |
| **F — Release** | T-060…T-065 | Telemetry, multi-GPU, crash recovery, settings, installer, E2E |

Structural changes since v5.0: the preset generator moved from D into C, because two Milestone C screens assert byte-identical output against it and could not do so while it lived downstream; Milestone D absorbed the endpoint listener; and v6.3 added T-025 to Milestone B, moving the empirical router questions out of the owner's checklist and into the task list.

No milestone is gated on hardware access. The owner's verification checklist covers only what a GPU-less machine genuinely cannot answer, and feeds corrections back as follow-up tasks.

**Two decision gates, and they are consecutive.**

1. **T-023** answers §2.1's `--models-preset` question *from `--help`*, and records `registration_channel`. If the help text does not answer it, the value is `Undetermined` — which is a legitimate outcome, not a failure.
2. **T-025** answers the same question *empirically*, along with reparse-point support, scan depth, and projector expression. **Its result overrides T-023's**: a capture of the binary's actual behaviour beats an inference from its help text, including when the two agree.

T-033 starts only once T-025 has recorded its outcome in Facts established. If the outcome is still `Undetermined` after T-025 — meaning the binary would neither confirm nor deny — T-033 is `Blocked` and the question goes to the owner, because at that point it has genuinely exhausted what a GPU-less machine can establish.

If the answer is scan-only, T-033's link-based path applies and T-029 must already provide **file linking** — symlink, else same-volume hard link. Not a junction: §2.1 establishes that a junction cannot link an individual `.gguf`. Check that before writing the generator, not during.

---

## 5. Risks

| Risk | Impact | Mitigation | Owner |
|---|---|---|---|
| Proxy buffers SSE, breaking streaming | **High** | Explicit streaming pass-through; chunk-boundary and timing assertions in T-042; differential test against direct upstream access | T-042 |
| Long model loads hit an idle timeout in the proxy | Medium | Timeouts configured explicitly, never inherited; startup hold is a named setting | T-042 |
| Router mode unreliable under VRAM pressure | High | `ModelOrchestrator` trait isolates it; `llama-swap` is a second implementation, at the cost of static group concurrency instead of a dynamic budget | T-041 |
| CUDA build lacks sm_120 kernels and falls back to CPU silently | High | Blackwell requires the CUDA 13 asset; never a silent downgrade; §2.2 | T-021 |
| Flags change between builds | Medium | Per-build verified flag list; omit-and-warn at runtime; stop-and-report at dev time | T-023, T-033 |
| Router registration is scan-only **and** file symlinks need privileges the user lacks | Medium | Capability probed at startup; hard link where the volume allows it; an honest failure otherwise. Whether the scan follows reparse points at all is established by T-025 before any of this is built | T-023, T-025, T-029, T-033 |
| Model paths become unreachable (drive removed, file renamed) | Medium | Marked `Missing`, catalogue keeps working, rescan restores | T-029, T-031 |
| VRAM estimate wrong → OOM | Medium | Conservative bias, calibration from launch history, owner-verified accuracy | T-032 |
| NVFP4 speedup absent on the shipped build | Low | Labelled experimental; capability never asserted; §2.6 | T-031 |
| 65 GB loads feel broken | Medium | Byte-level progress and ETA, preload, pinning, request held during startup rather than refused | T-035, T-044 |
| Orphaned llama-server after quit or logoff, holding VRAM and the port | Medium | Quit stops the server before exiting; session-end handled; orphan detected at next launch | T-046 |
| Scope creep toward a chat client | Low | `AGENTS.md` §2; `--no-webui` enforced | T-040 |
| Scope creep from proxy toward router | **Medium** | `AGENTS.md` invariant 3; a test asserts no request body is ever deserialized in `endpoint/` | T-042 |

---

## 6. Security

**Endpoint exposure.** The app's listener defaults to `127.0.0.1`. Binding `0.0.0.0` exposes an inference server to the LAN and requires explicit confirmation naming that consequence; combined with no API key it raises a persistent warning. **llama-server itself always binds `127.0.0.1` on an ephemeral port and is never configurable to do otherwise** — the only way in is through the app. Never open a firewall rule programmatically.

**API key.** Encrypt with Windows DPAPI (`CryptProtectData`, user scope); store only ciphertext in `settings`. Checked at the app's listener with `subtle`'s constant-time comparison. Never passed to llama-server, never in `presets.ini`, never in logs. Write-only in the UI: masked, replaceable, never redisplayed. A `tracing` redaction layer plus a test asserting the key never reaches the log file or the request log.

**Request log.** Method, path, status, duration, byte counts, timestamp. **Never request or response bodies**, never `Authorization` header contents. Capped ring buffer, in memory, not persisted.

**Downloaded archives.** SHA-256 verification before extraction; zip-slip rejection during it. Both are acceptance criteria, not best-effort.

**Log export.** Redacted identically to the crash bundle: paths outside the app's data directory reduced to basenames, API key removed. An exported log reaches the same places a bundle does.

**Crash bundle.** Includes app logs, server stderr tail, environment report, `presets.ini` with model paths reduced to basenames, app version, runtime build tag. Excludes API keys, request log contents, filesystem paths outside the app directory, model files. Written locally, never uploaded.

**Model paths.** Read-only access, always. The app opens model files to read GGUF headers and never writes to them, moves them, or deletes them. Links created under §2.1's fallback — file symlinks, same-volume hard links, and junctions for multi-file model directories only — live exclusively in the app's data directory and are removed with the entry that created them.

---

## 7. Definition of Done (v1.0)

- [ ] Clean install on Windows 11 + Blackwell GPU with no terminal use and no external runtime dependencies
- [ ] Install state, build version, and available updates detected reliably, with rollback
- [ ] Start/stop feedback accurate within 2 s of the real event; every illegal transition rejected
- [ ] Preloading is shown as progress, and traffic is served during it rather than held
- [ ] GGUF import via file explorer from any drive, including multi-part shards; files are never moved
- [ ] Per-model launch parameters and sampling defaults, with live `presets.ini` and command preview
- [ ] Config changes while running are persisted and signalled, never silently applied
- [ ] **The configured endpoint address never changes across server restarts, build switches, or crashes**
- [ ] **A request arriving while the server is stopped receives a structured 503, never a refused connection; one arriving during startup is held and then served**
- [ ] **Streaming responses are byte-identical and chunk-identical to direct upstream access, asserted by a differential test**
- [ ] Endpoint overhead stays within its stated budget, checked in CI rather than assumed
- [ ] OpenAI-compatible endpoint passes the recorded client contract tests
- [ ] `/v1/models` returns the full catalogue; changing the `model` field triggers a JIT load
- [ ] Startup failures produce an actionable diagnosis, never a raw stack trace
- [ ] No chat or inference feature; the server's own web UI disabled
- [ ] A 65 GB model loads via partial offload with correct estimates and warnings
- [ ] NVFP4 models recognized and labelled experimental, with no capability asserted
- [ ] Removing a model and re-importing the same file restores its parameters and its calibration
- [ ] Quitting from the tray leaves no llama-server process alive and no port bound, including on Windows logoff
- [ ] API key never stored or logged in plaintext; llama-server never receives it
- [ ] `cargo clippy -- -D warnings`, `npm run lint`, full test suite and E2E path all green

---

## 8. Deferred to v1.1+

Recorded so they are not silently reintroduced:

- Hugging Face direct download (`-hf user/model`)
- **NVFP4 promoted from experimental to supported, behind a real capability probe** (§2.6)
- i18n beyond English
- Hot-reload without restart
- LAN multi-user with per-user keys
- In-app quantization or conversion
- Rate limiting or per-client quotas at the endpoint
- Request-log persistence and export

- **Per-model enable/disable** — dropped rather than deferred (§2.9). Retention of settings and calibration on removal replaces it; reintroducing the toggle would reintroduce the scan-only granularity problem it never solved.

**Not deferred — these ship in v1:** API-key authentication, the request log as a live in-memory view, retention of model settings across removal, tray operation with a clean quit.
