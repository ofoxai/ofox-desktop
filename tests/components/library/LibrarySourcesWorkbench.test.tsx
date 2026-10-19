import { LibraryNativeSources } from "@/components/library/LibraryNativeSources";
import { StrictMode } from "react";
import {
  act,
  render,
  screen,
  within,
  fireEvent,
  waitFor,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { InteractionScope } from "@/components/ui/interaction-scope";
import { LibrarySection } from "@/components/library/LibrarySection";
import { libraryApi, type LibraryInventory } from "@/lib/api/library";
const originalScrollIntoView = Element.prototype.scrollIntoView;
afterEach(() => {
  vi.unstubAllGlobals();
  Element.prototype.scrollIntoView = originalScrollIntoView;
});

const resource = (id: string) => ({
  id,
  name: "codegraph",
  revision: 1,
  transport: "stdio",
  credentialState: "ready",
  fields: [],
  targets: [
    {
      toolId: "codex",
      path: "config.toml",
      targetGroupId: "codex",
      available: true,
      discoveryStatus: "ready",
      status: "unassigned",
      desired: false,
    },
  ],
});
const inventory: LibraryInventory = {
  ticketId: "native-ticket",
  kind: "mcp",
  expiresAtMs: Date.now() + 60000,
  sources: [
    {
      sourceId: "source",
      toolId: "codex",
      label: "nativeMcp",
      state: "ready",
      count: 2,
      sourceRef: "source-ref",
      locationKind: "file",
      displayPath: "config.toml",
    },
  ],
  entries: [
    {
      entryId: "native-only",
      kind: "mcp",
      name: "Only native",
      summary: "Local documentation",
      sources: [{ sourceId: "source", toolId: "codex" }],
      matchedResourceIds: [],
      conflict: false,
    },
    {
      entryId: "matched",
      kind: "mcp",
      name: "codegraph",
      sources: [{ sourceId: "source", toolId: "codex" }],
      matchedResourceIds: ["a"],
      conflict: false,
    },
  ],
};
/** Native scans allocate new opaque references, including references held by entries. */
function rotatedInventory(
  ticketId: string,
  base: LibraryInventory = inventory,
): LibraryInventory {
  const ids = new Map(
    base.sources.map((source, index) => [
      source.sourceId,
      `${ticketId}-source-${index}`,
    ]),
  );
  return {
    ...base,
    ticketId,
    sources: base.sources.map((source) => ({
      ...source,
      sourceId: ids.get(source.sourceId)!,
      sourceRef: ids.get(source.sourceId)!,
    })),
    entries: base.entries.map((entry) => ({
      ...entry,
      entryId: `${ticketId}-${entry.entryId}`,
      sources: entry.sources.map((source) => ({
        ...source,
        sourceId: ids.get(source.sourceId)!,
      })),
    })),
  };
}
function renderSection() {
  return render(
    <StrictMode>
      <QueryClientProvider
        client={
          new QueryClient({ defaultOptions: { queries: { retry: false } } })
        }
      >
        <LibrarySection
          kind="mcp"
          registerRefresh={() => () => undefined}
          registerDiscover={() => () => undefined}
          onChanged={vi.fn()}
        />
      </QueryClientProvider>
    </StrictMode>,
  );
}
async function resourceSources() {
  const group = await screen.findByRole("button", {
    name: "codegraph · 2 种配置",
  });
  if (group.getAttribute("aria-expanded") !== "true")
    await userEvent.click(group);
  const row = await screen.findByTestId("library-resource-a");
  const expand = within(row).getByRole("button", {
    name: /展开 codegraph|收起 codegraph/,
  });
  if (expand.getAttribute("aria-expanded") !== "true")
    await userEvent.click(expand);
  return screen.findByRole("region", { name: "codegraph 的工具配置" });
}
beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.spyOn(libraryApi, "list").mockResolvedValue({
    epoch: 1,
    servers: [resource("a"), resource("b")],
    operations: [],
  });
  vi.spyOn(libraryApi, "matchMcpRecipes").mockResolvedValue([]);
  vi.spyOn(libraryApi, "inventorySource").mockImplementation(
    async (ticketId, entryId, sourceRef) => ({
      ticketId,
      entryId,
      sourceRef,
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file",
      isSymlink: false,
      observedAtMs: Date.now(),
      canReveal: true,
      matchedResourceIds: ["a"],
      fields: [],
    }),
  );
  let scan = 0;
  vi.spyOn(libraryApi, "inventory").mockImplementation(async () =>
    ++scan === 1 ? inventory : rotatedInventory(`rescan-${scan}`),
  );
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
});
it("scans in the background once without blocking the library or importing native configuration", async () => {
  let finish!: (value: LibraryInventory) => void;
  vi.mocked(libraryApi.inventory).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const imported = vi.spyOn(libraryApi, "importInventory");
  renderSection();
  expect(await screen.findByText(/我的资源/)).toBeVisible();
  expect(libraryApi.inventory).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "添加 MCP" })).toBeEnabled();
  await act(async () => finish(inventory));
  const native = await screen.findByRole("region", { name: "工具中发现" });
  expect(within(native).getByText("Only native")).toBeVisible();
  expect(imported).not.toHaveBeenCalled();
});
it("searches native results and keeps matched sources available for navigation", async () => {
  renderSection();
  const native = await screen.findByRole("region", { name: "工具中发现" });
  await userEvent.type(
    screen.getByRole("textbox", { name: "搜索已有资源" }),
    "Only native",
  );
  expect(within(native).getByText("Only native")).toBeVisible();
  await userEvent.clear(screen.getByRole("textbox", { name: "搜索已有资源" }));
  const sources = await resourceSources();
  await userEvent.click(
    within(sources).getByRole("button", { name: /返回我的资源/ }),
  );
  await waitFor(() =>
    expect(screen.getByTestId("library-resource-a")).toHaveAttribute(
      "data-located",
      "true",
    ),
  );
});
it("groups variants without treating native presence as an Ofox application or repeating unchecked text", async () => {
  renderSection();
  const group = await screen.findByRole("button", {
    name: /codegraph.*2.*种配置/,
  });
  expect(screen.queryByText(/尚未检查/)).not.toBeInTheDocument();
  await userEvent.click(group);
  expect(screen.getByTestId("library-resource-a")).toBeVisible();
  expect(screen.getByTestId("library-resource-b")).toBeVisible();
  expect(
    screen.getByRole("button", { name: /codegraph · Codex · 配置中已有/ }),
  ).toHaveAttribute("aria-pressed", "false");
  expect(screen.queryByText("Ofox 已应用")).not.toBeInTheDocument();
});

