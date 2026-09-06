#!/usr/bin/env python3
"""
Synthetic GGUF fixture generator.

Usage:
    python scripts/make-gguf-fixture.py <output_dir> [options]

Options:
    --arch ARCH         Architecture name (default: llama)
    --params PARAMS     Parameter count string (default: 8B)
    --quant QUANT       Quantization version (default: Q4_K_M)
    --blocks N          Number of blocks (default: 32)
    --heads N           Number of attention heads (default: 32)
    --heads-kv N        Number of KV heads (default: 8)
    --context N         Context length (default: 4096)
    --shards N          Number of shards (default: 1)
    --sparse            Create sparse file (seek past data)
    --sparse-size GB    Sparse file size in GB (default: 65)
    --moe               Enable MoE (expert_count=8)
    --nvfp4             Use NVFP4 quantization
    --projector         Create mmproj- prefixed file
    --corrupt           Write corrupted file (bad magic)
    --truncated         Write truncated file
    --chat-template     Include tokenizer.chat_template
    --base-name NAME    Base name for model (default: synthetic)
"""

import argparse
import struct
import os
import sys


GGUF_MAGIC = 0x46554747  # "GGUF"
GGUF_VERSION = 3


def write_u8(f, v):
    f.write(struct.pack('<B', v))


def write_i8(f, v):
    f.write(struct.pack('<b', v))


def write_u16(f, v):
    f.write(struct.pack('<H', v))


def write_i16(f, v):
    f.write(struct.pack('<h', v))


def write_u32(f, v):
    f.write(struct.pack('<I', v))


def write_i32(f, v):
    f.write(struct.pack('<i', v))


def write_u64(f, v):
    f.write(struct.pack('<Q', v))


def write_i64(f, v):
    f.write(struct.pack('<q', v))


def write_f32(f, v):
    f.write(struct.pack('<f', v))


def write_f64(f, v):
    f.write(struct.pack('<d', v))


def write_bool(f, v):
    write_u8(f, 1 if v else 0)


def write_string(f, s):
    encoded = s.encode('utf-8')
    write_u64(f, len(encoded))
    f.write(encoded)


def write_kv_string(f, key, value):
    write_string(f, key)
    write_u32(f, 11)  # GGUF_TYPE_STRING
    write_string(f, value)


def write_kv_uint32(f, key, value):
    write_string(f, key)
    write_u32(f, 6)  # GGUF_TYPE_UINT32
    write_u32(f, value)


def write_kv_uint64(f, key, value):
    write_string(f, key)
    write_u32(f, 7)  # GGUF_TYPE_UINT64
    write_u64(f, value)


def write_tensor(f, name, shape, dtype=0):
    """Write a tensor descriptor (no actual data)."""
    write_string(f, name)
    write_u32(f, len(shape))
    for dim in shape:
        write_u64(f, dim)
    write_u32(f, dtype)
    write_u64(f, 0)  # offset (placeholder)


def create_gguf(filename, args):
    """Create a synthetic GGUF file with specified metadata."""
    arch = args.arch
    base_name = args.base_name

    # Determine shard naming
    if args.shards > 1:
        shard_idx = int(filename.split('-')[-1].split('.')[0])
        shard_total = args.shards
    else:
        shard_idx = 1
        shard_total = 1

    print(f"Creating {filename} (shard {shard_idx}/{shard_total})...")

    if args.sparse:
        # Create file with just header
        with open(filename, 'wb') as f:
            _write_gguf_content(f, args, arch, base_name, shard_idx, shard_total)
        
        # Set sparse flag using fsutil
        import subprocess
        result = subprocess.run(['fsutil', 'sparse', 'setflag', filename],
                               capture_output=True, text=True)
        if result.returncode != 0:
            print(f"Warning: fsutil sparse setflag failed: {result.stderr}")
        else:
            # Extend file to target size
            target_size = args.sparse_size * 1024 * 1024 * 1024
            with open(filename, 'r+b') as f:
                f.seek(target_size - 1)
                f.write(b'\x00')
            print(f"Extended to {args.sparse_size}GB (sparse)")
    else:
        with open(filename, 'wb') as f:
            _write_gguf_content(f, args, arch, base_name, shard_idx, shard_total)


