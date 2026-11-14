import test from "node:test";
import assert from "node:assert/strict";
import { documentTools } from "../src/document-tools.mjs";

function documentTool(formats, result) {
  return documentTools(
    {
      wrapTool: (_context, name, description, parameters, execute) => ({
        name,
        description,
        parameters,
        execute: (_id, args) => execute(args),
      }),
      hostTool: async () => result,
    },
    { params: { documents: { formats, python: false } } },
    { vision: false },
  )[0];
}

test("zero automated document findings do not certify formulas or visual quality", async () => {
  const tool = documentTool(["pptx"], {
    status: "completed",
    file: { id: "fixture", name: "math.pptx", bytes: 100, slides: 1 },
    issues: [],
  });
  const result = await tool.execute("math", {
    name: "math.pptx",
    format: "pptx",
    content: [{ command: "add", parent: "/", type: "slide" }],
  });
  assert.match(result.content[0].text, /Automated check findings: 0/);
  assert.match(result.content[0].text, /formulas.*not verified/i);
  assert.doesNotMatch(
    result.content[0].text,
    /all checks passed|no issues remain/i,
  );
  assert.match(
    tool.promptGuidelines.join("\n"),
    /status updates.*user.s language/i,
  );
});

test("HTML authoring loads an interaction workflow and does not claim a browser test", async () => {
  const tool = documentTool(["html"], {
    status: "completed",
    file: { id: "html", name: "interactive.html", bytes: 100 },
    issues: [],
  });
  assert.match(tool.promptGuidelines.join("\n"), /ofox-office-html/);
  const result = await tool.execute("html", {
    name: "interactive.html",
    format: "html",
    content: "<!doctype html><html lang='zh'><button>筛选</button></html>",
  });
  assert.match(result.content[0].text, /browser.*interactions.*not verified/i);
  assert.doesNotMatch(result.content[0].text, /Issues: 0/);
});
