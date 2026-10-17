import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import { useEffect, useState } from "react";
import { InteractionScope } from "@/components/ui/interaction-scope";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { LibraryPage } from "@/components/library/LibraryPage";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import {
  libraryApi,
  type LibraryData,
  type LibraryKind,
} from "@/lib/api/library";
import type { McpCheckProgress, McpCheckResult } from "@/lib/api/library";
import { promptsApi } from "@/lib/api/prompts";
import { listen } from "@tauri-apps/api/event";
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));

vi.mock("@/components/MarkdownEditor", () => ({
  default: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (v: string) => void;
  }) => (
    <textarea
      aria-label="Markdown"
      value={value}
      onChange={(e) => onChange(e.target.value)}
    />
  ),
}));
let data: LibraryData;
let scanSequence = 0;
function emptyInventory(kind: LibraryKind) {
  return {
    ticketId: `empty-${kind}-${++scanSequence}`,
    kind,
    expiresAtMs: Date.now() + 600000,
    sources: [],
    entries: [],
  };
}
const target = (toolId: string, desired = false, available = true) => ({
  toolId,
  path: `/fixture/${toolId}`,
  targetGroupId: toolId,
  available,
  discoveryStatus: "installed",
  status: desired ? "applied" : "unassigned",
  desired,
});
function Refresh() {
  const shell = useShell();
  useEffect(() => shell.setActive("library"), [shell.setActive]);
  return <button onClick={() => void shell.refresh()}>Refresh fixture</button>;
}
function Fixture({ active = true }: { active?: boolean }) {
  const [client] = useState(
    () => new QueryClient({ defaultOptions: { queries: { retry: false } } }),
  );
  return (
    <QueryClientProvider client={client}>
      <ShellProvider openSettings={() => undefined}>
        <InteractionScope active={active}>
          <LibraryPage />
        </InteractionScope>
        <Refresh />
      </ShellProvider>
    </QueryClientProvider>
  );
}
function mount() {
  return render(<Fixture />);
}
async function rowAction(name: string) {
  const resource = name.endsWith("MCP demo")
    ? "MCP demo"
    : name.endsWith("Skill demo")
      ? "Skill demo"
      : "Shared";
  await userEvent.click(
    await screen.findByRole("button", { name: `${resource} 的更多操作` }),
  );
  return screen.findByRole("menuitem", { name });
}

async function toolbarAction(name: string) {
  await userEvent.click(screen.getByRole("button", { name: "更多操作" }));
  return screen.findByRole("menuitem", { name });
}

it("opens scenario selection without saving, downloading or applying resources", async () => {
  const save = vi.spyOn(libraryApi, "saveInstruction");
  const apply = vi.spyOn(libraryApi, "runBatch");
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "场景套餐" }),
  );
  expect(screen.getByRole("button", { name: /前端开发/ })).toBeVisible();
  expect(screen.getByRole("button", { name: /查文档/ })).toBeVisible();
  expect(screen.getByRole("button", { name: /代码审查/ })).toBeVisible();
  expect(save).not.toHaveBeenCalled();
  expect(apply).not.toHaveBeenCalled();
});

