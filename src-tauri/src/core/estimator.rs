//! VRAM estimator — pure function per AGENTS.md invariant 5.
//!
//! Implements the estimation model stated in `docs/CONTRACTS.md` §1, "The
//! estimation model", with the T-038 corrections to its KV term and its
//! speculative-decoding terms. No database access; history is an argument
//! pre-filtered by the caller to records matching this model's absolute path.
//!
//! # Where the numbers come from
//!
//! Every coefficient and every structural rule below was either read from a
//! GGUF file on this machine or **measured against the pinned build's own
//! accounting** — `llama-server.exe` (b10883) loading the owner's real
//! Qwen3.8-27B and printing its `llama_kv_cache` / `llama_memory_recurrent`
//! buffer sizes (PROGRESS.md F-019). Where the documented model in
//! `docs/CONTRACTS.md` §1 disagrees with what the build does, the build wins
//! here and the disagreement is recorded as D-017: the document states a KV
//! term sized for *every* layer with 1 byte per `q8_0` element and a head
//! dimension derived by division, and each of those three is measurably wrong
//! on a real hybrid model.

use super::types::{EstimateConfidence, EstimateInputs, GgufMetadata, LaunchRecord, VramEstimate};

// ─── Constants from the estimation model (CONTRACTS.md §1) ───────────────────

/// Compute buffer coefficient.
const C_COMPUTE: f64 = 2.0;

/// CUDA context, allocator, cuBLAS workspaces overhead.
const C_CONTEXT: u64 = 512 * 1024 * 1024; // 512 MiB

/// Host-side overhead.
const C_HOST: u64 = 256 * 1024 * 1024; // 256 MiB

/// Conservative margin applied to the GPU estimate.
const MARGIN: f64 = 0.12;

/// Widened margin when metadata is missing and defaults are applied.
const WIDENED_MARGIN: f64 = 0.25;

/// Default head_dim when metadata is missing (for architectures where it's known).
const DEFAULT_HEAD_DIM: u32 = 128;

/// Default attention_head_count_kv when missing (for architectures where it's known).
const DEFAULT_ATTENTION_HEAD_COUNT_KV: u32 = 8;

/// The element type both halves of a recurrent layer's state are allocated in
/// (llama.cpp `llama_memory_recurrent` prints `R (f32)` and `S (f32)` — F-019).
const RECURRENT_BYTES_PER_ELEMENT: u64 = 4;

/// `--spec-draft-n-max`'s own default in the pinned build. Taken from b10883's
/// `--help` ("number of tokens to draft for speculative decoding (default:
/// 3)") and confirmed by the build's log: with the flag unset the recurrent
/// state is allocated for 1 + 3 sequences, and it follows the flag when set
/// (n_max = 1 → 2 sequences, n_max = 5 → 6). It is the build's default, not
/// this project's choice — the same class of captured value as
/// `core::speculative::SPEC_TYPE_VALUES`.
const SPEC_DRAFT_N_MAX_DEFAULT: u32 = 3;

/// Parameters for VRAM estimation calculation.
struct VramCalcParams<'a> {
    file_size_bytes: u64,
    gpu_layers: u32,
    block_count: u32,
    ctx_size: u32,
    attention_head_count_kv: u32,
    /// Per-head K and V dimensions (T-038). Priced independently, because they
    /// are independent in the header (`attention.key_length` /
    /// `attention.value_length`) and not always equal.
    key_dim: u32,
    value_dim: u32,
    /// `{arch}.full_attention_interval`, when the model is a hybrid (T-038).
    full_attention_interval: Option<u32>,
    cache_type_k: Option<&'a str>,
    cache_type_v: Option<&'a str>,
    ubatch_size: u32,
    embedding_length: u32,
    margin: f64,
    projector_bytes: u64,
    /// The recurrent layers' state, already sized for the configured draft
    /// stage (T-038). Independent of the context and of the layer split.
    recurrent_bytes: u64,
    /// Total cost of the draft companion, weights plus its own KV cache, or 0
    /// when none is configured (T-038).
    draft_bytes: u64,
    /// The MTP draft layers' own KV cache (T-038).
    mtp_bytes: u64,
}

