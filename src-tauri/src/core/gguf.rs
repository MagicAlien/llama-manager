//! GGUF header reader — parse magic, version, tensor count, and KV metadata
//! from a GGUF file without reading tensor data. Handles multi-part shards
//! and projector file detection.
//!
//! See PLAN.md §2.1, AGENTS.md invariant 8, and docs/TASKS.md T-030.

use std::fs::File;
use std::io::Read;
use std::io::{Seek, SeekFrom};
use std::path::{Path, PathBuf};

use super::types::{AppError, GgufMetadata};

/// GGUF magic number: "GGUF" in little-endian u32.
const GGUF_MAGIC: u32 = 0x46554747;

/// Current GGUF format version. Older versions (1, 2) are also accepted.
const GGUF_VERSION: u32 = 3;

/// Maximum reasonable tensor count to prevent integer overflow attacks.
const MAX_TENSOR_COUNT: u64 = 1_000_000;

/// Maximum reasonable KV pair count.
const MAX_KV_COUNT: u64 = 100_000;

/// Map llama.cpp's `general.file_type` (FTYPE enum) to the human quant
/// label. Values beyond the enum (writer-specific) become None → "unknown".
fn ftype_label(v: u32) -> String {
    let label = match v {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19 => "IQ2_XXS",
        20 => "IQ2_XS",
        21 => "Q2_K_S",
        22 => "IQ3_XXS",
        23 => "IQ1_S",
        24 => "IQ4_NL",
        25 => "IQ3_S",
        26 => "IQ2_S",
        27 => "IQ4_XS",
        28 => "MXFP4",
        32 => "NVFP4",
        _ => "unknown",
    };
    label.to_string()
}

/// The GGUF convention prefixes every per-architecture key with the
/// architecture string itself: `llama.block_count`, `qwen35.block_count`,
/// `qwen35.attention.head_count_kv`, … — llama.cpp reads `<arch>.<suffix>`
/// generically, never with a fixed `llama.` prefix.
///
/// The reader used to read a fixed `llama.` name for the shape keys
/// (`llama.block_count`, `llama.embedding_length`,
/// `llama.attention.head_count`, `llama.attention.head_count_kv`) with a
/// bare-name fallback. On the owner's real files
/// (`general.architecture = qwen35`) that returned `block_count = 0` for the
/// first key and left the other three at `None`, so the estimator's KV term
/// fell back to a per-architecture default instead of the model's own GQA
/// ratio. Verified against the files themselves: they carry
/// `qwen35.embedding_length = 5120`, `qwen35.attention.head_count = 24`,
/// `qwen35.attention.head_count_kv = 4` (PROGRESS.md F-019).
///
/// The bare-suffix fallback is kept for a writer that omits the prefix — the
/// arch-prefixed key is always tried first.
fn arch_key(arch: &str, suffix: &str) -> String {
    format!("{arch}.{suffix}")
}

/// Coerce any integer-typed GGUF value to `u32`.
fn as_u32(value: &GgufValue) -> Option<u32> {
    match value {
        GgufValue::UInt32(n) => Some(*n),
        GgufValue::Int32(n) => Some(*n as u32),
        GgufValue::UInt64(n) => Some(*n as u32),
        GgufValue::Int64(n) => Some(*n as u32),
        _ => None,
    }
}

/// Read an integer stored under `<arch>.<suffix>`, falling back to the bare
/// `<suffix>` key. `None` when neither key is present, or when the value it
/// carries is not an integer type.
fn arch_u32(
    kv: &std::collections::HashMap<String, GgufValue>,
    arch: &str,
    suffix: &str,
) -> Option<u32> {
    kv.get(arch_key(arch, suffix).as_str())
        .or_else(|| kv.get(suffix))
        .and_then(as_u32)
}

/// Read the GGUF header (magic, version, tensor count, KV count).
fn read_header<R: Read + Seek>(reader: &mut R) -> Result<GgufHeader, AppError> {
    let mut magic_bytes = [0u8; 4];
    reader
        .read_exact(&mut magic_bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read magic number: {e}"),
        })?;
    let magic = u32::from_le_bytes(magic_bytes);
    if magic != GGUF_MAGIC {
        return Err(AppError::GgufParse {
            message: format!("bad magic number: expected 0x{GGUF_MAGIC:08X}, got 0x{magic:08X}"),
        });
    }

    let mut version_bytes = [0u8; 4];
    reader
        .read_exact(&mut version_bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read version: {e}"),
        })?;
    let version = u32::from_le_bytes(version_bytes);
    if !(1..=GGUF_VERSION).contains(&version) {
        return Err(AppError::GgufParse {
            message: format!("unsupported GGUF version: {version}"),
        });
    }

    let tensor_count = read_u64(reader)?;
    if tensor_count > MAX_TENSOR_COUNT {
        return Err(AppError::GgufParse {
            message: format!("tensor count too large: {tensor_count}"),
        });
    }

    let kv_count = read_u64(reader)?;
    if kv_count > MAX_KV_COUNT {
        return Err(AppError::GgufParse {
            message: format!("KV count too large: {kv_count}"),
        });
    }

    Ok(GgufHeader {
        version,
        tensor_count,
        kv_count,
    })
}

/// Read a u64 from the reader.
fn read_u64<R: Read>(reader: &mut R) -> Result<u64, AppError> {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read u64: {e}"),
        })?;
    Ok(u64::from_le_bytes(bytes))
}

/// Read a u32 from the reader.
fn read_u32<R: Read>(reader: &mut R) -> Result<u32, AppError> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read u32: {e}"),
        })?;
    Ok(u32::from_le_bytes(bytes))
}

/// Read a GGUF string (u64 length + UTF-8 bytes).
fn read_gguf_string<R: Read>(reader: &mut R) -> Result<String, AppError> {
    let len = read_u64(reader)?;
    if len > 1_000_000 {
        return Err(AppError::GgufParse {
            message: format!("string too long: {len} bytes"),
        });
    }
    let mut bytes = vec![0u8; len as usize];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read string: {e}"),
        })?;
    String::from_utf8(bytes).map_err(|e| AppError::GgufParse {
        message: format!("string is not valid UTF-8: {e}"),
    })
}

/// GGUF metadata value type.
///
/// The discriminants are the GGUF v3 spec's own wire values (ggml-org/gguf,
/// `enum GGUFMetadataValueType`): u8=0, i8=1, u16=2, i16=3, u32=4, i32=5,
/// f32=6, bool=7, string=8, array=9, u64=10, i64=11, f64=12. The original
/// T-030 enum carried a shifted order (String=11, Array=12, Float32=8,
/// ...) which parsed the synthetic 63-byte fixtures written the same wrong
/// way, and failed on EVERY real llama.cpp-written file with
/// `string too long` — type 8 (string) was consumed as a 4-byte float,
/// desynchronising the reader. Fixed after importing a real
/// Qwen3-27B-NVFP4 GGUF failed (8 Sept 2026, log evidence).
#[derive(Debug, Clone, Copy)]
enum GgufValueType {
    UInt8 = 0,
    Int8 = 1,
    UInt16 = 2,
    Int16 = 3,
    UInt32 = 4,
    Int32 = 5,
    Float32 = 6,
    Bool = 7,
    String = 8,
    Array = 9,
    UInt64 = 10,
    Int64 = 11,
    Float64 = 12,
}

impl GgufValueType {
    fn from_u32(v: u32) -> Result<Self, AppError> {
        match v {
            0 => Ok(GgufValueType::UInt8),
            1 => Ok(GgufValueType::Int8),
            2 => Ok(GgufValueType::UInt16),
            3 => Ok(GgufValueType::Int16),
            4 => Ok(GgufValueType::UInt32),
            5 => Ok(GgufValueType::Int32),
            6 => Ok(GgufValueType::Float32),
            7 => Ok(GgufValueType::Bool),
            8 => Ok(GgufValueType::String),
            9 => Ok(GgufValueType::Array),
            10 => Ok(GgufValueType::UInt64),
            11 => Ok(GgufValueType::Int64),
            12 => Ok(GgufValueType::Float64),
            _ => Err(AppError::GgufParse {
                message: format!("unknown GGUF value type: {v}"),
            }),
        }
    }
}

/// GGUF metadata value.
#[derive(Debug, Clone)]
pub enum GgufValue {
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    Bool(bool),
    String(String),
    Array,
}

/// Read a u64 value from the reader.
fn read_u64_value<R: Read>(reader: &mut R) -> Result<u64, AppError> {
    read_u64(reader)
}

/// Read a u32 value from the reader.
fn read_u32_value<R: Read>(reader: &mut R) -> Result<u32, AppError> {
    read_u32(reader)
}

/// Read a bool value from the reader.
fn read_bool_value<R: Read>(reader: &mut R) -> Result<bool, AppError> {
    let mut bytes = [0u8; 1];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read bool: {e}"),
        })?;
    Ok(bytes[0] != 0)
}

