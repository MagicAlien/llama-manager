# Owner verification

**This file belongs to the project owner. The implementing agent does not edit it, does not tick its boxes, and is never blocked by an unticked one** (`AGENTS.md` §3, `docs/WORKFLOW.md` §6).

It holds the questions that cannot be answered without real hardware, real model files, or a real `llama-server`. Everything a task asserts is structural and testable without them; what lands here is the empirical confirmation that the structure was pointed at the right thing.

**How a finding comes back.** Record the answer here, then add what the code must change as a normal entry in `PROGRESS.md` — a Discrepancy if a document was wrong, a line in Facts established if it was merely unknown. Do not edit task acceptance criteria directly.

---

## What used to be Session 1 is now T-025

Up to v6.2 this file opened with four questions about router registration: whether `--models-preset` can declare a model by absolute path, whether the `--models-dir` scan follows reparse points, what the scan's real depth is, and which health endpoint the build exposes. They were described as "ten minutes with a real binary" and assigned here.

**They were misfiled, and the file that defines the rule is the one that broke it.** `AGENTS.md` §3 says a question belongs to the owner only when it needs real hardware. None of those four does: `AGENTS.md` already has the agent downloading a CPU build itself for T-023, T-025 and T-043, registration is not loading (`PLAN.md` §2.1) so no model ever has to occupy VRAM, and every answer is a query to `/v1/models`, `/health` or `/props`. They are now **T-025**, with a fifth question added about how a projector is expressed on the preset channel.

What that leaves for the owner is the residue T-025 reports rather than assumes — most likely one of two things: the router rejecting a synthetic GGUF whose tensor regions are sparse zeroes, or file-symlink creation being unavailable without Developer Mode. If either happens, T-025 raises a Discrepancy naming it and *that specific obstacle* comes here.

**The general lesson, since it will recur.** Deciding a question is the owner's has a cost that is easy to miss: it does not get answered by the next agent session, it gets answered when a person next sits down, and everything downstream waits. Before adding anything to this file, check what it actually requires — a GPU, a real model, a real client — rather than what it feels like it requires.

---

## Fixtures only you can capture

The agent captures `--help` and the failures reproducible without a GPU. These need the hardware.

- [ ] **`llama-server --help` from the GPU build**, in addition to the CPU captures — confirm the flag surface does not differ by backend
- [ ] **stderr on a missing CUDA DLL** — rename or remove the runtime DLL and start
- [ ] **stderr on a driver/runtime mismatch**
- [ ] **stderr on VRAM exhaustion** — load a model deliberately too large for the card
- [ ] **stderr on an unsupported architecture**

Commit each as a file under `src-tauri/fixtures/stderr/`, replacing the hand-written placeholder and **clearing its `unverified` marker**. T-043 has a test asserting the markers are accurate, so a replaced fixture that keeps its marker fails the build.

---

## Estimator accuracy

The model is written down in `docs/CONTRACTS.md` §1 with explicit constants: `C_compute = 2.0`, `C_context = 512 MiB`, `C_host = 256 MiB`, `margin = 0.12`. **The constants are yours to correct.** They are a starting point, and correcting them is expected rather than a sign something went wrong.

For each model measured, record: architecture, quantization, `block_count`, attention head counts, `gpu_layers`, `ctx_size`, cache types, **predicted VRAM**, **observed VRAM at steady state**, and load time.

- [ ] A dense model that fits entirely in VRAM
- [ ] The same model at two very different context sizes — isolates the KV cache term
- [ ] The same model with a quantized KV cache — confirms the direction and size of that lever
- [ ] A Grouped Query Attention model, where `attention_head_count_kv` is much smaller than `attention_head_count` — **the term most likely to be wrong**, and the one where being wrong is worst
- [ ] An MoE model with CPU offload
- [ ] An NVFP4 model
- [ ] A 65 GB model with partial offload

**What to look for.** Systematic error in one direction means a constant is wrong. Error that grows with context means the KV term is wrong. Error that grows with layer count means the weights term is wrong. Random error means the model is right and something else is moving.

**If any estimate came in *under* the observed usage, that is the serious case** — the margin exists to make underestimation rare, because it is the one that fails a 65 GB load minutes in.

---

## NVFP4 on the shipped build

The app labels NVFP4 `experimental` and asserts no acceleration (`PLAN.md` §2.6), because capability cannot be read from the flag list.

- [ ] Does the active build dispatch NVFP4 to Blackwell tensor cores, or fall back to a generic path?
- [ ] Is throughput materially different from a comparable Q4 quantization on the same card?
- [ ] Does anything in the startup output or `/props` distinguish the two?

The third is the useful one: if something reports it, promoting NVFP4 out of `experimental` becomes a v1.1 task with a real capability probe instead of a hopeful label.

---

## Router behaviour under pressure

The open risk in `PLAN.md` §2.3, and the reason `ModelOrchestrator` is a trait.

- [ ] With `--models-max` set, does LRU eviction behave predictably when a load would exceed VRAM?
- [ ] Does a load that cannot fit **return an error**, or hang?
- [ ] What does a request naming a model receive while that model is still loading?
- [ ] Does hot-swap latency match what the UI implies?

**A hang rather than an error is the finding that matters.** The app can present an error; it cannot present a hang. If loads hang under pressure, the `llama-swap` contingency stops being insurance and becomes a decision.

---

## Client interoperability

Asserted against the OpenAI schema in T-052, which is not the same as working with real clients.

- [ ] A mainstream OpenAI-compatible client lists models and completes a request **through the app's endpoint**
- [ ] Streaming arrives token by token, not as one delayed block — the failure mode ADR-002 is most worried about
- [ ] A request naming an unloaded model triggers a JIT load and is served
- [ ] With an API key set, an unauthenticated client is rejected cleanly rather than hanging
- [ ] The configured address survives a build switch and a crash without touching client config

---

## End-to-end on the real machine

- [ ] Clean install on Windows 11 + Blackwell, no terminal, no external runtime dependencies
- [ ] A 65 GB model loads with partial offload, and the progress shown resembles what is happening
- [ ] Preloading shows real progress rather than an apparently frozen bar (`PLAN.md` §2.12)
- [ ] Quitting from the tray leaves no `llama-server` process alive and no port bound
- [ ] Logging off Windows with the server running leaves no orphan
- [ ] Uninstall removes the app and its data directory and **no model file**
