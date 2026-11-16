import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import {
  assertRustResult,
  checks,
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
  assert.equal(calls.length, 8);
  assert.match(
    checks[0].filter,
    /::document_cache_requires_exact_specific_file_read_execute_acl$/,
  );
  assert.ok(
    checks.every(
      (check) =>
        !check.filter.includes("local_interruption_tests") &&
        !check.filter.includes("one_guarded_task"),
    ),
  );
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
