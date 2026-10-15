import { useEffect } from "react";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
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
beforeEach(() => {
  queryClient.clear();
  Element.prototype.scrollIntoView = vi.fn();
  localStorage.clear();
  mocks.supported = true;
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
          projectFilter: false,
          sessionAnalytics: mocks.supported,
          sessionProjectFilter: mocks.supported,
          sources: [],
        };
      if (command === "get_session_usage_analytics")
        return { ...sessionAnalyticsFixture(), appliedQuery: args.query };
      if (command === "get_usage_overview") {
        const fixture = sessionAnalyticsFixture();
        return {
          ...fixture,
          appliedQuery: args.query,
          period: "30d",
          start: args.query.startMs,
          end: args.query.endMs,
          granularity: "day",
          totals: { ...fixture.totals, requests: 3, costUsd: 0 },
          series: [],
          byTool: [],
          byModel: [],
          unsupportedTools: [],
          lastSyncedAt: null,
        };
      }
      if (command === "sync_session_usage") return {};
      throw new Error(command);
    });
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
      expect(mocks.invoke).toHaveBeenCalledWith("sync_session_usage"),
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
  await userEvent.click(screen.getByRole("combobox", { name: "趋势指标" }));
  await userEvent.click(screen.getByRole("option", { name: "输出 Tokens" }));
  const trend = screen.getByRole("combobox", { name: "趋势指标" });
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
