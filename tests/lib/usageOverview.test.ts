import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getBalanceTrend,
  getSpending,
  getUsageOverview,
  getSessionUsageAnalytics,
} from "@/lib/api/usageOverview";
import { createUsageQuery } from "@/lib/usageWorkbench";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

beforeEach(() => invoke.mockReset());

describe("usage overview api", () => {
  it("rejects a backend that silently ignores the requested query", async () => {
    const query = createUsageQuery(
      { period: "90d" },
      { toolIds: ["opencode"] },
      1791993600000,
    );
    invoke.mockResolvedValue({ period: "all", totals: { requests: 99 } });
    await expect(getUsageOverview("all", query)).rejects.toThrow(
      "usage.queryUnsupported",
    );
    expect(invoke).toHaveBeenCalledWith("get_usage_overview", {
      period: "all",
      query,
    });
  });

  it("rejects a receipt for different filters instead of showing its totals", async () => {
    const query = createUsageQuery(
      { period: "all" },
      { modelIds: ["model"] },
      1000,
    );
    invoke.mockResolvedValue({
      appliedQuery: { ...query, modelIds: [] },
      queryFingerprint: "other",
      snapshotVersion: "s1",
    });
    await expect(getUsageOverview("all", query)).rejects.toThrow(
      "usage.queryMismatch",
    );
  });
  it("asks for the period by name", async () => {
    invoke.mockResolvedValue({ period: "7d" });
    await getUsageOverview("7d");
    expect(invoke).toHaveBeenCalledWith("get_usage_overview", {
      period: "7d",
    });
  });

  it("reads the balance trend", async () => {
    const trend = { points: [], perDay: null, runsOutAt: null, daysLeft: null };
    invoke.mockResolvedValue(trend);
    await expect(getBalanceTrend()).resolves.toEqual(trend);
    expect(invoke).toHaveBeenCalledWith("ofox_get_balance_trend");
  });

  it("reads the monthly limit in camelCase", async () => {
    invoke.mockResolvedValue({
      forbidden: false,
      monthlyUsedUsd: 12.5,
      monthlyLimitUsd: 100,
    });
    const spending = await getSpending();
    expect(spending?.monthlyUsedUsd).toBe(12.5);
    expect(invoke).toHaveBeenCalledWith("ofox_get_spending");
  });
});

it("rejects session analysis from a different filter receipt", async () => {
  const query = createUsageQuery(
    { period: "all" },
    { projectIds: ["project-a"] },
    1000,
  );
  invoke.mockResolvedValue({
    appliedQuery: { ...query, projectIds: [] },
    queryFingerprint: "other",
    snapshotVersion: "s1",
  });
  await expect(getSessionUsageAnalytics(query)).rejects.toThrow(
    "usage.queryMismatch",
  );
  expect(invoke).toHaveBeenCalledWith("get_session_usage_analytics", { query });
});
it("rejects analysis without a receipt instead of treating it as supported", async () => {
  invoke.mockResolvedValue({ totals: { sessionCount: 100 } });
  await expect(
    getSessionUsageAnalytics(createUsageQuery({ period: "all" }, {}, 1000)),
  ).rejects.toThrow("usage.queryUnsupported");
});
