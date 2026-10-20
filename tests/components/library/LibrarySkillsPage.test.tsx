import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { beforeEach, expect, it, vi } from "vitest";
import zh from "@/i18n/locales/zh.json";
import {
  libraryApi,
  type LibraryInventory,
  type LibraryOperation,
  type ResourceTarget,
  type SkillDefinition,
  type SkillTarget,
} from "@/lib/api/library";
import { skillsApi, type DiscoverableSkill } from "@/lib/api/skills";
import { settingsApi } from "@/lib/api/settings";
import type { Settings } from "@/types";
import { LibrarySkillsPage } from "@/components/library/LibrarySkillsPage";

const agents = ["claude", "codex", "gemini", "opencode", "hermes", "pi"];
const labels: Record<string, string> = {
  claude: "Claude Code",
  codex: "Codex",
  gemini: "Gemini CLI",
  opencode: "OpenCode",
  hermes: "Hermes",
  pi: "Pi",
};
const target = (
  toolId: string,
  extra: Partial<ResourceTarget> = {},
): ResourceTarget => ({
  toolId,
  path: `~/.${toolId}/skills`,
  targetGroupId: toolId,
  available: true,
  discoveryStatus: "ready",
  status: "unassigned",
  desired: false,
  ...extra,
});
const skill = (
  id: string,
  extra: Partial<SkillDefinition> = {},
  overrides: Record<string, Partial<ResourceTarget>> = {},
): SkillDefinition => ({
  id,
  name: id,
  description: `${id} description`,
  directory: id,
  revision: 3,
  sourceState: "ready",
  targets: agents.map((tool) => target(tool, overrides[tool])),
  ...extra,
});
const operation = (status: string, reason?: string): LibraryOperation => ({
  operationId: `op-${Math.random()}`,
  kind: "skills",
  resourceId: "x",
  resourceRevision: 3,
  status: "previewed",
  targets: [
    {
      tools: ["claude"],
      readers: ["claude"],
      path: "~/.claude/skills/pdf",
      targetGroupId: "claude",
      status,
      reason,
      changedFields: [],
    },
  ],
});
const applied = (): LibraryOperation => ({
  ...operation("applied"),
  status: "applied",
});
let skills: SkillDefinition[];
let inventory: LibraryInventory;
const skillTargets: SkillTarget[] = agents.map((id) => ({
  id,
  label: labels[id],
  detected: true,
  readsShared: ["codex", "gemini", "opencode"].includes(id),
  scopeKnown: true,
}));

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  HTMLElement.prototype.scrollIntoView = vi.fn();
  skills = [
    skill(
      "pdf",
      {
        sourceKind: "repository",
        repository: "anthropics/skills",
        directory: "pdf",
      },
      {
        codex: { desired: true, status: "applied" },
        gemini: { status: "shared" },
        pi: { adoptable: true },
        hermes: {
          status: "conflict",
          disabledReason: "library.skillNameConflict",
        },
      },
    ),
    skill("notes", { sourceKind: "localOrArchive" }),
  ];
  inventory = {
    ticketId: "ticket",
    kind: "skills",
    expiresAtMs: Date.now() + 60_000,
    sources: [],
    entries: [
      {
        entryId: "found-automate",
        kind: "skills",
        name: "automate",
        summary: "Create Codex Automations.",
        sources: [{ sourceId: "s", toolId: "claude" }],
        matchedResourceIds: [],
        conflict: false,
        locations: [
          {
            targetId: "shared",
            displayPath: "~/.agents/skills/automate",
            link: false,
            adoptable: true,
          },
          {
            targetId: "claude",
            displayPath: "~/.claude/skills/automate",
            link: true,
            adoptable: true,
          },
        ],
      },
      {
        entryId: "found-review",
        kind: "skills",
        name: "review",
        sources: [{ sourceId: "s", toolId: "hermes" }],
        matchedResourceIds: [],
        conflict: true,
        locations: [
          {
            targetId: "hermes",
            displayPath: "~/.hermes/skills/review",
            link: false,
            adoptable: false,
          },
        ],
      },
      {
        entryId: "found-pdf",
        kind: "skills",
        name: "pdf",
        sources: [{ sourceId: "s", toolId: "pi" }],
        matchedResourceIds: ["pdf"],
        conflict: false,
      },
    ],
  };
  vi.restoreAllMocks();
  vi.spyOn(libraryApi, "list").mockImplementation(async () => ({
    epoch: 4,
    skills,
    skillTargets,
    operations: [],
  }));
  vi.spyOn(libraryApi, "inventory").mockImplementation(async () => inventory);
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  vi.spyOn(libraryApi, "apply").mockResolvedValue(applied());
  vi.spyOn(libraryApi, "cancel").mockResolvedValue(operation("cancelled"));
  vi.spyOn(settingsApi, "get").mockResolvedValue({
    skillSyncMethod: "symlink",
  } as Settings);
});
function renderPage() {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <LibrarySkillsPage
        registerRefresh={() => () => undefined}
        registerDiscover={() => () => undefined}
        onChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
}
const chip = (name: string, tool: string) =>
  screen.getByRole("button", { name: new RegExp(`^${name} · ${tool} · `) });

