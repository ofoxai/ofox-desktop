import { describe, expect, it } from "vitest";
import { parseSkillSource } from "@/components/library/skillSource";

describe("parseSkillSource", () => {
  it("reads owner/repo without inventing a branch", () => {
    expect(parseSkillSource(" anthropics/skills ")).toEqual({
      kind: "repository",
      owner: "anthropics",
      name: "skills",
    });
  });
  it("reads a GitHub link with a branch and a folder", () => {
    expect(
      parseSkillSource(
        "https://github.com/anthropics/skills/tree/dev/skills/pdf/",
      ),
    ).toEqual({
      kind: "repository",
      owner: "anthropics",
      name: "skills",
      branch: "dev",
      path: "skills/pdf",
    });
    expect(parseSkillSource("github.com/fixture/repo.git")).toEqual({
      kind: "repository",
      owner: "fixture",
      name: "repo",
    });
  });
  it("treats a path from / or ~ as one skill folder named by its last part", () => {
    expect(parseSkillSource("~/code/skills/pdf/")).toEqual({
      kind: "folder",
      path: "~/code/skills/pdf",
      leaf: "pdf",
    });
    expect(parseSkillSource("/tmp/research")).toMatchObject({
      kind: "folder",
      leaf: "research",
    });
  });
  it("refuses anything else", () => {
    for (const text of ["", "skills", "a/b/c", "https://gitlab.com/a/b", "~"])
      expect(parseSkillSource(text)).toBeNull();
  });
});