/// Bytes per element for different cache quantizations.
///
/// The `q8_0` figure is measured, not assumed: llama.cpp stores a `q8_0` block
/// as 32 quantized bytes plus a 2-byte scale, so an element costs 8.5 bits
/// (1.0625 bytes) and not 8. The build's own accounting shows it — a 512-cell,
/// 16-layer K cache reports 8.50 MiB where a flat 1 byte per element predicts
/// 8.00 MiB (F-019). `q4_0` is 18 bytes per 32 elements = 0.5625, which the
/// same measurement confirms at 4.50 MiB.
fn bytes_per_element(cache_type: Option<&str>) -> f64 {
    match cache_type {
        Some("f16") => 2.0,
        Some("q8_0") => 1.0625,
        Some("q4_0") => 0.5625,
        _ => 2.0, // f16 — the app's own default for an unset field
    }
}

/// The K and V dimension of one attention head, and whether either was
/// assumed rather than read.
struct HeadDims {
    key: u32,
    value: u32,
    assumed: bool,
}

/// Per-head dimensions, preferring the header's own `attention.key_length` /
/// `attention.value_length` over the division `embedding_length /
/// attention_head_count`.
///
/// The division is the documented fallback and it is a poor one for real
/// files: the owner's Qwen3.8-27B declares `embedding_length = 5120` with 24
/// query heads, and each head is 256 wide — the division yields 213, 20% under
/// the truth. llama.cpp itself reads the declared lengths (`print_info:
/// n_embd_head_k = 256`, F-019).
fn head_dims(metadata: &GgufMetadata) -> HeadDims {
    let division = match (metadata.embedding_length, metadata.attention_head_count) {
        (Some(emb), Some(heads)) if heads > 0 => Some(emb / heads),
        _ => None,
    };
    let key = metadata.attention_key_length.or(division);
    let value = metadata.attention_value_length.or(division);
    HeadDims {
        key: key.unwrap_or(DEFAULT_HEAD_DIM),
        value: value.unwrap_or(DEFAULT_HEAD_DIM),
        assumed: key.is_none() || value.is_none(),
    }
}

/// How many of the offloaded layers actually hold a KV cache.
///
/// A hybrid model (Qwen3-Next-style) declares `{arch}.full_attention_interval`
/// and interleaves a recurrent layer between full-attention ones. Only the
/// full-attention layers hold a KV cache, so sizing the term for every layer
/// overestimates it by the interval — 4× on the owner's files, which is the
/// "too large" reading T-038 was opened for. Measured on the real build:
/// `full_attention_interval = 4`, `block_count = 65` → `llama_kv_cache: …
/// 16 layers` while the recurrent memory accounts for the other 48 (F-019).
///
/// The offloaded share is taken proportionally. Which layers llama.cpp
/// offloads (a contiguous window) is not modelled, so a *partial* offload of a
/// hybrid model is approximate; a full offload — the default this app
/// generates — is exact.
fn kv_layer_count(offloaded_layers: u32, interval: Option<u32>) -> u32 {
    match interval {
        Some(n) if n > 1 => offloaded_layers / n,
        _ => offloaded_layers,
    }
}

/// Compute the KV cache size in bytes for the layers that hold one.
///
/// Formula (F-019, verified byte-exact against the build's own report):
/// K bytes = ctx_size × kv_layers × attention_head_count_kv × key_dim × bytes_per_element(cache_type_k)
/// V bytes = ctx_size × kv_layers × attention_head_count_kv × value_dim × bytes_per_element(cache_type_v)
///
/// K and V are priced separately and with their own dimensions: the cache type
/// is a per-side setting, and so is the dimension.
fn kv_cache_bytes(
    ctx_size: u32,
    kv_layers: u32,
    attention_head_count_kv: u32,
    key_dim: u32,
    value_dim: u32,
    cache_type_k: Option<&str>,
    cache_type_v: Option<&str>,
) -> u64 {
    if kv_layers == 0 {
        return 0;
    }

    let k_bytes_per_head = key_dim as f64 * bytes_per_element(cache_type_k);
    let v_bytes_per_head = value_dim as f64 * bytes_per_element(cache_type_v);

    let result = ctx_size as f64
        * kv_layers as f64
        * attention_head_count_kv as f64
        * (k_bytes_per_head + v_bytes_per_head);

    result as u64
}

/// How many layers the recurrent state is allocated for: every repeating layer
/// that is not a full-attention one.
///
/// The MTP layers are excluded first: llama.cpp treats `block_count` as
/// `n_layer_all` and runs the model on `n_layer_all − nextn_predict_layers`
/// repeating layers (`print_info: n_layer = 64, n_layer_all = 65` for the
/// owner's file, F-019).
fn recurrent_layer_count(block_count: u32, interval: Option<u32>, mtp_layers: u32) -> u32 {
    let repeating = block_count.saturating_sub(mtp_layers);
    repeating.saturating_sub(kv_layer_count(repeating, interval))
}

