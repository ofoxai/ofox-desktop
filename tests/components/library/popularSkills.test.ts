import { expect, it } from "vitest";
import snapshot from "@/components/library/popularSkills.json";
import {
  formatInstalls,
  ownerAvatar,
  POPULAR_SKILLS,
} from "@/components/library/popularSkills";

it("formats install counts including boundaries", () => {
  expect(
    [0, 999, 1000, 1500, 954000, 1000000, 1234567].map(formatInstalls),
  ).toEqual(["0", "999", "1K", "1.5K", "954K", "1M", "1.2M"]);
});
it("keeps at most four skills per repository and at most 60", () => {
  expect(POPULAR_SKILLS.length).toBeGreaterThan(0);
  expect(POPULAR_SKILLS.length).toBeLessThanOrEqual(60);
  const per = new Map<string, number>();
  for (const skill of POPULAR_SKILLS)
    per.set(skill.source, (per.get(skill.source) ?? 0) + 1);
  expect(Math.max(...per.values())).toBeLessThanOrEqual(4);
});
it("sorts by installs, highest first", () => {
  const counts = POPULAR_SKILLS.map((skill) => skill.installs);
  expect([...counts].sort((a, b) => b - a)).toEqual(counts);
});
it("bundles attributed metadata and local avatars without skill bodies", () => {
  expect(snapshot.source).toBe("magpie market_skills.json (MIT, yetone)");
  expect(Number.isNaN(Date.parse(snapshot.generatedAt))).toBe(false);
  for (const skill of POPULAR_SKILLS) {
    expect(typeof skill.official).toBe("boolean");
    expect(Object.keys(skill).sort()).toEqual(
      [
        "source",
        "skillId",
        "name",
        "installs",
        "official",
        ...(skill.description ? ["description"] : []),
      ].sort(),
    );
    expect(ownerAvatar(skill.source)).toMatch(/owners\/[^/]+\.png/);
    expect(ownerAvatar(skill.source)).not.toMatch(/^https?:/);
  }
  expect(ownerAvatar("unknown-author/example")).toBeUndefined();
  expect(ownerAvatar("constructor/example")).toBeUndefined();
});
