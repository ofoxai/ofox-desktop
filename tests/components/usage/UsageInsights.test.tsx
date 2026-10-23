import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";
import { UsageTimeInsights } from "@/components/usage-overview/UsageTimeInsights";
import { UsageDistributions } from "@/components/usage-overview/UsageDistributions";
import type { AnalyticsPage } from "@/types/sessionAnalytics";
const mocks = vi.hoisted(() => ({ page: vi.fn() }));
vi.mock("@/lib/api/usageOverview", () => ({
  getSessionAnalyticsPage: mocks.page,
}));

beforeEach(() => {
  localStorage.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.page.mockReset();
});

it("uses receipt-bound hourly trend buckets for today and retains daily calendar data", async () => {
  const base = sessionAnalyticsFixture();
  const start = new Date(2026, 9, 23).getTime();
  const hour = 3600000;
  const data = Object.assign({}, base, {
    appliedQuery: {
      ...base.appliedQuery,
      startMs: start,
      endMs: start + 2.5 * hour,
    },
    trend: {
      granularity: "hour" as const,
      series: [0, 1, 2].map((index) => ({
        ...base.series[0],
        startMs: start + index * hour,
        endMs: start + Math.min(index + 1, 2.5) * hour,
        inputTokens: index * 10,
        outputTokens: 0,
        totalTokens: index * 10,
      })),
    },
    series: [
      {
        ...base.series[0],
        date: "2026-10-23",
        startMs: start,
        totalTokens: 30,
      },
    ],
  });
  const view = render(<UsageSessions data={data} active />);
  expect(screen.getByRole("heading", { name: "每小时趋势" })).toBeVisible();
  expect(screen.getByText("每小时")).toBeVisible();
  const chart = screen.getByRole("tabpanel");
  const points = within(chart).getAllByRole("button");
  expect(points).toHaveLength(3);
  expect(points[0]).toHaveAccessibleName(/0 Tokens/);
  expect(points[1]).toHaveAccessibleName(/10 Tokens/);
  expect(points[2]).toHaveAccessibleName(/20 Tokens/);
  points[0].focus();
  await userEvent.keyboard("{End}");
  expect(points[2]).toHaveFocus();
  await userEvent.click(screen.getByRole("radio", { name: "活动日历" }));
  expect(screen.getByRole("button", { name: "2026-10-23 · 30" })).toBeVisible();

  view.rerender(<UsageSessions data={base} active />);
  expect(screen.getByRole("heading", { name: "每日趋势" })).toBeVisible();
  expect(screen.queryByText("每小时")).toBeNull();
});

it("keeps a full ranking frozen while new analysis arrives and ignores a late previous sort", async () => {
  const data = sessionAnalyticsFixture();
  data.analyticsSnapshotId = "frozen-one";
  let resolveFirst!: (value: AnalyticsPage) => void;
  const reply = (title: string, sortBy: "tokens" | "cost"): AnalyticsPage => ({
    ...data,
    analyticsSnapshotId: "frozen-one",
    data: [{ ...data.topSessions[0], title }],
    total: 1,
    page: 0,
    pageSize: 25,
    appliedView: { sortBy, search: null },
    viewFingerprint: sortBy,
  });
  mocks.page
    .mockImplementationOnce(
      () =>
        new Promise<AnalyticsPage>((resolve) => {
          resolveFirst = resolve;
        }),
    )
    .mockResolvedValue(reply("Cost from frozen", "cost"));
  const view = render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("button", { name: "查看全部" }));
  const dialog = screen.getByRole("dialog");
  view.rerender(
    <UsageSessions
      data={{ ...data, snapshotVersion: "new", analyticsSnapshotId: "new-two" }}
      active
    />,
  );
  await userEvent.click(
    within(dialog).getByRole("radio", { name: "现价估算" }),
  );
  await within(dialog).findByRole("button", {
    name: "查看会话 Cost from frozen",
  });
  expect(mocks.page.mock.calls.at(-1)?.[0]).toMatchObject({
    analyticsSnapshotId: "frozen-one",
    expectedReceipt: { snapshotVersion: "analysis-v1" },
    sortBy: "cost",
  });
  resolveFirst(reply("Late token leader", "tokens"));
  await waitFor(() =>
    expect(within(dialog).queryByText("Late token leader")).toBeNull(),
  );
  expect(within(dialog).getByText("Cost from frozen")).toBeVisible();
});

