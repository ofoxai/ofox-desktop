import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import {
  LocalTaskProgress,
  type LocalPlanStep,
} from "@/components/assistant/AssistantLocalConfig";

const plan: LocalPlanStep[] = [
  { step: "Choose a presentation style", status: "completed" },
  { step: "Draft the ten slides", status: "in_progress" },
  { step: "Review formulas and layout", status: "pending" },
];

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

describe("the assistant task checklist", () => {
  it("shows completed/total and distinct accessible step states", () => {
    render(<LocalTaskProgress intent={null} warning={null} plan={plan} />);
    const header = screen.getByRole("button", { name: "Task steps" });
    const list = screen.getByRole("list", { name: "Task steps" });
    expect(list).toHaveAttribute("role", "list");
    expect(header).toHaveAttribute("aria-expanded", "true");
    expect(header).toHaveAttribute("aria-controls", list.id);
    expect(header).toHaveTextContent("1/3");
    expect(header).toHaveAccessibleDescription("1/3");
    expect(within(list).getByRole("img", { name: "Completed" })).toBeVisible();
    expect(within(list).getByRole("img", { name: "Pending" })).toBeVisible();
    const current = within(list)
      .getByText("Draft the ten slides")
      .closest("li")!;
    expect(current).toHaveAttribute("aria-current", "step");
    expect(current).toHaveAttribute("aria-busy", "true");
    expect(
      within(current).getByRole("img", { name: "In progress" }),
    ).toBeVisible();
    expect(current.querySelector(".animate-spin")).toHaveClass(
      "motion-reduce:animate-none",
    );
  });

  it("collapses and expands by keyboard without hiding a host warning", async () => {
    const user = userEvent.setup();
    render(
      <LocalTaskProgress intent={null} warning="repeat_warning" plan={plan} />,
    );
    await user.tab();
    const header = screen.getByRole("button", { name: "Task steps" });
    expect(header).toHaveFocus();
    await user.keyboard("{Enter}");
    expect(header).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("list", { name: "Task steps" })).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent(
      /repeated three times/,
    );
    await user.keyboard(" ");
    expect(header).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByRole("list", { name: "Task steps" })).toBeVisible();
  });

  it("pauses execution animation while an approval is pending", () => {
    const { rerender } = render(
      <LocalTaskProgress intent={null} warning={null} plan={plan} paused />,
    );
    const current = screen.getByText("Draft the ten slides").closest("li")!;
    expect(current).toHaveAttribute("aria-current", "step");
    expect(current).toHaveAttribute("aria-busy", "false");
    expect(current.querySelector(".animate-spin")).toBeNull();
    expect(screen.getByRole("img", { name: "In progress" })).toBeVisible();
    rerender(<LocalTaskProgress intent={null} warning={null} plan={plan} />);
    expect(current).toHaveAttribute("aria-busy", "true");
    expect(current.querySelector(".animate-spin")).not.toBeNull();
  });

  it("keeps its collapsed state as a plan completes and stops every spinner", async () => {
    const user = userEvent.setup();
    const { rerender } = render(
      <LocalTaskProgress intent={null} warning={null} plan={plan} />,
    );
    await user.click(screen.getByRole("button", { name: "Task steps" }));
    rerender(
      <LocalTaskProgress
        intent={null}
        warning={null}
        plan={plan.map((step) => ({ ...step, status: "completed" }))}
      />,
    );
    const header = screen.getByRole("button", { name: "Task steps" });
    expect(header).toHaveTextContent("3/3");
    expect(header).toHaveAccessibleDescription("3/3");
    expect(header).toHaveAttribute("aria-expanded", "false");
    await user.click(header);
    const list = screen.getByRole("list", { name: "Task steps" });
    expect(
      within(list).getAllByRole("img", { name: "Completed" }),
    ).toHaveLength(3);
    expect(list.querySelector("[aria-current]")).toBeNull();
    expect(list.querySelector(".animate-spin")).toBeNull();
  });

  it("can omit the duplicate intent while keeping warnings and wrapped steps", () => {
    render(
      <LocalTaskProgress
        intent="Internal tool intent"
        warning="repeat_warning"
        plan={plan}
        showIntent={false}
      />,
    );
    expect(screen.queryByText("Internal tool intent")).toBeNull();
    expect(screen.getByRole("status")).toBeVisible();
    expect(screen.getByText("Review formulas and layout")).toHaveClass(
      "min-w-0",
      "break-words",
    );
  });

  it("preserves the default intent and renders nothing when all visible data is absent", () => {
    const { rerender, container } = render(
      <LocalTaskProgress
        intent="Reviewing the slides"
        warning={null}
        plan={[]}
      />,
    );
    expect(screen.getByText("Reviewing the slides")).toBeVisible();
    rerender(
      <LocalTaskProgress
        intent="Hidden duplicate intent"
        warning={null}
        plan={[]}
        showIntent={false}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("gives separate checklists separate disclosure targets", () => {
    render(
      <>
        <LocalTaskProgress intent={null} warning={null} plan={plan} />
        <LocalTaskProgress intent={null} warning={null} plan={plan} />
      </>,
    );
    const headers = screen.getAllByRole("button", { name: "Task steps" });
    const targets = headers.map((header) =>
      header.getAttribute("aria-controls"),
    );
    expect(new Set(targets).size).toBe(2);
    for (const target of targets)
      expect(document.getElementById(target!)).not.toBeNull();
  });
});
