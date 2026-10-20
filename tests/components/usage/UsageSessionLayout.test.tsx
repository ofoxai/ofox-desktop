import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageSessions } from "@/components/usage-overview/UsageSessions";
import { UsageBreakdownList } from "@/components/usage-overview/UsageBreakdownList";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";

beforeEach(() => {
  localStorage.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
});

it("keeps paired analysis cards as siblings with the project/tool control inside the ranking header", () => {
  render(<UsageSessions data={sessionAnalyticsFixture()} active />);
  const projects = screen.getByRole("heading", { name: "项目排行" })
    .parentElement!.parentElement!;
  const models = screen.getByRole("heading", { name: "模型排行" })
    .parentElement!.parentElement!;
  expect(projects.parentElement).toBe(models.parentElement);
  expect(
    within(projects).getByRole("radiogroup", { name: "项目或工具排行" }),
  ).toBeVisible();
  const time = screen.getByRole("region", { name: "时间洞察" });
  const sessions = screen.getByRole("region", { name: "最耗会话" });
  expect(time.parentElement).toBe(sessions.parentElement);
  expect(within(projects).getByRole("radio", { name: "项目" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
});

it("makes a selected project removable without filling its whole label and keeps actions discoverable by focus", async () => {
  const data = sessionAnalyticsFixture();
  data.appliedQuery.projectIds = [data.byProject[0].id];
  const onProject = vi.fn();
  const onRemoveProject = vi.fn();
  render(
    <UsageSessions
      data={data}
      active
      onProject={onProject}
      onRemoveProject={onRemoveProject}
    />,
  );
  const label = screen.getByRole("button", { name: "project" });
  expect(label).toHaveAttribute("aria-pressed", "true");
  const remove = screen.getByRole("button", { name: "移除 project 筛选" });
  expect(label.closest('[data-selected="true"]')).toContainElement(remove);
  remove.focus();
  await userEvent.keyboard("{Enter}");
  expect(onRemoveProject).toHaveBeenCalledWith(data.byProject[0].id);
  expect(onProject).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "project" })).toBeVisible();
  const actions = screen.getByRole("button", { name: "project 操作" });
  actions.focus();
  await userEvent.keyboard("{Enter}");
  await userEvent.click(screen.getByRole("menuitem", { name: "隐藏" }));
  expect(screen.queryByRole("button", { name: "project" })).toBeNull();
  expect(onProject).not.toHaveBeenCalled();
  expect(onRemoveProject).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "1 个已隐藏" }));
  await userEvent.click(screen.getByRole("button", { name: "显示 project" }));
  expect(screen.getByRole("button", { name: "project" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

it.each([
  ["projects", "project", "/fixture/project"],
  ["models", "model-a", "model-a"],
  ["tools", "Codex", "codex"],
] as const)(
  "removes one %s selection through its dedicated callback while label clicks keep the existing filter action",
  async (kind, name, id) => {
    const data = sessionAnalyticsFixture();
    data.byTool = [{ ...data.byProject[0], id: "codex", label: "Codex" }];
    data.appliedQuery.projectIds = ["/fixture/project", "project-b"];
    data.appliedQuery.modelIds = ["model-a", "model-b"];
    data.appliedQuery.toolIds = ["codex", "claude"];
    const onFilter = vi.fn();
    const onRemove = vi.fn();
    render(
      <UsageSessions
        data={data}
        active
        onProject={onFilter}
        onModel={onFilter}
        onTool={onFilter}
        onRemoveProject={onRemove}
        onRemoveModel={onRemove}
        onRemoveTool={onRemove}
      />,
    );
    if (kind === "tools")
      await userEvent.click(screen.getByRole("radio", { name: "工具" }));
    await userEvent.click(
      screen.getByRole("button", { name: `移除 ${name} 筛选` }),
    );
    expect(onRemove).toHaveBeenCalledOnce();
    expect(onRemove).toHaveBeenCalledWith(id);
    expect(onFilter).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name }));
    expect(onFilter).toHaveBeenCalledWith(id);
    expect(onRemove).toHaveBeenCalledOnce();
  },
);

