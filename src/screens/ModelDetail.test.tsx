import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import { ModelDetailScreen } from "./ModelDetail";

// T-036 — the speculative-decoding half of the model detail screen: the MTP
// badge comes from the GGUF header (never from the filename), and a draft
// companion is only ever accepted after the backend has validated it.

const plainModel = {
  id: "model-1",
  display_name: "Test Model",
  served_name: "test-model",
  file_path: "/path/to/model.gguf",
  shard_paths: [],
  size_bytes: 1073741824,
  sha256_head: "abc123",
  metadata: {
    architecture: "qwen35",
    param_count: 27000000000,
    quantization: "NVFP4",
    block_count: 64,
    context_length: 32768,
    embedding_length: 5120,
    attention_head_count: 40,
    attention_head_count_kv: 8,
    has_chat_template: true,
    is_moe: false,
    expert_count: null,
    is_draft_model: false,
    has_mtp_heads: false,
  },
  compatibility: "Supported",
  availability: "Present",
  duplicate_of: null,
  launch_params: {
    gpu_layers: 64,
    ctx_size: 8000,
    batch_size: 512,
    ubatch_size: 128,
    flash_attn: "Auto",
    cache_type_k: "f16",
    cache_type_v: "f16",
    n_cpu_moe: 16,
    tensor_split: null,
    main_gpu: 0,
    no_mmap: false,
    mlock: false,
    threads: 16,
    chat_template: null,
    mmproj_path: null,
    speculative: null,
    extra_args: [],
  },
  sampling_defaults: {
    temperature: 0.8,
    top_p: 0.9,
    top_k: 40,
    min_p: 0.05,
    repeat_penalty: 1.0,
    presence_penalty: null,
    frequency_penalty: null,
    seed: null,
  },
  preload: false,
  pinned: false,
  added_at: "2026-01-01T00:00:00Z",
};

const mtpModel = {
  ...plainModel,
  id: "model-mtp",
  display_name: "MTP Model",
  metadata: { ...plainModel.metadata, has_mtp_heads: true },
};

vi.mock("@/lib/ipc", () => ({
  getModel: vi.fn(),
  listModels: vi.fn().mockResolvedValue([]),
  estimateVram: vi.fn().mockResolvedValue({
    recommended_gpu_layers: 64,
    estimated_vram_bytes: 1000,
    estimated_ram_bytes: 1000,
    kv_cache_bytes: 100,
    vram_free_bytes: 2000,
    vram_total_bytes: 4000,
    projector_bytes: 0,
    fits_fully: true,
    notes: [],
  }),
  previewPresetParams: vi.fn().mockResolvedValue("[test-model]\n"),
  updateModelParams: vi.fn(),
  getServerState: vi.fn().mockResolvedValue("Stopped"),
  validateDraftCompanion: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(),
}));

