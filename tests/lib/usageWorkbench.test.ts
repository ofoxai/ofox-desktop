import { afterEach, describe, expect, it, vi } from "vitest";
import {
  createUsageQuery,
  resolveWorkbenchRange,
  sameUsageQuery,
} from "@/lib/usageWorkbench";

afterEach(() => vi.unstubAllEnvs());

describe("workbench ranges", () => {
  it("uses 90 local dates including today without fixed-hour subtraction", () => {
    vi.stubEnv("TZ", "America/New_York");
    const endMs = Date.parse("2026-03-10T00:30:00-04:00");
    expect(resolveWorkbenchRange({ period: "90d" }, endMs)).toEqual({
      startMs: Date.parse("2025-12-11T00:00:00-05:00"),
      endMs,
    });
  });

  it("includes the custom final date through its next local midnight", () => {
    vi.stubEnv("TZ", "America/New_York");
    expect(
      resolveWorkbenchRange({
        period: "custom",
        startDate: "2026-03-08",
        endDate: "2026-03-08",
      }),
    ).toEqual({
      startMs: Date.parse("2026-03-08T00:00:00-05:00"),
      endMs: Date.parse("2026-03-09T00:00:00-04:00"),
    });
  });

  it.each([
    ["2026-02-30", "2026-03-01"],
    ["2026-03-10", "2026-03-09"],
    ["", "2026-03-09"],
  ])("rejects an invalid custom interval %s to %s", (startDate, endDate) => {
    expect(() =>
      resolveWorkbenchRange({ period: "custom", startDate, endDate }),
    ).toThrow();
  });

  it("freezes one shared anchor and canonical exact identities", () => {
    const toolIds = ["opencode", "claude", "opencode"];
    const query = createUsageQuery(
      { period: "all" },
      { toolIds, modelIds: ["vendor/model%_", "model"] },
      123_456,
    );
    toolIds.push("gemini");
    expect(query).toEqual({
      version: 1,
      startMs: null,
      endMs: 123_456,
      toolIds: ["claude", "opencode"],
      modelIds: ["model", "vendor/model%_"],
      projectIds: [],
      sourceIds: [],
    });
    expect(sameUsageQuery(query, { ...query, endMs: 123_457 })).toBe(false);
  });
});
