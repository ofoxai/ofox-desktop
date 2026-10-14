import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

describe("release verification", () => {
  it("requires reusable CI and a separate exact-SHA Windows gate before builds, including dry runs", () => {
    const workflow = readFileSync(".github/workflows/release.yml", "utf8");
    expect(workflow).toMatch(
      /verify:\s*\n\s+uses: \.\/\.github\/workflows\/ci\.yml/,
    );
    expect(workflow).toMatch(/windows-probe:\s*\n/);
    expect(workflow).toMatch(/needs: \[verify, windows-probe\]/);
    expect(workflow).toContain("EXPECTED_SHA: ${{ github.sha }}");
    expect(workflow).toContain("EXPECTED_REPOSITORY: ${{ github.repository }}");
    expect(workflow).toContain("actions: read");
    expect(workflow).toContain("node scripts/check-windows-probe.mjs");
    expect(workflow).toContain(
      "if: github.event_name == 'push' || inputs.dry_run == false",
    );
  });

  it("rejects other commits, unfinished/failed runs, other workflows and fork runs", async () => {
    const { successfulProbe } = await import(
      "../../scripts/check-windows-probe.mjs"
    );
    const sha = "a".repeat(40);
    const run = {
      event: "push",
      head_sha: sha,
      status: "completed",
      conclusion: "success",
      path: ".github/workflows/windows-probe.yml",
      head_repository: { full_name: "ofox/desktop" },
      html_url: "https://github.com/ofox/desktop/actions/runs/1",
    };
    expect(successfulProbe([run], sha, "ofox/desktop")).toEqual(run);
    for (const change of [
      { event: "pull_request" },
      { event: "pull_request_target" },
      { head_sha: "b".repeat(40) },
      { status: "in_progress" },
      { conclusion: "failure" },
      { path: ".github/workflows/ci.yml" },
      { head_repository: { full_name: "fork/desktop" } },
    ]) {
      expect(
        successfulProbe([{ ...run, ...change }], sha, "ofox/desktop"),
      ).toBeUndefined();
    }
    expect(successfulProbe([], sha, "ofox/desktop")).toBeUndefined();
    expect(
      successfulProbe(
        [{ ...run, event: "workflow_dispatch" }],
        sha,
        "ofox/desktop",
      ),
    ).toEqual({ ...run, event: "workflow_dispatch" });
  });
});
