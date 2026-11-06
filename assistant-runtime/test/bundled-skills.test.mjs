import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import test from "node:test";

const skillsDir = new URL("../skills/", import.meta.url);
// The body of each SKILL.md is what use_skill returns to the model.
const bodyLimit = (name) => (name === "ofox-office-pdf" ? 8000 : 12000);
// Skills teach create_document batch arrays, never command-line calls.
const cliCall = /\bofficecli\s+(create|add|set|batch|view|get|query|remove)\b/i;

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
    const field = (key) =>
      new RegExp(`^${key}:[ \\t]*(.*)$`, "m").exec(front[1])?.[1].trim();
    skills.push({
      dir: entry.name,
      name: field("name"),
      description: field("description") ?? "",
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

test("bundled skills describe batch arrays, not command-line calls", async () => {
  for (const skill of await bundledSkills())
    for (const line of skill.body.split("\n"))
      assert.doesNotMatch(line, cliCall, skill.dir);
});

test("office skill examples have the create_document content shape", async () => {
  for (const skill of await bundledSkills()) {
    for (const [, block] of skill.body.matchAll(/```json\n([\s\S]*?)\n```/g)) {
      const items = JSON.parse(block);
      assert.ok(Array.isArray(items) && items.length > 0, skill.dir);
      for (const item of items)
        assert.equal(typeof item.command, "string", skill.dir);
    }
    for (const [, block] of skill.body.matchAll(/```typst\n([\s\S]*?)\n```/g))
      assert.doesNotMatch(block, /#(import|include)\b|#set (page|text)\b/);
  }
});
