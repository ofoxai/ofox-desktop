import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { beforeEach, expect, it, vi } from "vitest";
import zh from "@/i18n/locales/zh.json";
import {
  libraryApi,
  type LibraryInventory,
  type LibraryOperation,
  type McpDefinition,
  type ResourceTarget,
} from "@/lib/api/library";
import { LibraryMcpTable } from "@/components/library/LibraryMcpTable";

const target = (
  toolId: string,
  extra: Partial<ResourceTarget> = {},
): ResourceTarget => ({
  toolId,
  path: `/home/${toolId}`,
  targetGroupId: toolId,
  available: true,
  discoveryStatus: "ready",
  status: "unassigned",
  desired: false,
  ...extra,
});
const tools = [
  "claude",
  "codex",
  "gemini",
  "hermes",
  "opencode",
  "openclaw",
  "workbuddy",
  "dsh",
  "pi",
];
const server = (
  id: string,
  extra: Partial<McpDefinition> = {},
  overrides: Record<string, Partial<ResourceTarget>> = {},
): McpDefinition => ({
  id,
  name: id,
  revision: 2,
  transport: "stdio",
  credentialState: "nativeEnvironmentRequired",
  commandLine: `npx -y ${id}`,
  fields: [],
  targets: tools.map((tool) => target(tool, overrides[tool])),
  ...extra,
});
const operation = (status: string, reason?: string): LibraryOperation => ({
  operationId: `op-${Math.random()}`,
  kind: "mcp",
  resourceId: "x",
  resourceRevision: 2,
  status: "previewed",
  targets: [
    {
      tools: ["codex"],
      readers: ["codex"],
      path: "/home/codex",
      targetGroupId: "codex",
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
let servers: McpDefinition[];
let inventory: LibraryInventory;
beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  servers = [
    server("filesystem", {}, { codex: { desired: true, status: "applied" } }),
    server("context7", {
      transport: "http",
      commandLine: "mcp.context7.com/mcp",
    }),
  ];
  inventory = {
    ticketId: "ticket",
    kind: "mcp",
    expiresAtMs: Date.now() + 60_000,
    sources: [],
    entries: [
      {
        entryId: "seen",
        kind: "mcp",
        name: "context7",
        sources: [{ sourceId: "s", toolId: "opencode" }],
        matchedResourceIds: ["context7"],
        conflict: false,
      },
      {
        entryId: "new",
        kind: "mcp",
        name: "shadcn-ui",
        sources: [{ sourceId: "s", toolId: "codex" }],
        matchedResourceIds: [],
        conflict: false,
      },
    ],
  };
  vi.restoreAllMocks();
  vi.spyOn(libraryApi, "list").mockImplementation(async () => ({
    epoch: 4,
    servers,
    targets: tools.map((tool) => target(tool)),
    operations: [],
  }));
  vi.spyOn(libraryApi, "inventory").mockImplementation(async () => inventory);
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  vi.spyOn(libraryApi, "apply").mockResolvedValue(applied());
  vi.spyOn(libraryApi, "cancel").mockResolvedValue(operation("cancelled"));
});
function renderTable(onRefresh?: (refresh: () => Promise<unknown>) => void) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  render(
    <QueryClientProvider client={client}>
      <LibraryMcpTable
        registerRefresh={(_kind, refresh) => {
          onRefresh?.(refresh);
          return () => undefined;
        }}
        registerDiscover={() => () => undefined}
        onChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
  return client;
}
const cell = (name: string, tool: string) =>
  screen.getByRole("checkbox", { name: new RegExp(`^${name} · ${tool} · `) });

it("shows loading placeholders until native MCP states have been read", async () => {
  let finish!: (value: LibraryInventory) => void;
  vi.mocked(libraryApi.inventory).mockImplementationOnce(
    () => new Promise((resolve) => (finish = resolve)),
  );
  renderTable();
  await screen.findByTestId("mcp-row-context7");
  expect(screen.getByRole("status")).toHaveTextContent(
    "正在读取应用中的 MCP 状态",
  );
  expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
  const header = screen.getByRole("columnheader", { name: "OpenCode" });
  expect(within(header).queryByText("0")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: /扫描中/ })).toBeDisabled();
  await act(async () => finish(inventory));
  await waitFor(() => expect(cell("context7", "OpenCode")).toBeChecked());
  expect(within(header).getByText("1")).toBeVisible();
  expect(screen.getByRole("status")).toHaveTextContent("MCP 状态已更新");
});

