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
      },
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