import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { useRef, useState } from "react";
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
import { InteractionScope } from "@/components/ui/interaction-scope";

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
  artifactHash: "a".repeat(64),
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
function SkillsFixture({ active = true }: { active?: boolean }) {
  const [client] = useState(
    () => new QueryClient({ defaultOptions: { queries: { retry: false } } }),
  );
  const open = useRef<(trigger: HTMLElement) => void>(() => undefined);
  return (
    <QueryClientProvider client={client}>
      <InteractionScope active={active}>
        <div data-testid="skills-scroller" style={{ overflowY: "auto" }}>
          <button onClick={(event) => open.current(event.currentTarget)}>
            发现
          </button>
          <LibrarySkillsPage
            registerRefresh={() => () => undefined}
            registerDiscover={(_kind, callback) => {
              open.current = callback;
              return () => undefined;
            }}
            onChanged={vi.fn()}
          />
        </div>
      </InteractionScope>
    </QueryClientProvider>
  );
}
function renderPage() {
  return render(<SkillsFixture />);
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
  expect(chip("pdf", "Hermes")).not.toHaveAttribute("aria-disabled");
  expect(chip("pdf", "Pi")).toHaveAttribute("aria-pressed", "true");
  expect(
    within(row).getAllByRole("button", { name: /^pdf · [^·]+ · / }),
  ).toHaveLength(agents.length);
});

it("uses the bindable Skills scope for row chips, All, and install choices", async () => {
  const extras = ["cursor", "codebuddy", "claude-desktop"];
  skills = [
    skill("pdf", {
      targets: [...agents, "openclaw", ...extras].map((id) => target(id)),
    }),
  ];
  vi.mocked(libraryApi.list).mockImplementation(async () => ({
    epoch: 4,
    skills,
    operations: [],
    skillTargets: [
      ...skillTargets,
      ...["openclaw", ...extras].map((id) => ({
        id,
        label: id,
        detected: true,
        readsShared: false,
        scopeKnown: true,
      })),
    ],
  }));
  const preview = vi
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockResolvedValue(operation("pending"));
  renderPage();
  const row = await screen.findByTestId("skill-row-pdf");
  for (const id of extras)
    expect(
      within(row).queryByRole("button", {
        name: new RegExp(`^pdf · ${id} · `),
      }),
    ).not.toBeInTheDocument();
  expect(chip("pdf", "openclaw")).toBeVisible();
  await userEvent.click(
    within(row).getByRole("button", { name: /给全部 .* 个可用的 Agent 启用/ }),
  );
  await waitFor(() => expect(preview).toHaveBeenCalled());
  const given = preview.mock.calls.flatMap((call) => call[1]);
  expect(given).toEqual(expect.arrayContaining([...agents, "openclaw"]));
  for (const id of extras) expect(given).not.toContain(id);

  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "/sandbox/local-skill",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  for (const id of extras)
    expect(
      screen.queryByRole("button", { name: `安装后给 ${id} 启用` }),
    ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "安装后给 openclaw 启用" }),
  ).toBeVisible();
});

it("gives a skill to an agent with one click", async () => {
  const preview = vi
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(chip("pdf", "Claude Code"));
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith("pdf", ["claude"], [], 3, 4),
  );
  await waitFor(() => expect(libraryApi.apply).toHaveBeenCalledTimes(1));
});
it("appends an existing skill to B without rewriting a conflicted A", async () => {
  skills = [
    skill(
      "pdf",
      {},
      {
        codex: {
          desired: true,
          status: "conflict",
          disabledReason: "library.skillEntryChanged",
        },
      },
    ),
  ];
  const append = vi
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockResolvedValue(operation("pending"));
  const replace = vi.spyOn(libraryApi, "preview");
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(chip("pdf", "Claude Code"));
  await waitFor(() => expect(libraryApi.apply).toHaveBeenCalledTimes(1));
  expect(append).toHaveBeenCalledWith("pdf", ["claude"], [], 3, 4);
  expect(replace).not.toHaveBeenCalled();
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
  vi.spyOn(libraryApi, "previewSkillDistribution").mockResolvedValue(
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
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(
    screen.getByRole("button", {
      name: "pdf · 给全部 4 个可用的 Agent 启用",
    }),
  );
  await waitFor(() => expect(preview).toHaveBeenCalledTimes(3));
  expect(preview).toHaveBeenCalledWith("pdf", ["claude"], [], 3, 4);
  expect(preview).toHaveBeenCalledWith("pdf", ["opencode"], [], 3, 4);
  expect(preview).toHaveBeenCalledWith("pdf", ["pi"], ["pi"], 3, 4);
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

  await userEvent.click(
    within(row).getByRole("button", { name: "导入 automate" }),
  );
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("ticket", ["found-automate"], 4),
  );
  expect(await screen.findByText("automate 已加入资源库")).toBeVisible();
});

