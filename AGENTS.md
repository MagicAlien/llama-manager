# AGENTS.md — Binding rules for the implementing agent

This file is loaded automatically by agentic tooling at the start of every session. It is the only document whose rules apply unconditionally. Everything else is loaded on demand.

**Open `PROGRESS.md` next.** It is the project's memory across sessions — which tasks are done, which is in flight, what is blocked, what was learned. You have no memory of your own; that file is it. The session protocol is `docs/WORKFLOW.md`.

Environment setup: `docs/DEV-SETUP.md`. No GPU, CUDA toolkit or model files are required to complete any task. A CPU build of llama-server is required for three tasks only (T-023, T-025, T-043) and the agent downloads it itself. It runs on any machine; it is not a GPU requirement in disguise.

Project: **llama.cpp Manager** — a Windows desktop app that manages a llama.cpp server and owns the HTTP endpoint clients connect to. No inference in the app.

---

## 1. Never fabricate CLI flags or API endpoints

Every llama.cpp flag and HTTP endpoint in `docs/LLAMACPP.md` is a **claim to be verified**, not an established fact. llama.cpp renames, retypes, and removes flags between builds.

Before writing code against a flag, check it against `docs/verified-flags.md`, which is generated from a real `llama-server.exe --help` capture. Confirm the flag exists **and takes the expected value type**.

**Two different behaviours, do not confuse them:**

| Moment | Condition | Required behaviour |
|---|---|---|
| **Development time** (you, writing code) | A flag in `docs/LLAMACPP.md` is absent from `docs/verified-flags.md`, or has a different value type | **STOP. Report the discrepancy.** Do not guess a replacement, do not silently adapt the type, do not remove the feature. |
| **Runtime** (the shipped app) | A flag modelled in `LaunchParams` is absent from the active build's verified flag list | **Omit it and warn.** The app emits a UI warning naming the flag and continues. Never write a malformed preset file, never crash. |

The first is your rule. The second is a behaviour you implement (T-033).

**One documented exception:** `LaunchParams::extra_args` passes user-typed strings through unvalidated. It is never populated by app logic, only by direct user input; the UI labels it as unvalidated; T-033 emits it verbatim without parsing.

## 2. Do not add excluded features

- **No chat UI.** No message composer, no conversation view, no inference call from the app for user-facing text generation. Health checks and metadata calls are fine.
- **No Hugging Face model downloading in v1.**
- **No cloud sync, no accounts, no telemetry upload.**

If a task seems to require one of these, the task was misread. Stop and report.

## 3. Every task is yours to complete

There are no tasks assigned to a human. Every acceptance criterion in `docs/TASKS.md` is checkable by you, in CI or on a GPU-less development machine, using fixtures, mocks, stubs and synthetic data.

**This means acceptance criteria never depend on real GPU hardware, real model files, or real third-party clients.** Where a property can only be confirmed on the target machine — actual VRAM accuracy, real hot-swap latency, real client interoperability — the task asserts the *structural* property you can test (determinism, monotonicity, correct error typing, protocol shape) and the empirical confirmation happens separately, outside the task list, in `docs/owner-verification.md`. That file is the project owner's, not yours. Do not add tasks to it, do not tick its boxes, and do not treat an unticked box as blocking your work.

If you find yourself writing "this needs to be verified on real hardware" inside a task's acceptance criteria, the criterion is wrong. Rewrite it as a property you can test, and note the empirical question in your PR description so it can be added to the owner checklist.

**The same test applies in the other direction, to `docs/owner-verification.md` itself.** A question belongs to the owner only when it genuinely needs a GPU, a real model file, or a real client. A question answerable by running a CPU build of llama-server against a prepared directory is **yours**, and it lives in the task list — that is what T-025 is. If something in the owner's checklist looks answerable without hardware, do not do it silently: record it in Discrepancies and let the owner move it.

## 4. Dependency policy

A dependency must appear in `PLAN.md` §3 or be justified in the PR description. Prefer the standard library. Never add a crate or npm package that duplicates something already listed.

## 5. Working method

Full protocol in `docs/WORKFLOW.md`. The short form:

- One task (`T-xxx`) per branch, per PR. Do not batch.
- Take the lowest-numbered task whose dependencies are `Done` and which is not `Blocked`. Do not reorder.
- A task is done when every acceptance criterion passes — not when the code compiles.
- Write tests alongside the implementation, never after the PR is opened.
- If a criterion cannot be met because reality differs from the documents, **stop and record it in the Discrepancies section of `PROGRESS.md`**. That is where "stop and report" goes. Do not silently redesign, and do not route around it.
- A document is corrected by a task, never in passing. If a decision has been taken and the documents do not reflect it yet, that is a correction task in the `T-1xx` range (`docs/WORKFLOW.md` §7) — not something you fix while you are here.
- Update `PROGRESS.md` when you close a task. State and discoveries, not a narrative of what you did.
- Never commit secrets, absolute paths from your machine, or model files.
- Fill in `.github/pull_request_template.md` completely. An unchecked box is an unfinished task.

## 6. Architectural invariants

These hold across every task. Violating one is grounds for rejecting the PR regardless of test results.

**Cite them as "`AGENTS.md` invariant N", never as "§6.N".** They are list items, not headings, and the documentation lint (T-006, check 3) resolves section references against real headings only.

1. **`ipc/` contains no logic.** It deserializes, calls `core/`, serializes. `core/` never imports `tauri`.

2. **`ServerState` is owned by one actor task.** Commands go over an mpsc channel and await a reply. No shared `Mutex<ServerState>`.

3. **The app is a transparent reverse proxy in front of exactly one llama-server process. It is not a scheduler.**

   The app owns the listening socket that clients connect to and forwards every request to a single fixed upstream (`PLAN.md` §2.7). That is the whole of its involvement in the data path. Specifically, it **must not**:

   - inspect, parse, or rewrite request or response bodies — the `model` field is never read by the app;
   - choose which model serves a request, or when a model loads or unloads on a request's behalf;
   - run more than one llama-server process, or allocate a port per model;
   - implement eviction, queuing by model, or any residency policy;
   - buffer a streaming response, or alter status codes and headers except to reject an unauthenticated request.

   Model routing, JIT loading and LRU eviction belong to llama-server's router mode. `PLAN.md` §2.3 explains what this invariant protects — reintroducing that work is the single largest way this project can grow out of control. If code starts to look like `llama-swap`, it is wrong.

4. **The proxy is transparent by default.** With no API key configured, a response reaching the client is byte-identical to what the upstream produced. This is a testable property (T-042), not an aspiration.

5. **The estimator is a pure function.** It receives launch history as an argument; it never reads the database.

6. **No `unwrap()` or `expect()`** in `core/` or `ipc/` outside tests. Enforced by clippy in CI.

7. **All user-facing strings live in `src/lib/strings.ts`.** No string literals in JSX text positions.

8. **Model files are never copied, moved without consent, or deleted by the app.**

9. **`src/lib/types.ts` is generated, never edited.** `src/lib/ipc.ts` is hand-written but contains wrappers only — it imports types, never redefines them (ADR-001).