beforeEach(() => {
  HTMLElement.prototype.scrollIntoView = vi.fn();
  scanSequence = 0;
  vi.spyOn(libraryApi, "inventory").mockImplementation(async (kind) =>
    emptyInventory(kind),
  );
  vi.spyOn(libraryApi, "matchMcpRecipes").mockResolvedValue([]);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  data = {
    epoch: 1,
    instructions: [
      { id: "a", name: "Shared", content: "Original", revision: 1 },
    ],
    targets: [
      target("codex", true),
      target("claude", true, false),
      target("gemini"),
    ].map((t) => ({ ...t, desiredResourceId: t.desired ? "a" : null })),
    operations: [],
  };
  vi.spyOn(libraryApi, "summary").mockResolvedValue({
    instructions: 1,
    mcp: 1,
    skills: 1,
  });
  vi.spyOn(libraryApi, "list").mockImplementation(async (kind) =>
    kind === "instructions"
      ? data
      : kind === "mcp"
        ? {
            epoch: 1,
            servers: [
              {
                id: "server",
                name: "MCP demo",
                revision: 3,
                transport: "stdio",
                credentialState: "nativeEnvironmentRequired",
                fields: [
                  {
                    path: ["command"],
                    fieldRef: "ref-3",
                    valueType: "string",
                    exists: true,
                  },
                ],
                targets: [],
              },
            ],
            operations: [],
          }
        : {
            epoch: 1,
            skills: [
              {
                id: "skill",
                name: "Skill demo",
                directory: "sample",
                revision: 1,
                sourceState: "ready",
                targets: [
                  {
                    ...target("gemini"),
                    status: "shared",
                    readable: true,
                    readers: ["gemini", "codex", "copilot"],
                  },
                ],
              },
            ],
            operations: [],
          },
  );
  vi.spyOn(libraryApi, "preview").mockResolvedValue({
    operationId: "op-1",
    kind: "instructions",
    resourceId: "a",
    resourceRevision: 1,
    status: "previewed",
    targets: [],
  });
  vi.spyOn(libraryApi, "saveInstruction").mockResolvedValue(
    data.instructions![0],
  );
  vi.spyOn(libraryApi, "saveMcp").mockResolvedValue(undefined);
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
});
it("loads only visited subpages and keeps a dirty editor through refresh", async () => {
  mount();
  await screen.findByText("Shared");
  expect(libraryApi.list).toHaveBeenCalledTimes(1);
  await waitFor(() =>
    expect(libraryApi.inventory).toHaveBeenCalledWith("instructions"),
  );
  expect(libraryApi.inventory).not.toHaveBeenCalledWith("mcp");
  expect(libraryApi.inventory).not.toHaveBeenCalledWith("skills");
  await userEvent.click(screen.getByRole("button", { name: "编辑 Shared" }));
  fireEvent.change(screen.getByLabelText("Markdown"), {
    target: { value: "Unsaved" },
  });
  data = {
    ...data,
    instructions: [
      { ...data.instructions![0], content: "Remote", revision: 2 },
    ],
  };
  fireEvent.click(screen.getByText("Refresh fixture"));
  await waitFor(() => expect(libraryApi.list).toHaveBeenCalledTimes(2));
  expect(screen.getByLabelText("Markdown")).toHaveValue("Unsaved");
  expect(screen.getByRole("button", { name: "保存到资源库" })).toBeDisabled();
  expect(screen.getByText("资源已变化，请关闭并重新打开。")).toBeVisible();
  expect(libraryApi.saveInstruction).not.toHaveBeenCalled();
});
it("shows native instruction summaries even with no shared drafts without treating unknown as empty", async () => {
  data = {
    ...data,
    instructions: [],
    targets: [
      {
        ...target("codex"),
        userContentLines: 12,
        managedContentLines: 0,
        nativeState: "unmanaged",
      },
      {
        ...target("claude", false, false),
        userContentLines: null,
        managedContentLines: null,
        nativeState: "unreadable",
        status: "unreadable",
      },
    ],
  } as LibraryData;
  mount();
  const overview = await screen.findByRole("region", { name: "工具指令总览" });
  for (const summary of within(overview).getAllByText("技术详情"))
    await userEvent.click(summary);
  expect(within(overview).getByText("/fixture/codex")).toBeVisible();
  expect(
    within(overview).getByText("自有内容 12 行 · 受管内容 0 行"),
  ).toBeVisible();
  expect(
    within(overview).getByText("自有内容 未知 · 受管内容 未知"),
  ).toBeVisible();
  expect(
    within(overview).getByText("期望：未分配 · 实际可读：未知"),
  ).toBeVisible();
  expect(libraryApi.preview).not.toHaveBeenCalled();
});
it("does not infer readable instructions from missing older-backend health fields", async () => {
  data = { ...data, instructions: [], targets: [target("codex")] };
  mount();
  const overview = await screen.findByRole("region", { name: "工具指令总览" });
  expect(
    within(overview).getByText("期望：未分配 · 实际可读：未知"),
  ).toBeVisible();
});
it("distinguishes desired and readable instruction sets and keeps missing definitions visible", async () => {
  data = {
    ...data,
    instructions: [
      ...data.instructions!,
      { id: "b", name: "Older", content: "Old", revision: 1 },
    ],
    targets: [
      {
        ...target("codex", true),
        desiredResourceId: "a",
        readableResourceId: "b",
        lastAppliedRevision: 1,
        status: "pending",
      },
      {
        ...target("gemini"),
        desiredResourceId: "deleted",
        readableResourceId: "b",
        status: "shared",
        readers: ["codex", "gemini"],
      },
    ],
  } as LibraryData;
  mount();
  const overview = await screen.findByRole("region", { name: "工具指令总览" });
  expect(
    within(overview).getByText("期望：Shared · 实际可读：Older"),
  ).toBeVisible();
  expect(
    within(overview).getByText("期望：定义缺失（deleted） · 实际可读：Older"),
  ).toBeVisible();
  await userEvent.click(
    within(overview).getByRole("button", { name: "管理 Codex 的指令分配" }),
  );
  expect(screen.getByRole("dialog")).toBeVisible();
  expect(libraryApi.preview).not.toHaveBeenCalled();
});
it("shows skill provenance separately from its content hash and freezes detail identity", async () => {
  const base = vi.mocked(libraryApi.list).getMockImplementation()!;
  vi.mocked(libraryApi.list).mockImplementation(async (kind) =>
    kind === "skills"
      ? ({
          epoch: 7,
          skills: [
            {
              id: "skill",
              name: "Skill demo",
              directory: "sample",
              revision: 2,
              artifactHash: "artifact-hash",
              sourceState: "ready",
              sourceKind: "repository",
              sourceUrl: "https://github.com/fixture/repo/blob/abcd/SKILL.md",
              repository: "fixture/repo",
              trackingRef: "feature/skills",
              commitSha: "a".repeat(40),
              installedAtMs: 0,
              updatedAtMs: 1720000000000,
              targets: [],
            },
          ],
          operations: [],
        } as LibraryData)
      : base(kind),
  );
  vi.spyOn(libraryApi, "skillDetail").mockResolvedValue({
    resourceId: "skill",
    artifactHash: "artifact-hash",
    path: "SKILL.md",
    content: "Verified text",
    files: [],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await userEvent.click(
    await screen.findByRole("button", { name: "详情 Skill demo" }),
  );
  const dialog = screen.getByRole("dialog");
  expect(await within(dialog).findByText("Verified text")).toBeVisible();
  expect(within(dialog).getByText("feature/skills")).toBeVisible();
  expect(within(dialog).getByText("a".repeat(40))).toBeVisible();
  expect(within(dialog).getByText("未记录")).toBeVisible();
  expect(libraryApi.skillDetail).toHaveBeenCalledWith(
    "skill",
    "SKILL.md",
    2,
    7,
    "artifact-hash",
  );
});
it("offers a focused command repair after a measured missing-program check without saving", async () => {
  vi.mocked(listen).mockResolvedValue(vi.fn());
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "missing-program",
  });
  vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
    operationId: "missing-program",
    state: "blocked",
    reason: "programMissing",
    tools: [],
  });
  const save = vi.spyOn(libraryApi, "saveMcp");
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await screen.findByText("MCP demo");
  await userEvent.click(await rowAction("检查连接 MCP demo"));
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "修改命令" }),
  );
  expect(
    await screen.findByRole("combobox", { name: "command的操作" }),
  ).toHaveFocus();
  expect(save).not.toHaveBeenCalled();
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "MCP demo 的更多操作" }),
    ).toHaveFocus(),
  );
});