it("brings in one chosen version of every found skill, including different copies", async () => {
  const imported = vi
    .spyOn(libraryApi, "importInventory")
    .mockResolvedValue({ imported: 2, items: [] });
  renderPage();
  await screen.findByTestId("found-row-found-automate");
  await userEvent.click(screen.getByRole("button", { name: "全部导入" }));
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith(
      "ticket",
      ["found-automate", "found-review"],
      4,
    ),
  );
  expect(await screen.findByText(/2 个技能已加入资源库/)).toBeVisible();
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
  vi.spyOn(skillsApi, "discoverRepository").mockResolvedValue([
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
  skills[0].sourceUrl = `https://github.com/anthropics/skills/blob/${"a".repeat(40)}/skills/pdf/SKILL.md`;
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
  vi.spyOn(skillsApi, "discoverRepository").mockResolvedValue([
    discovered("pdf"),
    discovered("xlsx"),
    { ...discovered("other"), repoOwner: "someone" },
  ]);
  const installed = vi
    .spyOn(skillsApi, "collect")
    .mockImplementation(async (item) => {
      skills = [
        ...skills,
        skill(item.key, { name: item.name, directory: item.name, revision: 1 }),
      ];
      return {
        id: item.key,
        epoch: 4,
        revision: 1,
        artifactHash: "a".repeat(64),
      };
    });
  const preview = vi
    .spyOn(libraryApi, "previewSkillDistribution")
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
  expect(addRepo).not.toHaveBeenCalled();
  expect(removeRepo).not.toHaveBeenCalled();
  expect(screen.getByRole("checkbox", { name: "选择 pdf" })).toBeDisabled();
  expect(screen.getByText("已在资源库")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "安装后给 Hermes 启用" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "安装 1 个技能" }));
  await waitFor(() =>
    expect(installed).toHaveBeenCalledWith(
      expect.objectContaining({ key: "anthropics/skills:skills/xlsx" }),
      4,
    ),
  );
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith(
      "anthropics/skills:skills/xlsx",
      ["pi"],
      [],
      1,
      4,
    ),
  );
  expect(await screen.findByText("已安装 1 个技能")).toBeVisible();
  expect(preview).toHaveBeenCalledTimes(5);
});

it("retains a collected download and retries only failed distribution targets", async () => {
  const collected = vi
    .spyOn(skillsApi, "collect")
    .mockImplementation(async (item) => {
      skills.push(skill(item.key, { name: item.name, revision: 1 }));
      return {
        id: item.key,
        epoch: 4,
        revision: 1,
        artifactHash: "a".repeat(64),
      };
    });
  let fail = true;
  const preview = vi
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockImplementation(async (_id, tools) =>
      operation(
        fail && tools.includes("claude") ? "conflict" : "pending",
        "library.skillNameConflict",
      ),
    );
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(screen.getByRole("button", { name: "发现" }));
  await userEvent.click(screen.getByRole("radio", { name: "精选" }));
  await userEvent.click(
    screen.getByRole("button", { name: "选择 frontend-design" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "安装 1 个技能" }));
  await waitFor(() => expect(preview).toHaveBeenCalledTimes(6));
  expect(collected).toHaveBeenCalledTimes(1);
  expect(await screen.findByText(/Claude Code.*冲突/)).toBeVisible();
  fail = false;
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "安装 1 个技能" })).toBeEnabled(),
  );
  await userEvent.click(screen.getByRole("button", { name: "安装 1 个技能" }));
  await waitFor(() => expect(preview).toHaveBeenCalledTimes(7));
  expect(collected).toHaveBeenCalledTimes(1);
  expect(preview.mock.calls.at(-1)?.[1]).toEqual(["claude"]);
});

