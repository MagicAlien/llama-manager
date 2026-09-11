//! T-032: VRAM estimator tests.
//!
//! Table-driven tests over 8 configurations including NVFP4 and GQA.
//! Term-by-term assertions. Property tests for monotonicity and bounds.
//! Insta snapshots for determinism.

use crate::core::estimator::estimate;
use crate::core::types::{
    EstimateConfidence, EstimateInputs, GgufMetadata, LaunchParams, LaunchRecord,
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
        has_chat_template: false,
        is_moe: false,
        expert_count: None,
        is_draft_model: false,
        has_mtp_heads: false,
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
    // Should use default head_dim and note it
    let has_note = estimate.notes.iter().any(|n| n.contains("head_dim"));
    assert!(has_note, "Should note the assumed head_dim");
}

#[test]
fn test_missing_attention_head_count() {
    let mut metadata = make_metadata("llama", "Q4_K_M", 32);
    metadata.attention_head_count = None;
    let inputs = make_inputs(metadata, 4_000_000_000, 24 * 1024 * 1024 * 1024);
    let estimate = estimate(&inputs, &[]);
    let has_note = estimate.notes.iter().any(|n| n.contains("head_dim"));
    assert!(has_note, "Should note the assumed head_dim");
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
