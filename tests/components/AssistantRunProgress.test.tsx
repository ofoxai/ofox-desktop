import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { AssistantRunProgress } from "@/components/assistant/AssistantRunProgress";

beforeEach(async () => {
  for (const [language, resource] of Object.entries({ en, zh, ja }))
    i18n.addResourceBundle(language, "translation", resource, true, true);
  await i18n.changeLanguage("en");
});
afterEach(() => vi.useRealTimers());
const props = () => ({
  startedAt: Date.now(),
  activity: null,
  intent: null,
  plan: [],
  warning: null,
});

describe("a live assistant task", () => {
  it.each([
    ["en", "Preparing the next step…", "Confirming the generation plan…"],
    ["zh", "正在准备下一步操作…", "正在确认生成计划…"],
    ["ja", "次の操作を準備しています…", "生成計画を確認中…"],
  ])(
    "distinguishes preparation from execution and plan confirmation in %s",
    async (language, preparation, confirmation) => {
      await i18n.changeLanguage(language);
      const view = render(
        <AssistantRunProgress {...props()} activity="preparingToolRequest" />,
      );
      expect(screen.getByRole("status")).toHaveTextContent(preparation);
      expect(screen.queryByRole("list")).toBeNull();
      expect(screen.queryByRole("progressbar")).toBeNull();
      view.rerender(
        <AssistantRunProgress {...props()} activity="confirmingPlan" />,
      );
      expect(screen.getByRole("status")).toHaveTextContent(confirmation);
    },
  );

  it("has useful progress before model planning, without invented steps or percentages", () => {
    render(<AssistantRunProgress {...props()} />);
    const region = screen.getByRole("region", { name: "Task progress" });
    expect(within(region).getByRole("status")).toHaveTextContent(
      "Working on your request",
    );
    expect(within(region).queryByRole("progressbar")).toBeNull();
    expect(within(region).queryByRole("list")).toBeNull();
  });

  it("counts total elapsed time without making each clock tick a live announcement", () => {
    vi.useFakeTimers();
    const start = Date.now();
    const view = render(<AssistantRunProgress {...props()} />);
    act(() => vi.advanceTimersByTime(354_000));
    expect(screen.getByText("Elapsed 05:54")).toBeVisible();
    expect(screen.getByRole("status")).not.toHaveTextContent("05:54");
    expect(screen.getByText(/No new stage update yet/)).toBeVisible();
    view.rerender(
      <AssistantRunProgress
        {...props()}
        startedAt={start}
        lastEventAt={Date.now()}
        activity="makingDocument"
      />,
    );
    expect(screen.queryByText(/No new stage update yet/)).toBeNull();
    expect(screen.getByText("Elapsed 05:54")).toBeVisible();
    view.unmount();
    expect(vi.getTimerCount()).toBe(0);
  });

  it("exposes raw execution intent only on request", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        intent="Inspect content_slide/panel_slide internals"
      />,
    );
    const detail = screen.getByText(
      "Inspect content_slide/panel_slide internals",
    );
    expect(detail).not.toBeVisible();
    await userEvent.click(screen.getByText("Execution details"));
    expect(detail).toBeVisible();
  });

  it("puts pending approval ahead of tool activity, reasoning and retry", () => {
    render(
      <AssistantRunProgress
        {...props()}
        activity="makingDocument"
        thinking
        retryAttempt={1}
        approvals={1}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Waiting for your approval",
    );
    expect(
      screen.getByText("Review the operation below to continue."),
    ).toBeVisible();
    expect(screen.queryByText("Generating a document…")).toBeNull();
    expect(screen.queryByText(/No new stage update yet/)).toBeNull();
  });

  it("shows a failed step the model did again as retried, not as a failure", () => {
    render(
      <AssistantRunProgress
        {...props()}
        activities={[
          { id: "style", activity: "selectingStyle", status: "completed" },
          { id: "first-render", activity: "makingDocument", status: "failed" },
          { id: "render", activity: "makingDocument", status: "running" },
        ]}
      />,
    );
    const list = screen.getByRole("list", { name: "Recent operations" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("Query presentation styles");
    expect(
      within(rows[0]).getByRole("img", { name: "Returned" }),
    ).toBeVisible();
    // The step is being done again: nothing for the user to stop or fix.
    expect(rows[1]).toHaveTextContent("Retried: Request document generation");
    expect(rows[1]).not.toHaveTextContent("Failed");
    expect(within(rows[1]).getByText("Retried")).not.toHaveClass("text-danger");
    expect(rows[2]).toHaveTextContent("Running: Request document generation");
    expect(screen.queryByRole("list", { name: "Task steps" })).toBeNull();
  });

  it("hands focus back only when the removed task surface contained focus", async () => {
    const onFinish = vi.fn();
    const view = render(
      <AssistantRunProgress
        {...props()}
        onFinish={onFinish}
        plan={[{ step: "Build", status: "in_progress" }]}
      />,
    );
    const toggle = screen.getByRole("button", { name: "Task steps" });
    toggle.focus();
    view.unmount();
    expect(onFinish).toHaveBeenCalledOnce();
    onFinish.mockClear();
    const outside = render(
      <AssistantRunProgress {...props()} onFinish={onFinish} />,
    );
    outside.unmount();
    expect(onFinish).not.toHaveBeenCalled();
  });

  it("keeps observed failures visible alongside a model checklist", () => {
    render(
      <AssistantRunProgress
        {...props()}
        plan={[{ step: "Review slides", status: "in_progress" }]}
        activities={[
          { id: "style", activity: "selectingStyle", status: "completed" },
          { id: "render", activity: "makingDocument", status: "failed" },
        ]}
      />,
    );
    const recent = screen.getByRole("list", { name: "Recent operations" });
    expect(within(recent).getAllByRole("listitem")).toHaveLength(1);
    expect(recent).toHaveTextContent("Failed: Request document generation");
    expect(screen.getByRole("list", { name: "Task steps" })).toHaveTextContent(
      "Review slides",
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Working on your request",
    );
  });

  it("does not label all completed reported steps as a delivered result", () => {
    render(
      <AssistantRunProgress
        {...props()}
        plan={[{ step: "Build slides", status: "completed" }]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Working on your request",
    );
    expect(screen.getByRole("list", { name: "Task steps" })).toHaveTextContent(
      "Build slides",
    );
    expect(screen.queryByText("Delivered")).toBeNull();
  });

  it.each([
    ["en", "Task progress", "Organizing the next steps"],
    ["zh", "任务进度", "正在梳理思路"],
    ["ja", "タスクの進捗", "次の手順を整理しています"],
  ])("localizes the active task in %s", async (language, label, title) => {
    await i18n.changeLanguage(language);
    render(<AssistantRunProgress {...props()} thinking />);
    expect(screen.getByRole("region", { name: label })).toHaveTextContent(
      title,
    );
  });
});