it("does not treat the same leaf in another repository as already installed", async () => {
  vi.spyOn(skillsApi, "getRepos").mockResolvedValue([]);
  vi.spyOn(skillsApi, "addRepo").mockResolvedValue(true);
  vi.spyOn(skillsApi, "removeRepo").mockResolvedValue(true);
  vi.spyOn(skillsApi, "discoverRepository").mockResolvedValue([
    {
      key: "another/repo:nested/pdf",
      name: "pdf",
      description: "another source",
      directory: "nested/pdf",
      repoOwner: "another",
      repoName: "repo",
      repoBranch: "main",
    },
  ]);
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "another/repo",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  expect(
    await screen.findByRole("checkbox", { name: "选择 pdf" }),
  ).toBeEnabled();
});

it("imports one local folder as a skill", async () => {
  const imported = vi
    .spyOn(libraryApi, "importSkillDirectory")
    .mockImplementation(async () => {
      skills = [...skills, skill("local:1", { name: "research" })];
      return {
        id: "local:1",
        epoch: 4,
        revision: 3,
        artifactHash: "a".repeat(64),
      };
    });
  vi.spyOn(libraryApi, "previewSkillDistribution").mockResolvedValue(
    operation("pending"),
  );
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
      4,
    ),
  );
  const result = await screen.findByRole("region", { name: "安装结果" });
  expect(within(result).getByText("已在资源库")).toBeVisible();
  expect(within(result).queryByText("已收录")).toBeNull();
});

it("shows an explicit repository failure without changing saved repositories", async () => {
  vi.spyOn(skillsApi, "discoverRepository").mockRejectedValue(
    "library.skillDownloadFailed",
  );
  const save = vi.spyOn(skillsApi, "addRepo");
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "offline/repo",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    i18n.t("library.skillDownloadFailed"),
  );
  expect(screen.queryByText(i18n.t("library.skillsPage.noSkills"))).toBeNull();
  expect(save).not.toHaveBeenCalled();
});

it("collects a ZIP once and distributes independent entries while retaining a conflict", async () => {
  const collected = vi
    .spyOn(skillsApi, "collectZip")
    .mockImplementation(async () => {
      skills.push(skill("archive:one", { name: "one", revision: 1 }));
      return [
        {
          name: "one",
          directory: "one",
          status: "collected",
          collected: {
            id: "archive:one",
            epoch: 4,
            revision: 1,
            artifactHash: "a".repeat(64),
          },
        },
        {
          name: "conflicting",
          directory: "conflicting",
          status: "conflict",
          reason: "library.skillNameConflict",
        },
      ];
    });
  const preview = vi
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockResolvedValue(operation("pending"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "/tmp/skills.zip",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "安装 1 个技能" }),
  );
  await waitFor(() => expect(preview).toHaveBeenCalledTimes(6));
  expect(collected).toHaveBeenCalledTimes(1);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    i18n.t("library.skillNameConflict"),
  );
});

it("keeps successful and skipped ZIP results visible until dismissed", async () => {
  vi.spyOn(skillsApi, "collectZip").mockImplementation(async () => {
    skills.push(
      skill("archive:new", { name: "new", revision: 1 }),
      skill(
        "archive:kept",
        { name: "kept", revision: 1 },
        Object.fromEntries(
          agents.map((tool) => [tool, { desired: true, status: "applied" }]),
        ),
      ),
    );
    return ["new", "kept"].map((name) => ({
      name,
      directory: `nested/${name}`,
      status: name === "new" ? ("collected" as const) : ("skipped" as const),
      collected: {
        id: `archive:${name}`,
        epoch: 4,
        revision: 1,
        artifactHash: "a".repeat(64),
      },
    }));
  });
  vi.spyOn(libraryApi, "previewSkillDistribution").mockResolvedValue(
    operation("pending"),
  );
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.type(
    screen.getByRole("textbox", { name: "技能来源" }),
    "/tmp/skills.zip",
  );
  await userEvent.click(screen.getByRole("button", { name: "查找技能" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "安装 1 个技能" }),
  );
  const results = await screen.findByRole("region", { name: "安装结果" });
  expect(within(results).getByText("new")).toBeVisible();
  expect(within(results).getByText("kept")).toBeVisible();
  expect(within(results).getByText("已收录")).toBeVisible();
  expect(within(results).getByText("已在资源库，跳过收录")).toBeVisible();
  expect(within(results).getByText("Claude Code：已分发")).toBeVisible();
  expect(within(results).getByText("Claude Code：已生效，跳过")).toBeVisible();
  await userEvent.click(within(results).getByRole("button", { name: "关闭" }));
  expect(screen.queryByRole("region", { name: "安装结果" })).toBeNull();
  expect(screen.getByRole("textbox", { name: "技能来源" })).toHaveFocus();
  expect(screen.getByTestId("skill-row-archive:new")).toBeVisible();
});

