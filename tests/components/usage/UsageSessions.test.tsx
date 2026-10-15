import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";
beforeEach(() => {
  localStorage.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
});
it("keeps unknown activity distinct from zero and shows quantile exclusions and unassociated usage", async () => {
  render(<UsageSessions data={sessionAnalyticsFixture()} active />);
  expect(screen.getByText(/有效样本 2.*排除 1/)).toBeVisible();
  expect(screen.getByText(/无法按模型拆分/)).toBeVisible();
  expect(screen.getByText(/1 条未关联会话的记录.*7 Tokens/)).toBeVisible();
  await userEvent.click(screen.getByRole("combobox", { name: "趋势指标" }));
  expect(screen.getByRole("option", { name: "消息数" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});
it("keeps embedded detail frozen across a newer analysis response and returns focus", async () => {
  const initial = sessionAnalyticsFixture();
  const view = render(<UsageSessions data={initial} active />);
  const trigger = screen.getByRole("button", {
    name: "查看会话 Frozen session",
  });
  trigger.focus();
  await userEvent.keyboard("{Enter}");
  const newer = sessionAnalyticsFixture();
  newer.snapshotVersion = "analysis-v2";
  newer.topSessions[0].models[0].model = "new-model";
  view.rerender(<UsageSessions data={newer} active />);
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getByText("model-a")).toBeVisible();
  expect(within(dialog).queryByText("new-model")).toBeNull();
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(trigger).toHaveFocus());
});
it.each([false, true])(
  "does not offer manager navigation without a source and stable manager key (available=%s)",
  async (available) => {
    const data = sessionAnalyticsFixture();
    data.topSessions[0].sourceAvailable = available;
    data.topSessions[0].managerKey = available ? null : "manager-key";
    const onManage = vi.fn();
    render(<UsageSessions data={data} active onManage={onManage} />);
    await userEvent.click(
      screen.getByRole("button", { name: "查看会话 Frozen session" }),
    );
    expect(
      screen.getByRole("button", { name: "在会话管理中查看" }),
    ).toBeDisabled();
    expect(onManage).not.toHaveBeenCalled();
  },
);

it("closes the frozen dialog when handing a stable key to session management", async () => {
  const onManage = vi.fn();
  render(
    <UsageSessions
      data={sessionAnalyticsFixture()}
      active
      onManage={onManage}
    />,
  );
  await userEvent.click(
    screen.getByRole("button", { name: "查看会话 Frozen session" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "在会话管理中查看" }),
  );
  expect(onManage).toHaveBeenCalledWith("manager-key");
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("hides a ranking row without changing analysis totals or filters and can restore it", async () => {
  localStorage.clear();
  const onModel = vi.fn();
  render(
    <UsageSessions data={sessionAnalyticsFixture()} active onModel={onModel} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "model-a 操作" }));
  await userEvent.click(screen.getByRole("menuitem", { name: "隐藏" }));
  expect(screen.queryByRole("button", { name: "model-a" })).toBeNull();
  expect(onModel).not.toHaveBeenCalled();
  expect(screen.getByText(/有效样本 2.*排除 1/)).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "1 个已隐藏" }));
  await userEvent.click(screen.getByRole("button", { name: "显示 model-a" }));
  expect(screen.getByRole("button", { name: "model-a" })).toBeVisible();
});
it("explains unavailable message coverage rather than presenting zero activity", () => {
  const data = sessionAnalyticsFixture();
  data.totals.messageReason = "messageCoveragePartial";
  data.totals.activityReason = "messageCoveragePartial";
  render(<UsageSessions data={data} active />);
  expect(screen.getByText(/消息时间覆盖不完整/)).toBeVisible();
});