it("groups library skills by source with one chip per detected agent", async () => {
  renderPage();
  const row = await screen.findByTestId("skill-row-pdf");
  expect(
    screen.getByRole("heading", { name: /^anthropics\/skills\s+1 个技能$/ }),
  ).toBeVisible();
  expect(
    screen.getByRole("heading", { name: /^本机\s+1 个技能$/ }),
  ).toBeVisible();
  expect(within(row).getByText("pdf description")).toBeVisible();
  expect(chip("pdf", "Codex")).toHaveAttribute("aria-pressed", "true");
  expect(chip("pdf", "Codex")).toHaveAccessibleName(
    "pdf · Codex · Codex 已启用 — 点击移除",
  );
  expect(chip("pdf", "Claude Code")).toHaveAttribute("aria-pressed", "false");
  expect(chip("pdf", "Gemini CLI")).toHaveAttribute("aria-disabled", "true");
  expect(chip("pdf", "Gemini CLI")).toHaveAccessibleName(
    "pdf · Gemini CLI · 从共享技能文件夹读取",
  );
  expect(chip("pdf", "Hermes")).toHaveAttribute("aria-disabled", "true");
  expect(chip("pdf", "Pi")).toHaveAttribute("aria-pressed", "true");
  expect(
    within(row).getAllByRole("button", { name: /^pdf · [^·]+ · / }),
  ).toHaveLength(agents.length);
});

it("gives a skill to an agent with one click", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(chip("pdf", "Claude Code"));
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith(
      "skills",
      "pdf",
      ["codex", "claude"],
      3,
      4,
    ),
  );
  await waitFor(() => expect(libraryApi.apply).toHaveBeenCalledTimes(1));
});

it("takes over an identical skill in place before taking it from the agent", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  const adopt = vi
    .spyOn(libraryApi, "previewSkillAdoption")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(chip("pdf", "Pi"));
  await waitFor(() =>
    expect(adopt).toHaveBeenCalledWith("pdf", ["codex", "pi"], ["pi"], 3, 4),
  );
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith("skills", "pdf", ["codex"], 3, 4),
  );
  expect(libraryApi.apply).toHaveBeenCalledTimes(2);
});

it("refuses to overwrite and says why when a preview hits a conflict", async () => {
  vi.spyOn(libraryApi, "preview").mockResolvedValue(
    operation("conflict", "library.mcpNameConflict"),
  );
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(chip("pdf", "OpenCode"));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "同名原生项目或共享目标已有其他内容，未覆盖",
  );
  expect(libraryApi.cancel).toHaveBeenCalled();
  expect(libraryApi.apply).not.toHaveBeenCalled();
});

it("gives a skill to every agent that can take it from the All chip", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  const adopt = vi
    .spyOn(libraryApi, "previewSkillAdoption")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(
    screen.getByRole("button", {
      name: "pdf · 给全部 4 个可用的 Agent 启用",
    }),
  );
  await waitFor(() =>
    expect(adopt).toHaveBeenCalledWith("pdf", ["codex", "pi"], ["pi"], 3, 4),
  );
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith(
      "skills",
      "pdf",
      ["codex", "claude", "opencode", "pi"],
      3,
      4,
    ),
  );
});

it("withdraws a skill from its agents before removing it from the library", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  const remove = vi
    .spyOn(libraryApi, "deleteSkill")
    .mockResolvedValue(true as never);
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(screen.getByRole("button", { name: "移除 pdf" }));
  const dialog = await screen.findByRole("dialog");
  expect(within(dialog).getByText("移除 pdf？")).toBeVisible();
  await userEvent.click(within(dialog).getByRole("button", { name: "移除" }));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("pdf", 3, 4));
  expect(preview).toHaveBeenCalledWith("skills", "pdf", [], 3, 4);
  expect(preview.mock.invocationCallOrder[0]).toBeLessThan(
    remove.mock.invocationCallOrder[0],
  );
});