it("navigates the hourly heatmap with one tab stop and preserves an observed zero", async () => {
  const data = sessionAnalyticsFixture();
  data.activityByHour = {
    timezone: "local",
    coverage: {
      state: "complete",
      eligibleSessions: 1,
      excludedSessions: 0,
      unquantifiedSourceGaps: false,
      reasons: [],
    },
    cells: Array.from({ length: 168 }, (_, index) => ({
      weekday: Math.floor(index / 24),
      hour: index % 24,
      activeMs: index ? 0 : 60000,
    })),
  };
  const view = render(<UsageTimeInsights data={data} />);
  const first = screen.getByRole("button", { name: "周一 00:00 · 1 分钟" });
  const next = screen.getByRole("button", { name: "周一 01:00 · 0 分钟" });
  expect(first).toHaveAttribute("tabindex", "0");
  expect(next).toHaveAttribute("tabindex", "-1");
  act(() => first.focus());
  await userEvent.keyboard("{ArrowRight}");
  expect(next).toHaveFocus();
  expect(first).toHaveAttribute("tabindex", "-1");
  const newer = {
    ...data,
    activityByHour: {
      ...data.activityByHour,
      cells: data.activityByHour.cells.map((cell) =>
        cell.weekday === 0 && cell.hour === 1
          ? { ...cell, activeMs: 120000 }
          : cell,
      ),
    },
  };
  view.rerender(<UsageTimeInsights data={newer} />);
  expect(
    screen.getByRole("button", { name: "周一 01:00 · 2 分钟" }),
  ).toHaveFocus();
});

it("locates a session outside the compact leaders through its exact manager key", async () => {
  const data = sessionAnalyticsFixture();
  data.analyticsSnapshotId = "scope";
  mocks.page.mockResolvedValue({
    ...data,
    data: [{ ...data.topSessions[0], title: "Outside top" }],
    total: 1,
  });
  const located = vi.fn();
  render(
    <UsageSessions
      data={data}
      active
      location={{ key: "outside-manager", request: 2 }}
      onLocated={located}
    />,
  );
  expect(
    await within(await screen.findByRole("dialog")).findByText("Outside top"),
  ).toBeVisible();
  expect(mocks.page).toHaveBeenCalledWith(
    expect.objectContaining({ managerKey: "outside-manager", pageSize: 1 }),
  );
  expect(located).toHaveBeenCalledWith(2);
});
it("offers a mouse close action for the full ranking and returns focus to its opener", async () => {
  const data = sessionAnalyticsFixture();
  data.analyticsSnapshotId = "ranking";
  mocks.page.mockResolvedValue({ ...data, data: [], total: 0 });
  render(<UsageSessions data={data} active />);
  const opener = screen.getByRole("button", { name: "查看全部" });
  await userEvent.click(opener);
  await userEvent.click(
    within(screen.getByRole("dialog")).getByRole("button", { name: "关闭" }),
  );
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(opener).toHaveFocus();
});
it("labels zero activity separately from a positive duration below one minute", async () => {
  render(
    <UsageDistributions
      distributions={[
        {
          metric: "activeMs",
          edges: [0, 1, 60000],
          counts: [3, 1, 1],
          samples: 5,
          excludedSessions: 0,
          unquantifiedSourceGaps: false,
        },
      ]}
    />,
  );
  await userEvent.click(screen.getByText("会话分布"));
  expect(screen.getByText("0 分钟")).toBeVisible();
  expect(screen.getByLabelText("不足 1 分钟（非零）：1 个会话")).toBeVisible();
});
it("does not round a verifiable positive activity interval down to a displayed zero", () => {
  const data = sessionAnalyticsFixture();
  data.activityByHour = {
    timezone: "local",
    coverage: {
      state: "complete",
      eligibleSessions: 1,
      excludedSessions: 0,
      unquantifiedSourceGaps: false,
      reasons: [],
    },
    cells: [{ weekday: 0, hour: 0, activeMs: 1 }],
  };
  render(<UsageTimeInsights data={data} />);
  expect(
    screen.getByRole("button", { name: "周一 00:00 · 不足 1 秒" }),
  ).toBeVisible();
});

it("shows verified activity from a partial cohort without changing legacy unknown totals", () => {
  const data = Object.assign(sessionAnalyticsFixture(), {
    knownMetrics: { messages: 4, activeMs: 120000, activeDays: 1 },
    metricCoverage: {
      state: "partial",
      eligibleSessions: 1,
      excludedSessions: 2,
      unquantifiedSourceGaps: true,
      reasons: ["messageCoveragePartial"],
    },
  });
  data.totals.messageReason = "messageCoveragePartial";
  data.totals.activityReason = "messageCoveragePartial";
  render(<UsageSessions data={data} active />);
  expect(screen.getByText("2 分钟")).toBeVisible();
  expect(data.totals.activeMs).toBeNull();
});

it("switches to backend cost leaders outside the token top without reordering token rows", async () => {
  const data = sessionAnalyticsFixture();
  Object.assign(data, {
    compactTop: {
      tokens: data.topSessions,
      cost: [
        { ...data.topSessions[0], sessionKey: "cost", title: "Cost leader" },
      ],
      active: [],
    },
  });
  render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("radio", { name: "现价估算" }));
  expect(
    screen.getByRole("button", { name: "查看会话 Cost leader" }),
  ).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "查看会话 Frozen session" }),
  ).toBeNull();
  expect(data.topSessions[0].title).toBe("Frozen session");
});