/// The bytes one sequence's recurrent state costs, or `None` when the header
/// does not carry the four keys that size it.
///
/// Formula from llama.cpp itself — `llama_hparams::n_embd_r()` / `n_embd_s()`
/// at the pinned build's commit, both reproducing the build's own reported
/// buffer size exactly (F-019):
///
/// ```text
/// n_embd_s = state_size × inner_size
/// n_embd_r = (conv_kernel − 1) × (inner_size + 2 × group_count × state_size)
/// bytes    = (n_embd_s + n_embd_r) × 4        (f32)
/// ```
///
/// Measured: 786 432 + 30 720 elements → 149.62 MiB for 48 recurrent layers
/// and one sequence, and 598.50 MiB for four — exactly the numbers the build
/// prints.
///
/// This term does **not** grow with the context: a recurrent layer keeps a
/// fixed-size state, which is why it is a separate term and not part of the KV
/// cache.
fn recurrent_state_per_sequence(metadata: &GgufMetadata) -> Option<u64> {
    let state_size = metadata.ssm_state_size?;
    let inner_size = metadata.ssm_inner_size?;
    let group_count = metadata.ssm_group_count?;
    let conv_kernel = metadata.ssm_conv_kernel?;

    let n_embd_s = u64::from(state_size) * u64::from(inner_size);
    let n_embd_r = u64::from(conv_kernel.saturating_sub(1))
        * (u64::from(inner_size) + 2 * u64::from(group_count) * u64::from(state_size));

    Some((n_embd_s + n_embd_r) * RECURRENT_BYTES_PER_ELEMENT)
}

/// The draft companion's total cost — its weights plus its own KV cache — or
/// `None` when the configured companion could not be measured.
///
/// The companion is a second model: it is loaded next to the main one, so both
/// its file and its cache belong in the projection. The weights are counted in
/// full (a draft model is offloaded; `--spec-draft-ngl` is not modelled, and a
/// larger figure is the recoverable direction — see the margin note in
/// `docs/CONTRACTS.md` §1).
///
/// Its cache is sized with the same function as the main model's, from the
/// companion's **own** header — measured: the owner's dflash2 companion
/// (5 layers, 8 KV heads, 128-wide heads) reports 10.00 MiB at 512 cells,
/// which is exactly `512 × 5 × 8 × (128+128) × 2` (F-019).
fn draft_companion_bytes(
    draft: &super::types::DraftModelInputs,
    ctx_size: u32,
    cache_type_k: Option<&str>,
    cache_type_v: Option<&str>,
) -> Option<u64> {
    let size_bytes = draft.size_bytes?;
    let metadata = draft.metadata.as_ref()?;

    let kv_layers = kv_layer_count(metadata.block_count, metadata.full_attention_interval);
    let dims = head_dims(metadata);
    // A draft companion is a small model; an absent KV head count falls back to
    // the same default the main model uses, and the note says so.
    let kv_heads = metadata
        .attention_head_count_kv
        .unwrap_or(DEFAULT_ATTENTION_HEAD_COUNT_KV);

    let kv = kv_cache_bytes(
        ctx_size,
        kv_layers,
        kv_heads,
        dims.key,
        dims.value,
        cache_type_k,
        cache_type_v,
    );

    Some(size_bytes + kv)
}

/// Compute the compute buffer size in bytes.
///
/// Formula: ubatch_size × embedding_length × 4 bytes × C_compute
fn compute_buffer_size(ubatch_size: u32, embedding_length: u32) -> u64 {
    let result = ubatch_size as f64 * embedding_length as f64 * 4.0 * C_COMPUTE;
    result as u64
}

/// Compute estimated VRAM for a given number of GPU layers.
fn estimate_vram_for_layers(params: &VramCalcParams) -> u64 {
    if params.block_count == 0 {
        return 0;
    }

    let weights_gpu =
        params.file_size_bytes as f64 * (params.gpu_layers as f64 / params.block_count as f64);
    let kv_bytes = kv_cache_bytes(
        params.ctx_size,
        kv_layer_count(params.gpu_layers, params.full_attention_interval),
        params.attention_head_count_kv,
        params.key_dim,
        params.value_dim,
        params.cache_type_k,
        params.cache_type_v,
    ) as f64;
    let compute_buffer = compute_buffer_size(params.ubatch_size, params.embedding_length) as f64;
    // The projector (mmproj) is loaded fully into VRAM when the model has
    // vision capability; it is independent of the GPU-layer split.
    let projector = params.projector_bytes as f64;
    // T-038 — the two terms that are independent of how the layers are split
    // but are real VRAM at every split: the recurrent layers' state, and what
    // the draft stage adds (the companion, and the MTP layers' own cache).
    let recurrent = params.recurrent_bytes as f64;
    let draft = params.draft_bytes as f64;
    let mtp = params.mtp_bytes as f64;

    let total = weights_gpu
        + kv_bytes
        + compute_buffer
        + projector
        + recurrent
        + draft
        + mtp
        + C_CONTEXT as f64;
    (total * (1.0 + params.margin)) as u64
}

