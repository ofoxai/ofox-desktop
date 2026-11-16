import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import os from "node:os";
import path from "node:path";
import {
  assertRustResult,
  captureFunctional,
  checks,
  extractTraceTargets,
  nativeOutput,
  safeMetadata,
  shouldCaptureDevelopmentTrace,
  startNativeTrace,
  runDevelopmentChecks,
} from "../../scripts/document-engine-development-check.mjs";
import { successfulProbe } from "../../scripts/check-windows-probe.mjs";

test("document development checks have a separate manual workflow", () => {
  assert.ok(
    existsSync(
      new URL(
        "../../.github/workflows/document-engine-check.yml",
        import.meta.url,
      ),
    ),
    "standalone manual development workflow is missing",
  );
});

const nativeResult = (total, passed = total, ignored = 0) => ({
  exitCode: 0,
  output: `running ${total} tests\n\ntest result: ok. ${passed} passed; 0 failed; ${ignored} ignored; 0 measured; 3000 filtered out; finished in 0.01s\n`,
});

test("native result evidence rejects zero, ignored, incomplete and ambiguous executions", () => {
  assert.equal(assertRustResult(nativeResult(1), true).passed, 1);
  assert.equal(assertRustResult(nativeResult(7), false).passed, 7);
  for (const [result, exact] of [
    [nativeResult(0), false],
    [nativeResult(2), true],
    [nativeResult(1, 0, 1), false],
    [{ ...nativeResult(1), exitCode: 101 }, true],
    [{ ...nativeResult(1), outputLimited: true }, true],
    [{ ...nativeResult(1), output: nativeResult(1).output.repeat(2) }, true],
    [{ exitCode: 0, output: "Finished compiling; no test execution" }, true],
  ])
    assert.throws(() => assertRustResult(result, exact));
});

test("ACL failure starts no later test and cannot be replaced by thirteen repeated prerequisite failures", async () => {
  const calls = [];
  const report = await runDevelopmentChecks(async (check) => {
    calls.push(check.name);
    return {
      exitCode: 101,
      output:
        "running 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;",
    };
  });
  assert.deepEqual(calls, ["acl-prerequisite"]);
  assert.equal(report.success, false);
  assert.equal(report.steps.length, 1);
  assert.equal(report.steps[0].exitCode, 101);
  assert.equal(report.releaseGate, false);
  assert.equal(report.hostPipelineE2e.status, "not-run");
});

test("all focused checks run serially after ACL and count real tests", async () => {
  let active = false;
  const calls = [];
  const report = await runDevelopmentChecks(async (check) => {
    assert.equal(active, false);
    active = true;
    calls.push(check.name);
    await new Promise((resolve) => setTimeout(resolve, 1));
    active = false;
    return nativeResult(check.exact ? 1 : 7);
  });
  assert.equal(report.success, true);
  assert.deepEqual(
    calls,
    checks.map((check) => check.name),
  );
  assert.equal(calls.length, 9);
  assert.match(
    checks[0].filter,
    /::document_cache_requires_exact_specific_file_read_execute_acl$/,
  );
  assert.equal(checks[1].name, "functional-prerequisite");
  assert.equal(checks[1].exact, true);
  assert.match(
    checks[1].filter,
    /::document_lpac_proves_python_office_and_escape_denials$/,
  );
  assert.ok(
    checks.every(
      (check) =>
        !check.filter.includes("local_interruption_tests") &&
        !check.filter.includes("one_guarded_task"),
    ),
  );
});

test("functional prerequisite failure starts no whole-sandbox or follow-up cases", async () => {
  const calls = [];
  const report = await runDevelopmentChecks(async (check) => {
    calls.push(check.name);
    return check.name === "acl-prerequisite"
      ? nativeResult(1)
      : {
          exitCode: 101,
          output:
            "running 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;",
        };
  });
  assert.deepEqual(calls, ["acl-prerequisite", "functional-prerequisite"]);
  assert.equal(report.steps.length, 2);
  assert.equal(report.steps[0].status, "passed");
  assert.equal(report.steps[1].status, "failed");
  assert.equal(report.success, false);
  assert.equal(report.releaseGate, false);
});

test("failed ETW start prevents the functional process from running without trace", async () => {
  const calls = [];
  let captureCalls = 0;
  const report = await runDevelopmentChecks(
    async (check) => {
      calls.push(check.name);
      return nativeResult(1);
    },
    {
      functionalCapture: async () => {
        captureCalls += 1;
        throw new Error("fixed synthetic start failure");
      },
    },
  );
  assert.equal(captureCalls, 1);
  assert.deepEqual(calls, ["acl-prerequisite"]);
  assert.equal(report.success, false);
  assert.equal(report.steps.at(-1).name, "functional-prerequisite");
});

