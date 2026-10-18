import { useEffect } from "react";
import {
  render,
  screen,
  waitFor,
  fireEvent,
  within,
  act,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { queryClient } from "@/lib/query/queryClient";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  supported: true,
  sharedProjects: false,
  apex: "ofox.io",
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: vi.fn() } }));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: mocks.apex }),
}));
const account = { user: null } as OfoxAccount;
function Controls() {
  const shell = useShell();
  useEffect(() => shell.setActive("usage"), [shell.setActive]);
  return <button onClick={() => shell.setActive("tools")}>hide-page</button>;
}
function Page() {
  return (
    <ShellProvider openSettings={() => undefined}>
      <Controls />
      <UsagePage account={account} />
    </ShellProvider>
  );
}

it("opens original history from folded data status with its exact query and returns to effective records by keyboard", async () => {
  mocks.sharedProjects = true;
  localStorage.setItem("ofox.usageTab", "sessions");
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command: string, args: any) => {
    const result = await normal(command, args);
    if (command === "get_session_usage_analytics") {
      result.methodVersion = "magpie-native-v1";
      result.sourceCoverage = {
        observed: 2,
        read: 2,
        partial: 0,
        failed: 0,
        undated: 0,
        legacyHistoryAvailable: true,
      };
    }
    return result;
  });
  render(<Page />);
  await userEvent.click(
    await screen.findByRole("combobox", { name: "项目筛选" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "project" }));
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByText("部分本机记录未计入"));
  const history = screen.getByRole("button", { name: "查看原始历史记录" });
  history.focus();
  await userEvent.keyboard("{Enter}");
  const back = await screen.findByRole("button", { name: "返回有效统计" });
  expect(back).toHaveFocus();
  expect(screen.getByText(/原始历史记录仅供核对/)).toBeVisible();
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_request_logs",
      expect.objectContaining({
        query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
        requestFilters: expect.objectContaining({
          recordBasis: "savedHistory",
        }),
      }),
    ),
  );
  const audit = mocks.invoke.mock.calls.find(
    ([name, args]) =>
      name === "get_request_logs" &&
      args.requestFilters?.recordBasis === "savedHistory",
  )![1];
  const analytics = mocks.invoke.mock.calls
    .filter(([name]) => name === "get_session_usage_analytics")
    .at(-1)![1];
  expect(audit.query).toEqual(analytics.query);
  await userEvent.keyboard("{Enter}");
  await waitFor(() =>
    expect(
      screen.getByRole("textbox", { name: "搜索请求、模型或供应商" }),
    ).toHaveFocus(),
  );
  expect(screen.queryByText(/原始历史记录仅供核对/)).toBeNull();
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_request_logs",
      expect.objectContaining({ requestFilters: {} }),
    ),
  );
});

it("does not offer a stale analysis history jump under newly selected filters", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "get_session_usage_analytics" && args.query.toolIds.length)
      return new Promise(() => undefined);
    const result = await normal(command, args);
    if (command === "get_session_usage_analytics") {
      result.methodVersion = "magpie-native-v1";
      result.sourceCoverage = {
        observed: 1,
        read: 1,
        partial: 0,
        failed: 0,
        undated: 0,
        legacyHistoryAvailable: true,
      };
    }
    return result;
  });
  render(<Page />);
  await userEvent.click(await screen.findByText("部分本机记录未计入"));
  expect(
    screen.getByRole("button", { name: "查看原始历史记录" }),
  ).toBeVisible();
  await userEvent.click(
    within(screen.getByRole("group", { name: "工具筛选" })).getByRole(
      "button",
      { name: "Codex" },
    ),
  );
  expect(screen.queryByRole("button", { name: "查看原始历史记录" })).toBeNull();
});