/// Compute estimated RAM for a given number of GPU layers.
fn estimate_ram_for_layers(file_size_bytes: u64, gpu_layers: u32, block_count: u32) -> u64 {
    if block_count == 0 {
        return C_HOST;
    }

    let weights_gpu = file_size_bytes as f64 * (gpu_layers as f64 / block_count as f64);
    let weights_cpu = file_size_bytes as f64 - weights_gpu;
    (weights_cpu + C_HOST as f64) as u64
}

/// The draft-stage numbers the estimate needs: what the stage adds to the
/// estimate, and how it multiplies the recurrent state.
struct DraftStage {
    /// The companion's total cost when it could be measured.
    companion_bytes: u64,
    /// The companion's path when it is configured but could not be measured —
    /// the estimate then says so instead of counting it as zero.
    unaccounted: Option<String>,
    /// Copies of the recurrent state the draft stage keeps alive.
    /// `1 + --spec-draft-n-max` while drafting, exactly 1 otherwise (F-019).
    state_multiplier: u64,
}

/// Resolve the draft stage from the launch configuration.
///
/// Precedence matches `core::speculative::spec_type_for_entry`, which is what
/// the preset generator emits: an explicit companion wins over the model's own
/// MTP heads, and an MTP model drafts with `--spec-type draft-mtp` whether or
/// not the user touched the speculative settings.
fn draft_stage(
    inputs: &EstimateInputs,
    ctx_size: u32,
    mtp_layers: u32,
    draft_cache_type_k: Option<&str>,
    draft_cache_type_v: Option<&str>,
) -> DraftStage {
    let n_max = inputs
        .params
        .speculative
        .as_ref()
        .and_then(|spec| spec.n_max)
        .unwrap_or(SPEC_DRAFT_N_MAX_DEFAULT);

    match inputs.draft.as_ref() {
        Some(draft) => {
            let companion_bytes =
                draft_companion_bytes(draft, ctx_size, draft_cache_type_k, draft_cache_type_v);
            DraftStage {
                companion_bytes: companion_bytes.unwrap_or(0),
                unaccounted: if companion_bytes.is_none() {
                    Some(draft.path.display().to_string())
                } else {
                    None
                },
                state_multiplier: u64::from(n_max) + 1,
            }
        }
        // No companion: the model's own MTP heads draft, each in its own
        // layer, and the draft stage keeps one state per drafted token.
        None if mtp_layers > 0 => DraftStage {
            companion_bytes: 0,
            unaccounted: None,
            state_multiplier: u64::from(n_max) + 1,
        },
        None => DraftStage {
            companion_bytes: 0,
            unaccounted: None,
            state_multiplier: 1,
        },
    }
}

/// Find the largest number of GPU layers that fits in available VRAM.
fn recommend_gpu_layers(inputs: &EstimateInputs, params: &VramCalcParams) -> u32 {
    let block_count = params.block_count;
    if block_count == 0 {
        return 0;
    }

    let mut candidate = VramCalcParams {
        file_size_bytes: params.file_size_bytes,
        gpu_layers: 0,
        block_count: params.block_count,
        ctx_size: params.ctx_size,
        attention_head_count_kv: params.attention_head_count_kv,
        key_dim: params.key_dim,
        value_dim: params.value_dim,
        full_attention_interval: params.full_attention_interval,
        cache_type_k: params.cache_type_k,
        cache_type_v: params.cache_type_v,
        ubatch_size: params.ubatch_size,
        embedding_length: params.embedding_length,
        margin: params.margin,
        projector_bytes: params.projector_bytes,
        recurrent_bytes: params.recurrent_bytes,
        draft_bytes: params.draft_bytes,
        mtp_bytes: params.mtp_bytes,
    };

    // Binary search for the largest n that fits. Every term other than the
    // weights and the KV cache is present at every n, so the draft companion
    // and the recurrent state are inside the comparison — which is the point:
    // a layer count that only fits while the companion is ignored is not a
    // recommendation this project may make.
    let mut low = 0;
    let mut high = block_count;
    while low < high {
        let mid = (low + high).div_ceil(2);
        candidate.gpu_layers = mid;
        let vram = estimate_vram_for_layers(&candidate);
        if vram <= inputs.vram_free_bytes {
            low = mid;
        } else {
            high = mid - 1;
        }
    }

    low
}