it("refuses distribution retries after the library epoch changes", async () => {
  vi.spyOn(skillsApi, "collect").mockImplementation(async (item) => {
    skills.push(skill(item.key, { name: item.name, revision: 1 }));
    return {
      id: item.key,
      epoch: 4,
      revision: 1,
      artifactHash: "a".repeat(64),
    };
  });
  const preview = vi
    .spyOn(libraryApi, "previewSkillDistribution")
    .mockResolvedValue(operation("conflict", "library.skillNameConflict"));
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(screen.getByRole("button", { name: "发现" }));
  await userEvent.click(screen.getByRole("radio", { name: "精选" }));
  await userEvent.click(
    screen.getByRole("button", { name: "选择 frontend-design" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "安装 1 个技能" }));
  await waitFor(() => expect(preview).toHaveBeenCalledTimes(6));
  vi.mocked(libraryApi.list).mockResolvedValue({
    epoch: 5,
    skills,
    skillTargets,
    operations: [],
  });
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "安装 1 个技能" })).toBeEnabled(),
  );
  await userEvent.click(screen.getByRole("button", { name: "安装 1 个技能" }));
  expect(
    await screen.findByText(/Claude Code：失败.*应用数据已恢复/),
  ).toBeVisible();
  expect(preview).toHaveBeenCalledTimes(6);
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
    "那里没有找到 SKILL.md",
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
  vi.spyOn(libraryApi, "previewSkillDistribution").mockResolvedValue(
    operation("pending"),
  );
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

it("merges same-name variants, prefers the shared source and imports only that version", async () => {
  const shared = {
    ...inventory.entries[0],
    entryId: "shared-review",
    name: "review",
    conflict: true,
    locations: [
      {
        targetId: "shared",
        displayPath: "~/.agents/skills/review",
        link: false,
        adoptable: true,
      },
      {
        targetId: "claude",
        displayPath: "~/.claude/skills/review",
        link: true,
        adoptable: true,
      },
    ],
  };
  inventory.entries = [inventory.entries[1], shared];
  const imported = vi
    .spyOn(libraryApi, "importInventory")
    .mockResolvedValue({ imported: 1, items: [] });
  renderPage();
  const row = await screen.findByTestId("found-row-shared-review");
  expect(screen.queryByTestId("found-row-found-review")).toBeNull();
  expect(within(row).getByText("Hermes 中不同")).toBeVisible();
  expect(
    within(row).getByRole("button", { name: "导入 review" }),
  ).toBeEnabled();
  await userEvent.click(
    within(row).getByRole("button", { name: "导入 review" }),
  );
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("ticket", ["shared-review"], 4),
  );
});

it("keeps a different same-name native copy on the library row rather than offering a duplicate import", async () => {
  inventory.entries.push({
    ...inventory.entries[1],
    entryId: "different-pdf",
    name: "pdf",
  });
  renderPage();
  const row = await screen.findByTestId("skill-row-pdf");
  await screen.findByTestId("found-row-found-automate");
  expect(screen.queryByTestId("found-row-different-pdf")).toBeNull();
  expect(
    within(row).getByRole("button", { name: "pdf · Hermes 中不同" }),
  ).toBeEnabled();
  await userEvent.click(
    within(row).getByRole("button", { name: "pdf · Hermes 中不同" }),
  );
  const dialog = await screen.findByRole("dialog");
  expect(
    within(dialog).getByRole("button", { name: "用资源库的" }),
  ).toBeVisible();
  expect(
    within(dialog).getByRole("button", { name: "保留 Hermes 自己的" }),
  ).toBeVisible();
});

