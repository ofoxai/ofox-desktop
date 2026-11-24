import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

export function assertControlResult(result) {
  assert.equal(result.error, undefined, "PowerShell control subprocess failed");
  assert.equal(result.status, 0, "PowerShell control preflight failed");
  const output = result.stdout ?? "";
  assert.equal([...output.matchAll(/^TAP version 13\r?$/gm)].length, 1);
  assert.equal([...output.matchAll(/^1\.\.4\r?$/gm)].length, 1);
  for (const [field, expected] of Object.entries({
    tests: 4,
    pass: 4,
    fail: 0,
    cancelled: 0,
    skipped: 0,
    todo: 0,
  })) {
    const matches = [
      ...output.matchAll(new RegExp(`^# ${field} (\\d+)\\r?$`, "gm")),
    ];
    assert.equal(
      matches.length,
      1,
      "expected one complete control test result",
    );
    assert.equal(
      Number(matches[0][1]),
      expected,
      "all four control checks must pass without skips",
    );
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  const result = spawnSync(
    process.execPath,
    [
      "--test",
      "--test-reporter=tap",
      fileURLToPath(
        new URL(
          "../assistant-runtime/test/document-control-reader.test.mjs",
          import.meta.url,
        ),
      ),
    ],
    {
      env: { ...process.env, OFOX_DOCUMENT_CONTROL_REQUIRED: "1" },
      encoding: "utf8",
      timeout: 30000,
      maxBuffer: 1024 * 1024,
    },
  );
  if (result.stdout) process.stdout.write(result.stdout);
  try {
    assertControlResult(result);
  } catch {
    process.stderr.write(
      "PowerShell control preflight failed or did not execute all four checks.\n",
    );
    process.exitCode = 1;
  }
}
