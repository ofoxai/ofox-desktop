import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { assertControlResult } from "../../scripts/check-document-control.mjs";

test("control gate requires one complete four-pass result with no skipped checks", () => {
  const output =
    "TAP version 13\n1..4\n# tests 4\n# pass 4\n# fail 0\n# cancelled 0\n# skipped 0\n# todo 0\n";
  assert.doesNotThrow(() => assertControlResult({ status: 0, stdout: output }));
  for (const bad of [
    { status: 1, stdout: output },
    {
      status: 0,
      stdout: output
        .replace("# pass 4", "# pass 1")
        .replace("# skipped 0", "# skipped 3"),
    },
    {
      status: 0,
      stdout: output
        .replace("# tests 4", "# tests 0")
        .replace("# pass 4", "# pass 0"),
    },
    { status: 0, stdout: output.repeat(2) },
    { status: 0, stdout: output.replace("# cancelled 0\n", "") },
    {
      status: 0,
      stdout: output,
      error: new Error("bounded subprocess failure"),
    },
  ])
    assert.throws(() => assertControlResult(bad));
});

test("required control preflight cannot pass by skipping PowerShell", () => {
  const root = mkdtempSync(path.join(os.tmpdir(), "ofox-control-gate-"));
  try {
    const environment = {
      ...process.env,
      HOME: root,
      USERPROFILE: root,
      OFOX_DOCUMENT_TEST_PWSH: "",
      OFOX_DOCUMENT_CONTROL_REQUIRED: "1",
    };
    // Exercise a standalone preflight, not Node's nested-runner suppression.
    delete environment.NODE_TEST_CONTEXT;
    const result = spawnSync(
      process.execPath,
      [
        fileURLToPath(
          new URL("../../scripts/check-document-control.mjs", import.meta.url),
        ),
      ],
      {
        env: environment,
        timeout: 5000,
        stdio: "ignore",
      },
    );
    assert.equal(result.error, undefined);
    assert.equal(result.status, 1, "zero real control checks must fail");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
