import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import { DatePicker } from "@/components/ui/date-picker";
import { formatCalendarDate } from "@/lib/calendarDate";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";

function Picker({ initial = "2024-02-28", range = false }) {
  const [value, setValue] = useState(initial);
  return (
    <DatePicker
      label="开始日期"
      value={value}
      onChange={setValue}
      rangeStart={range ? "2024-02-27" : undefined}
      rangeEnd={range ? "2024-03-02" : undefined}
    />
  );
}

beforeEach(async () => {
  for (const [lang, resource] of Object.entries({ zh, en, ja }))
    i18n.addResourceBundle(lang, "translation", resource, true, true);
  await i18n.changeLanguage("zh");
});

describe("DatePicker", () => {
  it("selects a leap day and highlights the range across month boundaries", async () => {
    const user = userEvent.setup();
    render(<Picker range />);
    await user.click(
      screen.getByRole("button", { name: "为开始日期选择日期" }),
    );
    const calendar = screen.getByRole("grid", { name: "2024年2月" });
    expect(
      within(calendar).getAllByRole("gridcell", { selected: true }),
    ).toHaveLength(5);
    const selected = screen.getByRole("button", { name: /2024年2月28日/ });
    expect(selected).toHaveFocus();
    expect(selected).toHaveAttribute("aria-pressed", "true");
    await user.click(screen.getByRole("button", { name: /2024年2月29日/ }));
    expect(screen.getByLabelText("开始日期")).toHaveValue("2024-02-29");
    expect(screen.queryByRole("grid")).not.toBeInTheDocument();
  });

  it("moves focus across months with arrows and keeps one calendar tab stop", async () => {
    const user = userEvent.setup();
    render(<Picker />);
    await user.click(
      screen.getByRole("button", { name: "为开始日期选择日期" }),
    );
    await user.keyboard("{ArrowRight}{ArrowRight}");
    expect(screen.getByRole("grid", { name: "2024年3月" })).toBeVisible();
    const march = screen.getByRole("button", { name: /2024年3月1日/ });
    expect(march).toHaveFocus();
    expect(
      screen.getByRole("grid").querySelectorAll('button[tabindex="0"]'),
    ).toHaveLength(1);
    await user.tab();
    expect(screen.getByRole("button", { name: "今天" })).toHaveFocus();
    await user.tab({ shift: true });
    expect(march).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.getByLabelText("开始日期")).toHaveValue("2024-03-01");
  });

  it("clamps month and year keyboard jumps to valid leap-year dates", async () => {
    const user = userEvent.setup();
    render(<Picker initial="2024-01-31" />);
    await user.click(
      screen.getByRole("button", { name: "为开始日期选择日期" }),
    );
    await user.keyboard("{PageDown}");
    expect(screen.getByRole("button", { name: /2024年2月29日/ })).toHaveFocus();
    await user.keyboard("{Shift>}{PageDown}{/Shift}");
    expect(screen.getByRole("button", { name: /2025年2月28日/ })).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(screen.getByLabelText("开始日期")).toHaveValue("2025-02-28");
  });

  it("navigates without committing and restores trigger focus on Escape", async () => {
    const user = userEvent.setup();
    render(<Picker />);
    const trigger = screen.getByRole("button", { name: "为开始日期选择日期" });
    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: "下个月" }));
    expect(screen.getByRole("grid", { name: "2024年3月" })).toBeVisible();
    expect(screen.getByLabelText("开始日期")).toHaveValue("2024-02-28");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(trigger).toHaveFocus());
    await user.click(trigger);
    expect(screen.getByRole("grid", { name: "2024年2月" })).toBeVisible();
  });

  it("keeps invalid typed dates for validation and can return to today", async () => {
    const user = userEvent.setup();
    render(<Picker />);
    const input = screen.getByLabelText("开始日期");
    await user.clear(input);
    await user.type(input, "2023-02-29");
    expect(input).toHaveValue("2023-02-29");
    await user.click(
      screen.getByRole("button", { name: "为开始日期选择日期" }),
    );
    await user.click(screen.getByRole("button", { name: "今天" }));
    expect(input).toHaveValue(formatCalendarDate(new Date()));
  });

  it.each([
    ["en", "February 2024", "Next month"],
    ["ja", "2024年2月", "次の月"],
  ])("localizes the calendar in %s", async (language, month, next) => {
    await i18n.changeLanguage(language);
    render(<Picker />);
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: /開始|开始/ }));
    expect(screen.getByRole("grid", { name: month })).toBeVisible();
    expect(screen.getByRole("button", { name: next })).toBeVisible();
  });
});