test("capture stops after native failure and retains the original failed result", async () => {
  const order = [];
  const native = {
    exitCode: 101,
    output: "fixed native failure",
    outputLimited: false,
  };
  const result = await captureFunctional(
    async () => {
      order.push("native");
      return native;
    },
    async () => {
      order.push("start");
      return {
        finish: async (output) => {
          order.push("stop");
          assert.equal(output, native.output);
          return { status: "inconclusive" };
        },
      };
    },
  );
  assert.deepEqual(order, ["start", "native", "stop"]);
  assert.equal(result.exitCode, 101);
  assert.equal(result.output, native.output);
  assert.equal(result.trace.status, "inconclusive");
  const decodeFailure = await captureFunctional(
    async () => native,
    async () => ({
      finish: async () => {
        throw new Error("C:\\private-account S-1-5-21-private");
      },
    }),
  );
  assert.equal(decodeFailure.exitCode, 101);
  assert.equal(decodeFailure.trace.status, "inconclusive");
  assert.ok(!JSON.stringify(decodeFailure.trace).includes("private"));
});

test("capture start failure executes no native test and preserves only safe diagnostics", async () => {
  let invoked = false;
  const result = await captureFunctional(
    async () => {
      invoked = true;
    },
    async () => {
      throw new Error("C:\\private-account token-secret");
    },
  );
  assert.equal(invoked, false);
  assert.equal(result.exitCode, null);
  assert.equal(result.trace.reason, "capture-start-failed");
  assert.ok(!JSON.stringify(result).includes("private-account"));
});

test("trace target discovery includes panic stderr without changing stdout count evidence", async () => {
  const panic = `error:probe ${JSON.stringify({ worker: { evidence: { childPid: 66, primaryTid: 99 } } })}`;
  const result = await captureFunctional(
    async () => ({
      ...nativeResult(1),
      diagnosticOutput: `${nativeResult(1).output}\n${panic}`,
    }),
    async () => ({
      finish: async (text) => ({ targets: extractTraceTargets(text) }),
    }),
  );
  assert.equal(assertRustResult(result, true).passed, 1);
  assert.deepEqual(result.trace.targets, [{ childPid: 66, primaryTid: 99 }]);
});

test("ETW target identity is taken only from bounded fixed probe numeric evidence", () => {
  const failure = `assistant_document_sandbox_unavailable:probe ${JSON.stringify({ worker: { evidence: { childPid: 44, primaryTid: 88 } } })}`;
  assert.deepEqual(extractTraceTargets(`panic ${JSON.stringify(failure)}`), [
    { childPid: 44, primaryTid: 88 },
  ]);
  assert.deepEqual(extractTraceTargets(failure.repeat(2)), [
    { childPid: 44, primaryTid: 88 },
  ]);
  assert.deepEqual(
    extractTraceTargets("childPid=44 primaryTid=88 C:\\private-account"),
    [],
  );
  assert.deepEqual(
    extractTraceTargets(
      'error:probe {"worker":{"evidence":{"childPid":-1,"primaryTid":88}}}',
    ),
    [],
  );
});

test("interleaved pipe delivery never splices stdout into a split panic JSON", () => {
  const panic = `panic ${JSON.stringify(`error:probe ${JSON.stringify({ worker: { evidence: { childPid: 321, primaryTid: 654 } } })}`)}`;
  const stdout = [],
    stderr = [];
  const middle = Math.floor(panic.length / 2);
  stderr.push(Buffer.from(panic.slice(0, middle)));
  stdout.push(Buffer.from(nativeResult(1).output));
  stderr.push(Buffer.from(panic.slice(middle)));
  const result = nativeOutput(stdout, stderr);
  assert.equal(assertRustResult({ ...result, exitCode: 0 }, true).passed, 1);
  assert.deepEqual(extractTraceTargets(result.diagnosticOutput), [
    { childPid: 321, primaryTid: 654 },
  ]);
});

test("missing inventory metadata stays unknown and arbitrary strings are omitted", () => {
  assert.equal(safeMetadata(null), null);
  const metadata = safeMetadata({
    userInteractive: "C:\\private-account",
    windowsDeviceAliasAvailable: "S-1-5-21-private",
    tools: {
      xperf: {
        version: "token-secret",
        available: "unknown",
        signatureValid: "private",
      },
      wpr: { available: false },
    },
  });
  assert.equal(metadata.userInteractive, null);
  assert.equal(metadata.windowsDeviceAliasAvailable, null);
  assert.equal(metadata.tools.xperf.available, null);
  assert.equal(metadata.tools.xperf.signatureValid, null);
  assert.equal(metadata.tools.cdb.available, null);
  assert.equal(metadata.tools.wpr.available, false);
  for (const text of ["private", "token-secret", "C:\\"])
    assert.ok(!JSON.stringify(metadata).includes(text));
});