it("lists skills the agents have outside the library and brings one in", async () => {
  const imported = vi.spyOn(libraryApi, "importInventory").mockResolvedValue({
    imported: 1,
    items: [
      {
        entryId: "found-automate",
        resourceId: "automate",
        name: "automate",
        status: "imported",
        adopted: ["shared", "claude"],
      },
    ],
  });
  renderPage();
  const row = await screen.findByTestId("found-row-found-automate");
  expect(screen.queryByTestId("found-row-found-pdf")).toBeNull();
  expect(
    within(row).getByText("共享于 ~/.agents/skills/automate"),
  ).toBeVisible();
  expect(within(row).getByText("~/.claude/skills/automate")).toBeVisible();
  expect(
    within(screen.getByTestId("found-row-found-review")).getByText(
      "与资源库中的同名技能不同",
    ),
  ).toBeVisible();
  await userEvent.click(
    within(row).getByRole("button", { name: "导入 automate" }),
  );
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("ticket", ["found-automate"], 4),
  );
  expect(await screen.findByText("automate 已加入资源库")).toBeVisible();
});

it("brings in every found skill but the conflicting ones", async () => {
  const imported = vi
    .spyOn(libraryApi, "importInventory")
    .mockResolvedValue({ imported: 1, items: [] });
  renderPage();
  await screen.findByTestId("found-row-found-automate");
  await userEvent.click(screen.getByRole("button", { name: "全部导入" }));
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("ticket", ["found-automate"], 4),
  );
  expect(
    await screen.findByText(/1 个与资源库中的同名技能不同，已跳过/),
  ).toBeVisible();
});

it("deletes a found skill from the agents after asking", async () => {
  const removed = vi.spyOn(libraryApi, "removeFoundSkill").mockResolvedValue({
    removed: [
      { targetId: "shared", displayPath: "~/.agents/skills/automate" },
      { targetId: "claude", displayPath: "~/.claude/skills/automate" },
    ],
  });
  renderPage();
  await screen.findByTestId("found-row-found-automate");
  await userEvent.click(
    screen.getByRole("button", { name: "从 Agent 中删除 automate" }),
  );
  const dialog = await screen.findByRole("dialog");
  expect(
    within(dialog).getByText("会先备份到 ~/.ofox-desktop/skill-backups"),
  ).toBeVisible();
  await userEvent.click(within(dialog).getByRole("button", { name: "删除" }));
  await waitFor(() =>
    expect(removed).toHaveBeenCalledWith("ticket", "found-automate", [], 4),
  );
  expect(await screen.findByText("已从 2 处删除 automate")).toBeVisible();
});

it("looks a saved repository up without changing how it is saved", async () => {
  vi.spyOn(skillsApi, "getRepos").mockResolvedValue([
    {
      owner: "ComposioHQ",
      name: "awesome-claude-skills",
      branch: "master",
      enabled: true,
    },
  ]);
  const addRepo = vi.spyOn(skillsApi, "addRepo").mockResolvedValue(true);
  const removeRepo = vi.spyOn(skillsApi, "removeRepo").mockResolvedValue(true);
  vi.spyOn(skillsApi, "discoverAvailable").mockResolvedValue([
    {
      key: "ComposioHQ/awesome-claude-skills:canvas",
      name: "canvas",
      description: "Canvas",
      directory: "canvas",
      repoOwner: "ComposioHQ",
      repoName: "awesome-claude-skills",
      repoBranch: "master",
    },
  ]);
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "ComposioHQ/awesome-claude-skills",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  expect(
    await screen.findByText("ComposioHQ/awesome-claude-skills 中有 1 个技能"),
  ).toBeVisible();
  expect(addRepo).not.toHaveBeenCalled();
  expect(removeRepo).not.toHaveBeenCalled();
});