it("does not offer an individual remove action when only an exclusive filter callback is available", () => {
  const data = sessionAnalyticsFixture();
  data.appliedQuery.projectIds = ["/fixture/project", "project-b"];
  render(<UsageSessions data={data} active onProject={vi.fn()} />);
  expect(
    screen.queryByRole("button", { name: "移除 project 筛选" }),
  ).toBeNull();
  expect(screen.getByRole("button", { name: "project" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

it.each([
  ["projects", "project", "/fixture/project", false],
  ["projects", "project", "/fixture/project", true],
  ["models", "model-a", "model-a", false],
  ["models", "model-a", "model-a", true],
  ["tools", "Codex", "codex", false],
  ["tools", "Codex", "codex", true],
] as const)(
  "keeps focus on a stable card heading after removing a row and accepting a new result (%s, %s, %s, multiple=%s)",
  async (kind, name, id, multiple) => {
    const data = sessionAnalyticsFixture();
    const first = { ...data.byProject[0], id, label: name };
    const second = { ...first, id: "other", label: "Other" };
    const onFilter = vi.fn();
    const onRemove = vi.fn();
    const props = {
      kind,
      title: "Ranking",
      total: 100,
      onFilter,
      onRemove,
      compact: true,
    };
    const view = render(
      <UsageBreakdownList
        {...props}
        rows={[first, second]}
        selectedIds={multiple ? [id, "other"] : [id]}
      />,
    );
    const heading = screen.getByRole("heading", { name: "Ranking" });
    const remove = screen.getByRole("button", { name: `移除 ${name} 筛选` });
    if (multiple) {
      remove.focus();
      await userEvent.keyboard("{Enter}");
    } else await userEvent.click(remove);
    view.rerender(
      <UsageBreakdownList
        {...props}
        rows={[second]}
        selectedIds={multiple ? ["other"] : []}
      />,
    );
    expect(screen.queryByRole("button", { name })).toBeNull();
    expect(heading.isConnected).toBe(true);
    expect(heading).toHaveFocus();
    expect(onRemove).toHaveBeenCalledOnce();
    expect(onRemove).toHaveBeenCalledWith(id);
    expect(onFilter).not.toHaveBeenCalled();
  },
);

it("keeps long ranking names available to keyboard and assistive users", async () => {
  const data = sessionAnalyticsFixture();
  const name = "project-".repeat(80);
  data.byProject[0].label = name;
  data.byProject[0].path = `/very/long/${name}`;
  const onProject = vi.fn();
  render(<UsageSessions data={data} active onProject={onProject} />);
  const nameButton = screen.getByRole("button", { name });
  expect(nameButton).toHaveAttribute("title", `${name}\n/very/long/${name}`);
  nameButton.focus();
  expect(nameButton).toHaveFocus();
  await userEvent.keyboard("{Enter}");
  expect(onProject).toHaveBeenCalledWith(data.byProject[0].id);
  const session = screen.getByRole("button", {
    name: "查看会话 Frozen session",
  });
  session.focus();
  expect(session).toHaveFocus();
  expect(session).toHaveTextContent("Frozen session");
});

it("retains the partial price marker and exact amount in a compact estimate's tooltip", () => {
  const data = sessionAnalyticsFixture();
  data.byProject[0].estimate = {
    ...data.byProject[0].estimate!,
    availability: "partial",
    knownCostUsd: "0.0000001",
  };
  render(<UsageSessions data={data} active />);
  const estimate = screen.getByTitle(/≈\$1\.00e-7\+/);
  expect(estimate).toHaveAttribute("title", "≈$1.00e-7+\n0.0000001");
  expect(estimate).toHaveTextContent("≈$1.00e-7+");
});
