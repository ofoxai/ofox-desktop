import { expect, it } from "vitest";
import {
  selectPopularSkills,
  metaDescription,
} from "../../scripts/build-popular-skills.mjs";

it("selects highest installs before enforcing four per repository and sixty overall", () => {
  const entries = Array.from({ length: 100 }, (_, i) => ({
    source: `owner/repo-${Math.floor(i / 5)}`,
    skillId: `skill-${i}`,
    name: `skill-${i}`,
    installs: i,
    isOfficial: i === 99,
    body: "never copied",
  }));
  const selected = selectPopularSkills(entries);
  expect(selected).toHaveLength(60);
  expect(selected[0]).toEqual({
    source: "owner/repo-19",
    skillId: "skill-99",
    name: "skill-99",
    installs: 99,
    official: true,
  });
  expect(
    selected.some((skill: { skillId: string }) => skill.skillId === "skill-95"),
  ).toBe(false);
  const per = new Map<string, number>();
  for (const skill of selected)
    per.set(skill.source, (per.get(skill.source) ?? 0) + 1);
  expect(Math.max(...per.values())).toBe(4);
  expect(selected.map((skill: { installs: number }) => skill.installs)).toEqual(
    selected
      .map((skill: { installs: number }) => skill.installs)
      .sort((a: number, b: number) => b - a),
  );
});

it("never recommends excluded repositories or skills", () => {
  const entry = (source: string, skillId: string, installs: number) => ({
    source,
    skillId,
    name: skillId,
    installs,
  });
  const selected = selectPopularSkills([
    entry("genmedia-labs/skills", "ai-music", 9),
    entry("101-skills/superpowers", "ai-video-generation", 8),
    entry("flowkit-labs/skills", "reddit-automation", 7),
    entry("leonxlnx/taste-skill", "redesign-existing-projects", 6),
    entry("leonxlnx/taste-skill", "minimalist-ui", 5),
  ]);
  expect(selected.map((skill: { skillId: string }) => skill.skillId)).toEqual([
    "minimalist-ui",
  ]);
});

it("retains zero installs and false official metadata without copying bodies", () => {
  expect(
    selectPopularSkills([
      {
        source: "owner/repo",
        skillId: "zero",
        name: "zero",
        installs: 0,
        body: "private",
      },
    ]),
  ).toEqual([
    {
      source: "owner/repo",
      skillId: "zero",
      name: "zero",
      installs: 0,
      official: false,
    },
  ]);
});

it("extracts descriptions regardless of attribute order and decodes HTML entities", () => {
  expect(
    metaDescription(
      '<meta content="Build &amp; test &#39;skills&#39; &#x1F98A;" name="description">',
    ),
  ).toBe("Build & test 'skills' 🦊");
  expect(
    metaDescription(
      "<META NAME='description' CONTENT='Example &quot;skill&quot;'>",
    ),
  ).toBe('Example "skill"');
  expect(
    metaDescription('<meta name="other" content="unused">'),
  ).toBeUndefined();
});
