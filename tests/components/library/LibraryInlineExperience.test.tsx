import { McpCheck } from "@/components/library/McpCheck";
import { DraftStore } from "@/components/library/useDraftState";
import { InteractionScope } from "@/components/ui/interaction-scope";
import { useEffect } from "react";
import { LibraryPage } from "@/components/library/LibraryPage";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
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
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { LibrarySection } from "@/components/library/LibrarySection";
import { LibraryTargets } from "@/components/library/LibraryTargets";
import {
  libraryApi,
  type LibraryData,
  type LibraryInventory,
  type LibraryKind,
} from "@/lib/api/library";
import { listen } from "@tauri-apps/api/event";

vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
vi.mock("@/components/MarkdownEditor", () => ({
  default: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Markdown"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));
const target = {
  toolId: "codex",
  path: "/fixture/config.toml",
  targetGroupId: "codex",
  available: true,
  discoveryStatus: "ready",
  status: "unassigned",
  desired: false,
};
const server = {
  id: "docs",
  name: "Local docs",
  revision: 1,
  transport: "stdio",
  credentialState: "ready",
  displaySummary: "npx · chrome-devtools-mcp",
  fields: [],
  targets: [target],
};
let data: LibraryData;
let inventory: LibraryInventory;
let refresh: () => Promise<unknown>;
let client: QueryClient;
function mount(kind: LibraryKind = "mcp") {
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <LibrarySection
        kind={kind}
        registerRefresh={(_, action) => {
          refresh = action;
          return () => undefined;
        }}
        registerDiscover={() => () => undefined}
        onChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
}
beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  HTMLElement.prototype.scrollIntoView = vi.fn();
  data = { epoch: 1, servers: [server], operations: [] };
  inventory = {
    ticketId: "scan",
    kind: "mcp",
    expiresAtMs: Date.now() + 600000,
    sources: [
      {
        sourceId: "s",
        toolId: "codex",
        label: "nativeMcp",
        state: "ready",
        count: 1,
        sourceRef: "ref",
        locationKind: "file",
      },
    ],
    entries: [
      {
        entryId: "native",
        kind: "mcp",
        name: "Only native",
        summary: "npx · chrome-devtools-mcp",
        sources: [{ sourceId: "s", toolId: "codex" }],
        matchedResourceIds: [],
        conflict: false,
      },
    ],
  };
  vi.spyOn(libraryApi, "list").mockImplementation(async () => data);
  vi.spyOn(libraryApi, "inventory").mockImplementation(async () => inventory);
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  vi.spyOn(libraryApi, "matchMcpRecipes").mockResolvedValue([]);
  vi.spyOn(libraryApi, "matchBuiltinRecipes").mockResolvedValue([]);
  vi.mocked(listen).mockResolvedValue(vi.fn());
});

it("keeps discovery in the page while the native scan is slow and never starts a server", async () => {
  vi.mocked(libraryApi.inventory).mockImplementation(
    () => new Promise(() => undefined),
  );
  const check = vi.spyOn(libraryApi, "prepareMcpCheck");
  mount();
  const discovery = await screen.findByRole("region", { name: "发现 MCP" });
  expect(
    within(discovery).getByRole("article", { name: "Context7" }),
  ).toBeVisible();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(check).not.toHaveBeenCalled();
});

it("collects one native item using its existing ticket and locates the saved resource without touching tools", async () => {
  const apply = vi.spyOn(libraryApi, "previewBatch");
  const imported = vi
    .spyOn(libraryApi, "importInventory")
    .mockImplementation(async () => {
      data = {
        ...data,
        servers: [server, { ...server, id: "collected", name: "Only native" }],
      };
      return {
        imported: 1,
        items: [
          {
            entryId: "native",
            resourceId: "collected",
            name: "Only native",
            status: "imported",
          },
        ],
      };
    });
  mount();
  const native = await screen.findByRole("region", { name: "本机已有" });
  await userEvent.click(
    within(native).getByRole("button", { name: "收录 Only native" }),
  );
  await waitFor(() =>
    expect(imported).toHaveBeenCalledWith("scan", ["native"], 1),
  );
  expect(
    await screen.findByTestId("library-resource-collected"),
  ).toHaveAttribute("data-located", "true");
  expect(apply).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("opens MCP details inline and keeps full explicit check results only for the current revision", async () => {
  const check = vi
    .spyOn(libraryApi, "prepareMcpCheck")
    .mockResolvedValue({ operationId: "check" });
  vi.spyOn(libraryApi, "runMcpCheck").mockResolvedValue({
    operationId: "check",
    state: "ok",
    tools: [{ name: "lookup", description: "Find documentation" }],
  });
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "展开 Local docs" }),
  );
  const details = screen.getByRole("region", { name: "Local docs 详情" });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(check).not.toHaveBeenCalled();
  await userEvent.click(
    within(details).getByRole("button", { name: "开始检查" }),
  );
  expect(await within(details).findByText("lookup")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "收起 Local docs" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "展开 Local docs" }),
  );
  expect(
    within(screen.getByRole("region", { name: "Local docs 详情" })).getByText(
      "lookup",
    ),
  ).toBeVisible();
  expect(check).toHaveBeenCalledTimes(1);
  data = { ...data, servers: [{ ...server, revision: 2 }] };
  await act(async () => refresh());
  await waitFor(() =>
    expect(screen.queryByText("lookup")).not.toBeInTheDocument(),
  );
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await screen.findByText("lookup");
  await userEvent.click(
    screen.getByRole("button", { name: "收起 Local docs" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "展开 Local docs" }),
  );
  expect(screen.getByText("lookup")).toBeVisible();
});

