import { useEffect } from "react";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { queryClient } from "@/lib/query/queryClient";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import type { UsageOverview } from "@/lib/api/usageOverview";
const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  openExternal: vi.fn(),
  apex: "ofox.io",
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/api", () => ({
  settingsApi: { openExternal: mocks.openExternal },
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: mocks.apex }),
}));
const user = {
  name: "Ada",
  email: "a@example.test",
  org_id: "a",
  avatar_url: null,
  role: "owner" as const,
  balance: { balance: 20 },
};
const account = {
  user,
  retrying: false,
  retryAttempt: 0,
  loadFailed: false,
  balanceRefreshing: false,
  loadUser: vi.fn(),
  retryUser: vi.fn(),
  refreshBalance: vi.fn(),
} as OfoxAccount;
const totals = {
  requests: 10,
  inputTokens: 1000,
  outputTokens: 200,
  cacheReadTokens: 3000,
  cacheWriteTokens: 50,
  totalTokens: 1200,
  cacheHitRate: 0.75,
  costUsd: 0.02,
};
const overview: UsageOverview = {
  period: "7d",
  start: 1791907200000,
  end: 1791993600000,
  granularity: "day",
  totals,
  series: [{ start: 1791907200000, ...totals }],
  byTool: [{ appType: "claude", ...totals }],
  byModel: [
    { model: "unknown-model", appTypes: ["claude"], priced: false, ...totals },
  ],
  lastSyncedAt: 1791993600000,
  unsupportedTools: ["hermes", "pi", "dsh"],
};
function Refresh() {
  const shell = useShell();
  useEffect(() => shell.setActive("usage"), [shell.setActive]);
  return <button onClick={() => void shell.refresh()}>refresh-page</button>;
}
function Page({ value = account }: { value?: OfoxAccount }) {
  return (
    <ShellProvider openSettings={() => undefined}>
      <Refresh />
      <UsagePage account={value} boundTools={["claude", "pi"]} />
    </ShellProvider>
  );
}

it("refreshes only local statistics when there is no authenticated account", async () => {
  render(<Page value={{ ...account, user: null }} />);
  await screen.findAllByText("1.2K");
  mocks.invoke.mockClear();
  fireEvent.click(screen.getByText("refresh-page"));
  await waitFor(() =>
    expect(
      mocks.invoke.mock.calls.some(([name]) => name === "get_usage_overview"),
    ).toBe(true),
  );
  const commands = mocks.invoke.mock.calls.map(([name]) => name);
  expect(commands).toContain("sync_session_usage");
  expect(commands).not.toContain("ofox_refresh_user_info");
  expect(commands).not.toContain("ofox_get_spending");
  expect(commands).not.toContain("ofox_get_balance_trend");
});
beforeEach(() => {
  queryClient.clear();
  localStorage.clear();
  mocks.invoke.mockReset();
  mocks.apex = "ofox.io";
  mocks.openExternal.mockReset().mockResolvedValue(undefined);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.invoke.mockImplementation(async (command: string, args: unknown) => {
    if (command === "get_usage_overview")
      return { ...overview, ...(args as object) };
    if (command === "ofox_get_balance_trend")
      return { points: [], daysLeft: null, perDay: null, runsOutAt: null };
    if (command === "ofox_get_spending")
      return { forbidden: false, monthlyUsedUsd: 5, monthlyLimitUsd: 50 };
    if (command === "ofox_refresh_user_info") return user;
    if (command === "sync_session_usage") return {};
    throw new Error(command);
  });
});
it("publishes consecutive successful balance samples to the shared account", async () => {
  queryClient.setQueryData(["ofoxUser"], user);
  const normal = mocks.invoke.getMockImplementation()!;
  let amount = 20;
  mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
    command === "ofox_refresh_user_info"
      ? Promise.resolve({ ...user, balance: { balance: --amount } })
      : normal(command, ...args),
  );
  render(<Page />);
  const balance = () =>
    queryClient.getQueryData<typeof user>(["ofoxUser"])?.balance.balance;
  await waitFor(() => expect(balance()).toBe(19));
  fireEvent.click(screen.getByText("refresh-page"));
  await waitFor(() => expect(balance()).toBe(18));
});
describe("UsagePage", () => {
  it("shows KPIs, separate cache and unknown price, refreshes a balance sample on first visit", async () => {
    render(<Page />);
    expect(await screen.findByText("75%", { exact: false })).toBeVisible();
    expect(screen.getAllByText("1.2K").length).toBeGreaterThan(0);
    expect(screen.getAllByText("≈$0.020").length).toBeGreaterThan(0);
    expect(screen.getByText("—")).toBeVisible();
    expect(screen.getByText(/以下工具暂不统计/)).toHaveTextContent("Pi");
    expect(screen.getByText(/以下工具暂不统计/)).not.toHaveTextContent(
      "Hermes",
    );
    expect(mocks.invoke).toHaveBeenCalledWith("ofox_refresh_user_info");
    expect(screen.queryByText(/后用完/)).toBeNull();
    expect(screen.getByText(/本月已用/)).toHaveTextContent("$5.00 / $50.00");
  });
  it("changes and persists the period and refreshes through the shell", async () => {
    render(<Page />);
    await screen.findByText("unknown-model");
    fireEvent.click(screen.getByRole("radio", { name: "30 天" }));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_usage_overview", {
        period: "30d",
      }),
    );
    expect(localStorage.getItem("ofox.usagePeriod")).toBe("30d");
    fireEvent.click(screen.getByText("refresh-page"));
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("sync_session_usage"),
    );
  });
  it("hides and restores model rows without changing totals", async () => {
    render(<Page />);
    await screen.findByText("unknown-model");
    await userEvent.click(
      screen.getByRole("button", { name: "unknown-model 操作" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "隐藏" }));
    expect(
      JSON.parse(localStorage.getItem("ofox.usageHidden.models")!),
    ).toEqual(["unknown-model"]);
    expect(screen.queryByText("unknown-model")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "1 个已隐藏" }));
    fireEvent.click(screen.getByRole("button", { name: "显示 unknown-model" }));
    expect(screen.getByText("unknown-model")).toBeVisible();
  });
  it.each(["ofox.ai", "ofox.io"])(
    "composes wallet and detail links for %s",
    async (apex) => {
      mocks.apex = apex;
      render(<Page />);
      await screen.findByText("unknown-model");
      fireEvent.click(screen.getByRole("button", { name: "充值" }));
      fireEvent.click(
        screen.getByRole("button", { name: /在网页查看详细用量/ }),
      );
      expect(mocks.openExternal).toHaveBeenCalledWith(
        `https://app.${apex}/manage/wallet`,
      );
      expect(mocks.openExternal).toHaveBeenCalledWith(
        `https://app.${apex}/dashboard`,
      );
    },
  );
  it("keeps balance and a web link when an old backend lacks overview", async () => {
    const normal = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
      command === "get_usage_overview"
        ? Promise.reject("Command get_usage_overview not found")
        : normal(command, ...args),
    );
    render(<Page />);
    expect(await screen.findByText("本地用量暂不可用")).toBeVisible();
    expect(screen.getByText("$20.00")).toBeVisible();
    expect(
      screen.getByRole("button", { name: /在网页查看详细用量/ }),
    ).toBeVisible();
  });
  it("does not reuse account A balance history for account B", async () => {
    const normal = mocks.invoke.getMockImplementation()!;
    let release!: (v: unknown) => void;
    mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
      command === "ofox_get_balance_trend"
        ? new Promise((r) => {
            release = r;
          })
        : normal(command, ...args),
    );
    const view = render(<Page />);
    await waitFor(() => expect(release).toBeTypeOf("function"));
    const releaseA = release;
    const b = {
      ...account,
      user: {
        ...user,
        org_id: "b",
        email: "b@example.test",
        balance: { balance: 90 },
      },
    };
    view.rerender(<Page value={b} />);
    releaseA({ points: [], daysLeft: 123, perDay: 1, runsOutAt: Date.now() });
    await waitFor(() => expect(screen.queryByText(/123 天后用完/)).toBeNull());
  });
});

