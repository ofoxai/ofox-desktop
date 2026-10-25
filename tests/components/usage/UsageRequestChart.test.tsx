import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageRequestChart } from "@/components/usage-overview/UsageRequestChart";
import { requestDashboardFixture } from "./requestDashboardFixture";

beforeEach(() => {
  localStorage.removeItem("ofox.usageRequests.metric");
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
});

it("falls back to Token for the removed speed preference and exposes only model/Agent grouping", () => {
  localStorage.setItem("ofox.usageRequests.metric", "speed");
  render(
    <UsageRequestChart
      dashboard={requestDashboardFixture()}
      selection={undefined}
      onGroup={vi.fn()}
      onDay={vi.fn()}
    />,
  );
  expect(screen.getByRole("radio", { name: "Token" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  expect(
    screen.getAllByRole("radio").map((radio) => radio.textContent),
  ).toEqual(["Token", "费用", "请求", "模型", "Agent"]);
});

it("restores a focused ranking item into the visible scroll area", () => {
  const dashboard = requestDashboardFixture(20);
  render(
    <UsageRequestChart
      dashboard={dashboard}
      selection={undefined}
      onGroup={vi.fn()}
      onDay={vi.fn()}
      focusGroupId="fixture-model-1"
    />,
  );
  const item = screen.getByRole("button", { name: "筛选模型 fixture-model-1" });
  expect(item).toHaveFocus();
  expect(item.scrollIntoView).toHaveBeenCalledWith({
    block: "nearest",
    inline: "nearest",
  });
});

it("scales a sub-dollar leader to a full ranking bar", async () => {
  render(
    <UsageRequestChart
      dashboard={requestDashboardFixture(7)}
      selection={undefined}
      onGroup={vi.fn()}
      onDay={vi.fn()}
    />,
  );
  await userEvent.click(screen.getByRole("radio", { name: "费用" }));
  const first = screen.getAllByRole("button", { name: /筛选模型/ })[0];
  const fill = first.querySelector("div.h-full") as HTMLElement;
  expect(fill.style.width).toBe("100%");
});

it("does not label unpriced Other as a zero-dollar cost", async () => {
  render(
    <UsageRequestChart
      dashboard={requestDashboardFixture()}
      selection={undefined}
      onGroup={vi.fn()}
      onDay={vi.fn()}
    />,
  );
  await userEvent.click(screen.getByRole("radio", { name: "费用" }));
  const chart = screen.getByRole("img");
  fireEvent(
    chart,
    new MouseEvent("pointermove", { bubbles: true, clientX: 150 }),
  );
  const other = within(screen.getByRole("tooltip")).getByText(
    "其他",
  ).parentElement!;
  expect(other).toHaveTextContent("—");
  expect(other).not.toHaveTextContent("$0");
});
