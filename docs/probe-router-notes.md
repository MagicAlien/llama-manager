# T-025 — Empirical router probe

Run the b9196-cpu build in router mode against prepared fixtures and answer the five questions T-023 could only infer from --help.

## Setup

Fixtures are minimal GGUF files created by `make-minimal-gguf.py`. They are not real models — they are just enough for the router to register them.

## Q1: Does --models-preset register a model by absolute path?

**Answer: YES.**

Command: `llama-server --models-preset fixtures/preset-absolute.ini --port 8090`

The preset file `fixtures/preset-absolute.ini` contains:
```ini
[test-external]
model = E:\LMM\fixtures\model-a.gguf
```

Response from `/v1/models` showed `test-external` registered with path `E:\LMM\fixtures\model-a.gguf`.

**Q1b: Non-existent path**

Command: `llama-server --models-preset fixtures/preset-abs-missing.ini --port 8091`

The preset file `fixtures/preset-abs-missing.ini` contains:
```ini
[test-missing]
model = C:\nonexistent\model.gguf
```

Response from `/v1/models` showed `test-missing` registered with path `C:\nonexistent\model.gguf`. The model appears in the list but is not loaded (lazy loading).

## Q2: Does --models-dir scan follow reparse points?

**Answer: YES, both file symlinks and directory junctions are followed.**

Setup:
- `C:\temp\probe-q2a\real.gguf` — real file
- `C:\temp\probe-q2a\symlink.gguf` — file symlink to `real.gguf`
- `C:\temp\probe-q2a\jdir` — directory junction to `C:\temp\probe-q2c` (which contains `junction-model.gguf`)

Command: `llama-server --models-dir C:/temp/probe-q2a --port 8092`

Response from `/v1/models` showed three models registered:
- `jdir` — from the directory junction
- `real` — from the real file
- `symlink` — from the file symlink

## Q3: What is the scan's real depth?

**Answer: One level.**

Setup:
- `C:\temp\probe-q3\top.gguf` — top-level model
- `C:\temp\probe-q3\sub1\level1.gguf` — one level down
- `C:\temp\probe-q3\sub1\sub2\level2.gguf` — two levels down

Command: `llama-server --models-dir C:/temp/probe-q3 --port 8093`

Response from `/v1/models` showed two models registered:
- `top` — from the top-level file
- `sub1` — from the subdirectory one level down

`level2` was NOT registered.

## Q4: How is a projector expressed on the PresetDeclaresPath channel?

**Answer: CRASHES. The preset channel does not support projector models.**

Setup:
- `C:\temp\probe-q4\vision-model.gguf` — vision model
- `C:\temp\probe-q4\mmproj-vision.gguf` — projector file

Preset file `fixtures/preset-mmproj.ini`:
```ini
[vision-test]
model = C:/temp/probe-q4/vision-model.gguf
mmproj = C:/temp/probe-q4/mmproj-vision.gguf
```

Command: `llama-server --models-preset fixtures/preset-mmproj.ini --port 8094`

Result: Server crashed with `GGML_ASSERT(type_to_gguf_type<T>::value == type) failed`. The `mmproj` key in a preset INI is not supported.

**Implication:** T-033 must use the scan channel for projector models, relying on the `mmproj-` filename prefix convention.

## Q5: Which health endpoint is a reliable proof of life with no model loaded?

**Answer: All three endpoints work with no model loaded.**

Command: `llama-server --port 8090` (no model, no preset, no scan directory)

- `/health` → 200, `{"status":"ok"}`
- `/props` → 200, returns router properties including `role: "router"`
- `/v1/models` → 200, returns empty data array `{"data":[]}`

All three are reliable proof of life with no model loaded. `/health` is the simplest and most conventional choice.

## Registration channel

The registration channel is **PresetDeclaresPath**. `--models-preset` accepts absolute paths, and the scan channel (`--models-dir`) is a secondary discovery mechanism that follows reparse points but has a one-level depth limit.

This means T-033 can use `--models-preset` with absolute paths for the primary registration channel, and `--models-dir` for additional discovery.