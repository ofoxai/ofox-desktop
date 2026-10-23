import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageBars } from "@/components/usage-overview/UsageBars";
import type { UsageOverview } from "@/lib/api/usageOverview";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
});
it("offers metric switching and the same exact values to keyboard users", async () => {
  const overview = {
    granularity: "day",
    series: [
      {
        start: 1791993600000,
        requests: 3,
        inputTokens: 10,
        outputTokens: 20,
        cacheReadTokens: 90,
        cacheWriteTokens: 0,
        costUsd: 0.1,
      },
    ],
  } as UsageOverview;
  render(<UsageBars overview={overview} />);
  expect(screen.getByRole("button", { name: /30 Tokens/ })).toBeVisible();
  await userEvent.click(screen.getByRole("combobox", { name: "趋势指标" }));
  await userEvent.click(screen.getByRole("option", { name: "输出 Tokens" }));
  expect(screen.getByRole("button", { name: /20 输出 Tokens/ })).toBeVisible();
  await userEvent.click(screen.getByText("查看趋势数据表"));
  expect(screen.getByRole("table")).toHaveTextContent("20");
});

it("labels input and output without a theme-dependent light/dark claim", () => {
  render(
    <UsageBars
      overview={{ granularity: "day", series: [] } as unknown as UsageOverview}
    />,
  );
  expect(screen.queryByText(/浅色：|深色：/)).toBeNull();
  expect(screen.getByText("新输入")).toBeVisible();
  expect(screen.getByText("输出")).toBeVisible();
});

it("distinguishes repeated local hours at a daylight-saving fold", () => {
  const format = Date.prototype.toLocaleString;
  const spy = vi
    .spyOn(Date.prototype, "toLocaleString")
    .mockImplementation(function (this: Date, locales, options) {
      return format.call(this, locales, {
        ...options,
        timeZone: "America/New_York",
      });
    });
  try {
    render(
      <UsageBars
        analysis
        overview={{
          granularity: "hour",
          series: ["2026-11-01T05:00:00Z", "2026-11-01T06:00:00Z"].map(
            (at) => ({
              start: Date.parse(at),
              inputTokens: 10,
              outputTokens: 0,
            }),
          ),
        }}
      />,
    );
    expect(screen.getByRole("heading", { name: "每小时趋势" })).toBeVisible();
    expect(
      screen.getByRole("button", { name: /GMT-4.*10 Tokens/ }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: /GMT-5.*10 Tokens/ }),
    ).toBeVisible();
  } finally {
    spy.mockRestore();
  }
});

it("offers inline analysis metrics and skips unavailable message/activity tabs by keyboard", async () => {
  render(
    <UsageBars
      analysis
      unavailableMetrics={["messages", "active"]}
      overview={{
        granularity: "day",
        series: [
          {
            start: 1791993600000,
            inputTokens: 10,
            outputTokens: 20,
            estimate: undefined,
          },
        ],
      }}
    />,
  );
  const tokens = screen.getByRole("tab", { name: "Tokens" });
  expect(screen.getByRole("tab", { name: "消息数" })).toBeDisabled();
  tokens.focus();
  await userEvent.keyboard("{ArrowRight}");
  expect(screen.getByRole("tab", { name: "输出 Tokens" })).toHaveFocus();
  expect(screen.getByRole("button", { name: /20 输出 Tokens/ })).toBeVisible();
  await userEvent.keyboard("{End}");
  expect(screen.getByRole("tab", { name: "现价估算" })).toHaveFocus();
  expect(screen.getByRole("tabpanel")).toBeVisible();
});

it("keeps exact keyboard values and missing days when a long trend becomes continuous", async () => {
  const series = Array.from({ length: 40 }, (_, index) => ({
    start: 1791993600000 + index * 86400000,
    inputTokens: index,
    outputTokens: 1,
    messages: index === 2 ? null : index,
    estimate: undefined,
  }));
  render(
    <UsageBars
      analysis
      selectedMetric="messages"
      overview={{ granularity: "day", series }}
    />,
  );
  const points = screen
    .getAllByRole("button")
    .filter((button) => button.getAttribute("aria-label")?.includes(" · "));
  points[0].focus();
  await userEvent.keyboard("{ArrowRight}{ArrowRight}");
  expect(points[2]).toHaveFocus();
  expect(points[2]).toHaveAccessibleName(/— 消息数/);
  await userEvent.keyboard("{End}");
  expect(points[39]).toHaveFocus();
  expect(points[39]).toHaveAccessibleName(/39 消息数/);
  await userEvent.click(screen.getByText("查看趋势数据表"));
  expect(screen.getByRole("table")).toHaveTextContent("39 消息数");
});
