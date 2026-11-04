import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import test from "node:test";
import { validateToolArguments } from "@earendil-works/pi-ai";
import { AssistantRuntime } from "../src/runtime.mjs";

const samples = {
  read_attachment: { id: "attachment", offset: 0, length: 10 },
  read: { path: "config.json", offset: 1, limit: 10 },
  ls: { path: "." },
  find: { pattern: "*.json" },
  write: { path: "config.json", content: "{}" },
  edit: { path: "config.json", edits: [{ oldText: "old", newText: "new" }] },
  bash: { command: "echo test" },
  powershell: { command: "Write-Output test" },
  grep: { pattern: "config", limit: 10 },
  tool_status: { tool: "codex" },
  diagnose_in_folder: { tool: "codex" },
  verify_in_folder: { tool: "codex" },
  repair_launcher: { tool: "codex" },
  rebind_tool: { tool: "codex" },
  install_tool: { tool: "codex" },
  update_tool: { tool: "codex" },
  uninstall_tool: { tool: "codex" },
  set_tool_model: { tool: "codex", model: "openai/test" },
  updatePlan: { steps: [{ step: "Check settings", status: "in_progress" }] },
  generate_image: { prompt: "a fox", n: 1, size: "1024x1024" },
  generate_video: { prompt: "a fox", duration: 5, material_use: "reference" },
  generate_speech: { text: "hello", language: "en" },
  web_search: { query: "Pi SDK" },
  web_fetch: { url: "https://example.com/" },
};

for (const platform of ["macos", "windows"])
  for (const vision of [false, true])
    test(`Pi validates every assistant tool on ${platform}, vision=${vision}`, () => {
      const runtime = new AssistantRuntime();
      const context = {
        attachments: new Map([["attachment", {}]]),
        params: {
          apex: "ofox.io",
          apiKey: "synthetic-schema-only-credential",
          localConfig: { cwd: tmpdir(), platform },
          webSearch: true,
          media: {
            image: {
              model: "test/image",
              models: ["test/image"],
              sizes: ["1024x1024"],
              several: true,
              maxImages: 4,
            },
            video: {
              model: "test/video",
              models: ["test/video"],
              minDuration: 1,
              maxDuration: 10,
              audio: vision,
            },
            speech: {
              model: "test/speech",
              models: ["test/speech"],
              style: vision,
            },
          },
        },
      };
      const tools = runtime.tools(context, {
        input: vision ? ["text", "image"] : ["text"],
      });
      assert.equal(tools.length, Object.keys(samples).length - 1);
      for (const tool of tools) {
        assert.ok(tool.name in samples, tool.name);
        const args = structuredClone(samples[tool.name]);
        if (vision && tool.name === "read_attachment") args.page = 1;
        assert.deepEqual(
          validateToolArguments(tool, { name: tool.name, arguments: args }),
          args,
          tool.name,
        );
        assert.throws(
          () =>
            validateToolArguments(tool, { name: tool.name, arguments: null }),
          /Validation failed for tool/,
          tool.name,
        );
      }
    });
