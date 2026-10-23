import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageDistributions } from "@/components/usage-overview/UsageDistributions";
import type { AnalyticsDistribution } from "@/types/sessionAnalytics";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});
const tokens: AnalyticsDistribution = {
  metric: "tokens",
  edges: [0, 1000, 10000, 100000, 1000000, 10000000],
  counts: [141, 129, 80, 56, 10, 1],
  samples: 417,
  excludedSessions: 5,
  unquantifiedSourceGaps: false,
};
it("pairs compact ranges with exact counts and shares of the measured cohort", async () => {
  render(<UsageDistributions distributions={[tokens]} />);
  await userEvent.click(screen.getByText("会话分布"));
  const card = screen.getByRole("region", { name: "Tokens" });
  expect(within(card).getByText("已统计 417 个会话")).toBeVisible();
  expect(within(card).getByText("33.8%")).toBeVisible();
  expect(within(card).getByText("0.2%")).toBeVisible();
  expect(within(card).getByText("10M+")).toBeVisible();
  expect(
    within(card).getByRole("listitem", {
      name: "1K 至不足 10K：129 个会话",
    }),
  ).toBeVisible();
  expect(within(card).getByText("5 个会话未统计")).toBeVisible();
});
it("shows short message/minute ranges while preserving exact exclusive boundaries", async () => {
  render(
    <UsageDistributions
      activityLabel="工具活跃时长"
      distributions={[
        {
          ...tokens,
          metric: "messages",
          edges: [0, 1, 6, 16],
          counts: [0, 2, 1, 1],
          samples: 4,
          excludedSessions: 0,
        },
        {
          ...tokens,
          metric: "activeMs",
          edges: [0, 1, 60000, 1860000, 3660000],
          counts: [0, 1, 1, 1, 1],
          samples: 4,
          excludedSessions: 0,
        },
      ]}
    />,
  );
  await userEvent.click(screen.getByText("会话分布"));
  const messages = screen.getByRole("region", { name: "消息数" });
  expect(within(messages).getByText("1–5")).toBeVisible();
  const activity = screen.getByRole("region", { name: "工具活跃时长" });
  expect(within(activity).getByText("31–61 分钟")).toBeVisible();
  expect(
    within(activity).getByLabelText("31 分钟至不足 61 分钟：1 个会话"),
  ).toBeVisible();
  expect(
    within(activity).getByLabelText("不足 1 分钟（非零）：1 个会话"),
  ).toBeVisible();
});
it("keeps an unreadable empty distribution distinct from a measured zero", async () => {
  render(
    <UsageDistributions
      distributions={[
        {
          ...tokens,
          counts: [0, 0, 0, 0, 0, 0],
          samples: 0,
          excludedSessions: 4,
          unquantifiedSourceGaps: true,
        },
      ]}
    />,
  );
  await userEvent.click(screen.getByText("会话分布"));
  expect(screen.getByText("没有有效样本")).toBeVisible();
  expect(screen.queryByText("已统计 0 个会话")).toBeNull();
  expect(screen.getByText(/4 个会话未统计/)).toBeVisible();
  expect(screen.getByText(/部分来源未完整读取/)).toBeVisible();
});
