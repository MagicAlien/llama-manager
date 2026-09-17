//! T-032: VRAM estimator tests.
//!
//! Table-driven tests over 8 configurations including NVFP4 and GQA.
//! Term-by-term assertions. Property tests for monotonicity and bounds.
//! Insta snapshots for determinism.

use crate::core::estimator::estimate;
use crate::core::types::{
    DraftModelInputs, EstimateConfidence, EstimateInputs, GgufMetadata, LaunchParams, LaunchRecord,
};
use chrono::Utc;
use std::path::PathBuf;

fn make_metadata(architecture: &str, quantization: &str, block_count: u32) -> GgufMetadata {
    GgufMetadata {
        architecture: architecture.to_string(),
        param_count: Some(7_000_000_000),
        quantization: quantization.to_string(),
        block_count,
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

fn make_inputs(metadata: GgufMetadata, file_size: u64, vram_free: u64) -> EstimateInputs {
    EstimateInputs {
        metadata,
        file_size_bytes: file_size,
        params: LaunchParams::default(),
        vram_free_bytes: vram_free,
        vram_total_bytes: vram_free,
        projector_bytes: 0,
        draft: None,
        ram_free_bytes: 16 * 1024 * 1024 * 1024,
    }
}

// ─── Table-driven tests over 8 configurations ───────────────────────────────

#[test]
fn test_llama_q4_k_m_7b() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.estimated_ram_bytes > 0);
    assert!(estimate.kv_cache_bytes > 0);
    assert_eq!(estimate.confidence, EstimateConfidence::Heuristic);
    assert!(estimate.recommended_gpu_layers <= 32);
}

#[test]
fn test_llama_q8_0_7b() {
    let metadata = make_metadata("llama", "Q8_0", 32);
    let inputs = make_inputs(metadata, 8_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 32);
}

#[test]
fn test_llama_f16_7b() {
    let metadata = make_metadata("llama", "F16", 32);
    let inputs = make_inputs(metadata, 14_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 32);
}

#[test]
fn test_mistral_q4_k_m_7b() {
    let metadata = make_metadata("mistral", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 32);
}

#[test]
fn test_qwen3_q4_k_m_7b() {
    let metadata = make_metadata("qwen3", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 32);
}

#[test]
fn test_mixtral_moe_q4_k_m_8x7b() {
    let metadata = make_metadata("mixtral", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 24_000_000_000, 48 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 32);
}

#[test]
fn test_nvfp4_qwen3_30b() {
    let metadata = make_metadata("qwen3", "NVFP4", 60);
    let inputs = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 60);
}

#[test]
fn test_gqa_llama_q4_k_m_7b() {
    // GQA: attention_head_count=32, attention_head_count_kv=8 (4:1 ratio)
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(estimate.estimated_vram_bytes > 0);
    assert!(estimate.recommended_gpu_layers <= 32);
}

// ─── Term-by-term assertions ────────────────────────────────────────────────

#[test]
fn test_weights_gpu_term() {
    // weights_gpu = file_size_bytes × (gpu_layers / block_count)
    // For 4GB model, 16 layers, 32 block_count: 4GB × (16/32) = 2GB
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(16);
    let estimate = estimate(&inputs, &[]);
    // weights_gpu should be ~2GB, plus KV cache and compute buffer
    // Total will be higher due to KV cache, but weights term is correct
    assert!(estimate.estimated_vram_bytes > 2_000_000_000);
}

#[test]
fn test_kv_cache_term_grows_with_ctx_size() {
    // kv_bytes = 2 × ctx_size × min(gpu_layers, block_count) × attention_head_count_kv × head_dim × bytes_per_element
    // Larger ctx_size → larger KV cache
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(2048);
    let estimate_small = estimate(&inputs, &[]);

    inputs.params.ctx_size = Some(4096);
    let estimate_large = estimate(&inputs, &[]);

    assert!(
        estimate_large.kv_cache_bytes > estimate_small.kv_cache_bytes,
        "KV cache should grow with ctx_size"
    );
}