it("shows native cache rebuild progress and cancels only its exact operation", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const normal = mocks.invoke.getMockImplementation()!;
  let progress = {
    operationId: "rebuild-a",
    state: "running",
    completedSources: 2,
    totalSources: 5,
    errorCode: null,
  };
  mocks.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "get_usage_rebuild_progress") return progress;
    if (command === "cancel_usage_rebuild") {
      expect(args).toEqual({ operationId: "rebuild-a" });
      progress = { ...progress, state: "cancelling" };
      return progress;
    }
    const result = await normal(command, args);
    if (command === "get_usage_capabilities") result.nativeRebuild = true;
    return result;
  });
  render(<Page />);
  expect(
    await screen.findByText("正在重建 Codex 本机统计：2 / 5 个来源"),
  ).toBeVisible();
  const cancel = screen.getByRole("button", { name: "取消重建" });
  cancel.focus();
  await userEvent.keyboard("{Enter}");
  expect(
    await screen.findByText("正在停止后续重建，保留原有统计…"),
  ).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("cancel_usage_rebuild", {
    operationId: "rebuild-a",
  });
  expect(
    mocks.invoke.mock.calls.filter(([name]) => name === "sync_session_usage"),
  ).toHaveLength(0);
});
beforeEach(() => {
  queryClient.clear();
  Element.prototype.scrollIntoView = vi.fn();
  localStorage.clear();
  mocks.supported = true;
  mocks.sharedProjects = false;
  mocks.apex = "ofox.io";
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  mocks.invoke
    .mockReset()
    .mockImplementation(async (command: string, args: any) => {
      if (command === "get_usage_capabilities")
        return {
          queryVersion: 1,
          requestSnapshots: true,
          csv: true,
          projectFilter: mocks.sharedProjects,
          sessionAnalytics: mocks.supported,
          sessionProjectFilter: mocks.supported,
          localRefresh: true,
          sources: [],
        };
      if (command === "get_session_usage_analytics")
        return {
          ...sessionAnalyticsFixture(),
          appliedQuery: args.query,
          pricingCoverage: {
            ...sessionAnalyticsFixture().pricingCoverage,
            apex: mocks.apex,
          },
        };
      if (command === "get_usage_overview") {
        const fixture = sessionAnalyticsFixture();
        return {
          ...fixture,
          pricingCoverage: { ...fixture.pricingCoverage, apex: mocks.apex },
          appliedQuery: args.query,
          period: "30d",
          start: args.query.startMs,
          end: args.query.endMs,
          granularity: "day",
          totals: { ...fixture.totals, requests: 3, costUsd: 0 },
          series: [],
          byTool: [],
          byModel: [],
          byProject: mocks.sharedProjects ? fixture.byProject : undefined,
          attribution: mocks.sharedProjects
            ? {
                sessionCount: 3,
                projectCount: 1,
                unassociatedRecords: 1,
                partial: true,
              }
            : undefined,
          unsupportedTools: [],
          lastSyncedAt: null,
        };
      }
      if (command === "get_request_logs") {
        const fixture = sessionAnalyticsFixture();
        return {
          ...fixture,
          pricingCoverage: { ...fixture.pricingCoverage, apex: mocks.apex },
          appliedQuery: args.query,
          requestFilters: args.requestFilters,
          snapshotId: "project-snapshot",
          data: [],
          total: 0,
          page: args.page,
          pageSize: args.pageSize,
        };
      }
      if (command === "sync_session_usage" || command === "refresh_local_usage")
        return {};
      throw new Error(command);
    });
});
it("shares a supported project condition across analysis, overview and requests until reset", async () => {
  mocks.sharedProjects = true;
  localStorage.setItem("ofox.usageTab", "sessions");
  render(<Page />);
  const projectMenu = await screen.findByRole("combobox", { name: "项目筛选" });
  await userEvent.click(projectMenu);
  await userEvent.click(screen.getByRole("option", { name: "project" }));
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("get_session_usage_analytics", {
      query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
    }),
  );
  await userEvent.click(screen.getByRole("tab", { name: "概览" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_usage_overview",
      expect.objectContaining({
        query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
      }),
    ),
  );
  expect(screen.queryByText(/切换后已清除/)).toBeNull();
  await userEvent.click(screen.getByRole("tab", { name: "明细" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_request_logs",
      expect.objectContaining({
        query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
      }),
    ),
  );
  await userEvent.click(screen.getAllByRole("button", { name: "重置条件" })[0]);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_request_logs",
      expect.objectContaining({
        query: expect.objectContaining({ projectIds: [] }),
      }),
    ),
  );
});
it("offers shared project filters on first overview visit and keeps unknown statistics explicit", async () => {
  mocks.sharedProjects = true;
  render(<Page />);
  expect(
    await screen.findByRole("combobox", { name: "项目筛选" }),
  ).toBeVisible();
  expect(await screen.findByText("已识别会话数")).toBeVisible();
  expect(screen.getByText("已识别项目数")).toBeVisible();
  expect(screen.getByText(/1 条采集记录无法关联会话/)).toBeVisible();
  const rank = screen.getByRole("button", { name: "project" });
  await userEvent.click(rank);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_usage_overview",
      expect.objectContaining({
        query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
      }),
    ),
  );
});
it("keeps the analysis entry unavailable until the backend explicitly advertises it", async () => {
  mocks.supported = false;
  localStorage.setItem("ofox.usageTab", "sessions");
  render(<Page />);
  await screen.findByRole("tab", { name: "概览" });
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("get_usage_capabilities"),
  );
  expect(screen.queryByRole("tab", { name: "会话分析" })).toBeNull();
  expect(
    mocks.invoke.mock.calls.some(
      ([name]) => name === "get_session_usage_analytics",
    ),
  ).toBe(false);
});
it("restores analysis, scopes projects to analysis, and clears them before returning to overview", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  render(<Page />);
  expect(
    await screen.findByRole("tab", { name: "会话分析", selected: true }),
  ).toBeVisible();
  await screen.findByText(/有效样本 2.*排除 1/);
  await userEvent.click(
    screen.getByRole("combobox", { name: "项目筛选（仅会话分析）" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "project" }));
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("get_session_usage_analytics", {
      query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
    }),
  );
  await userEvent.click(screen.getByRole("tab", { name: "概览" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_usage_overview",
      expect.objectContaining({
        query: expect.objectContaining({ projectIds: [] }),
      }),
    ),
  );
  expect(
    screen.getByText(/项目条件仅适用于会话分析，切换后已清除/),
  ).toBeVisible();
});
it("refreshes analysis only while visible, without polling account balance", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const interval = vi.spyOn(window, "setInterval");
  try {
    render(<Page />);
    await screen.findByText(/有效样本 2.*排除 1/);
    const tick = interval.mock.calls.find(
      (call) => call[1] === 30_000,
    )![0] as () => void;
    mocks.invoke.mockClear();
    tick();
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("refresh_local_usage"),
    );
    await waitFor(() =>
      expect(
        mocks.invoke.mock.calls.some(
          ([name]) => name === "get_session_usage_analytics",
        ),
      ).toBe(true),
    );
    expect(
      mocks.invoke.mock.calls.some(([name]) => name.startsWith("ofox_")),
    ).toBe(false);
    fireEvent.click(screen.getByText("hide-page"));
    mocks.invoke.mockClear();
    tick();
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(mocks.invoke).not.toHaveBeenCalled();
  } finally {
    interval.mockRestore();
  }
});

