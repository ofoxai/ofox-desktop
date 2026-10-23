import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { LibrarySkillDiscover } from "@/components/library/LibrarySkillDiscover";
import { skillsApi, type DiscoverableSkill } from "@/lib/api/skills";
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
function page(rows: SkillDefinition[] = []) {
  return render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <LibrarySkillDiscover
        rows={rows}
        busy={false}
        onChoose={choose}
        onChooseRecipe={vi.fn()}
        onOpenSource={vi.fn()}
      />
    </QueryClientProvider>,
  );
}
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
