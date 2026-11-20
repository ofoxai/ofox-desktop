import test from "node:test";
import path from "node:path";
import os from "node:os";
import assert from "node:assert/strict";
import { localTools } from "../src/local-tools.mjs";

const HOST_ROOT = path.join(os.tmpdir(), "host-only-folder");

function fixture(access) {
  const calls = [];
  const runtime = {
    hostTool: async (_context, name, args, _signal, toolCallId) => {
      calls.push({ name, args, toolCallId });
      return { folder: args.folder };
    },
    wrapTool: (_context, name, description, parameters, execute) => ({
      name,
      description,
      parameters,
      execute: (id, args, signal) => execute(args, signal, id),
    }),
  };
  return {
    calls,
    tools: localTools(runtime, {
      localConfig: {
        cwd: HOST_ROOT,
        task: { tool: "codex", access },
      },
    }),
  };
}

test("configuration uses the host-selected tool and work folder for diagnosis and verification", async () => {
  const { calls, tools } = fixture("configure");
  for (const name of ["diagnose_in_folder", "verify_in_folder"]) {
    const tool = tools.find((entry) => entry.name === name);
    const result = await tool.execute(name, {
      tool: "codex",
      intent: "Inspect the configuration",
    });
    assert.match(result.content[0].text, /host-only-folder/);
    assert.deepEqual(calls.at(-1), {
      name,
      args: { tool: "codex", folder: HOST_ROOT },
      toolCallId: name,
    });
    assert.equal("folder" in tool.parameters.properties, false);
  }
});

test("bundled local config skill loads in a private agent workspace", async () => {
  const { mkdtemp, mkdir, rm } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const path = await import("node:path");
  const { fileURLToPath } = await import("node:url");
  const { DefaultResourceLoader, SettingsManager } =
    await import("@earendil-works/pi-coding-agent");
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
