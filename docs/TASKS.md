# Tasks

Every task is executable by the agent. Every acceptance criterion is checkable in CI or on a GPU-less development machine using fixtures, mocks, stubs and synthetic data. There are no tasks assigned to a human — see `AGENTS.md` §3.

Set your machine up first: `docs/DEV-SETUP.md`. No GPU, no CUDA toolkit and no real model files are needed. A CPU build of llama-server is downloaded by T-023 itself to capture `--help`, and reused by T-025 and T-043.

Conventions: `[dep: …]` lists prerequisites. **The `T-1xx` range (100 and above) is reserved for correction tasks** — document changes commissioned by the owner to apply a decision taken on a discrepancy (`docs/WORKFLOW.md` §7). They live in the Corrections section at the end of this file, sort after every milestone task, and therefore never displace feature work. Contracts referenced here are defined in `docs/CONTRACTS.md`. Flags are in `docs/LLAMACPP.md` and must be checked against `docs/verified-flags.md` before use.

## Renumbering in v6.0

Two structural changes: the preset generator moved from Milestone D into C, because two Milestone C screens assert byte-identical output against it; and the endpoint listener (`PLAN.md` §2.7) is new. If you find an old number in a branch name or an older document, this is the map:

| v5.0 | v6.0 | |
|---|---|---|
| T-040 preset generator | **T-033** | moved into Milestone C |
| T-033 models screen | **T-034** | |
| T-034 model detail | **T-035** | |
| T-041 supervisor | **T-040** | |
| T-042 orchestrator | **T-041** | |
| — | **T-042** endpoint listener | new |
| — | **T-047** tray, quit and session end | new |

Two further changes in v6.2:

| v6.0 | v6.2 | |
|---|---|---|
| — | **T-006** documentation lint | new |
| T-042 endpoint listener | **T-042** transport + **T-044** state behaviour | split: two properties, two PRs |
| T-043 diagnostics | **T-043** | unchanged |
| T-044 dashboard | **T-045** | |
| T-045 logs screen | **T-046** | |
| T-046 tray | **T-047** | |
| T-046 diagnostics | **T-043** | moved before the dashboard, which renders it |
| T-043 dashboard | **T-045** | |
| T-045 logs screen | **T-046** | |
| T-052 optional gateway | **T-051** auth and request log | no longer optional |
| T-051 client contract tests | **T-052** | now last in E, so it covers auth too |
| T-070 settings | **T-063** | was numbered after tasks that depend on it |
| T-063 installer | **T-064** | |
| T-064 E2E | **T-065** | |

One change in v6.3:

| v6.2 | v6.3 | |
|---|---|---|
| — | **T-025** empirical router probe | new: the questions a CPU build can answer, moved out of `docs/owner-verification.md` |

One change in v6.4:

| v6.3 | v6.4 | |
|---|---|---|
| — | **T-036** speculative decoding (MTP and draft models) | new: owner-commissioned on 11 Sept 2026 while implementing T-035 — draft-architecture files (DFlash, DSpark, EAGLE) must not enter the catalogue as launchable models, and MTP models must be launchable with `--spec-type draft-mtp` |

---

## Milestone A — Skeleton

### T-000 — CI pipeline
GitHub Actions on `windows-latest`: `cargo build`, `cargo test`, `cargo clippy -- -D warnings`, `cargo fmt --check`, `npm run lint`, `npm run test`. Cache Cargo and npm. Expose a reusable job so later tasks can add steps.

**The pipeline needs something to run against, and T-001 has not happened yet.** So this task also creates the **minimum** that makes those six commands meaningful: a `Cargo.toml` with an empty `src-tauri` crate containing nothing but `fn main() {}`, and a `package.json` with the `lint` and `test` scripts wired to tools that exit zero on an empty source tree. Minimum is the operative word — no Tauri, no React, no Tailwind, no dependency beyond what the commands themselves need. T-001 replaces this with the real scaffold and is entitled to overwrite every line of it. A gate that runs against nothing proves nothing, and a gate added after the code it is supposed to guard has never once caught the thing it was built for.

*Acceptance:* A PR with a clippy warning fails. A PR with unformatted code fails. **Both are demonstrated rather than asserted** — open a throwaway branch carrying a deliberate warning, confirm red, and record the run in the PR description; a pipeline whose failure path has never executed is a pipeline nobody has tested. Warm pipeline under 10 minutes. **No type-generation step yet** — T-002 adds it. The bootstrap crate and `package.json` contain no dependency that `docs/DEV-SETUP.md` does not already require.

### T-001 — Scaffold `[dep: T-000]`
Tauri 2 + React + TS + Tailwind, layout per `PLAN.md` §3. `tracing` writes to `%LOCALAPPDATA%\LlamaManager\logs\app.log` with rotation.

**Replace T-000's bootstrap crate and `package.json` outright.** They exist only so the gates had something to run against; nothing in them is worth preserving, and carrying one of their lines forward by accident is how a placeholder reaches release.

Create the repository configuration listed in `docs/DEV-SETUP.md`: `rust-toolchain.toml` pinning 1.88 with the MSVC target, `.cargo/config.toml` setting `TS_RS_EXPORT_DIR`, and a `.gitignore` covering `target/`, `node_modules/`, `dist/` and generated fixtures.

*Acceptance:* `cargo test` and `npm run test` pass with zero tests. `npm run tauri dev` opens a window. Clippy clean. Log file created on launch. A clone on a machine set up per `docs/DEV-SETUP.md` reaches a running window with `npm install` followed by `npm run tauri dev` and no further steps — if it does not, fix the document in the same PR.

### T-002 — Type generation `[dep: T-001]`
Generate TS types from the Rust structs in `docs/CONTRACTS.md` §1 using **`ts-rs` 12.x** — the choice is settled in `docs/adr/001-type-generation.md`; do not re-evaluate it. Derive `TS` alongside `Serialize`/`Deserialize` on every contract type, export them into a single `src/lib/types.ts`, and add the freshness check to the CI job created in T-000.

`src/lib/ipc.ts` is hand-written on top of the generated types: one thin wrapper per command in `docs/CONTRACTS.md` §4, importing types rather than redeclaring them.

*Acceptance:* `npm run generate-types` regenerates `src/lib/types.ts`; CI fails when it is stale. All contract types land in that one file with no duplicate declarations — if directing several types to a shared `export_to` target does not produce that, a collector script concatenates them and the freshness check covers the result. **Every enum round-trips:** for each of `FlashAttn`, `Compatibility`, `ServerState`, `EndpointState`, `InstallProgress`, `ImportProgress`, `ModelAvailability`, `LinkCapability`, `Backend`, `RegistrationChannel`, `CheckStatus`, `ModelLoadState`, `EstimateConfidence` and `AppError`, a test serializes a Rust value with `serde_json` and asserts the output satisfies the generated TypeScript type — asserting against what serde actually emits, not what the derive appears to promise. `Backend::Cuda { major }` in particular must round-trip for a major the code has no branch for. A wrapper in `ipc.ts` that declares a type inline instead of importing it fails a lint rule.

### T-003 — Database layer `[dep: T-001]`
`db/` with the schema in `docs/CONTRACTS.md` §3, forward-only migrations, `schema_version` table, `PRAGMA foreign_keys = ON` on every connection.

