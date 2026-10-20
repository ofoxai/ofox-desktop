import { useEffect } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import { queryClient } from "@/lib/query/queryClient";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import type { SessionAnalytics } from "@/types/sessionAnalytics";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: vi.fn() } }));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.ai" }),
}));
const free = {
  knownCostUsd: "0",
  availability: "free" as const,
  unpricedModelIds: [],
  unpricedRecordCount: 0,
  additionalUnitsUnsupported: false,
};
const unavailable = {
  state: "unavailable" as const,
  eligibleSessions: 0,
  excludedSessions: 1,
  unquantifiedSourceGaps: true,
  reasons: ["sourceReadIncomplete"],
};
function noUsage(
  availability: "available" | "partial" | "unavailable" | undefined,
): SessionAnalytics {
  const data = sessionAnalyticsFixture();
  data.methodVersion = "magpie-native-v1";
  data.sourceCoverage = {
    observed: 1,
    read: 0,
    partial: 0,
    failed: 1,
    undated: 0,
    legacyHistoryAvailable: false,
    usageAvailability: availability,
    knownUsageRecords: availability === "unavailable" ? 0 : 1,
  };
  data.totals = {
    ...data.totals,
    sessionCount: availability === "unavailable" ? 0 : 1,
    projectCount: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    totalTokens: 0,
    cacheHitRate: null,
    estimate: free,
    medianTokens: "0",
    p90Tokens: 0,
    quantileSamples: 0,
  };
  data.series = [
    {
      ...data.series[0],
      inputTokens: 0,
      outputTokens: 0,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      totalTokens: 0,
      cacheHitRate: null,
      sessionCount: data.totals.sessionCount,
      estimate: free,
    },
  ];
  data.messageCoverage = unavailable;
  data.activityCoverage = unavailable;
  data.metricCoverage = unavailable;
  data.knownMetrics = null;
  data.topSessions = [];
  data.byProject = [];
  data.byModel = [];
  data.byTool = [];
  return data;
}
function Activate() {
  const shell = useShell();
  useEffect(() => shell.setActive("usage"), [shell.setActive]);
  return null;
}
function Page() {
  return (
    <ShellProvider openSettings={() => undefined}>
      <Activate />
      <UsagePage account={{ user: null } as OfoxAccount} />
    </ShellProvider>
  );
}
function kpi(label: string, root: HTMLElement = document.body) {
  const labels = within(root)
    .getAllByText(label, { selector: "span" })
    .filter((node) => node.parentElement?.querySelector(":scope > b"));
  expect(labels).toHaveLength(1);
  return labels[0].parentElement!.querySelector("b");
}
let data: SessionAnalytics;
beforeEach(() => {
  queryClient.clear();
  localStorage.clear();
  data = noUsage("unavailable");
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.invoke
    .mockReset()
    .mockImplementation(async (command: string, args: any) => {
      if (command === "get_usage_capabilities")
        return {
          queryVersion: 1,
          requestSnapshots: true,
          csv: false,
          projectFilter: true,
          sessionAnalytics: true,
          sessionProjectFilter: true,
          sources: [],
        };
      if (command === "get_session_usage_analytics")
        return { ...data, appliedQuery: args.query };
      const totals = {
        ...data.totals,
        requests: data.sourceCoverage?.knownUsageRecords ?? 0,
        cacheHitRate: 0,
        costUsd: 0,
      };
      if (command === "get_usage_overview")
        return {
          ...data,
          appliedQuery: args.query,
          period: "30d",
          start: args.query.startMs,
          end: args.query.endMs,
          totals,
          granularity: "day",
          series: [{ start: 0, ...totals }],
          byProject: [],
          byTool: [],
          byModel: [],
          unsupportedTools: [],
          lastSyncedAt: null,
        };
      if (command === "get_request_logs")
        return {
          ...data,
          appliedQuery: args.query,
          requestFilters: args.requestFilters,
          data: [],
          total: 0,
          page: 0,
          pageSize: 20,
          snapshotId: "unknown-snapshot",
          summary: totals,
          trend: { granularity: "day", series: [{ start: 0, ...totals }] },
        };
      throw new Error(command);
    });
});