function fakeController() {
  const child = new EventEmitter();
  child.stdin = new PassThrough();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.killed = false;
  child.send = (message) => child.stdout.write(`${JSON.stringify(message)}\n`);
  child.close = () => {
    child.stdout.end();
    child.stderr.end();
    child.emit("close", 0);
  };
  child.kill = () => {
    child.killed = true;
    child.close();
    return true;
  };
  return child;
}

test("late started and failed messages are drained through owned-stop cleanup after startup timeout", async () => {
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "document-etw-protocol-"),
  );
  const child = fakeController();
  let syntheticSessionActive = false;
  child.stdin.on("finish", () => {
    setTimeout(() => {
      syntheticSessionActive = true;
      child.send({ state: "started" });
    }, 5);
    setTimeout(
      () => child.send({ state: "failed", reason: "controller_failure" }),
      10,
    );
    setTimeout(() => {
      syntheticSessionActive = false;
      child.send({
        state: "cleaned",
        ownedStopSucceeded: true,
        rawRemoved: true,
      });
      child.close();
    }, 25);
  });
  try {
    const result = await startNativeTrace(
      { RUNNER_TEMP: temporary },
      {
        spawnController: () => child,
        startupTimeoutMs: 5,
        cleanupTimeoutMs: 100,
      },
    );
    assert.equal(child.killed, false);
    assert.equal(syntheticSessionActive, false);
    assert.equal(result.failed.cleanup.ownedStopSucceeded, true);
    assert.equal(result.failed.cleanup.rawRemoved, true);
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("decode failure waits for the controller finally cleanup instead of killing on failed", async () => {
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "document-etw-protocol-"),
  );
  const child = fakeController();
  queueMicrotask(() => child.send({ state: "started" }));
  child.stdin.on("data", (data) => {
    if (!data.toString().includes("STOP")) return;
    setTimeout(
      () => child.send({ state: "failed", reason: "trace_decode_failed" }),
      5,
    );
    setTimeout(() => {
      child.send({
        state: "cleaned",
        ownedStopSucceeded: true,
        rawRemoved: true,
      });
      child.close();
    }, 20);
  });
  try {
    const capture = await startNativeTrace(
      { RUNNER_TEMP: temporary },
      {
        spawnController: () => child,
        startupTimeoutMs: 100,
        cleanupTimeoutMs: 100,
      },
    );
    const report = await capture.finish("fixed native failure");
    assert.equal(child.killed, false);
    assert.equal(report.cleanup.ownedStopSucceeded, true);
    assert.equal(report.status, "inconclusive");
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("local development keeps its original gate without ETW prerequisites", async () => {
  assert.equal(shouldCaptureDevelopmentTrace({}), false);
  assert.equal(
    shouldCaptureDevelopmentTrace({
      GITHUB_ACTIONS: "true",
      GITHUB_WORKFLOW: "Windows Probe",
    }),
    false,
  );
  assert.equal(
    shouldCaptureDevelopmentTrace({
      GITHUB_ACTIONS: "true",
      GITHUB_WORKFLOW: "Document Engine Development Checks",
    }),
    true,
  );
  const report = await runDevelopmentChecks(async (check) =>
    nativeResult(check.exact ? 1 : 7),
  );
  assert.equal(report.success, true);
  assert.deepEqual(report.startupTrace, { requested: false, attempted: false });
});

test("only dispatch or the exact development workflow bootstrap can trigger this diagnostic job", async () => {
  const workflow = await readFile(
    new URL(
      "../../.github/workflows/document-engine-check.yml",
      import.meta.url,
    ),
    "utf8",
  );
  const triggers = workflow
    .match(/^on:\n([\s\S]*?)\npermissions:/m)[1]
    .split("\n")
    .filter((line) => line.trim() && !line.trim().startsWith("#"))
    .join("\n");
  assert.equal(
    triggers,
    '  workflow_dispatch:\n  push:\n    branches: ["codex/desktop-next"]\n    paths: [".github/workflows/document-engine-check.yml"]',
  );
  assert.match(workflow, /^name: Document Engine Development Checks$/m);
  assert.match(
    workflow,
    /debug0-v3-.*hashFiles\('src-tauri\/Cargo.lock', 'src-tauri\/Cargo.toml'/,
  );
  assert.match(workflow, /windows-cargo-debug0-v2-/);
  assert.match(workflow, /always\(\).*cache-hit != 'true'/);
  const sha = "a".repeat(40);
  assert.equal(
    successfulProbe(
      [
        {
          name: "Windows Probe",
          event: "workflow_dispatch",
          head_sha: sha,
          path: ".github/workflows/document-engine-check.yml",
          head_repository: { full_name: "fixture/repository" },
          status: "completed",
          conclusion: "success",
        },
      ],
      sha,
      "fixture/repository",
    ),
    undefined,
  );
});
