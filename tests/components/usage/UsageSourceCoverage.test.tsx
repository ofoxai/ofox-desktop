import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageAnalysisStatus } from "@/components/usage-overview/UsageAnalysisStatus";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

it("distinguishes skipped native sources from message sample exclusions", async () => {
  const data = Object.assign(sessionAnalyticsFixture(), {
    methodVersion: "magpie-native-v1",
    sourceCoverage: {
      observed: 100,
      read: 34,
      partial: 64,
      failed: 2,
      undated: 3,
      legacyHistoryAvailable: true,
    },
    metricCoverage: {
      state: "partial" as const,
      eligibleSessions: 309,
      excludedSessions: 1,
      unquantifiedSourceGaps: true,
      reasons: [],
    },
  });
  const history = vi.fn();
  render(<UsageAnalysisStatus data={data} onSavedHistory={history} />);
  expect(screen.getByText("部分本机记录未计入")).toBeVisible();
  expect(screen.queryByText("有效 309 个会话 · 排除 1 个")).toBeNull();
  await userEvent.click(screen.getByText("部分本机记录未计入"));
  expect(
    screen.getByText("完整读取 34 个来源 · 部分可用 64 个 · 读取失败 2 个"),
  ).toBeVisible();
  expect(
    screen.getByText("消息与活跃样本：309 个有效 · 1 个不可用"),
  ).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "查看原始历史记录" }),
  );
  expect(history).toHaveBeenCalledOnce();
});

it("labels an older backend instead of claiming the new method or zero gaps", () => {
  render(<UsageAnalysisStatus data={sessionAnalyticsFixture()} />);
  expect(screen.getByText("当前后端使用旧统计口径")).toBeVisible();
  expect(screen.queryByText("统计已更新")).toBeNull();
});

it("explains an unproven counter revision without calling verified usage a request failure", async () => {
  const data = sessionAnalyticsFixture();
  data.methodVersion = "magpie-native-v1";
  data.coverage.reasons = ["unprovenCounterRevision"];
  render(<UsageAnalysisStatus data={data} />);
  await userEvent.click(screen.getByText("部分本机记录未计入"));
  expect(
    screen.getByText("累计计数修订缺少可核验证据，部分旧日志未计入。"),
  ).toBeVisible();
  expect(screen.queryByText("部分原生记录无法完整归属或解析。")).toBeNull();
});

it("does not describe unpriced records as missing native usage", () => {
  const data = sessionAnalyticsFixture();
  data.methodVersion = "magpie-native-v1";
  data.coverage.state = "complete";
  data.coverage.reasons = [];
  data.sourceCoverage = {
    observed: 2,
    read: 2,
    partial: 0,
    failed: 0,
    undated: 0,
    legacyHistoryAvailable: false,
  };
  render(<UsageAnalysisStatus data={data} />);
  expect(screen.getByText("统计已更新")).toBeVisible();
  expect(screen.queryByText("部分本机记录未计入")).toBeNull();
});

it("keeps known message counts when runtime activity has no valid samples", async () => {
  const data = Object.assign(sessionAnalyticsFixture(), {
    methodVersion: "magpie-native-v1",
    knownMessages: 7,
    messageCoverage: {
      state: "partial" as const,
      eligibleSessions: 2,
      excludedSessions: 1,
      reasons: [],
      unquantifiedSourceGaps: false,
    },
    activityCoverage: {
      state: "unavailable" as const,
      eligibleSessions: 0,
      excludedSessions: 3,
      reasons: [],
      unquantifiedSourceGaps: false,
    },
    metricCoverage: {
      state: "unavailable" as const,
      eligibleSessions: 0,
      excludedSessions: 3,
      reasons: [],
      unquantifiedSourceGaps: false,
    },
  });
  render(<UsageSessions data={data} active />);
  expect(screen.getByText(/消息 7 · 缓存写入/)).toBeVisible();
  expect(screen.queryByText("消息和活跃不可用")).toBeNull();
  await userEvent.click(screen.getByText("部分本机记录未计入"));
  expect(screen.getByText("消息样本：2 个有效 · 1 个不可用")).toBeVisible();
  expect(screen.getByText("工具活动样本：0 个有效 · 3 个不可用")).toBeVisible();
});

it("keeps known runtime activity while missing messages stay unknown", () => {
  const data = Object.assign(sessionAnalyticsFixture(), {
    methodVersion: "magpie-native-v1",
    knownActiveMs: 120000,
    knownActiveDays: 1,
    knownMetrics: { messages: 99, activeMs: 900000, activeDays: 1 },
    messageCoverage: {
      state: "unavailable" as const,
      eligibleSessions: 0,
      excludedSessions: 3,
      reasons: [],
      unquantifiedSourceGaps: false,
    },
    activityCoverage: {
      state: "partial" as const,
      eligibleSessions: 2,
      excludedSessions: 1,
      reasons: [],
      unquantifiedSourceGaps: false,
    },
    metricCoverage: {
      state: "unavailable" as const,
      eligibleSessions: 0,
      excludedSessions: 3,
      reasons: [],
      unquantifiedSourceGaps: false,
    },
  });
  render(<UsageSessions data={data} active />);
  expect(screen.getByText("2 分钟")).toBeVisible();
  expect(screen.getByText(/消息 — · 缓存写入/)).toBeVisible();
  expect(screen.queryByText(/消息 99/)).toBeNull();
});

it("marks a ranked runtime duration as a known part when some runtime timestamps are missing", async () => {
  const data = sessionAnalyticsFixture();
  data.methodVersion = "magpie-native-v1";
  const partial = {
    ...data.topSessions[0],
    activeMs: 120000,
    reasons: ["activityTimeUnavailable"],
  };
  data.compactTop = { tokens: data.topSessions, cost: [], active: [partial] };
  render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("radio", { name: "工具活跃时长" }));
  const row = screen.getByRole("button", { name: "查看会话 Frozen session" });
  expect(row).toHaveTextContent("2 分钟＋");
});