it("keeps honest empty and refresh-failure states", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
    command === "get_usage_overview"
      ? Promise.resolve({
          ...overview,
          totals: { ...totals, requests: 0 },
          byTool: [],
          byModel: [],
          series: [],
        })
      : command === "sync_session_usage"
        ? Promise.reject("offline sync")
        : normal(command, ...args),
  );
  render(<Page />);
  expect(await screen.findByText("该时间范围内暂无用量")).toBeVisible();
  fireEvent.click(screen.getByText("refresh-page"));
  expect(await screen.findByText("Error: offline sync")).toBeInTheDocument();
  expect(screen.getByText("$20.00")).toBeVisible();
});

it("shows hours only with a forecast and reads millisecond chart timestamps", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
    command === "ofox_get_balance_trend"
      ? Promise.resolve({
          points: [
            { at: 1791907200000, amount: 22 },
            { at: 1791993600000, amount: 20 },
          ],
          daysLeft: 0.5,
          perDay: 2,
          runsOutAt: 1792036800000,
        })
      : normal(command, ...args),
  );
  render(<Page />);
  expect(await screen.findByText(/约 12 小时后用完/)).toBeVisible();
  const stamp = new Date(overview.series[0].start).toLocaleString("zh", {
    month: "short",
    day: "numeric",
  });
  expect(screen.getByText(stamp)).toBeVisible();
});

it("uses a newer shared balance after the tools page refreshed it", async () => {
  const view = render(<Page />);
  await screen.findByText("unknown-model");
  view.rerender(
    <Page
      value={{ ...account, user: { ...user, balance: { balance: 10 } } }}
    />,
  );
  expect(screen.getByText("$10.00")).toBeVisible();
  expect(screen.queryByText("$20.00")).toBeNull();
});

it("separates same-organization users with missing email by stable id", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  let calls = 0;
  mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
    command === "ofox_get_balance_trend"
      ? Promise.resolve({
          points: [],
          perDay: 1,
          runsOutAt: Date.now(),
          daysLeft: ++calls === 1 ? 123 : null,
        })
      : normal(command, ...args),
  );
  const one = { ...account, user: { ...user, id: "user-a", email: null } };
  const two = { ...account, user: { ...user, id: "user-b", email: null } };
  const view = render(<Page value={one} />);
  expect(await screen.findByText(/123 天后用完/)).toBeVisible();
  view.rerender(<Page value={two} />);
  await waitFor(() => expect(screen.queryByText(/123 天后用完/)).toBeNull());
});

it("does not claim an extra hour when the balance is exhausted", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string, ...args: unknown[]) =>
    command === "ofox_get_balance_trend"
      ? Promise.resolve({
          points: [],
          daysLeft: 0,
          perDay: 1,
          runsOutAt: Date.now(),
        })
      : normal(command, ...args),
  );
  render(<Page />);
  await screen.findByText("unknown-model");
  expect(screen.queryByText(/小时后用完/)).toBeNull();
});
