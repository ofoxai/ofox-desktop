import { afterEach, describe, expect, it, vi } from "vitest";
import { resolveUsageRange } from "@/lib/usageRange";

afterEach(() => vi.unstubAllEnvs());

describe("usage natural days", () => {
  it("keeps seven local dates across the spring daylight-saving change", () => {
    vi.stubEnv("TZ", "America/New_York");
    const now = Date.parse("2026-03-10T00:30:00-04:00");
    expect(resolveUsageRange({ preset: "7d" }, now).startDate).toBe(
      Date.parse("2026-03-04T00:00:00-05:00") / 1000,
    );
  });
});