it("toggles a project ranking off on a second click", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  render(<Page />);
  const project = await screen.findByRole("button", { name: "project" });
  await userEvent.click(project);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("get_session_usage_analytics", {
      query: expect.objectContaining({ projectIds: ["/fixture/project"] }),
    }),
  );
  await userEvent.click(await screen.findByRole("button", { name: "project" }));
  expect(
    screen.getByRole("combobox", { name: "项目筛选（仅会话分析）" }),
  ).toHaveTextContent("全部");
});

it("keeps a frozen analysis detail open while a refreshed snapshot is loading", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const interval = vi.spyOn(window, "setInterval");
  render(<Page />);
  await userEvent.click(
    await screen.findByRole("button", { name: "查看会话 Frozen session" }),
  );
  const previousReads = mocks.invoke.mock.calls.filter(
    ([name]) => name === "get_session_usage_analytics",
  ).length;
  const normal = mocks.invoke.getMockImplementation()!;
  let release!: (data: unknown) => void;
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_session_usage_analytics"
      ? new Promise((resolve) => {
          release = resolve;
        })
      : normal(command, args),
  );
  // Use the same visible-page coordinator as an automatic refresh.
  const tick = interval.mock.calls.find(
    (call) => call[1] === 30_000,
  )![0] as () => void;
  interval.mockRestore();
  tick();
  await waitFor(() => expect(release).toBeTypeOf("function"));
  expect(screen.getByRole("dialog")).toBeVisible();
  expect(screen.getByText(/有效样本 2.*排除 1/)).toBeVisible();
  const latest = mocks.invoke.mock.calls
    .filter(([name]) => name === "get_session_usage_analytics")
    .at(-1)![1].query;
  const newer = sessionAnalyticsFixture();
  newer.snapshotVersion = "new-version";
  newer.topSessions[0].models[0].model = "new-model";
  release({ ...newer, appliedQuery: latest });
  await waitFor(() =>
    expect(screen.getByText(/有效样本 2.*排除 1/)).toBeVisible(),
  );
  expect(screen.getByRole("dialog")).toHaveTextContent("model-a");
  expect(screen.getByRole("dialog")).not.toHaveTextContent("new-model");
  expect(
    mocks.invoke.mock.calls.filter(
      ([name]) => name === "get_session_usage_analytics",
    ),
  ).toHaveLength(previousReads + 1);
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "查看会话 Frozen session" }),
    ).toHaveFocus(),
  );
});

