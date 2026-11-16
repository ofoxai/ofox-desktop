import test from "node:test";
import assert from "node:assert/strict";
import { documentTools } from "../src/document-tools.mjs";

function fixture(selectedStyle) {
  const calls = [];
  const runtime = {
    hostTool: async (_context, _name, args) => {
      calls.push(args);
      return {
        status: "completed",
        file: { id: "deck", name: "deck.pptx", bytes: 1, slides: 3 },
        issues: [],
      };
    },
    wrapTool: (_context, name, description, parameters, execute) => ({
      name,
      description,
      parameters,
      promptGuidelines: [],
      execute: (args) => execute(args, undefined, "call"),
    }),
  };
  const tools = documentTools(
    runtime,
    {
      params: {
        documents: {
          formats: ["pptx"],
          python: true,
          ...(selectedStyle === undefined ? {} : { selectedStyle }),
        },
      },
    },
    { vision: false },
  );
  return { calls, tool: tools.find((item) => item.name === "create_document") };
}
const deck = () => ({
  kind: "deck",
  title: "Topic",
  theme: "cartesian",
  slides: [
    { type: "hero", title: "Topic" },
    { type: "statement", title: "Claim", statement: "Evidence" },
    { type: "cta", title: "Next" },
  ],
});

test("a trusted manual style overrides the model's deck theme without mutating its content", async () => {
  const f = fixture("blue-professional");
  const content = deck();
  await f.tool.execute({ name: "deck.pptx", format: "pptx", content });
  assert.equal(f.calls.length, 1);
  assert.match(JSON.stringify(f.calls[0].content), /1E2BFA/);
  assert.equal(content.theme, "cartesian");
});

test("explicit navy is forced while auto and old requests preserve model choice", async () => {
  const run = async (style) => {
    const f = fixture(style);
    await f.tool.execute({
      name: "deck.pptx",
      format: "pptx",
      content: deck(),
    });
    return f.calls[0].content;
  };
  assert.deepEqual(await run("auto"), await run(undefined));
  assert.notDeepEqual(await run("navy"), await run(undefined));
});

test("unknown trusted selections and manually styled raw batches fail before the host", async () => {
  for (const [style, content] of [
    ["unknown-style", deck()],
    ["blue-professional", [{ command: "add", parent: "/", type: "slide" }]],
  ]) {
    const f = fixture(style);
    const result = await f.tool.execute({
      name: "deck.pptx",
      format: "pptx",
      content,
    });
    assert.equal(result.isError, true);
    assert.equal(f.calls.length, 0);
  }
});