it("does not discard a changed MCP form on Escape", async () => {
  const save = vi.spyOn(libraryApi, "saveMcp");
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "添加 MCP" }),
  );
  await userEvent.type(
    screen.getByRole("textbox", { name: "名称" }),
    "Unsaved MCP",
  );
  await userEvent.keyboard("{Escape}");
  expect(
    await screen.findByRole("heading", { name: "有未保存的修改" }),
  ).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  expect(screen.getByRole("textbox", { name: "名称" })).toHaveValue(
    "Unsaved MCP",
  );
  expect(save).not.toHaveBeenCalled();
});

it("keeps dirty instruction edits when cancelling a row switch, and saves before switching", async () => {
  const rules = {
    id: "rules",
    name: "Team rules",
    content: "Original",
    revision: 1,
  };
  data = {
    epoch: 1,
    instructions: [rules, { ...rules, id: "other", name: "Other rules" }],
    targets: [],
    operations: [],
  };
  const saved = vi
    .spyOn(libraryApi, "saveInstruction")
    .mockImplementation(async (set) => {
      data = {
        ...data,
        instructions: [{ ...set, revision: 2 }, data.instructions![1]],
      };
      return { ...set, revision: 2 };
    });
  mount("instructions");
  await userEvent.click(
    await screen.findByRole("button", { name: "展开 Team rules" }),
  );
  fireEvent.change(screen.getByLabelText("Markdown"), {
    target: { value: "Unsaved" },
  });
  await userEvent.click(
    screen.getByRole("button", { name: "展开 Other rules" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  expect(screen.getByLabelText("Markdown")).toHaveValue("Unsaved");
  await userEvent.click(
    screen.getByRole("button", { name: "展开 Other rules" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "保存并继续" }));
  await waitFor(() =>
    expect(saved).toHaveBeenCalledWith(
      expect.objectContaining({ id: "rules", content: "Unsaved" }),
      1,
      1,
    ),
  );
  expect(
    await screen.findByRole("region", { name: "Other rules 详情" }),
  ).toBeVisible();
});

it("shows native presence as a visible distinct state without claiming an application", () => {
  const toggle = vi.fn();
  render(
    <LibraryTargets
      name="Docs"
      targets={[target]}
      observedTools={["codex"]}
      busy={false}
      onToggle={toggle}
    />,
  );
  const observed = screen.getByRole("button", {
    name: /Docs · Codex · 配置中已有/,
  });
  expect(observed).toHaveAttribute("data-state", "observed");
  expect(observed).toHaveAttribute("aria-pressed", "false");
  expect(within(observed).getByText("◇")).toBeVisible();
});

it("reads Skills as safe inline Markdown without loading embedded resources", async () => {
  data = {
    epoch: 1,
    skills: [
      {
        id: "skill",
        name: "Design",
        description: "Design interfaces",
        directory: "design",
        revision: 1,
        artifactHash: "hash",
        sourceState: "ready",
        targets: [],
      },
    ],
    operations: [],
  };
  vi.spyOn(libraryApi, "skillDetail").mockResolvedValue({
    resourceId: "skill",
    artifactHash: "hash",
    path: "SKILL.md",
    content:
      "# Design guide\n\n![tracker](https://example.test/pixel)\n<script>unsafe()</script>",
    files: [],
  });
  mount("skills");
  await userEvent.click(
    await screen.findByRole("button", { name: "展开 Design" }),
  );
  expect(
    await screen.findByRole("heading", { name: "Design guide" }),
  ).toBeVisible();
  expect(
    document.querySelector('img[src="https://example.test/pixel"]'),
  ).toBeNull();
  expect(document.querySelector("script")).toBeNull();
});

it("does not accept an old dialog check after the configuration route changes", async () => {
  let finish!: (
    value: Awaited<ReturnType<typeof libraryApi.runMcpCheck>>,
  ) => void;
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "route-check",
  });
  vi.spyOn(libraryApi, "runMcpCheck").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "Local docs 的更多操作" }),
  );
  await userEvent.click(
    await screen.findByRole("menuitem", {
      name: zh.library.mcpCheckNamed.replace("{{name}}", "Local docs"),
    }),
  );
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(libraryApi.runMcpCheck).toHaveBeenCalled());
  await act(async () => {
    client.setQueryData(["settings"], { codexConfigDir: "/fixture/another" });
  });
  await act(async () =>
    finish({
      operationId: "route-check",
      state: "ok",
      tools: [{ name: "stale-lookup" }],
    }),
  );
  expect(screen.queryByText("stale-lookup")).not.toBeInTheDocument();
});
it("protects edits made only in the advanced MCP editor", async () => {
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "添加 MCP" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: zh.library.addFlow.advanced }),
  );
  await userEvent.type(
    screen.getByRole("textbox", { name: "名称" }),
    "Advanced unsaved",
  );
  await userEvent.keyboard("{Escape}");
  expect(
    await screen.findByRole("heading", { name: "有未保存的修改" }),
  ).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  expect(screen.getByRole("textbox", { name: "名称" })).toHaveValue(
    "Advanced unsaved",
  );
});

