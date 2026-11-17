// Focused native diagnostics only; never release or installed-client evidence.
import assert from "node:assert/strict";
import { randomUUID } from "node:crypto";
import { spawn } from "node:child_process";
import { lstatSync } from "node:fs";
import {
  mkdir,
  mkdtemp,
  open,
  realpath,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createInterface } from "node:readline";
import { projectDocumentLoaderSnaps } from "./document-loader-snaps.mjs";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const sandbox = "services::assistant::documents::windows_sandbox::tests::";
const attachment = "services::assistant::attachments::tests::";
const host = "services::assistant::host::tests::";
export const checks = [
  {
    name: "acl-prerequisite",
    filter: `${sandbox}document_cache_requires_exact_specific_file_read_execute_acl`,
    exact: true,
  },
  {
    name: "functional-prerequisite",
    filter: `${sandbox}document_lpac_proves_python_office_and_escape_denials`,
    exact: true,
  },
  { name: "windows-sandbox", filter: sandbox, exact: false },
  {
    name: "attachment-old-roundtrip",
    filter: `${attachment}preview_page_number_seventy_persists_and_roundtrips_backup`,
    exact: true,
  },
  {
    name: "attachment-long-path",
    filter: `${attachment}document_preview_backup_roundtrips_long_paths_and_still_rejects_hardlinks`,
    exact: true,
  },
  {
    name: "attachment-cancel-roundtrip",
    filter: `${attachment}document_preview_rolls_back_cancelled_publication_and_survives_backup`,
    exact: true,
  },
  {
    name: "host-interruption",
    filter: `${host}interrupted_journaled_mutation_keeps_verification_debt_across_run_result`,
    exact: true,
  },
  {
    name: "host-no-effect",
    filter: `${host}a_known_no_effect_mutation_failure_does_not_create_host_debt`,
    exact: true,
  },
  {
    name: "host-unknown-outcome",
    filter: `${host}lifecycle_unknown_outcome_keeps_host_debt_after_a_real_effect`,
    exact: true,
  },
];

export function assertRustResult(result, exact) {
  assert.equal(result.exitCode, 0, "native Cargo test failed");
  assert.ok(
    !result.outputLimited,
    "native output exceeded the diagnostic bound",
  );
  const running = [...result.output.matchAll(/^running (\d+) tests?\s*$/gm)];
  const summaries = [
    ...result.output.matchAll(
      /^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; (\d+) measured;/gm,
    ),
  ];
  assert.equal(running.length, 1, "expected one lib test execution");
  assert.equal(
    summaries.length,
    1,
    "expected one successful native test summary",
  );
  const total = Number(running[0][1]);
  const [passed, failed, ignored, measured] = summaries[0].slice(1).map(Number);
  assert.ok(total > 0, "zero selected native tests cannot pass");
  if (exact) assert.equal(total, 1, "exact filter must execute one test");
  assert.equal(passed, total);
  assert.equal(failed + ignored + measured, 0);
  return { total, passed, failed, ignored, measured };
}

export async function runDevelopmentChecks(invoke, { functionalCapture } = {}) {
  const report = {
    schemaVersion: 1,
    scope: "document-engine-development-checks",
    releaseGate: false,
    paidModelCalls: 0,
    installedClientAcceptance: "pending",
    hostPipelineE2e: {
      status: "not-run",
      reason:
        "capability gate remains off; enable separately after native core evidence",
    },
    success: false,
    startupTrace: {
      requested: typeof functionalCapture === "function",
      attempted: false,
    },
    steps: [],
  };
  for (const check of checks) {
    const started = Date.now();
    const step = {
      name: check.name,
      filter: check.filter,
      log: `${check.name}.log`,
      status: "failed",
    };
    report.steps.push(step);
    try {
      if (check.name === "functional-prerequisite" && functionalCapture)
        report.startupTrace.attempted = true;
      const result =
        check.name === "functional-prerequisite" && functionalCapture
          ? await functionalCapture((overrides) => invoke(check, overrides))
          : await invoke(check);
      step.exitCode = result.exitCode;
      if (result.trace) step.trace = result.trace;
      step.counts = assertRustResult(result, check.exact);
      step.status = "passed";
    } catch {
      step.failure = "native-command-or-test-count-failed";
      return report; // In particular, an ACL prerequisite failure starts no core probes.
    } finally {
      step.durationMs = Date.now() - started;
    }
  }
  report.success = true;
  return report;
}

