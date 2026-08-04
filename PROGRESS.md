# PROGRESS

**Open this first, every session.** It is the project's memory across sessions — you have none of your own.

Protocol: `docs/WORKFLOW.md`. Rules: `AGENTS.md`.

**This file grows and nothing trims it by itself.** It is read in full at the start of every session, so its length is a running cost paid out of the context budget you need for the work. Pruning is a correction task (`docs/WORKFLOW.md` §7) and is commissioned at the close of a milestone, never done in passing. The rules differ per section and the asymmetry is the point:

- **Done** — compress to one line per milestone plus the PR numbers once the milestone closes. The diffs already hold the detail, and better than prose does.
- **Observations** — triage each one: it becomes a `T-1xx`, or it is closed with the reason it no longer applies, or it stays. "It stays" must be a decision someone made, not what happens when nobody looks.
- **Discrepancies** — a `resolved` entry keeps its one-line summary and its resolution, and loses its working detail.
- **Facts established — never pruned.** Every line there cost an experiment to learn. Deleting one means a future session rediscovers it the expensive way, which is the exact failure this file exists to prevent. If the file must get shorter, it gets shorter somewhere else.

Last updated: 4 August 2026 — T-000 complete, PR #3 open; **three discrepancies raised in review (D-004, D-005, D-006)**, one of which blocks T-006 and one of which must be settled inside T-001. Repository initialised (git + remote `MagicAlien/llama-manager`, private); `main` holds the v6.3 planning documents only. The CI gates are live and their red path has been executed against deliberately broken code.

---

## In progress