/// Skip a GGUF value without reading its contents.
fn skip_value<R: Read + Seek>(reader: &mut R, vtype: GgufValueType) -> Result<(), AppError> {
    match vtype {
        GgufValueType::Int8 | GgufValueType::UInt8 | GgufValueType::Bool => {
            reader
                .seek(SeekFrom::Current(1))
                .map_err(|e| AppError::GgufParse {
                    message: format!("failed to skip value: {e}"),
                })?;
        }
        GgufValueType::Int16 | GgufValueType::UInt16 => {
            reader
                .seek(SeekFrom::Current(2))
                .map_err(|e| AppError::GgufParse {
                    message: format!("failed to skip value: {e}"),
                })?;
        }
        GgufValueType::Int32 | GgufValueType::UInt32 | GgufValueType::Float32 => {
            reader
                .seek(SeekFrom::Current(4))
                .map_err(|e| AppError::GgufParse {
                    message: format!("failed to skip value: {e}"),
                })?;
        }
        GgufValueType::Int64 | GgufValueType::UInt64 | GgufValueType::Float64 => {
            reader
                .seek(SeekFrom::Current(8))
                .map_err(|e| AppError::GgufParse {
                    message: format!("failed to skip value: {e}"),
                })?;
        }
        GgufValueType::String => {
            let len = read_u64(reader)?;
            reader
                .seek(SeekFrom::Current(len as i64))
                .map_err(|e| AppError::GgufParse {
                    message: format!("failed to skip string: {e}"),
                })?;
        }
        // A nested array carries its own element type + count on the wire
        // (type u32, count u64, then the elements) — it is not a 4-byte
        // scalar. The old enum mapped Array to the Int32 arm's 4-byte skip,
        // which desynchronised any reader that met one.
        GgufValueType::Array => {
            let mut t_buf = [0u8; 4];
            reader
                .read_exact(&mut t_buf)
                .map_err(|e| AppError::GgufParse {
                    message: format!("failed to read array type: {e}"),
                })?;
            let arr_type = GgufValueType::from_u32(u32::from_le_bytes(t_buf))?;
            let count = read_u64(reader)?;
            for _ in 0..count {
                skip_value(reader, arr_type)?;
            }
        }
    }
    Ok(())
}

/// GGUF header fields.
#[derive(Debug)]
struct GgufHeader {
    version: u32,
    tensor_count: u64,
    kv_count: u64,
}

/// Parse GGUF metadata from a file.
///
/// Reads only the header and KV metadata section; never reads tensor data.
pub fn parse_metadata<R: Read + Seek>(mut reader: R) -> Result<GgufMetadata, AppError> {
    let header = read_header(&mut reader)?;

    // Read all KV pairs
    let mut kv: std::collections::HashMap<String, GgufValue> = std::collections::HashMap::new();
    for _ in 0..header.kv_count {
        let key = read_gguf_string(&mut reader)?;
        let mut type_buf = [0u8; 4];
        reader
            .read_exact(&mut type_buf)
            .map_err(|e| AppError::GgufParse {
                message: format!("failed to read KV type: {e}"),
            })?;
        let vtype = GgufValueType::from_u32(u32::from_le_bytes(type_buf))?;

        match vtype {
            GgufValueType::Int8 => {
                let mut b = [0u8; 1];
                reader.read_exact(&mut b).map_err(|e| AppError::GgufParse {
                    message: format!("failed to read i8: {e}"),
                })?;
                kv.insert(key, GgufValue::Int8(b[0] as i8));
            }
            GgufValueType::Int16 => {
                let mut b = [0u8; 2];
                reader.read_exact(&mut b).map_err(|e| AppError::GgufParse {
                    message: format!("failed to read i16: {e}"),
                })?;
                kv.insert(key, GgufValue::Int16(i16::from_le_bytes(b)));
            }
            GgufValueType::Int32 => {
                let v = read_i32(&mut reader)?;
                kv.insert(key, GgufValue::Int32(v));
            }
            GgufValueType::Int64 => {
                let v = read_i64(&mut reader)?;
                kv.insert(key, GgufValue::Int64(v));
            }
            GgufValueType::UInt8 => {
                let mut b = [0u8; 1];
                reader.read_exact(&mut b).map_err(|e| AppError::GgufParse {
                    message: format!("failed to read u8: {e}"),
                })?;
                kv.insert(key, GgufValue::UInt8(b[0]));
            }
            GgufValueType::UInt16 => {
                let v = read_u16(&mut reader)?;
                kv.insert(key, GgufValue::UInt16(v));
            }
            GgufValueType::UInt32 => {
                let v = read_u32_value(&mut reader)?;
                kv.insert(key, GgufValue::UInt32(v));
            }
            GgufValueType::UInt64 => {
                let v = read_u64_value(&mut reader)?;
                kv.insert(key, GgufValue::UInt64(v));
            }
            GgufValueType::Float32 => {
                let v = read_f32(&mut reader)?;
                kv.insert(key, GgufValue::Float32(v));
            }
            GgufValueType::Float64 => {
                let v = read_f64(&mut reader)?;
                kv.insert(key, GgufValue::Float64(v));
            }
            GgufValueType::Bool => {
                let v = read_bool_value(&mut reader)?;
                kv.insert(key, GgufValue::Bool(v));
            }
            GgufValueType::String => {
                let v = read_gguf_string(&mut reader)?;
                kv.insert(key, GgufValue::String(v));
            }
            GgufValueType::Array => {
                let mut arr_type_buf = [0u8; 4];
                reader
                    .read_exact(&mut arr_type_buf)
                    .map_err(|e| AppError::GgufParse {
                        message: format!("failed to read array type: {e}"),
                    })?;
                let arr_type = GgufValueType::from_u32(u32::from_le_bytes(arr_type_buf))?;
                let count = read_u64(&mut reader)?;
                for _ in 0..count {
                    skip_value(&mut reader, arr_type)?;
                }
                kv.insert(key, GgufValue::Array);
            }
        }
    }

    // Skip tensor descriptors (we don't need tensor data)
    for _ in 0..header.tensor_count {
        let _name = read_gguf_string(&mut reader)?;
        let _dims = read_u32(&mut reader)?;
        for _ in 0.._dims {
            let _ = read_u64(&mut reader)?;
        }
        let _type = read_u32(&mut reader)?;
        let _offset = read_u64(&mut reader)?;
    }

    // Extract relevant metadata
    let architecture = kv
        .get("general.architecture")
        .and_then(|v| match v {
            GgufValue::String(s) => Some(s.as_str()),
            _ => None,
        })
        .unwrap_or("unknown")
        .to_string();

    // `general.parameters` is absent from real-world GGUF headers — the
    // unsloth Qwen3.8-27B-NVFP4 files carry the count in
    // `general.size_label` as a string like "27B" instead. Fall back to
    // the size label so the catalogue shows "27B" rather than "?".
    let param_count = kv
        .get("general.parameters")
        .and_then(|v| match v {
            GgufValue::String(s) => parse_param_count(s),
            GgufValue::UInt64(n) => Some(*n),
            GgufValue::Int64(n) => Some(*n as u64),
            _ => None,
        })
        .or_else(|| {
            kv.get("general.size_label").and_then(|v| match v {
                GgufValue::String(s) => parse_param_count(s),
                GgufValue::UInt64(n) => Some(*n),
                GgufValue::Int64(n) => Some(*n as u64),
                _ => None,
            })
        });

    // The human quantization label. `general.quantization_version` is the
    // GGUF *spec* version of the quant encoding (always 2 today), not a
    // label — the old code printed it, so every real file showed a bogus
    // "Q2". The label lives in `general.file_type` (llama.cpp's FTYPE enum):
    // 0 F32, 1 F16, 2 Q4_0, 3 Q4_1, 7 Q8_0, 8 Q5_0, 9 Q5_1, 10 Q2_K,
    // 11 Q3_K_S, 12 Q3_K_M, 13 Q3_K_L, 14 Q4_K_S, 15 Q4_K_M, 16 Q5_K_S,
    // 17 Q5_K_M, 18 Q6_K, 19 IQ2_XXS, 20 IQ2_XS, 21 Q2_K_S, 22 IQ3_XXS,
    // 23 IQ1_S, 24 IQ4_NL, 25 IQ3_S, 26 IQ2_S, 27 IQ4_XS, 28 MXFP4(+MoE
    // variants 29/30), 32 NVFP4 (unsloth/Blackwell builds). Unmapped values
    // degrade to "unknown" rather than inventing a label.
    let quantization = kv
        .get("general.file_type")
        .and_then(|v| match v {
            GgufValue::UInt32(n) => Some(*n),
            GgufValue::Int32(n) => Some(*n as u32),
            GgufValue::UInt64(n) => Some(*n as u32),
            GgufValue::Int64(n) => Some(*n as u32),
            _ => None,
        })
        .map(ftype_label)
        .unwrap_or_else(|| "unknown".to_string());

    let block_count = arch_u32(&kv, &architecture, "block_count").unwrap_or(0);

    let context_length = arch_u32(&kv, &architecture, "context_length");

    let embedding_length = arch_u32(&kv, &architecture, "embedding_length");

    let attention_head_count = arch_u32(&kv, &architecture, "attention.head_count");

    let attention_head_count_kv = arch_u32(&kv, &architecture, "attention.head_count_kv");

    // T-038 — the per-head dimensions. `attention.key_length` /
    // `attention.value_length` are what llama.cpp itself uses for
    // `n_embd_head_k` / `n_embd_head_v`; on the owner's real Qwen3.8 files they
    // are 256/256, while `embedding_length / attention_head_count` is
    // 5120 / 24 = 213 (integer division of a value that is not a multiple).
    // The KV cache is priced per head with these, which is why the estimator
    // prefers them and only falls back to the division.
    let attention_key_length = arch_u32(&kv, &architecture, "attention.key_length");
    let attention_value_length = arch_u32(&kv, &architecture, "attention.value_length");

    // T-038 — a hybrid model declares how often a full-attention layer occurs.
    // Every layer between them is a recurrent (SSM/linear-attention) layer
    // whose state does not grow with the context. Measured on the real build:
    // `full_attention_interval = 4` with `block_count = 65` gives exactly 16 KV
    // layers, and the 48 others are recurrent (PROGRESS.md F-019).
    let full_attention_interval = arch_u32(&kv, &architecture, "full_attention_interval");

    // The recurrent-state geometry, read only when the file declares it. These
    // are the keys llama.cpp itself reads for a Mamba-style layer, and the
    // estimator sizes that layer's state from them (F-019).
    let ssm_state_size = arch_u32(&kv, &architecture, "ssm.state_size");
    let ssm_inner_size = arch_u32(&kv, &architecture, "ssm.inner_size");
    let ssm_group_count = arch_u32(&kv, &architecture, "ssm.group_count");
    let ssm_conv_kernel = arch_u32(&kv, &architecture, "ssm.conv_kernel");

    let has_chat_template = chat_template(&kv).is_some();

    // T-037 — the two capability questions the template answers about itself.
    // Both stay `false` when the file carries no template (or a template whose
    // protocol this project does not recognise): "cannot be determined" is
    // rendered as *absent*, never as a guessed tag.
    let template_text = chat_template(&kv);
    let supports_tools = template_text
        .map(template_declares_tool_use)
        .unwrap_or(false);
    let supports_thinking = template_text
        .map(template_declares_thinking)
        .unwrap_or(false);

    // Name the rule that claimed each tag: "why does this model show Thinking?"
    // is answered from the log, with the same provenance the table carries.
    if let Some(text) = template_text {
        if let Some(protocol) = matched_protocol(text, TOOL_PROTOCOLS) {
            tracing::debug!(
                "model declares Tool use: matched protocol from {}",
                protocol.provenance
            );
        }
        if let Some(protocol) = matched_protocol(text, THINKING_PROTOCOLS) {
            tracing::debug!(
                "model declares Thinking: matched protocol from {}",
                protocol.provenance
            );
        }
    }

    // T-038 — the same arch-prefix rule applies to the MoE keys: a real
    // `mixtral`/`qwen3moe` file declares `{arch}.expert_count`, so reading a
    // fixed `llama.expert_count` reported every MoE model as dense.
    let expert_count = arch_u32(&kv, &architecture, "expert_count");

    let is_moe = architecture == "mixtral"
        || architecture == "deepseek2"
        || expert_count.is_some_and(|n| n > 1);

    let is_draft_model = is_draft_architecture(&architecture);
    // T-038 — the count, not just the presence: the estimator prices the extra
    // draft layer's KV cache from it. `has_mtp_heads` keeps T-036's meaning
    // ("the file declares Multi-Token-Prediction layers").
    let mtp_layer_count = arch_u32(&kv, &architecture, "nextn_predict_layers");
    let has_mtp_heads = mtp_layer_count.is_some_and(|n| n > 0);

    Ok(GgufMetadata {
        architecture,
        param_count,
        quantization,
        block_count,
        context_length,
        embedding_length,
        attention_head_count,
        attention_head_count_kv,
        attention_key_length,
        attention_value_length,
        full_attention_interval,
        ssm_state_size,
        ssm_inner_size,
        ssm_group_count,
        ssm_conv_kernel,
        has_chat_template,
        is_moe,
        expert_count,
        is_draft_model,
        has_mtp_heads,
        mtp_layer_count,
        supports_tools,
        supports_thinking,
    })
}