it("does not turn an unreadable overview with no trusted usage into zero Tokens or a free estimate", async () => {
  render(<Page />);
  const panel = await screen.findByRole("tabpanel", { name: "概览" });
  expect(await within(panel).findByText("本地用量暂不可用")).toBeVisible();
  expect(kpi("Tokens", panel)).toHaveTextContent("—");
  expect(within(panel).queryByText("≈$0")).toBeNull();
  expect(within(panel).queryByLabelText(zh.usagePage.chart)).toBeNull();
});

it("does not draw a request zero trend or claim a free total when all usage samples are unavailable", async () => {
  localStorage.setItem("ofox.usageTab", "requests");
  render(<Page />);
  const panel = await screen.findByRole("tabpanel", { name: "明细" });
  expect(await within(panel).findByText("Tokens: —")).toBeVisible();
  expect(within(panel).queryByText(/≈\$0/)).toBeNull();
  expect(within(panel).queryByLabelText(zh.usagePage.chart)).toBeNull();
  expect(within(panel).getByText("本地用量暂不可用")).toBeVisible();
});

it("keeps analysis Tokens, estimates and the empty trend unknown without trusted samples", () => {
  render(<UsageSessions data={data} active />);
  expect(kpi("Tokens")).toHaveTextContent("—");
  expect(screen.queryByText("≈$0")).toBeNull();
  expect(screen.queryByLabelText(zh.usagePage.chart)).toBeNull();
  expect(screen.getByText("本地用量暂不可用")).toBeVisible();
});

