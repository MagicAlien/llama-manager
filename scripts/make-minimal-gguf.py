#!/usr/bin/env python3
"""
T-025 fixture generator: write a minimal GGUF file the router will accept
for registration without loading. Registration is not loading (PLAN.md §2.1),
so a valid header with zero tensors is the hypothesis being tested.

Produces a file that:
- Has the correct GGUF magic and version
- Has zero tensors (so no tensor data region)
- Has one metadata string (name) so the router has something to display
- Is small enough to be created programmatically, not hand-typed
"""

import struct
import sys

def write_gguf_header(path, name="llama-test"):
    """Write a minimal GGUF v3 file with zero tensors."""
    with open(path, "wb") as f:
        # Magic: "GGUF"
        f.write(b"GGUF")
        # Version: 3
        f.write(struct.pack("<I", 3))
        # Tensor count: 0
        f.write(struct.pack("<Q", 0))
        # Metadata: one string KV pair (general.name)
        # First write the KV pairs, then patch the count
        kv_offset = f.tell()
        # Reserve space for the KV count (we'll patch it)
        f.write(struct.pack("<Q", 1))  # 1 KV pair

        # KV pair: key "general.name", type string (8 per the GGUF v3 spec)
        # Write key string
        f.write(struct.pack("<Q", len("general.name")))
        f.write("general.name".encode("utf-8"))
        # Write value string
        f.write(struct.pack("<I", 8))  # type: string
        f.write(struct.pack("<Q", len(name)))
        f.write(name.encode("utf-8"))

        # Tensor info region: empty (0 tensors)
        # Done.

    print(f"Written: {path}")
    import os
    print(f"Size: {os.path.getsize(path)} bytes")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <output.gguf> <model-name>")
        sys.exit(1)
    write_gguf_header(sys.argv[1], sys.argv[2])
