import {
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
const tools = ["claude", "codex", "gemini", "hermes", "opencode"];
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
function renderTable() {
  render(
    <QueryClientProvider
      client={
        new QueryClient({ defaultOptions: { queries: { retry: false } } })
      }
    >
      <LibraryMcpTable
        registerRefresh={() => () => undefined}
        registerDiscover={() => () => undefined}
        onChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
}
const cell = (name: string, tool: string) =>
  screen.getByRole("checkbox", { name: new RegExp(`^${name} · ${tool} · `) });

it("lays MCP servers out as rows with one checkbox per agent and per-agent counts", async () => {
  renderTable();
  const row = await screen.findByTestId("mcp-row-filesystem");
  expect(within(row).getByText("npx -y filesystem")).toBeVisible();
  expect(within(row).getByText("stdio")).toBeVisible();
  expect(screen.getByText("2 个 MCP")).toBeVisible();
  expect(cell("filesystem", "Codex")).toHaveAttribute("data-state", "checked");
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
  await userEvent.click(cell("filesystem", "Codex"));
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
