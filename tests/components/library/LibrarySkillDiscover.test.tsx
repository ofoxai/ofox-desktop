import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { LibrarySkillDiscover } from "@/components/library/LibrarySkillDiscover";
import { skillsApi, type DiscoverableSkill } from "@/lib/api/skills";
import {
  POPULAR_SKILLS,
  formatInstalls,
} from "@/components/library/popularSkills";
import type { SkillDefinition } from "@/lib/api/library";
const item = (name: string, owner = "example"): DiscoverableSkill => ({
  key: `${owner}/catalog:skills/${name}`,
  name,
  description: `${name} description`,
  directory: `skills/${name}`,
  repoOwner: owner,
  repoName: "catalog",
  repoBranch: "main",
});
const choose = vi.fn();
function discovery(rows: SkillDefinition[], client: QueryClient) {
  return (
    <QueryClientProvider client={client}>
      <LibrarySkillDiscover
        rows={rows}
        busy={false}
        onChoose={choose}
        onChooseRecipe={vi.fn()}
        onOpenSource={vi.fn()}
      />
    </QueryClientProvider>
  );
}
function page(
  rows: SkillDefinition[] = [],
  client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  }),
) {
  return render(discovery(rows, client));
}
const collectedRow = (skill: DiscoverableSkill): SkillDefinition =>
  ({
    id: skill.key,
    name: skill.name,
    directory: skill.directory,
    sourceKind: "repository",
    repository: `${skill.repoOwner}/${skill.repoName}`,
    sourceState: "ready",
    targets: [],
  }) as SkillDefinition;
beforeEach(async () => {
  vi.restoreAllMocks();
  choose.mockReset();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.spyOn(skillsApi, "getRepos").mockResolvedValue([
    { owner: "example", name: "catalog", branch: "main", enabled: true },
  ]);
  vi.spyOn(skillsApi, "discoverRepository").mockResolvedValue([
    item("alpha"),
    item("beta"),
  ]);
});
it("filters by description and exact provenance, and never installs while browsing", async () => {
  page([
    {
      id: "example/catalog:skills/alpha",
      name: "alpha",
      directory: "alpha",
      sourceKind: "repository",
      repository: "example/catalog",
      sourceState: "ready",
      targets: [],
    } as SkillDefinition,
  ]);
  await userEvent.click(screen.getByRole("radio", { name: /仓库/ }));
  const alpha = await screen.findByRole("article", { name: "alpha" });
  expect(within(alpha).getByRole("button", { name: "已收录" })).toBeDisabled();
  await userEvent.click(screen.getByRole("checkbox", { name: "只看未收录" }));
  expect(screen.queryByRole("article", { name: "alpha" })).toBeNull();
  await userEvent.type(screen.getByRole("searchbox"), "beta description");
  const beta = screen.getByRole("article", { name: "beta" });
  expect(choose).not.toHaveBeenCalled();
  await userEvent.click(within(beta).getByRole("button", { name: "选择安装" }));
  expect(choose).toHaveBeenCalledWith(item("beta"));
});
it("renders a bounded page and filters across the entire repository catalog", async () => {
  vi.mocked(skillsApi.discoverRepository).mockResolvedValue(
    Array.from({ length: 501 }, (_, i) =>
      item(`skill-${String(i).padStart(3, "0")}`),
    ),
  );
  page();
  await userEvent.click(screen.getByRole("radio", { name: /仓库/ }));
  await screen.findByRole("article", { name: "skill-000" });
  expect(screen.getAllByRole("article")).toHaveLength(40);
  await userEvent.type(screen.getByRole("searchbox"), "skill-500");
  expect(screen.getAllByRole("article")).toHaveLength(1);
  expect(screen.getByRole("article", { name: "skill-500" })).toBeVisible();
});
it("keeps successful repository results and retries a failed repository explicitly", async () => {
  vi.mocked(skillsApi.getRepos).mockResolvedValue(
    ["example", "other"].map((owner) => ({
      owner,
      name: "catalog",
      branch: "main",
      enabled: true,
    })),
  );
  vi.mocked(skillsApi.discoverRepository).mockImplementation(async (repo) => {
    if (repo.owner === "other") throw new Error("fixture offline");
    return [item("alpha")];
  });
  page();
  await userEvent.click(screen.getByRole("radio", { name: /仓库/ }));
  await screen.findByRole("article", { name: "alpha" });
  expect(await screen.findByRole("alert")).toHaveTextContent("other/catalog");
  vi.mocked(skillsApi.discoverRepository).mockImplementation(async (repo) => [
    item(repo.owner, repo.owner),
  ]);
  await userEvent.click(
    within(screen.getByRole("alert")).getByRole("button", { name: "重试" }),
  );
  await screen.findByRole("article", { name: "other" });
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
});
it("fences skills.sh search and page results by the current query", async () => {
  let release!: (value: any) => void;
  vi.spyOn(skillsApi, "searchSkillsSh").mockImplementation((query) =>
    query === "new"
      ? new Promise((resolve) => {
          release = resolve;
        })
      : Promise.resolve({
          query,
          totalCount: 81,
          skills: [{ ...item("old"), installs: 12 }],
        }),
  );
  page();
  await userEvent.click(screen.getByRole("radio", { name: "skills.sh" }));
  const search = screen.getByRole("searchbox");
  await userEvent.type(search, "old");
  await screen.findByRole("article", { name: "old" });
  await userEvent.click(screen.getByRole("button", { name: "下一页" }));
  await waitFor(() =>
    expect(skillsApi.searchSkillsSh).toHaveBeenCalledWith("old", 40, 40),
  );
  await userEvent.clear(search);
  await userEvent.type(search, "new");
  expect(screen.queryByRole("article", { name: "old" })).toBeNull();
  await waitFor(() => expect(release).toBeTypeOf("function"));
  release({
    query: "new",
    totalCount: 1,
    skills: [{ ...item("new"), installs: 1 }],
  });
  await screen.findByRole("article", { name: "new" });
  expect(skillsApi.searchSkillsSh).toHaveBeenCalledWith("new", 40, 0);
  expect(screen.queryByRole("article", { name: "old" })).toBeNull();
});