*Acceptance:* Migrations run idempotently on a fresh and on an already-migrated database. `launch_history` and `retained_model_settings` survive deletion of the corresponding `models` row, asserted directly — they are keyed by path and carry no foreign key (`PLAN.md` §2.9). A database whose `schema_version` exceeds the binary's known version yields `AppError::SchemaTooNew` and a clean refusal to start — not a crash, not a backward migration. CRUD covered per table against in-memory SQLite. A test asserts `foreign_keys` is on and that deleting a model cascades its `launch_history` rows. A test asserts a second `is_active` runtime is rejected by the unique index, not by application code. A test asserts `schema_version` cannot hold two rows.

### T-004 — Error model `[dep: T-001]`
One `AppError` enum (`thiserror`) per `docs/CONTRACTS.md` §1, carrying `code()`, user-facing `message()`, optional `remediation()`.

*Acceptance:* A clippy lint in CI rejects `unwrap()` and `expect()` in `core/` and `ipc/` outside tests. Every variant has a test asserting its serialized shape. No variant's `message()` is empty. `code()` values are unique across variants, asserted by a test that enumerates them.

### T-005 — Design system `[dep: T-001]`
Dark-first theme, single accent colour, shadcn primitives, app shell with sidebar and seven empty routes. All strings in `src/lib/strings.ts`.

The visual target is LM Studio's language, reimplemented from scratch (`PLAN.md` §2.5): dark-first palette with one accent, high information density, restrained motion, controls that expose real parameters rather than hiding them behind simplified toggles. **Take no assets, icons, CSS, or code from LM Studio — it is closed source.** Define the palette, spacing scale and type scale as tokens in one file so the whole app inherits them.

*Acceptance:* Seven routes navigable. No unstyled default HTML. Colours, spacing and type sizes come from tokens — a lint rule rejects hard-coded hex values and arbitrary pixel spacing in components. The loading component's label prop is **required** — a bare spinner is a TypeScript error, asserted by a type-level test. A lint rule rejects string literals in JSX text positions.

### T-006 — Documentation lint `[dep: T-000]`
A script, run in CI as a blocking check, that verifies the documents are internally consistent. They are the agent's specification: a stale cross-reference is not a typo, it is a wrong instruction that will be followed.

Checks, all mechanical:

1. Every `T-xxx` cited in any document exists in `docs/TASKS.md`, **except inside the renumbering tables at the top of this file**, whose left-hand columns exist precisely to name numbers that no longer resolve. The exemption is by section, not by pattern: a retired number anywhere else is the stale cross-reference this check exists to catch.
2. Every task's dependencies exist and are strictly lower-numbered — this is what makes the selection rule in `docs/WORKFLOW.md` §2 terminate.
3. Every `PLAN.md §x.y`, `AGENTS.md §n` and `CONTRACTS.md §n` reference resolves to a real heading.
4. Every capitalised type name used in backticks outside `docs/CONTRACTS.md` is defined in it, or is on a short allow-list of external names.
5. Every row of the IPC command table and the event table in `CONTRACTS.md` §4 names a task that exists in `docs/TASKS.md`. **This checks the `Implemented by` column, not the task prose.** Requiring each command to be mentioned in a task's text was the v6.2 formulation and it was wrong twice over: thirty-seven of forty-one commands failed it, and passing it would have meant padding acceptance criteria with names rather than assigning ownership.
6. Milestone task ranges in `PLAN.md` §4 match the tasks actually present in each milestone in `docs/TASKS.md`. Tasks in the Corrections range are excluded — they belong to no milestone by design.
7. Every `T-xxx` in `docs/TASKS.md` appears in exactly one milestone section **or in the Corrections section**, and every `AGENTS.md` invariant is cited as "invariant N" rather than "§6.N" — the invariants are list items and check 3 resolves headings only.
8. Every discrepancy in `PROGRESS.md` marked `resolved` **by a correction task** names a `T-1xx` that exists, and every `T-1xx` names a discrepancy that exists. This is the check that keeps the mechanism in `docs/WORKFLOW.md` §7 honest: a decision recorded as applied with nothing behind it looks exactly like a decision that landed, and is the failure mode the correction range was created to prevent.
9. Every file path cited in any document exists **with that exact case**. Compare against a directory listing character by character — not with an existence test. NTFS and `windows-latest` are both case-insensitive, so `os.path.exists` would return true for `docs/workflow.md` and the check would pass forever while GitHub's rendered view served a 404 and git, with its default `core.ignorecase`, quietly treated a case-only rename as no change at all. A check that asks the operating system whether a path works inherits the operating system's blind spot; this one asks what the directory actually contains. `docs/verified-flags.md` is exempt until T-023 generates it.

**Blocking.** A documentation inconsistency fails the build like a clippy warning does. The cost of a false positive is editing one line; the cost of a false negative is an agent implementing a specification that contradicts itself, which is the more expensive of the two by a wide margin.

*Acceptance:* The lint runs in the T-000 pipeline and fails the build on any violation. Each of the nine checks has a test that introduces a deliberate violation into a fixture document set and asserts the specific check catches it, with a message naming the file, the line and what is wrong — a lint that reports only "documents inconsistent" will be disabled by the first person who hits it. Running it against the repository as committed produces zero findings. The allow-list for check 4 is a named file, not a regex buried in the script. **Seed it with the names already known to trip the check** — `Blocked` and the other task-status words, Win32 symbols such as `DeviceIoControl` and `FSCTL_SET_SPARSE`, errno names such as `ECONNREFUSED`, and `WM_QUERYENDSESSION` — because a check whose first run against clean documents produces eight false positives teaches its reader that its output is noise, and that lesson is not unlearned later.

---

## Milestone B — Runtime

### T-010 — EnvironmentProbe `[dep: T-003, T-004]`
GPU detection via the `NvmlProvider` trait (`docs/CONTRACTS.md` §6), system RAM, free disk, OS build, `HealthCheck` entries.

*Acceptance:* Against a mocked provider: returns correct reports for a Blackwell profile, an Ada profile, a driver-too-old profile, and a no-GPU profile. No-GPU yields `gpu_present = Fail` with remediation and no panic. NVML failing mid-call is handled, not propagated as a panic. Every `HealthCheck` with status Warn or Fail has non-empty remediation. The `endpoint_bindable` check reports whether the configured listen port can be bound, without holding it.

### T-011 — Health-check screen `[dep: T-005, T-010]`
Traffic lights, remediation text, manual refresh. **This is the first-run view of the Runtime route, not an eighth route** — T-005 defines seven and that number does not change here.

*Acceptance:* Each `CheckStatus` renders distinctly (asserted by component tests over fixture reports). Remediation shown for Warn and Fail. Refresh triggers a fresh probe.

### T-020 — GitHub Releases client `[dep: T-004]`
Fetch releases, parse asset names into `(build_tag, Backend)`, expose the newest per backend, handle rate limiting and offline.

*Acceptance:* Table-driven tests over at least 12 real asset names including ones that must be rejected. **An asset naming a CUDA major the code has no branch for parses into `Backend::Cuda { major }` rather than failing** — the enum is open (`PLAN.md` §2.13). Offline returns a typed error. A rate-limit response returns `AppError::RateLimited` carrying the reset time. Malformed JSON returns a typed error, never a panic.

### T-021 — Backend selection `[dep: T-010, T-020]`
Pure function from `EnvironmentReport` plus available releases to a chosen asset.

Rules, per `PLAN.md` §2.2: **Blackwell (compute capability ≥ 10.0) requires the CUDA 13 build. Never select a lower CUDA major for Blackwell, and never with a warning either** — the failure being avoided is a silent fallback to CPU inside the binary, which no warning can recover from once inference has started. Non-Blackwell NVIDIA prefers the highest available CUDA major and may fall back. No NVIDIA GPU or driver too old yields Vulkan; CPU is last. Vulkan and CPU selections carry a warning that performance will be poor.