export function extractTraceTargets(output) {
  const candidates = [output];
  // Rust's panic Debug representation quotes/escapes a String error. Decode
  // only bounded string literals containing our fixed probe grammar.
  for (const match of output.matchAll(/"(?:[^"\\]|\\.)*"/g)) {
    if (candidates.length >= 16) break;
    if (match[0].length > 1048576 || !match[0].includes(":probe ")) continue;
    try {
      const value = JSON.parse(match[0]);
      if (typeof value === "string") candidates.push(value);
    } catch {}
  }
  const targets = new Map();
  for (const candidate of candidates) {
    for (
      let marker = candidate.indexOf(":probe ");
      marker >= 0;
      marker = candidate.indexOf(":probe ", marker + 7)
    ) {
      const tail = candidate.slice(marker + 7, marker + 7 + 1048576);
      if (!tail.startsWith("{")) continue;
      let depth = 0,
        quoted = false,
        escaped = false;
      for (let index = 0; index < tail.length; index++) {
        const character = tail[index];
        if (quoted) {
          if (escaped) escaped = false;
          else if (character === "\\") escaped = true;
          else if (character === '"') quoted = false;
        } else if (character === '"') quoted = true;
        else if (character === "{") depth++;
        else if (character === "}" && --depth === 0) {
          try {
            const evidence = JSON.parse(tail.slice(0, index + 1)).worker
              ?.evidence;
            const { childPid, primaryTid } = evidence ?? {};
            if (
              [childPid, primaryTid].every(
                (n) => Number.isInteger(n) && n > 0 && n <= 0xffffffff,
              )
            )
              targets.set(`${childPid}:${primaryTid}`, {
                childPid,
                primaryTid,
              });
          } catch {}
          break;
        }
      }
    }
  }
  return [...targets.values()].slice(0, 8);
}

async function privateRead(filename, limit) {
  const info = lstatSync(filename);
  assert.ok(
    info.isFile() &&
      !info.isSymbolicLink() &&
      info.nlink === 1 &&
      info.size <= limit,
    "private diagnostic input bound",
  );
  const bytes = await readFile(filename);
  assert.ok(bytes.length <= limit, "private diagnostic input bound");
  return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
}

export function safeMetadata(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const boolean = (value) => (typeof value === "boolean" ? value : null);
  const symbolShape = (source) => {
    if (!source || typeof source !== "object" || Array.isArray(source))
      return null;
    const result = {};
    for (const name of [
      "capturedBytes",
      "nulEvenBytes",
      "nulOddBytes",
      "crBytes",
      "lfBytes",
      "individualPassLines",
      "failedTotalLines",
      "passedTotalLines",
      "failedFiles",
      "passedOrIgnoredFiles",
    ]) {
      const number = source[name];
      result[name] =
        Number.isInteger(number) && number >= 0 && number <= 0xffffffff
          ? number
          : null;
    }
    result.bom = ["none", "utf8", "utf16le", "utf16be"].includes(source.bom)
      ? source.bom
      : null;
    result.usageMention = boolean(source.usageMention);
    const allowed = new Set([
      "SYMCHK",
      "SYMSRV",
      "DBGHELP",
      "PASSED",
      "FAILED",
      "IGNORED",
      "FILES",
      "NTDLL.DLL",
      "NTDLL.PDB",
      "USAGE",
      ":",
      "=",
      "+",
      "?",
    ]);
    result.tokenRows = Array.isArray(source.tokenRows)
      ? source.tokenRows
          .slice(0, 8)
          .filter(Array.isArray)
          .map((row) =>
            row.slice(0, 16).map((token) => (allowed.has(token) ? token : "?")),
          )
      : null;
    return result;
  };
  const tools = {};
  for (const name of ["xperf", "wpr", "cdb", "symchk", "gflags", "tracerpt"]) {
    const tool = value?.tools?.[name];
    tools[name] = { available: boolean(tool?.available) };
    if (tool && typeof tool === "object" && !Array.isArray(tool))
      Object.assign(tools[name], {
        version: /^\d+(?:\.\d+){3}$/.test(tool.version ?? "")
          ? tool.version
          : null,
        sha256: /^[a-f0-9]{64}$/.test(tool.sha256 ?? "") ? tool.sha256 : null,
        signatureValid: boolean(tool.signatureValid),
        microsoftSigned: boolean(tool.microsoftSigned),
      });
  }
  return {
    tools,
    userInteractive: boolean(value?.userInteractive),
    sessionId:
      Number.isInteger(value?.sessionId) && value.sessionId >= 0
        ? value.sessionId
        : null,
    windowsAliasCount:
      Number.isInteger(value?.windowsAliasCount) &&
      value.windowsAliasCount >= 0 &&
      value.windowsAliasCount <= 4
        ? value.windowsAliasCount
        : null,
    windowsDeviceAliasAvailable: boolean(value?.windowsDeviceAliasAvailable),
    windowsAliasStatus:
      Number.isInteger(value?.windowsAliasStatus) &&
      value.windowsAliasStatus >= 0 &&
      value.windowsAliasStatus <= 0xffffffff
        ? value.windowsAliasStatus
        : null,
    mode: "cdb-loader-snaps",
    debuggerJobKillOnClose: boolean(value?.debuggerJobKillOnClose),
    symbolPreparation: {
      outputShape: Object.fromEntries(
        ["stdout", "stderr"].map((name) => [
          name,
          symbolShape(value?.symbolPreparation?.outputShape?.[name]),
        ]),
      ),
      ...Object.fromEntries(
        ["started", "closed", "passedFile"].map((name) => [
          name,
          boolean(value?.symbolPreparation?.[name]),
        ]),
      ),
      ...Object.fromEntries(
        ["exitCode", "failedFiles", "passedOrIgnoredFiles"].map((name) => {
          const number = value?.symbolPreparation?.[name];
          return [
            name,
            Number.isInteger(number) &&
            number >= -0x80000000 &&
            number <= 0xffffffff
              ? number
              : null,
          ];
        }),
      ),
    },
    maximumFileMiB: 4,
    deadlineSeconds: 900,
    flags: ["cdb-pb", "cdb-ntdll-load-break", "process-local-sls"],
  };
}