it("keeps the previous states visible but read-only while refreshing", async () => {
  let refresh!: () => Promise<unknown>;
  renderTable((fn) => (refresh = fn));
  await waitFor(() => expect(cell("context7", "OpenCode")).toBeChecked());
  let finish!: (value: LibraryInventory) => void;
  vi.mocked(libraryApi.inventory).mockImplementationOnce(
    () => new Promise((resolve) => (finish = resolve)),
  );
  await act(async () => {
    await refresh();
  });
  expect(screen.getByRole("status")).toHaveTextContent("暂时显示上次结果");
  expect(cell("context7", "OpenCode")).toBeChecked();
  expect(cell("context7", "OpenCode")).toBeDisabled();
  expect(screen.getByRole("button", { name: /扫描中/ })).toBeDisabled();
  await act(async () => finish({ ...inventory, ticketId: "new", entries: [] }));
  await waitFor(() => expect(cell("context7", "OpenCode")).not.toBeChecked());
  expect(cell("context7", "OpenCode")).toBeEnabled();
});

it("shows a failed scan as unknown and retries instead of showing disabled MCPs", async () => {
  vi.mocked(libraryApi.inventory).mockRejectedValueOnce(new Error("offline"));
  renderTable();
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "无法读取应用中的 MCP 状态",
  );
  expect(screen.queryAllByRole("checkbox")).toHaveLength(0);
  screen.getByRole("button", { name: "重试" }).focus();
  await userEvent.keyboard("{Enter}");
  expect(screen.getByRole("searchbox")).toHaveFocus();
  await waitFor(() => expect(cell("context7", "OpenCode")).toBeChecked());
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it.each(["unreadable", "unsupported"])(
  "keeps %s source columns unknown even with a ready sibling source",
  async (state) => {
    vi.mocked(libraryApi.inventory).mockResolvedValueOnce({
      ...inventory,
      sources: [
        {
          sourceId: "good",
          toolId: "opencode",
          label: "nativeMcp",
          state: "ready",
          count: 1,
        },
        {
          sourceId: "bad",
          toolId: "opencode",
          label: "nativeMcp",
          state,
          count: 0,
        },
      ],
    });
    renderTable();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "OpenCode 的 MCP 状态未能完整读取",
    );
    expect(cell("filesystem", "Codex / ChatGPT")).toBeChecked();
    expect(
      screen.queryByRole("checkbox", { name: /^context7 · OpenCode · / }),
    ).not.toBeInTheDocument();
    expect(
      within(
        screen.getByRole("columnheader", { name: "OpenCode" }),
      ).queryByText("0"),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "重试" }));
    await waitFor(() => expect(cell("context7", "OpenCode")).toBeChecked());
  },
);

it("lays MCP servers out as rows with one checkbox per agent and per-agent counts", async () => {
  renderTable();
  const row = await screen.findByTestId("mcp-row-filesystem");
  expect(within(row).getByText("npx -y filesystem")).toBeVisible();
  expect(within(row).getByText("stdio")).toBeVisible();
  expect(screen.getByText("2 个 MCP")).toBeVisible();
  for (const name of [
    "OpenClaw",
    "WorkBuddy",
    "DeepSeek Harness",
    "Pi",
    "Codex / ChatGPT",
  ]) {
    expect(screen.getByRole("columnheader", { name })).toBeVisible();
  }
  expect(
    screen.queryByRole("columnheader", { name: "ChatGPT" }),
  ).not.toBeInTheDocument();
  expect(within(row).getAllByRole("checkbox")).toHaveLength(9);
  expect(cell("filesystem", "Codex / ChatGPT")).toHaveAttribute(
    "data-state",
    "checked",
  );
  expect(cell("filesystem", "Claude Code")).toHaveAttribute(
    "data-state",
    "unchecked",
  );
  await waitFor(() =>
    expect(cell("context7", "OpenCode")).toHaveAttribute(
      "data-state",
      "checked",
    ),
  );
  const header = screen.getByRole("columnheader", { name: "OpenCode" });
  expect(within(header).getByText("1")).toBeInTheDocument();
});

