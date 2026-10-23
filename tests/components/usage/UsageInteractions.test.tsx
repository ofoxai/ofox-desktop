import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import {
  UsageSessionShape,
  UsageSkillInsights,
  UsageToolInsights,
} from "@/components/usage-overview/UsageInteractions";
import type { AnalyticsInteractions } from "@/types/sessionAnalytics";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});
export const interactions: AnalyticsInteractions = {
  coverage: {
    state: "partial",
    eligibleSessions: 125,
    excludedSessions: 2,
    unquantifiedSourceGaps: false,
    reasons: [],
  },
  tools: {
    calls: 2400,
    sessions: 125,
    top: [
      { name: "exec_command", category: "Bash", calls: 2000, sessions: 100 },
      { name: "Read", category: "Read", calls: 400, sessions: 25 },
    ],
    categories: [
      { name: "Bash", category: "Bash", calls: 2000, sessions: 100 },
      { name: "Read", category: "Read", calls: 400, sessions: 25 },
    ],
    weeks: [{ start: "2026-10-12", calls: { Bash: 2000, Read: 400 } }],
  },
  skills: {
    calls: 20,
    count: 1,
    top: [
      {
        name: "support-triage",
        calls: 20,
        sessions: 12,
        last: "2026-10-21",
        agents: { claude: 20 },
        projects: [{ name: "demo", calls: 20 }],
      },
    ],
  },
  autonomy: {
    metric: "toolCallsPerPrompt",
    edges: [0, 1, 3, 6, 11, 21],
    counts: [0, 20, 30, 10, 5, 60],
    samples: 125,
    excludedSessions: 2,
    unquantifiedSourceGaps: false,
  },
};
it("shows full-cohort tool totals, category composition, weekly details and partial coverage", async () => {
  render(<UsageToolInsights data={interactions} />);
  expect(screen.getByRole("heading", { name: "工具使用" })).toBeVisible();
  expect(screen.getByText(/2.4K/)).toBeVisible();
  expect(screen.getByText("2.4K+ 次调用 · 125 个会话")).toBeVisible();
  expect(screen.getByText("exec_command")).toBeVisible();
  expect(screen.getByText(/125 个会话完整统计/)).toBeVisible();
  await userEvent.click(screen.getByText("查看工具数据表"));
  expect(screen.getByRole("table", { name: "每周工具调用" })).toHaveTextContent(
    "2,000",
  );
});
it("renders explicit skills with provenance and handles an older snapshot as unavailable", () => {
  const view = render(<UsageSkillInsights data={interactions} />);
  expect(screen.getByText("support-triage")).toBeVisible();
  expect(screen.getByText(/Claude Code 100%/)).toBeVisible();
  expect(screen.getByText(/demo/)).toBeVisible();
  view.rerender(<UsageSkillInsights />);
  expect(screen.getByText("暂无可用的调用统计")).toBeVisible();
  expect(screen.queryByText("0 次调用")).toBeNull();
});
it("switches the visible shape by keyboard without dropping zero or unknown buckets", async () => {
  render(
    <UsageSessionShape
      distributions={[
        {
          metric: "messages",
          edges: [0, 1, 6, 16, 31, 61, 121],
          counts: [1, 225, 127, 10, 29, 8, 11],
          samples: 411,
          excludedSessions: 2,
          unquantifiedSourceGaps: false,
        },
      ]}
      interactions={interactions}
    />,
  );
  const shape = screen.getByRole("region", { name: "会话形态" });
  expect(within(shape).getByText("225")).toBeVisible();
  expect(within(shape).getByText("0")).toBeVisible();
  within(shape).getByRole("radio", { name: "消息" }).focus();
  await userEvent.keyboard("{End}");
  expect(
    within(shape).getByRole("radio", { name: "工具调用" }),
  ).toHaveAttribute("aria-checked", "true");
  expect(within(shape).getByText("21+")).toBeVisible();
  expect(within(shape).getByText(/每条用户提示的工具调用数/)).toBeVisible();
});

it("does not label an incomplete empty projection as measured zero", () => {
  const data: AnalyticsInteractions = {
    ...interactions,
    coverage: {
      ...interactions.coverage,
      eligibleSessions: 0,
      excludedSessions: 1,
    },
    tools: { calls: 0, sessions: 0, top: [], categories: [], weeks: [] },
    skills: { calls: 0, count: 0, top: [] },
  };
  render(
    <>
      <UsageToolInsights data={data} />
      <UsageSkillInsights data={data} />
    </>,
  );
  expect(screen.getAllByText("暂无可用的调用统计")).toHaveLength(2);
  expect(screen.queryByText("当前范围内暂无调用")).toBeNull();
  expect(screen.queryByText(/0\+ 次调用/)).toBeNull();
});