/// Check if history matches the current configuration.
fn history_matches<'a>(
    history: &'a [LaunchRecord],
    inputs: &EstimateInputs,
) -> Vec<&'a LaunchRecord> {
    let gpu_layers = inputs.params.gpu_layers;
    let ctx_size = inputs.params.ctx_size;
    let cache_type_k = inputs.params.cache_type_k.clone();
    let cache_type_v = inputs.params.cache_type_v.clone();

    history
        .iter()
        .filter(|record| {
            record.succeeded
                && record.actual_vram_bytes.is_some()
                && record.params.gpu_layers == gpu_layers
                && record.params.ctx_size == ctx_size
                && record.params.cache_type_k == cache_type_k
                && record.params.cache_type_v == cache_type_v
        })
        .collect()
}

/// Compute the bias term from non-matching history.
fn compute_bias(history: &[LaunchRecord], inputs: &EstimateInputs, params: &VramCalcParams) -> f64 {
    let matching = history_matches(history, inputs);
    if matching.is_empty() {
        return 1.0;
    }

    // Use the most recent 3 matching records
    let recent: Vec<&LaunchRecord> = matching.iter().rev().take(3).cloned().collect();

    if recent.is_empty() {
        return 1.0;
    }

    // Average the ratio of actual to estimated
    let mut total_ratio = 0.0;

    for record in &recent {
        let Some(actual) = record.actual_vram_bytes else {
            continue;
        };
        let candidate = VramCalcParams {
            file_size_bytes: params.file_size_bytes,
            gpu_layers: record.params.gpu_layers.unwrap_or(0),
            block_count: params.block_count,
            ctx_size: record.params.ctx_size.unwrap_or(params.ctx_size),
            attention_head_count_kv: params.attention_head_count_kv,
            key_dim: params.key_dim,
            value_dim: params.value_dim,
            full_attention_interval: params.full_attention_interval,
            cache_type_k: record.params.cache_type_k.as_deref(),
            cache_type_v: record.params.cache_type_v.as_deref(),
            ubatch_size: record.params.ubatch_size.unwrap_or(params.ubatch_size),
            embedding_length: params.embedding_length,
            margin: MARGIN,
            projector_bytes: params.projector_bytes,
            recurrent_bytes: params.recurrent_bytes,
            draft_bytes: params.draft_bytes,
            mtp_bytes: params.mtp_bytes,
        };
        let estimated = estimate_vram_for_layers(&candidate);
        if estimated > 0 {
            total_ratio += actual as f64 / estimated as f64;
        }
    }

    total_ratio / recent.len() as f64
}

