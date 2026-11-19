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
  const cleanup = "OFOX_CHILD_CALL_CLEANUP_V1 ";
  const failure = "OFOX_CHILD_CALL_FAILURE_V2 ";
  for (const [rows, exit, verdict, incompleteStage, detail] of [
    [[marker + "access-denied"], 0, "access-denied", undefined],
    [
      ["OFOX_CHILD_CALL_CONTROLLER_V1 access-denied"],
      101,
      "inconclusive",
      "native-test",
      { controllerVerdict: "access-denied", pairedReturn: "access-denied" },
    ],
    [
      ["OFOX_CHILD_CALL_CONTROLLER_V1 not-reached"],
      101,
      "inconclusive",
      "native-test",
      { controllerVerdict: "not-reached" },
    ],
    [
      [
        "OFOX_CHILD_CALL_CONTROLLER_V1 access-denied",
        marker + "child-process-blocked",
      ],
      0,
      "inconclusive",
      "native-test",
    ],
    [
      ["OFOX_CHILD_CALL_CONTROLLER_V1 access-denied", marker + "access-denied"],
      0,
      "access-denied",
      undefined,
    ],
    [
      [
        "OFOX_CHILD_CALL_CONTROLLER_V1 access-denied",
        "OFOX_CHILD_CALL_CONTROLLER_V1 access-denied",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [failure + "exit child-process-blocked true true true sharing-violation"],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerIncompleteStage: "exit",
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: false,
        },
        pairedReturn: "child-process-blocked",
        scratchRemovalError: "sharing-violation",
      },
    ],
    [
      [failure + "exit access-denied true true true removed"],
      101,
      "inconclusive",
      "exit",
      {
        controllerIncompleteStage: "exit",
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: true,
        },
        pairedReturn: "access-denied",
      },
    ],
    [
      [failure + "symbols none true true true access-denied"],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerIncompleteStage: "symbols",
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: false,
        },
        scratchRemovalError: "access-denied",
      },
    ],
    [
      [failure + "return-conflict none true true true access-denied"],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerIncompleteStage: "return-conflict",
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: false,
        },
        scratchRemovalError: "access-denied",
      },
    ],
    [
      [
        failure +
          "return-conflict child-process-blocked true true true removed",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [failure + "return-observer-error none true true true removed"],
      101,
      "inconclusive",
      "return-observer-error",
      {
        controllerIncompleteStage: "return-observer-error",
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: true,
        },
      },
    ],
    [
      [
        failure +
          "return-error/before-exit-marker/syntax none true true true access-denied",
      ],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerIncompleteStage: "return-observer-error",
        observerError: { phase: "before-exit-marker", kind: "syntax" },
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: false,
        },
        scratchRemovalError: "access-denied",
      },
    ],
    [
      [
        failure +
          "return-error/after-exit-marker/command-file none true true true removed",
      ],
      101,
      "inconclusive",
      "return-observer-error",
      {
        controllerIncompleteStage: "return-observer-error",
        observerError: { phase: "after-exit-marker", kind: "command-file" },
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: true,
        },
      },
    ],
    [
      [
        failure +
          "return-error/after-exit-marker/breakpoint child-process-blocked true true true removed",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [
        failure +
          "return-error/before-exit-marker/private-path none true true true removed",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [failure + "exit private-path true true true removed"],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [failure + "exit not-reached true true true removed"],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [failure + "symbols access-denied true true true removed"],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [
        failure +
          "child-process-blocked access-denied true true true sharing-violation",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [
        failure +
          "child-process-blocked child-process-blocked true true true removed",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [failure + "exit none true true true private-path"],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [
        failure + "exit access-denied true true true removed",
        marker + "access-denied",
      ],
      0,
      "inconclusive",
      "native-test",
    ],
    [
      [
        failure + "exit access-denied true true true removed",
        cleanup + "exit true false true true",
      ],
      101,
      "inconclusive",
      "native-test",
    ],
    [
      [cleanup + "child-process-blocked true true true false"],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerVerdict: "child-process-blocked",
        cleanup: {
          jobTerminated: true,
          controllerReaped: true,
          jobEmpty: true,
          scratchRemoved: false,
        },
      },
    ],
    [
      [cleanup + "last-error-zero false true false true"],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerIncompleteStage: "last-error-zero",
        cleanup: {
          jobTerminated: false,
          controllerReaped: true,
          jobEmpty: false,
          scratchRemoved: true,
        },
      },
    ],
    [
      [cleanup + "symbols true false true true"],
      101,
      "inconclusive",
      "cleanup",
      {
        controllerIncompleteStage: "symbols",
        cleanup: {
          jobTerminated: true,
          controllerReaped: false,
          jobEmpty: true,
          scratchRemoved: true,
        },
      },
    ],
    [["OFOX_CHILD_CALL_INCOMPLETE_V1 cleanup"], 101, "inconclusive", "cleanup"],
    [
      [cleanup + "private-path true true true false"],
      101,
      "inconclusive",
      "cleanup",
    ],
    [[cleanup + "symbols true true true true"], 101, "inconclusive", "cleanup"],
    [
      [cleanup + "symbols true true true private-path"],
      101,
      "inconclusive",
      "cleanup",
    ],
    [
      [
        cleanup + "symbols true true true false",
        cleanup + "symbols true true true false",
      ],
      101,
      "inconclusive",
      "cleanup",
    ],
    [
      [cleanup + "symbols true true true false", marker + "access-denied"],
      0,
      "inconclusive",
      "cleanup",
    ],
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
    [["OFOX_CHILD_CALL_INCOMPLETE_V1 symbols"], 101, "inconclusive", "symbols"],
    [
      ["OFOX_CHILD_CALL_INCOMPLETE_V1 last-error-zero"],
      101,
      "inconclusive",
      "last-error-zero",
    ],
    [
      ["OFOX_CHILD_CALL_INCOMPLETE_V1 last-error-unreadable"],
      101,
      "inconclusive",
      "last-error-unreadable",
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
      ...detail,
    };
    const result = await runChildCallDiagnostic((check) =>
      invokeNative(check, dir, ci, (_cargo, args, options) => {
        assert.ok(
          args.includes("--exact") && args.includes(childCallCheck.filter),
        );
        const counts =
          exit === 0 ? "ok. 1 passed; 0 failed" : "FAILED. 0 passed; 1 failed";
        const out = `running 1 test\ntest result: ${counts}; 0 ignored; 0 measured;\n`;
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

test("cleanup details require the complete output of exactly one failed native test", async () => {
  const failed =
    "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured;";
  const fixture = {
    exitCode: 101,
    outputLimited: false,
    output: "running 1 test\r\n" + failed + "\r\n",
    stderr: "OFOX_CHILD_CALL_CLEANUP_V1 symbols true true true false\r\n",
  };
  const valid = await runChildCallDiagnostic(async () => fixture);
  assert.equal(valid.controllerIncompleteStage, "symbols");
  for (const change of [
    { outputLimited: true },
    { exitCode: null },
    { childCall: "access-denied" },
    { incompleteStage: "observation" },
    { output: "running 1 test\n" },
    { output: fixture.output + failed },
    { output: "running 2 tests\n" + failed },
  ]) {
    const report = await runChildCallDiagnostic(async () => ({
      ...fixture,
      ...change,
    }));
    assert.deepEqual(report, {
      scope: "windows-fixed-child-call-diagnostic",
      releaseGate: false,
      diagnosticRecorded: false,
      verdict: "inconclusive",
      incompleteStage: "cleanup",
    });
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