it("resolves a skills.sh leaf against HEAD and passes the exact nested source to install review", async () => {
  vi.spyOn(skillsApi, "searchSkillsSh").mockResolvedValue({
    query: "nested",
    totalCount: 1,
    skills: [{ ...item("nested"), directory: "nested", installs: 10 }],
  });
  const actual = { ...item("nested"), repoBranch: "HEAD" };
  vi.mocked(skillsApi.discoverRepository).mockImplementation(async (repo) =>
    repo.branch === "HEAD" ? [actual] : [],
  );
  page();
  await userEvent.click(screen.getByRole("radio", { name: "skills.sh" }));
  await userEvent.type(screen.getByRole("searchbox"), "nested");
  await userEvent.click(
    within(await screen.findByRole("article", { name: "nested" })).getByRole(
      "button",
      { name: "选择安装" },
    ),
  );
  const dialog = await screen.findByRole("dialog", { name: "确认技能来源" });
  expect(await within(dialog).findByText("skills/nested")).toBeVisible();
  expect(choose).not.toHaveBeenCalled();
  await userEvent.click(
    within(dialog).getByRole("button", { name: "使用此来源" }),
  );
  expect(skillsApi.discoverRepository).toHaveBeenCalledWith({
    owner: "example",
    name: "catalog",
    branch: "HEAD",
    enabled: true,
  });
  expect(choose).toHaveBeenCalledWith(actual);
});

it("returns keyboard focus to repository management after Escape", async () => {
  page();
  const manage = screen.getByRole("button", { name: "管理仓库" });
  await userEvent.click(manage);
  await screen.findByRole("dialog", { name: "管理仓库" });
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(manage).toHaveFocus());
});

it("shows popular skills before any search without querying skills.sh", () => {
  const search = vi.spyOn(skillsApi, "searchSkillsSh");
  page();
  const first = POPULAR_SKILLS[0];
  const card = screen.getByRole("article", { name: first.name });
  expect(
    within(card).getByText(`↓ ${formatInstalls(first.installs)}`),
  ).toBeVisible();
  expect(within(card).getByRole("button", { name: "添加" })).toBeEnabled();
  expect(card.parentElement).toHaveClass(
    "grid-cols-2",
    "lg:grid-cols-3",
    "xl:grid-cols-4",
  );
  expect(screen.getByRole("radio", { name: "skills.sh" })).toBeChecked();
  expect(search).not.toHaveBeenCalled();
});

it("shows no undefined text for a skill without a description, and displays zero installs and official metadata", () => {
  const missing = {
    source: "unknown-author/catalog",
    skillId: "missing-description",
    name: "missing-description",
    installs: 0,
    official: true,
  };
  POPULAR_SKILLS.unshift(missing);
  try {
    page();
    const card = screen.getByRole("article", { name: missing.name });
    expect(card).not.toHaveTextContent("undefined");
    expect(within(card).getByText("↓ 0")).toBeVisible();
    expect(within(card).getByLabelText("官方")).toBeVisible();
    expect(within(card).getByText("U")).toBeVisible();
    expect(card.querySelector("img")).toBeNull();
  } finally {
    POPULAR_SKILLS.shift();
  }
});