/// Main estimation function.
///
/// Implements the model from `docs/CONTRACTS.md` §1 with T-038's corrections.
///
/// # Parameters
/// - `inputs`: Model metadata, file size, launch parameters, available VRAM/RAM
/// - `history`: Previous launch records for this model (pre-filtered by path)
///
/// # Returns
/// A `VramEstimate` with recommended GPU layers, estimated VRAM/RAM usage,
/// confidence level, and explanatory notes.
pub fn estimate(inputs: &EstimateInputs, history: &[LaunchRecord]) -> VramEstimate {
    let mut notes = Vec::new();
    let metadata = &inputs.metadata;

    // ─── The draft stage, resolved first: it decides what is unaccounted ────
    // The draft-stage cache types are the user's `--spec-draft-type-k/v`, whose
    // own default is f16 (measured: the MTP layer's cache and the companion's
    // cache both report f16 while the main cache is f16 by default too).
    let (draft_ctk, draft_ctv) = match inputs.params.speculative.as_ref() {
        Some(spec) => (spec.cache_type_k.as_deref(), spec.cache_type_v.as_deref()),
        None => (None, None),
    };
    let mtp_layers = metadata.mtp_layer_count.unwrap_or(0);
    let ctx_size_for_draft = inputs.params.ctx_size.unwrap_or(2048);
    let draft = draft_stage(inputs, ctx_size_for_draft, mtp_layers, draft_ctk, draft_ctv);

    // ─── Metadata availability ────────────────────────────────────────────
    let dims = head_dims(metadata);
    let mut metadata_incomplete = false;

    if dims.assumed {
        notes.push(format!(
            "Assumed head dimension: {DEFAULT_HEAD_DIM} (the header declares neither attention.key_length/value_length nor a usable embedding_length / attention_head_count pair)"
        ));
        metadata_incomplete = true;
    }

    let attention_head_count_kv = match metadata.attention_head_count_kv {
        Some(kv) => kv,
        None => {
            notes.push(format!(
                "Assumed attention_head_count_kv: {DEFAULT_ATTENTION_HEAD_COUNT_KV} (metadata missing)"
            ));
            metadata_incomplete = true;
            DEFAULT_ATTENTION_HEAD_COUNT_KV
        }
    };

    // A companion that cannot be read makes the estimate knowingly incomplete;
    // say so and widen the margin rather than reporting a tidy number built on
    // a missing term.
    if let Some(path) = draft.unaccounted.as_deref() {
        notes.push(format!(
            "Draft companion {path} could not be measured; its memory is NOT included in this estimate"
        ));
        metadata_incomplete = true;
    }

    let margin = if metadata_incomplete {
        WIDENED_MARGIN
    } else {
        MARGIN
    };
    notes.push(format!("Conservative margin: {:.0}%", margin * 100.0));

    // ─── The two layer-independent terms (T-038) ──────────────────────────
    let kv_layers_total = kv_layer_count(metadata.block_count, metadata.full_attention_interval);
    let recurrent_layers = recurrent_layer_count(
        metadata.block_count,
        metadata.full_attention_interval,
        mtp_layers,
    );

    let recurrent_bytes = match recurrent_state_per_sequence(metadata) {
        Some(per_sequence) => {
            let total = per_sequence * u64::from(recurrent_layers) * draft.state_multiplier;
            if total > 0 {
                notes.push(format!(
                    "Recurrent (non-attention) layers: {recurrent_layers} of {} · state {}{}",
                    metadata.block_count,
                    format_bytes(total),
                    if draft.state_multiplier > 1 {
                        format!(
                            " (×{} — the draft stage keeps one state per drafted token)",
                            draft.state_multiplier
                        )
                    } else {
                        String::new()
                    }
                ));
            }
            total
        }
        None => 0,
    };

    // MTP layers run in their own draft context, with their own KV cache —
    // measured: 1 layer, 2.00 MiB at 512 cells (F-019).
    let mtp_bytes = if draft.companion_bytes == 0 && draft.unaccounted.is_none() && mtp_layers > 0 {
        let bytes = kv_cache_bytes(
            ctx_size_for_draft,
            mtp_layers,
            attention_head_count_kv,
            dims.key,
            dims.value,
            draft_ctk,
            draft_ctv,
        );
        if bytes > 0 {
            notes.push(format!(
                "MTP draft layers: {mtp_layers} × KV cache = {} (their own context, alongside the model's)",
                format_bytes(bytes)
            ));
        }
        bytes
    } else {
        0
    };

    if draft.companion_bytes > 0 {
        notes.push(format!(
            "Draft companion counted: {} in total (its weights plus its own KV cache at this context size)",
            format_bytes(draft.companion_bytes)
        ));
    }

    if metadata.full_attention_interval.is_some_and(|n| n > 1) {
        notes.push(format!(
            "Hybrid architecture: every {}th layer holds a KV cache ({kv_layers_total} of {}); the others keep a fixed-size state, counted separately",
            metadata.full_attention_interval.unwrap_or(1),
            metadata.block_count
        ));
    }

    // ─── The layer-dependent part ─────────────────────────────────────────
    let ctx_size = inputs.params.ctx_size.unwrap_or(2048);
    let ubatch_size = inputs.params.ubatch_size.unwrap_or(512);
    let embedding_length = metadata.embedding_length.unwrap_or(4096);

    let mut params = VramCalcParams {
        file_size_bytes: inputs.file_size_bytes,
        gpu_layers: 0,
        block_count: metadata.block_count,
        ctx_size,
        attention_head_count_kv,
        key_dim: dims.key,
        value_dim: dims.value,
        full_attention_interval: metadata.full_attention_interval,
        cache_type_k: inputs.params.cache_type_k.as_deref(),
        cache_type_v: inputs.params.cache_type_v.as_deref(),
        ubatch_size,
        embedding_length,
        margin,
        projector_bytes: inputs.projector_bytes,
        recurrent_bytes,
        draft_bytes: draft.companion_bytes,
        mtp_bytes,
    };

    // Determine GPU layers
    let gpu_layers = match inputs.params.gpu_layers {
        Some(layers) => layers,
        None => {
            params.gpu_layers = 0;
            let recommended = recommend_gpu_layers(inputs, &params);
            notes.push(format!(
                "No layer count chosen: the figures below describe {recommended} GPU layers"
            ));
            recommended
        }
    };
    params.gpu_layers = gpu_layers;

    let kv_bytes = kv_cache_bytes(
        ctx_size,
        kv_layer_count(gpu_layers, metadata.full_attention_interval),
        attention_head_count_kv,
        dims.key,
        dims.value,
        params.cache_type_k,
        params.cache_type_v,
    );

    let estimated_ram =
        estimate_ram_for_layers(inputs.file_size_bytes, gpu_layers, metadata.block_count);

    // ─── Calibration ──────────────────────────────────────────────────────
    let matching = history_matches(history, inputs);
    let confidence = if !matching.is_empty() {
        EstimateConfidence::Calibrated
    } else {
        EstimateConfidence::Heuristic
    };

    let estimated_vram = if confidence == EstimateConfidence::Calibrated {
        let recent: Vec<&LaunchRecord> = matching.iter().rev().take(3).cloned().collect();
        let measured: Vec<u64> = recent.iter().filter_map(|r| r.actual_vram_bytes).collect();
        if measured.is_empty() {
            0
        } else {
            let total: u64 = measured.iter().sum();
            let calibrated = (total / measured.len() as u64) as f64 * (1.0 + margin);
            notes.push(format!(
                "Calibrated from {} recent matching launches",
                measured.len()
            ));
            calibrated as u64
        }
    } else {
        let heuristic = estimate_vram_for_layers(&params);
        let bias = compute_bias(history, inputs, &params);
        if bias != 1.0 {
            notes.push(format!(
                "Bias applied from non-matching history: {bias:.2}x"
            ));
            (heuristic as f64 * bias) as u64
        } else {
            heuristic
        }
    };

    VramEstimate {
        recommended_gpu_layers: gpu_layers,
        estimated_vram_bytes: estimated_vram,
        estimated_ram_bytes: estimated_ram,
        kv_cache_bytes: kv_bytes,
        fits_fully: estimated_vram <= inputs.vram_free_bytes,
        confidence,
        notes,
        vram_total_bytes: inputs.vram_total_bytes,
        vram_free_bytes: inputs.vram_free_bytes,
        projector_bytes: inputs.projector_bytes,
        draft_bytes: draft.companion_bytes,
        recurrent_state_bytes: recurrent_bytes,
        mtp_draft_bytes: mtp_bytes,
    }
}