export function nativeOutput(stdoutChunks, stderrChunks) {
  const output = Buffer.concat(stdoutChunks).toString("utf8");
  return {
    output,
    diagnosticOutput: `${Buffer.concat(stderrChunks).toString("utf8")}\n${output}`,
  };
}

function traceFailure(reason, metadata, cleanup = null) {
  return {
    scope: "windows-node-loader-diagnostic",
    releaseGate: false,
    status: "inconclusive",
    reason,
    metadata,
    cleanup,
  };
}

function safeControllerFailure(value) {
  const stages = new Set([
    "symbol_inventory",
    "symbol_prepare",
    "symbol_cancelled",
    "symbol_deadline",
    "symbol_check_failed",
    "symbol_check_incomplete",
    "symbol_output_bound",
    "symbol_cleanup_unconfirmed",
    "symbol_cache_invalid",
    "controller_failure",
    "tool_inventory_failed",
    "required_tool_unavailable",
    "native_controller_compile_failed",
    "capture_start_failed",
    "capture_deadline",
    "capture_stop_failed",
    "trace_decode_failed",
    "decode_ack_deadline",
    "start_trace",
    "query_trace",
    "owner_layout",
    "owner_mismatch",
    "stop_trace",
    "xml_bound",
    "xml_field_bound",
    "ndjson_bound",
    "xml_deadline",
    "target_not_seen",
    "target_mismatch",
    "gflag_before_unreadable",
    "gflag_after_unreadable",
    "gflag_changed_unexpectedly",
    "init_tail_missing",
    "debugger_exited",
    "deadline",
    "cancelled",
    "debugger_inventory",
    "debugger_attach_start",
    "debugger_handshake",
    "controller_job_setup",
    "native_controller_compile",
    "controller_setup",
    "debugger_unavailable",
    "debugger_untrusted",
    "output_bound",
    "host_resume_mismatch",
    "debugger_stop_unconfirmed",
    "debugger_drain_unconfirmed",
  ]);
  return {
    stage: stages.has(value?.reason) ? value.reason : "controller_failure",
    compilerCodes: Array.isArray(value?.compilerCodes)
      ? value.compilerCodes
          .filter((code) => typeof code === "string" && /^CS\d{4}$/.test(code))
          .slice(0, 8)
      : [],
    nativeCode:
      Number.isInteger(value?.nativeCode) &&
      value.nativeCode >= 0 &&
      value.nativeCode <= 0xffffffff
        ? value.nativeCode
        : null,
  };
}

