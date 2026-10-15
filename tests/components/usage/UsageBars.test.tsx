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
