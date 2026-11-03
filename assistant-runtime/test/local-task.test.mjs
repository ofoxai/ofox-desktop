import test from "node:test";
import assert from "node:assert/strict";
import { LocalTask, validatePlan } from "../src/local-task.mjs";
const limits = {
  repeatWarning: 3,
  repeatStop: 5,
  maxDurationSeconds: 900,
  maxCostUsd: 1,
  maxToolCalls: 60,
  completionRetries: 2,
};
test("repeat guard ignores intent and argument order, warns at 3 and stops at 5", () => {
  const task = new LocalTask(limits);
  const a = { path: "/tmp", pattern: "x" };
  assert.equal(task.beforeTool("grep", a), null);
  task.beforeTool("grep", { pattern: "x", path: "/tmp", intent: "look" });
  assert.equal(task.beforeTool("grep", a), "repeat_warning");
  task.beforeTool("grep", a);
  assert.throws(() => task.beforeTool("grep", a), /assistant_repeat_limit/);
});
test("duration, cost and final tool-free step enforce independent bounds", () => {
  let now = 0;
  let task = new LocalTask(limits, () => now);
  now = 900001;
  assert.throws(() => task.beforeTool("read", {}), /assistant_duration_limit/);
  task = new LocalTask(limits);
  task.addUsage({ cost: { total: 0.6 } });
  task.addUsage({ cost: { total: 0.4 } });
  assert.throws(() => task.beforeRequest(), /assistant_cost_limit/);
  task = new LocalTask({ ...limits, maxToolCalls: 3 });
  task.beforeTool("read", { path: "a" });
  task.beforeTool("read", { path: "b" });
  assert.equal(task.finalStep, true);
  assert.throws(
    () => task.beforeTool("read", { path: "c" }),
    /assistant_tool_limit/,
  );
});
test("completion uses successful host state after the last change in every original problem folder", () => {
  const task = new LocalTask(limits);
  task.observe(
    "diagnose_in_folder",
    { tool: "codex", folder: "/original" },
    { folder: "/canonical", verified: false },
  );
  task.observe(
    "verify_in_folder",
    { tool: "codex", folder: "/original" },
    { folder: "/canonical", verified: true },
  );
  task.observe("rebind_tool", { tool: "codex" }, { changed: true });
  task.observe(
    "verify_in_folder",
    { tool: "codex", folder: "/wrong" },
    { folder: "/wrong", verified: true },
  );
  assert.equal(task.needsVerification, true);
  assert.ok(task.continuation());
  assert.ok(task.continuation());
  assert.equal(task.continuation(), null);
  task.observe(
    "verify_in_folder",
    { tool: "codex", folder: "/original" },
    { folder: "/canonical", verified: true },
  );
  assert.equal(task.needsVerification, false);
  task.observe("write", { path: "/config" }, { changed: true });
  assert.equal(task.needsVerification, true);
});
test("no-op mutations need no verification and unknown targets cannot be guessed by verification", () => {
  const task = new LocalTask(limits);
  task.observe("rebind_tool", { tool: "codex" }, { changed: false });
  assert.equal(task.needsVerification, false);
  task.observe("write", {}, { changed: true });
  task.observe(
    "verify_in_folder",
    { tool: "codex", folder: "/invented" },
    { verified: true },
  );
  assert.equal(task.needsVerification, true);
});
test("pending verification survives into the next run and clears once verified", () => {
  const task = new LocalTask(limits);
  const target = { tool: "codex", folder: "/problem" };
  task.observe("diagnose_in_folder", target, { verified: false });
  task.observe("write", { path: "/config" }, { changed: true });
  const next = new LocalTask(limits, Date.now, task.snapshot());
  assert.equal(next.needsVerification, true);
  next.observe("verify_in_folder", target, { verified: true });
  assert.equal(next.needsVerification, false);
  assert.equal(next.snapshot(), null);
  for (const state of [{ changed: "yes" }, { targets: "x" }, 7, null])
    assert.equal(
      new LocalTask(limits, Date.now, state).needsVerification,
      false,
    );
});
test("a change with no diagnosed folder is verified in the home folder", () => {
  const home = "/Users/me";
  const task = new LocalTask(limits, Date.now, null, home);
  task.observe("write", { path: "/config" }, { changed: true });
  assert.match(task.continuation(), /home folder/);
  task.observe(
    "verify_in_folder",
    { tool: "codex", folder: "/elsewhere" },
    { folder: "/elsewhere", verified: true },
  );
  assert.equal(task.needsVerification, true);
  task.observe(
    "verify_in_folder",
    { tool: "codex", folder: home },
    { folder: home, verified: true },
  );
  assert.equal(task.needsVerification, false);
  task.observe("write", { path: "/config" }, { changed: true });
  assert.equal(task.needsVerification, true);
});
test("plans cap steps, Unicode length, status and one active step", () => {
  assert.equal(
    validatePlan([{ step: "检查配置", status: "in_progress" }]).length,
    1,
  );
  for (const steps of [
    Array(13).fill({ step: "a", status: "pending" }),
    [{ step: "中".repeat(41), status: "pending" }],
    [
      { step: "a", status: "in_progress" },
      { step: "b", status: "in_progress" },
    ],
    [{ step: "a", status: "bogus" }],
  ])
    assert.throws(() => validatePlan(steps));
});

test("a host-reported command side effect invalidates earlier folder verification", () => {
  const task = new LocalTask(limits);
  const target = { tool: "codex", folder: "/problem" };
  task.observe("diagnose_in_folder", target, { verified: false });
  task.observe("verify_in_folder", target, { verified: true });
  task.observe(
    "bash",
    { command: "repair", folder: "/problem" },
    { requiresVerification: true },
  );
  assert.equal(task.needsVerification, true);
  task.observe("verify_in_folder", target, { verified: true });
  assert.equal(task.needsVerification, false);
});

test("paid search contributes to the same budget and missing prices fail closed", () => {
  const task = new LocalTask(limits);
  task.addSearchUsage(
    { model: "search", searchCalls: 1, inputTokens: 10, outputTokens: 10 },
    {
      model: "search",
      perSearch: "0.5",
      perInputToken: "0.01",
      perOutputToken: "0.04",
    },
  );
  assert.throws(() => task.beforeRequest(), /assistant_cost_limit/);
  assert.throws(
    () => new LocalTask(limits).addSearchUsage(null, null),
    /assistant_cost_unavailable/,
  );
});