*Acceptance:* No I/O. Table-driven tests over at least 10 hardware profiles including Blackwell with CUDA 13 available, **Blackwell with only CUDA 12 available (a hard failure with a message naming the silent-CPU-fallback risk, not a downgrade and not a warning)**, Blackwell with a CUDA major above 13 available, an old driver, and no GPU. Property test: the function never returns a CUDA build for a driver that cannot run it, and never returns a CUDA major below 13 when any GPU reports compute capability ≥ 10.0.

### T-022 — Runtime installer `[dep: T-020, T-021]`
Download with progress events, SHA-256 verification, extraction, registration. Atomic.

*Acceptance:* Against a local HTTP server serving fixture archives — an interrupted download leaves no partial install and reports a clean error; checksum mismatch aborts before extraction; re-installing an existing build is a no-op; disk-full during extraction rolls back. **Zip-slip rejected:** an entry whose normalized destination escapes the target (`../`, absolute path, drive letter, or a symlink entry) aborts the entire extraction with `AppError::UnsafeArchiveEntry` — covered by a malicious fixture archive. A release without a published checksum surfaces a warning rather than skipping verification silently.

### T-023 — Flag verification and registration channel `[dep: T-022]`
Run `llama-server.exe --help`, parse flags **and value types** into `Vec<VerifiedFlag>`, store per build in `runtimes.verified_flags_json`.

**This task also records which health endpoint the build offers** — `/health` where it exists, otherwise `/props` — and stores it with the verified flag list. T-040's transition out of `Starting` depends on it and must not guess.

**This task also settles `PLAN.md` §2.1.** Determine from the help text whether `--models-preset` can declare a model by absolute path, and record the result as `RuntimeBuild.registration_channel`. If the text does not answer it, record `Undetermined` — do not infer.

**Capturing the fixtures is part of this task.** Download a CPU build of llama.cpp (no GPU needed), run `--help`, and commit the capture. Do this for at least three builds of different ages so the parser is tested against real formatting variation rather than against its own assumptions. Do not hand-write a fixture that has never been produced by a real binary.

`docs/verified-flags.md` is **not** written by the app. `scripts/export-verified-flags.ps1` renders it from the database for the pinned build, and the result is committed. An app that writes into its own source tree is a bug, not a feature.

*Acceptance:* Parser tested against at least three captured `--help` fixtures from different builds; correctly distinguishes boolean flags, value-taking flags, and enumerated values. Every flag named in `docs/LLAMACPP.md` is checked; missing or retyped flags produce a UI warning naming the specific flag. `registration_channel` is derived from the same capture and asserted for each fixture, including one that yields `Undetermined`. Parsing an unrecognized help format degrades to an empty verified list plus a warning — never a panic, never a false positive. The export script's output is byte-stable for a given database state, asserted by a snapshot.

**Before closing this task, record the `registration_channel` result in the Facts established section of `PROGRESS.md`.** T-033 branches on it and must not guess.

### T-024 — Version management UI `[dep: T-011, T-022, T-023]`
Builds side by side, active marked, install/activate/remove, current vs latest, release notes link.

*Acceptance:* Switching the active build takes effect on next start. Activate and remove are rejected unless state is Stopped, with the reason displayed. Removing the last remaining build is blocked. A build whose `registration_channel` is `Undetermined` is flagged in the UI.

### T-025 — Empirical router probe `[dep: T-023]`
Run the CPU build T-023 already downloaded, in router mode, against prepared directories, and **answer by observation what T-023 could only infer from `--help`**. Five questions, all reachable without a GPU, a real model file, or a target machine — which is why they are a task and not an owner check (`AGENTS.md` §3, `PLAN.md` §2.1).

**T-025 makes its own fixture, and this is a real constraint rather than a preference.** `scripts/make-gguf-fixture.py` is written in T-030, which runs later — a probe that depended on it would invert the ordering the whole selection rule rests on. Write the minimum file the router will accept: registration is not loading (`PLAN.md` §2.1), so a valid header with no usable tensor data is the hypothesis being tested, and **whether the router accepts such a file is itself one of the probe's findings.** If it rejects it, that is a Discrepancy naming the rejection, not a reason to reach forward for a task that does not exist yet. Whatever minimal writer this task produces is throwaway: T-030 supersedes it, and T-030 does not build on it.

1. **Does `--models-preset` register a model by absolute path?** Write a preset naming a `.gguf` outside any scanned directory; start with the preset and no `--models-dir`; query `/v1/models`. Record whether it appears, the exact syntax that worked, whether a relative path also works, and what a non-existent path does.
2. **Does the `--models-dir` scan follow reparse points?** Scan a directory holding a real file, a file symlink to another volume, and a junction to a shard directory. Record the three outcomes separately — they are different mechanisms.
3. **What is the scan's real depth, and what does a non-model subdirectory produce?** A model at top level, one a level down, one two levels down, and an empty directory alongside.
4. **How is a projector expressed on the `PresetDeclaresPath` channel?** T-033 must emit a vision model on both channels; the `mmproj` filename convention is documented for the scan only.
5. **Which health endpoint is a reliable proof of life with no model loaded?** Request `/health`, `/props` and `/models` against the running router before anything is loaded; record status codes and shapes. T-023 records which endpoint the build *offers*; this records which one actually answers in the state T-040 polls it in — an endpoint that only responds once a model is resident would make the `Starting → Running` transition wrong in exactly the case that matters.

**Where a question cannot be answered on this machine, say so and stop there.** The plausible cases are known: the router may reject a synthetic GGUF whose tensor regions are sparse zeroes, and creating a file symlink for question 2 needs Developer Mode or Administrator. Either produces a Discrepancy naming the specific obstacle, and *that* is what goes to the owner — not the whole question set on the assumption it was always theirs.

**This task overrides T-023 on `registration_channel`**, including when they agree: observed behaviour beats an inference from help text. Update `runtimes.registration_channel` for the probed build and record the result in Facts established.

*Acceptance:* Each probe runs from a committed script under `scripts/probe-router.ps1` and writes a machine-readable result file, so a rerun on a different build is one command rather than a repeat of the reasoning. Every answer lands in Facts established with the command that produced it and the raw response quoted, or in Discrepancies with the specific obstacle named — never absent, never inferred. `registration_channel` is written from the observation, and a test asserts the stored value matches the recorded probe result. Where question 1 answers yes, the recorded preset syntax is the syntax T-033 emits, asserted by a snapshot shared between this task's result file and T-033's generator. A probe that cannot run at all fails loudly with the reason, and does not silently record `Undetermined`.

**Before closing this task, confirm that `PROGRESS.md` F-000 is either promoted to confirmed or contradicted in writing.** It has carried a **NOT YET CONFIRMED** marker since the v6.1 review and this is the task that removes it.

---

## Milestone C — Catalogue

### T-029 — Model paths and availability `[dep: T-003, T-004, T-025]`
Own everything about a model's absolute path: normalization, validation, escaping for the preset file, availability tracking, and — only under the scan-only fallback of `PLAN.md` §2.1 — creation and removal of **links** inside the app's own data directory.

**No directory the user chose is ever scanned on the router's behalf, and no file is ever copied or moved.** A link is created only under `%LOCALAPPDATA%\LlamaManager\`, points outward at the user's file, and is removed with the entry that created it. The primitive depends on what the machine allows: file symlink, else same-volume hard link, else a typed failure. **A junction is never used for an individual model file** — it cannot link one (`PLAN.md` §2.1); junctions appear only where a whole multi-file model directory is linked.