it("adds a popular skill through the existing skills.sh install path", async () => {
  const first = POPULAR_SKILLS[0];
  const [owner, repo] = first.source.split("/");
  const actual = {
    ...item(first.name, owner),
    repoName: repo,
    key: `${first.source}:nested/${first.skillId}`,
    directory: `nested/${first.skillId}`,
    repoBranch: "HEAD",
  };
  vi.mocked(skillsApi.discoverRepository).mockResolvedValue([actual]);
  page();
  await userEvent.click(
    within(screen.getByRole("article", { name: first.name })).getByRole(
      "button",
      { name: "添加" },
    ),
  );
  const dialog = await screen.findByRole("dialog", { name: "确认技能来源" });
  expect(await within(dialog).findByText(actual.directory)).toBeVisible();
  expect(skillsApi.discoverRepository).toHaveBeenCalledWith({
    owner,
    name: repo,
    branch: "HEAD",
    enabled: true,
  });
  expect(choose).not.toHaveBeenCalled();
  await userEvent.click(
    within(dialog).getByRole("button", { name: "使用此来源" }),
  );
  expect(choose).toHaveBeenCalledWith(actual);
});

it("switches from popular to live search and returns immediately when cleared", async () => {
  vi.spyOn(skillsApi, "searchSkillsSh").mockResolvedValue({
    query: "live",
    totalCount: 1,
    skills: [{ ...item("live-result", "unknown-author"), installs: 0 }],
  });
  page();
  const input = screen.getByRole("searchbox");
  await userEvent.type(input, "l");
  expect(
    screen.queryByRole("article", { name: POPULAR_SKILLS[0].name }),
  ).toBeNull();
  expect(screen.getByText(/输入至少 2/)).toBeVisible();
  await userEvent.type(input, "ive");
  const result = await screen.findByRole("article", { name: "live-result" });
  expect(within(result).getByText("↓ 0")).toBeVisible();
  expect(within(result).getByText("U")).toBeVisible();
  const requests = vi.mocked(skillsApi.searchSkillsSh).mock.calls.length;
  await userEvent.clear(input);
  await userEvent.click(screen.getByRole("button", { name: "刷新发现列表" }));
  expect(skillsApi.searchSkillsSh).toHaveBeenCalledTimes(requests);
  expect(
    screen.getByRole("article", { name: POPULAR_SKILLS[0].name }),
  ).toBeVisible();
  expect(screen.queryByRole("article", { name: "live-result" })).toBeNull();
});

it("returns focus to a popular Add button after cancelling source resolution", async () => {
  page();
  const trigger = within(
    screen.getByRole("article", { name: POPULAR_SKILLS[0].name }),
  ).getByRole("button", { name: "添加" });
  await userEvent.click(trigger);
  await screen.findByRole("dialog", { name: "确认技能来源" });
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(trigger).toHaveFocus());
});

it("keeps a confirmed nested identity collected after rows update and discovery remount, and filters it out", async () => {
  const popular = POPULAR_SKILLS[0];
  const [owner, name] = popular.source.split("/");
  const actual = {
    ...item(popular.name, owner),
    repoName: name,
    key: `${popular.source}:nested/${popular.skillId}`,
    directory: `nested/${popular.skillId}`,
    repoBranch: "HEAD",
  };
  vi.mocked(skillsApi.discoverRepository).mockResolvedValue([actual]);
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = page([], client);
  await userEvent.click(
    within(screen.getByRole("article", { name: popular.name })).getByRole(
      "button",
      { name: "添加" },
    ),
  );
  const dialog = await screen.findByRole("dialog", { name: "确认技能来源" });
  await userEvent.click(
    await within(dialog).findByRole("button", { name: "使用此来源" }),
  );
  expect(choose).toHaveBeenCalledWith(actual);
  view.rerender(discovery([collectedRow(actual)], client));
  expect(
    within(screen.getByRole("article", { name: popular.name })).getByRole(
      "button",
      { name: "已收录" },
    ),
  ).toBeDisabled();
  view.unmount();
  const reopened = page([collectedRow(actual)], client);
  expect(
    within(screen.getByRole("article", { name: popular.name })).getByRole(
      "button",
      { name: "已收录" },
    ),
  ).toBeDisabled();
  await userEvent.click(screen.getByRole("checkbox", { name: "只看未收录" }));
  expect(screen.queryByRole("article", { name: popular.name })).toBeNull();
  reopened.unmount();
  page([collectedRow(actual)]);
  const restarted = screen.getByRole("article", { name: popular.name });
  expect(within(restarted).getByRole("button", { name: "添加" })).toBeEnabled();
  expect(
    within(restarted).queryByRole("button", { name: "已收录" }),
  ).toBeNull();
});

