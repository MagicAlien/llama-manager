## Task

`T-___` — <title>

## What changed

<one paragraph>

## Acceptance criteria

Copy each criterion from `docs/TASKS.md` and check it only when a test or an automated check proves it.

- [ ] …
- [ ] …

## Invariants (`AGENTS.md` §6)

- [ ] `ipc/` contains no logic; `core/` does not import `tauri`
- [ ] No shared `Mutex<ServerState>` introduced
- [ ] Estimator remains pure (history passed as an argument)
- [ ] No `unwrap()` / `expect()` in `core/` or `ipc/` outside tests
- [ ] No string literals in JSX text positions
- [ ] No model file copied, moved without consent, or deleted
- [ ] Removal retains parameters and launch history keyed by path (`PLAN.md` §2.9)
- [ ] `src/lib/types.ts` not hand-edited

## Endpoint invariants (`AGENTS.md` invariants 3–4, `PLAN.md` §2.7)

Only applicable if this PR touches `core/endpoint/` or anything in the request path. Check "n/a" if it does not.

- [ ] n/a — this PR does not touch the request path
- [ ] No request or response body is deserialized, parsed, or inspected
- [ ] No routing decision is made from request content; the `model` field is never read
- [ ] Exactly one upstream; no per-model process, port, or residency policy added
- [ ] Streaming responses pass through unbuffered; chunk boundaries preserved
- [ ] Status codes and headers unaltered except for an authentication rejection
- [ ] The listener's lifecycle remains independent of `ServerState`
- [ ] The public port is never auto-incremented

## Flags used

List every llama.cpp flag this PR emits, and confirm each against `docs/verified-flags.md`.

| Flag | In verified list? | Value type matches? |
|---|---|---|
| | | |

- [ ] No flag was used that is absent from the verified list (except `extra_args`)
- [ ] Any omitted flag emits a `preset-warning` event

## Correction task (`T-1xx` only)

Skip this section entirely if this is an ordinary task.

- [ ] This PR closes `D-___`, and that entry moves to `resolved by T-1xx` in `PROGRESS.md`
- [ ] The owner's decision is quoted, not paraphrased
- [ ] **No code file is touched.** If the decision implies a code change, the follow-up task is named here: `T-___`
- [ ] If this PR changes `PLAN.md` §2 or an ADR, the owner's decision exists in writing and is linked

## Dependencies

- [ ] No new dependency, **or** the new dependency is listed in `PLAN.md` §3
- New dependency and justification: …

## Excluded scope check (`AGENTS.md` §2)

- [ ] No chat UI, message composer, or user-facing inference
- [ ] No Hugging Face downloading
- [ ] No cloud sync, accounts, or telemetry upload

## Security

- [ ] No secret, absolute path from my machine, or model file committed
- [ ] If this PR touches the API key: it is never logged, never sent upstream, never returned by an IPC command
- [ ] If this PR touches the request log: no body, header value, or model name is recorded

## Empirical questions raised

Anything this PR could not confirm without real hardware. These go to `docs/owner-verification.md` — do not add them to acceptance criteria.

- …

## PROGRESS.md

- [ ] Task moved to `Done` with its PR number and date
- [ ] Any discrepancy or ambiguity recorded in Discrepancies (not worked around)
- [ ] Any out-of-scope problem noticed recorded in Observations (not fixed here)
- [ ] Any discovery a later task depends on recorded in Facts established
- [ ] If this is T-023: `registration_channel` recorded in Facts established
- [ ] If this is T-025: every probe answered in Facts established or refused in Discrepancies with the obstacle named, and F-000's **NOT YET CONFIRMED** marker resolved
