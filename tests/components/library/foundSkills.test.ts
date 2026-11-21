import { expect, it } from "vitest";
import type { SkillDefinition } from "@/lib/api/library";
import {
  foundSkills,
  nativeSkillDifferences,
} from "@/components/library/foundSkills";

it("does not label metadata-only skill discovery as a verified content difference", () => {
  const row: SkillDefinition = {
    id: "writing",
    name: "writing",
    directory: "writing",
    revision: 1,
    sourceState: "ready",
    targets: [],
  };
  const entry = {
    entryId: "found",
    kind: "skills" as const,
    name: "writing",
    directory: "writing",
    sources: [{ sourceId: "source", toolId: "claude" }],
    matchedResourceIds: [],
    conflict: false,
    contentVerified: false,
  };
  expect(nativeSkillDifferences([entry], row)).toEqual([]);
  expect(
    nativeSkillDifferences([{ ...entry, contentVerified: true }], row),
  ).toEqual(["claude"]);
  const { contentVerified: _verified, ...olderBackend } = entry;
  expect(nativeSkillDifferences([olderBackend], row)).toEqual(["claude"]);
});

it("does not call separate physical copies different versions without content evidence", () => {
  const first = {
    entryId: "first",
    kind: "skills" as const,
    name: "writing",
    directory: "writing",
    sources: [{ sourceId: "source", toolId: "claude" }],
    matchedResourceIds: [],
    conflict: false,
    contentVerified: false,
    manifestHash: "a".repeat(64),
  };
  const second = {
    ...first,
    entryId: "second",
    sources: [{ sourceId: "second-source", toolId: "codex" }],
  };
  expect(foundSkills([first, second], [])[0].others).toEqual([]);
  expect(
    foundSkills([first, { ...second, manifestHash: "b".repeat(64) }], [])[0]
      .others,
  ).toEqual(["codex"]);
});