function ShellNavigation() {
  const shell = useShell();
  useEffect(() => shell.setActive("library"), [shell.setActive]);
  return (
    <>
      <button onClick={() => shell.locateUsage("native-session")}>
        Leave library
      </button>
      <output>{shell.active}</output>
      <output>{shell.usageLocation?.key}</output>
    </>
  );
}
function mountPage() {
  vi.spyOn(libraryApi, "summary").mockResolvedValue({
    instructions: 1,
    mcp: 1,
    skills: 1,
  });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ShellProvider openSettings={() => undefined}>
        <div data-testid="scroll" style={{ overflowY: "auto", height: 600 }}>
          <LibraryPage />
        </div>
        <ShellNavigation />
      </ShellProvider>
    </QueryClientProvider>,
  );
}
it("retains each category's scroll position independently", async () => {
  data = {
    epoch: 1,
    instructions: [
      { id: "rules", name: "Rules", content: "Original", revision: 1 },
    ],
    targets: [],
    operations: [],
  };
  mountPage();
  await screen.findByRole("button", { name: "展开 Rules" });
  const scroll = screen.getByTestId("scroll");
  scroll.scrollTop = 420;
  fireEvent.scroll(scroll);
  await userEvent.click(screen.getByRole("tab", { name: /^MCP/ }));
  expect(scroll.scrollTop).toBe(0);
  scroll.scrollTop = 90;
  fireEvent.scroll(scroll);
  await userEvent.click(screen.getByRole("tab", { name: /^指令/ }));
  expect(scroll.scrollTop).toBe(420);
});
it("guards both category changes and shell navigation without queueing a cancelled usage jump", async () => {
  data = {
    epoch: 1,
    instructions: [
      { id: "rules", name: "Rules", content: "Original", revision: 1 },
    ],
    targets: [],
    operations: [],
  };
  mountPage();
  await userEvent.click(
    await screen.findByRole("button", { name: "展开 Rules" }),
  );
  fireEvent.change(screen.getByLabelText("Markdown"), {
    target: { value: "Dirty" },
  });
  await userEvent.click(screen.getByRole("tab", { name: /^MCP/ }));
  await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  expect(screen.getByRole("tab", { name: /^指令/ })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await userEvent.click(screen.getByRole("button", { name: "Leave library" }));
  await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  expect(screen.getByText("library", { selector: "output" })).toBeVisible();
  expect(
    screen.queryByText("native-session", { selector: "output" }),
  ).not.toBeInTheDocument();
  await userEvent.click(screen.getByRole("button", { name: "Leave library" }));
  await userEvent.click(screen.getByRole("button", { name: "放弃并继续" }));
  expect(screen.getByText("usage", { selector: "output" })).toBeVisible();
  expect(
    screen.getByText("native-session", { selector: "output" }),
  ).toBeVisible();
});

it("cancels a late prepared check after leaving and returning to its route before executing", async () => {
  let finish!: (value: { operationId: string }) => void;
  vi.spyOn(libraryApi, "prepareMcpCheck").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const run = vi
    .spyOn(libraryApi, "runMcpCheck")
    .mockResolvedValue({ operationId: "late-route", state: "ok", tools: [] });
  const cancel = vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "Local docs 的更多操作" }),
  );
  await userEvent.click(
    await screen.findByRole("menuitem", {
      name: zh.library.mcpCheckNamed.replace("{{name}}", "Local docs"),
    }),
  );
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(libraryApi.prepareMcpCheck).toHaveBeenCalled());
  await act(async () => {
    client.setQueryData(["settings"], { codexConfigDir: "/fixture/other" });
  });
  await screen.findByText(zh.library.workbench2.staleEdit);
  await act(async () => {
    client.setQueryData(["settings"], {});
  });
  await act(async () => finish({ operationId: "late-route" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith("late-route"));
  expect(run).not.toHaveBeenCalled();
});
it("discards a modal instruction draft once without asking again", async () => {
  data = { epoch: 1, instructions: [], targets: [], operations: [] };
  mount("instructions");
  await userEvent.click(await screen.findByRole("button", { name: "新建" }));
  await userEvent.type(
    screen.getByRole("textbox", { name: "名称" }),
    "Discard me",
  );
  await userEvent.click(
    screen.getByRole("button", {
      name: zh.library.instructionExperience.discard,
    }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("carries an unsaved ordinary MCP draft into advanced editing", async () => {
  mount();
  await userEvent.click(
    await screen.findByRole("button", { name: "添加 MCP" }),
  );
  await userEvent.type(
    screen.getByRole("textbox", { name: "名称" }),
    "Inherited unsaved",
  );
  await userEvent.click(
    screen.getByRole("button", { name: zh.library.addFlow.advanced }),
  );
  await userEvent.keyboard("{Escape}");
  expect(
    await screen.findByRole("heading", { name: "有未保存的修改" }),
  ).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "继续编辑" }));
  expect(screen.getByRole("textbox", { name: "名称" })).toHaveValue(
    "Inherited unsaved",
  );
});

it("cancels an inline check on category suspension and does not publish a late result", async () => {
  let finish!: (
    result: Awaited<ReturnType<typeof libraryApi.runMcpCheck>>,
  ) => void;
  vi.spyOn(libraryApi, "prepareMcpCheck").mockResolvedValue({
    operationId: "suspended",
  });
  vi.spyOn(libraryApi, "runMcpCheck").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const cancel = vi.spyOn(libraryApi, "cancelMcpCheck").mockResolvedValue(true);
  const onResult = vi.fn(),
    store = new DraftStore(),
    onBusy = vi.fn();
  const tree = (active: boolean) => (
    <InteractionScope active={active}>
      <McpCheck
        store={store}
        server={server}
        epoch={1}
        readOnly={false}
        onBusyChange={onBusy}
        onResult={onResult}
      />
    </InteractionScope>
  );
  const rendered = render(tree(true));
  await userEvent.click(screen.getByRole("button", { name: "开始检查" }));
  await waitFor(() => expect(libraryApi.runMcpCheck).toHaveBeenCalled());
  rendered.rerender(tree(false));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith("suspended"));
  await act(async () =>
    finish({
      operationId: "suspended",
      state: "ok",
      tools: [{ name: "late-tool" }],
    }),
  );
  rendered.rerender(tree(true));
  expect(onResult).not.toHaveBeenCalled();
  expect(screen.queryByText("late-tool")).not.toBeInTheDocument();
});

it("retains keyboard focus when saving an inline instruction creates a new revision", async () => {
  const rule = { id: "rules", name: "Rules", content: "Original", revision: 1 };
  data = { epoch: 1, instructions: [rule], targets: [], operations: [] };
  vi.spyOn(libraryApi, "saveInstruction").mockImplementation(async (set) => {
    const saved = { ...set, revision: 2 };
    data = { ...data, instructions: [saved] };
    return saved;
  });
  mount("instructions");
  await userEvent.click(
    await screen.findByRole("button", { name: "展开 Rules" }),
  );
  await userEvent.type(
    screen.getByRole("textbox", { name: "名称" }),
    " changed",
  );
  await userEvent.keyboard("{Control>}s{/Control}");
  await waitFor(() => expect(libraryApi.list).toHaveBeenCalledTimes(2));
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "名称" })).toHaveFocus(),
  );
});
