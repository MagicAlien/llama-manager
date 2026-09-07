//! VRAM estimator — pure function per AGENTS.md invariant 5.
//!
//! Implements the estimation model stated in `docs/CONTRACTS.md` §1, "The
//! estimation model". No database access; history is an argument pre-filtered
//! by the caller to records matching this model's absolute path.
//!
//! See the doc comments on `estimate()` for the full model specification.

use super::types::{EstimateConfidence, EstimateInputs, LaunchRecord, VramEstimate};

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

/// Parameters for VRAM estimation calculation.
struct VramCalcParams<'a> {
    file_size_bytes: u64,
    gpu_layers: u32,
    block_count: u32,
    ctx_size: u32,
    attention_head_count_kv: u32,
    head_dim: u32,
    cache_type: Option<&'a str>,
    ubatch_size: u32,
    embedding_length: u32,
    margin: f64,
}

/// Bytes per element for different cache quantizations.
fn bytes_per_element(cache_type: Option<&str>) -> f64 {
    match cache_type {
        Some("f16") => 2.0,
        Some("q8_0") => 1.0,
        Some("q4_0") => 0.5625,
        _ => 2.0, // Default to f16
    }
}

/// Compute head dimension from embedding length and attention head count.
fn head_dim(embedding_length: Option<u32>, attention_head_count: Option<u32>) -> Option<u32> {
    match (embedding_length, attention_head_count) {
        (Some(emb), Some(heads)) if heads > 0 => Some(emb / heads),
        _ => None,
    }
}

/// Compute the KV cache size in bytes.
///
/// Formula: 2 × ctx_size × min(gpu_layers, block_count) × attention_head_count_kv × head_dim × bytes_per_element
fn kv_cache_bytes(
    ctx_size: u32,
    gpu_layers: u32,
    block_count: u32,
    attention_head_count_kv: u32,
    head_dim: u32,
    cache_type: Option<&str>,
) -> u64 {
    let min_layers = std::cmp::min(gpu_layers, block_count);
    if min_layers == 0 {
        return 0;
    }

    let result = 2.0
        * ctx_size as f64
        * min_layers as f64
        * attention_head_count_kv as f64
        * head_dim as f64
        * bytes_per_element(cache_type);

    result as u64
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
        params.gpu_layers,
        params.block_count,
        params.attention_head_count_kv,
        params.head_dim,
        params.cache_type,
    ) as f64;
    let compute_buffer = compute_buffer_size(params.ubatch_size, params.embedding_length) as f64;

    let total = weights_gpu + kv_bytes + compute_buffer + C_CONTEXT as f64;
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

