import test from "node:test";
import assert from "node:assert/strict";
import { localTools } from "../src/local-tools.mjs";

function fixture(reply) {
  const calls = [];
  const runtime = {
    hostTool: async (context, name, args, signal, toolCallId) => {
      calls.push({ name, args, toolCallId });
      return reply(name, args);
    },
    wrapTool: (context, name, description, parameters, execute) => ({
      name,
      description,
      parameters,
      execute: (id, args, signal) => execute(args, signal, id),
    }),
  };
  return {
    calls,
    tools: localTools(runtime, {
      params: { localConfig: { cwd: "/host-only-folder" } },
    }),
  };
}

test("local read retains Pi line selection while every filesystem access goes to the host", async () => {
  const { calls, tools } = fixture((name) =>
    name === "stat" ? { exists: true, file: true } : "first\nsecond\nthird",
  );
  const result = await tools
    .find((t) => t.name === "read")
    .execute("read-1", { path: "config.json", offset: 2, limit: 1 });
  assert.match(result.content[0].text, /second/);
  assert.doesNotMatch(result.content[0].text, /first/);
  assert.ok(
    calls.some(
      (c) =>
        c.name === "read" && c.args.path === "/host-only-folder/config.json",
    ),
  );
  assert.equal(new Set(calls.map((c) => c.toolCallId)).size, calls.length);
});

test("local grep is entirely host backed and strips intent from execution arguments", async () => {
  const { calls, tools } = fixture(() => ["config.json:1:found"]);
  const result = await tools
    .find((t) => t.name === "grep")
    .execute("grep-1", {
      pattern: "found",
      path: ".",
      intent: "Find a setting",
    });
  assert.match(result.content[0].text, /found/);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].name, "grep");
  assert.equal(calls[0].args.intent, undefined);
});

test("local find uses host glob operations and never launches fd", async () => {
  const { calls, tools } = fixture((name) =>
    name === "stat" ? { exists: true, directory: true } : ["one.json"],
  );
  const result = await tools
    .find((t) => t.name === "find")
    .execute("find-1", { pattern: "*.json" });
  assert.match(result.content[0].text, /one.json/);
  assert.ok(calls.some((c) => c.name === "find"));
});

test("local ls caches child metadata without a callback for every entry", async () => {
  const { calls, tools } = fixture((name) =>
    name === "stat"
      ? { exists: true, directory: true }
      : [
          { name: "config.json", directory: false },
          { name: "nested", directory: true },
        ],
  );
  const result = await tools
    .find((t) => t.name === "ls")
    .execute("ls-1", { path: "." });
  assert.match(result.content[0].text, /nested\//);
  assert.equal(calls.filter((c) => c.name === "stat").length, 1);
});

test("bundled local config skill loads in a private agent workspace", async () => {
  const { mkdtemp, mkdir, rm } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const path = await import("node:path");
  const { fileURLToPath } = await import("node:url");
  const { DefaultResourceLoader, SettingsManager } = await import(
    "@earendil-works/pi-coding-agent"
  );
  const root = await mkdtemp(path.join(tmpdir(), "ofox-local-skill-"));
  try {
    const agentDir = path.join(root, "agent");
    await mkdir(agentDir);
    const loader = new DefaultResourceLoader({
      cwd: root,
      agentDir,
      settingsManager: SettingsManager.inMemory(),
      noExtensions: true,
      noContextFiles: true,
      noPromptTemplates: true,
      noThemes: true,
      noSkills: true,
      additionalSkillPaths: [
        fileURLToPath(
          new URL("../skills/local-config/SKILL.md", import.meta.url),
        ),
      ],
    });
    await loader.reload();
    const skills = loader.getSkills().skills;
    assert.equal(skills.length, 1);
    assert.equal(skills[0].name, "local-config");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("local edit keeps Pi matching and diff while writes go to Rust with the observed text", async () => {
  const { calls, tools } = fixture((name) =>
    name === "stat"
      ? { exists: true, file: true }
      : name === "read"
        ? "model=old\n"
        : { changed: true },
  );
  const result = await tools
    .find((t) => t.name === "edit")
    .execute("edit-1", {
      path: "config",
      edits: [{ oldText: "model=old", newText: "model=new" }],
    });
  assert.match(result.details.diff, /model=new/);
  const write = calls.find((c) => c.name === "edit");
  assert.equal(write.args.content, "model=new\n");
  assert.equal(write.args.expected, "model=old\n");
});

test("local shell uses host cwd and bounded output without exporting Pi environment", async () => {
  const { calls, tools } = fixture(() => ({
    exitCode: 0,
    output: "version 1.2.3",
    truncated: false,
  }));
  const result = await tools
    .find((t) => t.name === "bash")
    .execute("exec-1", {
      command: "node --version",
      folder: "/problem",
      intent: "Check version",
    });
  assert.match(result.content[0].text, /1.2.3/);
  assert.equal(calls.length, 1);
  assert.deepEqual(calls[0].args, {
    command: "node --version",
    folder: "/problem",
    timeout: 60,
  });
});