#[test]
fn test_compute_buffer_term_grows_with_ubatch_size() {
    // compute_buffer = ubatch_size × embedding_length × 4 bytes × C_compute
    // Larger ubatch_size → larger compute buffer
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ubatch_size = Some(256);
    let estimate_small = estimate(&inputs, &[]);

    inputs.params.ubatch_size = Some(512);
    let estimate_large = estimate(&inputs, &[]);

    assert!(
        estimate_large.estimated_vram_bytes > estimate_small.estimated_vram_bytes,
        "Compute buffer should grow with ubatch_size"
    );
}

// ─── Monotonicity in gpu_layers ─────────────────────────────────────────────

#[test]
fn test_monotonic_in_gpu_layers() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);

    let mut prev_vram = 0u64;
    for layers in 0..=32 {
        inputs.params.gpu_layers = Some(layers);
        let estimate = estimate(&inputs, &[]);
        assert!(
            estimate.estimated_vram_bytes >= prev_vram,
            "VRAM estimate should be monotonic in gpu_layers"
        );
        prev_vram = estimate.estimated_vram_bytes;
    }
}

// ─── Bounds: recommended_gpu_layers never exceeds block_count ───────────────

#[test]
fn test_recommended_layers_bounded_by_block_count() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 100 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    assert!(
        estimate.recommended_gpu_layers <= 32,
        "recommended_gpu_layers should not exceed block_count"
    );
}

// ─── Zero VRAM → 0 layers ───────────────────────────────────────────────────

#[test]
fn test_zero_vram_recommends_zero_layers() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 0);
    let estimate = estimate(&inputs, &[]);
    assert_eq!(
        estimate.recommended_gpu_layers, 0,
        "With zero VRAM, should recommend 0 layers"
    );
}

// ─── Missing metadata degradation ───────────────────────────────────────────

#[test]
fn test_missing_attention_head_count_kv() {
    let mut metadata = make_metadata("llama", "Q4_K_M", 32);
    metadata.attention_head_count_kv = None;
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    // Should use default value and note it
    let has_note = estimate
        .notes
        .iter()
        .any(|n| n.contains("attention_head_count_kv"));
    assert!(has_note, "Should note the assumed attention_head_count_kv");
}

#[test]
fn test_missing_embedding_length() {
    let mut metadata = make_metadata("llama", "Q4_K_M", 32);
    metadata.embedding_length = None;
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    // With no declared per-head lengths and no usable division, the dimension
    // is assumed and the note says so (T-038 wording).
    let has_note = estimate
        .notes
        .iter()
        .any(|n| n.contains("Assumed head dimension"));
    assert!(has_note, "Should note the assumed head dimension");
}

#[test]
fn test_missing_attention_head_count() {
    let mut metadata = make_metadata("llama", "Q4_K_M", 32);
    metadata.attention_head_count = None;
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    let has_note = estimate
        .notes
        .iter()
        .any(|n| n.contains("Assumed head dimension"));
    assert!(has_note, "Should note the assumed head dimension");
}

// ─── Cache type quantization effect ─────────────────────────────────────────

#[test]
fn test_cache_quantization_reduces_kv_cache() {
    // q4_0 cache should use less memory than f16
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(8192);

    inputs.params.cache_type_k = Some("f16".to_string());
    let estimate_f16 = estimate(&inputs, &[]);

    inputs.params.cache_type_k = Some("q4_0".to_string());
    let estimate_q4 = estimate(&inputs, &[]);

    assert!(
        estimate_q4.kv_cache_bytes < estimate_f16.kv_cache_bytes,
        "q4_0 cache should use less memory than f16"
    );
}

#[test]
fn test_kv_cache_uses_v_cache_type_separately() {
    // The KV cache formula must read cache_type_k AND cache_type_v
    // independently: changing only cache_type_v (K held constant) must
    // change kv_cache_bytes. Regression test for the bug where only the K
    // type was consulted, so the V dropdown had no effect on the estimate.
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(8192);
    inputs.params.cache_type_k = Some("f16".to_string());

    inputs.params.cache_type_v = Some("f16".to_string());
    let estimate_v_f16 = estimate(&inputs, &[]);

    inputs.params.cache_type_v = Some("q4_0".to_string());
    let estimate_v_q4 = estimate(&inputs, &[]);

    assert!(
        estimate_v_q4.kv_cache_bytes < estimate_v_f16.kv_cache_bytes,
        "kv_cache_bytes must shrink when only cache_type_v is quantized"
    );
}

