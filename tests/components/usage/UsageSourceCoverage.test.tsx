import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
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