it("writes a server into a tool with one click", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(cell("filesystem", "Claude Code"));
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith(
      "mcp",
      "filesystem",
      ["codex", "claude"],
      2,
      4,
    ),
  );
  expect(libraryApi.apply).toHaveBeenCalledTimes(1);
});

it("removes a server Ofox wrote, and takes over one it finds before removing it", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  const adopt = vi
    .spyOn(libraryApi, "previewMcpAdoption")
    .mockResolvedValue(operation("pending"));
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(cell("filesystem", "Codex / ChatGPT"));
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith("mcp", "filesystem", [], 2, 4),
  );
  await waitFor(() =>
    expect(cell("context7", "OpenCode")).toHaveAttribute(
      "data-state",
      "checked",
    ),
  );
  await waitFor(() => expect(cell("context7", "OpenCode")).toBeEnabled());
  await userEvent.click(cell("context7", "OpenCode"));
  await waitFor(() =>
    expect(adopt).toHaveBeenCalledWith(
      "context7",
      ["opencode"],
      ["opencode"],
      2,
      4,
    ),
  );
  await waitFor(() =>
    expect(preview).toHaveBeenLastCalledWith("mcp", "context7", [], 2, 4),
  );
  expect(libraryApi.apply).toHaveBeenCalledTimes(3);
});

it("refuses to overwrite and says why when a preview hits a conflict", async () => {
  vi.spyOn(libraryApi, "preview").mockResolvedValue(
    operation("conflict", "library.mcpNameConflict"),
  );
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(cell("filesystem", "Gemini CLI"));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "同名原生项目或共享目标已有其他内容，未覆盖",
  );
  expect(libraryApi.cancel).toHaveBeenCalled();
  expect(libraryApi.apply).not.toHaveBeenCalled();
});

it("adds an MCP from the form and enables it in the chosen tool", async () => {
  const save = vi.spyOn(libraryApi, "saveMcp").mockImplementation(async () => {
    servers = [...servers, server("memory")];
    return undefined;
  });
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(screen.getByRole("button", { name: "添加 MCP" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("名称"), {
    target: { value: "memory" },
  });
  fireEvent.change(within(dialog).getByLabelText("命令"), {
    target: { value: "npx" },
  });
  await userEvent.click(
    within(dialog).getByRole("button", { name: /Claude Code/ }),
  );
  await userEvent.click(
    within(dialog).getByRole("button", { name: "保存并启用" }),
  );
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      "memory",
      "memory",
      undefined,
      expect.arrayContaining([
        expect.objectContaining({ path: ["command"], value: "npx" }),
      ]),
      4,
    ),
  );
  await waitFor(() =>
    expect(preview).toHaveBeenCalledWith("mcp", "memory", ["claude"], 2, 4),
  );
  expect(await screen.findByText("已添加 memory")).toBeVisible();
});

