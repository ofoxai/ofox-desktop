import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import os from "node:os";
import path from "node:path";
import {
  assertRustResult,
  captureTrace,
  checks,
  extractTraceTargets,
  invokeNative,
  nativeOutput,
  safeMetadata,
  safeDebuggerState,
  shouldCaptureDevelopmentTrace,
  startupComparisonRequested,
  comparisonCheck,
  traceCheck,
  checkEnvironment,
  startNativeTrace,
  runDevelopmentChecks,
} from "../../scripts/document-engine-development-check.mjs";
import {
  developmentReceipt,
  functionalProbeFailure,
  nativePanicLocation,
} from "../../scripts/document-development-log.mjs";
import { successfulProbe } from "../../scripts/check-windows-probe.mjs";
import { projectDocumentLoaderSnaps } from "../../scripts/document-loader-snaps.mjs";

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
  assert.equal(calls.length, 10);
  assert.match(
    checks[0].filter,
    /::document_cache_requires_exact_specific_file_read_execute_acl$/,
  );
  // Registry scope evidence is gathered before any engine is resumed.
  assert.equal(checks[1].name, "registry-scope-prerequisite");
  assert.equal(checks[1].exact, true);
  assert.match(
    checks[1].filter,
    /::document_registry_scope_is_measured_and_adds_no_write_rights$/,
  );
  assert.equal(checks[2].name, "functional-prerequisite");
  assert.equal(checks[2].exact, true);
  assert.match(
    checks[2].filter,
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

test("registry scope failure starts no engine and keeps its evidence step", async () => {
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
  assert.deepEqual(calls, ["acl-prerequisite", "registry-scope-prerequisite"]);
  assert.equal(report.steps.at(-1).name, "registry-scope-prerequisite");
  assert.equal(report.steps.at(-1).status, "failed");
  assert.equal(report.success, false);
});

test("an explicitly requested launch comparison runs right after ACL, never by default", async () => {
  const run = async (options) => {
    const calls = [];
    const report = await runDevelopmentChecks(async (check) => {
      calls.push(check.name);
      return nativeResult(check.exact ? 1 : 7);
    }, options);
    return { calls, report };
  };
  const plain = await run({});
  assert.ok(!plain.calls.includes("launch-comparison"));
  const requested = await run({ comparison: true });
  assert.deepEqual(requested.calls.slice(0, 3), [
    "acl-prerequisite",
    "launch-comparison",
    "registry-scope-prerequisite",
  ]);
  assert.equal(requested.calls.length, checks.length + 1);
  assert.equal(comparisonCheck.exact, true);
  assert.match(
    comparisonCheck.filter,
    /::document_startup_launch_comparison_when_requested$/,
  );
  assert.equal(startupComparisonRequested({}), false);
  assert.equal(
    startupComparisonRequested({ OFOX_DOCUMENT_STARTUP_COMPARISON: "" }),
    false,
  );
  assert.equal(
    startupComparisonRequested({
      OFOX_DOCUMENT_STARTUP_COMPARISON: "detachedWinsockGrant",
    }),
    true,
  );
});

test("only the comparison step ever receives the comparison candidate", () => {
  const shared = {
    PATH: "fixture",
    OFOX_DOCUMENT_STARTUP_COMPARISON: "detachedWinsockGrant",
  };
  for (const check of checks) {
    const environment = checkEnvironment(shared, check);
    assert.equal(
      environment.OFOX_DOCUMENT_STARTUP_COMPARISON,
      undefined,
      check.name,
    );
    assert.equal(environment.PATH, "fixture");
  }
  assert.equal(
    checkEnvironment(shared, comparisonCheck).OFOX_DOCUMENT_STARTUP_COMPARISON,
    "detachedWinsockGrant",
  );
  assert.equal(shared.OFOX_DOCUMENT_STARTUP_COMPARISON, "detachedWinsockGrant");
});

test("only the functional prerequisite receives the child block event opt-in", () => {
  const shared = { OFOX_DOCUMENT_CHILD_BLOCK_EVENT: "1" };
  for (const check of [...checks, comparisonCheck]) {
    assert.equal(
      checkEnvironment(shared, check).OFOX_DOCUMENT_CHILD_BLOCK_EVENT,
      check.name === "functional-prerequisite" ? "1" : undefined,
      check.name,
    );
  }
});

test("functional prerequisite failure starts no whole-sandbox or follow-up cases", async () => {
  const calls = [];
  const report = await runDevelopmentChecks(async (check) => {
    calls.push(check.name);
    return check.name !== "functional-prerequisite"
      ? nativeResult(1)
      : {
          exitCode: 101,
          output:
            "running 1 test\ntest result: FAILED. 0 passed; 1 failed; 0 ignored;",
        };
  });
  assert.deepEqual(calls, [
    "acl-prerequisite",
    "registry-scope-prerequisite",
    "functional-prerequisite",
  ]);
  assert.equal(report.steps.length, 3);
  assert.equal(report.steps[0].status, "passed");
  assert.equal(report.steps[1].status, "passed");
  assert.equal(report.steps[2].status, "failed");
  assert.equal(report.success, false);
  assert.equal(report.releaseGate, false);
});

test("failed trace start runs no traced step and no later check", async () => {
  const calls = [];
  let captureCalls = 0;
  const report = await runDevelopmentChecks(
    async (check) => {
      calls.push(check.name);
      return nativeResult(1);
    },
    {
      traceCapture: async () => {
        captureCalls += 1;
        throw new Error("fixed synthetic start failure");
      },
    },
  );
  assert.equal(captureCalls, 1);
  assert.deepEqual(calls, ["acl-prerequisite"]);
  assert.equal(report.success, false);
  assert.equal(report.steps.at(-1).name, "startup-trace");
  assert.equal(report.startupTrace.attempted, true);
});

test("a requested trace runs as its own exact step right after ACL and any comparison", async () => {
  const run = async (options) => {
    const calls = [];
    const report = await runDevelopmentChecks(async (check) => {
      calls.push(check.name);
      return nativeResult(check.exact ? 1 : 7);
    }, options);
    return { calls, report };
  };
  const capture = (invoke) => invoke();
  assert.ok(!(await run({})).calls.includes("startup-trace"));
  const traced = await run({ traceCapture: capture });
  assert.deepEqual(traced.calls.slice(0, 3), [
    "acl-prerequisite",
    "startup-trace",
    "registry-scope-prerequisite",
  ]);
  assert.equal(traced.calls.length, checks.length + 1);
  const both = await run({ traceCapture: capture, comparison: true });
  assert.deepEqual(both.calls.slice(0, 4), [
    "acl-prerequisite",
    "launch-comparison",
    "startup-trace",
    "registry-scope-prerequisite",
  ]);
  assert.equal(traceCheck.exact, true);
  assert.match(traceCheck.filter, /::document_startup_trace_when_requested$/);
  assert.ok(checks.every((check) => check.name !== traceCheck.name));
});

test("capture stops after native failure and retains the original failed result", async () => {
  const order = [];
  const native = {
    exitCode: 101,
    output: "fixed native failure",
    outputLimited: false,
  };
  const result = await captureTrace(
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
  const decodeFailure = await captureTrace(
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
  const result = await captureTrace(
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
  const result = await captureTrace(
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

test("failed debugger retains private evidence through STOP and removes it only after CLEAN", async () => {
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "document-cdb-evidence-"),
  );
  const child = fakeController();
  let raw,
    sawStop = false,
    cleanedAfterStop = false;
  const spawnController = (_exe, args) => {
    raw = args[args.indexOf("-RawDirectory") + 1];
    queueMicrotask(() => child.send({ state: "started" }));
    child.stdin.on("data", async (chunk) => {
      const command = chunk.toString();
      if (command.includes("STOP")) {
        sawStop = true;
        await Promise.all([
          writeFile(
            path.join(raw, "debugger-state.json"),
            JSON.stringify({
              stage: "debugger_closed",
              debuggerStarted: true,
              debuggerExited: true,
              debuggerClosed: true,
              stdoutLines: 9,
            }),
          ),
          writeFile(
            path.join(raw, "roots.json"),
            JSON.stringify({ windows: ["C:\\Windows"] }),
          ),
          writeFile(
            path.join(raw, "debugger.log"),
            [
              "OFOX_CDB_V2 BEFORE pid=2a tid=2b flags=70 END",
              "OFOX_CDB_V2 AFTER pid=2a tid=2b flags=72 END",
              "OFOX_CDB_V2 READY END",
              "OFOX_CDB_V2 PRE_ATTACH_HOLD pid=2a tid=2b suspend=1 paired=2 END",
              "OFOX_CDB_V2 CONTINUE_REQUESTED END",
              "OFOX_CDB_V2 HOST_RESUMED pid=2a tid=2b hold=1 paired=2 previous=1 END",
              '2a:2b @ 100 - LdrpInitializeNode - ERROR: Init routine 00007FFF12345678 for DLL "C:\\Windows\\System32\\KERNELBASE.dll" failed during DLL_PROCESS_ATTACH',
              "C:\\private-account secret-token",
            ].join("\n"),
          ),
        ]);
        child.send({ state: "failed", reason: "debugger_exited" });
      }
      if (command.includes("CLEAN")) {
        cleanedAfterStop =
          sawStop && existsSync(path.join(raw, "debugger.log"));
        await rm(raw, { recursive: true, force: true });
        child.send({
          state: "cleaned",
          ownedStopSucceeded: true,
          rawRemoved: true,
        });
        child.close();
      }
    });
    return child;
  };
  try {
    const result = await captureTrace(
      async () => ({
        exitCode: 101,
        outputLimited: false,
        output: `error:probe ${JSON.stringify({ worker: { evidence: { childPid: 42, primaryTid: 43 } } })}`,
      }),
      () =>
        startNativeTrace(
          { RUNNER_TEMP: temporary },
          { spawnController, startupTimeoutMs: 100, cleanupTimeoutMs: 100 },
        ),
    );
    assert.equal(result.exitCode, 101);
    assert.equal(result.trace.controllerState.stage, "debugger_closed");
    assert.equal(
      result.trace.diagnostic.observations[0].module,
      "kernelbase.dll",
    );
    assert.equal(cleanedAfterStop, true);
    assert.equal(existsSync(raw), false);
    assert.equal(result.trace.cleanup.rawRemoved, true);
    assert.equal(child.killed, false);
    for (const secret of ["private-account", "secret-token", "12345678"])
      assert.ok(!JSON.stringify(result.trace).includes(secret));
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
});

test("an undecodable private log still keeps the fixed controller state and failure stage", async () => {
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "document-cdb-overflow-"),
  );
  const child = fakeController();
  const spawnController = (_exe, args) => {
    const raw = args[args.indexOf("-RawDirectory") + 1];
    queueMicrotask(() => child.send({ state: "started" }));
    child.stdin.on("data", async (chunk) => {
      const command = chunk.toString();
      if (command.includes("STOP")) {
        await Promise.all([
          writeFile(
            path.join(raw, "debugger-state.json"),
            JSON.stringify({
              stage: "output_bound",
              debuggerStarted: true,
              debuggerClosed: true,
              capturedBytes: 4128000,
              stdoutLines: 41000,
            }),
          ),
          writeFile(
            path.join(raw, "roots.json"),
            JSON.stringify({ windows: ["C:\\Windows"] }),
          ),
          // Run 44226276948: CRLF lines made the file larger than counted.
          writeFile(
            path.join(raw, "debugger.log"),
            "x".repeat(4 * 1024 * 1024 + 1),
          ),
        ]);
        child.send({ state: "failed", reason: "output_bound" });
      }
      if (command.includes("CLEAN")) {
        await rm(raw, { recursive: true, force: true });
        child.send({
          state: "cleaned",
          ownedStopSucceeded: true,
          rawRemoved: true,
        });
        child.close();
      }
    });
    return child;
  };
  try {
    const result = await captureTrace(
      async () => ({ exitCode: 0, outputLimited: false, output: "" }),
      () =>
        startNativeTrace(
          { RUNNER_TEMP: temporary },
          { spawnController, startupTimeoutMs: 100, cleanupTimeoutMs: 100 },
        ),
    );
    assert.equal(result.trace.status, "inconclusive");
    assert.equal(result.trace.reason, "trace-decode-incomplete");
    assert.equal(result.trace.controller.stage, "output_bound");
    assert.equal(result.trace.controllerState.stage, "output_bound");
    assert.equal(result.trace.controllerState.capturedBytes, 4128000);
    assert.equal(result.trace.controllerState.stdoutLines, 41000);
    assert.equal(result.trace.cleanup.rawRemoved, true);
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
  // The debugger diagnostic forces the functional gate to fail by design, so
  // it runs only on explicit opt-in inside the development workflow.
  const development = {
    GITHUB_ACTIONS: "true",
    GITHUB_WORKFLOW: "Document Engine Development Checks",
  };
  assert.equal(shouldCaptureDevelopmentTrace(development), false);
  for (const value of ["0", "true", " 1", ""])
    assert.equal(
      shouldCaptureDevelopmentTrace({
        ...development,
        OFOX_DOCUMENT_STARTUP_TRACE: value,
      }),
      false,
      value,
    );
  assert.equal(
    shouldCaptureDevelopmentTrace({
      ...development,
      OFOX_DOCUMENT_STARTUP_TRACE: "1",
    }),
    true,
  );
  assert.equal(
    shouldCaptureDevelopmentTrace({
      GITHUB_ACTIONS: "true",
      GITHUB_WORKFLOW: "Windows Probe",
      OFOX_DOCUMENT_STARTUP_TRACE: "1",
    }),
    false,
  );
  const report = await runDevelopmentChecks(async (check) =>
    nativeResult(check.exact ? 1 : 7),
  );
  assert.equal(report.success, true);
  assert.deepEqual(report.startupTrace, { requested: false, attempted: false });
});

test("only the dedicated trace step receives the private CDB diagnostic request", async () => {
  const received = [];
  const report = await runDevelopmentChecks(
    async (check, overrides) => {
      received.push([check.name, overrides]);
      return nativeResult(check.exact ? 1 : 7);
    },
    {
      traceCapture: (invoke) =>
        captureTrace(invoke, async () => ({
          environment: {
            OFOX_DOCUMENT_CDB_DIAGNOSTIC_ROOT: "synthetic-private-directory",
          },
          finish: async () => ({ diagnosticOnly: true }),
        })),
    },
  );
  assert.equal(report.success, true);
  assert.deepEqual(
    received.filter(([, env]) => env),
    [
      [
        "startup-trace",
        { OFOX_DOCUMENT_CDB_DIAGNOSTIC_ROOT: "synthetic-private-directory" },
      ],
    ],
  );
});

test("controller raw setup markers coexist with normalized loader proof without spoofing it", async () => {
  const source = await readFile(
    new URL("../../scripts/document-engine-cdb.ps1", import.meta.url),
    "utf8",
  );
  assert.match(source, /OFOX_CDB_RAW_V1 TARGET/);
  assert.ok(!source.includes("OFOX_CDB_V2 TARGET"));
  const raw = [
    "OFOX_CDB_RAW_V1 TARGET_BEGIN",
    "OFOX_CDB_RAW_V1 TARGET 2a 2b END",
    "OFOX_CDB_RAW_V1 BEFORE_BEGIN",
    "Current NtGlobalFlag contents: 0x00000070",
    "OFOX_CDB_RAW_V1 BEFORE_END",
    "OFOX_CDB_RAW_V1 AFTER_BEGIN",
    "Current NtGlobalFlag contents: 0x00000072",
    "OFOX_CDB_RAW_V1 AFTER_END",
    "OFOX_CDB_RAW_V1 CONFIG_END",
  ];
  const normalized = [
    "OFOX_CDB_V2 BEFORE pid=2a tid=2b flags=70 END",
    "OFOX_CDB_V2 AFTER pid=2a tid=2b flags=72 END",
    "OFOX_CDB_V2 READY END",
    "OFOX_CDB_V2 PRE_ATTACH_HOLD pid=2a tid=2b suspend=1 paired=2 END",
    "OFOX_CDB_V2 CONTINUE_REQUESTED END",
    "OFOX_CDB_V2 HOST_RESUMED pid=2a tid=2b hold=1 paired=2 previous=1 END",
  ];
  const result = projectDocumentLoaderSnaps({
    text: [
      ...raw,
      ...normalized,
      '2a:2b @ 100 - LdrpInitializeNode - ERROR: Init routine 00007FFF12345678 for DLL "C:\\Windows\\System32\\KERNELBASE.dll" failed during DLL_PROCESS_ATTACH',
    ].join("\n"),
    target: { childPid: 42, primaryTid: 43 },
    roots: { windows: "C:\\Windows" },
  });
  assert.equal(result.handshake.status, "matched");
  assert.equal(result.observations[0].module, "kernelbase.dll");
  const g = source.indexOf("WriteLine('g')");
  assert.ok(
    source.indexOf("Assert-DocumentCdbRecord $permission 'continue'") < g,
  );
  assert.ok(source.indexOf("Read-Json 'go.json'") > g);
  assert.equal(source.split("WriteLine('g')").length - 1, 1);
  assert.match(source.slice(g), /Assert-DocumentCdbRecord \$go 'go'/);
  const unknown = safeDebuggerState({
    stage: "C:\\private\\secret",
    debuggerExited: "maybe",
    capturedBytes: -1,
    private: "S-1-5-21-secret",
  });
  assert.equal(unknown.stage, "unknown");
  assert.equal(unknown.debuggerExited, null);
  assert.equal(unknown.capturedBytes, null);
  assert.ok(!JSON.stringify(unknown).includes("secret"));
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
  // The WSAStartup trace is answered (run 44231498872) and registryRead is
  // accepted; the gate now runs unassisted, with no debugger or comparison.
  assert.doesNotMatch(workflow, /OFOX_DOCUMENT_STARTUP_COMPARISON/);
  assert.doesNotMatch(workflow, /OFOX_DOCUMENT_STARTUP_TRACE/);
  assert.match(
    workflow,
    /debug0-v3-.*hashFiles\('src-tauri\/Cargo.lock', 'src-tauri\/Cargo.toml'/,
  );
  assert.match(workflow, /windows-cargo-debug0-v2-/);
  assert.match(workflow, /always\(\).*cache-hit != 'true'/);
  const uploadPaths = workflow
    .split("name: Upload development diagnostics")[1]
    .split("      - name:")[0]
    .match(/^          path: \|\n((?:            [^\n]+\n)+)/m)?.[1]
    .trim()
    .split(/\n\s*/);
  const artifactRoot =
    "${{ runner.temp }}/document-engine-development-${{ github.run_attempt }}/";
  assert.deepEqual(
    uploadPaths?.sort(),
    [
      "summary.json",
      ...[...checks, comparisonCheck, traceCheck].map(
        (check) => check.name + ".log",
      ),
    ]
      .map((name) => artifactRoot + name)
      .sort(),
  );
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

test("native step artifacts retain registry counts and discard raw Cargo output on success and failure", async (t) => {
  const scan = {
    visited: 3510,
    vanished: 1,
    hostUnopenable: 0,
    enumerationStopped: 0,
    checkErrors: 0,
    readableByRuntimeOnly: 4,
    readableByProduction: 1802,
    addedByRegistryRead: 1798,
    writableByRuntimeOnly: 0,
    writableByProduction: 0,
    writeAddedByRegistryRead: 0,
  };
  const receipt = {
    keys: [
      "machine_root",
      "winsock_catalog",
      "user_environment",
      "user_software",
    ],
    gates: [true, false, true, false],
    production: [true, true, true, true],
    runtimeOnly: [false, false, false, false],
    productionVerified: true,
    runtimeOnlyVerified: false,
    scan,
  };
  for (const exit of [0, 101]) {
    const dir = await mkdtemp(path.join(os.tmpdir(), "document-check-log-"));
    t.after(() => rm(dir, { recursive: true, force: true }));
    const marker = "OFOX_REGISTRY_ISOLATION_V1 ";
    const check = checks[1];
    const stderr = [
      "S-1-5-21-314159 PID=876543 private-stderr",
      "error[E0308]: private compile error",
      " --> src/services/assistant/documents/windows_sandbox/probe.rs:45:6",
      "thread 'private-test' (112233) panicked at src/services/assistant/host/local_interruption_tests.rs:11:4:",
      marker +
        JSON.stringify({
          ...receipt,
          extra: "private-name",
          scan: { ...scan, body: "private-value" },
        }),
    ].join("\r\n");
    const stdout =
      "C:\\private-path\\test private-stdout\n" + nativeResult(1).output;
    const run = await invokeNative(
      check,
      dir,
      process.env,
      (_cargo, _args, options) =>
        spawn(
          process.execPath,
          [
            "-e",
            "console.log(process.argv[1]); console.error(process.argv[2]); process.exitCode=Number(process.argv[3]);",
            stdout,
            stderr,
            String(exit),
          ],
          options,
        ),
    );
    assert.equal(run.exitCode, exit);
    const log = await readFile(path.join(dir, check.name + ".log"), "utf8");
    assert.doesNotMatch(
      log,
      /private-|S-1-|876543|running 1 test|test result:/,
    );
    const native = JSON.parse(
      log.split("\n")[0].slice("OFOX_NATIVE_RESULT_V1 ".length),
    );
    assert.deepEqual(native.compiler, [
      { code: "E0308", location: { source: "sandbox-probe", line: 45 } },
    ]);
    assert.deepEqual(native.panic, {
      source: "host-interruption-tests",
      line: 11,
    });
    const line = log.split("\n").find((row) => row.startsWith(marker));
    assert.deepEqual(JSON.parse(line.slice(marker.length)), receipt);
  }
});

test("required receipts reject missing, ambiguous and malformed evidence without forwarding fields", () => {
  const step = "registry-scope-prerequisite";
  const marker = "OFOX_REGISTRY_ISOLATION_V1 ";
  for (const [text, status] of [
    ["private data", "missing"],
    [marker + "private data\n", "invalid"],
    [marker + "{}\n" + marker + "{}\n", "invalid"],
    [marker + '{"keys":["S-1-private"]}\n', "invalid"],
  ]) {
    assert.deepEqual(developmentReceipt(step, text), { status });
    assert.throws(() =>
      assertRustResult({ ...nativeResult(1), receiptStatus: status }, true),
    );
  }
  assert.equal(developmentReceipt("acl-prerequisite", marker + "{}\n"), null);
});

test("functional failures retain engine checkpoints and registry decisions without identities or panic text", () => {
  const receipt = {
    stage: "node_isolation",
    reason: "process_exit",
    native: { code: "80070005", sourceLine: 1831 },
    engine: {
      status: -1,
      timedOut: false,
      stdoutBytes: 0,
      stderrBytes: 1,
      nodeCheckpointBeforeStep: 3,
      childSpawnResult: "EPERM",
    },
    worker: {
      firstFailure: "token_registry_isolation",
      evidence: {
        processStatus: 2147483651,
        registryIsolation: [
          {
            key: "machine_root",
            opened: true,
            descriptorRead: true,
            accessChecked: true,
            readGranted: true,
          },
        ],
      },
    },
  };
  const raw = structuredClone(receipt);
  Object.assign(raw.worker.evidence, {
    childPid: 1234567,
    primaryTid: 8765432,
    contents: "private",
  });
  const failureMessage = (detail) =>
    "native Python + OfficeCLI and all LPAC negative probes must pass: " +
    JSON.stringify(
      "assistant_document_sandbox_unavailable:probe " + JSON.stringify(detail),
    ) +
    "\r\n";
  const panic = failureMessage(raw);
  assert.deepEqual(functionalProbeFailure(panic), receipt);
  const acl = {
    actualControl: "8004",
    expectedControl: "9004",
    actualRevision: 1,
    expectedRevision: null,
  };
  const aclPanic = failureMessage({
    ...raw,
    reason: "cache_acl_mismatch",
    native: { ...acl, ignored: "private" },
  });
  assert.deepEqual(functionalProbeFailure(aclPanic).native, acl);
  assert.equal(
    functionalProbeFailure(panic.replace("node_isolation", "private-name")),
    null,
  );
  assert.deepEqual(
    nativePanicLocation(
      "thread 'private-test' (112233) panicked at C:\\private\\src\\services\\assistant\\documents\\windows_sandbox\\tests.rs:77:11:\r\n",
    ),
    { source: "sandbox-tests", line: 77 },
  );
});

test("a trace keeps the ACL restoration and Host resume evidence without private target identities", () => {
  const evidence = {
    diagnosticOnly: true,
    configConfirmed: true,
    beforeFlags: 112,
    afterFlags: 114,
    preAttachSuspendPrevious: 1,
    preAttachPairedResumePrevious: 2,
    releaseProbeCount: 1,
    releaseSuspendPrevious: 1,
    releasePairedResumePrevious: 2,
    suspendPrevious: 1,
    pairedResumePrevious: 2,
    hostResumePrevious: 1,
    debuggerClosed: true,
    launchProfile: "detached_runtime_only_winsock_grant",
    creationFlags: 524300,
    winsockGrantApplied: true,
    winsockGrantRestored: false,
  };
  const marker = "OFOX_STARTUP_TRACE_V1";
  const detail = {
    worker: {
      evidence: {
        childPid: 12345,
        primaryTid: 54321,
        loaderDiagnostic: { ...evidence, privateName: "private-name" },
      },
    },
  };
  const line =
    marker +
    " assistant_document_sandbox_unavailable:probe " +
    JSON.stringify(detail) +
    "\r\n";
  assert.deepEqual(developmentReceipt("startup-trace", line), {
    status: "recorded",
    marker,
    data: evidence,
  });
});

test("an inconclusive comparison keeps partial rows and maps unknown libuv calls into the fixed vocabulary", () => {
  const row = {
    extraRegistryRead: false,
    consoleMode: "detached",
    creationFlags: 524300,
    winsockGrant: false,
    grantRestored: null,
    completed: true,
    stage: "complete",
    tokenVerified: true,
    capabilityCount: 1,
    capabilityAttributes: [4],
    lpacAccessMatched: true,
    childPolicy: 1,
    imagePolicy: 3,
    integrityRid: 4096,
    resumePrevious: 1,
    exitStatus: 2147483651,
    timedOut: false,
    markerMatched: false,
    stdoutBytes: 0,
    stderrBytes: 48,
    stderrShape: { kind: "libuv-fatal", call: "WSAStartup", code: 10107 },
    cleanupConfirmed: true,
  };
  const receipt = {
    schemaVersion: 1,
    scope: "fixed-node-startup-launch-comparison",
    diagnosticOnly: true,
    releaseGate: false,
    runtimePolicyChanged: false,
    admissionGranted: false,
    candidate: "detachedWinsockGrant",
    winsockGrantApplied: false,
    winsockGrantFailureRestored: true,
    sameProfileAndInputs: false,
    runtimeStable: false,
    debuggerUsed: false,
    status: "inconclusive",
    taskCleanup: true,
    profileCleanup: true,
    rows: [row],
  };
  const marker = "OFOX_STARTUP_COMPARISON_V1";
  const project = () =>
    developmentReceipt(
      "launch-comparison",
      marker + " " + JSON.stringify(receipt) + "\n",
    );
  assert.deepEqual(project(), { status: "recorded", marker, data: receipt });
  row.stderrShape.call = "private-call";
  row.pid = 123456;
  const output = project().data;
  assert.equal(output.rows[0].stderrShape.call, "?");
  assert.ok(!Object.hasOwn(output.rows[0], "pid"));
});

test("an unknown child refusal in the CI functional gate records only a bounded kernel event verdict and still fails", async (t) => {
  const check = checks.find(({ name }) => name === "functional-prerequisite");
  const evidence = {
    childPid: 897123,
    primaryTid: 897456,
    childCreationFiletime: "134390000000000001",
    processStatus: 1,
    registryIsolation: [],
  };
  const failure = {
    stage: "node_isolation",
    reason: "process_exit",
    native: null,
    engine: {
      status: 1,
      timedOut: false,
      stdoutBytes: 0,
      stderrBytes: 100,
      nodeCheckpointBeforeStep: 7,
      childSpawnResult: "UNKNOWN",
    },
    worker: { firstFailure: null, evidence },
  };
  const environment = {
    ...process.env,
    GITHUB_ACTIONS: "true",
    GITHUB_WORKFLOW: "Document Engine Development Checks",
    OFOX_DOCUMENT_CHILD_BLOCK_EVENT: "1",
  };
  const cases = [
    { reply: "matched", expected: "matched" },
    { reply: "private-event-data", expected: "query-unavailable" },
    { optIn: "0" },
    { workflow: "Windows Probe" },
    { actions: "false" },
    { creationTime: "9223372036854775808", expected: "identity-unavailable" },
  ];
  for (const item of cases) {
    const dir = await mkdtemp(path.join(os.tmpdir(), "document-child-event-"));
    t.after(() => rm(dir, { recursive: true, force: true }));
    evidence.childCreationFiletime = item.creationTime ?? "134390000000000001";
    const stderr =
      "native Python + OfficeCLI and all LPAC negative probes must pass: " +
      JSON.stringify(
        "assistant_document_sandbox_unavailable:probe " +
          JSON.stringify(failure),
      );
    let queries = 0;
    const result = await invokeNative(
      check,
      dir,
      {
        ...environment,
        GITHUB_ACTIONS: item.actions ?? "true",
        GITHUB_WORKFLOW: item.workflow ?? environment.GITHUB_WORKFLOW,
        OFOX_DOCUMENT_CHILD_BLOCK_EVENT: item.optIn ?? "1",
      },
      (_cargo, _args, options) =>
        spawn(
          process.execPath,
          ["-e", "console.error(process.argv[1]);process.exitCode=101", stderr],
          options,
        ),
      (target) => {
        queries++;
        assert.deepEqual(target, {
          childPid: 897123,
          primaryTid: 897456,
          childCreationFiletime: evidence.childCreationFiletime,
        });
        return item.reply;
      },
    );
    assert.equal(queries, item.reply ? 1 : 0);
    assert.equal(result.exitCode, 101);
    const log = await readFile(path.join(dir, check.name + ".log"), "utf8");
    assert.doesNotMatch(
      log,
      /897123|897456|134390000000000001|9223372036854775808|childPid|primaryTid|childCreationFiletime|private-event-data/,
    );
    const row = JSON.parse(log.trim().slice("OFOX_NATIVE_RESULT_V1 ".length));
    assert.equal(row.childBlockEvent, item.expected);
  }
});
