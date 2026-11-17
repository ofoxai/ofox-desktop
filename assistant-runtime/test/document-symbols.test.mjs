import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { safeMetadata } from "../../scripts/document-engine-development-check.mjs";

const pwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ??
  (process.platform === "win32" ? "pwsh" : null);
test(
  "symbol preparation requires exact successful totals and closes every owned process on failure",
  { skip: !pwsh },
  () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "ofox-symbol-check-"));
    try {
      for (const [mode, expected] of [
        ["success", null],
        ["details", null],
        ["pseudo", "symbol_check_incomplete"],
        ["foreign", "symbol_check_incomplete"],
        ["prefixed", "symbol_check_incomplete"],
        ["doublepass", "symbol_check_incomplete"],
        ["nonzero", "symbol_check_failed"],
        ["empty", "symbol_check_incomplete"],
        ["duplicate", "symbol_check_incomplete"],
        ["ignored", "symbol_check_incomplete"],
        ["stderr", "symbol_check_incomplete"],
        ["cancel", "symbol_cancelled"],
        ["livecancel", "symbol_cancelled"],
        ["deadline", "symbol_deadline"],
        ["bound", "symbol_output_bound"],
        ["stderrbound", "symbol_output_bound"],
        ["totalbound", "symbol_output_bound"],
      ]) {
        const result = spawnSync(
          pwsh,
          [
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-File",
            fileURLToPath(
              new URL("./fixtures/document-symbols.ps1", import.meta.url),
            ),
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
        assert.equal(report.state.closed, true, mode);
        assert.equal(report.state.started, mode !== "cancel", mode);
        assert.ok(!result.stdout.includes("private-token"), mode);
        assert.equal(
          report.state.passedFile,
          ["success", "details"].includes(mode),
          mode,
        );
        assert.ok(report.state.outputShape.stdout.capturedBytes >= 0, mode);
        assert.ok(report.state.outputShape.stderr.capturedBytes >= 0, mode);
        assert.deepEqual(report.arguments, [
          "/op",
          "/pt",
          "private image",
          "/s",
          "SRV*private cache*https://msdl.microsoft.com/download/symbols",
        ]);
      }
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);

test("symbol metadata projects fixed bounded fields without exposing tool output", () => {
  const report = safeMetadata({
    symbolPreparation: {
      started: true,
      closed: true,
      passedFile: true,
      exitCode: 0,
      failedFiles: 0,
      passedOrIgnoredFiles: 1,
      path: "private-token",
      output: "private-token",
    },
  });
  assert.deepEqual(report.symbolPreparation, {
    outputShape: { stdout: null, stderr: null },
    started: true,
    closed: true,
    passedFile: true,
    exitCode: 0,
    failedFiles: 0,
    passedOrIgnoredFiles: 1,
  });
  assert.ok(!JSON.stringify(report).includes("private-token"));
  const bad = safeMetadata({
    symbolPreparation: {
      started: "private-token",
      exitCode: Infinity,
      failedFiles: -0x80000001,
    },
  });
  for (const name of ["started", "exitCode", "failedFiles"])
    assert.equal(bad.symbolPreparation[name], null);
});

test(
  "native symbol output shape preserves only counts and fixed vocabulary",
  { skip: !pwsh },
  () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "ofox-symbol-shape-"));
    try {
      const result = spawnSync(
        pwsh,
        [
          "-NoLogo",
          "-NoProfile",
          "-NonInteractive",
          "-File",
          fileURLToPath(
            new URL("./fixtures/document-symbols.ps1", import.meta.url),
          ),
          "-Mode",
          "shape",
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
      for (const secret of [
        "private-account",
        "private-token",
        "12345",
        "S-1-5",
      ])
        assert.ok(!result.stdout.includes(secret));
      const report = JSON.parse(result.stdout);
      assert.equal(report.plain.individualPassLines, 1);
      assert.equal(report.plain.failedFiles, 0);
      assert.equal(report.plain.passedOrIgnoredFiles, 1);
      assert.equal(report.plain.bom, "none");
      assert.equal(report.wide.bom, "utf16le");
      assert.ok(report.wide.nulOddBytes > 0);
      assert.equal(report.wide.individualPassLines, 0);
      const safe = safeMetadata({
        symbolPreparation: {
          outputShape: { stdout: report.plain, stderr: report.wide },
        },
      });
      assert.deepEqual(safe.symbolPreparation.outputShape.stdout, report.plain);
      assert.deepEqual(safe.symbolPreparation.outputShape.stderr, report.wide);
      const bad = safeMetadata({
        symbolPreparation: {
          outputShape: {
            stdout: {
              capturedBytes: -1,
              bom: "private-token",
              tokenRows: [["private-token", "PASSED"]],
            },
          },
        },
      });
      assert.equal(
        bad.symbolPreparation.outputShape.stdout.capturedBytes,
        null,
      );
      assert.equal(bad.symbolPreparation.outputShape.stdout.bom, null);
      assert.deepEqual(bad.symbolPreparation.outputShape.stdout.tokenRows, [
        ["?", "PASSED"],
      ]);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);
