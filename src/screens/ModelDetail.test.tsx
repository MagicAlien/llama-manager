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
    supports_tools: false,
    supports_thinking: false,
  },
  capability_tags: [],
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
  // T-037 — the tag list the backend derives; the screen renders it as given.
  capability_tags: ["Mtp"],
};

// T-037 — a model whose header declares everything the app can read.
const capableModel = {
  ...plainModel,
  id: "model-capable",
  display_name: "Capable Model",
  metadata: {
    ...plainModel.metadata,
    has_mtp_heads: true,
    supports_tools: true,
    supports_thinking: true,
  },
  capability_tags: ["Thinking", "Mtp", "Vision", "ToolUse"],
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
    draft_bytes: 0,
    recurrent_state_bytes: 0,
    mtp_draft_bytes: 0,
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
    // The real case in this repo: a build whose `--help` offers no
    // `--models-preset` has no registration channel, so the generator refuses
    // (T-039 keeps that refusal as the dead-man's switch). The screen must say
    // so, not "[object Object]" and not an invented cause (D-015).
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (ipc.previewPresetParams as ReturnType<typeof vi.fn>).mockRejectedValue({
      kind: "Internal",
      detail: {
        message:
          "this build's registration channel is not established: its --help does not settle how a model enters the router, and no preset interface was observed for it. Install or activate a build that lists --models-preset (re-verify this one if it does), then try again. No preset was written to disk.",
      },
    });

    render(<ModelDetailScreen />);

    expect(
      await screen.findByText(
        /Preset preview unavailable: this build's registration channel is not established/,
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

describe("ModelDetailScreen — T-037 capability tags", () => {
  it("renders every capability tag the backend derived, above the model's own text", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(capableModel);

    render(<ModelDetailScreen />);

    const name = await screen.findByText("Capable Model");
    for (const label of ["Thinking", "MTP", "Vision", "Tool use"]) {
      expect(screen.getByText(label)).toBeTruthy();
    }
    // "Above the model's own text" is a structural property, not a style one:
    // the tag row precedes the display name in the document.
    const row = screen.getByText("Thinking").parentElement;
    expect(row).toBeTruthy();
    expect(
      row!.compareDocumentPosition(name) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("renders no tag row at all for a model with no capability", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Test Model")).toBeTruthy();
    for (const label of ["Thinking", "MTP", "Vision", "Tool use"]) {
      expect(screen.queryByText(label)).toBeNull();
    }
  });
});

describe("ModelDetailScreen — T-038 projection terms", () => {
  /** The estimate the backend would return, with only the fields in play. */
  const estimate = (over: Record<string, unknown>) => ({
    recommended_gpu_layers: 65,
    estimated_vram_bytes: 20_000_000_000,
    estimated_ram_bytes: 268_435_456,
    kv_cache_bytes: 12_000_000_000,
    vram_free_bytes: 30 * 1024 ** 3,
    vram_total_bytes: 30 * 1024 ** 3,
    projector_bytes: 0,
    draft_bytes: 0,
    recurrent_state_bytes: 0,
    mtp_draft_bytes: 0,
    fits_fully: true,
    confidence: "Heuristic",
    notes: [],
    ...over,
  });

  it("shows the draft, recurrent and MTP terms only when they cost something", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (ipc.estimateVram as ReturnType<typeof vi.fn>).mockResolvedValue(
      estimate({
        // The owner's real hybrid model: a companion, 588 MiB of recurrent
        // state, and the MTP draft layer's own cache.
        draft_bytes: 1_104_831_776,
        recurrent_state_bytes: 616_562_688,
        mtp_draft_bytes: 2_097_152,
      }),
    );

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Draft companion: 1.0 GB")).toBeTruthy();
    expect(screen.getByText("Recurrent layer state: 588 MB")).toBeTruthy();
    expect(screen.getByText("MTP draft cache: 2 MB")).toBeTruthy();
  });

  it("renders none of those rows for a dense model", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (ipc.estimateVram as ReturnType<typeof vi.fn>).mockResolvedValue(estimate({}));

    render(<ModelDetailScreen />);

    expect(await screen.findByText("Test Model")).toBeTruthy();
    // A placeholder row for a term that does not apply would read as a bug (UI
    // rule: no fake information).
    expect(screen.queryByText(/^Draft companion/)).toBeNull();
    expect(screen.queryByText(/^Recurrent layer state/)).toBeNull();
    expect(screen.queryByText(/^MTP draft cache/)).toBeNull();
    expect(screen.queryByText(/^Vision/)).toBeNull();
  });

  it("states the verdict as a delta against the GPU's own VRAM", async () => {
    // Owner decision, 17 Sept 2026: the two machine figures were siblings of
    // the costs in the same row, so "GPU VRAM" read as a term of the estimate
    // and its refusal to react to the controls read as a bug. It is the budget
    // the delta is measured against, and the delta is what a user decides on.
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);

    (ipc.estimateVram as ReturnType<typeof vi.fn>).mockResolvedValue(
      estimate({
        estimated_vram_bytes: 20 * 1024 ** 3,
        vram_free_bytes: 30 * 1024 ** 3,
        vram_total_bytes: 32 * 1024 ** 3,
        fits_fully: true,
      }),
    );
    const { unmount } = render(<ModelDetailScreen />);
    expect(
      await screen.findByText(
        "Fits fully in VRAM — 10.0 GB headroom (30.0 GB free of 32.0 GB)",
      ),
    ).toBeTruthy();
    // The budget is no longer presented as a row alongside the costs.
    expect(screen.queryByText(/^GPU VRAM/)).toBeNull();
    unmount();

    // …and the same in the other direction: what to give up is the message.
    (ipc.estimateVram as ReturnType<typeof vi.fn>).mockResolvedValue(
      estimate({
        estimated_vram_bytes: 35 * 1024 ** 3,
        vram_free_bytes: 30 * 1024 ** 3,
        vram_total_bytes: 32 * 1024 ** 3,
        fits_fully: false,
      }),
    );
    render(<ModelDetailScreen />);
    expect(
      await screen.findByText(
        "Does not fully fit in VRAM — 5.0 GB over (30.0 GB free of 32.0 GB)",
      ),
    ).toBeTruthy();
  });

  it("lists the figures in the decided order", async () => {
    const ipc = await import("@/lib/ipc");
    (ipc.getModel as ReturnType<typeof vi.fn>).mockResolvedValue(plainModel);
    (ipc.estimateVram as ReturnType<typeof vi.fn>).mockResolvedValue(
      estimate({ projector_bytes: 888_000_000, mtp_draft_bytes: 819_200_000 }),
    );

    render(<ModelDetailScreen />);
    // Await a node that only exists once the debounced estimate has resolved:
    // the model name arrives first, the projection ~150 ms later.
    await screen.findByText(/^Estimated VRAM:/);

    // Structural, like T-037's "above the model's own text" check: the order is
    // asserted by document position, not by reading a rendered string.
    const order = [
      "Estimated VRAM",
      "KV cache",
      "MTP draft cache",
      "Vision",
      "Estimated RAM",
      "Recommended GPU layers",
    ];
    const nodes = order.map((label) => screen.getByText(new RegExp(`^${label}:`)));
    for (let i = 1; i < nodes.length; i += 1) {
      expect(
        nodes[i - 1].compareDocumentPosition(nodes[i]) &
          Node.DOCUMENT_POSITION_FOLLOWING,
      ).toBeTruthy();
    }
  });
});