def _write_gguf_content(f, args, arch, base_name, shard_idx, shard_total):
    """Write GGUF content to file."""

    # Magic
    write_u32(f, GGUF_MAGIC)

    # Version
    write_u32(f, GGUF_VERSION)

    # Tensor count (we'll write a few dummy tensors)
    tensor_count = 4
    write_u64(f, tensor_count)

    # KV pair count — calculate based on architecture
    kv_count = 4  # general: architecture, name, parameters, quantization_version
    if args.chat_template:
        kv_count += 1
    if args.moe:
        kv_count += 2
    if arch in ("llama", "mistral", "qwen", "qwen3", "mixtral", "deepseek2"):
        kv_count += 5  # block_count, context_length, embedding_length, head_count, head_count_kv
    elif arch in ("gemma", "gemma2"):
        kv_count += 5  # block_count, context_length, embedding_length, head_count, head_count_kv
    elif arch in ("phi", "phi3"):
        kv_count += 3  # block_count, context_length, embedding_length
    write_u64(f, kv_count)

    # KV pairs
    write_kv_string(f, "general.architecture", arch)
    write_kv_string(f, "general.name", f"{base_name}-{args.params}")
    write_kv_string(f, "general.parameters", args.params)
    write_kv_string(f, "general.quantization_version", args.quant)

    # Architecture-specific
    if arch in ("llama", "mistral", "qwen", "qwen3", "mixtral", "deepseek2"):
        write_kv_uint32(f, "llama.block_count", args.blocks)
        write_kv_uint32(f, "llama.context_length", args.context)
        write_kv_uint32(f, "llama.embedding_length", 4096)
        write_kv_uint32(f, "llama.attention.head_count", args.heads)
        write_kv_uint32(f, "llama.attention.head_count_kv", args.heads_kv)
    elif arch in ("gemma", "gemma2"):
        write_kv_uint32(f, "llama.block_count", args.blocks)
        write_kv_uint32(f, "llama.context_length", args.context)
        write_kv_uint32(f, "llama.embedding_length", 3072)
        write_kv_uint32(f, "llama.attention.head_count", args.heads)
        write_kv_uint32(f, "llama.attention.head_count_kv", 1)
    elif arch in ("phi", "phi3"):
        write_kv_uint32(f, "llama.block_count", args.blocks)
        write_kv_uint32(f, "llama.context_length", args.context)
        write_kv_uint32(f, "llama.embedding_length", 3072)

    if args.chat_template:
        write_kv_string(f, "tokenizer.chat_template", "{% for m in messages %}{{ m.content }}{% endfor %}")

    if args.moe:
        write_kv_uint32(f, "llama.expert_count", 8)
        write_kv_uint32(f, "llama.expert_used_count", 2)

    # Shard info
    if shard_total > 1:
        write_kv_uint32(f, "llama.file.type", 1)

    # Dummy tensor descriptors
    write_tensor(f, "token_embd.weight", [4096, 32000])
    write_tensor(f, "output.weight", [32000, 4096])
    write_tensor(f, "norm.weight", [4096])
    write_tensor(f, "blk.0.attn_q.weight", [4096, 4096])


def main():
    parser = argparse.ArgumentParser(description="Generate synthetic GGUF fixtures")
    parser.add_argument("output_dir", help="Output directory")
    parser.add_argument("--arch", default="llama", help="Architecture name")
    parser.add_argument("--params", default="8B", help="Parameter count string")
    parser.add_argument("--quant", default="Q4_K_M", help="Quantization version")
    parser.add_argument("--blocks", type=int, default=32, help="Number of blocks")
    parser.add_argument("--heads", type=int, default=32, help="Attention heads")
    parser.add_argument("--heads-kv", type=int, default=8, help="KV heads")
    parser.add_argument("--context", type=int, default=4096, help="Context length")
    parser.add_argument("--shards", type=int, default=1, help="Number of shards")
    parser.add_argument("--sparse", action="store_true", help="Create sparse file")
    parser.add_argument("--sparse-size", type=int, default=65, help="Sparse file size in GB")
    parser.add_argument("--moe", action="store_true", help="Enable MoE")
    parser.add_argument("--nvfp4", action="store_true", help="Use NVFP4 quantization")
    parser.add_argument("--projector", action="store_true", help="Create projector file")
    parser.add_argument("--corrupt", action="store_true", help="Write corrupted file")
    parser.add_argument("--truncated", action="store_true", help="Write truncated file")
    parser.add_argument("--chat-template", action="store_true", help="Include chat template")
    parser.add_argument("--base-name", default="synthetic", help="Base model name")

    args = parser.parse_args()

    # Handle special cases
    if args.nvfp4:
        args.quant = "NVFP4"

    os.makedirs(args.output_dir, exist_ok=True)

    if args.corrupt:
        filename = os.path.join(args.output_dir, "corrupt.gguf")
        with open(filename, 'wb') as f:
            write_u32(f, 0x00000000)  # Bad magic
            write_u32(f, GGUF_VERSION)
        print(f"Created corrupt fixture: {filename}")
        return

    if args.truncated:
        filename = os.path.join(args.output_dir, "truncated.gguf")
        with open(filename, 'wb') as f:
            write_u32(f, GGUF_MAGIC)
            write_u32(f, GGUF_VERSION)
            # Truncate after version (no tensor/KV counts)
        print(f"Created truncated fixture: {filename}")
        return

    if args.projector:
        filename = os.path.join(args.output_dir, f"mmproj-{args.arch}.gguf")
        with open(filename, 'wb') as f:
            _write_gguf_content(f, args, args.arch, f"mmproj-{args.arch}", 1, 1)
        print(f"Created projector fixture: {filename}")
        return

    # Create model file(s)
    if args.shards > 1:
        for i in range(1, args.shards + 1):
            shard_name = f"{args.base_name}-{'{:05}'.format(i)}-of-{'{:05}'.format(args.shards)}.gguf"
            shard_path = os.path.join(args.output_dir, shard_name)
            if args.sparse and i == 1:
                # Only make first shard sparse
                with open(shard_path, 'wb') as f:
                    f.seek(args.sparse_size * 1024 * 1024 * 1024 - 1)
                    f.write(b'\x00')
                with open(shard_path, 'r+b') as f:
                    _write_gguf_content(f, args, args.arch, args.base_name, i, args.shards)
            else:
                with open(shard_path, 'wb') as f:
                    _write_gguf_content(f, args, args.arch, args.base_name, i, args.shards)
            print(f"Created shard {i}/{args.shards}: {shard_path}")
    else:
        filename = f"{args.base_name}-{args.quant}.gguf"
        filepath = os.path.join(args.output_dir, filename)
        if args.sparse:
            with open(filepath, 'wb') as f:
                f.seek(args.sparse_size * 1024 * 1024 * 1024 - 1)
                f.write(b'\x00')
            with open(filepath, 'r+b') as f:
                _write_gguf_content(f, args, args.arch, args.base_name, 1, 1)
        else:
            with open(filepath, 'wb') as f:
                _write_gguf_content(f, args, args.arch, args.base_name, 1, 1)
        print(f"Created fixture: {filepath}")


if __name__ == "__main__":
    main()