it("closes old pricing details when the resolved region changes", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const view = render(<Page />);
  await userEvent.click(
    await screen.findByRole("button", { name: "查看会话 Frozen session" }),
  );
  mocks.apex = "ofox.ai";
  view.rerender(<Page />);
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});

it("keeps the old snapshot and chart state when the refreshed analysis read fails, with one read", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const interval = vi.spyOn(window, "setInterval");
  render(<Page />);
  await screen.findByText(/有效样本 2.*排除 1/);
  await userEvent.click(screen.getByRole("tab", { name: "输出 Tokens" }));
  const trend = screen.getByRole("tab", { name: "输出 Tokens" });
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockClear();
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_session_usage_analytics"
      ? Promise.reject(new Error("usage.analytics.fixtureReadFailed"))
      : normal(command, args),
  );
  const tick = interval.mock.calls.find(
    (call) => call[1] === 30_000,
  )![0] as () => void;
  interval.mockRestore();
  tick();
  await screen.findByRole("alert");
  expect(screen.getByText(/有效样本 2.*排除 1/)).toBeVisible();
  expect(trend.isConnected).toBe(true);
  expect(trend).toHaveTextContent("输出");
  expect(
    mocks.invoke.mock.calls.filter(
      ([name]) => name === "get_session_usage_analytics",
    ),
  ).toHaveLength(1);
});
it.each(["hidden", "tabRoundtrip"])(
  "discards a late refresh snapshot after its owner changes (%s)",
  async (change) => {
    localStorage.setItem("ofox.usageTab", "sessions");
    const interval = vi.spyOn(window, "setInterval");
    render(<Page />);
    await screen.findByText(/有效样本 2.*排除 1/);
    let release!: (data: unknown) => void;
    const normal = mocks.invoke.getMockImplementation()!;
    mocks.invoke.mockImplementation((command: string, args: any) =>
      command === "get_session_usage_analytics"
        ? new Promise((resolve) => {
            release = resolve;
          })
        : normal(command, args),
    );
    const tick = interval.mock.calls.find(
      (call) => call[1] === 30_000,
    )![0] as () => void;
    interval.mockRestore();
    tick();
    await waitFor(() => expect(release).toBeTypeOf("function"));
    if (change === "hidden") fireEvent.click(screen.getByText("hide-page"));
    else {
      await userEvent.click(screen.getByRole("tab", { name: "概览" }));
      await userEvent.click(screen.getByRole("tab", { name: "会话分析" }));
    }
    const query = mocks.invoke.mock.calls
      .filter(([name]) => name === "get_session_usage_analytics")
      .at(-1)![1].query;
    const newer = sessionAnalyticsFixture();
    newer.topSessions[0].title = "late-new-snapshot";
    release({ ...newer, appliedQuery: query });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(screen.queryByText("late-new-snapshot")).toBeNull();
  },
);

it("never calls analysis from manual refresh while capability negotiation is pending", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  let resolveCapabilities!: (data: unknown) => void;
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_usage_capabilities"
      ? new Promise((resolve) => {
          resolveCapabilities = resolve;
        })
      : normal(command, args),
  );
  render(<Page />);
  await waitFor(() => expect(resolveCapabilities).toBeTypeOf("function"));
  await userEvent.click(screen.getByRole("button", { name: "刷新本机数据" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("sync_session_usage"),
  );
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(
    mocks.invoke.mock.calls.some(
      ([name]) => name === "get_session_usage_analytics",
    ),
  ).toBe(false);
  resolveCapabilities({
    queryVersion: 1,
    requestSnapshots: false,
    csv: false,
    projectFilter: false,
    sessionAnalytics: false,
    sources: [],
  });
});

it("keeps ranking-selected filters visible without jumping and removes only the chosen condition", async () => {
  mocks.sharedProjects = true;
  localStorage.setItem("ofox.usageTab", "sessions");
  render(<Page />);
  await userEvent.click(
    await screen.findByRole("combobox", { name: "模型筛选" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "model-a" }));
  await userEvent.keyboard("{Escape}");
  await userEvent.click(
    within(screen.getByRole("group", { name: "工具筛选" })).getByRole(
      "button",
      { name: "Codex" },
    ),
  );
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_session_usage_analytics",
      {
        query: expect.objectContaining({
          toolIds: ["codex"],
          modelIds: ["model-a"],
        }),
      },
    ),
  );
  vi.mocked(Element.prototype.scrollIntoView).mockClear();
  const project = screen.getByRole("button", { name: "project" });
  await userEvent.click(project);
  const active = within(screen.getByRole("group", { name: "已选筛选" }));
  expect(
    active.getByRole("button", { name: "移除项目筛选：project" }),
  ).toBeVisible();
  expect(project).toHaveFocus();
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  await userEvent.click(
    active.getByRole("button", { name: "移除模型筛选：model-a" }),
  );
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_session_usage_analytics",
      {
        query: expect.objectContaining({
          toolIds: ["codex"],
          modelIds: [],
          projectIds: ["/fixture/project"],
        }),
      },
    ),
  );
  expect(
    active.getByRole("button", { name: "移除工具筛选：Codex" }),
  ).toBeVisible();
  expect(
    active.getByRole("button", { name: "移除项目筛选：project" }),
  ).toBeVisible();
});