it("finds the skills in a repository and installs the picked ones for the picked agents", async () => {
  const discovered = (directory: string): DiscoverableSkill => ({
    key: `anthropics/skills:skills/${directory}`,
    name: directory,
    description: `${directory} from GitHub`,
    directory: `skills/${directory}`,
    repoOwner: "anthropics",
    repoName: "skills",
    repoBranch: "main",
  });
  vi.spyOn(skillsApi, "getRepos").mockResolvedValue([]);
  const addRepo = vi.spyOn(skillsApi, "addRepo").mockResolvedValue(true);
  const removeRepo = vi.spyOn(skillsApi, "removeRepo").mockResolvedValue(true);
  vi.spyOn(skillsApi, "discoverAvailable").mockResolvedValue([
    discovered("pdf"),
    discovered("xlsx"),
    { ...discovered("other"), repoOwner: "someone" },
  ]);
  const installed = vi
    .spyOn(skillsApi, "installUnified")
    .mockImplementation(async (item) => {
      skills = [
        ...skills,
        skill(item.key, { name: item.name, directory: item.name, revision: 1 }),
      ];
      return { id: item.key } as never;
    });
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "anthropics/skills",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  expect(
    await screen.findByText("anthropics/skills 中有 2 个技能"),
  ).toBeVisible();
  // Added only for the lookup, on the repository's default branch, then taken out again.
  expect(addRepo).toHaveBeenCalledWith({
    owner: "anthropics",
    name: "skills",
    branch: "",
    enabled: true,
  });
  expect(removeRepo).toHaveBeenCalledWith("anthropics", "skills");
  expect(screen.getByRole("checkbox", { name: "选择 pdf" })).toBeDisabled();
  expect(screen.getByText("已在资源库")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "安装后给 Hermes 启用" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "安装 1 个技能" }));
  await waitFor(() =>
    expect(installed).toHaveBeenCalledWith(
      expect.objectContaining({ key: "anthropics/skills:skills/xlsx" }),
      "claude",
      4,
    ),
  );
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith(
      "skills",
      "anthropics/skills:skills/xlsx",
      ["claude", "codex", "opencode", "gemini", "pi"],
      1,
      4,
    ),
  );
  expect(await screen.findByText("已安装 1 个技能")).toBeVisible();
});

it("imports one local folder as a skill", async () => {
  const imported = vi
    .spyOn(libraryApi, "importSkillDirectory")
    .mockImplementation(async () => {
      skills = [...skills, skill("local:1", { name: "research" })];
      return { id: "local:1" };
    });
  vi.spyOn(libraryApi, "preview").mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "/Users/me/skills/research",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "安装 1 个技能" }),
  );
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith(
      "/Users/me/skills/research",
      "research",
      "research",
    ),
  );
});

it("says so when a folder has no SKILL.md", async () => {
  vi.spyOn(libraryApi, "importSkillDirectory").mockRejectedValue(
    "library.skillDocumentMissing",
  );
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "/tmp/empty",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "安装 1 个技能" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "已跳过 empty：那里没有找到 SKILL.md",
  );
});

it("saves how skills are given to the agents", async () => {
  const save = vi.spyOn(settingsApi, "save").mockResolvedValue(true);
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  const method = screen.getByRole("radiogroup", { name: "Skills 启用方式" });
  await waitFor(() =>
    expect(
      within(method).getByRole("radio", { name: "符号链接" }),
    ).toBeEnabled(),
  );
  expect(
    within(method).getByRole("radio", { name: "符号链接" }),
  ).toHaveAttribute("aria-checked", "true");
  expect(
    screen.getByText(
      "符号链接跟着资源库里的 Skill 走：一更新，各 Agent 立刻就用上。",
    ),
  ).toBeVisible();
  await userEvent.click(within(method).getByRole("radio", { name: "副本" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ skillSyncMethod: "copy" }),
    ),
  );
});

it("counts a write that landed as done even when tidying up after it needs a look", async () => {
  vi.spyOn(libraryApi, "preview").mockResolvedValue(operation("pending"));
  vi.mocked(libraryApi.apply).mockResolvedValue({
    ...operation("applied", "library.cleanupNeedsReview"),
    status: "needsReview",
  });
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(chip("pdf", "Claude Code"));
  await waitFor(() => expect(libraryApi.apply).toHaveBeenCalledTimes(1));
  expect(screen.queryByRole("alert")).toBeNull();
});

it("withdraws what a failed write left behind before removing a skill", async () => {
  skills = [
    skill(
      "left",
      {},
      { claude: { status: "needsReview", lastAppliedRevision: 3 } },
    ),
  ];
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  const remove = vi
    .spyOn(libraryApi, "deleteSkill")
    .mockResolvedValue(true as never);
  renderPage();
  await screen.findByTestId("skill-row-left");
  await userEvent.click(screen.getByRole("button", { name: "移除 left" }));
  const dialog = await screen.findByRole("dialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "移除" }));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("left", 3, 4));
  expect(preview).toHaveBeenCalledWith("skills", "left", [], 3, 4);
});

it("names the skill that was brought in when others in the batch were not", async () => {
  vi.spyOn(libraryApi, "importInventory").mockResolvedValue({
    imported: 1,
    items: [
      {
        entryId: "found-review",
        resourceId: "review",
        name: "review",
        status: "existing",
      },
      {
        entryId: "found-automate",
        resourceId: "automate",
        name: "automate",
        status: "imported",
      },
    ],
  });
  renderPage();
  await screen.findByTestId("found-row-found-automate");
  await userEvent.click(screen.getByRole("button", { name: "全部导入" }));
  expect(await screen.findByText(/automate 已加入资源库/)).toBeVisible();
  expect(screen.queryByText(/review 已加入资源库/)).toBeNull();
});
