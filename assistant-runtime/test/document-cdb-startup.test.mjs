import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { safeDebuggerState } from "../../scripts/document-engine-development-check.mjs";

const pwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ??
  (process.platform === "win32" ? "pwsh" : null);
test(
  "CDB startup options precede the final process target and preserve each private path argument",
  { skip: !pwsh },
  () => {
    const root = mkdtempSync(path.join(os.tmpdir(), "ofox-cdb-arguments-"));
    try {
      const result = spawnSync(
        pwsh,
        [
          "-NoLogo",
          "-NoProfile",
          "-NonInteractive",
          "-File",
          fileURLToPath(
            new URL("./fixtures/document-cdb-arguments.ps1", import.meta.url),
          ),
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
      assert.equal(result.error, undefined);
      assert.equal(result.status, 0);
      const report = JSON.parse(result.stdout);
      const args = report.arguments;
      assert.deepEqual(args.slice(-2), ["-p", "42"]);
      assert.ok(args.indexOf("-cf") < args.indexOf("-p"));
      assert.equal(args[args.indexOf("-xe") + 1], "ld:ntdll.dll");
      assert.equal(args[args.indexOf("-cf") + 1], "private commands");
      assert.equal(args[args.indexOf("-y") + 1], "private symbols");
      for (const unsafe of ["-g", "-pr", "-o"])
        assert.ok(!args.includes(unsafe));
      assert.deepEqual(report.shape, {
        promptLines: 2,
        markerMentionLines: 2,
        numericTargetLines: 1,
        nullCharacters: 1,
        targetSeen: false,
        reportedWin32Errors: [5, 87],
      });
      assert.deepEqual(report.configured, {
        targetSeen: true,
        pid: 42,
        tid: 43,
        before: 0x70,
        after: 0x72,
        tail: true,
        gflagValueLines: 3,
      });
      assert.ok(!result.stdout.includes("private-token"));
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);

test("startup output shape retains only bounded counts without granting handshake proof", () => {
  const fields = [
    "promptLines",
    "markerMentionLines",
    "numericTargetLines",
    "gflagValueLines",
    "nullCharacters",
  ];
  const source = Object.fromEntries(fields.map((field) => [field, 2]));
  const report = safeDebuggerState({
    ...source,
    path: "C:\\private-account",
    raw: "secret",
    configConfirmed: true,
  });
  for (const field of fields) assert.equal(report[field], 2);
  assert.equal(report.configConfirmed, undefined);
  assert.ok(!JSON.stringify(report).includes("private-account"));
  assert.ok(!JSON.stringify(report).includes("secret"));
  for (const bad of [-1, 0x100000000, "secret", null]) {
    const result = safeDebuggerState(
      Object.fromEntries(fields.map((field) => [field, bad])),
    );
    for (const field of fields) assert.equal(result[field], null);
  }
  assert.deepEqual(
    safeDebuggerState({
      reportedWin32Errors: [5, "secret", -1, 0x100000000, 87],
    }).reportedWin32Errors,
    [5, 87],
  );
  assert.equal(safeDebuggerState({}).reportedWin32Errors, null);
  assert.equal(
    safeDebuggerState({ reportedWin32Errors: Array(20).fill(5) })
      .reportedWin32Errors.length,
    8,
  );
});