it("starts MCP inspection only explicitly and subscribes before execution", async () => {
  let subscribed!: (unlisten: () => void) => void;
  vi.mocked(listen).mockImplementation(
    () =>
      new Promise((resolve) => {
        subscribed = resolve;
      }),
  );
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "check-one",
  });
  vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
    operationId: "check-one",
    state: "ok",
    tools: [{ name: "fixture-tool" }],
  });
  vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await screen.findByText("MCP demo");
  expect(libraryApi.prepareMcpCheck).not.toHaveBeenCalled();
  await userEvent.click(await rowAction("检查连接 MCP demo"));
  expect(libraryApi.prepareMcpCheck).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(listen).toHaveBeenCalled());
  expect(libraryApi.runMcpCheck).not.toHaveBeenCalled();
  expect(libraryApi.prepareMcpCheck).toHaveBeenCalledWith("server", 3, 1);
  const unlisten = vi.fn();
  await act(async () => subscribed(unlisten));
  expect(await screen.findByText("fixture-tool")).toBeVisible();
  expect(libraryApi.runMcpCheck).toHaveBeenCalledWith("check-one");
  expect(unlisten).toHaveBeenCalledTimes(1);
  await userEvent.click(screen.getByRole("button", { name: "关闭" }));
  expect(screen.queryByText(/连接未检查/)).not.toBeInTheDocument();
  await userEvent.click(await rowAction("检查连接 MCP demo"));
  expect(screen.queryByText("fixture-tool")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "开始检查" })).toBeEnabled();
  expect(libraryApi.prepareMcpCheck).toHaveBeenCalledTimes(1);
  expect(libraryApi.runMcpCheck).toHaveBeenCalledTimes(1);
});
it("isolates MCP progress and cancels with immediate listener cleanup when the portal suspends", async () => {
  let progress!: (event: { payload: McpCheckProgress }) => void;
  const unlisten = vi.fn();
  vi.mocked(listen).mockImplementation(async (_event, callback) => {
    progress = callback as typeof progress;
    return unlisten;
  });
  let finish!: (value: McpCheckResult) => void;
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "owned-check",
  });
  vi.spyOn(libraryApi, "runMcpCheck").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  const rendered = render(<Fixture />);
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await userEvent.click(await rowAction("检查连接 MCP demo"));
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(libraryApi.runMcpCheck).toHaveBeenCalled());
  act(() =>
    progress({ payload: { operationId: "other-check", stage: "list" } }),
  );
  expect(screen.queryByText("读取工具列表…")).not.toBeInTheDocument();
  act(() =>
    progress({ payload: { operationId: "owned-check", stage: "list" } }),
  );
  expect(screen.getByText("读取工具列表…")).toBeVisible();
  rendered.rerender(<Fixture active={false} />);
  await waitFor(() =>
    expect(libraryApi.cancelMcpCheck).toHaveBeenCalledWith("owned-check"),
  );
  expect(unlisten).toHaveBeenCalledTimes(1);
  await act(async () =>
    finish({
      operationId: "owned-check",
      state: "ok",
      tools: [{ name: "late-tool" }],
    }),
  );
  rendered.rerender(<Fixture />);
  expect(await screen.findByText("检查已取消")).toBeVisible();
  expect(screen.queryByText("late-tool")).not.toBeInTheDocument();
  expect(unlisten).toHaveBeenCalledTimes(1);
});
it("cancels a late prepared MCP job after the dialog has closed without subscribing or executing", async () => {
  let prepared!: (value: { operationId: string }) => void;
  vi.spyOn(libraryApi, "prepareMcpCheck").mockImplementation(
    () =>
      new Promise((resolve) => {
        prepared = resolve;
      }),
  );
  vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
    operationId: "late-prepared",
    state: "ok",
    tools: [],
  });
  vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  vi.mocked(listen).mockClear();
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await userEvent.click(await rowAction("检查连接 MCP demo"));
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await userEvent.click(screen.getByRole("button", { name: "取消检查" }));
  await userEvent.click(screen.getByRole("button", { name: "关闭" }));
  await act(async () => prepared({ operationId: "late-prepared" }));
  await waitFor(() =>
    expect(libraryApi.cancelMcpCheck).toHaveBeenCalledWith("late-prepared"),
  );
  expect(listen).not.toHaveBeenCalled();
  expect(libraryApi.runMcpCheck).not.toHaveBeenCalled();
});
it("unsubscribes a late MCP listener exactly once after its portal has suspended", async () => {
  let subscribed!: (unlisten: () => void) => void;
  const unlisten = vi.fn();
  vi.mocked(listen).mockImplementation(
    () =>
      new Promise((resolve) => {
        subscribed = resolve;
      }),
  );
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "late-listener",
  });
  vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
    operationId: "late-listener",
    state: "ok",
    tools: [],
  });
  vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  const rendered = render(<Fixture />);
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await userEvent.click(await rowAction("检查连接 MCP demo"));
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(listen).toHaveBeenCalled());
  rendered.rerender(<Fixture active={false} />);
  await act(async () => subscribed(unlisten));
  expect(libraryApi.cancelMcpCheck).toHaveBeenCalledWith("late-listener");
  expect(unlisten).toHaveBeenCalledTimes(1);
  expect(libraryApi.runMcpCheck).not.toHaveBeenCalled();
});
it("previews the complete desired set and lets an unavailable prior target be withdrawn", async () => {
  mount();
  await screen.findByText("Shared");
  await userEvent.click(
    screen.getByRole("button", { name: "将 Shared 应用到工具" }),
  );
  const dialog = screen.getByRole("dialog");
  expect(
    within(dialog).getByRole("checkbox", { name: "claude" }),
  ).toBeEnabled();
  await userEvent.click(
    within(dialog).getByRole("checkbox", { name: "gemini" }),
  );
  await userEvent.click(
    within(dialog).getByRole("button", { name: "预览变更" }),
  );
  expect(libraryApi.preview).toHaveBeenCalledWith(
    "instructions",
    "a",
    ["codex", "claude", "gemini"],
    1,
    1,
  );
});
it("saves a name-only MCP edit without replacing saved private fields", async () => {
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await screen.findByText("MCP demo");
  await userEvent.click(screen.getByRole("button", { name: "编辑 MCP demo" }));
  fireEvent.change(screen.getByLabelText("名称"), {
    target: { value: "Renamed" },
  });
  await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
  expect(libraryApi.saveMcp).toHaveBeenCalledWith(
    "server",
    "Renamed",
    3,
    [],
    1,
  );
});
it("discloses shared Skills readability without claiming a separate assignment", async () => {
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await screen.findByText("Skill demo");
  await userEvent.click(
    screen.getByRole("button", { name: "将 Skill demo 应用到工具" }),
  );
  expect(screen.getByRole("checkbox", { name: "gemini" })).not.toBeChecked();
  expect(screen.queryByRole("checkbox", { name: "copilot" })).toBeNull();
  expect(
    screen.getByText("已知潜在读取工具：Gemini CLI, Codex, GitHub Copilot CLI"),
  ).toBeVisible();
  expect(screen.getByText("未单独分配，仍可从共享目录读取")).toBeVisible();
  expect(
    screen.getByText(
      "共享目录也可能被 Copilot 等兼容工具读取；潜在读取者不代表已启用或实际加载，请在工具中核对。",
    ),
  ).toBeVisible();
  expect(libraryApi.preview).not.toHaveBeenCalled();
});
it("keeps dirty input when the retained page releases and reopens its portal", async () => {
  const rendered = mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("button", { name: "编辑 Shared" }));
  fireEvent.change(screen.getByLabelText("Markdown"), {
    target: { value: "Retained draft" },
  });
  rendered.rerender(<Fixture active={false} />);
  expect(screen.queryByRole("dialog")).toBeNull();
  rendered.rerender(<Fixture />);
  expect(screen.getByLabelText("Markdown")).toHaveValue("Retained draft");
});
it("accepts actual backend pending targets and retains an in-flight preview across suspension", async () => {
  let finish!: (value: Awaited<ReturnType<typeof libraryApi.preview>>) => void;
  vi.mocked(libraryApi.preview).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  vi.spyOn(libraryApi, "apply").mockResolvedValue({
    operationId: "op-1",
    kind: "instructions",
    resourceId: "a",
    resourceRevision: 1,
    status: "applied",
    targets: [],
  });
  const rendered = mount();
  await screen.findByText("Shared");
  await userEvent.click(
    screen.getByRole("button", { name: "将 Shared 应用到工具" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "预览变更" }));
  rendered.rerender(<Fixture active={false} />);
  rendered.rerender(<Fixture />);
  expect(screen.getByRole("button", { name: "预览变更" })).toBeDisabled();
  expect(libraryApi.preview).toHaveBeenCalledTimes(1);
  await act(async () =>
    finish({
      operationId: "op-1",
      kind: "instructions",
      resourceId: "a",
      resourceRevision: 1,
      status: "previewed",
      targets: [
        {
          tools: ["codex"],
          readers: ["codex"],
          path: "/fixture",
          targetGroupId: "group",
          status: "pending",
          changedFields: [],
        },
      ],
    }),
  );
  expect(await screen.findByRole("button", { name: "确认应用" })).toBeEnabled();
  await userEvent.click(screen.getByRole("button", { name: "确认应用" }));
  expect(libraryApi.apply).toHaveBeenCalledWith("op-1");
});
it("keeps the skill file picker available after an unreadable file", async () => {
  vi.spyOn(libraryApi, "skillDetail").mockImplementation(async (_id, path) => {
    if (path === "binary.dat") throw "library.skillFileNotText";
    return {
      resourceId: "skill",
      artifactHash: "hash",
      path: "SKILL.md",
      content: "Text",
      files: [
        {
          path: "SKILL.md",
          isDirectory: false,
          sizeBytes: 4,
          executable: false,
        },
        {
          path: "binary.dat",
          isDirectory: false,
          sizeBytes: 4,
          executable: false,
        },
      ],
    };
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await userEvent.click(
    await screen.findByRole("button", { name: "详情 Skill demo" }),
  );
  await screen.findByText("Text");
  await userEvent.click(screen.getByRole("combobox", { name: "技能文件" }));
  await userEvent.click(
    await screen.findByRole("option", { name: /binary.dat/ }),
  );
  await screen.findByRole("alert");
  expect(screen.getByRole("combobox", { name: "技能文件" })).toBeEnabled();
});
it("locks the legacy import source while its explicit read is pending", async () => {
  let finish!: (
    value: Awaited<ReturnType<typeof promptsApi.getPrompts>>,
  ) => void;
  vi.spyOn(promptsApi, "getPrompts").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  mount();
  await screen.findByText("Shared");
  await userEvent.click(await toolbarAction("导入 Markdown／旧提示词"));
  await userEvent.click(screen.getByRole("combobox", { name: "导入来源" }));
  await userEvent.click(
    await screen.findByRole("option", { name: "claude 旧提示词" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "读取旧提示词" }));
  expect(screen.getByRole("combobox", { name: "导入来源" })).toBeDisabled();
  await act(async () =>
    finish({
      legacy: {
        id: "legacy",
        name: "Legacy",
        content: "Imported",
        enabled: true,
      },
    }),
  );
  expect(await screen.findByText("Legacy")).toBeVisible();
});
it("does not replace a new edit with a late Markdown file read", async () => {
  let finish!: (value: string) => void;
  const file = new File(["file"], "sample.md", { type: "text/markdown" });
  Object.assign(file, {
    text: () =>
      new Promise<string>((resolve) => {
        finish = resolve;
      }),
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(await toolbarAction("导入 Markdown／旧提示词"));
  fireEvent.change(screen.getByLabelText("Markdown 文件／粘贴内容"), {
    target: { files: [file] },
  });
  fireEvent.change(screen.getByLabelText("Markdown"), {
    target: { value: "New edit" },
  });
  await act(async () => finish("Old file"));
  expect(screen.getByLabelText("Markdown")).toHaveValue("New edit");
});
it("keeps the latest file selection when file reads finish out of order", async () => {
  let first!: (value: string) => void;
  let second!: (value: string) => void;
  const a = Object.assign(new File(["a"], "a.md"), {
    text: () =>
      new Promise<string>((resolve) => {
        first = resolve;
      }),
  });
  const b = Object.assign(new File(["b"], "b.md"), {
    text: () =>
      new Promise<string>((resolve) => {
        second = resolve;
      }),
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(await toolbarAction("导入 Markdown／旧提示词"));
  const picker = screen.getByLabelText("Markdown 文件／粘贴内容");
  fireEvent.change(picker, { target: { files: [a] } });
  fireEvent.change(picker, { target: { files: [b] } });
  await act(async () => second("Latest file"));
  await act(async () => first("Old file"));
  expect(screen.getByLabelText("Markdown")).toHaveValue("Latest file");
});
it("allows a preview with only desired assignment removals to be confirmed", async () => {
  vi.spyOn(libraryApi, "apply").mockResolvedValue({
    operationId: "op-1",
    kind: "instructions",
    resourceId: "a",
    resourceRevision: 1,
    status: "applied",
    targets: [],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(
    screen.getByRole("button", { name: "将 Shared 应用到工具" }),
  );
  await userEvent.click(screen.getByRole("checkbox", { name: "codex" }));
  await userEvent.click(screen.getByRole("checkbox", { name: "claude" }));
  await userEvent.click(screen.getByRole("button", { name: "预览变更" }));
  expect(await screen.findByRole("button", { name: "确认应用" })).toBeEnabled();
});
it("freezes the opened epoch when deleting after a background restore", async () => {
  vi.spyOn(libraryApi, "deleteInstruction").mockResolvedValue(true);
  mount();
  await screen.findByText("Shared");
  await userEvent.click(await rowAction("移除 Shared"));
  data = { ...data, epoch: 2 };
  fireEvent.click(screen.getByText("Refresh fixture"));
  await waitFor(() => expect(libraryApi.list).toHaveBeenCalledTimes(2));
  await userEvent.click(screen.getByRole("button", { name: "确认移除资源" }));
  expect(libraryApi.deleteInstruction).toHaveBeenCalledWith("a", 1, 1);
});
it("does not present an unreadable backend as an empty writable library", async () => {
  vi.mocked(libraryApi.list).mockRejectedValue(new Error("old backend"));
  mount();
  await screen.findByRole("alert");
  expect(screen.getByRole("button", { name: "新建" })).toBeDisabled();
  expect(
    screen.queryByText("尚未收录资源。添加或导入后，再明确选择应用目标。"),
  ).toBeNull();
});
it("keeps existing legacy rows read-only when their backend supplies no epoch", async () => {
  data = { ...data, epoch: undefined as unknown as number };
  mount();
  await screen.findByText("Shared");
  expect(screen.getByRole("button", { name: "编辑 Shared" })).toBeDisabled();
  expect(
    screen.getByRole("button", { name: "将 Shared 应用到工具" }),
  ).toBeDisabled();
  expect(await rowAction("移除 Shared")).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});
it("requires a resource-pack preview and confirms the frozen file fingerprint and revision", async () => {
  vi.spyOn(libraryApi, "pickPack").mockResolvedValue("/sandbox/resources.zip");
  vi.spyOn(libraryApi, "previewPack").mockResolvedValue({
    fingerprint: "pack-hash",
    revision: 12,
    epoch: 8,
    items: [
      {
        kind: "mcp",
        id: "pack-id",
        name: "Template",
        conflict: true,
        omittedFields: 3,
      },
    ],
  });
  vi.spyOn(libraryApi, "restorePack").mockResolvedValue({
    imported: 0,
    items: [{ kind: "mcp", name: "Template", status: "skipped" }],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("button", { name: "资源库更多操作" }));
  expect(screen.queryByRole("button", { name: "确认恢复草稿" })).toBeNull();
  await userEvent.click(screen.getByRole("menuitem", { name: "导入资源包" }));
  await screen.findByText("Template");
  expect(libraryApi.restorePack).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "确认恢复草稿" }));
  expect(libraryApi.restorePack).toHaveBeenCalledWith(
    "/sandbox/resources.zip",
    expect.objectContaining({
      fingerprint: "pack-hash",
      revision: 12,
      epoch: 8,
    }),
    "skip",
  );
  expect(await screen.findByText("已跳过")).toBeVisible();
});
it("keeps a resource-pack preview while the retained page releases its dialog", async () => {
  vi.spyOn(libraryApi, "pickPack").mockResolvedValue("/sandbox/resources.zip");
  vi.spyOn(libraryApi, "previewPack").mockResolvedValue({
    fingerprint: "pack-hash",
    revision: 12,
    epoch: 8,
    items: [],
  });
  const rendered = mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("button", { name: "资源库更多操作" }));
  await userEvent.click(screen.getByRole("menuitem", { name: "导入资源包" }));
  await screen.findByRole("button", { name: "确认恢复草稿" });
  rendered.rerender(<Fixture active={false} />);
  expect(screen.queryByRole("dialog")).toBeNull();
  rendered.rerender(<Fixture />);
  expect(screen.getByRole("button", { name: "确认恢复草稿" })).toBeEnabled();
  expect(libraryApi.previewPack).toHaveBeenCalledTimes(1);
});
it("retains an in-flight pack preview through portal suspension and rejects a second submission", async () => {
  let finish!: (v: Awaited<ReturnType<typeof libraryApi.previewPack>>) => void;
  vi.spyOn(libraryApi, "pickPack").mockResolvedValue("/sandbox/resources.zip");
  vi.spyOn(libraryApi, "previewPack").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const rendered = mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("button", { name: "资源库更多操作" }));
  await userEvent.click(screen.getByRole("menuitem", { name: "导入资源包" }));
  rendered.rerender(<Fixture active={false} />);
  rendered.rerender(<Fixture />);
  expect(
    screen.getByRole("button", { name: "选择资源包并预览" }),
  ).toBeDisabled();
  expect(screen.getByRole("button", { name: "关闭" })).toBeDisabled();
  await act(async () =>
    finish({ fingerprint: "hash", revision: 3, epoch: 4, items: [] }),
  );
  expect(
    await screen.findByRole("button", { name: "确认恢复草稿" }),
  ).toBeEnabled();
  expect(libraryApi.previewPack).toHaveBeenCalledTimes(1);
});
it("copies MCP using backend references and freezes the opened source revision and epoch", async () => {
  vi.spyOn(libraryApi, "duplicateMcp").mockResolvedValue({
    id: "copy",
    name: "Copy",
    revision: 1,
    transport: "stdio",
    credentialState: "needsMigration",
    fields: [],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await screen.findByText("MCP demo");
  await userEvent.click(await rowAction("复制模板 MCP demo"));
  fireEvent.change(screen.getByLabelText("新服务器 ID"), {
    target: { value: "new-server-id" },
  });
  fireEvent.change(screen.getByLabelText("名称"), {
    target: { value: "Copy" },
  });
  await userEvent.click(screen.getByRole("button", { name: "保存副本" }));
  expect(libraryApi.duplicateMcp).toHaveBeenCalledWith(
    "server",
    3,
    1,
    "new-server-id",
    "Copy",
  );
});
it("requires a second full preview and explicit confirmation for native MCP adoption", async () => {
  vi.mocked(libraryApi.list).mockResolvedValue({
    epoch: 1,
    servers: [
      {
        id: "server",
        name: "MCP demo",
        revision: 3,
        transport: "stdio",
        credentialState: "nativeEnvironmentRequired",
        fields: [],
        targets: [target("codex", true), target("claude", true)],
      },
    ],
    operations: [],
  });
  const nativeTarget = {
    tools: ["codex"],
    readers: ["codex"],
    path: "/sandbox/config.toml",
    targetGroupId: "codex",
    status: "conflict",
    reason: "library.mcpNameConflict",
    changedFields: [],
  };
  vi.mocked(libraryApi.preview).mockResolvedValue({
    operationId: "before-adopt",
    kind: "mcp",
    resourceId: "server",
    resourceRevision: 3,
    status: "previewed",
    targets: [nativeTarget],
  });
  vi.spyOn(libraryApi, "previewMcpAdoption").mockResolvedValue({
    operationId: "adopt-preview",
    kind: "mcp",
    resourceId: "server",
    resourceRevision: 3,
    status: "previewed",
    targets: [
      { ...nativeTarget, status: "pending", reason: "library.adoptionPreview" },
    ],
  });
  vi.spyOn(libraryApi, "apply").mockResolvedValue({
    operationId: "adopt-preview",
    kind: "mcp",
    resourceId: "server",
    resourceRevision: 3,
    status: "applied",
    targets: [],
  });
  mount();
  await screen.findByText("MCP demo");
  await userEvent.click(screen.getByRole("tab", { name: /MCP/ }));
  await userEvent.click(
    screen.getByRole("button", { name: "将 MCP demo 应用到工具" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "预览变更" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "预览接管一致字段" }),
  );
  expect(libraryApi.previewMcpAdoption).toHaveBeenCalledWith(
    "server",
    ["codex", "claude"],
    ["codex"],
    3,
    1,
  );
  expect(libraryApi.apply).not.toHaveBeenCalled();
  await userEvent.click(
    await screen.findByRole("button", { name: "确认应用" }),
  );
  expect(libraryApi.apply).toHaveBeenCalledWith("adopt-preview");
});
it("checks one skill explicitly and confirms its frozen candidate without applying native targets", async () => {
  vi.spyOn(libraryApi, "checkSkillUpdate").mockResolvedValue({
    resourceId: "skill",
    revision: 1,
    epoch: 1,
    state: "updateAvailable",
    currentArtifactHash: "old",
    targetConflicts: [],
    candidate: {
      ticketId: "ticket-a",
      artifactHash: "new",
      commitSha: "a".repeat(40),
      name: "New Skill",
      expiresAtMs: Date.now() + 600000,
    },
  });
  vi.spyOn(libraryApi, "compareSkillUpdate").mockResolvedValue({
    path: "SKILL.md",
    before: "Old body",
    after: "New body",
    files: [
      {
        path: "SKILL.md",
        status: "modified",
        beforeExecutable: false,
        afterExecutable: false,
      },
    ],
  });
  vi.spyOn(libraryApi, "confirmSkillUpdate").mockResolvedValue({
    state: "updated",
    resourceId: "skill",
    revision: 2,
    artifactHash: "new",
    nativeApplyRequired: true,
  });
  vi.spyOn(libraryApi, "apply");
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await screen.findByText("Skill demo");
  expect(libraryApi.checkSkillUpdate).not.toHaveBeenCalled();
  await userEvent.click(await rowAction("检查更新 Skill demo"));
  await userEvent.click(screen.getByRole("button", { name: "检查来源更新" }));
  await screen.findByText("New body");
  expect(libraryApi.checkSkillUpdate).toHaveBeenCalledWith("skill", 1, 1);
  await userEvent.click(screen.getByRole("button", { name: "更新源草稿" }));
  expect(libraryApi.confirmSkillUpdate).toHaveBeenCalledWith(
    "ticket-a",
    1,
    1,
    "update",
    undefined,
    undefined,
  );
  expect(libraryApi.apply).not.toHaveBeenCalled();
  expect(
    await screen.findByText(
      "源草稿已更新。工具仍保留旧内容，请预览并应用分发。",
    ),
  ).toBeVisible();
});
it("labels local skills as untracked instead of current and keeps updates disabled", async () => {
  vi.spyOn(libraryApi, "checkSkillUpdate").mockResolvedValue({
    resourceId: "skill",
    revision: 1,
    epoch: 1,
    state: "notTracked",
    currentArtifactHash: "old",
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await userEvent.click(await rowAction("检查更新 Skill demo"));
  await userEvent.click(screen.getByRole("button", { name: "检查来源更新" }));
  expect(
    await screen.findByText(
      "此技能没有可跟踪的仓库来源；本机或 ZIP 导入不会被标记为已是最新。",
    ),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: "更新源草稿" })).toBeNull();
});
it("retries a failed comparison using the same frozen ticket before enabling a skill update", async () => {
  vi.spyOn(libraryApi, "checkSkillUpdate").mockResolvedValue({
    resourceId: "skill",
    revision: 1,
    epoch: 1,
    state: "updateAvailable",
    currentArtifactHash: "old",
    sourceVerified: true,
    candidate: {
      ticketId: "keep-ticket",
      artifactHash: "new",
      commitSha: "a".repeat(40),
      name: "New",
      expiresAtMs: Date.now() + 600000,
    },
  });
  vi.spyOn(libraryApi, "compareSkillUpdate")
    .mockRejectedValueOnce("library.skillReadFailed")
    .mockResolvedValue({
      path: "SKILL.md",
      before: "Old",
      after: "Retried candidate",
      files: [
        {
          path: "SKILL.md",
          status: "modified",
          beforeExecutable: false,
          afterExecutable: false,
        },
      ],
    });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await userEvent.click(await rowAction("检查更新 Skill demo"));
  await userEvent.click(screen.getByRole("button", { name: "检查来源更新" }));
  await screen.findByRole("alert");
  expect(screen.getByRole("button", { name: "更新源草稿" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "另存候选副本" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "重试比较" }));
  await screen.findByText("Retried candidate");
  expect(libraryApi.checkSkillUpdate).toHaveBeenCalledTimes(1);
  expect(libraryApi.compareSkillUpdate).toHaveBeenLastCalledWith(
    "keep-ticket",
    "SKILL.md",
  );
  expect(screen.getByRole("button", { name: "更新源草稿" })).toBeEnabled();
});

it("stages agent logo changes and requires a composed preview before native writes", async () => {
  const preview = vi.spyOn(libraryApi, "previewBatch").mockResolvedValue({
    operationId: "batch-1",
    kind: "instructions",
    status: "previewed",
    resources: [{ resourceId: "a" }],
    groups: [],
  });
  const apply = vi.spyOn(libraryApi, "runBatch").mockResolvedValue({
    operationId: "batch-1",
    kind: "instructions",
    status: "applied",
    resources: [{ resourceId: "a" }],
    groups: [],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(
    screen.getByRole("button", { name: "Shared · Gemini CLI · 未分配" }),
  );
  expect(preview).not.toHaveBeenCalled();
  expect(apply).not.toHaveBeenCalled();
  expect(screen.getByText("待应用 1 项")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "预览变更" }));
  expect(preview).toHaveBeenCalledWith(
    "instructions",
    [
      {
        resourceId: "a",
        expectedRevision: 1,
        addTools: ["gemini"],
        withdrawTools: [],
      },
    ],
    [],
    "apply",
    1,
  );
  expect(apply).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "确认应用" }));
  await waitFor(() => expect(apply).toHaveBeenCalledWith("batch-1"));
});
it("scans native MCP without importing until selected entries are confirmed", async () => {
  const scan = vi.spyOn(libraryApi, "inventory").mockResolvedValue({
    ticketId: "scan-1",
    kind: "mcp",
    expiresAtMs: Date.now() + 600000,
    sources: [
      {
        sourceId: "source-1",
        toolId: "codex",
        label: "Codex",
        state: "ready",
        count: 2,
      },
    ],
    entries: [
      {
        entryId: "entry-1",
        kind: "mcp",
        name: "Native one",
        sources: [{ sourceId: "source-1", toolId: "codex" }],
        matchedResourceIds: [],
        conflict: false,
      },
      {
        entryId: "entry-2",
        kind: "mcp",
        name: "Native two",
        sources: [{ sourceId: "source-1", toolId: "codex" }],
        matchedResourceIds: [],
        conflict: false,
      },
    ],
  });
  const imported = vi.spyOn(libraryApi, "importInventory").mockResolvedValue({
    imported: 1,
    items: [
      {
        entryId: "entry-2",
        resourceId: "draft-2",
        name: "Native two",
        status: "imported",
      },
    ],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /^MCP/ }));
  await screen.findByText("MCP demo");
  await userEvent.click(await toolbarAction("从本机导入"));
  await within(screen.getByRole("dialog")).findByText("Native two");
  expect(scan).toHaveBeenCalledWith("mcp");
  expect(imported).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getByRole("checkbox", { name: "选择 Native two" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "导入选中项（1）" }),
  );
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("scan-1", ["entry-2"], 1),
  );
});

