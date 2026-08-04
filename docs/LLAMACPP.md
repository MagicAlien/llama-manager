# llama.cpp — flags and endpoints

**Every line in this document is a claim to be verified, not an established fact.** That is not a disclaimer; it is the document's purpose. `AGENTS.md` §1 exists because llama.cpp renames, retypes and removes flags between builds, and because documentation lags binaries.

**Before writing code against anything here, check it against `docs/verified-flags.md`**, which is generated from a real `llama-server.exe --help` capture. If a flag is absent there, or takes a different value type, **stop and record a discrepancy** — do not adapt this document, do not guess a replacement, do not drop the feature.

Each entry carries a confidence marker:

| Marker | Meaning |
|---|---|
| **[doc]** | Present in current upstream documentation or a `--help` output seen during planning. Still unverified against *our* pinned build. |
| **[assumed]** | Modelled in `LaunchParams` or `ServerConfig` and believed to exist, but not seen in any capture. **Treat as unverified until T-023 confirms it.** |

Nothing here is marked "verified". That marker only exists in `docs/verified-flags.md`, and only T-023 can put it there.

---

## 1. Router mode

Router mode is the foundation of this project (`PLAN.md` §2.3). It is entered by starting `llama-server` **without** `-m` or `--model`.

| Flag | Value | Notes |
|---|---|---|
| `--models-dir PATH` | path | **[doc]** Directory scanned for GGUF files. Env: `LLAMA_ARG_MODELS_DIR`. |
| `--models-preset PATH` | path | **[doc]** INI file of per-model presets. Env: `LLAMA_ARG_MODELS_PRESET`. Whether an entry can *introduce* a model by absolute path, rather than only carry settings for one the scan found, is the open question in `PLAN.md` §2.1 — settled by T-023 from the capture and confirmed by T-025 against the running binary; T-025's result is the one that binds. |
| `--models-max N` | integer | **[doc]** Maximum models resident simultaneously; LRU eviction beyond it. Env: `LLAMA_ARG_MODELS_MAX`. |
| `--models-autoload` / `--no-models-autoload` | boolean pair | **[doc]** Whether a request for an unloaded model triggers its load. Env: `LLAMA_ARG_MODELS_AUTOLOAD`. Note the paired negative form — this is a boolean expressed as two flags, not a flag taking a value. |
| *idle unload* — **spelling unknown** | seconds? | **[assumed]**, and weaker than the rest of this section: `ServerConfig.idle_unload_seconds` is modelled and shown in T-050, but **no spelling for this flag has been seen in any capture or document**. T-023 looks for it; if the capture has nothing, the field is dropped rather than left as a setting with no mechanism. See `PROGRESS.md` D-003. |

**Directory grammar under `--models-dir`** (**[doc]**, and see `PROGRESS.md` F-000):

- Single-file models sit directly in the scanned directory.
- A model made of several files — multi-shard, or multimodal with a projector — sits in **its own subdirectory**.
- The projector file's name must **begin with `mmproj`**. A naming convention, not a flag.
- Every subdirectory is treated as a candidate model. There is no exclude flag, so a subdirectory created for organisation becomes a phantom entry.
- Scan depth is one level below the scanned directory. An upstream request to increase it is open.

**How a projector is declared in a preset entry is unknown** (T-025 question 4). The `mmproj` filename convention above is documented for the `--models-dir` scan; whether a `PresetDeclaresPath` entry honours the same convention, requires an explicit key, or cannot express a projector at all is unestablished, and T-033 must emit a vision model on that channel.

**Adding a model requires a restart** (**[doc]**). Consistent with the no-hot-reload decision in `docs/CONTRACTS.md` §2.

---

## 2. Server and binding

| Flag | Value | Notes |
|---|---|---|
| `--host ADDR` | address | **[doc]** The app always passes `127.0.0.1` (`PLAN.md` §2.7). Never user-configurable. |
| `--port N` | integer | **[doc]** The app passes the ephemeral upstream port it allocated, never the user's listen port. |
| `--no-webui` | boolean | **[doc]** Disables llama-server's own web UI. **Always emitted** — `AGENTS.md` §2 excludes a chat interface, and the shipped server must not provide one either. |
| `--api-key KEY` | string | **[doc]** **Never emitted by this app.** Authentication happens at the app's listener (ADR-002); the key must not reach the server or `presets.ini`. Listed here so nobody adds it in good faith. |
| `--jinja` | boolean | **[doc]** Enables Jinja chat-template handling. |

---

## 3. Per-model launch flags

These back `LaunchParams` in `docs/CONTRACTS.md` §1. **Every one is [assumed] unless marked otherwise** — this is the section where a retyped flag will hurt, and where T-033's omit-and-warn behaviour earns its keep.