it("preserves the frozen analysis DOM and focus while a filtered request is delayed, and labels its original scope", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  render(<Page />);
  const project = await screen.findByRole("button", { name: "project" });
  const normal = mocks.invoke.getMockImplementation()!;
  let release!: (data: unknown) => void;
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_session_usage_analytics"
      ? new Promise((resolve) => {
          release = resolve;
        })
      : normal(command, args),
  );
  await userEvent.click(project);
  expect(screen.getByRole("button", { name: "project" })).toBe(project);
  expect(project).toHaveFocus();
  expect(screen.getByRole("tabpanel", { name: "会话分析" })).toHaveAttribute(
    "aria-busy",
    "true",
  );
  expect(
    screen.getByText("正在更新筛选结果，暂时显示上次结果。", { exact: false }),
  ).toHaveTextContent("项目筛选: 全部项目");
  const args = mocks.invoke.mock.calls
    .filter(([command]) => command === "get_session_usage_analytics")
    .at(-1)![1];
  await act(async () =>
    release(await normal("get_session_usage_analytics", args)),
  );
  await waitFor(() =>
    expect(screen.getByRole("tabpanel", { name: "会话分析" })).toHaveAttribute(
      "aria-busy",
      "false",
    ),
  );
  expect(
    screen.queryByText("正在更新筛选结果，暂时显示上次结果。", {
      exact: false,
    }),
  ).toBeNull();
  expect(screen.getByRole("button", { name: "project" })).toBe(project);
});

it("keeps failed-filter results clearly previous and never carries their prices across apex changes", async () => {
  localStorage.setItem("ofox.usageTab", "sessions");
  const view = render(<Page />);
  const project = await screen.findByRole("button", { name: "project" });
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_session_usage_analytics"
      ? Promise.reject(new Error("fixture filter failed"))
      : normal(command, args),
  );
  await userEvent.click(project);
  expect(
    await screen.findByText("当前条件读取失败，仍显示上次结果。", {
      exact: false,
    }),
  ).toHaveTextContent("项目筛选: 全部项目");
  expect(screen.getByRole("button", { name: "project" })).toBe(project);
  expect(screen.getByRole("tabpanel", { name: "会话分析" })).toHaveAttribute(
    "aria-busy",
    "false",
  );
  mocks.apex = "ofox.ai";
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_session_usage_analytics"
      ? new Promise(() => undefined)
      : normal(command, args),
  );
  view.rerender(<Page />);
  expect(screen.queryByText("Frozen session")).toBeNull();
  expect(
    screen.queryByText("当前条件读取失败，仍显示上次结果。", { exact: false }),
  ).toBeNull();
});

it("removes one project from a multi-selected ranking without altering other dimensions", async () => {
  mocks.sharedProjects = true;
  localStorage.setItem("ofox.usageTab", "sessions");
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command: string, args: any) => {
    const result = await normal(command, args);
    if (command === "get_session_usage_analytics") {
      result.options.projects.push({
        id: "/fixture/second",
        label: "second",
        path: "/fixture/second",
      });
    }
    return result;
  });
  render(<Page />);
  await userEvent.click(
    await screen.findByRole("combobox", { name: "项目筛选" }),
  );
  await userEvent.click(screen.getByRole("option", { name: "project" }));
  await userEvent.click(screen.getByRole("option", { name: "second" }));
  await userEvent.keyboard("{Escape}");
  await userEvent.click(
    within(screen.getByRole("group", { name: "工具筛选" })).getByRole(
      "button",
      { name: "Codex" },
    ),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "移除 project 筛选" }),
    ).toBeVisible(),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "移除 project 筛选" }),
  );
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenLastCalledWith(
      "get_session_usage_analytics",
      {
        query: expect.objectContaining({
          toolIds: ["codex"],
          projectIds: ["/fixture/second"],
        }),
      },
    ),
  );
});