/// Find the largest number of GPU layers that fits in available VRAM.
fn recommend_gpu_layers(inputs: &EstimateInputs, block_count: u32, margin: f64) -> u32 {
    if block_count == 0 {
        return 0;
    }

    let ctx_size = inputs.params.ctx_size.unwrap_or(2048);
    let ubatch_size = inputs.params.ubatch_size.unwrap_or(512);
    let embedding_length = inputs.metadata.embedding_length.unwrap_or(4096);

    let head = head_dim(
        inputs.metadata.embedding_length,
        inputs.metadata.attention_head_count,
    );
    let head_dim = head.unwrap_or(DEFAULT_HEAD_DIM);

    let attention_head_count_kv = inputs
        .metadata
        .attention_head_count_kv
        .unwrap_or(DEFAULT_ATTENTION_HEAD_COUNT_KV);

    // Binary search for the largest n that fits
    let mut low = 0;
    let mut high = block_count;
    while low < high {
        let mid = (low + high).div_ceil(2);
        let params = VramCalcParams {
            file_size_bytes: inputs.file_size_bytes,
            gpu_layers: mid,
            block_count,
            ctx_size,
            attention_head_count_kv,
            head_dim,
            cache_type: inputs.params.cache_type_k.as_deref(),
            ubatch_size,
            embedding_length,
            margin,
        };
        let vram = estimate_vram_for_layers(&params);
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
fn compute_bias(history: &[LaunchRecord], inputs: &EstimateInputs) -> f64 {
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
    let ctx_size = inputs.params.ctx_size.unwrap_or(2048);
    let ubatch_size = inputs.params.ubatch_size.unwrap_or(512);
    let embedding_length = inputs.metadata.embedding_length.unwrap_or(4096);

    let head = head_dim(
        inputs.metadata.embedding_length,
        inputs.metadata.attention_head_count,
    );
    let head_dim = head.unwrap_or(DEFAULT_HEAD_DIM);

    let attention_head_count_kv = inputs
        .metadata
        .attention_head_count_kv
        .unwrap_or(DEFAULT_ATTENTION_HEAD_COUNT_KV);

    for record in &recent {
        let Some(actual) = record.actual_vram_bytes else {
            continue;
        };
        let params = VramCalcParams {
            file_size_bytes: inputs.file_size_bytes,
            gpu_layers: record.params.gpu_layers.unwrap_or(0),
            block_count: inputs.metadata.block_count,
            ctx_size: record.params.ctx_size.unwrap_or(ctx_size),
            attention_head_count_kv,
            head_dim,
            cache_type: record.params.cache_type_k.as_deref(),
            ubatch_size: record.params.ubatch_size.unwrap_or(ubatch_size),
            embedding_length,
            margin: MARGIN,
        };
        let estimated = estimate_vram_for_layers(&params);
        if estimated > 0 {
            total_ratio += actual as f64 / estimated as f64;
        }
    }

    total_ratio / recent.len() as f64
}

/// Main estimation function.
///
/// Implements the model from `docs/CONTRACTS.md` §1.
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

    // Determine margin based on metadata availability
    let margin = if inputs.metadata.attention_head_count_kv.is_none()
        || inputs.metadata.embedding_length.is_none()
        || inputs.metadata.attention_head_count.is_none()
    {
        WIDENED_MARGIN
    } else {
        MARGIN
    };

    notes.push(format!("Conservative margin: {:.0}%", margin * 100.0));

    // Compute head dimension
    let head = head_dim(
        inputs.metadata.embedding_length,
        inputs.metadata.attention_head_count,
    );
    let head_dim = match head {
        Some(h) => h,
        None => {
            notes.push(format!(
                "Assumed head_dim: {DEFAULT_HEAD_DIM} (metadata missing)"
            ));
            DEFAULT_HEAD_DIM
        }
    };

    // Compute attention head count KV
    let attention_head_count_kv = match inputs.metadata.attention_head_count_kv {
        Some(kv) => kv,
        None => {
            notes.push(format!("Assumed attention_head_count_kv: {DEFAULT_ATTENTION_HEAD_COUNT_KV} (metadata missing)"));
            DEFAULT_ATTENTION_HEAD_COUNT_KV
        }
    };

    // Determine GPU layers
    let gpu_layers = if let Some(layers) = inputs.params.gpu_layers {
        layers
    } else {
        // Recommend based on available VRAM
        recommend_gpu_layers(inputs, inputs.metadata.block_count, margin)
    };

    // Check for calibration from history
    let matching = history_matches(history, inputs);
    let confidence = if !matching.is_empty() {
        EstimateConfidence::Calibrated
    } else {
        EstimateConfidence::Heuristic
    };

    if confidence == EstimateConfidence::Calibrated {
        // Use calibrated estimate from recent matching history
        let recent: Vec<&LaunchRecord> = matching.iter().rev().take(3).cloned().collect();
        let total: u64 = recent.iter().filter_map(|r| r.actual_vram_bytes).sum();
        let count = recent
            .iter()
            .filter(|r| r.actual_vram_bytes.is_some())
            .count() as u64;
        if count == 0 {
            return VramEstimate {
                recommended_gpu_layers: gpu_layers,
                estimated_vram_bytes: 0,
                estimated_ram_bytes: 0,
                kv_cache_bytes: 0,
                fits_fully: true,
                confidence,
                notes,
            };
        }
        let calibrated_vram = (total / count) as f64 * (1.0 + margin);
        let estimated_vram = calibrated_vram as u64;
        let estimated_ram = estimate_ram_for_layers(
            inputs.file_size_bytes,
            gpu_layers,
            inputs.metadata.block_count,
        );
        let kv_bytes = kv_cache_bytes(
            inputs.params.ctx_size.unwrap_or(2048),
            gpu_layers,
            inputs.metadata.block_count,
            attention_head_count_kv,
            head_dim,
            inputs.params.cache_type_k.as_deref(),
        );
        notes.push(format!(
            "Calibrated from {} recent matching launches",
            recent.len()
        ));
        return VramEstimate {
            recommended_gpu_layers: gpu_layers,
            estimated_vram_bytes: estimated_vram,
            estimated_ram_bytes: estimated_ram,
            kv_cache_bytes: kv_bytes,
            fits_fully: estimated_vram <= inputs.vram_free_bytes,
            confidence,
            notes,
        };
    }

    // Heuristic estimate
    let ctx_size = inputs.params.ctx_size.unwrap_or(2048);
    let ubatch_size = inputs.params.ubatch_size.unwrap_or(512);
    let embedding_length = inputs.metadata.embedding_length.unwrap_or(4096);

    let params = VramCalcParams {
        file_size_bytes: inputs.file_size_bytes,
        gpu_layers,
        block_count: inputs.metadata.block_count,
        ctx_size,
        attention_head_count_kv,
        head_dim,
        cache_type: inputs.params.cache_type_k.as_deref(),
        ubatch_size,
        embedding_length,
        margin,
    };
    let estimated_vram = estimate_vram_for_layers(&params);

    let estimated_ram = estimate_ram_for_layers(
        inputs.file_size_bytes,
        gpu_layers,
        inputs.metadata.block_count,
    );

    let kv_bytes = kv_cache_bytes(
        ctx_size,
        gpu_layers,
        inputs.metadata.block_count,
        attention_head_count_kv,
        head_dim,
        inputs.params.cache_type_k.as_deref(),
    );

    // Apply bias from non-matching history
    let bias = compute_bias(history, inputs);
    if bias != 1.0 {
        notes.push(format!(
            "Bias applied from non-matching history: {bias:.2}x"
        ));
        let biased_vram = (estimated_vram as f64 * bias) as u64;
        return VramEstimate {
            recommended_gpu_layers: gpu_layers,
            estimated_vram_bytes: biased_vram,
            estimated_ram_bytes: estimated_ram,
            kv_cache_bytes: kv_bytes,
            fits_fully: biased_vram <= inputs.vram_free_bytes,
            confidence,
            notes,
        };
    }

    VramEstimate {
        recommended_gpu_layers: gpu_layers,
        estimated_vram_bytes: estimated_vram,
        estimated_ram_bytes: estimated_ram,
        kv_cache_bytes: kv_bytes,
        fits_fully: estimated_vram <= inputs.vram_free_bytes,
        confidence,
        notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{GgufMetadata, LaunchParams};

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
            has_chat_template: false,
            is_moe: false,
            expert_count: None,
        }
    }

    fn make_inputs(metadata: GgufMetadata, file_size: u64) -> EstimateInputs {
        EstimateInputs {
            metadata,
            file_size_bytes: file_size,
            params: LaunchParams::default(),
            vram_free_bytes: 24 * 1024 * 1024 * 1024, // 24 GB
            ram_free_bytes: 16 * 1024 * 1024 * 1024,  // 16 GB
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
        // 2 × 2048 × 32 × 8 × 128 × 2 = 268,435,456 bytes
        let result = kv_cache_bytes(2048, 32, 32, 8, 128, Some("f16"));
        assert_eq!(result, 268_435_456);
    }

    #[test]
    fn test_compute_buffer_size() {
        // 512 × 4096 × 4 × 2 = 16,777,216 bytes
        let result = compute_buffer_size(512, 4096);
        assert_eq!(result, 16_777_216);
    }
}
