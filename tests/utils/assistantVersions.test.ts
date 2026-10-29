import { describe, expect, it } from "vitest";
import { turnVersions } from "@/utils/assistantVersions";
import type { AssistantMessage } from "@/lib/api/assistant";

let clock = 0;
const user = (
  id: string,
  over: Partial<AssistantMessage> = {},
): AssistantMessage => ({
  id,
  role: "user",
  content: id,
  createdAt: ++clock,
  status: "completed",
  nativeEntryId: `entry-${id}`,
  runId: `run-${id}`,
  branchFrom: null,
  superseded: false,
  ...over,
});
const reply = (
  id: string,
  runId: string,
  over: Partial<AssistantMessage> = {},
): AssistantMessage => ({
  id,
  role: "assistant",
  content: id,
  createdAt: ++clock,
  status: "completed",
  nativeEntryId: `entry-${id}`,
  runId,
  superseded: false,
  ...over,
});

describe("turnVersions", () => {
  it("names the versions of a turn on the shown reply, in the order they were made", () => {
    const versions = turnVersions([
      user("u1", { superseded: true }),
      reply("r1", "run-u1", { superseded: true }),
      user("u2"),
      reply("r2", "run-u2"),
    ]);
    expect([...versions.keys()]).toEqual(["r2"]);
    expect(versions.get("r2")).toMatchObject({
      index: 2,
      count: 2,
      previousId: "u1",
    });
    expect(versions.get("r2")?.nextId).toBeUndefined();
  });

  it("offers both neighbours from the middle version", () => {
    const versions = turnVersions([
      user("u1", { superseded: true }),
      user("u2"),
      reply("r2", "run-u2"),
      user("u3", { superseded: true }),
    ]);
    expect(versions.get("r2")).toMatchObject({
      index: 2,
      count: 3,
      previousId: "u1",
      nextId: "u3",
    });
  });

  it("orders versions by when they were made, not by where they are listed", () => {
    const versions = turnVersions([
      user("late", { createdAt: 50, superseded: true }),
      user("early", { createdAt: 10 }),
      reply("shown", "run-early"),
    ]);
    expect(versions.get("shown")).toMatchObject({
      index: 1,
      count: 2,
      nextId: "late",
    });
  });

  it("keeps the same group for every version of a turn, so the switcher stays put", () => {
    const first = turnVersions([
      user("u1"),
      reply("r1", "run-u1"),
      user("u2", { superseded: true }),
    ]).get("r1");
    const second = turnVersions([
      user("u1", { superseded: true }),
      user("u2"),
      reply("r2", "run-u2"),
    ]).get("r2");
    expect(first?.group).toBe(second?.group);
  });

  it("keeps different turns apart by what they follow", () => {
    const versions = turnVersions([
      user("a1"),
      reply("ra1", "run-a1"),
      user("a2", { superseded: true }),
      user("b1", { branchFrom: "ra1" }),
      reply("rb1", "run-b1"),
      user("b2", { branchFrom: "ra1", superseded: true }),
    ]);
    expect(versions.get("ra1")?.group).not.toBe(versions.get("rb1")?.group);
    expect(versions.get("rb1")).toMatchObject({ index: 1, count: 2 });
  });

  it("puts the switcher on the question while its reply is not there", () => {
    const versions = turnVersions([
      user("u1", { superseded: true }),
      user("u2"),
    ]);
    expect([...versions.keys()]).toEqual(["u2"]);
  });

  it("shows nothing for a turn with one version, an unknown parent or no native entry", () => {
    expect(turnVersions([user("only"), reply("r", "run-only")]).size).toBe(0);
    expect(
      turnVersions([
        user("a", { branchFrom: undefined }),
        user("b", { branchFrom: undefined, superseded: true }),
      ]).size,
    ).toBe(0);
    expect(
      turnVersions([
        user("a", { nativeEntryId: null }),
        user("b", { nativeEntryId: null, superseded: true }),
      ]).size,
    ).toBe(0);
  });

  it("ignores a group with no version on screen", () => {
    expect(
      turnVersions([
        user("a", { superseded: true }),
        user("b", { superseded: true }),
      ]).size,
    ).toBe(0);
  });
});