it("marks a tool whose matching entry is switched off as disabled", async () => {
  vi.mocked(libraryApi.inventory).mockResolvedValue({
    ...inventory,
    entries: inventory.entries.map((entry) =>
      entry.entryId === "matched"
        ? {
            ...entry,
            sources: [{ sourceId: "source", toolId: "codex", disabled: true }],
          }
        : entry,
    ),
  });
  renderSection();
  await userEvent.click(
    await screen.findByRole("button", { name: /codegraph.*2.*种配置/ }),
  );
  expect(
    await screen.findByRole("button", {
      name: /codegraph · Codex · 配置中已停用/,
    }),
  ).toBeVisible();
});

it("drops displayed source evidence when definitions change during refresh", async () => {
  let refresh!: () => Promise<unknown>;
  vi.spyOn(libraryApi, "inventorySource").mockResolvedValue({
    ticketId: "native-ticket",
    entryId: "native-only",
    sourceRef: "source-ref",
    toolId: "codex",
    label: "nativeMcp",
    locationKind: "file",
    logicalPath: "/fixture/config.toml",
    isSymlink: false,
    observedAtMs: Date.now(),
    canReveal: true,
    matchedResourceIds: [],
    fields: [{ path: ["secret-field-shape"], valueType: "string" }],
  });
  render(
    <QueryClientProvider client={new QueryClient()}>
      <LibrarySection
        kind="mcp"
        registerRefresh={(_, callback) => {
          refresh = callback;
          return () => undefined;
        }}
        registerDiscover={() => () => undefined}
        onChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
  await userEvent.click(
    await screen.findByRole("button", {
      name: "查看 Only native 的 Codex 来源",
    }),
  );
  await userEvent.click(await screen.findByText("脱敏配置结构"));
  expect(screen.getByText("secret-field-shape")).toBeVisible();
  vi.mocked(libraryApi.list).mockResolvedValue({
    epoch: 1,
    servers: [{ ...resource("a"), revision: 2 }],
    operations: [],
  });
  await act(async () => {
    await refresh();
  });
  expect(screen.queryByText("secret-field-shape")).not.toBeInTheDocument();
  expect(screen.getByText("来源待确认，请重新扫描。")).toBeVisible();
});

it("keeps the last native locate intent when same-ticket responses finish out of order", async () => {
  vi.mocked(libraryApi.inventory).mockResolvedValue({
    ...inventory,
    entries: [{ ...inventory.entries[1], matchedResourceIds: ["a", "b"] }],
  });
  const finishes: ((
    value: import("@/lib/api/library").InventorySourceDetail,
  ) => void)[] = [];
  vi.mocked(libraryApi.inventorySource).mockImplementation(
    () => new Promise((resolve) => finishes.push(resolve)),
  );
  renderSection();
  const native = await resourceSources();
  const locations = within(native).getAllByRole("button", {
    name: /返回我的资源/,
  });
  fireEvent.click(locations[0]);
  fireEvent.click(locations[1]);
  const detail = {
    ticketId: "native-ticket",
    entryId: "matched",
    sourceRef: "source-ref",
    toolId: "codex",
    label: "nativeMcp",
    locationKind: "file" as const,
    isSymlink: false,
    observedAtMs: 0,
    canReveal: true,
    matchedResourceIds: ["a", "b"],
    fields: [],
  };
  await act(async () => {
    finishes[1](detail);
    await Promise.resolve();
  });
  expect(screen.getByTestId("library-resource-b")).toHaveAttribute(
    "data-located",
    "true",
  );
  await act(async () => {
    finishes[0](detail);
    await Promise.resolve();
  });
  expect(screen.getByTestId("library-resource-b")).toHaveAttribute(
    "data-located",
    "true",
  );
});

it("cancels a pending native locate intent when the retained page becomes inactive", async () => {
  let finish!: (
    value: import("@/lib/api/library").InventorySourceDetail,
  ) => void;
  vi.mocked(libraryApi.inventorySource).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const client = new QueryClient();
  const section = (active: boolean) => (
    <QueryClientProvider client={client}>
      <InteractionScope active={active}>
        <LibrarySection
          kind="mcp"
          registerRefresh={() => () => undefined}
          registerDiscover={() => () => undefined}
          onChanged={vi.fn()}
        />
      </InteractionScope>
    </QueryClientProvider>
  );
  const view = render(section(true));
  const native = await resourceSources();
  fireEvent.click(within(native).getByRole("button", { name: /返回我的资源/ }));
  view.rerender(section(false));
  await act(async () => {
    finish({
      ticketId: "native-ticket",
      entryId: "matched",
      sourceRef: "source-ref",
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file",
      isSymlink: false,
      observedAtMs: 0,
      canReveal: true,
      matchedResourceIds: ["a"],
      fields: [],
    });
    await Promise.resolve();
  });
  expect(document.querySelector('[data-located="true"]')).toBeNull();
});

it("focuses and centers a freshly expanded matched variant even when a frame precedes the React commit", async () => {
  const frames: FrameRequestCallback[] = [];
  vi.stubGlobal(
    "requestAnimationFrame",
    vi.fn((callback: FrameRequestCallback) => {
      frames.push(callback);
      return frames.length;
    }),
  );
  const scroll = vi.fn();
  Element.prototype.scrollIntoView = scroll;
  let finish!: (
    detail: Awaited<ReturnType<typeof libraryApi.inventorySource>>,
  ) => void;
  vi.mocked(libraryApi.inventorySource).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  renderSection();
  const native = await resourceSources();
  expect(screen.getByTestId("library-resource-a")).toHaveAttribute(
    "data-located",
    "false",
  );
  fireEvent.click(within(native).getByRole("button", { name: /返回我的资源/ }));
  await act(async () => {
    finish({
      ticketId: "native-ticket",
      entryId: "matched",
      sourceRef: "source-ref",
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file",
      isSymlink: false,
      observedAtMs: 0,
      canReveal: true,
      matchedResourceIds: ["a"],
      fields: [],
    });
    await Promise.resolve();
    frames.splice(0).forEach((frame) => frame(1));
  });
  act(() => {
    frames.splice(0).forEach((frame) => frame(2));
  });
  const row = screen.getByTestId("library-resource-a");
  expect(row).toHaveAttribute("data-located", "true");
  expect(row).toHaveFocus();
  expect(scroll.mock.contexts).toContain(row);
});

it("clears a failed-source locate warning after a successful scan and revalidates the subsequent locate", async () => {
  let finishScan!: (value: LibraryInventory) => void;
  vi.mocked(libraryApi.inventory)
    .mockResolvedValueOnce(inventory)
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishScan = resolve;
        }),
    );
  vi.mocked(libraryApi.inventorySource)
    .mockRejectedValueOnce(new Error("library.inventoryChanged"))
    .mockResolvedValueOnce({
      ticketId: "fresh-ticket",
      entryId: "fresh-ticket-matched",
      sourceRef: "fresh-ticket-source-0",
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file",
      isSymlink: false,
      observedAtMs: 0,
      canReveal: true,
      matchedResourceIds: ["a"],
      fields: [],
    });
  renderSection();
  const native = await resourceSources();
  await userEvent.click(
    within(native).getByRole("button", { name: /返回我的资源/ }),
  );
  expect(await screen.findByText("来源待确认，请重新扫描。")).toBeVisible();
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  await act(async () => finishScan(rotatedInventory("fresh-ticket")));
  expect(
    screen.queryByText("来源待确认，请重新扫描。"),
  ).not.toBeInTheDocument();
  await userEvent.click(
    await within(await resourceSources()).findByRole("button", {
      name: /返回我的资源/,
    }),
  );
  await waitFor(() =>
    expect(screen.getByTestId("library-resource-a")).toHaveAttribute(
      "data-located",
      "true",
    ),
  );
  expect(libraryApi.inventorySource).toHaveBeenLastCalledWith(
    "fresh-ticket",
    "fresh-ticket-matched",
    "fresh-ticket-source-0",
    1,
  );
  expect(
    screen.queryByText("来源待确认，请重新扫描。"),
  ).not.toBeInTheDocument();
});