it.each([true, false])(
  "resolves a different native version only after a deliberate choice (use library: %s)",
  async (useLibrary) => {
    const preview = vi
      .spyOn(libraryApi, "previewSkillReplacement")
      .mockResolvedValue(operation("pending"));
    const keep = vi
      .spyOn(libraryApi, "keepSkillNative")
      .mockResolvedValue(true);
    renderPage();
    const opener = await screen.findByRole("button", {
      name: "pdf · Hermes 中不同",
    });
    await userEvent.click(opener);
    const dialog = await screen.findByRole("dialog");
    expect(preview).not.toHaveBeenCalled();
    expect(keep).not.toHaveBeenCalled();
    await userEvent.click(
      within(dialog).getByRole("button", {
        name: useLibrary ? "用资源库的" : "保留 Hermes 自己的",
      }),
    );
    await waitFor(() =>
      expect(useLibrary ? preview : keep).toHaveBeenCalledWith(
        "pdf",
        "hermes",
        3,
        4,
      ),
    );
    if (useLibrary) {
      expect(keep).not.toHaveBeenCalled();
      await waitFor(() => expect(libraryApi.apply).toHaveBeenCalled());
    } else {
      expect(preview).not.toHaveBeenCalled();
      expect(libraryApi.apply).not.toHaveBeenCalled();
    }
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(opener).toHaveFocus();
  },
);

it("cancels a conflict choice with Escape and restores keyboard focus without writes", async () => {
  const preview = vi.spyOn(libraryApi, "previewSkillReplacement");
  const keep = vi.spyOn(libraryApi, "keepSkillNative");
  renderPage();
  const opener = await screen.findByRole("button", {
    name: "pdf · Hermes 中不同",
  });
  await userEvent.click(opener);
  await screen.findByRole("dialog");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(opener).toHaveFocus();
  expect(preview).not.toHaveBeenCalled();
  expect(keep).not.toHaveBeenCalled();
});

it("does not apply a replacement if the new preview reports changed ownership", async () => {
  vi.spyOn(libraryApi, "previewSkillReplacement").mockResolvedValue(
    operation("conflict", "library.ownershipChanged"),
  );
  renderPage();
  await userEvent.click(
    await screen.findByRole("button", { name: "pdf · Hermes 中不同" }),
  );
  await userEvent.click(
    within(await screen.findByRole("dialog")).getByRole("button", {
      name: "用资源库的",
    }),
  );
  await waitFor(() => expect(libraryApi.cancel).toHaveBeenCalled());
  expect(libraryApi.apply).not.toHaveBeenCalled();
});

it("keeps distinct folder identities visible even when SKILL.md gives them the same display name", async () => {
  inventory.entries = [
    {
      ...inventory.entries[0],
      entryId: "review-one",
      name: "review",
      directory: "review-one",
    },
    {
      ...inventory.entries[1],
      entryId: "review-two",
      name: "review",
      directory: "review-two",
    },
  ];
  skills = [skill("review-one", { name: "review", directory: "review-one" })];
  renderPage();
  expect(await screen.findByTestId("found-row-review-two")).toBeVisible();
  expect(screen.queryByTestId("found-row-review-one")).toBeNull();
});

it("keeps differences in nested read-only locations visible on the library row", async () => {
  skills = [skill("review")];
  inventory.entries = [
    { ...inventory.entries[1], name: "review", directory: "review" },
  ];
  renderPage();
  const row = await screen.findByTestId("skill-row-review");
  await waitFor(() =>
    expect(within(row).getByText("Hermes 中不同")).toBeVisible(),
  );
  expect(
    within(row).queryByRole("button", { name: "review · Hermes 中不同" }),
  ).toBeNull();
  expect(screen.queryByTestId("found-row-found-review")).toBeNull();
});