export function safeDebuggerState(value) {
  const stages = new Set([
    "target_not_seen",
    "target_mismatch",
    "gflag_before_unreadable",
    "gflag_after_unreadable",
    "gflag_changed_unexpectedly",
    "init_tail_missing",
    "debugger_exited",
    "deadline",
    "cancelled",
    "debugger_closed",
    "debugger_inventory",
    "debugger_attach_start",
    "debugger_handshake",
    "controller_job_setup",
    "native_controller_compile",
    "controller_setup",
    "waiting_host_resume",
    "running_diagnostic",
    "output_bound",
    "host_resume_mismatch",
  ]);
  const result = {
    stage: stages.has(value?.stage) ? value.stage : "unknown",
    cleanupFailure:
      value?.cleanupFailure === "debugger_cleanup_unconfirmed"
        ? value.cleanupFailure
        : null,
  };
  for (const name of [
    "debuggerStarted",
    "debuggerExited",
    "debuggerClosed",
    "targetSeen",
    "beforeReadable",
    "afterReadable",
    "initTailSeen",
  ])
    result[name] = typeof value?.[name] === "boolean" ? value[name] : null;
  for (const name of [
    "stdoutLines",
    "stderrLines",
    "capturedBytes",
    "promptLines",
    "markerMentionLines",
    "numericTargetLines",
    "gflagValueLines",
    "nullCharacters",
  ])
    result[name] =
      Number.isInteger(value?.[name]) &&
      value[name] >= 0 &&
      value[name] <= 0xffffffff
        ? value[name]
        : null;
  result.debuggerExitCode =
    Number.isInteger(value?.debuggerExitCode) &&
    value.debuggerExitCode >= -0x80000000 &&
    value.debuggerExitCode <= 0xffffffff
      ? value.debuggerExitCode
      : null;
  result.reportedWin32Errors = Array.isArray(value?.reportedWin32Errors)
    ? value.reportedWin32Errors
        .filter(
          (code) => Number.isInteger(code) && code >= 0 && code <= 0xffffffff,
        )
        .slice(0, 8)
    : null;
  result.gflagHints = Object.fromEntries(
    ["symbols", "types", "peb", "globalFlags", "failure", "extension"].map(
      (name) => [
        name,
        typeof value?.gflagHints?.[name] === "boolean"
          ? value.gflagHints[name]
          : null,
      ],
    ),
  );
  return result;
}

function protocolQueue(input) {
  const reader = createInterface({ input, crlfDelay: Infinity });
  const queue = [],
    waiters = [];
  let ended = false;
  const deliver = (value) => {
    const waiting = waiters.shift();
    if (waiting) {
      clearTimeout(waiting.timer);
      waiting.resolve(value);
    } else if (queue.length < 16) queue.push(value);
  };
  reader.on("line", (line) => {
    let value = { state: "invalid" };
    if (line.length <= 32768) {
      try {
        const parsed = JSON.parse(line);
        if (
          parsed &&
          typeof parsed === "object" &&
          !Array.isArray(parsed) &&
          ["started", "failed", "raw-ready", "cleaned"].includes(parsed.state)
        )
          value = parsed;
      } catch {}
    }
    deliver(value);
  });
  reader.on("close", () => {
    ended = true;
    while (waiters.length) deliver(null);
  });
  return {
    next(milliseconds) {
      if (queue.length) return Promise.resolve(queue.shift());
      if (ended) return Promise.resolve(null);
      return new Promise((resolve, reject) => {
        const waiting = { resolve, timer: null };
        waiting.timer = setTimeout(() => {
          const index = waiters.indexOf(waiting);
          if (index >= 0) waiters.splice(index, 1);
          reject(new Error("trace protocol deadline"));
        }, milliseconds);
        waiters.push(waiting);
      });
    },
    close() {
      reader.close();
    },
  };
}