it("keeps unconfirmed popular identities addable and unfiltered even when a row has the same name or slug", async () => {
  const popular = POPULAR_SKILLS[0];
  const [owner, name] = popular.source.split("/");
  const actual = {
    ...item(popular.name, owner),
    repoName: name,
    key: `${popular.source}:${popular.skillId}`,
    directory: popular.skillId,
  };
  page([collectedRow(actual)]);
  const card = screen.getByRole("article", { name: popular.name });
  expect(within(card).queryByText("收录状态待确认")).toBeNull();
  expect(within(card).queryByRole("button", { name: "已收录" })).toBeNull();
  await userEvent.click(screen.getByRole("checkbox", { name: "只看未收录" }));
  expect(screen.getByRole("article", { name: popular.name })).toBeVisible();
});

it("does not merge same-name or same-basename candidates until each upstream identity is explicitly confirmed", async () => {
  const first = {
    source: "example/catalog",
    skillId: "foo",
    name: "same display",
    installs: 20,
    official: false,
    description: "first upstream",
  };
  const second = { ...first, skillId: "bar", description: "second upstream" };
  const actualA = {
    ...item(first.name),
    key: "example/catalog:a/foo",
    directory: "a/foo",
    repoBranch: "HEAD",
  };
  const actualB = {
    ...actualA,
    key: "example/catalog:b/foo",
    directory: "b/foo",
  };
  POPULAR_SKILLS.unshift(first, second);
  try {
    vi.mocked(skillsApi.discoverRepository).mockResolvedValue([
      actualA,
      actualB,
    ]);
    page([collectedRow(actualA)]);
    let cards = screen.getAllByRole("article", { name: first.name });
    await userEvent.click(
      within(cards[0]).getByRole("button", { name: "添加" }),
    );
    let dialog = await screen.findByRole("dialog", { name: "确认技能来源" });
    await within(dialog).findByText("a/foo");
    expect(
      within(dialog).getAllByRole("button", { name: "使用此来源" }),
    ).toHaveLength(2);
    expect(choose).not.toHaveBeenCalled();
    await userEvent.click(
      within(
        within(dialog).getByText("a/foo").parentElement!.parentElement!,
      ).getByRole("button", { name: "使用此来源" }),
    );
    cards = screen.getAllByRole("article", { name: first.name });
    expect(
      within(cards[0]).getByRole("button", { name: "已收录" }),
    ).toBeDisabled();
    expect(
      within(cards[1]).getByRole("button", { name: "添加" }),
    ).toBeEnabled();
    await userEvent.click(
      within(cards[1]).getByRole("button", { name: "添加" }),
    );
    dialog = await screen.findByRole("dialog", { name: "确认技能来源" });
    await userEvent.click(
      within(
        within(dialog).getByText("b/foo").parentElement!.parentElement!,
      ).getByRole("button", { name: "使用此来源" }),
    );
    expect(choose).toHaveBeenLastCalledWith(actualB);
    await userEvent.click(screen.getByRole("checkbox", { name: "只看未收录" }));
    cards = screen.getAllByRole("article", { name: first.name });
    expect(cards).toHaveLength(1);
    expect(cards[0]).toHaveTextContent("second upstream");
    expect(
      within(cards[0]).getByRole("button", { name: "添加" }),
    ).toBeEnabled();
  } finally {
    POPULAR_SKILLS.splice(0, 2);
  }
});

it("uses the same confirmed exact identity for a matching live skills.sh result", async () => {
  const popular = POPULAR_SKILLS[0];
  const [owner, name] = popular.source.split("/");
  const actual = {
    ...item(popular.name, owner),
    repoName: name,
    key: `${popular.source}:nested/${popular.skillId}`,
    directory: `nested/${popular.skillId}`,
    repoBranch: "HEAD",
  };
  vi.mocked(skillsApi.discoverRepository).mockResolvedValue([actual]);
  vi.spyOn(skillsApi, "searchSkillsSh").mockResolvedValue({
    query: "live",
    totalCount: 1,
    skills: [{ ...actual, directory: popular.skillId, installs: 10 }],
  });
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const view = page([], client);
  await userEvent.click(
    within(screen.getByRole("article", { name: popular.name })).getByRole(
      "button",
      { name: "添加" },
    ),
  );
  const dialog = await screen.findByRole("dialog", { name: "确认技能来源" });
  await userEvent.click(
    await within(dialog).findByRole("button", { name: "使用此来源" }),
  );
  view.rerender(discovery([collectedRow(actual)], client));
  await userEvent.type(screen.getByRole("searchbox"), "live");
  const result = await screen.findByRole("article", { name: popular.name });
  expect(within(result).getByRole("button", { name: "已收录" })).toBeDisabled();
});
