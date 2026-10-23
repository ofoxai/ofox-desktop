import { expect, it } from "vitest";
import { collectedRepositorySkill } from "@/components/library/skillIdentity";
import type { SkillDefinition } from "@/lib/api/library";

const row: SkillDefinition = {
  id: "legacy",
  name: "foo",
  directory: "foo",
  sourceState: "ready",
  sourceKind: "repository",
  repository: "Owner/Repo",
  sourceUrl: `https://github.com/Owner/Repo/blob/${"a".repeat(40)}/nested/foo/SKILL.md`,
  targets: [],
};
it("uses full pinned paths and distinguishes a root skill from nested names", () => {
  expect(collectedRepositorySkill(row, "owner/repo:nested/foo")).toBe(true);
  for (const key of [
    "owner/repo:foo",
    "owner/repo:nested/Foo",
    "another/repo:nested/foo",
  ])
    expect(collectedRepositorySkill(row, key)).toBe(false);
  expect(
    collectedRepositorySkill(
      { ...row, id: "owner/repo:", sourceUrl: undefined },
      "OWNER/REPO:",
    ),
  ).toBe(true);
  expect(
    collectedRepositorySkill(
      {
        ...row,
        sourceUrl:
          "https://github.com/Owner/Repo/blob/feature/branch/nested/foo/SKILL.md",
      },
      "owner/repo:nested/foo",
    ),
  ).toBe(false);
});

it("prioritizes pinned full provenance over a legacy leaf ID", () => {
  const legacy = { ...row, id: "owner/repo:foo" };
  expect(collectedRepositorySkill(legacy, "owner/repo:foo")).toBe(false);
  expect(collectedRepositorySkill(legacy, "owner/repo:nested/foo")).toBe(true);
});
