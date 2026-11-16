import type { ComponentProps } from "react";
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
  it("groups repeated calls and shows returned status with checks in last-occurrence order", () => {
    render(
      <AssistantRunProgress
        {...props()}
        thinking
        activities={[style("style-1"), guidance("guidance"), style("style-2")]}
      />,
    );
    // Use the list role first: the original UI must fail on the actual three
    // rows versus two groups, rather than just on the renamed heading.
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
    expect(
      screen.getByText(
        "Tool responses are operation records, not completed task steps.",
      ),
    ).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Organizing the next steps",
    );
    expect(screen.queryByRole("list", { name: "Task steps" })).toBeNull();
  });

  it("groups by category and state without merging running or failed work into returned calls", () => {
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
    const list = screen.getByRole("list", { name: "Recent operations" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    returnedBadge(rows[0]);
    expect(rows[0]).toHaveTextContent("2 recent calls");
    expect(rows[1]).toHaveTextContent("Running:");
    expect(rows[1].querySelector(".animate-spin")).not.toBeNull();
    expect(rows[2]).toHaveTextContent("Failed:");
    const failure = within(rows[2]).getByText("Failed", { exact: true });
    expect(failure).toBeVisible();
    expect(failure).not.toHaveClass("sr-only");
    expect(list).not.toHaveTextContent("4 recent calls");
  });

  it("deduplicates call IDs before grouping and uses the latest state", () => {
    const activities = [style("same"), style("other"), style("same", "failed")];
    const view = render(
      <AssistantRunProgress {...props()} activities={activities} />,
    );
    const assertRecords = () => {
      const rows = within(
        screen.getByRole("list", { name: "Recent operations" }),
      ).getAllByRole("listitem");
      expect(rows).toHaveLength(2);
      returnedBadge(rows[0]);
      expect(rows[1]).toHaveTextContent("Failed:");
      expect(screen.queryByText(/recent calls/)).toBeNull();
    };
    assertRecords();
    view.rerender(
      <AssistantRunProgress {...props()} activities={activities} />,
    );
    assertRecords();
  });

  it("reads a failure the same step came after as retried, and only the last one as failed", () => {
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
    const rows = within(
      screen.getByRole("list", { name: "Recent operations" }),
    ).getAllByRole("listitem");
    expect(rows).toHaveLength(3);
    expect(rows[0]).toHaveTextContent("Retried:");
    expect(rows[1]).toHaveTextContent("Running:");
    expect(rows[2]).toHaveTextContent("Failed:");
    for (const row of rows) expect(row).not.toHaveTextContent(/recent calls/);
  });

  it("pauses only running records while approval is pending", () => {
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
    const rows = within(
      screen.getByRole("list", { name: "Recent operations" }),
    ).getAllByRole("listitem");
    returnedBadge(rows[0]);
    expect(rows[1]).toHaveTextContent("Retried:");
    expect(rows[2]).toHaveTextContent("Awaiting approval:");
    expect(rows[2].querySelector(".animate-spin")).toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Waiting for your approval",
    );
  });

  it("keeps running and failed operations separate from model-plan completion", () => {
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
    const records = screen.getByRole("list", { name: "Recent operations" });
    const rows = within(records).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveTextContent("Retried:");
    expect(rows[1]).toHaveTextContent("Running:");
    expect(within(records).queryByText("Returned", { exact: true })).toBeNull();
    expect(records.querySelector("svg.lucide-check")).toBeNull();
    const plan = screen.getByRole("list", { name: "Task steps" });
    expect(within(plan).getByRole("img", { name: "Completed" })).toBeVisible();
    expect(plan.querySelector("svg.lucide-check")).not.toBeNull();
  });

  it("deduplicates before hiding returned calls so a model plan cannot revive stale running or failed records", () => {
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
