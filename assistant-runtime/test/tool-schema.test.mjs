import assert from "node:assert/strict";
import { tmpdir } from "node:os";
import test from "node:test";
import { validateToolArguments } from "@earendil-works/pi-ai";
import { AssistantRuntime } from "../src/runtime.mjs";

const samples = {
  find_skills: { query: "fixture" },
  install_skill: { resultId: "result" },
  save_skill: { name: "fixture", description: "Fixture", body: "Body" },
  use_skill: { name: "sample" },
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
  create_document: { name: "notes.md", format: "md", content: "# Notes" },
  get_presentation_style: { id: "cartesian" },
};

// create_document's other content shape: an officecli batch.
const batch = {
  name: "deck.pptx",
  format: "pptx",
  content: [
    { command: "add", parent: "/", type: "slide" },
    { command: "set", path: "/slide[1]/shape[1]", props: { text: "Hi" } },
  ],
};

for (const platform of ["macos", "windows"])
  for (const vision of [false, true])
    test(`Pi validates every assistant tool on ${platform}, vision=${vision}`, () => {
      const runtime = new AssistantRuntime();
      const context = {
        attachments: new Map([["attachment", {}]]),
        params: {
          skills: [],
          apex: "ofox.io",
          apiKey: "synthetic-schema-only-credential",
          localConfig: { cwd: tmpdir(), platform },
          webSearch: true,
          documents: {
            formats: [
              "md",
              "txt",
              "csv",
              "html",
              "json",
              "pptx",
              "docx",
              "xlsx",
              "pdf",
            ],
          },
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
        if (tool.name === "create_document")
          assert.deepEqual(
            validateToolArguments(tool, {
              name: tool.name,
              arguments: structuredClone(batch),
            }),
            batch,
          );
        if (
          ["use_skill", "find_skills", "install_skill", "save_skill"].includes(
            tool.name,
          )
        )
          assert.throws(
            () =>
              validateToolArguments(tool, {
                name: tool.name,
                arguments: { ...args, dir: "/untrusted" },
              }),
            /Validation failed for tool/,
          );
        assert.throws(
          () =>
            validateToolArguments(tool, { name: tool.name, arguments: null }),
          /Validation failed for tool/,
          tool.name,
        );
      }
    });

test("Pi save schema accepts Unicode scalar limits through the actual wrapper", async () => {
  let calls = 0;
  const runtime = new AssistantRuntime({
    hostCall: async (payload) => {
      calls++;
      assert.equal([...payload.args.description].length, 500);
      assert.equal([...payload.args.body].length, 60000);
      return { saved: true };
    },
  });
  const context = {
    attachments: new Map(),
    params: { skills: [] },
    calls: new Map(),
    callCount: 0,
  };
  const tool = runtime.tools(context, {}).find((t) => t.name === "save_skill");
  const args = {
    name: "fixture",
    description: "😀".repeat(500),
    body: "😀".repeat(60000),
  };
  const checked = validateToolArguments(tool, {
    name: tool.name,
    arguments: args,
  });
  await tool.execute("unicode", checked);
  assert.equal(calls, 1);
  for (const key of ["description", "body"]) {
    const over = { ...args, [key]: args[key] + "😀" };
    assert.throws(
      () => validateToolArguments(tool, { name: tool.name, arguments: over }),
      /Validation failed/,
    );
  }
});