export async function startNativeTrace(
  environment,
  {
    spawnController = spawn,
    startupTimeoutMs = 60000,
    cleanupTimeoutMs = 30000,
    decodeTimeoutMs = 600000,
  } = {},
) {
  const raw = path.join(
    await realpath(environment.RUNNER_TEMP),
    `ofox-document-etw-${randomUUID()}`,
  );
  await mkdir(raw, { mode: 0o700 });
  const controller = spawnController(
    "pwsh",
    [
      "-NoLogo",
      "-NoProfile",
      "-NonInteractive",
      "-File",
      path.join(root, "scripts/document-engine-cdb.ps1"),
      "-RawDirectory",
      raw,
    ],
    {
      cwd: root,
      env: environment,
      stdio: ["pipe", "pipe", "pipe"],
    },
  );
  controller.stdin.on("error", () => {});
  controller.stderr.on("data", () => {}); // Never forward native messages/paths.
  const completion = new Promise((resolve) => {
    controller.on("error", () => resolve(null));
    controller.on("close", resolve);
  });
  const protocol = protocolQueue(controller.stdout);
  let metadata = null,
    startFailure = null;
  async function finalize(command) {
    controller.stdin.end(command);
    let cleanup = null;
    const until = Date.now() + cleanupTimeoutMs;
    try {
      while (Date.now() < until) {
        const value = await protocol.next(Math.max(1, until - Date.now()));
        if (!value) break;
        if (value.state === "started")
          metadata ??= safeMetadata(value.metadata);
        if (value.state === "failed")
          startFailure ??= safeControllerFailure(value);
        if (value.state === "cleaned") {
          cleanup = {
            ownedStopSucceeded: value.ownedStopSucceeded === true,
            rawRemoved: value.rawRemoved === true,
            forcedTermination: false,
          };
          break;
        }
      }
    } catch {}
    // A failed notification precedes PowerShell's finally. Never kill merely
    // because that notification arrived; drain the owned-stop acknowledgement.
    if (!cleanup) {
      controller.kill();
      cleanup = {
        ownedStopSucceeded: false,
        rawRemoved: false,
        forcedTermination: true,
        limitation: "unconfirmed-session-cleanup-requires-disposable-ci-vm",
      };
    }
    let closeTimer;
    const closed = await Promise.race([
      completion.then(() => true),
      new Promise((resolve) => {
        closeTimer = setTimeout(() => resolve(false), 5000);
      }),
    ]);
    clearTimeout(closeTimer);
    if (!closed) {
      controller.kill();
      controller.stdout.destroy();
      controller.stderr.destroy();
      controller.unref?.();
      cleanup.forcedTermination = true;
    }
    protocol.close();
    if (cleanup.ownedStopSucceeded || cleanup.rawRemoved) {
      try {
        await rm(raw, { recursive: true, force: true });
      } catch {
        cleanup.rawRemoved = false;
      }
    }
    return cleanup;
  }
  try {
    const started = await protocol.next(startupTimeoutMs);
    assert.ok(started, "trace controller exited");
    metadata = safeMetadata(started.metadata);
    if (started.state === "failed")
      startFailure = safeControllerFailure(started);
    assert.equal(started.state, "started", "trace did not start");
  } catch {
    const cleanup = await finalize();
    return {
      failed: {
        ...traceFailure("capture-start-failed", metadata, cleanup),
        controller: startFailure,
      },
    };
  }
  return {
    environment: { OFOX_DOCUMENT_CDB_DIAGNOSTIC_ROOT: raw },
    async finish(output) {
      let report = traceFailure("trace-decode-incomplete", metadata),
        cleanup = null;
      try {
        controller.stdin.write("STOP\n");
        const ready = await protocol.next(decodeTimeoutMs);
        assert.ok(ready, "trace controller exited");
        if (ready.state === "failed")
          report.controller = safeControllerFailure(ready);
        assert.ok(
          ["raw-ready", "failed"].includes(ready.state),
          "debugger data unavailable",
        );
        const state = JSON.parse(
          await privateRead(path.join(raw, "debugger-state.json"), 32768),
        );
        const safeState = safeDebuggerState(state);
        const roots = JSON.parse(
          await privateRead(path.join(raw, "roots.json"), 32768),
        );
        const targets = extractTraceTargets(output);
        const text = await privateRead(
          path.join(raw, "debugger.log"),
          4 * 1024 * 1024,
        );
        const diagnostic = projectDocumentLoaderSnaps({
          text,
          target: targets.length === 1 ? targets[0] : null,
          roots: {
            ...roots,
            runtime: [path.join(root, "assistant-runtime/resources")],
          },
        });
        report = {
          scope: "windows-node-loader-diagnostic",
          releaseGate: false,
          diagnosticOnly: true,
          metadata,
          controllerState: safeState,
          diagnostic,
        };
        if (ready.state === "failed") {
          report.status = "inconclusive";
          report.controller = safeControllerFailure(ready);
        }
      } catch {
        /* Retain the original native result and a fixed inconclusive diagnostic. */
      } finally {
        cleanup = await finalize("CLEAN\n");
      }
      report.cleanup = cleanup;
      if (!cleanup?.ownedStopSucceeded || !cleanup?.rawRemoved)
        return traceFailure("trace-cleanup-incomplete", metadata, cleanup);
      return report;
    },
  };
}

