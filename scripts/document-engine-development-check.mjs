// Focused native diagnostics only; never release or installed-client evidence.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  open,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

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

export async function runDevelopmentChecks(invoke) {
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
      const result = await invoke(check);
      step.exitCode = result.exitCode;
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
      output: Buffer.concat(chunks).toString("utf8"),
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
    const report = await runDevelopmentChecks((check) =>
      invokeNative(check, output, environment),
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
