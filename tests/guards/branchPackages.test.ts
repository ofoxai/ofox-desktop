import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";

const sha = "a".repeat(40);
const identity = {
  EXPECTED_SHA: sha,
  EXPECTED_REPOSITORY: "ofoxai/ofox-desktop",
};
const probe = {
  id: 10,
  event: "push",
  head_sha: sha,
  path: ".github/workflows/windows-probe.yml",
  head_repository: { full_name: identity.EXPECTED_REPOSITORY },
  status: "completed",
  conclusion: "success",
  html_url: "https://github.com/ofoxai/ofox-desktop/actions/runs/10",
};

describe("branch installer packages", () => {
  it("builds only branch artifacts after CI and an independent exact-SHA Windows probe", () => {
    const workflow = readFileSync(
      ".github/workflows/branch-packages.yml",
      "utf8",
    );
    const triggers = workflow.split("\npermissions:")[0];
    expect(triggers).toContain('branches: ["release/**"]');
    expect(triggers).toContain("workflow_dispatch:");
    expect(triggers).not.toMatch(/\n\s+(tags|pull_request|release):/);
    expect(workflow).toContain("uses: ./.github/workflows/ci.yml");
    expect(workflow).toContain("needs: [verify, windows-probe]");
    expect(workflow).toContain("node scripts/check-windows-probe.mjs --wait");
    expect(workflow).toContain("EXPECTED_SHA: ${{ github.sha }}");
    expect(workflow).toContain("EXPECTED_REPOSITORY: ${{ github.repository }}");
    expect(workflow).toContain("contents: read");
    expect(workflow).not.toMatch(
      /contents: write|secrets\.|environment:|gh release|aws s3|latest\.json/,
    );
    expect(workflow).toContain("platform: darwin-aarch64");
    expect(workflow).toContain("platform: windows-x86_64");
    expect(workflow).toContain('APPLE_SIGNING_IDENTITY: "-"');
    expect(workflow).toContain("pnpm tauri build --bundles app,dmg --");
    // Tauri can bundle auxiliary Cargo binaries found in the restored target.
    // Build all binary targets so cached probes cannot enter a new installer.
    expect(workflow).not.toContain("--bin ofox-desktop");
    expect(workflow).toContain(
      "node scripts/release-artifacts.mjs record-build",
    );
    expect(workflow).toContain("SHA256SUMS.txt");
    expect(workflow).toContain(
      "name: branch-packages-${{ matrix.platform }}-${{ github.sha }}",
    );
    expect(workflow).toContain("retention-days: 30");
    const windows = readFileSync(".github/workflows/windows-probe.yml", "utf8");
    expect(windows.split("\nconcurrency:")[0]).toContain('"release/**"');
    const ci = readFileSync(".github/workflows/ci.yml", "utf8");
    expect(ci).toContain("branches: [main]");
    expect(ci).not.toContain('branches: [main, "release/**"]');
  });

  it("waits for this commit's own completed Windows result", async () => {
    const { waitForWindowsProbe } = await import(
      "../../scripts/check-windows-probe.mjs"
    );
    const listRuns = vi
      .fn()
      .mockReturnValueOnce([])
      .mockReturnValueOnce([
        { ...probe, head_sha: "b".repeat(40) },
        { ...probe, head_repository: { full_name: "fork/desktop" } },
        { ...probe, event: "pull_request" },
      ])
      .mockReturnValueOnce([
        { ...probe, status: "in_progress", conclusion: null },
      ])
      .mockReturnValueOnce([probe]);
    const sleep = vi.fn();
    await expect(
      waitForWindowsProbe(identity, { listRuns, sleep, maxAttempts: 4 }),
    ).resolves.toEqual(probe);
    expect(listRuns).toHaveBeenCalledTimes(4);
    expect(sleep).toHaveBeenCalledTimes(3);
    expect(sleep).toHaveBeenCalledWith(30_000);
  });

  it("fails promptly on the newest matching failed probe", async () => {
    const { waitForWindowsProbe } = await import(
      "../../scripts/check-windows-probe.mjs"
    );
    const sleep = vi.fn();
    await expect(
      waitForWindowsProbe(identity, {
        listRuns: () => [{ ...probe, conclusion: "failure" }],
        sleep,
        maxAttempts: 2,
      }),
    ).rejects.toThrow("Windows Probe failed");
    expect(sleep).not.toHaveBeenCalled();
  });

  it("allows a newer matching retry to finish before deciding failure", async () => {
    const { waitForWindowsProbe } = await import(
      "../../scripts/check-windows-probe.mjs"
    );
    const listRuns = vi
      .fn()
      .mockReturnValueOnce([
        { ...probe, id: 9, conclusion: "failure" },
        { ...probe, id: 11, status: "in_progress", conclusion: null },
      ])
      .mockReturnValueOnce([{ ...probe, id: 11 }]);
    await expect(
      waitForWindowsProbe(identity, {
        listRuns,
        sleep: vi.fn(),
        maxAttempts: 2,
      }),
    ).resolves.toEqual({ ...probe, id: 11 });
  });

  it("bounds waiting and never accepts missing evidence or API errors", async () => {
    const { waitForWindowsProbe } = await import(
      "../../scripts/check-windows-probe.mjs"
    );
    const listRuns = vi.fn(() => []);
    const sleep = vi.fn();
    await expect(
      waitForWindowsProbe(identity, { listRuns, sleep, maxAttempts: 2 }),
    ).rejects.toThrow("Timed out waiting for Windows Probe");
    expect(listRuns).toHaveBeenCalledTimes(2);
    expect(sleep).toHaveBeenCalledTimes(1);
    await expect(
      waitForWindowsProbe(identity, {
        listRuns: () => {
          throw new Error("API unavailable");
        },
        sleep,
        maxAttempts: 2,
      }),
    ).rejects.toThrow("API unavailable");
  });
});