/// Parse a parameter count string like "8B" or "70B" into u64.
fn parse_param_count(s: &str) -> Option<u64> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("B") {
        n.parse::<u64>().ok().map(|n| n * 1_000_000_000)
    } else if let Some(n) = s.strip_suffix("M") {
        n.parse::<u64>().ok().map(|n| n * 1_000_000)
    } else if let Some(n) = s.strip_suffix("K") {
        n.parse::<u64>().ok().map(|n| n * 1_000)
    } else {
        s.parse::<u64>().ok()
    }
}

/// Read an i32 from the reader.
fn read_i32<R: Read>(reader: &mut R) -> Result<i32, AppError> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read i32: {e}"),
        })?;
    Ok(i32::from_le_bytes(bytes))
}

/// Read an i64 from the reader.
fn read_i64<R: Read>(reader: &mut R) -> Result<i64, AppError> {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read i64: {e}"),
        })?;
    Ok(i64::from_le_bytes(bytes))
}

/// Read a u16 from the reader.
fn read_u16<R: Read>(reader: &mut R) -> Result<u16, AppError> {
    let mut bytes = [0u8; 2];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read u16: {e}"),
        })?;
    Ok(u16::from_le_bytes(bytes))
}

/// Read an f32 from the reader.
fn read_f32<R: Read>(reader: &mut R) -> Result<f32, AppError> {
    let mut bytes = [0u8; 4];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read f32: {e}"),
        })?;
    Ok(f32::from_le_bytes(bytes))
}

/// Read an f64 from the reader.
fn read_f64<R: Read>(reader: &mut R) -> Result<f64, AppError> {
    let mut bytes = [0u8; 8];
    reader
        .read_exact(&mut bytes)
        .map_err(|e| AppError::GgufParse {
            message: format!("failed to read f64: {e}"),
        })?;
    Ok(f64::from_le_bytes(bytes))
}

/// Architectures that are speculative-decoding DRAFT models (companion
/// models used with `-md`/`--spec-type`). These are not
/// standalone-launchable and must not enter the catalogue as models.
const DRAFT_ARCHITECTURES: &[&str] = &[
    "dflash", "dflash2", "dspark", "dspark2", "eagle", "eagle2", "eagle3", "eagle4",
];

/// Detect whether an architecture is a speculative-decoding draft model.
pub fn is_draft_architecture(arch: &str) -> bool {
    DRAFT_ARCHITECTURES.contains(&arch.to_lowercase().as_str())
}

/// The chat template the file itself carries (`tokenizer.chat_template`) —
/// the writer's own statement of how the model is meant to be prompted.
/// T-037 reads the two capability questions below out of it.
pub fn chat_template(metadata: &std::collections::HashMap<String, GgufValue>) -> Option<&str> {
    metadata
        .get("tokenizer.chat_template")
        .and_then(|v| match v {
            GgufValue::String(s) => Some(s.as_str()),
            _ => None,
        })
}

/// One recognised way a writer spells a capability inside its chat template.
///
/// A capability is claimed when **any** protocol in its list matches. That is
/// deliberate: the writers disagree with each other, not with themselves, and a
/// template is written by exactly one writer.
///
/// The table is the extension point. Adding a writer is one entry plus the
/// fixture/test that proves it — never a marker typed from memory. Keep each
/// entry's `provenance` re-openable (a file path, or a URL a future session can
/// fetch again), because a marker whose source is gone cannot be re-checked
/// when a template changes.
struct TemplateProtocol {
    /// Substrings the template must **all** contain.
    ///
    /// An entry with more than one string is a delimiter pair: a template that
    /// opens a reasoning block without closing it is not a reasoning block, and
    /// requiring both is also what keeps a template that merely *mentions* the
    /// word (a system prompt, a doc string) from qualifying. An empty list
    /// would match every template in existence — a test forbids it.
    all_of: &'static [&'static str],
    /// Where these markers were read from, and when. Not decoration: it is the
    /// audit trail that lets a later session re-verify instead of re-guessing.
    provenance: &'static str,
}

/// T-037 — the writers recognised as declaring **tool calling** in their own
/// chat template.
///
/// Every row below was read out of a real template, and the source is named on
/// the row so a later session can re-fetch it. The Qwen row is byte-verified
/// against a file on this machine; the others were fetched from Hugging Face
/// raw template files on 16 Sept 2026 and grepped programmatically (the exact
/// substrings, with their occurrence counts, are in the row's note). Nothing
/// here is from memory, and a family that is absent from this table is a family
/// whose template this project does not recognise — its tag is omitted.
const TOOL_PROTOCOLS: &[TemplateProtocol] = &[
    TemplateProtocol {
        all_of: &["<tool_call>"],
        provenance: "Qwen + GLM families — byte-verified in the owner's local \
                     Qwen3.8-27B-NVFP4-MTP-HIGH.gguf (x22) and grepped the same day in the raw \
                     templates of Qwen/Qwen2.5-72B-Instruct (x3), Qwen/Qwen3-8B (x3), \
                     Qwen/QwQ-32B (x3) and zai-org/GLM-4.5 (x2) on huggingface.co",
    },
    TemplateProtocol {
        all_of: &["[TOOL_CALLS]"],
        provenance: "Mistral family — grepped in mistralai/Mistral-7B-Instruct-v0.3 \
                     (tokenizer_config.json) and unsloth/Mistral-Small-3.2-24B-Instruct-2506 \
                     (chat_template.jinja), 16 Sept 2026; both also carry [AVAILABLE_TOOLS]",
    },
    TemplateProtocol {
        all_of: &["<|python_tag|>"],
        provenance: "Llama 3.1/3.2/3.3 Instruct — grepped in \
                     unsloth/Meta-Llama-3.1-8B-Instruct (tokenizer_config.json), 16 Sept 2026 \
                     (the official meta-llama repos are gated; the mirror carries the same template)",
    },
    TemplateProtocol {
        all_of: &["<|tool_calls_section_begin|>"],
        provenance: "Kimi K2 (Moonshot) — grepped in moonshotai/Kimi-K2-Instruct and \
                     moonshotai/Kimi-K2-Thinking (chat_template.jinja), 16 Sept 2026; the \
                     section header precedes <|tool_call_begin|> and <|tool_call_argument_begin|>",
    },
    TemplateProtocol {
        all_of: &["<|tool|>"],
        provenance: "Phi-4-mini-instruct (Microsoft) — grepped in \
                     microsoft/Phi-4-mini-instruct (tokenizer_config.json), 16 Sept 2026: the \
                     template renders the tools list between <|tool|> and <|/tool|>",
    },
    TemplateProtocol {
        all_of: &["<tool_calls>"],
        provenance: "MiniMax-M1 — grepped in MiniMaxAI/MiniMax-M1-80k \
                     (tokenizer_config.json), 16 Sept 2026, alongside <tools>/</tools>",
    },
    TemplateProtocol {
        all_of: &["<TOOLCALL>"],
        provenance: "NVIDIA Nemotron Nano 9B v2 — grepped in \
                     nvidia/NVIDIA-Nemotron-Nano-9B-v2 (tokenizer_config.json), 16 Sept 2026: \
                     the template shows <TOOLCALL>{\"name\": …, \"arguments\": …}</TOOLCALL>",
    },
    TemplateProtocol {
        all_of: &["<｜tool▁call▁begin｜>"],
        provenance: "DeepSeek V3/V3.1/R1-distill — grepped in deepseek-ai/DeepSeek-V3.1 and \
                     deepseek-ai/DeepSeek-R1 (tokenizer_config.json), 16 Sept 2026. Note the \
                     spelling: the bars are U+FF5C and the spaces are U+2581, not ASCII",
    },
];

