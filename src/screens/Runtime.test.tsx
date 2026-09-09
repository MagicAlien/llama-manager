import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";

import App from "@/App";
import { strings } from "@/lib/strings";
import type { EnvironmentReport, HealthCheck, RuntimeBuild } from "@/lib/types";

// Mocks the `@/lib/ipc` surface this screen (and its always-rendered
// version-management section) touches: `probeEnvironment` (T-011) and the
// three version-management data calls (T-024). `VersionManagementView` now
// always renders below the health section (9 Sept 2026 owner decision), so
// its data calls must not reach the real Tauri `invoke` in jsdom. Defaults
// resolve to an empty catalogue / Stopped server / no releases; tests that
// need builds override with `mockResolvedValueOnce`. Every other
// `src/lib/ipc.ts` export passes through untouched, so a future screen that
// also imports from this module is unaffected by this file's mock.
vi.mock("@/lib/ipc", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/ipc")>();
  return {
    ...actual,
    probeEnvironment: vi.fn(),
    listRuntimes: vi.fn().mockResolvedValue([]),
    getServerState: vi.fn().mockResolvedValue("Stopped"),
    checkForUpdates: vi.fn().mockResolvedValue([]),
  };
});

import { listRuntimes, probeEnvironment } from "@/lib/ipc";

const mockedProbeEnvironment = vi.mocked(probeEnvironment);
const mockedListRuntimes = vi.mocked(listRuntimes);

const healthCopy = strings.screens.runtime.healthCheck;

afterEach(() => {
  cleanup();
  window.location.hash = "";
  vi.clearAllMocks();
});

// Navigates exactly the way src/App.test.tsx does (T-005's established
// pattern): drive window.location.hash and dispatch the same
// `hashchange` event a real `<a href="#/...">` click produces.
function goToRuntime() {
  window.location.hash = "#/runtime";
  window.dispatchEvent(new HashChangeEvent("hashchange"));
}

function healthCheck(
  id: string,
  status: HealthCheck["status"],
  message: string,
  remediation: string | null = null,
): HealthCheck {
  return { id, status, message, remediation };
}

// TypeScript equivalents of T-010's four `core/env_probe.rs` fixture
// profiles (Blackwell, Ada, driver-too-old, no-GPU — PROGRESS.md /
// docs/TASKS.md T-011), at the `EnvironmentReport` level this screen
// actually consumes. `disk_space` and `endpoint_bindable` are
// orthogonal to the GPU profile in the Rust source too (their own
// `disk_status`/`endpoint_bindable` unit tests, not one of the four
// named profiles), so a Warn and a second Fail are folded into the
// driver-too-old and no-GPU fixtures respectively rather than adding a
// fifth invented profile — this is enough to exercise all three
// `CheckStatus` values without inventing a scenario the backend
// doesn't produce.

const blackwellReport: EnvironmentReport = {
  gpus: [
    {
      index: 0,
      name: "NVIDIA GeForce RTX 5090",
      compute_capability: [12, 0],
      vram_total_bytes: 32n * 1024n * 1024n * 1024n,
      vram_free_bytes: 30n * 1024n * 1024n * 1024n,
      driver_version: "581.29",
      cuda_version: "13.0",
    },
  ],
  system_ram_bytes: 64n * 1024n * 1024n * 1024n,
  free_disk_bytes: 500n * 1024n * 1024n * 1024n,
  os_build: "Windows 11 24H2 (26100.1000)",
  checks: [
    healthCheck("gpu_present", "Pass", "1 NVIDIA GPU(s) detected."),
    healthCheck("driver_ok", "Pass", "NVIDIA driver 581.29 detected."),
    healthCheck("cuda13_ok", "Pass", "NVIDIA driver supports CUDA 13."),
    healthCheck("disk_space", "Pass", "500 GiB free."),
    healthCheck("endpoint_bindable", "Pass", "Port 8080 is free."),
  ],
};

