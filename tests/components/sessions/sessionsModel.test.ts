import { describe, expect, it } from "vitest";
import i18n from "i18next";
import {
  fmtTokens,
  fmtTokenCount,
  fmtBytes,
  fmtCost,
  fmtAgo,
} from "@/lib/format";
import {
  groupByFolder,
  agentCounts,
  descendants,
  nestSessions,
  sessionSubline,
  subagentTree,
} from "@/components/sessions/model";
import type { ManagedSession } from "@/lib/api/sessions";
describe("session presentation", () => {
  it("formats token scales and millisecond relative times", () => {
    expect([1200, 3400000, 1100000000].map(fmtTokens)).toEqual([
      "1.2K",
      "3.4M",
      "1.1B",
    ]);
    expect(fmtBytes(11000000)).toBe("11 MB");
    expect(fmtAgo(1000, "en", 181000)).toBe("3 minutes ago");
  });
  it("marks estimates and distinguishes missing price and tiny costs", () => {
    expect([null, 0, 0.000001, 0.5, 12.34, 123].map(fmtCost)).toEqual([
      "—",
      "≈$0.000",
      "<$0.001",
      "≈$0.500",
      "≈$12.34",
      "≈$123",
    ]);
  });
  it("sorts project groups by activity and counts agents", () => {
    const rows = [
      { projectDir: "/a", providerId: "claude", lastActiveAt: 1 },
      { projectDir: "/b", providerId: "codex", lastActiveAt: 3 },
      { projectDir: "/a", providerId: "claude", lastActiveAt: 2 },
    ] as ManagedSession[];
    expect(groupByFolder(rows).map((g) => g.path)).toEqual(["/b", "/a"]);
    expect(agentCounts(rows)).toEqual([
      ["claude", 2],
      ["codex", 1],
    ]);
  });
  it("writes session token counts as magpie does", () => {
    expect(
      [999, 1234, 4000000, 123456, 1234567, 22345678, 1523456789].map(
        fmtTokenCount,
      ),
    ).toEqual(["999", "1.2K", "4.0M", "123K", "1.2M", "22M", "1.52B"]);
  });
  it("says size and short id after the message count", () => {
    const t = i18n.getFixedT("en");
    const row = {
      sessionId: "01a13a8e-aa6a-7f90",
      title: "Merge the branches",
      messageCount: 780,
      sizeBytes: 460000000,
      models: ["gpt-6.1-sol", "gpt-6-astra"],
    } as ManagedSession;
    const line = sessionSubline(row, "en", t);
    expect(line).toContain("gpt-6.1-sol, gpt-6-astra");
    expect(line).toContain("460 MB");
    expect(line.endsWith("01a13a8e")).toBe(true);
    expect(
      sessionSubline({ ...row, title: row.sessionId }, "en", t),
    ).not.toContain("01a13a8e");
  });
});

/** Magpie tests/sessions-subagents.test.cjs (MIT). */
describe("subagent nesting", () => {
  const thread = (
    id: string,
    parentSessionId?: string,
    createdAt = 0,
  ): ManagedSession =>
    ({
      providerId: "codex",
      sessionId: id,
      sessionKey: `key-${id}`,
      parentSessionId,
      createdAt,
      projectDir: "/work",
    }) as ManagedSession;
  const rows = [
    thread("x-1"),
    thread("x-2", "x-1"),
    thread("x-3", "x-2"),
    thread("x-4", "x-5"),
    thread("x-5", "x-4"),
    thread("x-6", "x-gone"),
  ];
  it("nests a thread under its listed parent; a loop or a missing parent stays on top", () => {
    const { tops, kids } = nestSessions(rows);
    expect(tops.map((s) => s.sessionId)).toEqual(["x-1", "x-4", "x-5", "x-6"]);
    expect(kids.get("key-x-1")?.map((s) => s.sessionId)).toEqual(["x-2"]);
    expect(kids.get("key-x-2")?.map((s) => s.sessionId)).toEqual(["x-3"]);
    const folder = tops.flatMap((s) => descendants(s, kids));
    expect(folder).toHaveLength(6);
  });
  it("orders a parent's threads oldest first and keeps agents apart", () => {
    const { kids, tops } = nestSessions([
      thread("p"),
      thread("late", "p", 30),
      thread("early", "p", 10),
      { ...thread("other"), providerId: "claude", parentSessionId: "p" },
    ]);
    expect(kids.get("key-p")?.map((s) => s.sessionId)).toEqual([
      "early",
      "late",
    ]);
    expect(tops.map((s) => s.sessionId)).toEqual(["p", "other"]);
  });
});

/** Magpie sessions client.js `subTree` (MIT). */
describe("subagent order", () => {
  it("puts each subagent under the one that started it; a loop stays shown", () => {
    const tree = subagentTree([
      { id: "a" },
      { id: "c", parent: "b" },
      { id: "b", parent: "a" },
      { id: "x", parent: "y" },
      { id: "y", parent: "x" },
      { id: "z", parent: "gone" },
    ]);
    expect(tree.map(([s, depth]) => `${s.id}:${depth}`)).toEqual([
      "a:0",
      "b:1",
      "c:2",
      "z:0",
      "x:0",
      "y:1",
    ]);
  });
});