it("retains known messages when Token evidence is unavailable", () => {
  data.messageCoverage = {
    ...unavailable,
    state: "partial",
    eligibleSessions: 1,
    excludedSessions: 0,
  };
  data.knownMessages = 7;
  data.series[0].knownMessages = 7;
  render(<UsageSessions data={data} active />);
  expect(kpi("Tokens")).toHaveTextContent("—");
  expect(screen.getByText(/消息 7 · 缓存写入/)).toBeVisible();
  expect(screen.getByRole("tab", { name: "消息数" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
});

it("retains independently known activity and its trend when usage and message samples are unavailable", () => {
  data.activityCoverage = {
    ...unavailable,
    state: "partial",
    eligibleSessions: 1,
    excludedSessions: 0,
  };
  data.knownActiveMs = 90000;
  data.knownActiveDays = 1;
  data.series[0].knownActiveMs = 90000;
  render(<UsageSessions data={data} active />);
  expect(kpi("Tokens")).toHaveTextContent("—");
  expect(kpi("工具活跃时长")).toHaveTextContent("1.5 分钟");
  expect(screen.getByRole("tab", { name: "工具活跃时长" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("tab", { name: "消息数" })).toBeDisabled();
});

it("does not present unavailable Token calendar cells as observed zero", async () => {
  render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("radio", { name: "活动日历" }));
  expect(
    screen.getByRole("button", { name: `${data.series[0].date} · —` }),
  ).toBeVisible();
  expect(screen.queryByText("统计已更新")).toBeNull();
});

it("keeps a wholly unverified calendar session count unknown", async () => {
  render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("radio", { name: "活动日历" }));
  await userEvent.click(screen.getByRole("combobox", { name: "日历指标" }));
  await userEvent.click(screen.getByRole("option", { name: "会话数" }));
  expect(
    screen.getByRole("button", { name: `${data.series[0].date} · —` }),
  ).toBeVisible();
});

it("retains message-only project and tool relations without presenting unobserved zero consumption", async () => {
  data.messageCoverage = {
    ...unavailable,
    state: "partial",
    eligibleSessions: 1,
    excludedSessions: 0,
  };
  data.knownMessages = 7;
  const relation = {
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    totalTokens: 0,
    cacheHitRate: null,
    path: null,
    sessionCount: 1,
    estimate: { ...free, knownCostUsd: null, availability: "unknown" as const },
  };
  data.byProject = [
    { ...relation, id: "message-only-project", label: "Message-only project" },
  ];
  data.byTool = [{ ...relation, id: "codex", label: "Codex" }];
  render(<UsageSessions data={data} active />);
  const projectRow = screen
    .getAllByText("Message-only project")
    .map((node) => node.closest('[role="listitem"]'))
    .find(Boolean) as HTMLElement;
  expect(within(projectRow).getByText("—", { selector: "b" })).toBeVisible();
  expect(within(projectRow).queryByText("0")).toBeNull();
  await userEvent.click(screen.getByRole("radio", { name: "工具" }));
  const toolRow = screen
    .getByText("Codex")
    .closest('[role="listitem"]') as HTMLElement;
  expect(within(toolRow).getByText("—", { selector: "b" })).toBeVisible();
  expect(within(toolRow).queryByText("0")).toBeNull();
});

it.each(["overview", "requests", "sessions"] as const)(
  "does not publish the first %s response priced for a different region",
  async (tab) => {
    data = noUsage("available");
    localStorage.setItem("ofox.usageTab", tab);
    const original = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation(async (command: string, args: any) => {
      const result = await original(command, args);
      if (result.pricingCoverage)
        result.pricingCoverage = { ...result.pricingCoverage, apex: "ofox.io" };
      return result;
    });
    render(<Page />);
    await waitFor(() =>
      expect(screen.queryAllByRole("alert").length).toBeGreaterThan(0),
    );
    expect(screen.queryByText("≈$0")).toBeNull();
    expect(
      screen.queryByRole("button", { name: "查看原始历史记录" }),
    ).toBeNull();
  },
);

it("validates the refresh owner's captured region before publishing an analysis refresh", async () => {
  data = noUsage("available");
  localStorage.setItem("ofox.usageTab", "sessions");
  let reads = 0;
  const original = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "refresh_local_usage" || command === "sync_session_usage")
      return {};
    const result = await original(command, args);
    if (command === "get_session_usage_analytics" && ++reads > 1) {
      result.pricingCoverage = { ...result.pricingCoverage, apex: "ofox.io" };
      result.totals = {
        ...result.totals,
        totalTokens: 999999,
        inputTokens: 999999,
      };
    }
    return result;
  });
  render(<Page />);
  await screen.findByText("≈$0");
  await userEvent.click(screen.getByRole("button", { name: "刷新本机数据" }));
  await waitFor(() =>
    expect(screen.queryAllByRole("alert").length).toBeGreaterThan(0),
  );
  expect(reads).toBe(2);
  expect(kpi("Tokens")).toHaveTextContent("0");
});

it.each(["available", "partial", undefined] as const)(
  "preserves genuine zero and the older backend behavior for availability %s",
  (availability) => {
    const zero = noUsage(availability);
    render(<UsageSessions data={zero} active />);
    expect(kpi("Tokens")).toHaveTextContent("0");
    expect(screen.getByText("≈$0")).toBeVisible();
  },
);

it("preserves a genuine zero overview when a trusted zero-valued event exists", async () => {
  data = noUsage("available");
  render(<Page />);
  const panel = await screen.findByRole("tabpanel", { name: "概览" });
  expect(await within(panel).findByText("≈$0")).toBeVisible();
  expect(kpi("Tokens", panel)).toHaveTextContent("0");
  expect(within(panel).queryByText("本地用量暂不可用")).toBeNull();
});

it("preserves the zero request total and trend for a trusted zero-valued event", async () => {
  data = noUsage("available");
  localStorage.setItem("ofox.usageTab", "requests");
  render(<Page />);
  const panel = await screen.findByRole("tabpanel", { name: "明细" });
  expect(await within(panel).findByText("Tokens: 0")).toBeVisible();
  expect(within(panel).getByText(/≈\$0/)).toBeVisible();
  expect(within(panel).getByLabelText(zh.usagePage.chart)).toBeVisible();
  expect(within(panel).queryByText("本地用量暂不可用")).toBeNull();
});
