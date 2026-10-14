import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { queryClient } from "@/lib/query/queryClient";
import BalanceWarningBanner from "@/components/tray/BalanceWarningBanner";
import type { BalanceTrend } from "@/lib/api/usageOverview";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openExternal: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io" }),
}));
vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({ status: { user: { role: "owner" } } }),
}));

function trend(daysLeft: number | null): BalanceTrend {
  return {
    points: [
      { at: 1_790_000_000_000, amount: 9 },
      { at: 1_790_003_600_000, amount: 8 },
    ],
    perDay: daysLeft === null ? null : 2,
    runsOutAt: daysLeft === null ? null : 1_790_100_000_000,
    daysLeft,
  };
}

function answer(value: BalanceTrend) {
  mocks.invoke.mockImplementation(async (command: string) => {
    if (command === "ofox_get_balance_trend") return value;
    throw new Error(`Unexpected command: ${command}`);
  });
}

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  queryClient.clear();
  mocks.invoke.mockReset();
});

describe("tray low-balance banner", () => {
  it("says in how many days the balance runs out at the current pace", async () => {
    answer(trend(3.4));
    render(<BalanceWarningBanner threshold={10} balanceUsd={4} />);
    expect(screen.getByText("余额不足 $10")).toBeInTheDocument();
    expect(
      await screen.findByText("按当前速度，约 3 天后用完"),
    ).toBeInTheDocument();
  });

  it("says less than a day when it runs out within a day", async () => {
    answer(trend(0.4));
    render(<BalanceWarningBanner threshold={10} balanceUsd={1} />);
    expect(
      await screen.findByText("按当前速度可用不足 1 天，建议尽快充值"),
    ).toBeInTheDocument();
  });

  it("says nothing about the pace without a trend", async () => {
    answer(trend(null));
    render(<BalanceWarningBanner threshold={5.5} balanceUsd={1} />);
    expect(screen.getByText("余额不足 $5.5")).toBeInTheDocument();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("ofox_get_balance_trend"),
    );
    await waitFor(() =>
      expect(queryClient.getQueryState(["balanceTrend"])?.status).toBe(
        "success",
      ),
    );
    expect(screen.queryByText(/用完|不足 1 天/)).toBeNull();
  });

  it("says nothing about the pace when the trend cannot be read", async () => {
    mocks.invoke.mockRejectedValue(new Error("not signed in"));
    render(<BalanceWarningBanner threshold={10} balanceUsd={1} />);
    await waitFor(() =>
      expect(queryClient.getQueryState(["balanceTrend"])?.status).toBe("error"),
    );
    expect(screen.queryByText(/用完|不足 1 天/)).toBeNull();
  });

  it("offers 充值 at the wallet of the current region", async () => {
    answer(trend(null));
    render(<BalanceWarningBanner threshold={10} balanceUsd={1} />);
    screen.getByRole("button", { name: "充值" }).click();
    expect(mocks.openExternal).toHaveBeenCalledWith(
      "https://app.ofox.io/manage/wallet",
    );
  });
});
