# PROGRESS

**Open this first, every session.** It is the project's memory across sessions — you have none of your own.

Protocol: `docs/WORKFLOW.md`. Rules: `AGENTS.md`.

**This file grows and nothing trims it by itself.** It is read in full at the start of every session, so its length is a running cost paid out of the context budget you need for the work. Pruning is a correction task (`docs/WORKFLOW.md` §7) and is commissioned at the close of a milestone, never done in passing. The rules differ per section and the asymmetry is the point:

- **Done** — compress to one line per milestone plus the PR numbers once the milestone closes. The diffs already hold the detail, and better than prose does.
- **Observations** — triage each one: it becomes a `T-1xx`, or it is closed with the reason it no longer applies, or it stays. "It stays" must be a decision someone made, not what happens when nobody looks.
- **Discrepancies** — a `resolved` entry keeps its one-line summary and its resolution, and loses its working detail.
- **Facts established — never pruned.** Every line there cost an experiment to learn. Deleting one means a future session rediscovers it the expensive way, which is the exact failure this file exists to prevent. If the file must get shorter, it gets shorter somewhere else.

Last updated: 6 August 2026 — T-005 (Design system) implemented on `t-005-design-system` (branched off `main` at PR #7's merge commit), full local gate green **run for real by the agent this session** — first frontend/TypeScript task where that was true; `npm ci`, `npm run lint`, `npm run test` (4/4), `npm run build` (`tsc` + `vite build`) all confirmed passing, including a red-then-green demonstration of both new ESLint rules and the type-level `Loading.label` test (all in the PR body below). **Not moved to Done**: this session had a full Node/npm toolchain but no GitHub push or PR-creation access — the inverse of T-001–T-004's problem (toolchain missing, push fine) rather than the same one, and worth distinguishing from those entries' "handed over as a patch" language, which assumed push access existed. Branch is committed locally; a patch bundle and the filled `.github/pull_request_template.md` are handed to the owner to push and open as PR #8. Move T-005 to Done with that PR number once it exists. D-004, D-005, D-010 and D-011 remain open, owner decisions, not touched this session. T-010 is also unblocked (T-003 and T-004 both Done) but is not the next task while T-005 sits `In progress` — `docs/WORKFLOW.md` §1: a task already in progress is picked up, not superseded.

---

## In progress

- **T-005** — Design system · branch `t-005-design-system`, no PR yet
  - Implementation complete against `docs/TASKS.md` T-005 and `PLAN.md` §2.5 / the approved-npm-packages line in §3. `tailwind.config.js`'s `theme` block is the single tokens file (palette replaces Tailwind's default entirely — dark background/surface/border/foreground/muted-foreground plus one accent, an original composition, not sampled from LM Studio; spacing is an explicit 0–24rem/0.25rem-step scale; five-size type scale). `src/components/ui/{button,separator,tooltip,loading}.tsx` — `Button` is local (no Radix; nothing in this task needed `asChild` polymorphism), `Separator` and `Tooltip` wrap `@radix-ui/react-separator`/`@radix-ui/react-tooltip` (justified in the PR body, same D-008 path — neither package is itself on `PLAN.md` §3, only the `class-variance-authority`/`clsx` styling helpers are). `Loading.label` is a required prop; `src/components/ui/loading.test.tsx` carries both a runtime test (`@testing-library/react`) and a `@ts-expect-error` type-level fixture checked by `tsc` (not by `vitest`, which doesn't type-check — see Facts established). `src/components/{Sidebar,ui/*}.tsx` and `src/screens/{Dashboard,Runtime,Models,ModelDetail,Api,Logs,Settings}.tsx` implement the app shell and seven routes; navigation is real `<a href="#/...">` links plus `src/lib/useHashRoute.ts` (a `hashchange` listener), not a router package — none is on the approved list and this task's scope excludes adding state management. `src/App.test.tsx` proves all seven routes render by driving the hash directly, plus a fallback-to-Dashboard case.
  - `eslint.config.js` gained one new scoped block (`src/components/**/*.tsx`, `src/screens/**/*.tsx`, `src/App.tsx`, excluding `*.test.tsx`) with three `no-restricted-syntax` selectors: hard-coded hex colours, Tailwind arbitrary-bracket pixel spacing, and non-whitespace `JSXText` (AGENTS.md invariant 7). All three demonstrated red then green with throwaway fixture files, the same way T-000 demonstrated CI's red path — not just asserted.
  - Full local gate run for real this session (first frontend task where that was possible — T-001/T-002/T-003 had no toolchain at all for their respective languages, and T-004 had no Rust toolchain): `npm ci`, `npm run lint`, `npm run test` (4 passing), `npm run build` (`tsc` then `vite build`, both clean).
  - **Branch pushed to `origin/t-005-design-system` — PR still not opened.** The owner supplied a personal access token mid-session; `git push` over `https://github.com/...` succeeded (this sandbox's network allowlist permits `github.com`), so the branch now exists on GitHub for real, not just as a local patch. Opening the PR itself is still blocked: this sandbox's proxy allowlist blocks `api.github.com` (`blocked-by-allowlist`, confirmed via a direct request), so the GitHub REST/GraphQL API is unreachable, and no browser was connected (`list_connected_browsers` returned empty) to complete the web form instead. GitHub's own push output supplied the ready-made compare link (`https://github.com/MagicAlien/llama-manager/pull/new/t-005-design-system`); the owner has that link and the filled PR body and needs one click plus a paste to finish. This is *not* a Discrepancy (nothing about the documents or task design is in question) and not an Observation — it's this task's own completion blocked on an environment gap, recorded per `docs/WORKFLOW.md` §4's instruction not to move a task to Done before it is actually confirmed passing. **Security note, not this task's to fix but worth recording:** a live push token was pasted directly into chat; recommended the owner revoke and reissue it once this is done, since a chat transcript is not where a credential should live.

---

## Done

- **T-004** — Error model · PR #7 · 2026-08-06
  - Note: `impl AppError { code, message, remediation }` added directly beneath the enum in `src-tauri/src/core/types.rs`, matching `docs/CONTRACTS.md` §1's own code fence (the `impl` shown right after the enum, commented `// core/src/types.rs`) — not the ambiguity it looked like going in; see D-011. `code()`: static match, one arm per variant, unique by construction, re-checked by a test that enumerates them. `message()`: delegates to the `Display` impl `thiserror` already generates from each variant's `#[error("...")]` string, so the wording lives in one place. `remediation()`: `Some` for the nine variants with an actionable next step, `None` for the six that don't have one.
  - Note: `every_app_error_variant_has_a_serialized_shape` (one assertion per variant — `app_error_round_trips`, already present from T-002, covered only three), `no_variant_message_is_empty`, and `code_values_are_unique_across_variants` (both of the latter share a new `all_app_error_variants()` fixture) all confirmed passing by a real `cargo test` run.
  - Note: `AGENTS.md` invariant 6's `unwrap()`/`expect()` clippy gate introduced as `#![deny(clippy::unwrap_used, clippy::expect_used)]` in `src-tauri/src/core/mod.rs` — module-scoped (propagates to `core::types` and any future submodule), not crate-wide, so `db/` and `main.rs` stay outside it as the invariant says. `deny`, not `warn`: an in-source attribute for a specific lint outranks a command-line group flag (`-D warnings`) for that same lint in rustc's precedence order, so `warn` would not actually have failed CI. New `src-tauri/clippy.toml` (`allow-unwrap-in-tests`, `allow-expect-in-tests`) keeps the deny from also rejecting the `.unwrap()` calls already inside `#[cfg(test)]` — confirmed by a real `cargo clippy -- -D warnings` green run. **`ipc/` does not exist yet**, so only `core/` carries the attribute today; whichever task creates `ipc/mod.rs` must copy the identical line, or invariant 6 goes unenforced there silently (Observations).
  - Note: D-011 raised in pre-implementation review — `PLAN.md`'s Repository layout diagram lists `src-tauri/src/error.rs` as a file, which `docs/CONTRACTS.md` §1's own code fence settles is not where the `impl` belongs (see above); the diagram line is stale, not a second instruction, and nothing on the task list will ever create that file under this design. Left `open` for the owner (a `PLAN.md` correction, same class as D-009, needs sign-off before a `T-1xx` can apply it).
  - Note: this session had no Rust toolchain and no push access — handed to the owner as a patch. The owner applied it on a real Windows machine and ran the full local gate for real. One real finding: `cargo fmt --check` wanted the `assert!` in `no_variant_message_is_empty` collapsed onto one line — fixed, same class of first-real-toolchain fixup every prior task (T-001, T-002, T-003) hit on its own first real gate run. Full gate green after that fix: `cargo fmt --check`, `cargo build`, `cargo clippy -- -D warnings`, `cargo test` (all tests, including the three new to this task), `npm run generate-types` (no diff to `src/lib/types.ts`, as expected — `AppError`'s shape is unchanged, only its `impl` gained methods, which `#[ts(export)]` doesn't serialize).

- **T-003** — Database layer · PR #6 · 2026-08-05
  - Note: `db/` (migrations + queries) implemented against `docs/CONTRACTS.md` §3's schema, transcribed verbatim — `launch_history` and `retained_model_settings` carry no `FOREIGN KEY` to `models` (D-010 below). Forward-only migration runner: idempotent on fresh and already-migrated databases, `AppError::SchemaTooNew` on a too-new recorded version. `rusqlite` (`bundled`) added per `PLAN.md` §3's approved list, not re-justified. `Backend::Display`/`FromStr` (compact `'cuda:13'`/`'vulkan'`/`'cpu'` form for `runtimes.backend`, distinct from the serde-tagged JSON form) added to `core/types.rs`, round-tripped by test.
  - Note: D-010 raised in pre-implementation review, same pattern as D-004 — `docs/TASKS.md` T-003's acceptance text contradicts itself on whether `launch_history` cascades from `models` ("survives deletion, no FK" vs. "cascades", in the same paragraph), and `docs/CONTRACTS.md` §3's intro line implies a `REFERENCES` clause absent from every transcribed `CREATE TABLE`. Implemented against `CONTRACTS.md`'s own stated rationale ("Removal is not amnesia" — no FK, rows survive); the "cascade" sentence is not implemented or tested as one. Left `open` for the owner.
  - Note: This session had no Rust toolchain and, new versus T-001/T-002, no push access to this repository — the branch was handed to the owner as a patch rather than pushed directly. The owner applied it on a real Windows machine and ran the full local gate for real. One real finding: `cargo clippy -- -D warnings` flagged `clippy::type_complexity` on two tuple types in `db/queries.rs` (16-element `models` row, 8-element `launch_history` row); fixed by factoring them into named types (`ModelRowTuple`, `LaunchHistoryTuple`), per clippy's own suggestion, rather than suppressed. Full gate green after that fix: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test` (all T-003 tests, including the unique-active-runtime index, the `schema_version` `CHECK (id = 1)` constraint, `foreign_keys` pragma, and the `Backend` round-trip), `npm run lint`, `npm run test`, `npm run generate-types` (no diff to `src/lib/types.ts`, as expected — this task touches no frontend).
  - Note: `src-tauri/Cargo.lock` regenerated for real as part of the owner's gate run; now lists `rusqlite` and its transitive dependencies.
  - Note: a stray `package-lock.json` diff (npm rewriting `"peer": true` entries under a locally different npm version than produced the committed lockfile) showed up alongside the real `cargo fmt` diff after the clippy fixup and was reverted — not part of this task, not committed. Worth knowing if it recurs: it's npm/local-environment noise, not something T-003 or any Rust change caused.
  - Note: **fixing forward from CI, not just from a re-run of the specific check that failed, matters** — after the `clippy::type_complexity` fixup, only `cargo clippy` was re-run locally before pushing, not the full gate; CI then caught a `cargo fmt --check` failure on that same fixup that a full local re-run would have caught first. Worth remembering for future fixup patches: re-run the whole gate, not just the check that originally failed.

- **T-000** — CI pipeline · PR #3 · 2026-08-04
  - Note: `jobs.gates` in `.github/workflows/ci.yml` is the job T-002 extends — **by editing it, not by calling it**; it is not a `workflow_call` job (see Observations). Red path demonstrated by throwaway PRs #1 (clippy warning) and #2 (unformatted); both closed without merging and their branches deleted — run ids and step-level evidence are recorded in PR #3's body. The `demo:failure` label job re-proves the red path on demand. Warm green run: 51–63 s **on a crate with no dependencies** — see Observations before treating that as the pipeline's real cost.
  - Note: T-000 also shipped `.gitignore`, which `docs/TASKS.md` assigns to T-001. T-001 still owes `rust-toolchain.toml` and `.cargo/config.toml`.
  - Note: `main` runs red until PR #3 merges (it carries no code yet). That run (30926658728) failing at `cargo fmt --check` is expected, not a regression.

- **T-001** — Scaffold · PR #4 · 2026-08-04
  - Note: D-006 closed in this PR (both halves, demonstrated, not just asserted): `dtolnay/rust-toolchain` pinned to `@1.88` in both `ci.yml` jobs, `docs/DEV-SETUP.md` §194 corrected. CI on PR #4 shows `cargo clippy -- -D warnings` and `cargo fmt --check` both green under the pin.
  - Note: **Warm CI time: 6 m 17 s**, real Tauri dependency tree — supersedes T-000's 51 s on a dependency-free crate, which proved nothing about this budget. `docs/DEV-SETUP.md`'s "under 10 minutes" is still unenforced (`timeout-minutes: 30`, per T-000 Observations); 6 m 17 s fits inside it today but nothing stops that from drifting — worth an owner decision on whether to gate it, not urgent.
  - Note: Cargo-cache staleness (T-000 Observations: `actions/cache` never re-saves on a hit) fixed in this PR — key now suffixed with `github.run_id`, `restore-keys` falls back to the dependency-hash prefix.
  - Note: `scripts/lint-empty.js` replaced by a real ESLint flat config in the same commit range that added `src/`, per its own tripwire comment.
  - Note: One `cargo fmt` fixup needed after the first CI run (the `LOCALAPPDATA` `.map()` chain wanted wrapping) — applied the exact diff CI reported, second run green.
  - Note: This session had no Rust toolchain and no Windows/WebView2 machine; `cargo build/test/clippy/fmt` were verified by CI, and `npm run tauri dev` opening a window plus the log file at `%LOCALAPPDATA%\LlamaManager\logs\app.log` were verified by the project owner on a real Windows checkout — not by the agent directly. Worth knowing if a future session needs to distinguish agent-verified from owner-verified evidence in this entry's history.
  - Note: `src-tauri/Cargo.lock` was not committed by the agent (no toolchain/registry access to generate an accurate one); confirm it has been committed from a real `cargo build` (CI's or the owner's Windows run) before or as part of merging PR #4.

- **T-002** — Type generation · PR #5 · 2026-08-05
  - Note: Full `jobs.gates` green on a real run, 6 m — `cargo fmt --check`, `cargo build`, `cargo clippy -- -D warnings`, `cargo test` (all 14 enum round-trip tests, including `Backend::Cuda { major: 200 }` against a major the code has no branch for), `npm ci`, the `src/lib/types.ts` freshness check, `npm run lint`, `npm run test` all confirmed passing. `demo:failure` job correctly skipped — it only runs when a PR carries that label, which this one does not.
  - Note: D-007 (`TS_RS_EXPORT_DIR` path resolution), D-008 (`chrono` dependency, justified per `AGENTS.md` §4), D-009 (`FlashAttn`/`AppError` missing `PartialEq`) all resolved and now confirmed correct by the green build, not just reasoned.
  - Note: This session had no Rust toolchain; every fix after the initial push was applied against real CI output the owner relayed rather than guessed — `cargo fmt` diff, one `clippy::large_enum_variant` finding (`ImportProgress::FileDone`, fixed by boxing), and three rounds of `npm run generate-types` diffs before a byte-exact CI artifact settled `src/lib/types.ts` for good. F-002 records what the real run confirmed about `ts-rs`'s actual output (quoted tag keys, trailing commas, JSDoc from doc comments, the `serde-json-impl`/`JsonValue` dangling-reference bug worked around with `#[ts(type = "unknown")]`, and a trailing-space formatting detail that caused three rounds of false diffs before the real artifact was diffed byte for byte).
  - Note: `src/lib/ipc.ts` hand-writes all 41 `docs/CONTRACTS.md` §4 commands against the generated types; its `no-restricted-syntax` lint rule was demonstrated red then green for real.

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
- Status: resolved in T-001 (PR #4). Both halves landed: `dtolnay/rust-toolchain@1.88` pinned in both `ci.yml` jobs; `docs/DEV-SETUP.md` §194 corrected (`components = ["clippy", "rustfmt"]` was already present in §143's example, so that half needed no change). Demonstrated, not just asserted: PR #4's CI shows `cargo clippy -- -D warnings` and `cargo fmt --check` both green under the pin.

### D-007 — `.cargo/config.toml`'s `TS_RS_EXPORT_DIR` would have resolved outside the repo
- Type: discrepancy
- Found in: `.cargo/config.toml` (T-001), T-002 pre-implementation review
- Evidence: cargo's `[env]` table resolves a `relative = true` value against the directory containing the `.cargo/` folder that sets it — here, the repo root, since `.cargo/config.toml` sits at the root, not inside `src-tauri/` where `Cargo.toml` and every `cargo` invocation in `ci.yml` (`working-directory: src-tauri`) actually live. T-001's committed value, `"../src/lib"`, resolves from the repo root, one level above it — a sibling directory, not `src/lib`. (Cargo docs: https://doc.rust-lang.org/cargo/reference/config.html#env; ts-rs docs repeat the same rule.)
- Affects: `.cargo/config.toml`, `src/lib/types.ts` generation
- Proposed: point `TS_RS_EXPORT_DIR` at a scratch directory instead of `src/lib` directly — bundled with the export-mechanism decision recorded in F-002. Value changed to `src-tauri/target/ts-rs-bindings` (relative to the repo root, which resolves correctly under the same rule).
- Status: resolved in T-002. Confirmed by a real `cargo test` run in CI (`jobs.gates`, PR #5, green) — the path resolution works as fixed.

### D-008 — `chrono` is not in `PLAN.md` §3's approved crate list, but `docs/CONTRACTS.md` §1 requires it
- Type: discrepancy
- Found in: `PLAN.md` §3, T-002 pre-implementation review
- Evidence: `PLAN.md` §3's "Approved Rust crates" line does not mention `chrono` or any alternative. `docs/CONTRACTS.md` §1 declares `DateTime<Utc>` on eleven fields across `RuntimeBuild`, `AvailableRelease`, `WatchedFolder`, `ModelEntry`, `RetainedModelSettings`, `LaunchRecord`, `TelemetrySnapshot`, `RequestLogEntry`, `LoadedModelState` and twice in `ServerState`. `DateTime<Utc>` is `chrono`'s type; there is no std equivalent.
- Affects: `PLAN.md` §3, `src-tauri/Cargo.toml`
- Proposed: `AGENTS.md` §4 already provides the path for exactly this case — "or be justified in the PR description" — so `chrono` (with the `serde` feature) was added to `src-tauri/Cargo.toml` and justified in this PR rather than treated as blocking. `ts-rs`'s `chrono-impl` Cargo feature was enabled alongside it, needed for `#[derive(TS)]` to know `DateTime<Utc>`. Whether `PLAN.md` §3's list itself should be amended so future PRs don't re-justify the same crate is the owner's call — worth a `T-1xx` if so, not done here (document correction, not code).
- Status: resolved in T-002 (dependency added and justified per `AGENTS.md` §4); the `PLAN.md` §3 list itself is left as-is pending an owner decision on whether to amend it.

### D-009 — Two of `docs/CONTRACTS.md` §1's shown derive lists don't compile as written
- Type: discrepancy
- Found in: `docs/CONTRACTS.md` §1, T-002 pre-implementation review
- Evidence: `LaunchParams` derives `PartialEq` and has a field `flash_attn: Option<FlashAttn>`; `FlashAttn`'s shown derive list is `Serialize, Deserialize, Clone, Debug` — no `PartialEq`. `#[derive(PartialEq)]` on a struct requires every field's type to implement `PartialEq`; `Option<FlashAttn>: PartialEq` requires `FlashAttn: PartialEq`, which the shown code does not provide. The same problem recurs one section down: `EndpointState` derives `PartialEq` and its `BindFailed` variant carries `error: AppError`, but `AppError`'s shown derive list (`Serialize, Deserialize, Clone, Debug, thiserror::Error`) also omits `PartialEq`.
- Affects: `docs/CONTRACTS.md` §1 (`FlashAttn`, `AppError`), `src-tauri/src/core/types.rs`
- Proposed: add `PartialEq` to both derive lists — mechanical, single reading, no design question involved (unlike D-004/D-005, which are genuinely the owner's to arbitrate). Applied directly in `core/types.rs`, following the precedent D-006 set in T-001 for a fix that belongs in the code being written, not in a document-only correction task.
- Status: resolved in T-002. `FlashAttn` and `AppError` both gained `PartialEq` in `core/types.rs`, confirmed compiling by a real `cargo build`/`cargo test` run in CI (PR #5, green).

### D-010 — `docs/TASKS.md` T-003 and `docs/CONTRACTS.md` §3 disagree with themselves on whether `launch_history` cascades from `models`
- Type: ambiguity (a three-way contradiction inside the spec itself, not reality-vs-documents)
- Found in: T-003, pre-implementation review
- Evidence: `docs/TASKS.md` T-003's acceptance text reads, in the same paragraph: "`launch_history` and `retained_model_settings` survive deletion of the corresponding `models` row, asserted directly — they are keyed by path and carry no foreign key (`PLAN.md` §2.9)" and then, two sentences later: "A test asserts `foreign_keys` is on and that deleting a model cascades its `launch_history` rows." A row cannot both survive a delete and be removed by it. Separately, `docs/CONTRACTS.md` §3's own intro line — "the `REFERENCES` clause below is decorative without [`PRAGMA foreign_keys = ON`]" — implies the schema contains a `REFERENCES` clause; none of the seven `CREATE TABLE` statements transcribed in that section has one. `CONTRACTS.md`'s own design commentary two paragraphs later ("Removal is not amnesia", citing `PLAN.md` §2.9) is unambiguous and comes with a stated rationale: `launch_history` and `retained_model_settings` are keyed by absolute path with no foreign key to `models`, specifically so that removing and re-adding a model costs nothing more than the import — this is why per-model enable/disable was dropped.
- Affects: `docs/CONTRACTS.md` §3 (`launch_history`, `retained_model_settings`, and its intro line), `docs/TASKS.md` T-003's acceptance text, the `launch_history` schema and its tests
- Proposed: retype both the "cascades" sentence in `docs/TASKS.md` T-003 and the "`REFERENCES` clause below" line in `docs/CONTRACTS.md` §3 to match the no-FK design that has a stated rationale behind it — they read like they belong to an earlier draft that predated "Removal is not amnesia". No table in this schema needs a `REFERENCES` clause pointing at `models` to satisfy anything else in T-003.
- Status: open

This PR implements against `docs/CONTRACTS.md`'s explicit design reasoning (no foreign key, rows survive deletion of the `models` row) — the one instance in this contradiction with a stated rationale rather than a bare assertion. `launch_history` and `retained_model_settings` therefore carry no `FOREIGN KEY` to `models`, and the T-003 test suite asserts the "survives deletion" half of the acceptance text directly. The "cascade" half is deliberately not implemented and not tested as a cascade; treating that sentence as stale is this PR's working assumption, not a decision — it is recorded here per `AGENTS.md`'s working method and left `open` for the project owner, the same pattern as D-004.

### D-011 — `PLAN.md`'s Repository layout lists `src-tauri/src/error.rs`, which no task populates
- Type: discrepancy
- Found in: T-004, pre-implementation review
- Evidence: `PLAN.md`'s "Repository layout" section shows `src-tauri/src/error.rs` as a file, a sibling of `core/`, `db/` and `ipc/` in the tree. T-004 (the only task that could plausibly own such a file — it is the one that adds `AppError`'s behaviour) is directed elsewhere by a more specific, more authoritative source: `docs/CONTRACTS.md` §1 shows the full `impl AppError { code, message, remediation }` signature block directly beneath the enum, in the same code fence, which opens with the comment `// core/src/types.rs`. `AppError` as a type has lived in `core/types.rs` since T-002, with a doc comment on the enum itself already anticipating T-004's methods landing in that file ("The `code()` / `message()` / `remediation()` methods themselves are T-004's, per the doc comment on the enum below — only the derive lives here"). Nothing in `docs/TASKS.md` T-004 names a file. Read together, this was worth checking carefully rather than assuming either document — but it resolves cleanly: `CONTRACTS.md` §1 is explicit and specific about where the code lives, `PLAN.md`'s tree is a whole-project map that also lists many files no task has created yet (`ipc/`, `core/env_probe.rs`, etc.), and unlike those, nothing on the current task list is ever going to create a standalone `error.rs` under this design. This is not the ambiguity it looked like — it's `PLAN.md`'s diagram carrying a stale line.
- Affects: `PLAN.md`'s Repository layout section (the `error.rs` line)
- Proposed: drop the `error.rs` line from the diagram. Mechanical, single reading, same class as D-009 — but `PLAN.md` corrections still need the owner's decision in writing before a `T-1xx` can apply it (`docs/WORKFLOW.md` §7), so it is recorded rather than fixed here.
- Status: open

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

- **`npm audit` reports 5 pre-existing vulnerabilities (3 moderate, 1 high, 1 critical) in transitive dependencies (T-005).** Present before this session's `npm install` additions (`@radix-ui/react-separator`, `@radix-ui/react-tooltip`, `class-variance-authority`, `clsx`) — a baseline `npm ci` against the T-001 scaffold's original lockfile already carries them. Not investigated: T-005 is a design-system task, and auditing/upgrading transitive dependencies is unrelated scope (`docs/WORKFLOW.md` §3). Worth a look before v1 ships.

- **`db/migrations/mod.rs::known_schema_version()` calls `.expect("MIGRATIONS is never empty")` outside any test (T-004 review).** `AGENTS.md` invariant 6 bans `unwrap()`/`expect()` outside tests only in `core/` and `ipc/`; `db/` is deliberately not in scope (T-004's own instructions were explicit on this point), so this is not a violation and the new clippy gate does not touch it. Noted rather than fixed here per scope discipline (`docs/WORKFLOW.md` §3) — `db/queries.rs` and `db/migrations/mod.rs` otherwise only use `.unwrap()`/`.expect()` inside `#[cfg(test)]` code, so this is the one instance outside tests in `db/` today.
- **`ipc/mod.rs`, once created, must carry the same `#![deny(clippy::unwrap_used, clippy::expect_used)]` line T-004 added to `core/mod.rs`.** `AGENTS.md` invariant 6 names both `core/` and `ipc/`; the lint is scoped per-module (there is no crate-wide mechanism that would also spare `db/` and `main.rs`), so it only covers what exists today. Nothing currently enforces it for `ipc/`, because `ipc/` doesn't exist yet. Whichever task creates it should copy the attribute (and lean on `src-tauri/clippy.toml`'s `allow-unwrap-in-tests`/`allow-expect-in-tests`, which already covers both modules) rather than rediscover the precedence reasoning in `core/mod.rs`'s comment.

- **`(node:...) DEP0040` — Node's built-in `punycode` module is deprecated (T-002 CI logs).** Not ours: `npm why punycode` traces it to `eslint@9.39.5` → `@eslint/eslintrc` → `ajv@6.15.0` → `uri-js` (and separately `jsdom` → `whatwg-url` → `tr46`, already deduped to the same resolved version). `eslint` is already at the newest version inside its approved `^9.9.0` range (`PLAN.md` §3); the old `ajv` 6.x comes from eslint's own legacy-config compatibility layer, not from anything this project chose. A plain `require("punycode")` anywhere in that chain gets Node's deprecated built-in ahead of the real `punycode` npm package installed alongside it (also present, `2.3.1`) — that's the warning, not a version mismatch fixable from here. Harmless (warning only, nothing fails), and not fixable within this project without dropping or forking `eslint` or `jsdom`, so left alone rather than worked around.
- **`Cache not found for input keys: Windows-npm-...` (T-002 CI logs) is expected, not a problem.** First run of `actions/cache` against this exact `package-lock.json` hash — nothing to restore yet, so `npm ci` ran cold. The cache saves at the end of this run and later runs against an unchanged lockfile will hit it.
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

### F-003 — ESLint's core `no-undef` false-positives on TypeScript's ambient globals; `tsc`'s `@ts-expect-error` matches on comment text, not intent (T-005)

Two related gotchas hit while wiring T-005's type-level test and the app's screen-registry type.

**`no-undef` and the global `JSX` namespace.** `@types/react` declares `namespace JSX` both under `React.JSX` and, separately, globally via `declare global { namespace JSX {...} } ` (deprecated but present, and what `tsc` itself resolves). ESLint's core `no-undef` rule doesn't read ambient `.d.ts` global declarations, so referencing the bare `JSX` type (e.g. `Record<RouteId, () => JSX.Element>` in `src/App.tsx`) passes `tsc` but fails `eslint`'s `no-undef` with `'JSX' is not defined`. This is a known class of false positive — typescript-eslint's own docs recommend disabling `no-undef` for TypeScript files, since the type checker already covers it more accurately. Fixed by adding `"no-undef": "off"` to `eslint.config.js`'s `**/*.{ts,tsx}` rules block, with a citation to typescript-eslint's troubleshooting page. Not scoped to `src/`-only files or to T-005 — the false positive applies to any ambient global type, so the fix is general.

**`@ts-expect-error` is matched structurally, not semantically.** `tsc` treats *any* single-line comment whose text begins with `@ts-expect-error` (immediately after `//`) as a real suppression directive for the following line — including one that appears inside an explanatory paragraph of prose, with no intent to suppress anything. `src/components/ui/loading.test.tsx`'s original draft explained the type-level-test pattern in a comment block that included the sentence "`@ts-expect-error` only needs to be *seen* by tsc, not executed" — and because that sentence started its own comment line, `tsc` parsed *that* line as the real directive, found no error on the following line (another comment), and failed the build with `Unused '@ts-expect-error' directive`, while the actual test fixture two lines later went unchecked. Fixed by rewording the prose so no explanatory line begins with the literal token sequence. Worth knowing for any future type-level-test file: never start a comment line with `@ts-expect-error` unless that line is the real directive.

### F-001 — GitHub Actions pwsh steps propagate the last native exit code (T-000)

GitHub Actions appends `exit $LASTEXITCODE` after every pwsh `run:` block. A step that deliberately runs a command expected to fail (e.g. `cargo clippy` on broken code) **fails the step even after a successful `$LASTEXITCODE` check**, unless the script ends with an explicit `exit 0`. Caught in the T-000 demo job (run 30928203565): the "Expect clippy to fail" step printed "clippy failed as expected (exit 101)" and then failed with exit code 1. **T-025's `scripts/probe-router.ps1` steps must end each probe with an explicit exit code** when a command is expected to fail.

Also observed: `windows-latest` currently resolves to Windows Server 2025 (image `windows-2025-vs2026`, VS 2026 build tools preinstalled), and `actions/checkout@v4` triggers a Node-20 deprecation warning — it runs on Node 24.

### F-002 — ts-rs 12.x: a shared `export_to` target does not produce one clean file; per-type files plus a collector does (T-002)

`docs/adr/adr-001-type-generation.md` left this as T-002's open question. Reasoned from ts-rs's docs first, then **confirmed empirically**: a real `npm run generate-types` run (owner, real toolchain) produced all 39 per-type files and collected them into one `src/lib/types.ts` exactly as designed, no collisions, no missing types.

ts-rs's own docs (https://docs.rs/ts-rs/12.0.0/ts_rs/) say `#[ts(export)]` generates one `#[test]` fn per type, each writing that type's binding to disk when `cargo test` runs. Directing several types at the same `#[ts(export_to = "...")]` path means several independent test functions — run by `cargo test`'s own parallel test harness, order and concurrency unspecified — writing to the same file; nothing in ts-rs's documented behavior claims these writes merge or serialize against each other. The default (no `export_to` override) writes one file per type, named after the type, which has no such collision because no two types share a target path. **T-002 uses the default and collects afterward** (`scripts/generate-types.mjs`): `TS_RS_EXPORT_DIR` points at a scratch directory (`src-tauri/target/ts-rs-bindings/`, see D-007), and the script concatenates every `*.ts` file there into `src/lib/types.ts`, stripping the self-referential `import type { X } from "./Y"` lines ts-rs emits for cross-type references — meaningless once everything shares one file.

Also confirmed by the same run: `ts-rs` 12 without the `format` cargo feature emits unformatted, single-line-per-type output, matching the docs' own example — but *with* quoted object keys on every field (`{ "kind": "Cuda", major: number, }` — the tag key quoted, the rest not) and a trailing comma after every field, including nested inline objects; the first hand-transcribed version of this file got both wrong (unquoted tag keys, missing some trailing commas) and was replaced with the real captured output. Rust doc comments are carried over as JSDoc `/** ... */` blocks ahead of the field or type they document. `TS_RS_LARGE_INT` was left unset, so `u64`/`i64`/`u128`/`i128` fields map to TypeScript `bigint`, not `number`. `DateTime<Utc>` needs the `chrono-impl` cargo feature. **`serde_json::Value` (`ServerProps.raw`) does *not* work cleanly with the `serde-json-impl` feature**: it emits a bare `raw: JsonValue` reference with no `JsonValue` definition anywhere in the exported set — a dangling type the moment files are concatenated. Fixed with `#[ts(type = "unknown")]` on that one field instead, and the feature is no longer enabled. `PathBuf` and `IpAddr` need no extra feature — both are covered by ts-rs's default foreign-type impls, confirmed by the same run.

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
