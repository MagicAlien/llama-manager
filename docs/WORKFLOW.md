# Workflow — how a session runs

`AGENTS.md` says what the rules are. This says how a working session actually goes: where to start, which task is next, what to write down, and what to do when you cannot proceed.

Read this once. After that, `PROGRESS.md` is the file you open first every session.

---

## 1. Session start

You wake up with no memory of previous sessions. Everything you need is on disk.

1. **`AGENTS.md`** — loaded automatically. The rules and invariants.
2. **`PROGRESS.md`** — the living state of the project. Which tasks are done, which is in flight, what is blocked, what was learned.
3. **`docs/TASKS.md`** — the task you are about to work, and only that one.

Do not read `PLAN.md`, `docs/CONTRACTS.md` or `docs/LLAMACPP.md` up front. Open them when the task points you at them. Reading everything every session wastes context you will need for the work.

**If `PROGRESS.md` shows a task `In progress`,** that is your task — a previous session started it. Check the branch, read what it left, continue. Do not start something new.

**Task numbers changed in v6.0.** If a branch name or an old note uses a number that no longer matches, the map is at the top of `docs/TASKS.md`. Trust the map, not the branch name.

## 2. Choosing the next task

When nothing is in progress, take **the lowest-numbered task whose dependencies are all `Done` and which is not `Blocked`**.

That rule is mechanical on purpose. Do not optimize the order, do not batch related tasks because they seem to go together, do not skip ahead to something more interesting. Dependencies encode real constraints and the numbering encodes intended sequence.

If the rule selects nothing — everything available is blocked — stop and write that in `PROGRESS.md`. Do not invent work.

### One decision gate

**T-033 must not start until T-025 has recorded `registration_channel` in Facts established.** T-023 answers from `--help` whether `--models-preset` can declare a model by absolute path (`PLAN.md` §2.1); **T-025 answers it by running the binary, and its result wins** — including when the two agree, because one is an observation and the other is a reading. T-033 generates a different configuration depending on the answer, and a guess produces a file that silently exposes the wrong catalogue.

If the value is still `Undetermined` after T-025, T-033 is `Blocked` — write it in Discrepancies and take the next task. At that point it is genuinely an owner decision, because the binary itself declined to answer.

This is the only gate of its kind in the project. Everywhere else the dependency list is sufficient.

## 3. Working a task

```
git checkout -b t-030-gguf-header-reader
```

Branch name is the task id plus a slug. One task per branch, one branch per PR, always.

Then, in order:

1. Re-read the task's acceptance criteria. They are the specification; the prose above them is context.
2. Open the documents the task references. Nothing else.
3. Check every llama.cpp flag you intend to emit against `docs/verified-flags.md` (`AGENTS.md` §1).
4. If the task touches `core/endpoint/`, re-read `AGENTS.md` invariant 3 and `PLAN.md` §2.7 first. That module is one refactor away from becoming a router, and the invariant is the only thing standing in the way.
5. Write the tests and the implementation together.
6. Run the full local gate from `docs/DEV-SETUP.md` before opening the PR.

**Scope discipline.** If you notice something broken outside your task, do not fix it. Write it in the Observations section of `PROGRESS.md` and leave it. A PR that fixes two things is a PR that cannot be reverted cleanly.

## 4. Closing a task

1. Fill in `.github/pull_request_template.md` completely. An unchecked box means the task is not done.
2. Update `PROGRESS.md`: move the task to `Done`, add anything future-you needs to know.
3. Open the PR.

**What belongs in `PROGRESS.md`, and what does not.** It carries state and discoveries — things a fresh session could not reconstruct from the code. It does not carry a narrative of what you did; the diff already says that.

Worth recording: a llama.cpp flag that turned out different from `docs/LLAMACPP.md`; the `registration_channel` result; a library that behaved unexpectedly; a decision you made inside a task that a later task will need to match; a test that is flaky and why.

Not worth recording: "implemented the parser", "added tests", "refactored for clarity".

## 5. When you cannot proceed

Three situations, three different responses. Getting these apart matters — the common failure is treating all of them as the third.

### Reality differs from the documents

A flag does not exist, a type is different, an API behaves unlike what is written here.

**Do not adapt silently.** Stop, and write an entry in the Discrepancies section of `PROGRESS.md`:

