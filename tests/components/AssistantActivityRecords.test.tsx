import type { ComponentProps } from "react";
import userEvent from "@testing-library/user-event";
import { render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { AssistantRunProgress } from "@/components/assistant/AssistantRunProgress";

type Activity = NonNullable<
  ComponentProps<typeof AssistantRunProgress>["activities"]
>[number];
const props = () => ({
  startedAt: Date.now(),
  activity: null,
  intent: null,
  plan: [],
  warning: null,
});
const style = (
  id: string,
  status: Activity["status"] = "completed",
): Activity => ({ id, activity: "selectingStyle", status });
const guidance = (id: string): Activity => ({
  id,
  activity: "readingGuidance",
  status: "completed",
});
const operationRows = () =>
  screen
    .getAllByRole("list", { name: "Recent operations" })
    .flatMap((list) => within(list).getAllByRole("listitem"));
const recordWithState = (name: string) =>
  operationRows().find((row) => within(row).queryByRole("img", { name }))!;
const openRecords = async () =>
  userEvent.click(screen.getByRole("button", { name: "Execution details" }));
const returnedBadge = (row: HTMLElement) => {
  const badge = within(row).getByRole("img", { name: "Returned" });
  expect(badge).toBeVisible();
  expect(badge).not.toHaveClass("sr-only");
  return badge;
};

beforeEach(async () => {
  for (const [language, resource] of Object.entries({ en, zh, ja }))
    i18n.addResourceBundle(language, "translation", resource, true, true);
  await i18n.changeLanguage("en");
});

describe("observed operation records", () => {
  it("groups repeated calls and shows returned status with checks in last-occurrence order", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        thinking
        activities={[style("style-1"), guidance("guidance"), style("style-2")]}
      />,
    );
    expect(screen.queryByRole("list")).toBeNull();
    await openRecords();
    const list = screen.getByRole("list");
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(list).toHaveAccessibleName("Recent operations");
    expect(rows[0]).toHaveTextContent("Load task guidance");
    expect(rows[1]).toHaveTextContent("Query presentation styles");
    returnedBadge(rows[0]);
    returnedBadge(rows[1]);
    const count = within(rows[1]).getByText("2 recent calls");
    expect(count).toBeVisible();
    expect(count.closest('[aria-hidden="true"]')).toBeNull();
    expect(rows[0]).not.toHaveTextContent("1 recent calls");
    expect(within(list).getAllByRole("img", { name: "Returned" })).toHaveLength(
      2,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "Organizing the next steps",
    );
    expect(screen.queryByRole("list", { name: "Task steps" })).toBeNull();
  });

  it("groups by category and state without merging running or failed work into returned calls", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        activities={[
          style("returned-1"),
          style("returned-2"),
          style("running", "running"),
          style("failed", "failed"),
        ]}
      />,
    );
    // Failures stay visible before routine execution details are opened.
    expect(operationRows()).toHaveLength(1);
    expect(recordWithState("Failed")).toHaveTextContent("Failed:");
    expect(screen.queryByRole("img", { name: "Returned" })).toBeNull();
    await openRecords();
    expect(operationRows()).toHaveLength(3);
    const returned = recordWithState("Returned");
    returnedBadge(returned);
    expect(returned).toHaveTextContent("2 recent calls");
    const running = recordWithState("Running");
    expect(running).toHaveTextContent("Running:");
    expect(running.querySelector(".animate-spin")).not.toBeNull();
    const failure = within(recordWithState("Failed")).getByText("Failed", {
      exact: true,
    });
    expect(failure).toBeVisible();
    expect(failure).not.toHaveClass("sr-only");
    for (const row of operationRows())
      expect(row).not.toHaveTextContent("4 recent calls");
  });

  it("deduplicates call IDs before grouping and uses the latest state", async () => {
    const activities = [style("same"), style("other"), style("same", "failed")];
    const view = render(
      <AssistantRunProgress {...props()} activities={activities} />,
    );
    expect(operationRows()).toHaveLength(1);
    expect(recordWithState("Failed")).toBeVisible();
    await openRecords();
    const assertRecords = () => {
      const rows = operationRows();
      expect(rows).toHaveLength(2);
      returnedBadge(recordWithState("Returned"));
      expect(recordWithState("Failed")).toHaveTextContent("Failed:");
      expect(screen.queryByText(/recent calls/)).toBeNull();
    };
    assertRecords();
    view.rerender(
      <AssistantRunProgress {...props()} activities={activities} />,
    );
    assertRecords();
  });

  it("keeps failures visible when another call shares their category", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        activities={[
          style("failed-1", "failed"),
          style("active", "running"),
          style("failed-2", "failed"),
        ]}
      />,
    );
    const failed = recordWithState("Failed");
    expect(failed).toBeVisible();
    expect(failed).toHaveTextContent("2 recent calls");
    expect(screen.queryByText("Retried")).toBeNull();
    await openRecords();
    expect(operationRows()).toHaveLength(2);
    expect(recordWithState("Running")).toHaveTextContent("Running:");
    expect(recordWithState("Failed")).toHaveTextContent("Failed:");
  });

  it("pauses only running records while approval is pending", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        approvals={1}
        activities={[
          style("returned"),
          style("failed", "failed"),
          style("active", "running"),
        ]}
      />,
    );
    expect(recordWithState("Failed")).toBeVisible();
    await openRecords();
    returnedBadge(recordWithState("Returned"));
    expect(recordWithState("Failed")).toHaveTextContent("Failed:");
    const running = recordWithState("Awaiting approval");
    expect(running).toHaveTextContent("Awaiting approval:");
    expect(running.querySelector(".animate-spin")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Waiting for your approval",
    );
  });

  it("keeps running and failed operations separate from model-plan completion", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        plan={[
          { step: "Outline the content", status: "completed" },
          { step: "Review slides", status: "in_progress" },
        ]}
        activities={[
          style("returned"),
          style("failed", "failed"),
          style("active", "running"),
        ]}
      />,
    );
    expect(recordWithState("Failed")).toBeVisible();
    await openRecords();
    expect(operationRows()).toHaveLength(3);
    expect(recordWithState("Failed")).toHaveTextContent("Failed:");
    expect(recordWithState("Running")).toHaveTextContent("Running:");
    returnedBadge(recordWithState("Returned"));
    for (const row of operationRows()) {
      expect(within(row).queryByRole("img", { name: "Completed" })).toBeNull();
    }
    const plan = screen.getByRole("list", { name: "Task steps" });
    expect(
      within(plan).getAllByRole("img", { name: "Completed" }),
    ).toHaveLength(1);
    expect(
      within(plan).getByRole("img", { name: "In progress" }),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Task steps" }),
    ).toHaveTextContent("1/2");
  });

  it("deduplicates before folding returned calls so a model plan cannot revive stale running or failed records", async () => {
    render(
      <AssistantRunProgress
        {...props()}
        plan={[{ step: "Review slides", status: "in_progress" }]}
        activities={[
          style("finished-run", "running"),
          style("recovered-call", "failed"),
          style("finished-run", "completed"),
          style("recovered-call", "completed"),
        ]}
      />,
    );
    expect(
      screen.queryByRole("list", { name: "Recent operations" }),
    ).toBeNull();
    expect(screen.getByRole("list", { name: "Task steps" })).toHaveTextContent(
      "Review slides",
    );
    await openRecords();
    expect(operationRows()).toHaveLength(1);
    returnedBadge(recordWithState("Returned"));
    expect(recordWithState("Returned")).toHaveTextContent("2 recent calls");
    expect(screen.queryByRole("img", { name: "Failed" })).toBeNull();
    expect(screen.queryByRole("img", { name: "Running" })).toBeNull();
  });

  it.each([
    [
      "en",
      en,
      /^Returned$/,
      /^Query presentation styles$/,
      /^Load task guidance$/,
      /^Request document generation$/,
    ],
    ["zh", zh, /返回|响应/, /查询|读取|获取/, /读取|加载/, /请求|发起|调用/],
    [
      "ja",
      ja,
      /応答|返却|戻/,
      /照会|取得|参照|問い合わせ|検索/,
      /読|ロード|取得/,
      /依頼|要求|リクエスト/,
    ],
  ] as const)(
    "uses operation-result semantics in %s without changing real plan completion labels",
    async (
      language,
      resource,
      returned,
      queryStyles,
      loadGuidance,
      requestDocument,
    ) => {
      const text = resource.assistant.progress;
      expect(text.states.completed).toMatch(returned);
      expect(text.states.completed).not.toBe(
        resource.assistant.local.steps.completed,
      );
      expect(text.activities.selectingStyle).toMatch(queryStyles);
      expect(text.activities.readingGuidance).toMatch(loadGuidance);
      expect(text.activities.makingDocument).toMatch(requestDocument);
      await i18n.changeLanguage(language);
      render(
        <AssistantRunProgress
          {...props()}
          activities={[style("one"), style("two")]}
        />,
      );
      expect(screen.queryByRole("list", { name: text.recent })).toBeNull();
      await userEvent.click(screen.getByRole("button", { name: text.details }));
      const list = screen.getByRole("list", { name: text.recent });
      expect(within(list).getAllByRole("listitem")).toHaveLength(1);
      const badge = within(list).getByRole("img", {
        name: text.states.completed,
      });
      expect(badge).toBeVisible();
      expect(badge).not.toHaveClass("sr-only");
      expect(list).toHaveTextContent(
        language === "en"
          ? "2 recent calls"
          : language === "zh"
            ? "近期 2 次"
            : "直近 2 回",
      );
    },
  );
});
