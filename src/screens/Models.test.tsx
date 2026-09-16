import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { ModelsScreen } from "./Models";

vi.mock("@/lib/ipc", () => ({
  listModels: vi.fn().mockResolvedValue([
    {
      id: "model-1",
      display_name: "Test Model",
      served_name: "test-model",
      file_path: "/path/to/model.gguf",
      shard_paths: [],
      size_bytes: 1073741824,
      sha256_head: "abc123",
      metadata: {
        architecture: "llama",
        param_count: 8000,
        quantization: "Q4_K_M",
        block_count: 32,
        context_length: 4096,
        embedding_length: 4096,
        attention_head_count: 32,
        attention_head_count_kv: 8,
        has_chat_template: true,
        is_moe: false,
        expert_count: 0,
        expert_used_count: 0,
        supports_tools: true,
        supports_thinking: true,
      },
      capability_tags: ["Thinking", "Mtp", "Vision", "ToolUse"],
      compatibility: "Supported",
      availability: "Present",
      duplicate_of: null,
      launch_params: { gpu_layers: 30, ctx_size: 4096, batch_size: 512, threads: 8, flash_attn: true },
      sampling_defaults: { temperature: 0.7, top_p: 0.9, top_k: 40, repeat_penalty: 1.1 },
      preload: false,
      pinned: false,
      added_at: "2026-01-01T00:00:00Z",
    },
  ]),
  listWatchedFolders: vi.fn().mockResolvedValue([]),
  importModels: vi.fn().mockResolvedValue("job-1"),
  removeModel: vi.fn().mockResolvedValue(undefined),
  addWatchedFolder: vi.fn().mockResolvedValue([]),
  removeWatchedFolder: vi.fn().mockResolvedValue(undefined),
  previewPreset: vi.fn().mockResolvedValue(""),
  setModelPreload: vi.fn().mockResolvedValue({}),
  setModelPinned: vi.fn().mockResolvedValue({}),
  rescanModels: vi.fn().mockResolvedValue([]),
}));

afterEach(() => {
  cleanup();
});

describe("ModelsScreen", () => {
  it("renders model list", async () => {
    render(<ModelsScreen />);
    expect(await screen.findByText("Test Model")).toBeTruthy();
  });

  it("shows empty state when no models", async () => {
    const { listModels } = await import("@/lib/ipc");
    (listModels as ReturnType<typeof vi.fn>).mockResolvedValueOnce([]);

    render(<ModelsScreen />);
    expect(await screen.findByText("No models imported yet")).toBeTruthy();
  });

  it("renders remove button", async () => {
    render(<ModelsScreen />);
    expect(await screen.findByText("Remove")).toBeTruthy();
  });

  it("renders a Details link to the model-detail route", async () => {
    render(<ModelsScreen />);
    const link = await screen.findByText("Details");
    // The link target carries the model id so the detail screen can load it.
    expect(link.closest("a")?.getAttribute("href")).toBe("#/models/detail/model-1");
  });
});

describe("ModelsScreen — T-037 capability tags", () => {
  it("renders the derived tags in one row above the model's own text", async () => {
    render(<ModelsScreen />);

    const name = await screen.findByText("Test Model");
    for (const label of ["Thinking", "MTP", "Vision", "Tool use"]) {
      expect(screen.getByText(label)).toBeTruthy();
    }
    // The row sits above the model's own text in the document, not in the
    // metadata line shared with the compatibility/availability badges.
    const row = screen.getByText("Thinking").parentElement;
    expect(row).toBeTruthy();
    expect(
      row!.compareDocumentPosition(name) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // One row: every tag shares the same container.
    expect(screen.getByText("Tool use").parentElement).toBe(row);
  });

  it("does not render an empty tag row for a model with no capability", async () => {
    const { listModels } = await import("@/lib/ipc");
    (listModels as ReturnType<typeof vi.fn>).mockResolvedValueOnce([
      {
        id: "model-plain",
        display_name: "Plain Model",
        served_name: "plain-model",
        file_path: "/path/to/plain.gguf",
        shard_paths: [],
        size_bytes: 1024,
        sha256_head: "abc",
        metadata: {
          architecture: "llama",
          param_count: 8000,
          quantization: "Q4_K_M",
          block_count: 32,
          context_length: 4096,
          embedding_length: 4096,
          attention_head_count: 32,
          attention_head_count_kv: 8,
          has_chat_template: false,
          is_moe: false,
          expert_count: 0,
          supports_tools: false,
          supports_thinking: false,
        },
        capability_tags: [],
        compatibility: "Supported",
        availability: "Present",
        duplicate_of: null,
        launch_params: {},
        sampling_defaults: {},
        preload: false,
        pinned: false,
        added_at: "2026-01-01T00:00:00Z",
      },
    ]);

    render(<ModelsScreen />);

    expect(await screen.findByText("Plain Model")).toBeTruthy();
    for (const label of ["Thinking", "MTP", "Vision", "Tool use"]) {
      expect(screen.queryByText(label)).toBeNull();
    }
  });

  it("no longer renders the always-true Supported and Present badges", async () => {
    render(<ModelsScreen />);

    await screen.findByText("Test Model");
    expect(screen.queryByText("Supported")).toBeNull();
    expect(screen.queryByText("Present")).toBeNull();
  });

  it("still renders the informative availability badge", async () => {
    const { listModels } = await import("@/lib/ipc");
    (listModels as ReturnType<typeof vi.fn>).mockResolvedValueOnce([
      {
        id: "model-missing",
        display_name: "Missing Model",
        served_name: "missing-model",
        file_path: "D:/gone/missing.gguf",
        shard_paths: [],
        size_bytes: 1024,
        sha256_head: "abc",
        metadata: {
          architecture: "llama",
          param_count: 8000,
          quantization: "Q4_K_M",
          block_count: 32,
          context_length: 4096,
          embedding_length: 4096,
          attention_head_count: 32,
          attention_head_count_kv: 8,
          has_chat_template: false,
          is_moe: false,
          expert_count: 0,
          supports_tools: false,
          supports_thinking: false,
        },
        capability_tags: [],
        compatibility: { SupportedWithWarnings: [{ text: "NVFP4", experimental: false }] },
        availability: "Missing",
        duplicate_of: null,
        launch_params: {},
        sampling_defaults: {},
        preload: false,
        pinned: false,
        added_at: "2026-01-01T00:00:00Z",
      },
    ]);

    render(<ModelsScreen />);

    expect(await screen.findByText("Missing Model")).toBeTruthy();
    expect(screen.getByText("Missing")).toBeTruthy();
    expect(screen.getByText("Warnings")).toBeTruthy();
  });
});