const driverTooOldReport: EnvironmentReport = {
  gpus: [
    {
      index: 0,
      name: "NVIDIA GeForce RTX 4090",
      compute_capability: [8, 9],
      vram_total_bytes: 24n * 1024n * 1024n * 1024n,
      vram_free_bytes: 22n * 1024n * 1024n * 1024n,
      driver_version: "470.10",
      cuda_version: null,
    },
  ],
  system_ram_bytes: 64n * 1024n * 1024n * 1024n,
  free_disk_bytes: 8n * 1024n * 1024n * 1024n,
  os_build: "Windows 11 24H2 (26100.1000)",
  checks: [
    healthCheck("gpu_present", "Pass", "1 NVIDIA GPU(s) detected."),
    healthCheck(
      "driver_ok",
      "Fail",
      "NVIDIA driver 470.10 is too old (need 525+).",
      "Update your NVIDIA driver from nvidia.com/drivers.",
    ),
    healthCheck(
      "cuda13_ok",
      "Fail",
      "NVIDIA driver 470.10 does not support CUDA 13 (need 580+).",
      "Update your NVIDIA driver from nvidia.com/drivers.",
    ),
    healthCheck(
      "disk_space",
      "Warn",
      "Only 8 GiB free.",
      "Large models can be tens of gigabytes; consider freeing up space.",
    ),
    healthCheck("endpoint_bindable", "Pass", "Port 8080 is free."),
  ],
};

const noGpuReport: EnvironmentReport = {
  gpus: [],
  system_ram_bytes: 32n * 1024n * 1024n * 1024n,
  free_disk_bytes: 500n * 1024n * 1024n * 1024n,
  os_build: "Windows 11 24H2 (26100.1000)",
  checks: [
    healthCheck(
      "gpu_present",
      "Fail",
      "No NVIDIA GPU was detected.",
      "Install a supported NVIDIA GPU and driver, or continue on Vulkan/CPU (expect much lower performance, PLAN.md §2.2).",
    ),
    healthCheck(
      "driver_ok",
      "Fail",
      "No NVIDIA driver was detected.",
      "Install a supported NVIDIA GPU and driver, or continue on Vulkan/CPU (expect much lower performance, PLAN.md §2.2).",
    ),
    healthCheck(
      "cuda13_ok",
      "Fail",
      "No NVIDIA driver was detected; CUDA 13 support cannot be confirmed.",
      "Install a supported NVIDIA GPU and driver, or continue on Vulkan/CPU (expect much lower performance, PLAN.md §2.2).",
    ),
    healthCheck("disk_space", "Pass", "500 GiB free."),
    healthCheck(
      "endpoint_bindable",
      "Fail",
      "Port 8080 could not be bound: Address already in use (os error 98).",
      "Close whatever else on this machine is using port 8080, or choose a different listen port once settings are available.",
    ),
  ],
};

