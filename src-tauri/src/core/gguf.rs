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

/// Architecture-specific layer count key.
/// The KV key carrying the block (layer) count for an architecture. The
/// GGUF convention prefixes every per-arch key with the architecture
/// string itself: `llama.block_count`, `qwen35.block_count`, ... — llama.cpp
/// reads `<arch>.block_count` generically, never a fixed `llama.` prefix.
/// The table below exists only for the (none, today) archs that deviate;
/// everything else uses the arch-prefixed key. The old fallback returned
/// `llama.block_count` for unknown archs, so a real Qwen3-35B
/// (`general.architecture = qwen35`) parsed with block_count = 0.
fn layer_count_key(arch: &str) -> String {
    match arch {
        "llama" | "mistral" | "qwen" | "qwen3" | "gemma" | "gemma2" | "phi" | "phi3"
        | "mixtral" | "deepseek2" | "llama4" | "smollm" | "stablelm" => {
            format!("{arch}.block_count")
        }
        other => format!("{other}.block_count"),
    }
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
enum GgufValue {
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

    let block_count = kv
        .get(layer_count_key(&architecture).as_str())
        .and_then(|v| match v {
            GgufValue::UInt32(n) => Some(*n),
            GgufValue::Int32(n) => Some(*n as u32),
            GgufValue::UInt64(n) => Some(*n as u32),
            GgufValue::Int64(n) => Some(*n as u32),
            _ => None,
        })
        .unwrap_or(0);

    let context_length = kv
        .get("llama.context_length")
        .or_else(|| kv.get("context_length"))
        .and_then(|v| match v {
            GgufValue::UInt32(n) => Some(*n),
            GgufValue::Int32(n) => Some(*n as u32),
            GgufValue::UInt64(n) => Some(*n as u32),
            GgufValue::Int64(n) => Some(*n as u32),
            _ => None,
        });

    let embedding_length = kv
        .get("llama.embedding_length")
        .or_else(|| kv.get("embedding_length"))
        .and_then(|v| match v {
            GgufValue::UInt32(n) => Some(*n),
            GgufValue::Int32(n) => Some(*n as u32),
            GgufValue::UInt64(n) => Some(*n as u32),
            GgufValue::Int64(n) => Some(*n as u32),
            _ => None,
        });

    let attention_head_count = kv
        .get("llama.attention.head_count")
        .or_else(|| kv.get("attention.head_count"))
        .and_then(|v| match v {
            GgufValue::UInt32(n) => Some(*n),
            GgufValue::Int32(n) => Some(*n as u32),
            GgufValue::UInt64(n) => Some(*n as u32),
            GgufValue::Int64(n) => Some(*n as u32),
            _ => None,
        });

    let attention_head_count_kv = kv
        .get("llama.attention.head_count_kv")
        .or_else(|| kv.get("attention.head_count_kv"))
        .and_then(|v| match v {
            GgufValue::UInt32(n) => Some(*n),
            GgufValue::Int32(n) => Some(*n as u32),
            GgufValue::UInt64(n) => Some(*n as u32),
            GgufValue::Int64(n) => Some(*n as u32),
            _ => None,
        });

    let has_chat_template = kv
        .get("tokenizer.chat_template")
        .map(|_| true)
        .unwrap_or(false);

    let is_moe = architecture == "mixtral"
        || architecture == "deepseek2"
        || kv
            .get("llama.expert_count")
            .and_then(|v| match v {
                GgufValue::UInt32(n) => Some(*n > 1),
                GgufValue::Int32(n) => Some(*n > 1),
                GgufValue::UInt64(n) => Some(*n > 1),
                GgufValue::Int64(n) => Some(*n > 1),
                _ => None,
            })
            .unwrap_or(false);

    let expert_count = kv.get("llama.expert_count").and_then(|v| match v {
        GgufValue::UInt32(n) => Some(*n),
        GgufValue::Int32(n) => Some(*n as u32),
        GgufValue::UInt64(n) => Some(*n as u32),
        GgufValue::Int64(n) => Some(*n as u32),
        _ => None,
    });

    Ok(GgufMetadata {
        architecture,
        param_count,
        quantization,
        block_count,
        context_length,
        embedding_length,
        attention_head_count,
        attention_head_count_kv,
        has_chat_template,
        is_moe,
        expert_count,
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

/// Parse GGUF metadata from a file path.
pub fn parse_file(path: &Path) -> Result<GgufMetadata, AppError> {
    let file = File::open(path).map_err(|e| AppError::GgufParse {
        message: format!("failed to open {}: {e}", path.display()),
    })?;
    parse_metadata(file)
}

/// Check if a path is a projector file (mmproj- prefix).
pub fn is_projector(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().starts_with("mmproj-"))
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

    // Match projectors to models by base name
    for proj in projectors {
        let proj_name = proj
            .file_name()
            .ok_or_else(|| AppError::GgufParse {
                message: format!("path has no file name: {}", proj.display()),
            })?
            .to_string_lossy()
            .to_string();
        let model_base = proj_name
            .strip_prefix("mmproj-")
            .map(|s| s.to_string())
            .unwrap_or_default();
        for set in &mut sets {
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
    fn test_parse_moe_metadata() {
        let kv = vec![
            (
                "general.architecture",
                GgufValue::String("mixtral".to_string()),
            ),
            // FTYPE 7 = Q8_0
            ("general.file_type", GgufValue::UInt32(7)),
            ("llama.block_count", GgufValue::UInt32(32)),
            ("llama.expert_count", GgufValue::UInt32(8)),
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
            eprintln!(
                "parsed real GGUF: {} {} {} blocks params={:?}",
                meta.architecture, meta.quantization, meta.block_count, meta.param_count
            );
        } else {
            eprintln!("LLAMA_MANAGER_REAL_GGUF not set, skipping");
        }
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
