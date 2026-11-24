import test from "node:test";
import assert from "node:assert/strict";
import { LocalTask } from "../src/local-task.mjs";

const limits = {
  repeatWarning: 3,
  repeatStop: 5,
  maxDurationSeconds: 900,
  maxCostUsd: 1,
  maxToolCalls: 60,
  completionRetries: 2,
};

test("a resumed repair with incomplete scope cannot be cleared by guessing its home", () => {
  const task = new LocalTask(
    limits,
    Date.now,
    { changed: true, targets: [], scopeIncomplete: true },
    "/authorized-home",
  );
  task.observe(
    "verify_in_folder",
    { tool: "codex" },
    { folder: "/authorized-home", verified: true },
  );
  assert.equal(task.needsVerification, true);
  assert.equal(task.snapshot().scopeIncomplete, true);
});

test("same-folder verification for another tool cannot clear a scoped repair", () => {
  const task = new LocalTask(limits, Date.now, null, "/authorized-home");
  task.observe("rebind_tool", { tool: "codex" }, { changed: true });
  task.observe(
    "verify_in_folder",
    { tool: "opencode" },
    { folder: "/authorized-home", verified: true },
  );
  assert.equal(task.needsVerification, true);
  task.observe(
    "verify_in_folder",
    { tool: "codex" },
    { folder: "/authorized-home", verified: true },
  );
  assert.equal(task.needsVerification, false);
});
