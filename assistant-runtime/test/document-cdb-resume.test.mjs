import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
const pwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ??
  (process.platform === "win32" ? "pwsh" : null);
test(
  "GO confirms an actual Resume after immediate debugger exit; continue still refuses abort",
  { skip: !pwsh },
  () => {
    for (const [mode, expected] of [
      ["late-go", null],
      ["go-abort", null],
      ["continue-abort", "cancelled"],
      ["cancel-go", "cancelled"],
      ["deadline", "deadline"],
      ["closed-continue", "debugger_exited"],
    ]) {
      const root = mkdtempSync(path.join(os.tmpdir(), "ofox-cdb-resume-"));
      try {
        const result = spawnSync(
          pwsh,
          [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-File",
            fileURLToPath(
              new URL("./fixtures/document-cdb-resume.ps1", import.meta.url),
            ),
            "-Directory",
            root,
            "-Mode",
            mode,
          ],
          {
            env: {
              ...process.env,
              HOME: root,
              USERPROFILE: root,
              APPDATA: root,
              LOCALAPPDATA: root,
              POWERSHELL_TELEMETRY_OPTOUT: "1",
            },
            encoding: "utf8",
            timeout: 15000,
            maxBuffer: 16384,
          },
        );
        assert.equal(result.error, undefined, mode);
        assert.equal(result.status, 0, mode);
        const report = JSON.parse(result.stdout);
        assert.equal(report.failure, expected, mode);
        if (mode === "late-go" || mode === "go-abort")
          assert.equal(report.debuggerExited, true);
        if (mode === "late-go") assert.ok(report.drains > 0);
      } finally {
        rmSync(root, { recursive: true, force: true });
      }
    }
  },
);

test(
  "continue and GO records require exact identities and integer counts",
  { skip: !pwsh },
  () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "ofox-cdb-record-"));
    try {
      const result = spawnSync(
        pwsh,
        [
          "-NoLogo",
          "-NoProfile",
          "-NonInteractive",
          "-File",
          fileURLToPath(
            new URL("./fixtures/document-cdb-resume.ps1", import.meta.url),
          ),
          "-Directory",
          root,
          "-Mode",
          "validate",
        ],
        {
          env: {
            ...process.env,
            HOME: root,
            USERPROFILE: root,
            APPDATA: root,
            LOCALAPPDATA: root,
            POWERSHELL_TELEMETRY_OPTOUT: "1",
          },
          encoding: "utf8",
          timeout: 10000,
          maxBuffer: 16384,
        },
      );
      assert.equal(result.status, 0);
      const rows = JSON.parse(result.stdout);
      assert.equal(rows.length, 11);
      for (const row of rows)
        assert.equal(
          row.failure,
          row.valid
            ? null
            : row.kind === "go"
              ? "host_resume_mismatch"
              : "continue_mismatch",
        );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);
