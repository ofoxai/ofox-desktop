import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import {
  childCallCheck,
  childCallTraceRequested,
  runChildCallDiagnostic,
} from "../../scripts/document-engine-development-check.mjs";

const ci = {
  GITHUB_ACTIONS: "true",
  GITHUB_WORKFLOW: "Document Engine Child Call Diagnostic",
  GITHUB_REF: "refs/heads/codex/desktop-next",
  GITHUB_EVENT_NAME: "workflow_dispatch",
  OFOX_DOCUMENT_CHILD_CALL_TRACE: "1",
};
test("only the dedicated development run selects the fixed native observation", () => {
  assert.ok(childCallTraceRequested(ci));
  assert.ok(childCallTraceRequested({ ...ci, GITHUB_EVENT_NAME: "push" }));
  assert.equal(
    childCallTraceRequested({
      GITHUB_WORKFLOW: "Document Engine Development Checks",
    }),
    false,
  );
  assert.throws(
    () => childCallTraceRequested({ ...ci, GITHUB_EVENT_NAME: "pull_request" }),
    /invalid child-call/,
  );
  for (const value of ["", "0", "true", "1"]) {
    assert.throws(
      () =>
        childCallTraceRequested({
          GITHUB_WORKFLOW: "Document Engine Development Checks",
          OFOX_DOCUMENT_CHILD_CALL_TRACE: value,
        }),
      /invalid child-call/,
    );
  }
  for (const key of Object.keys(ci))
    assert.throws(
      () => childCallTraceRequested({ ...ci, [key]: "" }),
      /invalid child-call/,
    );
  assert.throws(
    () =>
      childCallTraceRequested({
        ...ci,
        OFOX_DOCUMENT_STARTUP_COMPARISON: "detachedWinsockGrant",
      }),
    /invalid child-call/,
  );
});
test("the separate debugger check needs one passing native case and a closed receipt", async () => {
  const output =
    "running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n";
  for (const [exitCode, verdict, expected] of [
    [0, "child-process-blocked", true],
    [0, "not-reached", true],
    [101, "child-process-blocked", false],
    [0, "C:\\private-name", false],
    [0, null, false],
  ]) {
    const report = await runChildCallDiagnostic(async (check) => {
      assert.deepEqual(check, childCallCheck);
      return { exitCode, output, outputLimited: false, childCall: verdict };
    });
    assert.equal(report.diagnosticRecorded, expected);
    assert.equal(report.releaseGate, false);
    assert.equal(report.verdict, expected ? verdict : "inconclusive");
    assert.doesNotMatch(JSON.stringify(report), /private-name|running 1 test/);
  }
});
const pwsh =
  process.env.OFOX_DOCUMENT_TEST_PWSH ||
  (process.platform === "win32" && "pwsh");
test(
  "the debugger parser accepts only an armed, completed and correctly paired observation",
  { skip: !pwsh },
  () => {
    const result = spawnSync(
      pwsh,
      [
        "-NoProfile",
        "-NonInteractive",
        "-File",
        fileURLToPath(
          new URL("./fixtures/document-child-call-trace.ps1", import.meta.url),
        ),
      ],
      { encoding: "utf8", timeout: 10000, maxBuffer: 8192 },
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stdout.trim(), "passed");
  },
);

test("only a closed failure stage and verdict survive the native artifact boundary", async (t) => {
  const { invokeNative } =
    await import("../../scripts/document-engine-development-check.mjs");
  const { mkdtemp, readFile, rm } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const { join } = await import("node:path");
  const { spawn } = await import("node:child_process");
  const marker = "OFOX_CHILD_CALL_V1 ";
  for (const [rows, exit, verdict, incompleteStage] of [
    [[marker + "access-denied"], 0, "access-denied", undefined],
    [
      [marker + "access-denied", marker + "access-denied"],
      0,
      "inconclusive",
      "native-test",
    ],
    [
      ["OFOX_CHILD_CALL_INCOMPLETE_V1 return-pair"],
      101,
      "inconclusive",
      "return-pair",
    ],
    [
      ["OFOX_CHILD_CALL_INCOMPLETE_V1 private-path"],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [marker + "other-win32", "OFOX_CHILD_CALL_INCOMPLETE_V1 arm"],
      0,
      "inconclusive",
      "arm",
    ],
  ]) {
    const dir = await mkdtemp(join(tmpdir(), "document-child-call-"));
    t.after(() => rm(dir, { recursive: true, force: true }));
    const expected = {
      scope: "windows-fixed-child-call-diagnostic",
      releaseGate: false,
      diagnosticRecorded: verdict !== "inconclusive",
      verdict,
      ...(incompleteStage && { incompleteStage }),
    };
    const result = await runChildCallDiagnostic((check) =>
      invokeNative(check, dir, ci, (_cargo, args, options) => {
        assert.ok(
          args.includes("--exact") && args.includes(childCallCheck.filter),
        );
        const out =
          "running 1 test\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured;\n";
        return spawn(
          process.execPath,
          [
            "-e",
            "console.log(process.argv[1]); console.error(process.argv[2]); process.exitCode=Number(process.argv[3]);",
            out,
            ["private-path SID PID", ...rows].join("\r\n"),
            String(exit),
          ],
          options,
        );
      }),
    );
    assert.deepEqual(result, expected);
    assert.deepEqual(
      await readFile(join(dir, "child-call-trace.log"), "utf8"),
      "OFOX_NATIVE_RESULT_V1 " + JSON.stringify(expected) + "\n",
    );
  }
});

test("the dedicated workflow bootstraps only its own file and uploads two fixed files", async () => {
  const { readFile } = await import("node:fs/promises");
  const yaml = await readFile(
    new URL("../../.github/workflows/document-child-call.yml", import.meta.url),
    "utf8",
  );
  assert.match(
    yaml,
    /^    paths: \["\.github\/workflows\/document-child-call\.yml"\]$/m,
  );
  assert.match(
    yaml,
    /^    if: github.ref == 'refs\/heads\/codex\/desktop-next'$/m,
  );
  assert.match(yaml, /^    runs-on: windows-2022$/m);
  const upload = yaml
    .split("uses: actions/upload-artifact@")[1]
    .split("retention-days:")[0]
    .split("path: |")[1];
  assert.deepEqual(
    upload
      .trim()
      .split(/\n\s*/)
      .map((s) => s.split("/").pop()),
    ["summary.json", "child-call-trace.log"],
  );
});