#[test]
fn test_projector_bytes_included_in_estimate() {
    // The projector (mmproj) is loaded into VRAM alongside the model; the
    // estimate must grow when a projector is present.
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(8192);
    inputs.projector_bytes = 0;
    let without = estimate(&inputs, &[]);

    inputs.projector_bytes = 1_000_000_000; // 1 GiB projector
    let with = estimate(&inputs, &[]);

    assert!(
        with.estimated_vram_bytes > without.estimated_vram_bytes,
        "projector_bytes must be included in estimated_vram_bytes"
    );
}

// ─── Calibration from history ───────────────────────────────────────────────

#[test]
fn test_calibrated_estimate() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(8192);

    // Create matching history
    let history = vec![LaunchRecord {
        file_path: PathBuf::from("/path/to/model.gguf"),
        launched_at: Utc::now(),
        params: inputs.params.clone(),
        succeeded: true,
        actual_vram_bytes: Some(6_000_000_000),
        load_seconds: Some(10.0),
    }];

    let estimate = estimate(&inputs, &history);
    assert_eq!(
        estimate.confidence,
        EstimateConfidence::Calibrated,
        "Should be calibrated with matching history"
    );
}

#[test]
fn test_heuristic_estimate() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);

    let estimate = estimate(&inputs, &[]);
    assert_eq!(
        estimate.confidence,
        EstimateConfidence::Heuristic,
        "Should be heuristic with no history"
    );
}

#[test]
fn test_bias_from_non_matching_history() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(8192);

    // Create non-matching history (different gpu_layers)
    let mut non_matching_params = inputs.params.clone();
    non_matching_params.gpu_layers = Some(16);
    let history = vec![LaunchRecord {
        file_path: PathBuf::from("/path/to/model.gguf"),
        launched_at: Utc::now(),
        params: non_matching_params,
        succeeded: true,
        actual_vram_bytes: Some(4_000_000_000),
        load_seconds: Some(5.0),
    }];

    let estimate = estimate(&inputs, &history);
    assert_eq!(
        estimate.confidence,
        EstimateConfidence::Heuristic,
        "Should still be heuristic with non-matching history"
    );
}

// ─── Determinism ────────────────────────────────────────────────────────────

#[test]
fn test_deterministic() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);

    let e1 = estimate(&inputs, &[]);
    let e2 = estimate(&inputs, &[]);

    assert_eq!(e1.estimated_vram_bytes, e2.estimated_vram_bytes);
    assert_eq!(e1.estimated_ram_bytes, e2.estimated_ram_bytes);
    assert_eq!(e1.recommended_gpu_layers, e2.recommended_gpu_layers);
    assert_eq!(e1.kv_cache_bytes, e2.kv_cache_bytes);
}

// ─── Insta snapshots ────────────────────────────────────────────────────────