/// T-037 — the writers recognised as declaring a **reasoning block** in their own
/// chat template.
const THINKING_PROTOCOLS: &[TemplateProtocol] = &[
    TemplateProtocol {
        all_of: &["<think>", "</think>"],
        provenance: "One delimiter pair shared across families — required together, so an \
                     opening tag without its closing one does not qualify. Byte-verified in the \
                     owner's local Qwen3.8-27B-NVFP4-MTP-HIGH.gguf (2026-09-16, character codes \
                     recorded in PROGRESS.md F-017) and grepped the same day in the raw templates \
                     of Qwen/Qwen3-8B, Qwen/QwQ-32B, deepseek-ai/DeepSeek-R1, \
                     deepseek-ai/DeepSeek-V3.1, zai-org/GLM-4.5, moonshotai/Kimi-K2-Thinking, \
                     MiniMaxAI/MiniMax-M2 and nvidia/NVIDIA-Nemotron-Nano-9B-v2",
    },
    TemplateProtocol {
        all_of: &["<|channel|>analysis"],
        provenance: "gpt-oss (20b/120b) — grepped in openai/gpt-oss-20b (chat_template.jinja), \
                     16 Sept 2026: reasoning is a channel, not a block — the template emits \
                     <|start|>assistant<|channel|>analysis<|message|> for it",
    },
];

/// The first recognised protocol that matches this template, if any.
fn matched_protocol<'a>(
    template: &str,
    protocols: &'a [TemplateProtocol],
) -> Option<&'a TemplateProtocol> {
    protocols.iter().find(|protocol| {
        protocol
            .all_of
            .iter()
            .all(|marker| template.contains(marker))
    })
}

/// Does any recognised protocol match this template?
fn template_matches_any(template: &str, protocols: &[TemplateProtocol]) -> bool {
    matched_protocol(template, protocols).is_some()
}

/// T-037 — does the model's own chat template declare a tool-calling format?
///
/// The evidence is the template's own rendering of the reply protocol, never a
/// substring of the filename. A writer whose protocol is not in
/// [`TOOL_PROTOCOLS`] is **not** guessed at — the tag is omitted, which is
/// T-037's rule for a header that cannot answer. Widening recognition is one row
/// in that table plus the evidence for it.
pub fn template_declares_tool_use(template: &str) -> bool {
    template_matches_any(template, TOOL_PROTOCOLS)
}

/// T-037 — does the model's own chat template declare a reasoning block?
///
/// Same source, same rule, same extension point as above.
pub fn template_declares_thinking(template: &str) -> bool {
    template_matches_any(template, THINKING_PROTOCOLS)
}

/// Parse GGUF metadata from a file path.
pub fn parse_file(path: &Path) -> Result<GgufMetadata, AppError> {
    let file = File::open(path).map_err(|e| AppError::GgufParse {
        message: format!("failed to open {}: {e}", path.display()),
    })?;
    parse_metadata(file)
}

/// Check if a path is a projector file (contains "mmproj" in the name).
pub fn is_projector(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().contains("mmproj"))
        .unwrap_or(false)
}

/// Parse a shard index from a filename like "model-00001-of-00003.gguf".
/// Returns (shard_index, total_shards) or None if not a shard.
pub fn parse_shard_info(path: &Path) -> Option<(u32, u32)> {
    let name = path.file_name()?.to_string_lossy();
    if let Some(pos) = name.find("-of-") {
        let prefix = &name[..pos];
        let suffix = &name[pos + 4..];
        if let Some(dot) = suffix.find(".gguf") {
            let total_str = &suffix[..dot];
            if let Ok(total) = total_str.parse::<u32>() {
                if let Some(dash_pos) = prefix.rfind('-') {
                    let shard_str = &prefix[dash_pos + 1..];
                    if let Ok(shard) = shard_str.parse::<u32>() {
                        return Some((shard, total));
                    }
                }
            }
        }
    }
    None
}

/// Resolve all files for a multi-part model from any member.
/// Returns a list of all expected shard paths.
pub fn resolve_model_files(path: &Path) -> Result<Vec<PathBuf>, AppError> {
    let info = parse_shard_info(path);
    let (_shard_idx, total) = match info {
        Some(i) => i,
        None => return Ok(vec![path.to_path_buf()]),
    };

    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| AppError::GgufParse {
            message: format!("path has no file name: {}", path.display()),
        })?
        .to_string_lossy();
    // Find the shard pattern -NNNNN-of- and extract base name before it
    let base_name = name
        .split("-of-")
        .next()
        .ok_or_else(|| AppError::GgufParse {
            message: "failed to parse shard base name".to_string(),
        })?;
    // base_name now includes the shard index, strip it
    let base = base_name
        .rsplit_once('-')
        .ok_or_else(|| AppError::GgufParse {
            message: "failed to parse shard base name".to_string(),
        })?
        .0;

    let mut shards = Vec::new();
    for i in 1..=total {
        let shard_path = dir.join(format!("{base}-{i:05}-of-{total:05}.gguf"));
        if !shard_path.exists() {
            return Err(AppError::GgufParse {
                message: format!(
                    "missing shard {} of {} (expected {})",
                    i,
                    total,
                    shard_path.display()
                ),
            });
        }
        shards.push(shard_path);
    }
    Ok(shards)
}

/// A model set: main model files plus optional projector.
#[derive(Debug)]
pub struct ModelSet {
    pub model_files: Vec<PathBuf>,
    pub projector: Option<PathBuf>,
}