| Flag | Expected type | `LaunchParams` field |
|---|---|---|
| `-ngl`, `--n-gpu-layers N` | integer **[doc]** | `gpu_layers` |
| `-c`, `--ctx-size N` | integer **[doc]** | `ctx_size` |
| `-b`, `--batch-size N` | integer **[assumed]** | `batch_size` |
| `-ub`, `--ubatch-size N` | integer **[assumed]** | `ubatch_size` |
| `--flash-attn` | **tri-state `on\|off\|auto`** **[assumed]** | `flash_attn` |
| `-ctk`, `--cache-type-k TYPE` | enum **[assumed]** | `cache_type_k` |
| `-ctv`, `--cache-type-v TYPE` | enum **[assumed]** | `cache_type_v` |
| `--n-cpu-moe N` | integer **[assumed]** | `n_cpu_moe` |
| `--tensor-split LIST` | comma list of floats **[assumed]** | `tensor_split` |
| `--main-gpu N` | integer **[doc]** | `main_gpu` |
| `--split-mode MODE` | enum **[doc]** | not modelled in v1 |
| `--no-mmap` | boolean **[assumed]** | `no_mmap` |
| `--mlock` | boolean **[assumed]** | `mlock` |
| `-t`, `--threads N` | integer **[assumed]** | `threads` |
| `--chat-template NAME` | string **[assumed]** | `chat_template` |
| `--mmproj PATH` | path **[assumed]** | `mmproj_path` — but see §1: under `--models-dir` the projector is discovered by filename prefix, not passed as a flag |

**`--flash-attn` is the standing example of why this document is not authoritative.** Recent builds accept `on|off|auto` where older ones took a boolean. `FlashAttn` is modelled as a tri-state enum for that reason, and the type **must** be confirmed against the capture before use. `docs/WORKFLOW.md` §5 uses exactly this case as its worked example of a discrepancy.

---

## 4. Sampling defaults

Backing `SamplingDefaults`. All **[assumed]**.

`--temp`, `--top-p`, `--top-k`, `--min-p`, `--repeat-penalty`, `--presence-penalty`, `--frequency-penalty`, `--seed`.

**These are defaults, not policy.** A client request that specifies its own sampling parameters overrides them. T-035 requires this to be stated in visible helper text rather than a tooltip, because a user who believes these are enforced will spend a long time confused.

---

## 5. HTTP endpoints

Two distinct surfaces, and conflating them is a real hazard.

**Control surface — the app's own calls, made directly to the upstream. These never pass through the endpoint listener.**

| Endpoint | Purpose | Confidence |
|---|---|---|
| `GET /models` | Router registry with load state | **[doc]** — *not* the same as `/v1/models` |
| `GET /props` | Server properties | **[doc]** |
| `GET /health` | Liveness | **[assumed]** — existence is established by T-023 from the capture and confirmed by T-025 question 5, which polls it in the state T-040 actually polls it in; `/props` is the fallback |
| load / unload | Explicit residency control | **[assumed]** — exact paths and shapes are T-041's to establish against the real build |

**Client surface — forwarded, never interpreted.**

`GET /v1/models`, `POST /v1/chat/completions`, `POST /v1/completions`, `POST /v1/embeddings`, and anything else the build exposes.

**The app never parses a body on this surface.** Not to log a model name, not to count tokens, not to improve an error message. `AGENTS.md` invariant 3. Forwarding is by path and method only — which is also why an unrecognised path forwards correctly without this list needing to be complete.

---

## 6. Flags this app must never emit

Not because they are broken, but because emitting them contradicts a decision.

| Flag | Why |
|---|---|
| `--api-key` | Authentication is the app's (ADR-002). The key must never reach the server or a config file on disk. |
| `-hf`, `--hf-repo` | Hugging Face downloading is out of scope for v1 (`AGENTS.md` §2). |
| `--host` with anything but `127.0.0.1` | Exposure is the app's decision at its own listener. The server stays on loopback permanently. |
| `--models-dir` under the `PresetDeclaresPath` channel | Registration is by declared path; a scan alongside it would register models the catalogue does not know about (T-033). |
| Any flag whose spelling was inferred rather than observed | The point of this document is that inference is a hypothesis. T-023 and T-025 turn hypotheses into evidence; nothing else does. |

---

## 7. Maintaining this document

- **It is never edited to match a build.** When a capture contradicts it, that is a discrepancy: record it in `PROGRESS.md` and stop. The owner decides whether the document was wrong or the build changed.
- **`docs/verified-flags.md` is the authority**, generated from a real capture by `scripts/export-verified-flags.ps1`. This file is the hypothesis; that file is the evidence.
- **A flag not in the verified list is omitted at runtime with a warning**, never emitted hopefully (`AGENTS.md` §1, T-033).
- **`extra_args` is the one exception** and passes user-typed strings through unparsed. It is never populated by app logic.