it("keeps an unrelated export failure visible after a successful source locate", async () => {
  vi.spyOn(libraryApi, "exportSelected").mockRejectedValue(
    new Error("library.packWriteFailed"),
  );
  renderSection();
  const native = await screen.findByRole("region", { name: "工具中发现" });
  await userEvent.click(screen.getByRole("button", { name: "选择" }));
  await userEvent.click(screen.getByRole("checkbox", { name: "选择当前结果" }));
  await userEvent.click(screen.getByRole("button", { name: "导出选中项" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("无法保存资源包");
  await userEvent.click(
    within(native).getByRole("button", { name: "重新扫描" }),
  );
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("alert")).toHaveTextContent("无法保存资源包");
  const sources = await resourceSources();
  await userEvent.click(
    within(sources).getByRole("button", { name: /返回我的资源/ }),
  );
  await screen.findByTestId("library-resource-a");
  expect(screen.getByRole("alert")).toHaveTextContent("无法保存资源包");
});

it("keeps an unconfirmed source warning until that source is successfully rescanned", async () => {
  const failed = {
    ...rotatedInventory("failed-ticket"),
    sources: rotatedInventory("failed-ticket").sources.map((source) => ({
      ...source,
      state: "unreadable",
    })),
  };
  vi.mocked(libraryApi.inventory)
    .mockResolvedValueOnce(inventory)
    .mockResolvedValueOnce(failed)
    .mockResolvedValue(rotatedInventory("manual-fresh"));
  vi.mocked(libraryApi.inventorySource).mockRejectedValueOnce(
    new Error("library.inventoryChanged"),
  );
  renderSection();
  const native = await resourceSources();
  await userEvent.click(
    within(native).getByRole("button", { name: /返回我的资源/ }),
  );
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  expect(await screen.findByText("来源待确认，请重新扫描。")).toBeVisible();
  await userEvent.click(
    within(await screen.findByRole("region", { name: "工具中发现" })).getByRole(
      "button",
      { name: "重新扫描" },
    ),
  );
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(3));
  await waitFor(() =>
    expect(
      screen.queryByText("来源待确认，请重新扫描。"),
    ).not.toBeInTheDocument(),
  );
});

it("hands focus from a source details dialog to the located row after its close autofocus settles", async () => {
  const scroll = vi.fn();
  Element.prototype.scrollIntoView = scroll;
  renderSection();
  const native = await resourceSources();
  await userEvent.click(
    within(native).getByRole("button", {
      name: "查看 codegraph 的 Codex 来源",
    }),
  );
  const dialog = await screen.findByRole("dialog", { name: "来源详情" });
  await userEvent.click(
    await within(dialog).findByRole("button", { name: /返回我的资源/ }),
  );
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  const row = screen.getByTestId("library-resource-a");
  expect(row).toHaveFocus();
  expect(row).toHaveAttribute("data-located", "true");
  expect(scroll.mock.contexts).toContain(row);
});

it("does not let a late locate steal focus from a newer add dialog", async () => {
  const scroll = vi.fn();
  Element.prototype.scrollIntoView = scroll;
  let finish!: (
    detail: Awaited<ReturnType<typeof libraryApi.inventorySource>>,
  ) => void;
  vi.mocked(libraryApi.inventorySource).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  renderSection();
  const native = await resourceSources();
  fireEvent.click(within(native).getByRole("button", { name: /返回我的资源/ }));
  await userEvent.click(screen.getByRole("button", { name: "添加 MCP" }));
  const dialog = screen.getByRole("dialog", { name: "添加 MCP" });
  await act(async () =>
    finish({
      ticketId: "native-ticket",
      entryId: "matched",
      sourceRef: "source-ref",
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file",
      isSymlink: false,
      observedAtMs: 0,
      canReveal: true,
      matchedResourceIds: ["a"],
      fields: [],
    }),
  );
  expect(dialog).toContainElement(document.activeElement as HTMLElement);
  expect(screen.getByTestId("library-resource-a")).toHaveAttribute(
    "data-located",
    "false",
  );
  expect(scroll).not.toHaveBeenCalled();
});

it("does not replay an unconsumed locate focus when the retained page is reactivated", async () => {
  const scroll = vi.fn();
  Element.prototype.scrollIntoView = scroll;
  let finish!: (
    detail: Awaited<ReturnType<typeof libraryApi.inventorySource>>,
  ) => void;
  vi.mocked(libraryApi.inventorySource).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const client = new QueryClient();
  const section = (active: boolean) => (
    <QueryClientProvider client={client}>
      <InteractionScope active={active}>
        <LibrarySection
          kind="mcp"
          registerRefresh={() => () => undefined}
          registerDiscover={() => () => undefined}
          onChanged={vi.fn()}
        />
      </InteractionScope>
    </QueryClientProvider>
  );
  const view = render(section(true));
  const native = await resourceSources();
  fireEvent.click(within(native).getByRole("button", { name: /返回我的资源/ }));
  await act(async () => {
    finish({
      ticketId: "native-ticket",
      entryId: "matched",
      sourceRef: "source-ref",
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file",
      isSymlink: false,
      observedAtMs: 0,
      canReveal: true,
      matchedResourceIds: ["a"],
      fields: [],
    });
    await Promise.resolve();
    view.rerender(section(false));
  });
  expect(scroll).not.toHaveBeenCalled();
  view.rerender(section(true));
  expect(scroll).not.toHaveBeenCalled();
  expect(screen.getByTestId("library-resource-a")).not.toHaveFocus();
});

it.each([
  "old ambiguity",
  "new ambiguity",
  "old missing location",
  "new missing location",
  "old file missing path",
  "new file missing path",
  "different source",
  "fresh unreadable",
  "fresh unsupported",
])("preserves the source warning after a scan with %s", async (scenario) => {
  let old = inventory;
  let freshBase = inventory;
  if (scenario === "old ambiguity")
    old = {
      ...inventory,
      sources: [
        ...inventory.sources,
        {
          ...inventory.sources[0],
          sourceId: "other-old",
          sourceRef: "other-old",
        },
      ],
    };
  if (scenario === "new ambiguity")
    freshBase = {
      ...inventory,
      sources: [
        ...inventory.sources,
        {
          ...inventory.sources[0],
          sourceId: "other-new",
          sourceRef: "other-new",
        },
      ],
    };
  if (scenario === "old missing location")
    old = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        locationKind: undefined,
        displayPath: undefined,
      })),
    };
  if (scenario === "new missing location")
    freshBase = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        locationKind: undefined,
        displayPath: undefined,
      })),
    };
  if (scenario === "old file missing path")
    old = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        displayPath: undefined,
      })),
    };
  if (scenario === "new file missing path")
    freshBase = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        displayPath: undefined,
      })),
    };
  if (scenario === "different source")
    freshBase = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        displayPath: "different.toml",
      })),
    };
  if (scenario === "fresh unreadable" || scenario === "fresh unsupported")
    freshBase = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        state: scenario === "fresh unreadable" ? "unreadable" : "unsupported",
      })),
    };
  vi.mocked(libraryApi.inventory)
    .mockResolvedValueOnce(old)
    .mockResolvedValue(rotatedInventory("rotated-fresh", freshBase));
  vi.mocked(libraryApi.inventorySource).mockRejectedValueOnce(
    new Error("library.inventoryChanged"),
  );
  renderSection();
  const native = await resourceSources();
  await userEvent.click(
    within(native).getByRole("button", { name: /返回我的资源/ }),
  );
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  expect(screen.getByText("来源待确认，请重新扫描。")).toBeVisible();
});

