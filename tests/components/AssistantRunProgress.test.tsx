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
  it("does not hide a failed write when a different write of the same category succeeds", () => {
    render(
      <AssistantRunProgress
        {...props()}
        activities={[
          { id: "write-a", activity: "editingFile", status: "failed" },
          { id: "write-b", activity: "editingFile", status: "completed" },
        ]}
      />,
    );
    expect(
      within(screen.getByRole("list", { name: "Recent operations" })).getByText(
        "Update a working file",
      ),
    ).toBeVisible();
    expect(
      screen.getByRole("list", { name: "Recent operations" }),
    ).toHaveTextContent("Failed");
  });

  it("returns focus when an unplanned task ends and removes its details", async () => {
    const onFinish = vi.fn();
    const view = render(
      <AssistantRunProgress
        {...props()}
        intent="Read context"
        onFinish={onFinish}
      />,
    );
    screen.getByRole("button", { name: "Execution details" }).focus();
    view.rerender(
      <AssistantRunProgress
        {...props()}
        intent={null}
        onFinish={onFinish}
        status="completed"
        completedAt={Date.now()}
      />,
    );
    expect(onFinish).toHaveBeenCalledOnce();
  });

  it("keeps unresolved MCP failures visible after the final checklist folds", () => {
    render(
      <AssistantRunProgress
        {...props()}
        status="completed"
        completedAt={Date.now()}
        plan={[{ step: "Read external data", status: "pending" }]}
        mcpSteps={[
          {
            id: "failed",
            server: "Unavailable integration",
            tool: "fetch",
            status: "failed",
          },
        ]}
      />,
    );
    expect(screen.getByText(/Unavailable integration/)).toBeVisible();
  });

  it("keeps simple replies to a lightweight line and hides generic card copy", () => {
    render(<AssistantRunProgress {...props()} />);
    expect(
      screen.queryByText(
        "New task steps and results will appear here as work continues.",
      ),
    ).toBeNull();
    expect(screen.queryByRole("region", { name: "Task progress" })).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Working on your request",
    );
    expect(screen.queryByText(/Elapsed/)).toBeNull();
  });

  it("shows the checklist before a collapsed execution log", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        plan={[{ step: "Draft", status: "in_progress" }]}
        activities={[
          { id: "read", activity: "readingFile", status: "completed" },
        ]}
      />,
    );
    expect(screen.getByRole("list", { name: "Task steps" })).toBeVisible();
    expect(
      screen.queryByRole("list", { name: "Recent operations" }),
    ).toBeNull();
    await userEvent.click(
      screen.getByRole("button", { name: "Execution details" }),
    );
    expect(
      screen.getByRole("list", { name: "Recent operations" }),
    ).toBeVisible();
  });

  it("folds a finished checklist and freezes its duration without completing unfinished steps", async () => {
    vi.useFakeTimers();
    const start = Date.now();
    const view = render(
      <AssistantRunProgress
        {...props()}
        startedAt={start}
        status="completed"
        completedAt={start + 83_000}
        plan={[{ step: "Draft", status: "in_progress" }]}
      />,
    );
    expect(screen.queryByRole("list", { name: "Task steps" })).toBeNull();
    expect(screen.getByText("Elapsed 1m 23s")).toBeVisible();
    act(() => vi.advanceTimersByTime(60_000));
    expect(screen.getByText("Elapsed 1m 23s")).toBeVisible();
    expect(vi.getTimerCount()).toBe(0);
    act(() => screen.getByRole("button", { name: /Run ended/ }).click());
    expect(screen.getByRole("list", { name: "Task steps" })).toHaveTextContent(
      "Draft",
    );
    expect(screen.queryByRole("img", { name: "Completed" })).toBeNull();
    expect(view.container.querySelector(".animate-spin")).toBeNull();
  });

  it("does not invent a duration for an interrupted run without an end time", () => {
    render(
      <AssistantRunProgress
        {...props()}
        status="stopped"
        errorCode="assistant_interrupted"
        plan={[{ step: "Draft", status: "pending" }]}
      />,
    );
    expect(screen.getByRole("button", { name: /Interrupted/ })).toBeVisible();
    expect(screen.queryByText(/Elapsed/)).toBeNull();
  });

  it("moves focus from hidden execution details to the final summary", async () => {
    const shared = {
      ...props(),
      plan: [{ step: "Draft", status: "in_progress" as const }],
      intent: "Generating draft",
    };
    const view = render(<AssistantRunProgress {...shared} />);
    const toggle = screen.getByRole("button", { name: "Execution details" });
    await userEvent.click(toggle);
    toggle.focus();
    view.rerender(
      <AssistantRunProgress
        {...shared}
        status="completed"
        completedAt={Date.now()}
      />,
    );
    expect(screen.getByRole("button", { name: /Run ended/ })).toHaveFocus();
  });

  it("keeps MCP failures visible while folding routine MCP execution details", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        mcpSteps={[
          { id: "read", server: "docs", tool: "lookup", status: "running" },
          {
            id: "unavailable",
            server: "local",
            tool: "read",
            status: "unavailable",
            reason: "needs_node",
          },
        ]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Using docs");
    expect(screen.getByText("Install Node.js with npx")).toBeVisible();
    expect(screen.getByText("docs · lookup")).not.toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Execution details" }),
    );
    expect(screen.getByText("docs · lookup")).toBeVisible();
  });

  it("preserves a manually folded live checklist through plan updates", async () => {
    const view = render(
      <AssistantRunProgress
        {...props()}
        plan={[{ step: "Draft", status: "in_progress" }]}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "Task steps" }));
    view.rerender(
      <AssistantRunProgress
        {...props()}
        plan={[
          { step: "Draft", status: "completed" },
          { step: "Review", status: "in_progress" },
        ]}
      />,
    );
    expect(screen.getByRole("button", { name: "Task steps" })).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.queryByRole("list", { name: "Task steps" })).toBeNull();
  });

  it.each([
    ["en", "Run ended", "Elapsed 1m 23s"],
    ["zh", "本轮结束", "已用 1分23秒"],
    ["ja", "実行終了", "経過 1分23秒"],
  ])(
    "localizes final summaries and short durations in %s",
    async (language, label, elapsed) => {
      await i18n.changeLanguage(language);
      render(
        <AssistantRunProgress
          {...props()}
          startedAt={1000}
          status="completed"
          completedAt={84000}
          plan={[{ step: "Draft", status: "completed" }]}
        />,
      );
      expect(
        screen.getByRole("button", { name: new RegExp(label) }),
      ).toBeVisible();
      expect(screen.getByText(elapsed)).toBeVisible();
    },
  );
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

  it("shows a single search until reply text starts, without opening execution details", () => {
    const view = render(
      <AssistantRunProgress
        {...props()}
        activity="searching"
        activities={[
          { id: "search", activity: "searching", status: "running" },
        ]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(/Search/);
    expect(screen.queryByRole("region", { name: "Task progress" })).toBeNull();
    expect(
      screen.queryByRole("button", { name: "Execution details" }),
    ).toBeNull();
    expect(view.container.querySelector(".lucide-chevron-right")).toBeNull();
    view.rerender(
      <AssistantRunProgress
        {...props()}
        replyStarted
        activities={[
          { id: "search", activity: "searching", status: "completed" },
        ]}
      />,
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("counts total elapsed time without making each clock tick a live announcement", () => {
    vi.useFakeTimers();
    const start = Date.now();
    const plan = [{ step: "Make the slides", status: "in_progress" as const }];
    const view = render(<AssistantRunProgress {...props()} plan={plan} />);
    act(() => vi.advanceTimersByTime(354_000));
    expect(screen.getByText("Elapsed 5m 54s")).toBeVisible();
    expect(screen.getByRole("status")).not.toHaveTextContent("5m 54s");
    expect(screen.getByText(/No new stage update yet/)).toBeVisible();
    view.rerender(
      <AssistantRunProgress
        {...props()}
        plan={plan}
        startedAt={start}
        lastEventAt={Date.now()}
        activity="makingDocument"
      />,
    );
    expect(screen.queryByText(/No new stage update yet/)).toBeNull();
    expect(screen.getByText("Elapsed 5m 54s")).toBeVisible();
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
    await userEvent.click(
      screen.getByRole("button", { name: "Execution details" }),
    );
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

  it("keeps a prior failure visible while a same-category call runs", async () => {
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
    expect(
      screen.getByRole("list", { name: "Recent operations" }),
    ).toHaveTextContent("Failed: Request document generation");
    await userEvent.click(
      screen.getByRole("button", { name: "Execution details" }),
    );
    const list = screen.getAllByRole("list", { name: "Recent operations" })[1];
    const rows = within(list).getAllByRole("listitem");
    expect(rows[0]).toHaveTextContent("Query presentation styles");
    expect(
      within(rows[0]).getByRole("img", { name: "Returned" }),
    ).toBeVisible();
    expect(rows[1]).toHaveTextContent("Running: Request document generation");
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
    render(
      <AssistantRunProgress
        {...props()}
        plan={[{ step: "Make the slides", status: "in_progress" }]}
        thinking
      />,
    );
    expect(screen.getByRole("region", { name: label })).toHaveTextContent(
      title,
    );
  });
});