/// Human-readable size for the notes. Kept local: `notes` are shown verbatim
/// and a bare byte count is unreadable at these magnitudes.
fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const GIB: f64 = 1024.0 * MIB;
    let value = bytes as f64;
    if value >= GIB {
        format!("{:.2} GiB", value / GIB)
    } else {
        format!("{:.0} MiB", value / MIB)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::LaunchParams;

    fn make_metadata() -> GgufMetadata {
        GgufMetadata {
            architecture: "llama".to_string(),
            param_count: Some(7_000_000_000),
            quantization: "Q4_K_M".to_string(),
            block_count: 32,
            context_length: Some(8192),
            embedding_length: Some(4096),
            attention_head_count: Some(32),
            attention_head_count_kv: Some(8),
            attention_key_length: None,
            attention_value_length: None,
            full_attention_interval: None,
            ssm_state_size: None,
            ssm_inner_size: None,
            ssm_group_count: None,
            ssm_conv_kernel: None,
            has_chat_template: false,
            is_moe: false,
            expert_count: None,
            is_draft_model: false,
            has_mtp_heads: false,
            mtp_layer_count: None,
            supports_tools: false,
            supports_thinking: false,
        }
    }

    fn make_inputs(metadata: GgufMetadata, file_size: u64) -> EstimateInputs {
        EstimateInputs {
            metadata,
            file_size_bytes: file_size,
            params: LaunchParams::default(),
            vram_free_bytes: 24 * 1024 * 1024 * 1024, // 24 GB
            vram_total_bytes: 24 * 1024 * 1024 * 1024, // 24 GB
            projector_bytes: 0,
            draft: None,
            ram_free_bytes: 16 * 1024 * 1024 * 1024, // 16 GB
        }
    }

    #[test]
    fn test_estimate_basic() {
        let metadata = make_metadata();
        let inputs = make_inputs(metadata, 4_000_000_000); // 4 GB model
        let estimate = estimate(&inputs, &[]);

        assert!(estimate.estimated_vram_bytes > 0);
        assert!(estimate.estimated_ram_bytes > 0);
        assert_eq!(estimate.confidence, EstimateConfidence::Heuristic);
    }

    #[test]
    fn test_estimate_deterministic() {
        let metadata = make_metadata();
        let inputs = make_inputs(metadata, 4_000_000_000);

        let e1 = estimate(&inputs, &[]);
        let e2 = estimate(&inputs, &[]);

        assert_eq!(e1.estimated_vram_bytes, e2.estimated_vram_bytes);
        assert_eq!(e1.estimated_ram_bytes, e2.estimated_ram_bytes);
        assert_eq!(e1.recommended_gpu_layers, e2.recommended_gpu_layers);
    }

    #[test]
    fn test_estimate_zero_vram() {
        let metadata = make_metadata();
        let mut inputs = make_inputs(metadata, 4_000_000_000);
        inputs.vram_free_bytes = 0;

        let estimate = estimate(&inputs, &[]);
        assert_eq!(estimate.recommended_gpu_layers, 0);
    }

    #[test]
    fn test_kv_cache_bytes() {
        // 2048 × 32 layers × 8 KV heads × (128 × 2 + 128 × 2) = 268,435,456
        let result = kv_cache_bytes(2048, 32, 8, 128, 128, Some("f16"), Some("f16"));
        assert_eq!(result, 268_435_456);
    }

    #[test]
    fn test_kv_cache_bytes_mixed_cache_types() {
        // K as f16 (2.0 bytes/element), V as q8_0 (8.5 bits = 1.0625 bytes)
        // 2048 × 32 × 8 × (128 × 2 + 128 × 1.0625) = 205,520,896
        let result = kv_cache_bytes(2048, 32, 8, 128, 128, Some("f16"), Some("q8_0"));
        assert_eq!(result, 205_520_896);
    }

    #[test]
    fn test_compute_buffer_size() {
        // 512 × 4096 × 4 × 2 = 16,777,216 bytes
        let result = compute_buffer_size(512, 4096);
        assert_eq!(result, 16_777_216);
    }

    #[test]
    fn a_hybrid_model_sizes_the_kv_term_for_its_attention_layers_only() {
        // Measured on the real build: interval 4 over 65 blocks → 16 KV layers
        // (F-019). Sizing all 65 would overestimate this term four-fold.
        assert_eq!(kv_layer_count(65, Some(4)), 16);
        assert_eq!(kv_layer_count(64, Some(4)), 16);
        // A model with no interval key holds a KV cache on every layer.
        assert_eq!(kv_layer_count(65, None), 65);
        assert_eq!(kv_layer_count(65, Some(1)), 65);
    }

    #[test]
    fn the_recurrent_layers_are_the_repeating_layers_that_are_not_attention() {
        // 65 blocks of which 1 is an MTP layer and 16 hold a KV cache.
        assert_eq!(recurrent_layer_count(65, Some(4), 1), 48);
        // No MTP layer, no hybrid: every layer is a KV layer.
        assert_eq!(recurrent_layer_count(32, None, 0), 0);
    }

    #[test]
    fn recurrent_state_matches_the_builds_own_arithmetic() {
        // The owner's real Qwen3.8-27B header: state 128, inner 6144, 16
        // groups, conv kernel 4 → 786,432 + 30,720 elements per layer, ×4
        // bytes = 3,268,608 bytes. 48 layers → 149.62 MiB, which is exactly
        // what the build reports for one sequence (`llama_memory_recurrent:
        // R (f32): 5.62 MiB, S (f32): 144.00 MiB`).
        let mut metadata = make_metadata();
        metadata.ssm_state_size = Some(128);
        metadata.ssm_inner_size = Some(6144);
        metadata.ssm_group_count = Some(16);
        metadata.ssm_conv_kernel = Some(4);

        let per_sequence = recurrent_state_per_sequence(&metadata).expect("all four keys present");
        assert_eq!(per_sequence, 3_268_608);
        let total = per_sequence * 48;
        assert_eq!(total, 156_893_184);
        // 149.62 MiB, within a rounding of the build's own report.
        assert_eq!(format_bytes(total), "150 MiB");

        // A header missing any one of the four keys leaves the term unmodelled
        // rather than half-computed.
        metadata.ssm_conv_kernel = None;
        assert!(recurrent_state_per_sequence(&metadata).is_none());
    }
}