it.each(["inline", "legacy"] as const)(
  "uses an explicit unique %s location kind without inventing a file path",
  async (locationKind) => {
    const inline = {
      ...inventory,
      sources: inventory.sources.map((source) => ({
        ...source,
        label: locationKind === "inline" ? "nativeMcpInline" : "legacyPrompts",
        locationKind,
        displayPath: undefined,
      })),
    };
    let finish!: (value: LibraryInventory) => void;
    vi.mocked(libraryApi.inventory)
      .mockResolvedValueOnce(inline)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      );
    vi.mocked(libraryApi.inventorySource).mockRejectedValueOnce(
      new Error("library.inventoryChanged"),
    );
    renderSection();
    const native = await resourceSources();
    await userEvent.click(
      within(native).getByRole("button", { name: /返回我的资源/ }),
    );
    expect(await screen.findByText("来源待确认，请重新扫描。")).toBeVisible();
    await act(async () => finish(rotatedInventory("fresh-inline", inline)));
    expect(
      screen.queryByText("来源待确认，请重新扫描。"),
    ).not.toBeInTheDocument();
  },
);

it("keeps an old-backend warning without location evidence until a fresh locate is verified", async () => {
  const old = {
    ...inventory,
    sources: inventory.sources.map((source) => ({
      ...source,
      locationKind: undefined,
      displayPath: undefined,
    })),
  };
  let finish!: (value: LibraryInventory) => void;
  vi.mocked(libraryApi.inventory)
    .mockResolvedValueOnce(old)
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
  vi.mocked(libraryApi.inventorySource).mockRejectedValueOnce(
    new Error("library.inventoryChanged"),
  );
  renderSection();
  await resourceSources();
  await userEvent.click(
    within(await resourceSources()).getByRole("button", {
      name: /返回我的资源/,
    }),
  );
  expect(await screen.findByText("来源待确认，请重新扫描。")).toBeVisible();
  await act(async () => finish(rotatedInventory("verified-fresh")));
  expect(screen.getByText("来源待确认，请重新扫描。")).toBeVisible();
  await userEvent.click(
    within(await resourceSources()).getByRole("button", {
      name: /返回我的资源/,
    }),
  );
  await waitFor(() =>
    expect(libraryApi.inventorySource).toHaveBeenLastCalledWith(
      "verified-fresh",
      "verified-fresh-matched",
      "verified-fresh-source-0",
      1,
    ),
  );
  expect(
    screen.queryByText("来源待确认，请重新扫描。"),
  ).not.toBeInTheDocument();
});