it.each([
  ["添加 MCP", "关闭"],
  ["添加 MCP", "取消"],
  ["编辑 filesystem", "关闭"],
])(
  "closes %s with %s without saving and returns focus to its opener",
  async (openerName, closeName) => {
    const save = vi.spyOn(libraryApi, "saveMcp").mockResolvedValue(undefined);
    renderTable();
    await screen.findByTestId("mcp-row-filesystem");
    const opener = screen.getByRole("button", { name: openerName });
    await userEvent.click(opener);
    const dialog = await screen.findByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("名称"), {
      target: { value: "unsaved server" },
    });
    await userEvent.click(
      within(dialog).getByRole("button", { name: closeName }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(save).not.toHaveBeenCalled();
    expect(opener).toHaveFocus();
  },
);

it("disables both exit buttons while an MCP save is in progress", async () => {
  let finish!: () => void;
  vi.spyOn(libraryApi, "saveMcp").mockImplementationOnce(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(screen.getByRole("button", { name: "添加 MCP" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("名称"), {
    target: { value: "memory" },
  });
  fireEvent.change(within(dialog).getByLabelText("命令"), {
    target: { value: "npx" },
  });
  await userEvent.click(within(dialog).getByRole("button", { name: "保存" }));
  expect(within(dialog).getByRole("button", { name: "关闭" })).toBeDisabled();
  expect(within(dialog).getByRole("button", { name: "取消" })).toBeDisabled();
  await userEvent.keyboard("{Escape}");
  expect(dialog).toBeInTheDocument();
  await act(async () => finish());
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
});

it("imports what the tools already have", async () => {
  const imported = vi
    .spyOn(libraryApi, "importInventory")
    .mockResolvedValue({ imported: 1, items: [] } as never);
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  const button = await screen.findByRole("button", { name: "从应用导入 · 1" });
  await userEvent.click(button);
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("ticket", ["new"], 4),
  );
  expect(await screen.findByText("已导入 1 个 MCP")).toBeVisible();
});

it("withdraws a server from its tools before deleting it", async () => {
  const preview = vi
    .spyOn(libraryApi, "preview")
    .mockResolvedValue(operation("pending"));
  const remove = vi
    .spyOn(libraryApi, "deleteMcp")
    .mockResolvedValue(true as never);
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(
    screen.getByRole("button", { name: "filesystem 的更多操作" }),
  );
  await userEvent.click(await screen.findByRole("menuitem", { name: "删除" }));
  const dialog = await screen.findByRole("dialog");
  await userEvent.click(within(dialog).getByRole("button", { name: "删除" }));
  await waitFor(() => expect(remove).toHaveBeenCalledWith("filesystem", 2, 4));
  expect(preview).toHaveBeenCalledWith("mcp", "filesystem", [], 2, 4);
});

it("counts a write that landed as done even when tidying up after it needs a look", async () => {
  vi.spyOn(libraryApi, "preview").mockResolvedValue(operation("pending"));
  vi.mocked(libraryApi.apply).mockResolvedValue({
    ...operation("applied", "library.cleanupNeedsReview"),
    status: "needsReview",
  });
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(cell("filesystem", "Claude Code"));
  await waitFor(() => expect(libraryApi.apply).toHaveBeenCalledTimes(1));
  expect(screen.queryByRole("alert")).toBeNull();
});

it.each(["openclaw", "workbuddy", "dsh", "pi"])(
  "writes to newly supported %s using the same preview/apply flow",
  async (tool) => {
    const preview = vi
      .spyOn(libraryApi, "preview")
      .mockResolvedValue(operation("pending"));
    renderTable();
    await screen.findByTestId("mcp-row-filesystem");
    const label = {
      openclaw: "OpenClaw",
      workbuddy: "WorkBuddy",
      dsh: "DeepSeek Harness",
      pi: "Pi",
    }[tool];
    await userEvent.click(cell("filesystem", label!));
    await waitFor(() => expect(preview).toHaveBeenCalled());
    expect(preview.mock.calls[0][2]).toContain(tool);
  },
);

it("keeps the complete pasted server when the dialog updates its identifier", async () => {
  const save = vi
    .spyOn(libraryApi, "saveMcp")
    .mockResolvedValue({ revision: 1 } as never);
  renderTable();
  await screen.findByTestId("mcp-row-filesystem");
  await userEvent.click(screen.getByRole("button", { name: "添加 MCP" }));
  const dialog = await screen.findByRole("dialog");
  await userEvent.click(
    within(dialog).getByRole("radio", { name: "粘贴配置" }),
  );
  fireEvent.change(within(dialog).getByLabelText("MCP 配置 JSON"), {
    target: {
      value: JSON.stringify({
        mcpServers: {
          pasted: {
            command: "node",
            args: ["", "one two"],
            env: { TEST: "value" },
            cwd: "/project",
          },
        },
      }),
    },
  });
  await userEvent.click(
    within(dialog).getByRole("button", { name: "识别配置" }),
  );
  await userEvent.click(
    within(dialog).getByRole("checkbox", { name: "选择 pasted" }),
  );
  await userEvent.click(
    within(dialog).getByRole("button", { name: "填入表单" }),
  );
  expect(within(dialog).getByLabelText("名称")).toHaveValue("pasted");
  expect(within(dialog).getByLabelText("命令")).toHaveValue("node");
  expect(within(dialog).getByLabelText("配置标识")).toHaveValue("pasted");
  await userEvent.click(within(dialog).getByRole("button", { name: /^保存$/ }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      "pasted",
      "pasted",
      undefined,
      expect.arrayContaining([
        expect.objectContaining({ path: ["command"], value: "node" }),
        expect.objectContaining({ path: ["args"], value: ["", "one two"] }),
        expect.objectContaining({ path: ["cwd"], value: "/project" }),
      ]),
      4,
    ),
  );
});
