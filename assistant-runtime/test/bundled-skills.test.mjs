import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import test from "node:test";
import { parseDeck } from "../src/deck/schema.mjs";
import { options, run } from "./fixtures/python-engine.mjs";

const skillsDir = new URL("../skills/", import.meta.url);
// The body of each SKILL.md is what use_skill returns to the model.
// The pptx skill carries two routes (Python deck code, and the deck object
// where Python is unavailable) and a worked example for each.
const BODY_LIMITS = { "ofox-office-pdf": 8000, "ofox-office-pptx": 26000 };
const bodyLimit = (name) => BODY_LIMITS[name] ?? 12000;
// Skills teach create_document content (batch arrays, deck objects), never
// command-line calls.
const cliCall = /\bofficecli\s+[a-z]/i;
// CLI-only workflows that do not exist behind create_document.
const cliOnly = [/\bresidents?\b/i, /\bwatch(es|ing)?\b/i, /\bMCP\b/];
// The host reads frontmatter with a YAML parser: a plain scalar must not
// contain ": " or " #" or start with an indicator; quoted values are fine.
const yamlValue = (value) =>
  /^"(?:[^"\\]|\\.)*"$/.test(value) ||
  /^'(?:[^']|'')*'$/.test(value) ||
  !/^[-?:,[\]{}#&*!|>'"%@`]|: |:$| #/.test(value);
const unquote = (value) =>
  value.startsWith('"')
    ? JSON.parse(value)
    : value.startsWith("'")
      ? value.slice(1, -1).replaceAll("''", "'")
      : value;

const length = (text) => [...text].length;

const bundledSkills = async () => {
  const entries = await readdir(skillsDir, { withFileTypes: true });
  const skills = [];
  for (const entry of entries) {
    // local-config reaches the model through its own channel.
    if (!entry.isDirectory() || entry.name === "local-config") continue;
    const text = await readFile(
      new URL(`${entry.name}/SKILL.md`, skillsDir),
      "utf8",
    );
    const front = /^---\n([\s\S]*?)\n---\n/.exec(text);
    assert.ok(front, `${entry.name}: frontmatter`);
    const fields = new Map();
    for (const line of front[1].split("\n")) {
      const match = /^([A-Za-z][\w-]*):[ \t]*(.*)$/.exec(line);
      assert.ok(match, `${entry.name}: frontmatter line ${line}`);
      fields.set(match[1], match[2].trim());
    }
    skills.push({
      dir: entry.name,
      fields,
      name: unquote(fields.get("name") ?? ""),
      description: unquote(fields.get("description") ?? ""),
      body: text.slice(front[0].length),
    });
  }
  return skills.sort((a, b) => a.dir.localeCompare(b.dir));
};

test("the four office authoring skills are bundled", async () => {
  const dirs = (await bundledSkills()).map((skill) => skill.dir);
  for (const format of ["docx", "pdf", "pptx", "xlsx"])
    assert.ok(dirs.includes(`ofox-office-${format}`), format);
});

test("bundled skill frontmatter is valid YAML", async () => {
  for (const skill of await bundledSkills())
    for (const [key, value] of skill.fields)
      assert.ok(yamlValue(value), `${skill.dir}: ${key}: ${value}`);
});

test("bundled skills are named after their directory with the ofox- prefix", async () => {
  for (const skill of await bundledSkills()) {
    assert.equal(skill.name, skill.dir);
    assert.ok(skill.name.startsWith("ofox-"), skill.dir);
  }
});

test("bundled skill descriptions fit the skill list", async () => {
  for (const skill of await bundledSkills()) {
    const size = length(skill.description);
    assert.ok(size >= 1 && size <= 150, `${skill.dir}: ${size}`);
  }
});

test("bundled skill bodies stay within their size limits", async () => {
  for (const skill of await bundledSkills()) {
    const size = length(skill.body);
    assert.ok(size <= bodyLimit(skill.dir), `${skill.dir}: ${size}`);
  }
});

test("bundled skills describe batch arrays and deck objects, not command-line calls", async () => {
  for (const skill of await bundledSkills())
    for (const line of skill.body.split("\n")) {
      assert.doesNotMatch(line, cliCall, skill.dir);
      for (const word of cliOnly) assert.doesNotMatch(line, word, skill.dir);
    }
});

test("office skill examples have the create_document content shape", async () => {
  for (const skill of await bundledSkills()) {
    for (const [, block] of skill.body.matchAll(/```json\n([\s\S]*?)\n```/g)) {
      const items = JSON.parse(block);
      // A deck object must be one create_document accepts as it is.
      if (items?.kind === "deck") {
        const { problems = [], warnings = [] } = parseDeck(items);
        assert.deepEqual(problems, [], skill.dir);
        assert.deepEqual(warnings, [], skill.dir);
        continue;
      }
      assert.ok(Array.isArray(items) && items.length > 0, skill.dir);
      for (const item of items)
        assert.equal(typeof item.command, "string", skill.dir);
    }
    for (const [, block] of skill.body.matchAll(/```typst\n([\s\S]*?)\n```/g))
      assert.doesNotMatch(block, /#(import|include)\b|#set (page|text)\b/);
  }
});

// The worked example has its own fixture, rendered to zero issues by the
// real engine when it changes; deck-object.json is the end-to-end deck, on
// another topic, so the real-model eval does not measure copying.
test("the pptx skill's worked example is the deck-example fixture", async () => {
  const skill = (await bundledSkills()).find(
    (item) => item.dir === "ofox-office-pptx",
  );
  const decks = [...skill.body.matchAll(/```json\n([\s\S]*?)\n```/g)]
    .map(([, block]) => JSON.parse(block))
    .filter((block) => block?.kind === "deck");
  assert.ok(decks.length >= 1, "a deck example");
  const fixture = JSON.parse(
    await readFile(
      new URL(
        "../../src-tauri/tests/fixtures/assistant-documents/deck-example.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  assert.deepEqual(decks[0], fixture);
  const { problems, warnings } = parseDeck(fixture);
  assert.equal(problems, undefined);
  assert.deepEqual(warnings, []);
});

// The pptx skill teaches two routes: Python deck code, and the deck object
// where Python is unavailable. These tests keep the Python half honest.
const pptxSkill = async () =>
  (await bundledSkills()).find((item) => item.dir === "ofox-office-pptx");

const section = (body, heading) => {
  const start = body.search(new RegExp(`^## ${heading}$`, "m"));
  assert.ok(start >= 0, `a "${heading}" section`);
  const rest = body.slice(start + 1);
  const end = rest.search(/^## /m);
  return end < 0 ? rest : rest.slice(0, end);
};

// "name(a, b=1)" -> ["a", "b"]
const parameterNames = (parameters) =>
  parameters
    .split(",")
    .map((parameter) => parameter.split("=")[0].trim())
    .filter(Boolean);

test("the pptx skill names only helpers that exist, with their parameters", async () => {
  const deckPy = await readFile(
    new URL("../src/python/deck.py", import.meta.url),
    "utf8",
  );
  // Top-level helpers the model's code can call: no leading underscore.
  const defined = new Map(
    [...deckPy.matchAll(/^def ([A-Za-z]\w*)\((.*)\):/gm)].map(
      ([, name, parameters]) => [name, parameterNames(parameters)],
    ),
  );
  const documented = [
    ...section((await pptxSkill()).body, "Helpers").matchAll(
      /`([a-z_]+)\(([^`]*)\)`/g,
    ),
  ].map(([, name, parameters]) => [name, parameterNames(parameters)]);
  assert.ok(documented.length > 0, "documented helpers");
  for (const [name, parameters] of documented) {
    assert.ok(defined.has(name), `${name} is not defined in deck.py`);
    assert.deepEqual(parameters, defined.get(name), `${name} parameters`);
  }
  // A helper added to deck.py must reach the skill.
  assert.deepEqual(
    documented.map(([name]) => name).sort(),
    [...defined.keys()].sort(),
  );
});

test("the pptx skill describes preview then deliver, and the deck object where Python is unavailable", async () => {
  const { body } = await pptxSkill();
  assert.match(body, /preview: true/);
  assert.match(body, /fromPreview: "<previewId>"/);
  // The same mechanics the tool itself states.
  assert.match(body, /twice per reply/);
  assert.match(body, /delivered automatically/);
  // A delivery under another name would leave the preview to be delivered
  // as well.
  assert.match(body, /same file name/);
  const fallback = section(body, "When Python is unavailable");
  for (const word of ['"kind": "deck"', "pillars", "evidence", "timeline"])
    assert.ok(fallback.includes(word), `the fallback section has ${word}`);
  // The deck object is the fallback: Python comes first.
  assert.ok(
    body.indexOf("preview: true") <
      body.indexOf("## When Python is unavailable"),
  );
});

test(
  "the pptx skill's Python example builds a deck with no lint",
  options,
  async (t) => {
    const { body } = await pptxSkill();
    const blocks = [...body.matchAll(/```python\n([\s\S]*?)\n```/g)].map(
      ([, code]) => code,
    );
    assert.ok(blocks.length >= 1, "a Python example");
    for (const code of blocks) {
      assert.doesNotMatch(code, /prs\.save|sys\.exit|subprocess|\bopen\(/);
      const result = await run(t, code);
      assert.equal(result.status, 0, result.stderr);
      const lint = JSON.parse(await readFile(result.lint, "utf8"));
      assert.deepEqual(lint, []);
    }
  },
);