**The dependency on T-025 is deliberate.** Half this module — `link_into`, `unlink`, `link_capability`, `LinkCapability` — exists only for the `ScanOnly` channel, and `PLAN.md` §2.1 says that if the preset can declare paths, that branch "never runs". Building it before knowing would be writing a module whose larger half may be dead on arrival. **Whether to implement it anyway is the owner's call, not yours** — see the Observation in `PROGRESS.md`. Absent a decision, implement the full surface: an unused function is cheaper than a missing one discovered in Milestone C.

```rust
pub fn normalize(path: &Path) -> Result<PathBuf, AppError>;   // canonical, absolute
pub fn preset_literal(path: &Path) -> String;                 // escaped for presets.ini
pub fn probe(path: &Path) -> ModelAvailability;               // Present | Missing | Unreadable
pub fn link_into(app_dir: &Path, target: &Path, name: &str) -> Result<PathBuf, AppError>;
pub fn unlink(link: &Path) -> Result<(), AppError>;
pub fn link_capability(app_dir: &Path) -> LinkCapability;   // Symlink | HardLinkOnly | None
```

`link_capability` **probes by attempting a symlink in a scratch directory and deleting it**, not by reading a registry key or checking a token privilege — those report what should work, and the question is what does. `link_into` picks the strongest primitive available for the given target: symlink, else hard link when target and app directory share a volume, else a typed error naming Developer Mode as the remedy. A directory junction is never used for an individual model file; it cannot link one (`PLAN.md` §2.1).