#[test]
fn snapshot_llama_q4_k_m_7b() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_llama_q8_0_7b() {
    let metadata = make_metadata("llama", "Q8_0", 32);
    let inputs = make_inputs(metadata, 8_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_llama_f16_7b() {
    let metadata = make_metadata("llama", "F16", 32);
    let inputs = make_inputs(metadata, 14_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_mistral_q4_k_m_7b() {
    let metadata = make_metadata("mistral", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_qwen3_q4_k_m_7b() {
    let metadata = make_metadata("qwen3", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_mixtral_moe_q4_k_m_8x7b() {
    let metadata = make_metadata("mixtral", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 24_000_000_000, 48 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_nvfp4_qwen3_30b() {
    let metadata = make_metadata("qwen3", "NVFP4", 60);
    let inputs = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

#[test]
fn snapshot_gqa_llama_q4_k_m_7b() {
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    insta::assert_json_snapshot!(estimate);
}

// ─── T-038: KV-cache accuracy and speculative-decoding memory accounting ────
//
// Every expected number below was measured on the pinned build (b10883) loading
// the owner's real Qwen3.8-27B, reading its own `llama_kv_cache` and
// `llama_memory_recurrent` reports — PROGRESS.md F-019. Where a test's
// expectation is a measurement rather than a derivation, the comment says so.

/// A hybrid model (Qwen3-Next style) with MTP layers: the shape of all three of
/// the owner's real files.
fn hybrid_metadata() -> GgufMetadata {
    let mut metadata = make_metadata("qwen35", "NVFP4", 65);
    metadata.param_count = Some(27_000_000_000);
    metadata.context_length = Some(262_144);
    metadata.embedding_length = Some(5120);
    metadata.attention_head_count = Some(24);
    metadata.attention_head_count_kv = Some(4);
    metadata.attention_key_length = Some(256);
    metadata.attention_value_length = Some(256);
    metadata.full_attention_interval = Some(4);
    metadata.ssm_state_size = Some(128);
    metadata.ssm_inner_size = Some(6144);
    metadata.ssm_group_count = Some(16);
    metadata.ssm_conv_kernel = Some(4);
    metadata.has_mtp_heads = true;
    metadata.mtp_layer_count = Some(1);
    metadata
}

/// The draft companion's own header: the owner's real DFlash2 file.
fn companion_metadata() -> GgufMetadata {
    let mut metadata = make_metadata("dflash", "NVFP4", 5);
    metadata.param_count = Some(1_900_000_000);
    metadata.context_length = Some(262_144);
    metadata.embedding_length = Some(5120);
    metadata.attention_head_count = Some(32);
    metadata.attention_head_count_kv = Some(8);
    metadata.attention_key_length = Some(128);
    metadata.attention_value_length = Some(128);
    metadata.is_draft_model = true;
    metadata
}

#[test]
fn test_t038_kv_term_uses_the_kv_head_count_not_the_query_head_count() {
    // Acceptance: "the KV-cache term of a GQA model uses
    // attention_head_count_kv, never attention_head_count — asserted
    // term-by-term against a hand-computed value for a model with an 8:1 ratio".
    // Here the ratio is 32 : 4 = 8 : 1.
    let mut metadata = make_metadata("llama", "Q4_K_M", 32);
    metadata.attention_head_count = Some(32);
    metadata.attention_head_count_kv = Some(4);
    metadata.attention_key_length = Some(128);
    metadata.attention_value_length = Some(128);

    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(8192);
    inputs.params.cache_type_k = Some("f16".to_string());
    inputs.params.cache_type_v = Some("f16".to_string());

    let estimate = estimate(&inputs, &[]);

    // Hand-computed: ctx 8192 × 32 layers × 4 KV heads × (128×2 + 128×2) bytes
    let expected = 8192u64 * 32 * 4 * (128 * 2 + 128 * 2);
    assert_eq!(expected, 536_870_912);
    assert_eq!(
        estimate.kv_cache_bytes, expected,
        "the KV term must be priced with the 4 KV heads"
    );
    // …and not with the 32 query heads, which is 8× larger.
    assert_ne!(estimate.kv_cache_bytes, 8192u64 * 32 * 32 * 512);
}

#[test]
fn test_t038_head_dimension_comes_from_the_header_not_the_division() {
    // Owners' real files declare 256-wide heads while
    // embedding_length / attention_head_count = 5120 / 24 = 213. The cache is
    // priced per head, so the declared dimension decides the term.
    let metadata = hybrid_metadata();
    let mut inputs = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    inputs.params.ctx_size = Some(512);
    inputs.params.gpu_layers = Some(65);

    let estimate = estimate(&inputs, &[]);

    // 512 cells × 16 KV layers × 4 KV heads × (256×2 + 256×2) = 32.00 MiB —
    // exactly what the build reports for this model at this context
    // (`llama_kv_cache: size = 32.00 MiB (512 cells, 16 layers)`).
    assert_eq!(estimate.kv_cache_bytes, 33_554_432);
    // The division would have produced this instead.
    let division_dim = 5120 / 24; // 213
    assert_ne!(
        estimate.kv_cache_bytes,
        512u64 * 16 * 4 * (division_dim * 2 + division_dim * 2)
    );
}

#[test]
fn test_t038_hybrid_model_sizes_the_cache_for_its_attention_layers_only() {
    // 65 blocks, one full-attention layer every 4th → 16 KV layers, and the
    // other 48 keep a fixed-size recurrent state. Sizing the cache for all 65
    // layers is the 4× overestimate T-038 was opened for.
    let metadata = hybrid_metadata();
    let mut all_layers = make_inputs(metadata.clone(), 16_000_000_000, 48 * 1024 * 1024 * 1024);
    all_layers.params.ctx_size = Some(512);
    all_layers.params.gpu_layers = Some(65);

    let mut every_layer = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    every_layer.params.ctx_size = Some(512);
    every_layer.params.gpu_layers = Some(65);
    every_layer.metadata.full_attention_interval = None; // as if it were dense

    let hybrid = estimate(&all_layers, &[]);
    let dense = estimate(&every_layer, &[]);

    assert_eq!(hybrid.kv_cache_bytes, 33_554_432);
    assert_eq!(dense.kv_cache_bytes, 33_554_432 / 16 * 65);
    assert!(
        hybrid.kv_cache_bytes < dense.kv_cache_bytes,
        "the hybrid cache must be the smaller of the two"
    );
}

#[test]
fn test_t038_recurrent_state_matches_the_builds_own_buffer_size() {
    // The build prints, for this model with one sequence:
    //   `llama_memory_recurrent: size = 149.62 MiB (1 cells, 64 layers, 1 seqs 0 rs_seq),
    //    R (f32): 5.62 MiB, S (f32): 144.00 MiB`
    // and 4× that when the draft stage runs (`598.50 MiB`, `3 rs_seq`).
    let metadata = hybrid_metadata();
    let mut inputs = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    inputs.params.ctx_size = Some(512);
    inputs.params.gpu_layers = Some(65);

    let estimate = estimate(&inputs, &[]);

    // 48 recurrent layers × (786 432 + 30 720) elements × 4 bytes = 149.62 MiB
    // for one sequence; this model has MTP heads, so the draft stage keeps
    // 1 + --spec-draft-n-max (default 3) = 4 of them.
    let per_sequence = 48u64 * (786_432 + 30_720) * 4;
    assert_eq!(per_sequence, 156_893_184);
    assert_eq!(estimate.recurrent_state_bytes, per_sequence * 4);
    assert!(estimate
        .notes
        .iter()
        .any(|n| n.contains("Recurrent (non-attention) layers: 48 of 65")));
}

#[test]
fn test_t038_mtp_draft_layers_are_priced_and_named() {
    // Acceptance: "a model with MTP heads names the MTP cost in notes".
    // Measured: the MTP layer runs in its own context with its own cache —
    // `llama_kv_cache: size = 2.00 MiB (512 cells, 1 layers)` at this context.
    let metadata = hybrid_metadata();
    let mut inputs = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    inputs.params.ctx_size = Some(512);
    inputs.params.gpu_layers = Some(65);

    let estimate = estimate(&inputs, &[]);

    assert_eq!(estimate.mtp_draft_bytes, 2_097_152);
    assert!(
        estimate
            .notes
            .iter()
            .any(|n| n.contains("MTP draft layers: 1")),
        "the MTP cost must be named: {:?}",
        estimate.notes
    );
}

#[test]
fn test_t038_a_model_without_mtp_pays_nothing_for_it() {
    let mut metadata = hybrid_metadata();
    metadata.has_mtp_heads = false;
    metadata.mtp_layer_count = None;
    let mut inputs = make_inputs(metadata, 16_000_000_000, 48 * 1024 * 1024 * 1024);
    inputs.params.ctx_size = Some(512);
    inputs.params.gpu_layers = Some(65);

    let estimate = estimate(&inputs, &[]);

    assert_eq!(estimate.mtp_draft_bytes, 0);
    // …and with nothing drafting, the recurrent state is one sequence again.
    // The layer arithmetic follows llama.cpp's own: it runs the model on
    // `block_count − nextn_predict_layers` repeating layers, so without an MTP
    // layer all 65 blocks repeat and 65 − 16 attention layers keep a state.
    assert_eq!(estimate.recurrent_state_bytes, 49 * (786_432 + 30_720) * 4);
}

#[test]
fn test_t038_a_draft_companion_includes_its_weights_and_its_own_cache() {
    // Acceptance: "a configuration with a draft companion includes the
    // companion's weights in the estimate". The companion is a second model, so
    // its own cache is part of it too.
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.params.ctx_size = Some(512);
    inputs.draft = Some(DraftModelInputs {
        path: PathBuf::from("E:/models/draft-dflash2.gguf"),
        size_bytes: Some(1_094_346_016), // the real file, measured with stat
        metadata: Some(companion_metadata()),
    });

    let with_companion = estimate(&inputs, &[]);

    // 512 cells × 5 layers × 8 KV heads × (128×2 + 128×2) = 10.00 MiB — the
    // build's own figure for this companion (`llama_kv_cache: size = 10.00 MiB
    // (512 cells, 5 layers)`), plus the file itself.
    let companion_kv = 512u64 * 5 * 8 * (128 * 2 + 128 * 2);
    assert_eq!(companion_kv, 10_485_760);
    assert_eq!(with_companion.draft_bytes, 1_094_346_016 + companion_kv);
    assert!(with_companion
        .notes
        .iter()
        .any(|n| n.contains("Draft companion counted")));

    // The same configuration without the companion is strictly cheaper.
    let mut without = inputs.clone();
    without.draft = None;
    let without_companion = estimate(&without, &[]);
    assert!(
        with_companion.estimated_vram_bytes > without_companion.estimated_vram_bytes,
        "the companion's memory must appear in the projection"
    );
    assert_eq!(without_companion.draft_bytes, 0);
}

#[test]
fn test_t038_an_unmeasurable_companion_is_not_silently_zero() {
    // Acceptance: "An unquantified draft term must not be silently zero."
    // The configured companion is gone, so the term cannot be quantified — and
    // the estimate says so instead of presenting a tidy number without it.
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let mut inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    inputs.params.gpu_layers = Some(32);
    inputs.draft = Some(DraftModelInputs {
        path: PathBuf::from("E:/models/deleted-draft.gguf"),
        size_bytes: None,
        metadata: None,
    });

    let estimate = estimate(&inputs, &[]);

    assert_eq!(estimate.draft_bytes, 0);
    assert!(
        estimate
            .notes
            .iter()
            .any(|n| n.contains("deleted-draft.gguf") && n.contains("NOT included")),
        "the missing term must be named: {:?}",
        estimate.notes
    );
    // A knowingly incomplete estimate is reported with the widened margin.
    assert!(
        estimate
            .notes
            .iter()
            .any(|n| n.contains("Conservative margin: 25%")),
        "an unaccounted term must widen the margin: {:?}",
        estimate.notes
    );
}

#[test]
fn test_t038_recommended_layers_never_ignore_the_draft_term() {
    // Acceptance: "recommended_gpu_layers never recommends a layer count that
    // stops fitting once the draft term is included."
    let metadata = make_metadata("llama", "Q4_K_M", 32);
    let six_gib = 6 * 1024 * 1024 * 1024;

    let mut without = make_inputs(metadata.clone(), 4_000_000_000, six_gib);
    without.params.ctx_size = Some(2048);
    let plain = estimate(&without, &[]);
    assert_eq!(
        plain.recommended_gpu_layers, 32,
        "a 4 GB model fits in 6 GiB"
    );

    let mut with = make_inputs(metadata, 4_000_000_000, six_gib);
    with.params.ctx_size = Some(2048);
    with.draft = Some(DraftModelInputs {
        path: PathBuf::from("E:/models/draft.gguf"),
        size_bytes: Some(4 * 1024 * 1024 * 1024),
        metadata: Some(companion_metadata()),
    });
    let drafted = estimate(&with, &[]);

    assert!(
        drafted.recommended_gpu_layers < 32,
        "4 GiB of companion cannot be ignored: {}",
        drafted.recommended_gpu_layers
    );
    assert!(
        drafted.estimated_vram_bytes <= without.vram_free_bytes,
        "the recommendation must not exceed the available VRAM once the draft term is in"
    );
}
