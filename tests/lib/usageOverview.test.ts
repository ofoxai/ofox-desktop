import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  getBalanceTrend,
  getSpending,
  getUsageOverview,
} from "@/lib/api/usageOverview";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

beforeEach(() => invoke.mockReset());

describe("usage overview api", () => {
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