it("moves focus to the stable source field when an imported found row disappears", async () => {
  vi.spyOn(libraryApi, "importInventory").mockImplementation(async () => {
    skills.push(skill("automate"));
    inventory.entries = inventory.entries.filter(
      (entry) => entry.entryId !== "found-automate",
    );
    return { imported: 1, items: [] };
  });
  renderPage();
  await userEvent.click(
    await screen.findByRole("button", { name: "导入 automate" }),
  );
  await screen.findByTestId("skill-row-automate");
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "技能来源" })).toHaveFocus(),
  );
});

it("searches repository skills with provenance and chooses through the existing install review", async () => {
  vi.spyOn(skillsApi, "getRepos").mockResolvedValue([
    { owner: "example", name: "catalog", branch: "main", enabled: true },
  ]);
  const item: DiscoverableSkill = {
    key: "example/catalog:skills/research",
    name: "Research helper",
    description: "Find reliable evidence",
    directory: "skills/research",
    repoOwner: "example",
    repoName: "catalog",
    repoBranch: "main",
  };
  vi.spyOn(skillsApi, "discoverRepository").mockResolvedValue([item]);
  const collect = vi.spyOn(skillsApi, "collect");
  renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(screen.getByRole("button", { name: "发现" }));
  const search = await screen.findByRole("searchbox", {
    name: "搜索 Skill、描述或仓库",
  });
  await userEvent.type(search, "evidence");
  const row = await screen.findByRole("article", { name: "Research helper" });
  expect(within(row).getByText("example/catalog")).toBeVisible();
  await userEvent.click(within(row).getByRole("button", { name: "选择安装" }));
  expect(
    await screen.findByRole("checkbox", { name: /Research helper/ }),
  ).toBeChecked();
  expect(collect).not.toHaveBeenCalled();
  expect(screen.queryByRole("searchbox")).toBeNull();
  expect(screen.getByRole("textbox", { name: "技能来源" })).toHaveFocus();
});

it("opens discovery on demand and returns to the saved Skills list without losing the search", async () => {
  const repos = vi
    .spyOn(skillsApi, "getRepos")
    .mockResolvedValue([
      { owner: "example", name: "catalog", branch: "main", enabled: true },
    ]);
  const catalog = vi
    .spyOn(skillsApi, "discoverRepository")
    .mockResolvedValue([]);
  renderPage();
  const row = await screen.findByTestId("skill-row-pdf");
  expect(screen.queryByRole("searchbox")).toBeNull();
  expect(repos).not.toHaveBeenCalled();
  expect(catalog).not.toHaveBeenCalled();
  const trigger = screen.getByRole("button", { name: "发现" });
  const scroller = screen.getByTestId("skills-scroller");
  scroller.scrollTop = 1200;
  await userEvent.click(trigger);
  const search = await screen.findByRole("searchbox", {
    name: "搜索 Skill、描述或仓库",
  });
  expect(search).toHaveFocus();
  expect(row).not.toBeVisible();
  expect(screen.queryByRole("region", { name: "Agent 中已有" })).toBeNull();
  await userEvent.type(search, "evidence");
  await waitFor(() => expect(catalog).toHaveBeenCalledTimes(1));
  scroller.scrollTop = 300;
  await userEvent.click(screen.getByRole("button", { name: "返回 Skills" }));
  expect(row).toBeVisible();
  expect(screen.queryByRole("searchbox")).toBeNull();
  expect(scroller.scrollTop).toBe(1200);
  expect(trigger).toHaveFocus();
  await userEvent.click(trigger);
  expect(screen.getByRole("searchbox")).toHaveValue("evidence");
  expect(screen.getByRole("searchbox")).toHaveFocus();
  expect(catalog).toHaveBeenCalledTimes(1);
});

it("dismisses discovery dialogs when the retained Skills page becomes inactive", async () => {
  vi.spyOn(skillsApi, "getRepos").mockResolvedValue([]);
  const view = renderPage();
  await screen.findByTestId("skill-row-pdf");
  await userEvent.click(screen.getByRole("button", { name: "发现" }));
  await userEvent.click(screen.getByRole("button", { name: "管理仓库" }));
  await screen.findByRole("dialog", { name: "管理仓库" });
  view.rerender(<SkillsFixture active={false} />);
  expect(screen.queryByRole("dialog")).toBeNull();
  view.rerender(<SkillsFixture active />);
  expect(screen.queryByRole("dialog")).toBeNull();
});