beforeEach(() => {
  window.location.hash = "#/models/detail/model-1";
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe("ModelDetailScreen — T-036 speculative decoding", () => {
  it("shows the backend's reason when the preset preview is refused", async () => {
    // The real case in this repo: the build's registration channel is
    // `Undetermined`, so the generator refuses. The screen must say so, not
    // "[object Object]" and not an invented cause (D-015).
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (ipc.previewPresetParams as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "Internal",
      detail: { message: "registration channel is Undetermined; cannot preview preset" },
    });

    render(<ModelDetailScreen />);

    expect(
      await screen.findByText(
        /Preset preview unavailable: registration channel is Undetermined/,
      ),
    ).toBeTruthy();
  });

  it("shows the MTP badge for a model whose header carries MTP heads", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(mtpModel);

    render(<ModelDetailScreen />);

    expect(await screen.findByText("MTP")).toBeTruthy();
  });

  it("shows no MTP badge for a model without MTP heads", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Test Model")).toBeTruthy();
    expect(screen.queryByText("MTP")).toBeNull();
  });

  it("states that a plain model has no MTP heads and offers a draft model", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Speculative decoding")).toBeTruthy();
    expect(
      screen.getByText(
        "This model has no MTP heads. Attach a draft model to draft tokens with a companion.",
      ),
    ).toBeTruthy();
    expect(screen.getByText("Choose draft model…")).toBeTruthy();
    // No companion yet: no tuning controls, no role badge.
    expect(screen.queryByText("Draft tuning")).toBeNull();
    expect(screen.queryByText("Draft companion")).toBeNull();
  });

  it("refuses a chosen file that is not a draft model, showing the backend's reason", async () => {
    const ipc = await import("@/lib/ipc");
    const dialog = await import("@tauri-apps/plugin-dialog");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (dialog.open as ReturnType<typeof vi.fn>).mockResolvedValue("E:/models/main.gguf");
    (ipc.validateDraftCompanion as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "GgufParse",
      detail: {
        message:
          "draft companion E:/models/main.gguf has architecture 'qwen35', which is not a draft architecture",
      },
    });

    render(<ModelDetailScreen />);
    const button = await screen.findByText("Choose draft model…");
    button.click();

    await waitFor(() =>
      expect(
        screen.getByText(
          /This file cannot be used as a draft model\. draft companion E:\/models\/main\.gguf has architecture 'qwen35'/,
        ),
      ).toBeTruthy(),
    );
    // A refused candidate must not become the stored companion.
    expect(screen.queryByText("Draft tuning")).toBeNull();
    expect(screen.queryByText("Draft companion")).toBeNull();
  });

  it("accepts a validated draft companion and shows its architecture and spec type", async () => {
    const ipc = await import("@/lib/ipc");
    const dialog = await import("@tauri-apps/plugin-dialog");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (dialog.open as ReturnType<typeof vi.fn>).mockResolvedValue("E:/models/draft-dflash2.gguf");
    (ipc.validateDraftCompanion as ReturnType<typeof vi.fn>).mockResolvedValue({
      path: "E:/models/draft-dflash2.gguf",
      architecture: "dflash2",
      spec_type: "draft-dflash",
      size_bytes: 536870912,
      block_count: 16,
      quantization: "BF16",
    });

    render(<ModelDetailScreen />);
    const button = await screen.findByText("Choose draft model…");
    button.click();

    expect(await screen.findByText("Architecture: dflash2")).toBeTruthy();
    expect(screen.getByText("Speculative type: draft-dflash")).toBeTruthy();
    // The role is now visible on the screen, and the tuning controls are
    // reachable because the companion is validated.
    expect(screen.getByText("Draft companion")).toBeTruthy();
    expect(screen.getByText("Draft tuning")).toBeTruthy();
    expect(screen.getByText("Draft tokens (max)")).toBeTruthy();
    expect(screen.getByText("Draft K cache type")).toBeTruthy();
  });

  it("shows the draft tuning controls for an MTP model with no companion", async () => {
    // Owner refinement (14 Sept 2026): an MTP model drafts with its own heads,
    // so its draft-stage tuning must be reachable without picking a file.
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(mtpModel);

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Draft tuning")).toBeTruthy();
    expect(screen.getByText("Draft tokens (max)")).toBeTruthy();
    // No companion: no "Draft companion" badge, and no draft-model row.
    expect(screen.queryByText("Draft companion")).toBeNull();
    // The draft-model row is present and empty, next to the projector row.
    expect(screen.getByText("Draft model")).toBeTruthy();
  });

  it("shows the tuning controls for a companion already stored on the model", async () => {
    const ipc = await import("@/lib/ipc");
    const withCompanion = {
      ...plainModel,
      launch_params: {
        ...plainModel.launch_params,
        speculative: {
          draft_companion: "E:/models/draft-dspark.gguf",
          n_max: 4,
          n_min: null,
          p_min: null,
          threads: null,
          cache_type_k: null,
          cache_type_v: null,
        },
      },
    };
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(withCompanion);
    (ipc.validateDraftCompanion as ReturnType<typeof vi.fn>).mockResolvedValue({
      path: "E:/models/draft-dspark.gguf",
      architecture: "dspark2",
      spec_type: "draft-dspark",
      size_bytes: 1024,
      block_count: 8,
      quantization: "F16",
    });

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Speculative type: draft-dspark")).toBeTruthy();
    expect(screen.getByText("Draft tuning")).toBeTruthy();
  });
});