*(nothing — T-000 is Done; its PR #3 awaits review. When it merges, take T-001.)*

> One task at a time. If something is listed here, it is yours: check out its branch and continue it. Do not start a new task.

---

## Next up

**T-001 — Scaffold** `[dep: T-000]`. Take it once PR #3 has merged. **Read D-006 first.** The `rust-toolchain.toml` T-001 is required to create silently disables the clippy and fmt gates unless `ci.yml` changes in the same PR.

Selection rule: the lowest-numbered task in `docs/TASKS.md` whose dependencies are all `Done` and which is not `Blocked`.

**Task numbers changed again in the v6.3 review** (new T-025 in Milestone B; before that, v6.2 split the endpoint and added T-006). The maps are at the top of `docs/TASKS.md`. Nothing was renumbered in v6.3 — T-025 was appended.

---

## Done

- **T-000** — CI pipeline · PR #3 · 2026-08-04
  - Note: `jobs.gates` in `.github/workflows/ci.yml` is the job T-002 extends — **by editing it, not by calling it**; it is not a `workflow_call` job (see Observations). Red path demonstrated by throwaway PRs #1 (clippy warning) and #2 (unformatted); both closed without merging and their branches deleted — run ids and step-level evidence are recorded in PR #3's body. The `demo:failure` label job re-proves the red path on demand. Warm green run: 51–63 s **on a crate with no dependencies** — see Observations before treating that as the pipeline's real cost.
  - Note: T-000 also shipped `.gitignore`, which `docs/TASKS.md` assigns to T-001. T-001 still owes `rust-toolchain.toml` and `.cargo/config.toml`.
  - Note: `main` runs red until PR #3 merges (it carries no code yet). That run (30926658728) failing at `cargo fmt --check` is expected, not a regression.

---

## Blocked

- **T-006** — blocked by D-005 (see Discrepancies). Check 9 cannot pass as specified, and the exemption mechanism is an owner decision. Checks 1–8 are writable today, but shipping those and quietly omitting 9 would leave a blocking gate that looks complete and is not — which is the failure T-006 exists to prevent, committed by T-006 itself.

<!-- Format:
- **T-033** — blocked by D-001 (see Discrepancies). Cannot proceed until resolved.
-->

---

## Discrepancies

Things where reality differs from the documents, or where a task is ambiguous. **These are decisions for the project owner.** Record and stop; do not route around them.

### D-001 — Router registration channel is unknown until T-023 and T-025 run
- Type: ambiguity (open by design, with a scheduled answer)
- Found in: `PLAN.md` §2.1, pre-implementation review
- Evidence: no `--help` capture and no probe result exists yet for any build
- Affects: T-023 (reads it from `--help`), T-025 (observes it against the running binary), T-033 (branches on it), `CONTRACTS.md` `RegistrationChannel`
- Proposed: T-023 records `PresetDeclaresPath`, `ScanOnly` or `Undetermined` in Facts established from the help text. **T-025 then records what the binary actually does, and its value is the one that binds** — including where the two agree, since one is an observation and the other a reading. T-033 does not start before T-025's entry exists. If the value is still `Undetermined` after T-025, T-033 becomes Blocked and this entry is reopened as an owner decision, because the binary itself declined to answer.
- Status: open — expected to resolve in T-025 (v6.2 expected T-023; the help text alone was never going to settle questions 2 and 3)

### D-002 — GPU-dependent stderr fixtures cannot be captured by the agent
- Type: discrepancy
- Found in: T-043, pre-implementation review
- Evidence: missing-CUDA-DLL, VRAM-exhaustion and driver-mismatch messages require a GPU machine to produce
- Affects: T-043 fixture set
- Proposed: those patterns are hand-written from upstream error strings and **labelled unverified in the fixture file**; the owner replaces them with real captures later. A test asserts the labelling is accurate. The task is not blocked by this.
- Status: resolved (labelled-fixture approach adopted in T-043)

### D-003 — `idle_unload_seconds` is modelled with no known flag behind it
- Type: discrepancy
- Found in: `docs/CONTRACTS.md` §1 `ServerConfig`, T-050, v6.3 review
- Evidence: `docs/LLAMACPP.md` listed no flag for it in any section; the field is nonetheless exposed in the API screen
- Affects: `ServerConfig`, T-023 (must look for it in the capture), T-050 (renders the control), T-033 (would emit it)
- Proposed: `docs/LLAMACPP.md` §1 now carries it as **[assumed]** with no observed spelling. T-023 confirms or refutes it against the capture. T-050 renders the control only when the active build's verified list contains the flag, and omits it entirely otherwise. If T-023 finds nothing, the field is dropped rather than left as a setting that does nothing.
- Status: open — expected to resolve in T-023

### D-004 — ADR filenames on disk do not match the citations
- Type: discrepancy
- Found in: T-000, PR #3 review
- Evidence: the files are `docs/adr/adr-001-type-generation.md` and `docs/adr/adr-002-endpoint-ownership.md`. `PLAN.md` §3 cites `docs/adr/001-type-generation.md` in the dependency table and lists `001-type-generation.md` / `002-endpoint-ownership.md` in the directory tree; `docs/TASKS.md` T-002 cites `docs/adr/001-type-generation.md`. Three citations disagree with the two files.
- Affects: `PLAN.md` §3, `docs/TASKS.md` T-002, T-006 check 9
- Proposed: either rename both files to drop the `adr-` prefix, or retype the three citations. The choice is arbitrary and the change is a few lines either way — but it has to be made, because every link is dead in GitHub's rendered view today and T-006 check 9 is a blocking gate that depends on T-000 alone. Recorded rather than fixed inside T-000: applying a decision is a `T-1xx` the owner commissions (`docs/WORKFLOW.md` §7), and no decision has been made yet.
- Status: open — needs an owner decision before T-006

### D-005 — T-006 check 9 fails on every path a later task is due to create
- Type: discrepancy (the check as specified cannot pass)
- Found in: `docs/TASKS.md` T-006 check 9, T-000 PR review
- Evidence: check 9 requires every file path cited in any document to exist with that exact case, and exempts only `docs/verified-flags.md`. A scan of the documents as committed finds six further cited paths that do not exist and are legitimate deliverables of later tasks: `.cargo/config.toml` (T-001), `src/lib/types.ts` and `src/lib/ipc.ts` (T-002), `src/lib/strings.ts` (T-005), `scripts/export-verified-flags.ps1` (T-023), `scripts/probe-router.ps1` (T-025). T-006 depends on T-000 alone, so it runs in Milestone A when none of them exists. Separately, check 9's own prose uses `docs/workflow.md` as its worked example of a miscased path, so the check flags its own description.
- Affects: `docs/TASKS.md` T-006, and through it the T-000 pipeline, since check 9 is blocking
- Proposed: three options, all cheap. **(a)** A named allow-list file, exactly as check 4 already has, one line per not-yet-created path with the owning task named on it — the exemption becomes visible, reviewable, and shrinks on its own as tasks land. **(b)** Restrict check 9 to paths under `docs/`, `.github/` and the repository root, which is where a case-only mistake actually costs something. **(c)** Exempt any path named as a deliverable in `docs/TASKS.md` — no second list, but it couples the lint to task prose, which is the coupling check 5 was rewritten in v6.3 to remove. **(a)** is the closest fit to what T-006 already does elsewhere. Under any option, check 9's prose must stop using a real-looking path as its example.
- Status: open — T-006 is Blocked until this is settled

### D-006 — The CI toolchain and `rust-toolchain.toml` will disagree from T-001 onward
- Type: discrepancy
- Found in: `.github/workflows/ci.yml`, `docs/DEV-SETUP.md` §143 and §194, T-000 PR review
- Evidence: `ci.yml` installs the toolchain with `dtolnay/rust-toolchain@stable` and adds clippy and rustfmt to *that* toolchain. T-001 must create `rust-toolchain.toml` pinning `channel = "1.88"`, and that file overrides the action's default for every subsequent cargo invocation. As `docs/DEV-SETUP.md` §143 specifies it, the file declares a channel and a target and no components — so rustup fetches 1.88 without clippy or rustfmt, and `cargo clippy` fails with "no such command". A gate red for the wrong reason is worse than a gate that is missing, because it trains everyone to ignore it. The action's own documentation states it is incompatible with a checked-in `rust-toolchain.toml`. Separately, `docs/DEV-SETUP.md` §194 states that T-000's pipeline installs nothing beyond components and cached dependencies; it installs a toolchain.
- Affects: `.github/workflows/ci.yml`, `docs/DEV-SETUP.md` §143 §194, T-001
- Proposed: pin the action to the version the file pins (`dtolnay/rust-toolchain@1.88`) **and** add `components = ["clippy", "rustfmt"]` to `rust-toolchain.toml`, so the pin holds whichever of the two wins; then correct §194 to say the pipeline installs a pinned toolchain with components. Unlike D-004 and D-005 this is not a document-only fix, so it is not a `T-1xx`: both halves belong in T-001, which is the PR that creates the file that breaks it.
- Status: open — must be settled inside T-001, not after it

<!-- Format:
### D-00x — <one-line summary>
- Type: discrepancy | ambiguity
- Found in: T-xxx, llama.cpp build bXXXX (if relevant)
- Evidence: file and line, command output, or test name
- Affects: which documents and which tasks
- Proposed: what you would do, if you have a view
- Status: open | resolved (<how>) | resolved by T-1xx
-->

**A discrepancy is not resolved by agreeing on the answer.** It is resolved when the documents say it. Where the fix is a document change, the entry stays `open` until its correction task is `Done`, and then reads `resolved by T-1xx` (`docs/WORKFLOW.md` §7). T-006 check 8 verifies that the named task exists.

---

## Observations

Things noticed in passing that are not part of any current task — a rough edge, a probable future problem, a small bug outside your scope. Noted here rather than fixed, per `docs/WORKFLOW.md` §3.

- **The endpoint listener is the project's most likely source of a silent regression.** Buffering an SSE stream produces correct-looking output in every test that compares concatenated bodies. T-042's chunk-boundary assertion is the only thing that catches it; if that test ever becomes flaky, fix it rather than relaxing it.
- **Whether llama-server's `--models-dir` scan follows reparse points is unknown**, and `--help` will not answer it. The whole scan-only fallback depends on it. **This is T-025's question 2 as of v6.3**, not the owner's — running a CPU build against a directory of links needs no hardware.
- **T-029's link half may be dead code, and only T-025 can say.** `link_into`, `unlink`, `link_capability` and `LinkCapability` serve the `ScanOnly` channel alone; `PLAN.md` §2.1 states that under `PresetDeclaresPath` that branch never runs, and F-000's preliminary evidence points that way. T-029 now depends on T-025 so the answer is known before the module is written, but **the decision of what to do with the answer is the owner's**: implement the full surface regardless (an unused function costs less than a missing one), or implement only what the confirmed channel needs and add the rest as a correction task if the channel ever changes. Default, absent a decision, is the first. Worth settling before Milestone C rather than during it.
- **`docs/LLAMACPP.md` now exists but contains no verified entry.** Every flag in it is marked `[doc]` or `[assumed]`; nothing is marked verified, because only T-023 can do that. Treat a `[assumed]` flag exactly as `AGENTS.md` §1 says: check it against `docs/verified-flags.md` first, and stop if it is absent or retyped.
- **The four questions that used to be `docs/owner-verification.md` Session 1 are now T-025**, joined by a fifth on how a projector is declared on the preset channel, and the reason is worth remembering: none of them needs a GPU, a real model, or the target machine, so classifying them as the owner's violated `AGENTS.md` §3 in the one document that defines it. What stayed with the owner is what genuinely needs hardware.
- **`docs/verified-flags.md` does not exist yet, and cannot until T-023 runs.** `AGENTS.md` §1 makes checking it an unconditional precondition for emitting any flag, and the PR template has a mandatory column for it. This is not a contradiction, because no task before T-033 emits a flag and T-033 depends on T-023 — but the ordering is load-bearing and worth knowing before someone "fixes" it by hand-writing the file. It is produced by `scripts/export-verified-flags.ps1` from the database, **after** T-023, and committed. Nothing writes it before then, and nothing outside that script writes it at all. *(An earlier version of this note had lost its subject and claimed the file was scheduled to be written before T-023, which is the opposite of what T-023 and `docs/DEV-SETUP.md` say.)*
- **`actions/checkout@v4` is on borrowed time (T-000).** Every CI run prints a deprecation warning because the action targets Node 20, which GitHub is phasing out; the runner forces it onto Node 24. Harmless today, but a future task should bump to a Node-24-native checkout major version — in the same PR as whatever else touches `ci.yml`, never alone.
- **The Cargo cache is written once and then never refreshed (T-000).** `actions/cache` saves only on a miss, and the key is the hash of `Cargo.toml` and `Cargo.lock` alone. Every push within a task shares that key, so the first run's `target/` is what every later run restores and nothing updates it until a dependency changes. Invisible today, because the bootstrap crate has no dependencies and an empty `target/` costs nothing to restore. From T-001 it means the cache stops doing the one thing it was added to do, and the symptom is slow builds rather than a failure. `Swatinem/rust-cache` handles this correctly; the manual equivalent is a run-id suffix in the key plus `restore-keys` for the fallback.
- **"Warm pipeline under 10 minutes" has not really been measured yet (T-000).** The 51 s is `cargo build` over `fn main() {}` with zero dependencies. It satisfies the criterion as written and tells you nothing about the budget once Tauri, WebView2 and the frontend toolchain are in the tree. Nor is it enforced: `timeout-minutes` is 30 on both jobs. Re-measure at T-001 and decide then whether the number deserves a gate — a hard 10-minute timeout would also kill legitimate cold-cache runs, so the honest options are a soft check on the warm path or nothing.
- **`jobs.gates` is reusable by convention only (T-000).** There is no `workflow_call` and no composite action; later tasks extend it by editing the file. That is what T-000 asked for, but the comment at the top of `ci.yml` says "deliberately reusable", which in Actions vocabulary means something the file does not do. Either make it callable or reword the comment, before someone writes `uses:` against it and finds out.
- **`scripts/lint-empty.js` goes red the moment `src/` contains a file (T-000).** This is deliberate and documented in the script: it is a tripwire that forces T-001 to wire ESLint in the same PR that adds the first source file, rather than leaving a green lint gate reading nothing. Worth knowing before someone diagnoses the red as a bug and deletes the check.

---

## Facts established

Discoveries that later tasks depend on and that no document predicted. This is the section that saves a future session from re-learning something the hard way.

### F-001 — GitHub Actions pwsh steps propagate the last native exit code (T-000)

GitHub Actions appends `exit $LASTEXITCODE` after every pwsh `run:` block. A step that deliberately runs a command expected to fail (e.g. `cargo clippy` on broken code) **fails the step even after a successful `$LASTEXITCODE` check**, unless the script ends with an explicit `exit 0`. Caught in the T-000 demo job (run 30928203565): the "Expect clippy to fail" step printed "clippy failed as expected (exit 101)" and then failed with exit code 1. **T-025's `scripts/probe-router.ps1` steps must end each probe with an explicit exit code** when a command is expected to fail.

Also observed: `windows-latest` currently resolves to Windows Server 2025 (image `windows-2025-vs2026`, VS 2026 build tools preinstalled), and `actions/checkout@v4` triggers a Node-20 deprecation warning — it runs on Node 24.

### F-000 — Preliminary evidence on the registration channel — **NOT YET CONFIRMED**

Gathered from documentation and upstream discussions during the v6.1 review, **not** from an `--help` capture. Recorded so T-023 starts from a hypothesis instead of from nothing. **Do not build on it before T-023 confirms it against a real binary.**

- **`--models-preset` appears able to declare a model by absolute path.** Upstream documentation describes the router discovering models from more than one source — the `--models-dir` scan *and* the preset INI defining specific models — and a maintainer answer in an upstream discussion advises keeping models outside the scanned directory and referencing them by absolute path in the preset. If confirmed, `registration_channel` is `PresetDeclaresPath` and the entire `ScanOnly` branch, with its link primitives and privilege dependency, never runs.
  - Sources: `github.com/ggml-org/llama.cpp/discussions/21805`, `deepwiki.com/ggml-org/llama.cpp/6.3-router-mode-and-model-management`

- **Under `--models-dir` the directory must be flat, and subdirectories have a reserved meaning.** A subdirectory is how a *single* multi-file model is expressed — multimodal or multi-shard — not how a library is organised. The scanner treats every subdirectory as a candidate model, there is no exclude flag, and a folder created for tidiness produces phantom entries. Scan depth is limited to one level below the scanned directory; an upstream feature request to increase it is open.
  - Sources: `github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md`, `github.com/ggml-org/llama.cpp/discussions/21805`, `github.com/ggml-org/llama.cpp/issues/23050`

- **The mmproj file is identified by a filename prefix.** In a multi-file model directory the projector file's name must begin with `mmproj`. This is a naming convention, not a flag, and it applies regardless of which registration channel is in use.
  - Source: `github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md`

- **Adding a model requires a server restart.** Consistent with the no-hot-reload decision already in `docs/CONTRACTS.md` §2; noted because it means the restart banner is not a self-imposed limitation.

<!-- Required entries, by the task that produces them:
- T-023: RuntimeBuild.registration_channel for the pinned build — PresetDeclaresPath | ScanOnly | Undetermined. T-033 branches on this and must not start without it.
- T-023: which flags in docs/LLAMACPP.md turned out absent or retyped.

Other examples of what belongs here:
- ts-rs 12.x: multiple types sharing one `export_to` target do / do not produce a single clean file (T-002)
- axum/hyper default idle timeout that must be overridden for long model loads (T-042)
- WebView2 version X breaks <thing>; pinned workaround in <file>
-->

---

## Decisions taken in the v6.0 review

Recorded here so a fresh session does not reopen them. The reasoning lives in `PLAN.md`; this is the index.

| Decision | Where | Effect |
|---|---|---|
| The app owns the client-facing port and reverse-proxies to one llama-server | `PLAN.md` §2.7 | New T-042; `AGENTS.md` invariant 3 rewritten; `ServerConfig` and `ServerState` changed |
| Blackwell requires CUDA 13 — corrected reasoning, same rule | `PLAN.md` §2.2 | T-021 message names the silent-CPU-fallback risk |
| NVFP4 is labelled experimental, never asserted; memory profile still modelled | `PLAN.md` §2.6 | T-031, T-032, T-034, T-035 |
| Model identity is the absolute path; `sha256_head` is a duplicate signal | `PLAN.md` §2.13 | T-031, schema |
| `Backend` is open (`Cuda { major }`) | `PLAN.md` §2.13 | T-020, T-021, T-002 |
| Junctions permitted under the app's data directory only, as the scan-only fallback | `PLAN.md` §2.1 | T-029, T-033 |
| `docs/verified-flags.md` is script-generated, not app-written | `docs/DEV-SETUP.md` | T-023 |
| Preset generator moved into Milestone C | `docs/TASKS.md` | Renumbering map |
| Per-model `enabled` dropped; settings and calibration retained by path on removal | `PLAN.md` §2.9 | T-003, T-031, T-033, T-034, schema |
| Closing the window hides to tray; quit stops the server first | `PLAN.md` §2.10 | New T-047, T-063, T-065 |
| `Running` means preloads are in VRAM; traffic forwarded during preloading | `PLAN.md` §2.12 | T-023, T-040, T-044, T-045; `StartupPhase`, two timeouts |
| Estimation model written down, with a declared 12% conservative margin | `docs/CONTRACTS.md` §1 | T-030 extracts head counts, T-032 implements the stated model |
| Documentation consistency linted in CI, blocking | `docs/TASKS.md` T-006 | New T-006 |
| Endpoint task split: T-042 transport, T-044 state behaviour | `docs/TASKS.md` | Renumbering map, v6.2 |
| Endpoint imposes no concurrency limit by default; never queues | `PLAN.md` §2.13 | `ServerConfig`, T-042 |
| Endpoint latency budgeted and regression-tested in CI | `docs/TASKS.md` T-042 | p99 < 5 ms non-streaming, < 15 ms to first byte |
| Log export redacted like the crash bundle | `PLAN.md` §6 | T-046 |
| `LLAMACPP.md`, `owner-verification.md`, ADR-001 and ADR-002 written | `docs/` | Closes the four documents cited but absent |

## Decisions taken in the v6.3 review

A consistency pass, not an architectural one. Nothing in §2 of `PLAN.md` changed.

| Decision | Where | Effect |
|---|---|---|
| The empirical router questions are a task, not an owner check | `AGENTS.md` §3, `PLAN.md` §2.1 §4 | New T-025; `docs/owner-verification.md` Session 1 removed; T-033 now depends on T-025 |
| T-025's observation overrides T-023's reading of `--help` on `registration_channel` | `PLAN.md` §4 | Two consecutive gates instead of one |
| Every IPC command and event names the task that implements it | `docs/CONTRACTS.md` §4 | New column; T-006 check 5 rewritten — 37 of 41 commands were owned by no task |
| Milestones are an ordering, not a schedule in developer-weeks | `PLAN.md` §4 | Week estimates removed; the review queue named as the real critical path |
| `LinkCapability` defined | `docs/CONTRACTS.md` §1 | Was used by T-029 and defined nowhere; added to T-002's round-trip list |
| The estimator's degradation and its recommendation procedure are specified | `docs/CONTRACTS.md` §1 | T-032 gains criteria; `head_dim` no longer undefined when a field is absent |
| `idle_unload_seconds` has no known flag | D-003 above | T-050 hides the control unless the verified list has it |
| Path round-tripping restated as idempotence plus identity on normalized paths | `docs/TASKS.md` T-029 | The v6.2 criterion was unsatisfiable on Windows |
| The endpoint latency budget has a stated statistic | `docs/TASKS.md` T-042 | Median of five batch p99s, so a shared runner cannot quietly erode the number |
| "Junction" no longer used to mean "link" | `PLAN.md` §4 §6, T-031, T-064 | §2.1 establishes a junction cannot link a file; the rest of the documents now agree |
| Applying an owner decision is a numbered task, not prose | `docs/WORKFLOW.md` §7 | `T-1xx` range reserved; §6's prohibition narrowed from absolute to "not on your own initiative"; T-006 check 8 |
| This file is pruned at milestone boundaries, and Facts established never is | Top of this file | Pruning is a correction task; the context cost of an unpruned PROGRESS is paid every session |
