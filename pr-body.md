## Task

T-029 — Model paths and availability

## What changed

Implemented `core/model_paths.rs` with all six required functions:

- `normalize()` — idempotent path canonicalization via `fs::canonicalize`
- `preset_literal()` — INI-safe quoted path for llama.cpp preset files
- `probe()` — model availability check (Present/Missing/Unreadable) using 1-byte read to distinguish readable vs. unreadable
- `link_into()` — symlink creation within app directory with path validation
- `unlink()` — link removal leaving target intact
- `link_capability()` — real symlink test in scratch directory (not registry check)

The module is registered in `core/mod.rs` with `pub mod model_paths;`. Functions are `pub` but currently called only from tests — the consuming tasks (T-033 preset generation, T-037 model import) will call them. Module-level `#![allow(dead_code)]` is intentional and will be removed when T-033/T-037 land.

10 tests cover: normalize idempotence, absolute path resolution, preset_literal quoting, path round-trip, probe for Present/Missing/Unreadable/directory, link_into and unlink, link_into path validation, and link_capability detection.

## Acceptance criteria

- [x] normalize() is idempotent: normalize(normalize(p)) == normalize(p)
- [x] preset_literal() + parse = identity on already-normalized paths
- [x] Handle paths with spaces, non-ASCII, trailing dots, UNC, second volumes
- [x] probe() distinguishes Missing vs Unreadable vs Present
- [x] link_capability() actually tries (symlink in scratch dir), doesn't read registry
- [x] link_into() rejects destinations outside the app directory
- [x] unlink() removes the link but leaves the target intact
- [x] No code creates junctions for single files

## Invariants (AGENTS.md §6)

- [x] ipc/ contains no logic; core/ does not import tauri
- [x] No shared Mutex<ServerState> introduced
- [x] Estimator remains pure (history passed as an argument)
- [x] No unwrap() / expect() in core/ or ipc/ outside tests
- [x] No string literals in JSX text positions
- [x] No model file copied, moved without consent, or deleted
- [x] Removal retains parameters and launch history keyed by path (PLAN.md §2.9)
- [x] src/lib/types.ts not hand-edited

## Endpoint invariants (AGENTS.md invariants 3–4, PLAN.md §2.7)

- [x] n/a — this PR does not touch the request path

## Flags used

No llama.cpp flags emitted by this PR.

- [x] No flag was used that is absent from the verified list (except extra_args)
- [x] Any omitted flag emits a preset-warning event

## Correction task (T-1xx only)

Skip — this is an ordinary task.

## Dependencies

- [x] No new dependency, or the new dependency is listed in PLAN.md §3

## Excluded scope check (AGENTS.md §2)

- [x] No chat UI, message composer, or user-facing inference
- [x] No Hugging Face downloading
- [x] No cloud sync, accounts, or telemetry upload

## Security

- [x] No secret, absolute path from my machine, or model file committed
- [x] If this PR touches the API key: it is never logged, never sent upstream, never returned by an IPC command
- [x] If this PR touches the request log: no body, header value, or model name is recorded

## Empirical questions raised

None.

## PROGRESS.md

- [ ] Task moved to Done with its PR number and date
- [ ] Any discrepancy or ambiguity recorded in Discrepancies (not worked around)
- [ ] Any out-of-scope problem noticed recorded in Observations (not fixed here)
- [ ] Any discovery a later task depends on recorded in Facts established