*Acceptance:* Path handling is asserted as **two separate properties**, because the v6.2 formulation — "normalize then `preset_literal` then parse back yields the original path" — is false by construction on Windows: `normalize` resolves reparse points, may prefix `\\?\`, and the filesystem strips trailing dots, so the original string does not survive and a test demanding it can only be satisfied by not normalizing. Instead: (a) **`normalize` is idempotent** — `normalize(normalize(p)) == normalize(p)` — over paths containing spaces, non-ASCII characters, trailing dots, UNC prefixes, and a second volume, asserted by `proptest` over generated fragments; (b) **`preset_literal` then parse is the identity on already-normalized paths**, over the same corpus. A path whose normalized form differs from its input is not a failure and is asserted to be handled, not rejected. `normalize` on a path that does not resolve — a disconnected volume, a renamed file — returns the lexically absolute form with a typed marker rather than failing, since `probe` must still classify it as `Missing`. A relative path is rejected with `AppError::InvalidPath`. `probe` distinguishes a missing file, a file on a disconnected volume, and a file that exists but cannot be opened, without blocking on an unreachable network path beyond a bounded timeout. `link_into` refuses any destination outside the app directory, asserted by a test passing a user path. `link_capability` returns `HardLinkOnly` or `None` without panicking on a machine lacking symlink privilege, and the three branches of `link_into` are each exercised — the hard-link branch against a same-volume fixture, the failure branch with an error naming Developer Mode. A test asserts no code path creates a junction for a file. `unlink` removes the link and leaves the target untouched, asserted by a checksum of the target before and after. **Nothing in this module opens a file for writing outside the app directory** — asserted by a checksum of the whole fixture tree before and after the test run.

### T-030 — GGUF header reader `[dep: T-004]`
Read the header only — magic, version, tensor count, KV metadata. Never read tensor data. Handle multi-part shards.

**A multi-file model is a set, not a file.** Shards follow the `-NNNNN-of-MMMMM.gguf` naming; a projector file is recognised by a name beginning with `mmproj`, which is a convention rather than anything declared inside the file (`PROGRESS.md` F-000). The reader resolves a set from any member of it and reports what is missing.

Fixtures are **synthetic**: `scripts/make-gguf-fixture.py` writes valid headers followed by sparse zero regions. Never download real models for tests.

**Sparseness must be explicit.** NTFS does not create holes automatically the way ext4 does — the file must be flagged sparse before it is extended (`fsutil sparse setflag`, or `DeviceIoControl` with `FSCTL_SET_SPARSE`), otherwise Windows zero-fills it and the 65 GB fixture consumes 65 GB for real.

**Optional real-model tests.** Tests guarded by the `LLAMA_MANAGER_REAL_GGUF` environment variable parse a real model file when a developer has one. They exist because the generator and the parser share one understanding of the format: if it is wrong, both agree and the synthetic suite passes anyway. A real file is the only thing that catches that. CI does not set the variable.

*Acceptance:* Parses fixtures for at least four architectures, including one MoE and one NVFP4. **`attention_head_count` and `attention_head_count_kv` are extracted where the header declares them and left `None` where it does not** — they carry the KV cache term of the estimator, and a Grouped Query Attention fixture where the two differ by 8× is included specifically to catch a parser that reads one and copies it into the other. Reads the 65 GB sparse fixture's header in under 500 ms and never allocates proportionally to file size (asserted by a peak-memory check). **After generation, the 65 GB fixture's allocated size is under 10 MB** — asserted by a test reading its on-disk allocation, so a generator that fails to set the sparse flag fails here instead of filling the disk. With `LLAMA_MANAGER_REAL_GGUF` set, the real file parses and its metadata is self-consistent (declared layer count matches the tensors present, quantization is a known value, architecture is non-empty); with the variable unset those tests skip with an explicit message and never fail. Truncated, corrupt, and wrong-magic files return `AppError::GgufParse`. Incomplete shard sets are detected and named — the error says which indices are absent, not merely that the set is broken. A directory holding shards plus a file whose name begins with `mmproj` resolves to one model with a projector, asserted over a fixture set; a file named `mmproj` alone, with no companion model, is reported rather than treated as a model. `proptest` over mutated headers produces no panic and no unbounded allocation.

### T-031 — Model registry `[dep: T-003, T-023, T-029, T-030]`
Import individual files or whole folders, persist, list, rescan, remove. **Removal retains the model's parameters, sampling defaults and launch history, keyed by absolute path** (`PLAN.md` §2.9); re-importing the same file restores them. Watched folders are folders the user asked the app to remember; rescanning one picks up newly added files. Watching is a convenience for the catalogue only — it never affects what the router sees. Compute `Compatibility` against the active build.

**Identity is the absolute path** (`PLAN.md` §2.13). Importing the same path twice updates the existing entry. Importing a *different* path whose `sha256_head` matches an existing entry creates a second entry with `duplicate_of` set and surfaces it in the UI — two copies of one model on two drives are two models, and the user decides what to do about it.

`served_name` is derived from the display name and made unique across the catalogue, so two files with the same basename on different drives are separately addressable.

Import runs as a **background queue, parallelism 4, cancellable, per-file progress** via `import-progress` events.

*Acceptance:* **A multi-file model imports as a single entry**: a shard set, and a directory holding a model plus an `mmproj`-prefixed projector, each produce one `ModelEntry` whose `shard_paths` and `mmproj_path` are populated from the set — never one entry per file. Importing the same path twice creates one entry. **Removing an entry and re-importing the same file restores its `LaunchParams`, `SamplingDefaults`, `preload`, `pinned` and its `launch_history`, asserted end to end including that the estimator returns `Calibrated` rather than `Heuristic` after the round trip.** A *different* file placed at the same path has its retained settings discarded on the `sha256_head` mismatch, not applied. Importing two different paths with identical content creates two entries, the second carrying `duplicate_of`. Non-GGUF import fails cleanly with no entry created. Removing an entry never deletes the file, and removes any link it created — symlink, hard link or, for a multi-file model directory, junction. Cancelling mid-import leaves a consistent catalogue with no partial entries. Removing a watched folder unregisters its models and never touches the files on disk. Rescanning a folder whose drive is disconnected marks its models `Missing` and leaves the rest of the catalogue intact. A checksum of the fixture tree is identical before and after every registry operation. Importing 200 synthetic fixtures completes without unbounded memory growth. **NVFP4 models are classified `SupportedWithWarnings` with an `experimental: true` note that states the memory profile is modelled but the speedup is unconfirmed on the active build** (`PLAN.md` §2.6) — the note never claims acceleration is present and never claims it is absent. `served_name` collisions across drives are resolved deterministically, asserted by a test. `remove_model` for a Loading or Loaded model returns `InvalidTransition`.

### T-032 — VRAM estimator `[dep: T-030]`
**Pure function** `estimate(&EstimateInputs, &[LaunchRecord]) -> VramEstimate`. No database access — the caller supplies history, pre-filtered by the model's absolute path per the calibration contract in `docs/CONTRACTS.md` §1.

NVFP4 is modelled like any other quantization: the memory saving follows from the format, so the estimate is produced, with a note that the figure is unvalidated against real hardware.

Implement the model stated in `docs/CONTRACTS.md` §1, "The estimation model" — including its constants. Do not substitute a different formulation: the snapshots are what let the owner's empirical check attribute an error to the model rather than to the implementation.

*Acceptance:* Deterministic — same inputs and same history produce identical output, asserted by `insta` snapshots over at least eight configurations including one NVFP4 model and one Grouped Query Attention model. **Each term of the stated model is asserted separately** against hand-computed values for one configuration, so a compensating error in two terms cannot hide behind a plausible total. **With `attention_head_count_kv` absent the estimator does not assume it equals `attention_head_count`**: it applies the architecture default, records the assumption in `notes`, and widens the margin — asserted by a test comparing the output against the same fixture with the field present. **`embedding_length` or `attention_head_count` absent degrades per the contract and never divides by a substituted zero** — with an architecture default known, the default is applied, named in `notes`, and the margin widens; with none known, `kv_cache_bytes` is the stated bound rather than a modelled figure, and `notes` says which of the two happened. Asserted separately for each of the four combinations. **`recommended_gpu_layers` is asserted against the scan the contract specifies**: the largest layer count fitting `vram_free_bytes` with the margin applied, asserted against hand-computed values for one configuration, and asserted to be independent of `inputs.params.gpu_layers` — passing a different evaluated configuration changes `estimated_vram` and leaves the recommendation alone. With `params.gpu_layers` set to `None`, `estimated_vram` describes the recommended count and `notes` says so. The conservative margin appears in `notes` in every estimate. Never recommends `gpu_layers` above `block_count`. With zero free VRAM recommends 0 with an explanatory note. With empty history returns `Heuristic`; with history matching the contract's definition returns `Calibrated`; with history for a *different* parameter set returns `Heuristic` but with the bias applied, asserted separately. Property tests: estimated VRAM is monotonically non-decreasing in `gpu_layers` and in `ctx_size`; KV cache size decreases when a quantized cache type is selected; a synthetic history whose actual usage consistently exceeds the heuristic shifts subsequent estimates upward.

Empirical accuracy against real models is not a criterion here — it is an owner check (`docs/owner-verification.md`), fed back as a follow-up task if the model needs correcting.

### T-033 — Preset generator `[dep: T-025, T-031]`
Serialize **every registered model** into `presets.ini`, and generate the router command line via `router_arguments` (`docs/CONTRACTS.md` §5). There is no enable filter — `enabled` was dropped in `PLAN.md` §2.9. Emit only flags in the active build's verified list, except `extra_args`; emit a `preset-warning` event for each flag omitted.

**Branch on `RuntimeBuild.registration_channel`, as established empirically by T-025** — which overrides T-023's reading of the help text:

- `PresetDeclaresPath` — one section per registered model naming its absolute path via `preset_literal` (T-029), **in the exact syntax T-025 recorded as working**, not a syntax inferred from the INI grammar. `--models-dir` is never emitted. A multimodal model's projector is expressed as T-025 question 4 established; if that question returned an obstacle rather than an answer, a vision model on this channel is a Discrepancy, not a guess.
- `ScanOnly` — the app links each registered model into its own data directory via `link_into` (T-029) and emits `--models-dir` pointing there. Sections carry settings only. A model that cannot be linked on this machine is reported, not silently omitted.

  **The link directory must match the scanner's grammar exactly** (`PLAN.md` §2.1): single-file models linked at the top level, one subdirectory per multi-file model containing its shards and its `mmproj`-prefixed projector, and nothing else at any level. A subdirectory that is not a model becomes a phantom entry in `/v1/models`, and there is no flag to exclude it.
- `Undetermined` — **stop.** This now means T-025 ran and still could not tell, which is an owner decision. Record a discrepancy in `PROGRESS.md` and do not proceed. Guessing here produces a config that silently exposes the wrong catalogue.

*Acceptance:* `insta` snapshots over at least eight configurations **in each of the two supported channels**: paths with spaces, non-ASCII paths, a path on a second volume, a UNC path, a vision model with `mmproj`, an MoE model with CPU offload, an NVFP4 model, one with `extra_args`, and one where a modelled flag is absent from the verified list. That last case omits the flag, emits `preset-warning`, and never writes a malformed file. Output is byte-identical to `preview_preset`. A round-trip test parses the generated ini back and confirms every intended setting survived. Adding one model to the catalogue adds exactly one section and changes nothing else, asserted by a diff test. Two models with the same filename on different drives produce distinct sections with distinct `served_name`s. In the `ScanOnly` branch, a test asserts every link lives under the app directory, that generation creates no file outside it, and that a model whose link could not be created produces a warning naming it rather than a silently shorter catalogue. **The generated link directory is asserted against the scanner's grammar**: no directory that is not a model, no nesting below one level, projector files named with the `mmproj` prefix — asserted by walking the tree, including for a catalogue mixing single-file, multi-shard and multimodal models. A build with `Undetermined` causes generation to refuse with a typed error rather than emit anything. `router_arguments` always contains `--no-webui`, `--host 127.0.0.1` and the upstream port it was given, asserted over a table of configurations — the loopback bind is not conditional on any setting (`PLAN.md` §6).

### T-034 — Models screen `[dep: T-005, T-031, T-033]`
List/grid with metadata and compatibility badges, native `.gguf` picker, add-folder flow, preload and pin toggles, availability indicators, duplicate indicators, watched-folder management.

*Acceptance:* 200 fixture models render responsively (virtualized list, asserted by a render-count test). Badges match the backing enum exhaustively — a new `Compatibility` variant fails to compile. A note with `experimental: true` renders visibly distinctly from an ordinary warning. `Missing` and `Unreadable` models are visually distinct, cannot be preloaded, and show their last known path. An entry with `duplicate_of` shows what it duplicates. The two toggles (preload, pinned) are independently settable and their meanings are visible in the UI, not only in a tooltip. **Removing a model states that its settings and calibration are kept and will return if the file is re-imported** — the retention is worthless if the user does not know it happened. Adding or removing a model changes the next generated preset accordingly, asserted against the real T-033 generator; the UI states that the change applies on restart. Empty state gives a clear call to action. Import progress and cancellation are reachable from the UI.

### T-035 — Model detail screen `[dep: T-032, T-033, T-034]`
Launch and Sampling tabs, presets, layer-budget slider with live projection, live preset preview, dirty-config banner.

Presets: Balanced, Max context, Max speed, Low VRAM, Custom.

*Acceptance:* Slider updates are debounced and never block the render thread. The preview is produced by **the same code path as T-033** and is asserted byte-identical to `preview_preset`. The Sampling tab shows visible helper text — not a tooltip — stating that client requests override these defaults. The `extra_args` field is labelled unvalidated with its warning. The projector path is shown as resolved from the model's file set rather than offered as a free path field — it is discovered by naming convention, not chosen (`PROGRESS.md` F-000). An NVFP4 model's experimental note is shown on the Launch tab, not only in the list. Selecting a preset fills the launch fields and leaves them editable. When the server is Running, saving shows the restart banner and does not restart.

### T-036 — Speculative decoding (MTP and draft models) `[dep: T-030, T-031, T-033, T-035]`
Classify models by their speculative-decoding role and make each role launchable: **MTP models** are complete models carrying Multi-Token-Prediction heads (`{arch}.nextn_predict_layers` in the header) that draft tokens via `--spec-type draft-mtp`; **draft models** (architectures `dflash`, `dspark`, `eagle` and successors) are small companion models that are never standalone — they exist only as a `--model-draft` companion of a main model.

The GGUF reader (T-030) exposes both signals: `is_draft_model` (architecture-based) and `has_mtp_heads` (header-key based). The registry (T-031) rejects draft-architecture files at import — a draft model entering the catalogue as a launchable model is wrong by construction, since `llama-server` cannot run it without a main model.

*Acceptance:*
- **Detection is header-derived, not name-derived.** A draft model is classified by its `general.architecture` value, never by a filename pattern — a file named `...DFlash....gguf` whose architecture is a normal one (e.g. a MTP-enabled Qwen variant) imports normally. A MTP model is classified by the presence of `{arch}.nextn_predict_layers`, never by a "MTP" substring in the name. Both rules are asserted with synthetic fixtures where name and header deliberately disagree.
- **Draft files are rejected at import with a typed, named error** stating the architecture and that draft models are companions — not silently skipped, not imported. Rejection happens before any row is written: a cancelled or draft-rejected import leaves no partial entry.
- **An MTP model launches with `--spec-type draft-mtp`** emitted by the preset generator (T-033) when the model's `has_mtp_heads` is true, using only flags present in `docs/verified-flags.md`. The flag is emitted for the model section, not as a global default.
- **A main+draft pair is expressible**: the launch parameters for a model carry an optional draft-companion reference (path of a draft-architecture file on disk). The preset generator emits `--model-draft` naming it, plus any verified draft tuning flags the user set. The companion is validated: it must exist, parse as GGUF, and carry a draft architecture — each failure is a typed error naming the file, not a warning.
- **The UI surfaces the role**: the model detail screen (T-035) shows an MTP badge when `has_mtp_heads` is true, and offers a draft-companion picker (file picker restricted to parseable draft-architecture GGUFs) for models that support an external draft model. Draft-architecture files are not shown as launchable in the catalogue list.
- **No regression for ordinary models**: a model with neither signal emits no speculative flags at all — the generated preset is byte-identical to the pre-task output for such a model, asserted by a snapshot diff.

---

## Milestone D — Server

### T-040 — Process supervisor `[dep: T-022, T-033]`
Actor task owning `ServerState` per `docs/CONTRACTS.md` §2. Allocate an ephemeral upstream port on `127.0.0.1`, spawn with `--no-webui --host 127.0.0.1 --port <upstream>`, capture stdout/stderr into structured events, run the health check as defined in §2, graceful stop then forced tree kill, crash detection.

**Startup has two phases.** The coordinator answering is seconds; preloading a 65 GB model is minutes. `StartupPhase` reports which one is in progress and how far it has got, and the two have separate timeouts — one value covering both would be uselessly loose for the first.

*Acceptance:* Against a stub child process that can be told to hang, exit, or emit arbitrary output — every legal transition in §2 is exercised, **including both `Starting` phases and the shortcut straight to Running when no model is marked `preload`**; a coordinator that answers while a preload is still in flight yields `Starting { Preloading }` and not `Running`, asserted explicitly, and `Running` is reached only once every preload reports `Loaded`; `process_timeout_seconds` and `preload_timeout_seconds` are honoured independently, and a preload failure produces a diagnosis naming the model; three consecutive probe failures are required to reach `Crashed` and a single dropped response is not enough, asserted by a stub that drops exactly one; every rejected command returns `InvalidTransition` without changing state; concurrent `start_server` calls resolve with exactly one winner; stopping during Starting kills the partial process; the grace timeout escalates to a tree kill; no orphaned children remain after stop, asserted by inspecting the process tree. **The upstream port is always on `127.0.0.1` and never user-configurable, asserted by a test over the spawned argument vector.** An occupied upstream port causes a silent retry with another port from the range, up to a bounded number of attempts, then a typed error — this is the one place auto-increment is correct (`PLAN.md` §2.7). Log capture is line-oriented and lossless under high output volume.

### T-041 — Orchestrator `[dep: T-040]`
`ModelOrchestrator` trait plus the router implementation (`docs/CONTRACTS.md` §5). Poll `GET /models`, explicit load and unload, read `/props`, back off on errors. Distinguish `/models` from `/v1/models`.

These are the app's own control calls, made directly to the upstream. **They do not pass through the endpoint listener.**

**Writes `launch_history`** on every load attempt, keyed by the model's absolute path — `succeeded`, `actual_vram_bytes`, `load_seconds`, `error_message`. Without this the estimator's calibration path is dead code.

*Acceptance:* Against a stub HTTP server — load state syncs within one poll interval; a load failure surfaces as a typed error carrying the server's message, never a hang; repeated failures back off with a bounded ceiling; a slow load emits progress rather than blocking. A `launch_history` row is written for every load attempt, success or failure, verified against the database. The trait has a second no-op implementation used in tests, proving the abstraction is real rather than router-shaped — this is the seam the `llama-swap` fallback would use. The router implementation passes model paths through unchanged, asserted for a path on a second volume.

### T-042 — Endpoint listener `[dep: T-040]`
The app's own HTTP listener (`axum`), bound per `ServerConfig`, forwarding every request to the single upstream allocated by T-040. This is `PLAN.md` §2.7 and it is governed by `AGENTS.md` invariant 3 — read both before starting.

**This task is the transport only.** How the endpoint answers when the upstream is not available is T-044; authentication and the request log are T-051. The split is deliberate: "the bytes arrive intact" and "the app behaves correctly when the server is down" are different properties, and one checkbox covering both is a checkbox nobody reads.

Behaviour:

- Binds at app start, independently of `ServerState`, and stays bound. `EndpointState` is reported separately and `endpoint-state-changed` is emitted on every change.
- Forwards method, path, query, headers and body unchanged. **Never deserializes a body.**
- Streams responses through without buffering.
- A bind failure on the configured port yields `AppError::PortInUse` and `EndpointState::BindFailed`. **Never auto-increments** — the address is the app's contract with configured clients.

*Acceptance:* Against the T-041 stub upstream —

- **Transparency:** a differential test issues a matrix of requests directly to the stub and through the listener, and asserts responses are byte-identical, including status, headers (minus hop-by-hop), and trailers.
- **Streaming:** an SSE response of at least 50 chunks emitted with deliberate gaps arrives through the listener with the **same chunk boundaries and comparable inter-chunk timing** — a test that only compares concatenated bodies does not satisfy this criterion, because buffering passes it.
- **Long requests:** a response that begins after a 3-minute delay completes rather than timing out, with the timeout configured explicitly in code and asserted by a test rather than left to a default.
- **Latency budget:** over at least 200 non-streaming requests against the stub, the p99 of (through the listener − direct to the stub) is **under 5 ms**; for streaming, the added time to first byte is **under 15 ms**. Without a number, "transparent" only means byte-identical, and a change that doubles latency passes every other test in this list. The measurement runs in CI against the stub, so it is a regression check rather than a benchmark. **It runs on a shared `windows-latest` runner, so the statistic is specified rather than left to chance:** five sequential batches of 200, the p99 taken per batch, and the **median of the five** compared against the threshold. A single batch over budget is noise; three of five over budget is the regression. Both numbers are printed on every run, pass or fail, so drift is visible before it crosses. If this test becomes flaky, fix the measurement — never the threshold; the whole point of a number here is that it cannot be negotiated downward one commit at a time.
- **Concurrency:** with `max_concurrent_requests` unset, 100 simultaneous streams are all served. With it set, requests beyond the limit receive 503 with `Retry-After` **immediately** — a test asserts they are not queued, since queuing would be the app scheduling traffic.
- **Invariant:** a source-level test asserts that no type in `core/endpoint/` derives `Deserialize` for a request body and that `serde_json::from_*` is not called on forwarded content. If that proves impractical as a source check, an equivalent runtime assertion over a request whose body is invalid JSON — which must forward successfully — is acceptable.

### T-043 — Startup failure diagnostics `[dep: T-040]`
Map llama-server stderr patterns to `Diagnosis` with actionable remediation. Minimum catalogue: missing CUDA DLL, driver/runtime mismatch, out of VRAM, out of RAM, unknown or malformed flag, model file not found, unsupported architecture, upstream port in use, corrupt GGUF, junction target missing.

**Fixtures:** patterns reproducible without a GPU — port in use, model file not found, malformed flag, corrupt GGUF, junction target missing — are captured from a real CPU build, as in T-023. The GPU-dependent patterns are hand-written from the upstream source's error strings, **and each is marked in the fixture file as unverified against a real failure**, so the owner's checklist can replace them later. Do not present a hand-written fixture as a capture.

*Acceptance:* Table-driven tests over stderr fixtures for each pattern in the catalogue. An unmatched failure yields a generic diagnosis carrying the log tail — never an empty message. Every `Diagnosis` has non-empty `remediation`. Adding a pattern requires no change outside `diagnostics.rs`. A test asserts every fixture is labelled either captured or unverified, and that the counts match what the file claims.

This is the highest-value task in the project for support load. A user who reads "CUDA runtime not found — this build targets CUDA 13 but your driver reports 12.4; install a newer driver or switch builds in Runtime settings" does not file a ticket.

### T-044 — Endpoint behaviour across server states `[dep: T-042, T-043]`
What the listener answers when the upstream is not there. T-042 proved the bytes pass through; this proves the app is honest when they cannot.

Behaviour follows the table in `docs/CONTRACTS.md` §2. In short: forwarded when Running; forwarded also while `Starting { phase: Preloading }`, because the coordinator is already answering; held up to `startup_hold_seconds` while `Starting { phase: WaitingForProcess }`; `503` in OpenAI error shape when Stopped, Stopping or Crashed, carrying the `Diagnosis` in the last case.

*Acceptance:* Requests in each `ServerState` and each `Starting` phase produce the response in the §2 table. The `503` body is built from `AppError::UpstreamUnavailable` and parses as an OpenAI error object; the `Crashed` body carries the T-043 diagnosis code and remediation. A request held during `WaitingForProcess` that becomes servable within the hold window is served, and one that does not receives `503` after the window and not before, asserted on timing rather than only on status. **A request arriving during `Preloading` is forwarded, not held** — asserted against a stub whose coordinator answers while a preload is still in flight, because holding it would make the app less available than the server it manages. The listener remains bound across a full stop/start cycle, asserted by a client holding a keep-alive connection throughout. An occupied listen port yields `EndpointState::BindFailed` carrying `AppError::PortInUse`, never a different port; `rebind_endpoint` succeeds once the port is free, asserted by a test that releases the conflicting socket mid-run.

### T-045 — Dashboard `[dep: T-005, T-041, T-043, T-044]`
Server state, endpoint state and address, GPU stats, loaded models, uptime, recent logs.

*Acceptance:* All values arrive via events; a test asserts the renderer issues no polling IPC. Every panel has a defined empty state when Stopped. **`Starting { Preloading }` renders as progress with a count and the model being loaded, not as an indeterminate spinner** — a 65 GB preload is minutes long and an opaque bar is indistinguishable from a hang. `Crashed` renders the T-043 diagnosis with its remediation. **The endpoint address is displayed with its state and is copyable**, and a `BindFailed` endpoint is visually distinct from a stopped server — they are different problems and the dashboard must not conflate them.

### T-046 — Logs screen `[dep: T-040]`
Raw output, filter by instance and level, export, capped ring buffer.

**Export is redacted the same way the crash bundle is.** Absolute paths outside the app's data directory are reduced to their basename, and any configured API key is removed. An exported log goes into a support ticket exactly as a crash bundle does; redacting one and not the other protects nothing.

*Acceptance:* 100k synthetic lines render responsively (virtualized). The ring buffer respects its cap under sustained input. Export content matches what is displayed, **except for redaction, which is asserted directly**: a fixture log containing a user model path and a configured API key exports with the path reduced to its basename and the key absent, asserted by the same test helper the crash bundle uses (`PLAN.md` §6). Filtering is correct over a fixture log with mixed instances and levels.

### T-047 — Tray, quit sequence and session end `[dep: T-044, T-045]`
Closing the window hides to the system tray; the endpoint and the server keep running (`PLAN.md` §2.10). The tray menu reopens the window and offers **Quit**, which stops the server before the process exits.

Quit sequence, in order and without shortcuts: graceful stop → the **same 10-second grace timeout the `Stopping` state already uses**, not a second constant → process-tree kill → listener unbind → exit. Windows session end (`WM_QUERYENDSESSION`) runs the same sequence with the shorter grace period the OS permits.

*Acceptance:* Closing the window hides it and leaves `ServerState` and `EndpointState` unchanged, asserted by a client holding a connection across the close. The tray icon renders distinctly for Running, Stopped and Crashed, asserted over fixture states. A first-close notification is shown once and not on subsequent closes, asserted across a simulated restart. **Quit with a stub child that exits promptly leaves no process in the tree and no port bound; quit with a stub child that refuses to exit escalates to a tree kill after the shared grace constant and still leaves nothing behind** — both asserted by inspecting the process tree and attempting to re-bind the port immediately afterwards. A test asserts the grace value is read from one place, not duplicated. Quitting during a long unload shows progress and the window can be reopened while it runs. The session-end path is exercised against a simulated `WM_QUERYENDSESSION` and leaves no orphan. **On launch, a llama-server process left over from a previous run is detected and reported rather than ignored or silently killed.**

---

## Milestone E — API

### T-050 — API screen `[dep: T-005, T-042]`
Listen address and port, endpoint state, API key, router policy (`--models-max`, autoload, idle unload), startup hold, client snippets.

**The idle-unload control is conditional on the flag existing.** `ServerConfig.idle_unload_seconds` is modelled but no flag spelling for it has been observed (`docs/LLAMACPP.md` §1, `PROGRESS.md` D-003). Render the control only when the active build's verified list contains the flag; otherwise omit it entirely. A control that silently does nothing is worse than an absent one.

*Acceptance:* Setting `listen_address` to `0.0.0.0` requires explicit confirmation naming the exposure risk; without confirmation it is rejected. Combining `0.0.0.0` with no API key shows a persistent warning. **Changing the listen address or port while the server is not Stopped is rejected with the reason displayed** — the address clients use never moves implicitly. Changing a router setting while Running raises `config_dirty` and shows the restart banner. `request_log_enabled` and `startup_hold_seconds` apply immediately and say so. The idle-unload control is absent against a fixture build whose verified list lacks the flag and present against one that has it, asserted both ways. Snippets are generated from the live config, point at the app's address rather than the upstream, and are validated by a test that parses them.

### T-051 — Authentication and request log `[dep: T-042, T-050]`
API-key authentication and request logging at the listener. **Not optional** — the listener is always in the path (`PLAN.md` §2.7); what is optional is whether a key is configured.

The log records method, path, status, duration, byte counts, and whether the response streamed. **Never bodies, never header values, never the model name** (`PLAN.md` §6). In-memory ring buffer, emitted as `request-logged`.

**This task also adds the log panel to the API screen.** T-050 builds that screen before the log exists, so the panel arrives here rather than being stubbed there — a screen shipped with a placeholder that a later task fills in is how placeholders reach release.

*Acceptance:* With no key configured, responses are byte-identical to the T-042 baseline, asserted by re-running that differential test with this layer enabled. With a key configured, unauthenticated requests receive 401 before anything reaches the upstream, asserted by a test that the stub received no request. Key comparison is constant-time, asserted by a test over the comparison function. The key never appears in the app log, the request log, or a crash bundle, asserted by a test that greps all three. Disabling `request_log_enabled` stops collection and clears the buffer, and the panel says so rather than showing an empty list. The panel renders 10k entries responsively (virtualized) and updates from `request-logged` events without polling. The ring buffer respects its cap under sustained load and does not grow the process's memory across a 100k-request synthetic run.

### T-052 — Client contract tests `[dep: T-041, T-051]`
Automated conformance tests for the OpenAI-compatible surface, run **through the app's listener** against the stub server: model listing shape, completion request and response shape, streaming SSE framing, error response shape, and the JIT path where a request naming an unloaded model triggers a load.

*Acceptance:* The suite runs in CI with no GPU and no real model. Response shapes are asserted against the OpenAI schema, not against hand-written expectations. The JIT test asserts that a request for an unloaded model produces a load call followed by a normal response — **and that the app made no routing decision**, i.e. the stub received the `model` field exactly as the client sent it. The suite runs twice, once with authentication configured and once without, with identical results for authenticated traffic. The `503` responses the listener produces when the upstream is down also satisfy the OpenAI error schema.

Interoperability with real third-party clients is an owner check, not a criterion here.

---

## Milestone F — Release

### T-060 — Telemetry `[dep: T-010, T-041, T-042]`
NVML sampling behind the `NvmlProvider` trait (`docs/CONTRACTS.md` §6), tokens/sec parsed from server output, request counters from the listener, charts.

*Acceptance:* Parsing tested against output fixtures including malformed lines. `active_requests` and `requests_last_minute` are sourced from the listener's counters and asserted against a synthetic request pattern. **`tokens_per_sec` is parsed from the server's stderr and never from a response body**, asserted by a test that the value is produced with the listener idle. A simulated 24-hour session shows bounded memory and a bounded chart series. Sampling failures degrade to absent values, never to a panic or a stalled UI.

### T-061 — Multi-GPU `[dep: T-010, T-032, T-035]`
`--tensor-split` and `--main-gpu` in the UI; estimator aware of multiple devices.

*Acceptance:* Against multi-GPU mocked environment reports — split values are validated (count matches device count, values sum to a positive number, negatives rejected); the estimator distributes layers across devices and never exceeds any single device's free VRAM; the UI hides the controls entirely on single-GPU systems.

### T-062 — Crash recovery `[dep: T-010, T-022, T-044]`
Bounded auto-restart with backoff, crash report bundle.

*Acceptance:* The restart loop is bounded and its state observable; repeated crashes stop retrying and surface the diagnosis. **The endpoint stays bound throughout a crash and restart cycle**, and requests during it receive the `503` carrying the diagnosis rather than a refused connection. The bundle's contents match `PLAN.md` §6 exactly — a test asserts that a configured API key, request log contents, absolute paths outside the app directory, and model file contents are all absent.

### T-063 — Settings screen `[dep: T-005, T-024, T-029, T-031]`
Watched folders, default folder for the file picker, theme, launch at Windows startup, auto-update policy, backend selection, log retention, retained model settings.

*Acceptance:* Adding and removing watched folders never touches files on disk. Launch-at-startup writes and removes its registry entry correctly through `RegistryProvider` (`docs/CONTRACTS.md` §6), asserted against a fake implementation — the real registry is never touched by a test. Every setting round-trips through `AppSettings` and the database and survives a restart. **Launch at startup starts the app minimized to the tray, not with the window open**, asserted over the generated command. Retained settings for files no longer in the catalogue are listed and individually clearable, so `retained_model_settings` is never a table that grows invisibly. No setting is applied silently while the server is Running — each either blocks or raises `config_dirty`.

### T-064 — Installer and auto-update `[dep: T-063]`
Signed MSI/NSIS, update channel.

*Acceptance:* The installer builds in CI and installs unattended into a clean Windows container image with no developer tools present, then launches and reaches the health-check screen. Uninstall removes the app and its data directory — including any links it created, of every primitive — but never a user model file, asserted by comparing a checksum of the fixture model tree before and after.

### T-065 — E2E suite `[dep: T-064]`
Playwright: first run → install runtime (stubbed release server) → attach folder → import synthetic model → configure → start (stub server) → **query through the app's endpoint** → stop → query again and receive the structured 503 → restart → query succeeds at the same address → close the window and confirm the endpoint still answers → quit from the tray and confirm nothing is left listening.

*Acceptance:* The full path runs green in CI on a GPU-less runner using the stub runtime and synthetic fixtures. The suite fails loudly if any step silently no-ops. The final leg asserts the client used one unchanged address across the whole sequence — this is the behaviour `PLAN.md` §2.7 exists to deliver, and it is verified end to end rather than only in unit tests.

---

## Corrections — the `T-1xx` range

Commissioned by the owner to apply a decision, never opened on your own initiative. The rules are in `docs/WORKFLOW.md` §7; the short form is that the owner decides and you redraft, that the task names the discrepancy it closes, and that a correction task changes documents and not code.

These sit outside the milestones on purpose. A milestone is a functional outcome; a correction is maintenance of the specification, and folding one into the other would make the milestone ranges in `PLAN.md` §4 stop meaning anything.

*(none yet)*

<!-- Format:

### T-1xx — <what the document should say now> `[closes: D-00x]`
Which documents change, and what the decision was. Quote the owner's decision rather than paraphrasing it — a correction task that restates a decision in its own words is how a decision drifts on the way to being applied.

*Acceptance:* The named documents say what the decision says. `docs/TASKS.md` lint (T-006) passes. The discrepancy is moved to `resolved` in `PROGRESS.md`, naming this task. No code file is touched — if the decision implies a code change, it is a separate task and this one names it.

-->