it("does not report no local resources when every discovered item is already collected", async () => {
  vi.mocked(libraryApi.inventory).mockResolvedValue({
    ...inventory,
    entries: inventory.entries.filter(
      (entry) => entry.matchedResourceIds.length > 0,
    ),
  });
  renderSection();
  const native = await screen.findByRole("region", { name: "工具中发现" });
  expect(within(native).queryByRole("button", { name: /已收录/ })).toBeNull();
  expect(
    within(native).getByText("已发现的配置都已加入我的资源。"),
  ).toBeVisible();
  expect(
    within(native).queryByText(zh.library.workbench.noNativeResources),
  ).not.toBeInTheDocument();
});
it("does not describe an unreadable native source as an empty inventory", async () => {
  vi.mocked(libraryApi.inventory).mockResolvedValue({
    ...inventory,
    entries: [],
    sources: inventory.sources.map((source) => ({
      ...source,
      state: "unreadable",
    })),
  });
  renderSection();
  const native = await screen.findByRole("region", { name: "工具中发现" });
  expect(
    within(native).queryByText(zh.library.workbench.noNativeResources),
  ).not.toBeInTheDocument();
  expect(within(native).getAllByRole("status").length).toBeGreaterThan(0);
});

it("returns from a fully added tool-only view to My resources without changing tool configurations", async () => {
  Element.prototype.scrollIntoView = vi.fn();
  vi.mocked(libraryApi.inventory).mockResolvedValue({
    ...inventory,
    entries: inventory.entries.filter(
      (entry) => entry.matchedResourceIds.length > 0,
    ),
  });
  const apply = vi.spyOn(libraryApi, "previewBatch");
  renderSection();
  await screen.findByText("我的资源 · 2");
  await userEvent.click(screen.getByRole("button", { name: "筛选" }));
  await userEvent.click(screen.getByRole("combobox", { name: "显示范围" }));
  await userEvent.click(screen.getByRole("option", { name: "工具中发现" }));
  await userEvent.keyboard("{Escape}");
  expect(screen.queryByText("我的资源 · 2")).toBeNull();
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => {
    callback(0);
    return 1;
  });
  await userEvent.click(screen.getByRole("button", { name: "查看我的资源" }));
  await waitFor(() =>
    expect(document.getElementById("library-owned-mcp")).toHaveFocus(),
  );
  expect(screen.getByText("我的资源 · 2")).toBeVisible();
  expect(apply).not.toHaveBeenCalled();
});