it.each(["kpi", "row", "detail"])(
  "labels an identified Unknown project consistently in %s",
  async (place) => {
    const data = sessionAnalyticsFixture();
    data.byProject[0] = {
      ...data.byProject[0],
      id: "__unknown_project__",
      label: "__unknown_project__",
      path: null,
    };
    data.topSessions[0].projectId = "__unknown_project__";
    data.topSessions[0].projectLabel = null;
    data.topSessions[0].projectPath = null;
    render(<UsageSessions data={data} active />);
    if (place === "kpi")
      expect(
        within(screen.getByText("项目数").parentElement!).getByText("未知项目"),
      ).toBeVisible();
    else if (place === "row")
      expect(
        screen.getByRole("button", { name: "查看会话 Frozen session" }),
      ).toHaveTextContent("未知项目");
    else {
      await userEvent.click(
        screen.getByRole("button", { name: "查看会话 Frozen session" }),
      );
      expect(
        within(screen.getByRole("dialog")).getByText("未知项目"),
      ).toBeVisible();
    }
  },
);
it("retains the trend selection when a filter is loading a new analysis", async () => {
  const data = sessionAnalyticsFixture();
  const view = render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("combobox", { name: "趋势指标" }));
  await userEvent.click(screen.getByRole("option", { name: "输出 Tokens" }));
  view.rerender(<UsageSessions data={data} active showData={false} />);
  view.rerender(<UsageSessions data={data} active />);
  expect(screen.getByRole("combobox", { name: "趋势指标" })).toHaveTextContent(
    "输出 Tokens",
  );
});
it("bounds large project and model rankings and expands each list on request", async () => {
  localStorage.clear();
  const data = sessionAnalyticsFixture();
  data.byProject = Array.from({ length: 300 }, (_, n) => ({
    ...data.byProject[0],
    id: `project-${n}`,
    label: `project-${n}`,
  }));
  data.byModel = Array.from({ length: 300 }, (_, n) => ({
    ...data.byModel[0],
    id: `model-${n}`,
    label: `model-${n}`,
  }));
  render(
    <UsageSessions data={data} active onProject={vi.fn()} onModel={vi.fn()} />,
  );
  const project = screen.getByRole("heading", { name: "项目排行" })
    .parentElement!.parentElement!;
  const model = screen.getByRole("heading", { name: "模型排行" }).parentElement!
    .parentElement!;
  expect(
    within(project).getAllByRole("button", { name: /^project-\d+$/ }),
  ).toHaveLength(50);
  expect(
    within(model).getAllByRole("button", { name: /^model-\d+$/ }),
  ).toHaveLength(50);
  await userEvent.click(
    within(project).getByRole("button", { name: "显示更多（剩余 250）" }),
  );
  expect(
    within(project).getAllByRole("button", { name: /^project-\d+$/ }),
  ).toHaveLength(100);
  expect(
    within(model).getAllByRole("button", { name: /^model-\d+$/ }),
  ).toHaveLength(50);
});
it("bounds restoration controls for a large hidden project list", async () => {
  const data = sessionAnalyticsFixture();
  data.byProject = Array.from({ length: 300 }, (_, n) => ({
    ...data.byProject[0],
    id: `project-${n}`,
    label: `project-${n}`,
  }));
  localStorage.setItem(
    "ofox.usageAnalysisHidden.projects",
    JSON.stringify(data.byProject.map((row) => row.id)),
  );
  render(<UsageSessions data={data} active />);
  await userEvent.click(screen.getByRole("button", { name: "300 个已隐藏" }));
  expect(
    screen.getAllByRole("button", { name: /^显示 project-/ }),
  ).toHaveLength(50);
  await userEvent.click(
    screen.getByRole("button", { name: "更多隐藏项（剩余 250）" }),
  );
  expect(
    screen.getAllByRole("button", { name: /^显示 project-/ }),
  ).toHaveLength(100);
  localStorage.clear();
});

it("bounds visible models and hidden restoration for a ten-thousand-project snapshot", async () => {
  const data = sessionAnalyticsFixture();
  data.byProject = Array.from({ length: 10000 }, (_, n) => ({
    ...data.byProject[0],
    id: `project-${n}`,
    label: `project-${n}`,
  }));
  data.byModel = Array.from({ length: 10000 }, (_, n) => ({
    ...data.byModel[0],
    id: `model-${n}`,
    label: `model-${n}`,
  }));
  localStorage.setItem(
    "ofox.usageAnalysisHidden.projects",
    JSON.stringify(data.byProject.map((row) => row.id)),
  );
  render(<UsageSessions data={data} active onModel={vi.fn()} />);
  expect(screen.getAllByRole("button", { name: /^model-\d+$/ })).toHaveLength(
    50,
  );
  await userEvent.click(screen.getByRole("button", { name: "10000 个已隐藏" }));
  expect(
    screen.getAllByRole("button", { name: /^显示 project-/ }),
  ).toHaveLength(50);
});
it.each([
  [null, "项目尚不可解析"],
  ["known-project-id", "known-project-id"],
])("reserves Unparsed for a missing project identity (%s)", (id, label) => {
  const data = sessionAnalyticsFixture();
  const row = data.topSessions[0];
  row.projectId = id;
  row.projectLabel = null;
  row.projectPath = null;
  render(<UsageSessions data={data} active />);
  expect(
    screen.getByRole("button", { name: "查看会话 Frozen session" }),
  ).toHaveTextContent(label!);
});

it("bounds a frozen session's model detail and expands it without altering totals", async () => {
  const data = sessionAnalyticsFixture();
  const row = data.topSessions[0];
  row.models = Array.from({ length: 300 }, (_, n) => ({
    ...row.models[0],
    model: `detail-model-${n}`,
  }));
  render(<UsageSessions data={data} active />);
  await userEvent.click(
    screen.getByRole("button", { name: "查看会话 Frozen session" }),
  );
  const dialog = screen.getByRole("dialog");
  expect(within(dialog).getAllByText(/^detail-model-\d+$/)).toHaveLength(50);
  await userEvent.click(
    within(dialog).getByRole("button", { name: "显示更多（剩余 250）" }),
  );
  expect(within(dialog).getAllByText(/^detail-model-\d+$/)).toHaveLength(100);
});
