import test from "node:test";
import assert from "node:assert/strict";
import { AssistantRuntime } from "../src/runtime.mjs";
import { skillPrompt, skillTools, replaceSkillIndex } from "../src/skills.mjs";
const skill = (name = "one", mode = "auto") => ({
  name,
  mode,
  dir: `/skills/${name}`,
  description: "😀".repeat(205) + "\nnext",
  hasScripts: false,
  reviewed: false,
});
const context = (skills = [skill()], localConfig) => ({
  params: { skills, localConfig },
  calls: new Map(),
  callCount: 0,
});
test("auto list budgets heading, instructions and omission tail with scalar-safe descriptions", () => {
  const ctx = context([
    ...Array.from({ length: 300 }, (_, i) => skill(`skill-${i}`)),
    skill("manual", "manual"),
  ]);
  const text = skillPrompt(ctx);
  assert.ok([...text].length <= 8000);
  assert.doesNotMatch(text, /manual/);
  const listed = text.split("\n").filter((l) => l.startsWith("- "));
  for (const line of listed) assert.equal([...line.split(": ")[1]].length, 200);
  assert.ok(
    text.endsWith(
      `(${300 - listed.length} more skills not listed; the user can pick them with /)`,
    ),
  );
  assert.equal(
    skillPrompt({ params: { skills: [skill()], model: { tools: false } } }),
    "",
  );
});
test("skill bodies, files and no-local status use the shared wrapper", async () => {
  for (const localConfig of [
    undefined,
    { definition: { limits: { maxToolCalls: 20 } } },
  ]) {
    let calls = 0;
    const rt = new AssistantRuntime({
      hostCall: async () => {
        calls++;
        return {
          name: "one",
          dir: "/skills/one",
          body: "😀".repeat(60001),
          files: Array.from({ length: 201 }, (_, i) => `file-${i}`),
          truncated: false,
        };
      },
    });
    const ctx = context(undefined, localConfig);
    const tool = skillTools(rt, ctx)[0];
    const result = await tool.execute("id", { name: "one" });
    assert.equal([...result.details.body].length, 60000);
    assert.equal(result.details.files.length, 200);
    assert.equal(result.details.truncated, true);
    assert.equal(result.details.scriptsUnavailable, !localConfig);
    assert.deepEqual(await tool.execute("id", { name: "one" }), result);
    assert.equal(calls, 1);
    assert.equal(ctx.callCount, 1);
    await assert.rejects(tool.execute("id", { name: "other" }), /reused/);
    ctx.callCount = 20;
    await assert.rejects(
      tool.execute("over", { name: "one" }),
      /Maximum tool-call/,
    );
    const abort = new AbortController();
    abort.abort();
    await assert.rejects(tool.execute("cancel", { name: "one" }, abort.signal));
  }
});
test("only exact auto names can load; host refresh replaces rather than accumulates", async () => {
  const ctx = context([
    skill(),
    skill("manual", "manual"),
    { ...skill("broken"), broken: "bad" },
    skill("duplicate"),
    skill("duplicate"),
  ]);
  let calls = 0;
  const rt = new AssistantRuntime({
    hostCall: async () => {
      calls++;
      return { body: "ok", files: [] };
    },
  });
  const tool = skillTools(rt, ctx)[0];
  for (const name of [
    "manual",
    "broken",
    "unknown",
    "../x",
    "a/b",
    "a\\b",
    "duplicate",
  ])
    await assert.rejects(tool.execute(name, { name }), /not_available/);
  assert.equal(calls, 0);
  assert.equal(
    replaceSkillIndex(ctx, { skills: [skill("new")], revision: 1 }),
    true,
  );
  assert.equal(
    replaceSkillIndex(ctx, { skills: [skill()], revision: 0 }),
    false,
  );
  await assert.rejects(tool.execute("old", { name: "one" }), /not_available/);
  await tool.execute("new", { name: "new" });
  assert.equal(calls, 1);
});