it("groups skills by provenance without exposing paths in compact rows", async () => {
  const original = vi.mocked(libraryApi.list).getMockImplementation()!;
  vi.mocked(libraryApi.list).mockImplementation(async (kind) =>
    kind === "skills"
      ? {
          epoch: 1,
          skills: [
            {
              id: "s1",
              name: "First skill",
              description: "One",
              directory: "one",
              revision: 1,
              sourceState: "ready",
              sourceKind: "repository",
              repository: "anthropics/skills",
              targets: [],
            },
            {
              id: "s2",
              name: "Second skill",
              directory: "two",
              revision: 1,
              sourceState: "ready",
              sourceKind: "repository",
              repository: "fixture/example",
              targets: [],
            },
          ],
          operations: [],
        }
      : original(kind),
  );
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /Skills/ }));
  await screen.findByText("First skill");
  await userEvent.click(screen.getByRole("combobox", { name: "排序" }));
  await userEvent.click(screen.getByRole("option", { name: "按来源" }));
  expect(
    screen.getByRole("heading", { name: "anthropics/skills" }),
  ).toBeVisible();
  expect(
    screen.getByRole("heading", { name: "fixture/example" }),
  ).toBeVisible();
});
it("keeps unrelated staged logo changes after applying a separate selection", async () => {
  data = {
    ...data,
    instructions: [
      ...data.instructions!,
      { id: "b", name: "Second", content: "Second content", revision: 1 },
    ],
  };
  vi.spyOn(libraryApi, "previewBatch").mockResolvedValue({
    operationId: "selected-b",
    kind: "instructions",
    status: "previewed",
    resources: [{ resourceId: "b" }],
    groups: [],
  });
  vi.spyOn(libraryApi, "runBatch").mockResolvedValue({
    operationId: "selected-b",
    kind: "instructions",
    status: "applied",
    resources: [{ resourceId: "b" }],
    groups: [],
  });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(
    screen.getByRole("button", { name: "Shared · Gemini CLI · 未分配" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "选择" }));
  await userEvent.click(screen.getByRole("checkbox", { name: "选择 Second" }));
  await userEvent.click(screen.getByRole("button", { name: "Codex" }));
  await userEvent.click(screen.getByRole("button", { name: "应用到所选工具" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "确认应用" }),
  );
  await waitFor(() =>
    expect(libraryApi.runBatch).toHaveBeenCalledWith("selected-b"),
  );
  await userEvent.click(screen.getByRole("button", { name: "关闭" }));
  expect(screen.getByText("待应用 1 项")).toBeVisible();
});
it("keeps read-only discovery results in the native section and rescans before row import", async () => {
  vi.spyOn(libraryApi, "inventory").mockResolvedValue({
    ticketId: "fresh-native",
    kind: "mcp",
    expiresAtMs: Date.now() + 600000,
    sources: [
      {
        sourceId: "codex-native",
        toolId: "codex",
        label: "nativeMcp",
        state: "ready",
        count: 1,
      },
    ],
    entries: [
      {
        entryId: "entry-native",
        kind: "mcp",
        name: "Native source",
        sources: [{ sourceId: "codex-native", toolId: "codex" }],
        matchedResourceIds: [],
        conflict: false,
      },
    ],
  });
  const importing = vi
    .spyOn(libraryApi, "importInventory")
    .mockResolvedValue({ imported: 1, items: [] });
  mount();
  await screen.findByText("Shared");
  await userEvent.click(screen.getByRole("tab", { name: /^MCP/ }));
  await screen.findByText("MCP demo");
  await userEvent.click(await toolbarAction("从本机导入"));
  await within(screen.getByRole("dialog")).findByText("Native source");
  await userEvent.click(screen.getByRole("button", { name: "关闭" }));
  const native = screen.getByRole("region", { name: "本机配置中" });
  const scansBeforeRowImport = vi
    .mocked(libraryApi.inventory)
    .mock.calls.filter(([kind]) => kind === "mcp").length;
  expect(within(native).getByText("Native source")).toBeVisible();
  expect(within(native).getByText(/原生 MCP 配置/)).toBeVisible();
  expect(importing).not.toHaveBeenCalled();
  await userEvent.click(
    within(native).getByRole("button", { name: "导入 Native source" }),
  );
  await waitFor(() =>
    expect(
      vi
        .mocked(libraryApi.inventory)
        .mock.calls.filter(([kind]) => kind === "mcp"),
    ).toHaveLength(scansBeforeRowImport + 1),
  );
  expect(
    await screen.findByRole("button", { name: "导入选中项（1）" }),
  ).toBeEnabled();
  expect(importing).not.toHaveBeenCalled();
});
it.each(["mcp", "skills"] as const)(
  "keeps older %s backends read-only without a native snapshot",
  async (kind) => {
    const original = vi.mocked(libraryApi.list).getMockImplementation()!;
    vi.mocked(libraryApi.list).mockImplementation(async (selectedKind) => ({
      ...(await original(selectedKind)),
      epoch: undefined as unknown as number,
    }));
    mount();
    await screen.findByText("Shared");
    await userEvent.click(
      screen.getByRole("tab", { name: kind === "mcp" ? /^MCP/ : /Skills/ }),
    );
    await screen.findByText(kind === "mcp" ? "MCP demo" : "Skill demo");
    expect(
      within(
        screen.getByRole("tabpanel", {
          name: kind === "mcp" ? "MCP" : "Skills",
        }),
      ).getByText(zh.library.legacyReadOnly),
    ).toBeVisible();
    expect(
      screen.getByRole("button", {
        name: kind === "mcp" ? "添加 MCP" : zh.library.addSkill,
      }),
    ).toBeDisabled();
  },
);