it("keeps the inline source region and keyboard focus during a rescan without showing stale matches", async () => {
  renderSection();
  const sources = await resourceSources();
  const rescan = within(sources).getByRole("button", { name: "重新扫描" });
  let finish!: (value: LibraryInventory) => void;
  vi.mocked(libraryApi.inventory).mockImplementationOnce(
    () => new Promise((resolve) => (finish = resolve)),
  );
  rescan.focus();
  await userEvent.click(rescan);
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("region", { name: "codegraph 的工具配置" })).toBe(
    sources,
  );
  expect(rescan).toHaveFocus();
  expect(
    within(sources).getByText(zh.library.workbench2.scanning),
  ).toBeVisible();
  expect(
    within(sources).queryByRole("button", { name: /codegraph.*Codex.*来源/ }),
  ).toBeNull();
  await act(async () => finish(rotatedInventory("refreshed")));
  expect(screen.getByRole("region", { name: "codegraph 的工具配置" })).toBe(
    sources,
  );
  expect(rescan).toHaveFocus();
  expect(
    within(sources).getByRole("button", { name: /codegraph.*Codex.*来源/ }),
  ).toBeEnabled();
});

it.each([
  { tool: "gemini", status: "all" },
  { tool: "all", status: "applied" },
])(
  "reports no matches for a filtered native inventory instead of claiming discovery is empty: %s",
  ({ tool, status }) => {
    render(
      <LibraryNativeSources
        inventory={inventory}
        loading={false}
        failed={false}
        search=""
        tool={tool}
        status={status}
        busy={false}
        onRescan={vi.fn()}
        onImport={vi.fn()}
        onSource={vi.fn()}
        onLocate={vi.fn()}
        onCompare={vi.fn()}
      />,
    );
    expect(screen.getByText(zh.library.noMatch)).toBeVisible();
    expect(
      screen.queryByText(zh.library.workbench.noNativeResources),
    ).not.toBeInTheDocument();
  },
);