```markdown
### D-00x — `--flash-attn` takes on|off|auto, docs assume boolean
- Type: discrepancy
- Found in: T-033, build b9196
- Evidence: docs/verified-flags.md line 22
- Affects: CONTRACTS.md §1 FlashAttn, T-035 launch tab
- Proposed: keep the tri-state enum, update LLAMACPP.md §2
- Status: open
```

(`D-00x` is a placeholder. Take the next free number in `PROGRESS.md` — D-001 through D-003 are in use.)

Then stop working that task. A discrepancy is a decision for the project owner, not something to route around. Pick up the next unblocked task if there is one.

### The task is genuinely ambiguous

Two readings of an acceptance criterion, both defensible, leading to different code.

Write it in Discrepancies the same way, marked `ambiguity`. Do not pick one and proceed hoping it was right — an ambiguity resolved silently becomes a wrong assumption baked into everything downstream.

### Something needs the target hardware

You have hit a property that cannot be confirmed without a GPU, real model files, or a real llama-server under load.

**This is not a blocker** (`AGENTS.md` §3). Assert the structural property you can test, finish the task, and note the empirical question in the PR description. It goes to `docs/owner-verification.md`, which is the owner's file — do not edit it yourself and do not wait on it.

Note the boundary, and note that it sits further out than it looks: downloading a CPU build of llama-server to capture `--help`, to reproduce a stderr message, or to **watch how the router registers models** is not hardware-dependent. That is T-023, T-043 and T-025. Only what genuinely needs a GPU, a real model file, or a real third-party client goes to the owner. If a question in `docs/owner-verification.md` looks answerable without those, that is a Discrepancy worth raising — the classification has been wrong before.

## 6. What you never do without being asked

- Change `AGENTS.md`, `PLAN.md`, or any ADR. They are decisions, not working notes. Propose a change in Discrepancies instead. **"Without being asked" now has a precise meaning: a correction task, §7.** If a `T-1xx` on your branch names the document and the discrepancy, editing it is the work; outside that, it is not.
- Edit `docs/owner-verification.md`.
- Delete or rewrite `PROGRESS.md` history. Pruning it is a correction task and follows the rules at the top of that file — in particular, Facts established is never pruned.
- Add a dependency not in `PLAN.md` §3.
- Write into `docs/` from application code. Generated documentation is produced by scripts, run deliberately.
- Start a second task before the current one is closed.

## 7. Correction tasks (`T-1xx`)

A discrepancy ends with an owner decision. Nothing in the protocol said what applies it, so decisions stayed as prose in `PROGRESS.md` while the documents they corrected stayed wrong.

**The owner decides; you redraft.** Those are different jobs, and only the first needs a person.

So a resolved discrepancy produces a **numbered task in the `T-1xx` range**, listed in the Corrections section at the end of `docs/TASKS.md`. It is selected, branched, reviewed and merged exactly like any other task — the selection rule in §2 already handles it, since `T-1xx` sorts after everything in the milestones and therefore never displaces feature work.

**Rules, and they are the whole mechanism:**

- **A correction task names the discrepancy it closes**, and that discrepancy stays `open` until the task is `Done`. A `D-00x` marked `resolved` with no correction task behind it is the failure this exists to prevent — the decision was made and never landed.
- **A correction task that touches `PLAN.md` §2 or an ADR requires the owner's decision in writing**, not their approval of the idea. You are drafting the consequence of a choice, not making it. If the decision is not written down, that is what you report.
- **Its scope is the documents.** A correction task does not also change code. If the decision implies a code change, it produces a second task, in the ordinary range if it belongs to a milestone.
- **Milestone-boundary review is a correction task too.** The pass that produced v6.1, v6.2 and v6.3 reads every document together, which §1 tells you not to do in an ordinary session for good reason: it costs the context you need for the work. It is a different activity and it is commissioned as one, at the close of a milestone, with the documents as its only subject.

**What this does not cover.** T-006 checks that the documents agree with each other. It cannot check that they agree with reality — a `PLAN.md` describing a world that no longer exists is internally consistent and passes every check. That gap is closed by discrepancies being raised, which depends on you stopping when something does not match rather than adapting to it. The lint is the floor, not the ceiling.
