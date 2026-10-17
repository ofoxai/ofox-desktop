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
beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.spyOn(libraryApi, "list").mockResolvedValue({
    epoch: 1,
    servers: [resource("a"), resource("b")],
    operations: [],
  });
  vi.spyOn(libraryApi, "matchMcpRecipes").mockResolvedValue([]);
  vi.spyOn(libraryApi, "inventorySource").mockResolvedValue({
    ticketId: "native-ticket",
    entryId: "matched",
    sourceRef: "source-ref",
    toolId: "codex",
    label: "nativeMcp",
    locationKind: "file",
    isSymlink: false,
    observedAtMs: Date.now(),
    canReveal: true,
    matchedResourceIds: ["a"],
    fields: [],
  });
  vi.spyOn(libraryApi, "inventory").mockResolvedValue(inventory);
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
  expect(await screen.findByText(/资源库中/)).toBeVisible();
  expect(libraryApi.inventory).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "添加 MCP" })).toBeEnabled();
  await act(async () => finish(inventory));
  const native = await screen.findByRole("region", { name: "本机配置中" });
  expect(within(native).getByText("Only native")).toBeVisible();
  expect(imported).not.toHaveBeenCalled();
});
it("searches native results and keeps matched sources available for navigation", async () => {
  renderSection();
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.type(
    screen.getByRole("textbox", { name: "搜索资源" }),
    "Only native",
  );
  expect(within(native).getByText("Only native")).toBeVisible();
  await userEvent.clear(screen.getByRole("textbox", { name: "搜索资源" }));
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  await userEvent.click(
    within(native).getByRole("button", { name: /定位到资源库/ }),
  );
  expect(await screen.findByTestId("library-resource-a")).toHaveAttribute(
    "data-located",
    "true",
  );
});
it("groups variants without treating native presence as an OFox application or repeating unchecked text", async () => {
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
  expect(screen.queryByText("OFox 已应用")).not.toBeInTheDocument();
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  const locations = within(native).getAllByRole("button", {
    name: /定位到资源库/,
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  fireEvent.click(within(native).getByRole("button", { name: /定位到资源库/ }));
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  expect(screen.queryByTestId("library-resource-a")).toBeNull();
  fireEvent.click(within(native).getByRole("button", { name: /定位到资源库/ }));
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
  renderSection();
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  await userEvent.click(
    within(native).getByRole("button", { name: /定位到资源库/ }),
  );
  expect(await screen.findByText("来源待确认，请重新扫描。")).toBeVisible();
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  await act(async () => finishScan({ ...inventory, ticketId: "fresh-ticket" }));
  expect(
    screen.queryByText("来源待确认，请重新扫描。"),
  ).not.toBeInTheDocument();
  await userEvent.click(
    await within(native).findByRole("button", { name: /定位到资源库/ }),
  );
  expect(await screen.findByTestId("library-resource-a")).toHaveAttribute(
    "data-located",
    "true",
  );
  expect(libraryApi.inventorySource).toHaveBeenLastCalledWith(
    "fresh-ticket",
    "matched",
    "source-ref",
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(screen.getByRole("button", { name: "选择" }));
  await userEvent.click(screen.getByRole("checkbox", { name: "选择当前结果" }));
  await userEvent.click(screen.getByRole("button", { name: "导出选中项" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("无法保存资源包");
  await userEvent.click(
    within(native).getByRole("button", { name: "重新扫描" }),
  );
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("alert")).toHaveTextContent("无法保存资源包");
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  await userEvent.click(
    within(native).getByRole("button", { name: /定位到资源库/ }),
  );
  await screen.findByTestId("library-resource-a");
  expect(screen.getByRole("alert")).toHaveTextContent("无法保存资源包");
});

it("keeps an unconfirmed source warning until that source is successfully rescanned", async () => {
  const failed = {
    ...inventory,
    ticketId: "failed-ticket",
    sources: inventory.sources.map((source) => ({
      ...source,
      state: "readFailed",
    })),
  };
  vi.mocked(libraryApi.inventory)
    .mockResolvedValueOnce(inventory)
    .mockResolvedValueOnce(failed)
    .mockResolvedValue({ ...inventory, ticketId: "manual-fresh" });
  vi.mocked(libraryApi.inventorySource).mockRejectedValueOnce(
    new Error("library.inventoryChanged"),
  );
  renderSection();
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  await userEvent.click(
    within(native).getByRole("button", { name: /定位到资源库/ }),
  );
  await waitFor(() => expect(libraryApi.inventory).toHaveBeenCalledTimes(2));
  expect(await screen.findByText("来源待确认，请重新扫描。")).toBeVisible();
  await userEvent.click(
    within(native).getByRole("button", { name: "重新扫描" }),
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  await userEvent.click(
    within(native).getByRole("button", {
      name: "查看 codegraph 的 Codex 来源",
    }),
  );
  const dialog = await screen.findByRole("dialog", { name: "来源详情" });
  await userEvent.click(
    await within(dialog).findByRole("button", { name: /定位到资源库/ }),
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  fireEvent.click(within(native).getByRole("button", { name: /定位到资源库/ }));
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
  expect(screen.queryByTestId("library-resource-a")).toBeNull();
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
  const native = await screen.findByRole("region", { name: "本机配置中" });
  await userEvent.click(within(native).getByRole("button", { name: /已收录/ }));
  fireEvent.click(within(native).getByRole("button", { name: /定位到资源库/ }));
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