export function shouldCaptureDevelopmentTrace(environment) {
  return (
    environment.GITHUB_ACTIONS === "true" &&
    environment.GITHUB_WORKFLOW === "Document Engine Development Checks"
  );
}

export async function captureFunctional(invoke, start) {
  let capture;
  try {
    capture = await start();
  } catch {
    return {
      exitCode: null,
      output: "",
      outputLimited: false,
      trace: traceFailure("capture-start-failed", null),
    };
  }
  if (capture.failed)
    return {
      exitCode: null,
      output: "",
      outputLimited: false,
      trace: capture.failed,
    };
  let result;
  try {
    result = await invoke(capture.environment ?? {});
  } catch {
    result = { exitCode: null, output: "", outputLimited: false };
  } finally {
    let trace;
    try {
      trace = await capture.finish(
        result?.diagnosticOutput ?? result?.output ?? "",
      );
    } catch {
      trace = traceFailure("trace-decode-incomplete", null);
    }
    result = { ...result, trace };
  }
  return result;
}

async function invokeNative(check, output, environment) {
  const args = [
    "test",
    "--locked",
    "--manifest-path",
    "src-tauri/Cargo.toml",
    "--lib",
    check.filter,
    "--",
  ];
  if (check.exact) args.push("--exact");
  args.push("--nocapture", "--test-threads=1");
  const log = await open(path.join(output, `${check.name}.log`), "wx", 0o600);
  const chunks = [];
  const stderrChunks = [];
  let bytes = 0,
    limited = false,
    writeFailed = false,
    writes = Promise.resolve();
  try {
    const exitCode = await new Promise((resolve) => {
      const child = spawn("cargo", args, {
        cwd: root,
        env: environment,
        stdio: ["ignore", "pipe", "pipe"],
      });
      const keep = (chunk, stdout) => {
        bytes += chunk.length;
        if (bytes > 32 * 1024 * 1024) {
          limited = true;
          return;
        }
        if (stdout) chunks.push(chunk);
        else stderrChunks.push(chunk);
        process.stdout.write(chunk);
        writes = writes.then(async () => {
          try {
            await log.write(chunk);
          } catch {
            writeFailed = true;
          }
        });
      };
      child.stdout.on("data", (chunk) => keep(chunk, true));
      child.stderr.on("data", (chunk) => keep(chunk, false));
      child.on("error", () => resolve(null));
      child.on("close", resolve);
    });
    await writes;
    return {
      exitCode,
      ...nativeOutput(chunks, stderrChunks),
      outputLimited: limited || writeFailed,
    };
  } finally {
    await log.close();
  }
}

export async function main(argv = process.argv.slice(2)) {
  assert.equal(process.platform, "win32", "native Windows is required");
  assert.equal(process.arch, "x64");
  assert.ok(
    argv.length === 2 && argv[0] === "--out" && path.isAbsolute(argv[1]),
    "expected --out ABS_NEW_DIRECTORY",
  );
  const output = path.join(
    await realpath(path.dirname(argv[1])),
    path.basename(argv[1]),
  );
  const relative = path.relative(await realpath(root), output);
  assert.ok(
    relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative),
    "keep diagnostics outside the checkout",
  );
  await mkdir(output, { mode: 0o700 });
  const temporary = await mkdtemp(
    path.join(os.tmpdir(), "ofox-document-development-"),
  );
  const environment = {
    ...process.env,
    CARGO_TERM_COLOR: "never",
    RUST_BACKTRACE: "0",
  };
  for (const [key, name] of [
    ["CC_SWITCH_TEST_HOME", "home"],
    ["APPDATA", "roaming"],
    ["LOCALAPPDATA", "local"],
    ["HERMES_HOME", "hermes"],
  ]) {
    environment[key] = path.join(temporary, name);
    await mkdir(environment[key]);
  }
  try {
    const report = await runDevelopmentChecks(
      (check, overrides) =>
        invokeNative(check, output, { ...environment, ...overrides }),
      shouldCaptureDevelopmentTrace(environment)
        ? {
            functionalCapture: (invoke) =>
              captureFunctional(invoke, () => startNativeTrace(environment)),
          }
        : {},
    );
    await writeFile(
      path.join(output, "summary.json"),
      `${JSON.stringify(report, null, 2)}\n`,
      { flag: "wx", mode: 0o600 },
    );
    return report.success ? 0 : 1;
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  process.exitCode = await main();