describe("RuntimeScreen — health checks (docs/TASKS.md T-011)", () => {
  it("shows the Loading component's required label while the first probe is in flight", async () => {
    let resolveProbe: (report: EnvironmentReport) => void = () => {};
    mockedProbeEnvironment.mockReturnValueOnce(
      new Promise<EnvironmentReport>((resolve) => {
        resolveProbe = resolve;
      }),
    );

    render(<App />);
    goToRuntime();

    expect(await screen.findByText(healthCopy.loadingLabel)).toBeTruthy();

    resolveProbe(blackwellReport);
    await waitFor(() => expect(screen.getByText("GPU detected")).toBeTruthy());
  });

  it("renders each CheckStatus distinctly and shows remediation only for Warn and Fail", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(driverTooOldReport);

    const { container } = render(<App />);
    goToRuntime();

    await screen.findByText("Driver version");

    const passDot = container.querySelector('li[data-status="Pass"] span[aria-hidden="true"]');
    const warnDot = container.querySelector('li[data-status="Warn"] span[aria-hidden="true"]');
    const failDots = container.querySelectorAll('li[data-status="Fail"] span[aria-hidden="true"]');

    // Three different statuses, three different visual treatments —
    // proves "each CheckStatus renders distinctly" structurally, not
    // just by differing text.
    expect(passDot?.className).toContain("bg-pass");
    expect(warnDot?.className).toContain("bg-warn");
    expect(failDots.length).toBe(2); // driver_ok and cuda13_ok
    for (const dot of failDots) {
      expect(dot.className).toContain("bg-destructive");
    }

    // Fail remediation shown.
    expect(
      screen.getAllByText((_, node) => node?.textContent === `${healthCopy.remediationLabel} Update your NVIDIA driver from nvidia.com/drivers.`)
        .length,
    ).toBeGreaterThan(0);

    // Warn remediation shown.
    expect(
      screen.getByText(
        (_, node) =>
          node?.textContent ===
          `${healthCopy.remediationLabel} Large models can be tens of gigabytes; consider freeing up space.`,
      ),
    ).toBeTruthy();

    // Pass rows carry no remediation text at all.
    const passRow = container.querySelector('li[data-status="Pass"]');
    expect(passRow?.textContent).not.toContain(healthCopy.remediationLabel);
  });

  it("shows remediation for every failing check in the no-GPU profile without crashing on an empty gpu list", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(noGpuReport);

    const { container } = render(<App />);
    goToRuntime();

    await screen.findByText("GPU detected");

    const failRows = container.querySelectorAll('li[data-status="Fail"]');
    expect(failRows.length).toBe(4); // gpu_present, driver_ok, cuda13_ok, endpoint_bindable
    for (const row of failRows) {
      expect(row.textContent).toContain(healthCopy.remediationLabel);
    }

    const passRow = container.querySelector('li[data-status="Pass"]');
    expect(passRow?.textContent).not.toContain(healthCopy.remediationLabel);
  });

  it("refresh triggers a fresh probe and replaces the rendered report", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(blackwellReport);

    render(<App />);
    goToRuntime();

    await screen.findByText("1 NVIDIA GPU(s) detected.");
    expect(mockedProbeEnvironment).toHaveBeenCalledTimes(1);

    mockedProbeEnvironment.mockResolvedValueOnce(noGpuReport);
    fireEvent.click(screen.getByRole("button", { name: healthCopy.refresh }));

    await waitFor(() => expect(mockedProbeEnvironment).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("No NVIDIA GPU was detected.")).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// Runtime screen layout (9 Sept 2026 owner decision): both sections are
// ALWAYS shown — not mutually exclusive views. The version-management
// section sits first (its empty state is the first-run call to action);
// the environment health section sits below it. The four tests above prove
// the health section's content; these prove the layout itself.
// ---------------------------------------------------------------------------

const versionsCopy = strings.screens.runtime.versions;

function buildFixture(overrides: Partial<RuntimeBuild> = {}): RuntimeBuild {
  return {
    build_tag: "b10867",
    backend: { kind: "Cpu" },
    install_path: "C:/Users/Alieno/AppData/Local/LlamaManager/runtimes/b10867-cpu",
    is_active: false,
    installed_at: "2026-09-08T21:45:00Z",
    verified_flags: [],
    health_endpoint: "/props",
    registration_channel: "PresetDeclaresPath",
    ...overrides,
  };
}

describe("Runtime screen layout (health always visible, 9 Sept 2026)", () => {
  it("shows the environment health section even when a build is installed", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(blackwellReport);
    mockedListRuntimes.mockResolvedValueOnce([buildFixture()]);

    render(<App />);
    goToRuntime();

    // Health section present (T-011 content) AND version-management section
    // present (T-024 heading) — both on the same screen, not mutually
    // exclusive views.
    await screen.findByText(healthCopy.heading);
    expect(screen.getByText("GPU detected")).toBeTruthy();
    expect(screen.getByText(versionsCopy.heading)).toBeTruthy();
    expect(screen.getByText("b10867")).toBeTruthy();
  });

  it("shows the version-management empty state when no build is installed", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(blackwellReport);

    render(<App />);
    goToRuntime();

    await screen.findByText(healthCopy.heading);
    expect(screen.getByText(versionsCopy.heading)).toBeTruthy();
    expect(screen.getByText(versionsCopy.empty)).toBeTruthy();
  });

  it("explains why the last remaining build cannot be removed", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(blackwellReport);
    mockedListRuntimes.mockResolvedValueOnce([buildFixture()]);

    render(<App />);
    goToRuntime();

    await screen.findByText(versionsCopy.heading);
    // The Remove button is disabled (it is the last build) and the reason is
    // displayed — T-024: "rejected ... with the reason displayed".
    const removeButton = screen.getByRole("button", {
      name: versionsCopy.removeButton,
    }) as HTMLButtonElement;
    expect(removeButton.disabled).toBe(true);
    expect(screen.getByText(versionsCopy.lastBuildReason)).toBeTruthy();
  });

  it("explains that an Undetermined registration channel blocks model loading", async () => {
    mockedProbeEnvironment.mockResolvedValueOnce(blackwellReport);
    mockedListRuntimes.mockResolvedValueOnce([
      buildFixture({ registration_channel: "Undetermined" }),
    ]);

    render(<App />);
    goToRuntime();

    await screen.findByText(versionsCopy.heading);
    // The jargon-free flag and its explanation, not the internal T-025 note.
    expect(screen.getByText(versionsCopy.undeterminedFlag)).toBeTruthy();
    expect(screen.getByText(versionsCopy.undeterminedDescription)).toBeTruthy();
  });
});