/// Scan a directory for GGUF models, resolving shards and projectors.
pub fn scan_directory(dir: &Path) -> Result<Vec<ModelSet>, AppError> {
    let entries = std::fs::read_dir(dir).map_err(|e| AppError::GgufParse {
        message: format!("failed to read directory {}: {e}", dir.display()),
    })?;

    let mut models: Vec<PathBuf> = Vec::new();
    let mut projectors: Vec<PathBuf> = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| AppError::GgufParse {
            message: format!("failed to read directory entry: {e}"),
        })?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path
            .file_name()
            .ok_or_else(|| AppError::GgufParse {
                message: format!("path has no file name: {}", path.display()),
            })?
            .to_string_lossy();
        if !name.ends_with(".gguf") {
            continue;
        }
        if is_projector(&path) {
            projectors.push(path);
        } else {
            models.push(path);
        }
    }

    // Resolve shard sets
    let mut sets: Vec<ModelSet> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in models {
        let name = path
            .file_name()
            .ok_or_else(|| AppError::GgufParse {
                message: format!("path has no file name: {}", path.display()),
            })?
            .to_string_lossy()
            .to_string();
        if seen.contains(&name) {
            continue;
        }
        let files = resolve_model_files(&path)?;
        for f in &files {
            seen.insert(
                f.file_name()
                    .ok_or_else(|| AppError::GgufParse {
                        message: format!("path has no file name: {}", f.display()),
                    })?
                    .to_string_lossy()
                    .to_string(),
            );
        }
        sets.push(ModelSet {
            model_files: files,
            projector: None,
        });
    }

    // Match projectors to models. The `mmproj-` prefix alone is not enough:
    // real projectors are named like `mmproj-F16.gguf` (quant tag), not
    // `mmproj-<model>.gguf`, so a name-prefix match almost never fires.
    // The reliable association is the DIRECTORY: a projector sits next to the
    // model it augments. When a directory holds a single model set, its
    // projector(s) belong to it. Only when several model sets share a
    // directory do we fall back to a name-prefix guess.
    for proj in projectors {
        let proj_dir = proj.parent().map(|p| p.to_path_buf());
        let proj_name = proj
            .file_name()
            .ok_or_else(|| AppError::GgufParse {
                message: format!("path has no file name: {}", proj.display()),
            })?
            .to_string_lossy()
            .to_string();
        // A projector's model base name: `mmproj-Foo.gguf` → `Foo`,
        // `Foo-mmproj-BF16.gguf` → `Foo`.
        let proj_stem = proj_name.strip_suffix(".gguf").unwrap_or(&proj_name);
        let model_base = proj_stem
            .strip_prefix("mmproj-")
            .or_else(|| proj_stem.strip_suffix("-mmproj"))
            .or_else(|| proj_stem.strip_suffix("-mmproj-BF16"))
            .map(|s| s.to_string())
            .unwrap_or_default();

        // Candidate sets in the same directory.
        let mut dir_candidates: Vec<&mut ModelSet> = sets
            .iter_mut()
            .filter(|s| {
                if let Some(first) = s.model_files.first() {
                    first.parent() == proj_dir.as_deref()
                } else {
                    false
                }
            })
            .collect();

        // Single model set in this directory: the projector is unambiguous.
        if dir_candidates.len() == 1 {
            dir_candidates[0].projector = Some(proj.clone());
            continue;
        }

        // Multiple model sets share the directory. A DRAFT set is not a
        // candidate host and must not be allowed to win the match below:
        // it is not a launchable model, llama.cpp loads a projector together
        // with the MAIN model, and a draft file never becomes a catalogue
        // entry — so a projector attached to it is a projector lost.
        //
        // Measured (T-037) on the owner's real
        // `ToBeStyled/Qwen3.8-27B-ColdFusion-GAIN-Blackwell-DFlash2-Ultra-V1.0`:
        // that directory holds the main `…-NVFP4.gguf` (arch `qwen35`), the
        // draft `…-DFlash2-NVFP4.gguf` (arch `dflash`) and
        // `…-mmproj-BF16.gguf`. The projector's base name
        // (`…-Ultra-V1.0`) is a prefix of BOTH file names, so the name rule
        // matched whichever `read_dir` returned first — the draft — and the
        // real model silently imported with `mmproj_path = null`.
        //
        // Excluding drafts costs one header read per extra candidate, and only
        // in a directory that actually carries a projector; the single-set
        // fast path above stays free. A head that cannot be parsed counts as
        // hostable (conservative: an unreadable file must not silently steal
        // the projector away from a readable one).
        if dir_candidates.len() > 1 {
            let mut hostable: Vec<&mut ModelSet> = Vec::new();
            for set in dir_candidates {
                let head_is_draft = set
                    .model_files
                    .first()
                    .and_then(|head| parse_file(head).ok())
                    .map(|meta| meta.is_draft_model)
                    .unwrap_or(false);
                if !head_is_draft {
                    hostable.push(set);
                }
            }

            // Exactly one set left that could host a projector: unambiguous.
            if hostable.len() == 1 {
                hostable[0].projector = Some(proj.clone());
                continue;
            }

            // Still several: fall back to a name-prefix match among them.
            for set in hostable {
                if let Some(first) = set.model_files.first() {
                    let model_name = first
                        .file_name()
                        .ok_or_else(|| AppError::GgufParse {
                            message: format!("path has no file name: {}", first.display()),
                        })?
                        .to_string_lossy()
                        .to_string();
                    if model_name.starts_with(&model_base) {
                        set.projector = Some(proj.clone());
                        break;
                    }
                }
            }
        }
    }

    Ok(sets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::time::Instant;

    fn make_gguf_bytes(tensor_count: u64, kv_pairs: Vec<(&str, GgufValue)>) -> Vec<u8> {
        let mut buf = Vec::new();
        // Magic
        buf.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        // Version
        buf.extend_from_slice(&GGUF_VERSION.to_le_bytes());
        // Tensor count
        buf.extend_from_slice(&tensor_count.to_le_bytes());
        // KV count
        buf.extend_from_slice(&(kv_pairs.len() as u64).to_le_bytes());

        // KV pairs
        for (key, value) in kv_pairs {
            // Key (string)
            let key_bytes = key.as_bytes();
            buf.extend_from_slice(&(key_bytes.len() as u64).to_le_bytes());
            buf.extend_from_slice(key_bytes);

            // Value type and data — wire types per the GGUF v3 spec:
            // string=8, u32=4, u64=10, i32=5, i64=11.
            match value {
                GgufValue::String(s) => {
                    buf.extend_from_slice(&8u32.to_le_bytes());
                    let s_bytes = s.as_bytes();
                    buf.extend_from_slice(&(s_bytes.len() as u64).to_le_bytes());
                    buf.extend_from_slice(s_bytes);
                }
                GgufValue::UInt32(n) => {
                    buf.extend_from_slice(&4u32.to_le_bytes());
                    buf.extend_from_slice(&n.to_le_bytes());
                }
                GgufValue::UInt64(n) => {
                    buf.extend_from_slice(&10u32.to_le_bytes());
                    buf.extend_from_slice(&n.to_le_bytes());
                }
                GgufValue::Int32(n) => {
                    buf.extend_from_slice(&5u32.to_le_bytes());
                    buf.extend_from_slice(&n.to_le_bytes());
                }
                GgufValue::Int64(n) => {
                    buf.extend_from_slice(&11u32.to_le_bytes());
                    buf.extend_from_slice(&n.to_le_bytes());
                }
                _ => unreachable!("unsupported value type in test"),
            }
        }

        buf
    }

    #[test]
    fn test_parse_llama_metadata() {
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("llama".to_string()),
            ),
            // general.file_type carries the FTYPE label on the wire (15 = Q4_K_M).
            ("general.file_type", GgufValue::UInt32(15)),
            ("llama.block_count", GgufValue::UInt32(32)),
            ("llama.context_length", GgufValue::UInt32(4096)),
            ("llama.embedding_length", GgufValue::UInt32(4096)),
            ("llama.attention.head_count", GgufValue::UInt32(32)),
            ("llama.attention.head_count_kv", GgufValue::UInt32(8)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert_eq!(meta.architecture, "llama");
        assert_eq!(meta.quantization, "Q4_K_M");
        assert_eq!(meta.block_count, 32);
        assert_eq!(meta.context_length, Some(4096));
        assert_eq!(meta.embedding_length, Some(4096));
        assert_eq!(meta.attention_head_count, Some(32));
        assert_eq!(meta.attention_head_count_kv, Some(8));
        assert!(!meta.is_moe);
    }

    #[test]
    fn test_draft_detection_is_architecture_based() {
        // T-036 acceptance: a draft model is classified by its
        // general.architecture value, never by a filename pattern.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("dflash".to_string()),
            ),
            ("dflash.block_count", GgufValue::UInt32(16)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert!(
            meta.is_draft_model,
            "dflash architecture must be a draft model"
        );
        assert!(!meta.has_mtp_heads);

        // A normal architecture is not a draft model.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("llama".to_string()),
            ),
            ("llama.block_count", GgufValue::UInt32(32)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert!(
            !meta.is_draft_model,
            "llama architecture is not a draft model"
        );
    }

    #[test]
    fn test_mtp_detection_is_header_based() {
        // T-036 acceptance: an MTP model is classified by the presence of
        // {arch}.nextn_predict_layers, never by a "MTP" substring in the
        // name. A normal architecture with the header key is MTP.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("qwen3".to_string()),
            ),
            ("qwen3.block_count", GgufValue::UInt32(64)),
            ("qwen3.nextn_predict_layers", GgufValue::UInt32(1)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert!(
            meta.has_mtp_heads,
            "qwen3 with nextn_predict_layers must be MTP"
        );
        assert!(
            !meta.is_draft_model,
            "a complete MTP model is not a draft companion"
        );

        // Same architecture without the header key is not MTP.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("qwen3".to_string()),
            ),
            ("qwen3.block_count", GgufValue::UInt32(64)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert!(
            !meta.has_mtp_heads,
            "no nextn_predict_layers key means no MTP heads"
        );
    }

    #[test]
    fn test_draft_and_mtp_signals_are_independent() {
        // A draft-architecture file must not be reported as MTP, and an
        // MTP file must not be reported as a draft companion — the two
        // roles are mutually exclusive by construction.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("eagle3".to_string()),
            ),
            ("eagle3.block_count", GgufValue::UInt32(10)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert!(meta.is_draft_model);
        assert!(!meta.has_mtp_heads);
    }

    #[test]
    fn test_t037_capability_detection_is_header_derived_not_name_derived() {
        // T-037 acceptance: every tag is header-derived, never name-derived.
        // The same bytes are written under a name that announces the
        // capabilities and under one that hides them; in both cases the answer
        // follows the header.
        let dir = std::env::temp_dir().join(format!(
            "lm-mgr-t037-caps-name-vs-header-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create fixture dir");

        // Header of a model whose template declares both capabilities.
        let template = concat!(
            "{%- set tools_ns = namespace(value=0) %}",
            "{%- if tools and tools is iterable %}",
            "{{- '<tool_call>' }}",
            "{%- endif %}",
            "{%- if enable_thinking is undefined or enable_thinking is true %}",
            "{{- '<think>' + reasoning_content + '</think>' }}",
            "{%- endif %}"
        );

        let declares_both = make_gguf_bytes(
            0,
            vec![
                (
                    "general.architecture",
                    GgufValue::String("qwen35".to_string()),
                ),
                ("qwen35.block_count", GgufValue::UInt32(65)),
                (
                    "tokenizer.chat_template",
                    GgufValue::String(template.to_string()),
                ),
            ],
        );
        // Same architecture, no template at all: the file answers nothing.
        let declares_nothing = make_gguf_bytes(
            0,
            vec![
                (
                    "general.architecture",
                    GgufValue::String("qwen35".to_string()),
                ),
                ("qwen35.block_count", GgufValue::UInt32(65)),
            ],
        );

        // The name announces every capability; the header has none of them.
        let loud = dir.join("Qwen3.8-27B-Tool-Calling-Thinking-MTP-Q4_K_M.gguf");
        std::fs::write(&loud, &declares_nothing).expect("write loud fixture");
        let meta = parse_file(&loud).expect("parse loud fixture");
        assert!(
            !meta.supports_tools,
            "a name is not evidence: no template means no Tool use tag"
        );
        assert!(
            !meta.supports_thinking,
            "a name is not evidence: no template means no Thinking tag"
        );
        assert!(
            !meta.has_mtp_heads,
            "a name is not evidence: no nextn_predict_layers means no MTP tag"
        );
        assert!(!meta.has_chat_template);

        // The name hides them; the header declares them.
        let quiet = dir.join("plain-llama-8b-Q4_K_M.gguf");
        std::fs::write(&quiet, &declares_both).expect("write quiet fixture");
        let meta = parse_file(&quiet).expect("parse quiet fixture");
        assert!(
            meta.supports_tools,
            "the header's tool-call protocol must win over a quiet name"
        );
        assert!(
            meta.supports_thinking,
            "the header's reasoning block must win over a quiet name"
        );
        assert!(meta.has_chat_template);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_t037_protocol_table_is_wellformed() {
        // The table is the extension point, so it needs a guard: an entry with
        // an empty `all_of` would match EVERY template in existence and tag
        // every model in the catalogue, and an entry without a source cannot be
        // re-verified when a writer changes its template.
        for (capability, protocols) in [
            ("tool use", TOOL_PROTOCOLS),
            ("thinking", THINKING_PROTOCOLS),
        ] {
            assert!(
                !protocols.is_empty(),
                "{capability}: no protocol recognised"
            );
            for protocol in protocols {
                assert!(
                    !protocol.all_of.is_empty(),
                    "{capability}: an empty all_of matches every template"
                );
                assert!(
                    protocol.all_of.iter().all(|m| !m.is_empty()),
                    "{capability}: an empty marker matches every template"
                );
                assert!(
                    protocol.provenance.len() > 20,
                    "{capability}: every protocol must name the source it was read from"
                );
            }
        }
    }

    #[test]
    fn test_t037_markers_themselves() {
        // Spelled exactly as the real Qwen3.8-27B-NVFP4-MTP-HIGH.gguf template
        // spells them, byte-verified (PROGRESS.md F-017 carries the character
        // codes): the token is plain, with no backslash — the `\n` that sits
        // beside it in the template is the Jinja escape and is not part of the
        // token. Spelling it with a backslash is the transcription trap that was
        // hit once here: the fixture built that way failed against the corrected
        // table.
        let real_template = concat!(
            "{%- if tools and tools is iterable %}",
            "{{- '# Tools: you have access to these functions:' }}",
            "{{- '<tool_call>' }}",
            "{%- endif %}",
            "{%- if enable_thinking is undefined or enable_thinking is true %}",
            "{{- '<think>' + reasoning_content + '</think>' }}",
            "{%- endif %}"
        );
        assert!(template_declares_tool_use(real_template));
        assert!(template_declares_thinking(real_template));

        // Prose is not a protocol. A template that merely talks about thinking
        // or tools declares neither.
        assert!(!template_declares_tool_use(""));
        assert!(!template_declares_tool_use(
            "tools are not available in this prompt"
        ));
        assert!(!template_declares_tool_use("<tool_callx>"));
        assert!(!template_declares_thinking(
            "You may think about the problem first."
        ));
        assert!(!template_declares_thinking(
            "reasoning_effort is set to high"
        ));
        assert!(!template_declares_thinking("thinking"));
        assert!(
            !template_declares_thinking("<think>"),
            "an opening delimiter with no closing one is not a reasoning block"
        );
        assert!(
            !template_declares_thinking("</think>"),
            "a closing delimiter with no opening one is not a reasoning block"
        );
    }

    #[test]
    fn test_t037_recognised_writers_match_their_own_spelling() {
        // One case per row of the two tables, using a verbatim excerpt of the
        // template that row's provenance names. A marker that stops matching its
        // own source would silently drop a tag from the catalogue, so this test
        // fails here instead.
        let cases: &[(&str, &str, bool, bool)] = &[
            // (writer, verbatim excerpt, expects Tool use, expects Thinking)
            (
                "Qwen3.8 (local file, byte-verified)",
                "{{- '<tool_call>\n<function=x>\n</tool_call>' }} <think>{reasoning}</think>",
                true,
                true,
            ),
            ("Mistral-7B-v0.3", "{{- \"[TOOL_CALLS] [\" }}", true, false),
            (
                "Llama-3.1",
                "{{- \"<|python_tag|>\" + tool_call.name + \".call(\" }}",
                true,
                false,
            ),
            (
                "Kimi-K2-Instruct",
                "<|tool_calls_section_begin|><|tool_call_begin|>id",
                true,
                false,
            ),
            (
                "Phi-4-mini",
                "<|tool|>[{\"name\": \"x\"}]<|/tool|>",
                true,
                false,
            ),
            ("MiniMax-M1", "<tool_calls>...</tool_calls>", true, false),
            (
                "Nemotron-Nano-9B-v2",
                "{{- '<TOOLCALL>[{\"name\": \"t\", \"arguments\": \"a\"}, ' -}}",
                true,
                false,
            ),
            (
                "DeepSeek-V3.1",
                "<｜tool▁call▁begin｜><｜tool▁sep｜>",
                true,
                false,
            ),
            (
                "Qwen3-8B / QwQ-32B / DeepSeek-R1 / GLM-4.5 / Kimi-K2-Thinking / MiniMax-M2",
                "{%- if reasoning_content -%}<think>{{ reasoning_content }}</think>{%- endif -%}",
                false,
                true,
            ),
            (
                "gpt-oss-20b",
                "{% if channel == 'analysis' -%}<|start|>assistant<|channel|>analysis<|message|>",
                false,
                true,
            ),
            // Families whose templates were checked and carry no recognised
            // protocol: the tag stays absent rather than being guessed.
            (
                "Gemma-3 (no tool protocol in its template)",
                "<start_of_turn>user\nhi<end_of_turn>\n<start_of_turn>model\n",
                false,
                false,
            ),
            (
                "Hunyuan-A13B (794-char template, no tools)",
                "<|startoftext|>{{ content }}<|extra_4|>",
                false,
                false,
            ),
            (
                "Mixtral-8x7B-Instruct-v0.1 (no tool protocol)",
                "[INST] hi [/INST] no tools here",
                false,
                false,
            ),
        ];

        for (writer, excerpt, expects_tools, expects_thinking) in cases {
            assert_eq!(
                template_declares_tool_use(excerpt),
                *expects_tools,
                "{writer}: Tool use should be {expects_tools}"
            );
            assert_eq!(
                template_declares_thinking(excerpt),
                *expects_thinking,
                "{writer}: Thinking should be {expects_thinking}"
            );
        }
    }

    #[test]
    fn test_t037_an_unrecognised_protocol_yields_no_tag() {
        // A writer whose template this project does not recognise answers
        // nothing: the tags stay absent rather than being guessed. The file
        // DOES carry a template — this is the "header cannot answer" case, not
        // the "no header" case.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("gemma2".to_string()),
            ),
            ("gemma2.block_count", GgufValue::UInt32(42)),
            (
                "tokenizer.chat_template",
                GgufValue::String("{{ messages[0].role }}: {{ messages[0].content }}".to_string()),
            ),
        ];
        let meta = parse_metadata(Cursor::new(make_gguf_bytes(0, kv))).unwrap();
        assert!(meta.has_chat_template);
        assert!(
            !meta.supports_tools,
            "an unrecognised tools protocol is omitted, never guessed"
        );
        assert!(
            !meta.supports_thinking,
            "an unrecognised thinking protocol is omitted, never guessed"
        );
    }

    #[test]
    fn test_parse_moe_metadata() {
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("mixtral".to_string()),
            ),
            // FTYPE 7 = Q8_0
            ("general.file_type", GgufValue::UInt32(7)),
            ("mixtral.block_count", GgufValue::UInt32(32)),
            ("mixtral.expert_count", GgufValue::UInt32(8)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert_eq!(meta.architecture, "mixtral");
        assert!(meta.is_moe);
        assert_eq!(meta.expert_count, Some(8));
    }

    #[test]
    fn test_parse_nvfp4_metadata() {
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("llama".to_string()),
            ),
            // FTYPE 32 = NVFP4 (unsloth/Blackwell writers)
            ("general.file_type", GgufValue::UInt32(32)),
            ("llama.block_count", GgufValue::UInt32(32)),
        ];
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert_eq!(meta.architecture, "llama");
        assert_eq!(meta.quantization, "NVFP4");
    }

    #[test]
    fn test_parse_size_label_fallback() {
        // Real-world headers (unsloth Qwen3.8-27B-NVFP4, captured 9 Sept
        // 2026) carry the parameter count in `general.size_label` as a
        // string like "27B" and have no `general.parameters` key at all.
        // The parser must fall back to the size label so the catalogue
        // shows a count instead of "?".
        let mut kv = vec![
            (
                "general.architecture",
                GgufValue::String("qwen35".to_string()),
            ),
            ("general.size_label", GgufValue::String("27B".to_string())),
            ("general.file_type", GgufValue::UInt32(7)),
            ("qwen35.block_count", GgufValue::UInt32(65)),
        ];
        let data = make_gguf_bytes(0, kv.clone());
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert_eq!(meta.architecture, "qwen35");
        assert_eq!(meta.param_count, Some(27_000_000_000));
        assert_eq!(meta.block_count, 65);

        // `general.parameters` wins when both keys are present.
        kv.push(("general.parameters", GgufValue::String("70B".to_string())));
        let data = make_gguf_bytes(0, kv);
        let meta = parse_metadata(Cursor::new(data)).unwrap();
        assert_eq!(meta.param_count, Some(70_000_000_000));
    }

    #[test]
    fn test_bad_magic() {
        let mut data = vec![0u8; 16];
        data[0] = 0xFF;
        let result = parse_metadata(Cursor::new(data));
        assert!(matches!(result, Err(AppError::GgufParse { .. })));
    }

    #[test]
    fn test_truncated_file() {
        let data = vec![
            (GGUF_MAGIC >> 0) as u8,
            (GGUF_MAGIC >> 8) as u8,
            (GGUF_MAGIC >> 16) as u8,
            (GGUF_MAGIC >> 24) as u8,
            3,
            0,
            0,
            0,
        ];
        let result = parse_metadata(Cursor::new(data));
        assert!(matches!(result, Err(AppError::GgufParse { .. })));
    }

    #[test]
    fn test_is_projector() {
        assert!(is_projector(Path::new("mmproj-Qwen2-VL-7B.gguf")));
        assert!(!is_projector(Path::new("Qwen2-VL-7B.gguf")));
    }

    #[test]
    fn test_parse_shard_info() {
        let info = parse_shard_info(Path::new("model-00001-of-00003.gguf"));
        assert_eq!(info, Some((1, 3)));

        let info = parse_shard_info(Path::new("model.gguf"));
        assert_eq!(info, None);
    }

    // Integration tests with real fixture files

    fn fixtures_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/gguf")
    }

    #[test]
    fn test_parse_llama_fixture() {
        let path = fixtures_dir().join("meta-llama-Llama-3.1-8B-Q4_K_M.gguf");
        let meta = parse_file(&path).expect("should parse llama fixture");
        assert_eq!(meta.architecture, "llama");
        assert_eq!(meta.quantization, "Q4_K_M");
        assert_eq!(meta.block_count, 32);
        assert_eq!(meta.context_length, Some(4096));
        assert_eq!(meta.embedding_length, Some(4096));
        assert_eq!(meta.attention_head_count, Some(32));
        assert_eq!(meta.attention_head_count_kv, Some(8));
        assert!(!meta.is_moe);
    }

    #[test]
    fn test_parse_moe_fixture() {
        let path = fixtures_dir().join("mistralai-Mixtral-8x7B-Q8_0.gguf");
        let meta = parse_file(&path).expect("should parse MoE fixture");
        assert_eq!(meta.architecture, "mixtral");
        assert!(meta.is_moe);
        assert_eq!(meta.expert_count, Some(8));
    }

    #[test]
    fn test_parse_nvfp4_fixture() {
        let path = fixtures_dir().join("meta-llama-Llama-3-7B-NVFP4.gguf");
        let meta = parse_file(&path).expect("should parse NVFP4 fixture");
        assert_eq!(meta.architecture, "llama");
        assert_eq!(meta.quantization, "NVFP4");
    }

    #[test]
    fn test_parse_gemma_fixture() {
        let path = fixtures_dir().join("google-gemma-Q4_K_M.gguf");
        let meta = parse_file(&path).expect("should parse gemma fixture");
        assert_eq!(meta.architecture, "gemma");
        assert_eq!(meta.embedding_length, Some(3072));
    }

    #[test]
    fn test_parse_phi_fixture() {
        let path = fixtures_dir().join("microsoft-Phi-3-mini-Q4_K_M.gguf");
        let meta = parse_file(&path).expect("should parse phi fixture");
        assert_eq!(meta.architecture, "phi3");
        assert_eq!(meta.embedding_length, Some(3072));
    }

    #[test]
    fn test_t038_shape_keys_are_read_from_the_arch_prefix() {
        // T-038 acceptance: the three fields the KV-cache term needs, on a file
        // whose architecture is not `llama`. The owner's real Qwen3.8 files
        // declare exactly these keys (`qwen35.…`), and reading a fixed
        // `llama.` name returned `None` for all three — which is what made the
        // estimate fall back to a default KV head count and misprice the cache.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("qwen35".to_string()),
            ),
            ("qwen35.block_count", GgufValue::UInt32(65)),
            ("qwen35.context_length", GgufValue::UInt32(262_144)),
            ("qwen35.embedding_length", GgufValue::UInt32(5120)),
            ("qwen35.attention.head_count", GgufValue::UInt32(24)),
            ("qwen35.attention.head_count_kv", GgufValue::UInt32(4)),
        ];
        let meta = parse_metadata(Cursor::new(make_gguf_bytes(0, kv))).unwrap();
        assert_eq!(meta.block_count, 65);
        assert_eq!(meta.context_length, Some(262_144));
        assert_eq!(meta.embedding_length, Some(5120));
        assert_eq!(meta.attention_head_count, Some(24));
        assert_eq!(meta.attention_head_count_kv, Some(4));
    }

    #[test]
    fn test_t038_a_wrong_prefix_is_not_a_fallback() {
        // The other half of the same rule. `llama.embedding_length` inside a
        // `gemma` file is not a dialect to be accommodated: it is a file no
        // writer produces, and it is what this repo's own fixtures used to
        // contain (regenerated in T-038 — see the gemma fixture test below).
        // Reading it anyway would have hidden the bug this task exists to fix.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("gemma".to_string()),
            ),
            ("llama.block_count", GgufValue::UInt32(32)),
            ("llama.embedding_length", GgufValue::UInt32(3072)),
        ];
        let meta = parse_metadata(Cursor::new(make_gguf_bytes(0, kv))).unwrap();
        assert_eq!(meta.block_count, 0);
        assert_eq!(meta.embedding_length, None);
    }

    #[test]
    fn test_t038_bare_suffix_keys_still_read() {
        // A writer that omits the architecture prefix entirely is still
        // readable — that is what the bare-suffix fallback is for, and it is
        // tried second, never first.
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("mystery".to_string()),
            ),
            ("block_count", GgufValue::UInt32(12)),
            ("embedding_length", GgufValue::UInt32(2048)),
            ("attention.head_count_kv", GgufValue::UInt32(2)),
        ];
        let meta = parse_metadata(Cursor::new(make_gguf_bytes(0, kv))).unwrap();
        assert_eq!(meta.block_count, 12);
        assert_eq!(meta.embedding_length, Some(2048));
        assert_eq!(meta.attention_head_count_kv, Some(2));
    }

    #[test]
    fn test_t038_gemma_fixture_uses_its_own_prefix() {
        // The committed fixture was regenerated in T-038: it now carries
        // `gemma.…` keys, the way a real file does. Before that it carried
        // `llama.…` under a `gemma` architecture, so the fixture and the
        // reader agreed with each other rather than with reality.
        let path = fixtures_dir().join("google-gemma-Q4_K_M.gguf");
        let meta = parse_file(&path).expect("should parse gemma fixture");
        assert_eq!(meta.architecture, "gemma");
        assert_eq!(meta.block_count, 32);
        assert_eq!(meta.context_length, Some(4096));
        assert_eq!(meta.embedding_length, Some(3072));
        assert_eq!(meta.attention_head_count, Some(32));
        assert_eq!(meta.attention_head_count_kv, Some(1));
    }

    #[test]
    fn test_t038_hybrid_fixture_carries_the_estimator_geometry() {
        // The same shape as the owner's real Qwen3.8-27B file: a hybrid
        // attention/SSM model with MTP layers. Every field the estimator's
        // T-038 terms are built from is read here.
        let path = fixtures_dir().join("synthetic-Qwen3.8-27B-Hybrid-NVFP4.gguf");
        let meta = parse_file(&path).expect("should parse hybrid fixture");
        assert_eq!(meta.architecture, "qwen35");
        assert_eq!(meta.block_count, 65);
        assert_eq!(meta.embedding_length, Some(5120));
        assert_eq!(meta.attention_head_count, Some(24));
        assert_eq!(meta.attention_head_count_kv, Some(4));
        assert_eq!(meta.attention_key_length, Some(256));
        assert_eq!(meta.attention_value_length, Some(256));
        assert_eq!(meta.full_attention_interval, Some(4));
        assert_eq!(meta.ssm_state_size, Some(128));
        assert_eq!(meta.ssm_inner_size, Some(6144));
        assert_eq!(meta.ssm_group_count, Some(16));
        assert_eq!(meta.ssm_conv_kernel, Some(4));
        assert_eq!(meta.mtp_layer_count, Some(1));
        assert!(meta.has_mtp_heads);
        assert!(!meta.is_draft_model);
    }

    #[test]
    fn test_sparse_file_performance() {
        let path = fixtures_dir().join("huge-sparse-model-Q4_K_M.gguf");
        if !path.exists() {
            eprintln!("sparse fixture not found, skipping");
            return;
        }

        let start = Instant::now();
        let meta = parse_file(&path).expect("should parse sparse file");
        let elapsed = start.elapsed();

        assert_eq!(meta.architecture, "llama");
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "sparse file parse took {:?} (expected < 500ms)",
            elapsed
        );
        eprintln!("sparse file parsed in {:?}", elapsed);
    }

    #[test]
    fn test_sparse_file_size() {
        let path = fixtures_dir().join("huge-sparse-model-Q4_K_M.gguf");
        if !path.exists() {
            eprintln!("sparse fixture not found, skipping");
            return;
        }

        // Verify file is sparse by checking it can be opened and read
        // (actual on-disk size verification requires platform-specific APIs)
        let file = std::fs::File::open(&path).expect("should open sparse file");
        let meta = file.metadata().expect("should get metadata");
        // Logical size should be ~65GB
        let size_gb = meta.len() as f64 / (1024.0 * 1024.0 * 1024.0);
        assert!(
            size_gb > 60.0,
            "sparse file logical size is {}GB (expected ~65GB)",
            size_gb
        );
        eprintln!("sparse file logical size: {:.1}GB", size_gb);
    }

    #[test]
    fn test_resolve_shard_set() {
        let path = fixtures_dir().join("meta-llama-Llama-3.1-70B-00001-of-00005.gguf");
        let files = resolve_model_files(&path).expect("should resolve shard set");
        assert_eq!(files.len(), 5);
    }

    #[test]
    fn test_scan_directory() {
        let dir = fixtures_dir();
        let models = scan_directory(&dir).expect("should scan directory");
        assert!(!models.is_empty());
    }

    #[test]
    fn test_scan_directory_associates_projector_by_directory() {
        // A projector sitting next to a model in the same directory is
        // associated with it, even when the name prefix does not match
        // (real projectors are named like `mmproj-F16.gguf`, not
        // `mmproj-<model>.gguf`). Regression test for the directory-based
        // association fix.
        let tmp = std::env::temp_dir().join(format!("lm-mgr-proj-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // A model and a projector whose name prefix does NOT match the model.
        std::fs::write(tmp.join("SomeModel-Q4_K_M.gguf"), [0u8; 16]).unwrap();
        std::fs::write(tmp.join("mmproj-F16.gguf"), [0u8; 16]).unwrap();

        let sets = scan_directory(&tmp).expect("should scan temp directory");
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(sets.len(), 1, "one model set expected");
        let proj = sets[0]
            .projector
            .as_ref()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string());
        assert_eq!(
            proj,
            Some("mmproj-F16.gguf".to_string()),
            "projector in the same directory must be associated"
        );
    }

    #[test]
    fn test_t037_a_projector_never_lands_on_a_draft_set() {
        // T-037 regression, reproducing the owner's real directory:
        // `ToBeStyled/Qwen3.8-27B-ColdFusion-GAIN-Blackwell-DFlash2-Ultra-V1.0`
        // holds a main model (arch qwen35), a draft companion (arch dflash)
        // and one projector whose base name is a prefix of BOTH file names.
        // Before the fix the projector went to whichever `read_dir` returned
        // first — the draft — and the real model imported with no projector,
        // so the capability tags showed no Vision.
        let tmp =
            std::env::temp_dir().join(format!("lm-mgr-t037-proj-draft-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let base = "Qwen3.8-27B-ColdFusion-GAIN-Blackwell-DFlash2-Ultra-V1.0";
        let main_name = format!("{base}-NVFP4.gguf");
        let draft_name = format!("{base}-DFlash2-NVFP4.gguf");
        let proj_name = format!("{base}-mmproj-BF16.gguf");

        std::fs::write(
            tmp.join(&main_name),
            make_gguf_bytes(
                0,
                vec![
                    (
                        "general.architecture",
                        GgufValue::String("qwen35".to_string()),
                    ),
                    ("qwen35.block_count", GgufValue::UInt32(65)),
                ],
            ),
        )
        .unwrap();
        std::fs::write(
            tmp.join(&draft_name),
            make_gguf_bytes(
                0,
                vec![
                    (
                        "general.architecture",
                        GgufValue::String("dflash".to_string()),
                    ),
                    ("dflash.block_count", GgufValue::UInt32(8)),
                ],
            ),
        )
        .unwrap();
        // A real header is not required for the projector itself.
        std::fs::write(tmp.join(&proj_name), [0u8; 16]).unwrap();

        let sets = scan_directory(&tmp).expect("should scan the temp directory");
        let _ = std::fs::remove_dir_all(&tmp);

        assert_eq!(sets.len(), 2, "a main set and a draft set");
        let head_name = |s: &ModelSet| {
            s.model_files
                .first()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default()
        };
        let projector_of = |name: &str| {
            sets.iter()
                .find(|s| head_name(s) == name)
                .and_then(|s| s.projector.as_ref())
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        };

        assert_eq!(
            projector_of(&main_name),
            Some(proj_name.clone()),
            "the projector belongs to the launchable model, not to a draft"
        );
        assert_eq!(
            projector_of(&draft_name),
            None,
            "a draft file is never a catalogue entry and cannot host a projector"
        );
    }

    #[test]
    fn test_t037_two_hostable_sets_still_match_by_name() {
        // The name-prefix rule must keep working where it is the only
        // discriminator: two launchable models in one directory.
        let tmp =
            std::env::temp_dir().join(format!("lm-mgr-t037-proj-two-mains-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let model = |arch: &str| {
            make_gguf_bytes(
                0,
                vec![
                    ("general.architecture", GgufValue::String(arch.to_string())),
                    ("llama.block_count", GgufValue::UInt32(32)),
                ],
            )
        };
        std::fs::write(tmp.join("alpha-8B-Q4_K_M.gguf"), model("llama")).unwrap();
        std::fs::write(tmp.join("beta-8B-Q4_K_M.gguf"), model("llama")).unwrap();
        std::fs::write(tmp.join("alpha-8B-mmproj-F16.gguf"), [0u8; 16]).unwrap();

        let sets = scan_directory(&tmp).expect("should scan the temp directory");
        let _ = std::fs::remove_dir_all(&tmp);

        let projector_of = |name: &str| {
            sets.iter()
                .find(|s| {
                    s.model_files
                        .first()
                        .and_then(|p| p.file_name())
                        .map(|n| n.to_string_lossy() == name)
                        .unwrap_or(false)
                })
                .and_then(|s| s.projector.as_ref())
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
        };

        assert_eq!(
            projector_of("alpha-8B-Q4_K_M.gguf"),
            Some("alpha-8B-mmproj-F16.gguf".to_string()),
            "the projector's base name selects its model among several"
        );
        assert_eq!(projector_of("beta-8B-Q4_K_M.gguf"), None);
    }

    #[test]
    fn test_t037_a_directory_of_only_draft_sets_attaches_nothing() {
        // If every set in the directory is a draft companion there is nothing
        // launchable for a projector to belong to: better unassigned than
        // attached to a file that will be rejected at import anyway.
        let tmp = std::env::temp_dir().join(format!(
            "lm-mgr-t037-proj-drafts-only-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        for name in ["a-DFlash2.gguf", "b-DSpark.gguf"] {
            std::fs::write(
                tmp.join(name),
                make_gguf_bytes(
                    0,
                    vec![
                        (
                            "general.architecture",
                            GgufValue::String("dflash".to_string()),
                        ),
                        ("dflash.block_count", GgufValue::UInt32(8)),
                    ],
                ),
            )
            .unwrap();
        }
        std::fs::write(tmp.join("shared-mmproj-F16.gguf"), [0u8; 16]).unwrap();

        let sets = scan_directory(&tmp).expect("should scan the temp directory");
        let _ = std::fs::remove_dir_all(&tmp);

        assert!(
            sets.iter().all(|s| s.projector.is_none()),
            "no launchable model in the directory means no projector assignment"
        );
    }

    #[test]
    fn test_real_gguf_file() {
        let path_str = std::env::var("LLAMA_MANAGER_REAL_GGUF");
        if let Ok(path_str) = path_str {
            let path = PathBuf::from(path_str);
            if !path.exists() {
                eprintln!("real GGUF file not found: {}", path.display());
                return;
            }
            let meta = parse_file(&path).expect("should parse real GGUF file");
            assert!(!meta.architecture.is_empty());
            assert!(!meta.quantization.is_empty());
            assert!(meta.block_count > 0);
            // T-038 acceptance: for a real file whose architecture is not
            // `llama`, the shape keys are arch-prefixed and the reader must
            // return them. They used to come back `None` here, which is what
            // sent the KV-cache term to a per-architecture default.
            if meta.architecture != "llama" {
                assert!(
                    meta.embedding_length.is_some()
                        && meta.attention_head_count.is_some()
                        && meta.attention_head_count_kv.is_some(),
                    "a real non-llama file declares {}.embedding_length / \
                     {}.attention.head_count(_kv): got embedding={:?} heads={:?} kv_heads={:?}",
                    meta.architecture,
                    meta.architecture,
                    meta.embedding_length,
                    meta.attention_head_count,
                    meta.attention_head_count_kv
                );
            }
            eprintln!(
                "parsed real GGUF: {} {} {} blocks params={:?} draft={} mtp={} tools={} thinking={}",
                meta.architecture,
                meta.quantization,
                meta.block_count,
                meta.param_count,
                meta.is_draft_model,
                meta.has_mtp_heads,
                meta.supports_tools,
                meta.supports_thinking
            );
            eprintln!(
                "T-038 geometry: ctx={:?} emb={:?} heads={:?} kv_heads={:?} key_len={:?} \
                 value_len={:?} attn_interval={:?} ssm=({:?},{:?},{:?},{:?}) mtp_layers={:?}",
                meta.context_length,
                meta.embedding_length,
                meta.attention_head_count,
                meta.attention_head_count_kv,
                meta.attention_key_length,
                meta.attention_value_length,
                meta.full_attention_interval,
                meta.ssm_state_size,
                meta.ssm_inner_size,
                meta.ssm_group_count,
                meta.ssm_conv_kernel,
                meta.mtp_layer_count,
            );
        } else {
            eprintln!("LLAMA_MANAGER_REAL_GGUF not set, skipping");
        }
    }

    /// T-037 — the markers above were read out of a real file, so the real file
    /// must produce the tags they describe. Gated on the same env var as
    /// `test_real_gguf_file`, and skipped unless that file is the one the
    /// markers came from: a different writer's template legitimately answers
    /// differently, and asserting otherwise would turn an evidence-based rule
    /// into a guess.
    #[test]
    fn test_t037_real_gguf_capabilities() {
        let Ok(path_str) = std::env::var("LLAMA_MANAGER_REAL_GGUF") else {
            eprintln!("LLAMA_MANAGER_REAL_GGUF not set, skipping");
            return;
        };
        let path = PathBuf::from(&path_str);
        if !path.exists() {
            eprintln!("real GGUF file not found: {}", path.display());
            return;
        }
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if !name.contains("Qwen3.8-27B-NVFP4-MTP") {
            eprintln!(
                "skipping: {name} is not the file T-037's markers were read from \
                 (its own template decides its own tags)"
            );
            return;
        }
        let meta = parse_file(&path).expect("should parse real GGUF file");
        assert!(
            meta.has_chat_template,
            "the Qwen3.8 file carries a tokenizer.chat_template"
        );
        assert!(
            meta.has_mtp_heads,
            "qwen35.nextn_predict_layers = 1 is present in the Qwen3.8 MTP file"
        );
        assert!(
            meta.supports_tools,
            "the template renders a tools block and the <tool_call> protocol"
        );
        assert!(
            meta.supports_thinking,
            "the template opens a reasoning block with  thinking"
        );
        eprintln!(
            "T-037 real-file capabilities: tools={} thinking={} mtp={}",
            meta.supports_tools, meta.supports_thinking, meta.has_mtp_heads
        );
    }

    #[test]
    fn test_truncated_fixture() {
        let path = fixtures_dir().join("truncated.gguf");
        if !path.exists() {
            eprintln!("truncated fixture not found, skipping");
            return;
        }
        let result = parse_file(&path);
        assert!(matches!(result, Err(AppError::GgufParse { .. })));
    }

    #[test]
    fn test_corrupt_fixture() {
        let path = fixtures_dir().join("corrupt.gguf");
        if !path.exists() {
            eprintln!("corrupt fixture not found, skipping");
            return;
        }
        let result = parse_file(&path);
        assert!(matches!(result, Err(AppError::GgufParse { .. })));
    }

    // Proptest: mutated headers should not panic or allocate unboundedly
    #[test]
    fn test_proptest_random_bytes_no_panic() {
        fn check(data: Vec<u8>) {
            let result = parse_metadata(Cursor::new(data));
            // Should not panic; either success or typed error
            match result {
                Ok(_) => {}
                Err(AppError::GgufParse { .. }) => {}
                Err(e) => panic!("unexpected error type: {:?}", e),
            }
        }

        proptest::proptest!(|(data in proptest::collection::vec(0u8..255, 0..2048))| {
            check(data);
        });
    }

    #[test]
    fn test_proptest_malformed_headers_no_panic() {
        fn make_malformed(mut data: Vec<u8>) -> Vec<u8> {
            // Ensure at least 24 bytes (magic + version + counts)
            while data.len() < 24 {
                data.push(0);
            }
            // Corrupt magic
            data[0] = 0xFF;
            data
        }

        proptest::proptest!(|(data in proptest::collection::vec(0u8..255, 0..1024))| {
            let malformed = make_malformed(data);
            let result = parse_metadata(Cursor::new(malformed));
            assert!(matches!(result, Err(AppError::GgufParse { .. })));
        });
    }
}
