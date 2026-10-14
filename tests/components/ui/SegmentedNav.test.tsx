import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it, vi } from "vitest";
import { SegmentedControl, SegmentedNav } from "@/components/ui/segmented";

type Page = "tools" | "sessions" | "usage";

const PAGES = [
  { value: "tools" as const, label: "工具" },
  { value: "sessions" as const, label: "会话" },
  { value: "usage" as const, label: "用量" },
];

function Nav({ onPick }: { onPick?: (page: Page) => void }) {
  const [page, setPage] = useState<Page>("tools");
  return (
    <SegmentedNav<Page>
      aria-label="Pages"
      items={PAGES}
      value={page}
      onChange={(next) => {
        onPick?.(next);
        setPage(next);
      }}
    />
  );
}

describe("SegmentedNav", () => {
  it("is a tablist where only the active tab is selected and in the tab order", () => {
    render(<Nav />);
    expect(screen.getByRole("tablist", { name: "Pages" })).toBeInTheDocument();
    const [tools, sessions, usage] = screen.getAllByRole("tab");
    expect(tools).toHaveAccessibleName("工具");
    expect(tools).toHaveAttribute("aria-selected", "true");
    expect(tools).toHaveAttribute("tabindex", "0");
    for (const tab of [sessions, usage]) {
      expect(tab).toHaveAttribute("aria-selected", "false");
      expect(tab).toHaveAttribute("tabindex", "-1");
    }
  });

  it("draws 24px, 12.5px tabs in the control colour, the active one in ink on a white thumb", () => {
    render(<Nav />);
    const tools = screen.getByRole("tab", { name: "工具" });
    const sessions = screen.getByRole("tab", { name: "会话" });
    expect(tools).toHaveClass("h-6", "text-nav", "text-ink");
    expect(sessions).toHaveClass("h-6", "text-nav", "text-ink-ctl");
    const thumb = tools.querySelector("[data-seg-thumb]");
    expect(thumb).toHaveClass(
      "bg-seg-thumb",
      "shadow-[0_1px_2px_rgba(0,0,0,.14),0_0_0_.5px_rgba(0,0,0,.08)]",
    );
    expect(sessions.querySelector("[data-seg-thumb]")).toBeNull();
  });

  it("selects the tab clicked", async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(<Nav onPick={onPick} />);
    await user.click(screen.getByRole("tab", { name: "用量" }));
    expect(onPick).toHaveBeenCalledWith("usage");
    expect(screen.getByRole("tab", { name: "用量" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("moves focus and selection with the arrow keys, wrapping, and with Home and End", async () => {
    const user = userEvent.setup();
    render(<Nav />);
    screen.getByRole("tab", { name: "工具" }).focus();

    await user.keyboard("{ArrowRight}");
    const sessions = screen.getByRole("tab", { name: "会话" });
    expect(sessions).toHaveFocus();
    expect(sessions).toHaveAttribute("aria-selected", "true");
    expect(sessions).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("tab", { name: "工具" })).toHaveAttribute(
      "tabindex",
      "-1",
    );

    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(screen.getByRole("tab", { name: "用量" })).toHaveFocus();
    expect(screen.getByRole("tab", { name: "用量" })).toHaveAttribute(
      "aria-selected",
      "true",
    );

    await user.keyboard("{Home}");
    expect(screen.getByRole("tab", { name: "工具" })).toHaveFocus();
    await user.keyboard("{End}");
    expect(screen.getByRole("tab", { name: "用量" })).toHaveFocus();
    await user.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "工具" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });
});

describe("SegmentedControl", () => {
  type Period = "today" | "7d" | "30d";
  const PERIODS = [
    { value: "today" as const, label: "今天" },
    { value: "7d" as const, label: "7 天" },
    { value: "30d" as const, label: "30 天" },
  ];

  function Periods({ onPick }: { onPick?: (period: Period) => void }) {
    const [period, setPeriod] = useState<Period>("7d");
    return (
      <SegmentedControl<Period>
        aria-label="Period"
        options={PERIODS}
        value={period}
        onChange={(next) => {
          onPick?.(next);
          setPeriod(next);
        }}
      />
    );
  }

  it("is a radiogroup on a pill track with 22px options and a card thumb on the checked one", () => {
    render(<Periods />);
    const group = screen.getByRole("radiogroup", { name: "Period" });
    expect(group).toHaveClass("bg-pill", "p-[2px]", "rounded-ctl");
    const week = screen.getByRole("radio", { name: "7 天" });
    const today = screen.getByRole("radio", { name: "今天" });
    expect(week).toHaveAttribute("aria-checked", "true");
    expect(week).toHaveAttribute("tabindex", "0");
    expect(today).toHaveAttribute("aria-checked", "false");
    expect(today).toHaveAttribute("tabindex", "-1");
    expect(today).toHaveClass("h-[22px]", "text-ctl", "text-ink-muted");
    expect(week).toHaveClass("text-ink");
    expect(week.querySelector("[data-seg-thumb]")).toHaveClass("bg-card");
  });

  it("checks the option clicked or reached with the arrow keys", async () => {
    const user = userEvent.setup();
    const onPick = vi.fn();
    render(<Periods onPick={onPick} />);
    await user.click(screen.getByRole("radio", { name: "今天" }));
    expect(onPick).toHaveBeenLastCalledWith("today");

    await user.keyboard("{ArrowRight}");
    expect(onPick).toHaveBeenLastCalledWith("7d");
    expect(screen.getByRole("radio", { name: "7 天" })).toHaveFocus();
    await user.keyboard("{ArrowLeft}{ArrowLeft}");
    expect(screen.getByRole("radio", { name: "30 天" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });
});
