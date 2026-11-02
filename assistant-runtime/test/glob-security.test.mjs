import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const sdkRequire = createRequire(
  import.meta.resolve("@earendil-works/pi-coding-agent"),
);
const { Minimatch, minimatch } = sdkRequire("minimatch");

test("the SDK's glob parser bounds preprocessing as well as brace expansion", () => {
  const pattern = Array(6).fill("**/../a/b").join("/");
  const matcher = new Minimatch(pattern, {
    optimizationLevel: 2,
    braceExpandMax: 8,
  });

  assert.ok(
    matcher.set.length <= 8,
    `glob preprocessing expanded to ${matcher.set.length} patterns beyond its limit`,
  );
});

test("the SDK's glob dependency preserves model and resource matching", () => {
  const cases = [
    ["OpenAI/gpt-test", "openai/*", { nocase: true }, true],
    ["other/model", "openai/*", { nocase: true }, false],
    ["skills/team/format/SKILL.md", "skills/**/SKILL.md", {}, true],
    ["skills/.hidden/SKILL.md", "skills/**/SKILL.md", {}, false],
    ["skills/.hidden/SKILL.md", "skills/**/SKILL.md", { dot: true }, true],
    ["templates/header.md", "templates/*.{md,txt}", {}, true],
    ["templates/header.js", "templates/*.{md,txt}", {}, false],
  ];

  for (const [filename, pattern, options, expected] of cases)
    assert.equal(minimatch(filename, pattern, options), expected, filename);
});
