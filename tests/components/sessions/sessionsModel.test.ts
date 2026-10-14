import { describe, expect, it } from "vitest";
import { fmtTokens, fmtBytes, fmtCost, fmtAgo } from "@/lib/format";
import { groupByFolder, agentCounts } from "@/components/sessions/model";
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
});
