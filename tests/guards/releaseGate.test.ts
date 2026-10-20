import { readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { describe, expect, it } from "vitest";

describe("release verification", () => {
  it.skipIf(process.platform === "win32")(
    "preserves existing macOS keychain paths containing spaces",
    () => {
      const workflow = readFileSync(".github/workflows/release.yml", "utf8");
      const fragment = workflow
        .split(
          "# 加进搜索列表而非替换默认 keychain，降低对 runner 环境的侵入\n",
        )[1]
        ?.split("          security default-keychain")[0];
      expect(fragment).toBeDefined();
      const output = execFileSync(
        "bash",
        [
          "-c",
          `
      set -euo pipefail
      security() {
        if [[ "$1" == "list-keychains" && "\${4:-}" != "-s" ]]; then
          printf '    "%s"\\n' '/tmp/login keychain-db' '/tmp/other.keychain-db'
        else
          printf '%s\\0' "$@"
        fi
      }
      KEYCHAIN_PATH='/tmp/build keychain-db'
      ${fragment}
    `,
        ],
        { encoding: "utf8" },
      );
      expect(output.split("\0").filter(Boolean)).toEqual([
        "list-keychains",
        "-d",
        "user",
        "-s",
        "/tmp/build keychain-db",
        "/tmp/login keychain-db",
        "/tmp/other.keychain-db",
      ]);
    },
  );

  it("finishes notarization in dry-run and publishes only the verified bytes", () => {
    const workflow = readFileSync(".github/workflows/release.yml", "utf8");
    const verifyJob = workflow
      .split("\n  verify-artifacts:\n")[1]
      ?.split("\n  publish:\n")[0];
    expect(
      verifyJob,
      "dry-run needs an unconditional artifact verification job",
    ).toBeDefined();
    expect(verifyJob).not.toContain("inputs.dry_run");
    expect(verifyJob).toContain("node scripts/release-artifacts.mjs verify");
    expect(verifyJob).toContain("verified-release-${{ github.sha }}");
    const publishJob = workflow.split("\n  publish:\n")[1];
    expect(publishJob).toContain("needs: [build, verify-artifacts]");
    expect(publishJob).toContain("name: verified-release-${{ github.sha }}");
    expect(publishJob).toContain("node scripts/release-artifacts.mjs check");
    expect(publishJob).not.toContain("pattern: build-*");
    expect(publishJob).not.toContain("notarytool");
    expect(publishJob).not.toContain("stapler");
  });

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
