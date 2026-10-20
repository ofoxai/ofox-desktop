import { useEffect } from "react";
import userEvent from "@testing-library/user-event";
import { emitTauriEvent } from "../msw/tauriMocks";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
  act,
} from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import {
  SessionsPage,
  groupSessions,
} from "@/components/sessions/SessionsPage";
import { sessionsApi, type ManagedSession } from "@/lib/api/sessions";
import * as clipboard from "@/lib/clipboard";
import * as platform from "@/lib/platform";

const apexState = vi.hoisted(() => ({ apex: "ofox.ai", isLoading: false }));
vi.mock("@/hooks/useOfoxApex", () => ({ useOfoxApex: () => apexState }));
let rerenderPanel = () => {};
const toastError = vi.hoisted(() => vi.fn());
const saveDialog = vi.hoisted(() => vi.fn());
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: toastError } }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: saveDialog }));
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getTotalSize: () => count * 70,
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({
        index,
        key: index,
        start: index * 70,
      })),
    measureElement: () => undefined,
    scrollToIndex: () => undefined,
  }),
}));
vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({
    isOpen,
    onConfirm,
    onCancel,
  }: {
    isOpen: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }) =>
    isOpen ? (
      <div data-testid="confirm">
        <button onClick={onConfirm}>confirm</button>
        <button onClick={onCancel}>cancel</button>
      </div>
    ) : null,
}));

function session(
  id: string,
  providerId = "claude",
  canTrash = true,
): ManagedSession {
  return {
    sessionId: id,
    sessionKey: id.repeat(64).slice(0, 64),
    providerId,
    title: `title-${id}`,
    projectDir: "/project",
    sourcePath: `/sessions/${id}.jsonl`,
    revision: "revision",
    capabilities: {
      canReadMessages: providerId !== "dsh",
      canExport: providerId !== "dsh",
      canResume: providerId === "claude",
      canTrash,
      resumeUsesSessionModel: providerId === "pi",
    },
  };
}
function RefreshControl() {
  const shell = useShell();
  useEffect(() => shell.setActive("sessions"), [shell.setActive]);
  return (
    <>
      <button onClick={() => void shell.refresh()}>refresh-page</button>
      <button onClick={() => shell.locateSession?.("analytics-target")}>
        locate-analysis-session
      </button>
    </>
  );
}
function renderPanel(sessions: ManagedSession[]) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(["sessions", "ofox.ai"], sessions);
  vi.spyOn(sessionsApi, "list").mockResolvedValue(sessions);
  const element = () => (
    <QueryClientProvider client={client}>
      <ShellProvider openSettings={() => undefined}>
        <RefreshControl />
        <SessionsPage className="h-full" />
      </ShellProvider>
    </QueryClientProvider>
  );
  const view = render(element());
  rerenderPanel = () => view.rerender(element());
  return client;
}

beforeEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
  apexState.apex = "ofox.ai";
  apexState.isLoading = false;
  HTMLElement.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle(
    "zh",
    "translation",
    { sessionExperience: zh.sessionExperience },
    true,
    true,
  );
  vi.spyOn(sessionsApi, "resume").mockResolvedValue({
    command: "safe command",
    launched: false,
  });
  vi.spyOn(sessionsApi, "getMessagesByKey").mockResolvedValue([]);
  vi.spyOn(sessionsApi, "getTranscript").mockResolvedValue({
    readRevision: "verified",
    messages: [],
  });
  vi.spyOn(sessionsApi, "listTrash").mockResolvedValue([]);
  vi.spyOn(sessionsApi, "cancelSearch").mockResolvedValue(true);
  vi.spyOn(clipboard, "copyText").mockResolvedValue();
});

describe("SessionsPage capabilities and reversible actions", () => {
  it("keeps dsh metadata-only and shared-index providers out of trash", async () => {
    renderPanel([session("d", "dsh", false)]);
    expect(
      await screen.findByText("此工具目前仅支持会话元数据。"),
    ).toBeInTheDocument();
    expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("button", { name: "导出 Markdown" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "复制续聊命令" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("checkbox", { name: "选择会话 title-d" }),
    ).toBeDisabled();
  });

  it("keeps capability notes collapsed and scoped to the affected tools", () => {
    renderPanel([session("a"), session("d", "dsh", false)]);
    expect(screen.getByText("此工具目前仅支持会话元数据。")).not.toBeVisible();
    const summary = screen.getByText("工具说明");
    expect(summary).toBeVisible();
    fireEvent.click(summary);
    const notes = summary.closest("details")!;
    expect(within(notes).getByText("DeepSeek Harness")).toBeVisible();
    expect(within(notes).queryByText("Claude Code")).toBeNull();
    expect(
      within(notes).getByText("此工具目前仅支持会话元数据。"),
    ).toBeVisible();
    expect(
      within(notes).getByText("此来源暂不支持安全文件回收。"),
    ).toBeVisible();
    fireEvent.click(summary);
    expect(
      within(notes).getByText("此工具目前仅支持会话元数据。"),
    ).not.toBeVisible();
  });

  it("groups full project paths and filters tool metadata without reading bodies", async () => {
    const a = session("a");
    const b = { ...session("b", "pi"), projectDir: "/other/project" };
    expect(groupSessions([a, b]).size).toBe(2);
    renderPanel([a, b]);
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "pi" },
    });
    await waitFor(() =>
      expect(screen.queryByText("title-a")).not.toBeInTheDocument(),
    );
    expect(screen.getAllByText("title-b").length).toBeGreaterThan(0);
    expect(
      screen.getByText("Pi 续聊使用会话内记录的模型；新默认模型仅影响新会话。"),
    ).toBeInTheDocument();
  });

  it("uses backend session keys and revisions and reports partial trash failures", async () => {
    const a = session("a");
    const b = session("b");
    const trash = vi.spyOn(sessionsApi, "trash").mockResolvedValue([
      { sessionKey: a.sessionKey, success: true, trashKey: "trash-a" },
      { sessionKey: b.sessionKey, success: false, error: "Session active" },
    ]);
    const client = renderPanel([a, b]);
    vi.mocked(sessionsApi.list).mockResolvedValue([b]);
    fireEvent.click(
      screen.getByRole("checkbox", { name: "选择 /project 中的全部会话" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "移至回收站（2）" }));
    fireEvent.click(within(screen.getByTestId("confirm")).getByText("confirm"));
    await waitFor(() =>
      expect(trash).toHaveBeenCalledWith([
        { sessionKey: a.sessionKey, revision: "revision" },
        { sessionKey: b.sessionKey, revision: "revision" },
      ]),
    );
    await waitFor(() =>
      expect(
        client
          .getQueryData<ManagedSession[]>(["sessions", "ofox.ai"])
          ?.map((s) => s.sessionKey),
      ).toEqual([b.sessionKey]),
    );
    expect(toastError).toHaveBeenCalledWith("Session active");
  });

  it("marks partially selected projects and drops hidden selections", async () => {
    renderPanel([session("a"), session("b", "pi")]);
    fireEvent.click(screen.getByRole("checkbox", { name: "选择会话 title-a" }));
    const project = screen.getByRole("checkbox", {
      name: "选择 /project 中的全部会话",
    }) as HTMLInputElement;
    expect(project).toHaveAttribute("aria-checked", "mixed");
    fireEvent.click(screen.getByRole("checkbox", { name: "选择会话 title-b" }));
    expect(project).toBeChecked();
    await userEvent.click(screen.getByRole("combobox", { name: "工具" }));
    await userEvent.click(screen.getByRole("option", { name: /Claude/ }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "移至回收站（1）" }),
      ).toBeEnabled(),
    );
  });

  it("copies a backend-generated command when macOS terminal launch fails", async () => {
    vi.spyOn(platform, "isMac").mockReturnValue(true);
    const writeText = vi
      .spyOn(clipboard, "copyText")
      .mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const a = { ...session("a"), resumeCommand: "unsafe legacy metadata" };
    const resume = vi
      .spyOn(sessionsApi, "resume")
      .mockRejectedValueOnce("Terminal unavailable")
      .mockResolvedValueOnce({
        command: "backend literal command",
        launched: false,
      });
    renderPanel([a]);
    fireEvent.click(await screen.findByRole("button", { name: "在终端继续" }));
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith("backend literal command"),
    );
    expect(resume).toHaveBeenNthCalledWith(1, a.sessionKey, true);
    expect(resume).toHaveBeenNthCalledWith(2, a.sessionKey, false);
  });

  it("cancels exports before any file write and asks backend for resume", async () => {
    const a = session("a");
    const exportMarkdown = vi
      .spyOn(sessionsApi, "exportMarkdown")
      .mockResolvedValue({ filename: "session.md", content: "full body" });
    saveDialog.mockResolvedValue(null);
    renderPanel([a]);
    fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
    fireEvent.click(
      await screen.findByRole("button", { name: "导出 Markdown" }),
    );
    await waitFor(() => expect(saveDialog).toHaveBeenCalled());
    expect(exportMarkdown).toHaveBeenCalledTimes(1);
    expect(exportMarkdown).toHaveBeenCalledWith(a.sessionKey);
  });

  it("shows restore occupancy failures and requires confirmation before purge", async () => {
    const entry = {
      key: "trash",
      providerId: "claude",
      sessionId: "a",
      title: "deleted",
      deletedAt: 1,
      state: "trashed" as const,
      itemCount: 4,
    };
    vi.mocked(sessionsApi.listTrash).mockResolvedValue([entry]);
    const restore = vi
      .spyOn(sessionsApi, "restore")
      .mockRejectedValue("Restore destination is occupied");
    const purge = vi.spyOn(sessionsApi, "purge").mockResolvedValue();
    renderPanel([]);
    fireEvent.click(screen.getByRole("button", { name: /^回收站/ }));
    fireEvent.click(await screen.findByRole("button", { name: "恢复" }));
    await waitFor(() => expect(restore).toHaveBeenCalledWith("trash"));
    await waitFor(() =>
      expect(toastError).toHaveBeenCalledWith(
        "Restore destination is occupied",
      ),
    );
    fireEvent.click(screen.getByRole("button", { name: "永久清除" }));
    expect(purge).not.toHaveBeenCalled();
    fireEvent.click(within(screen.getByTestId("confirm")).getByText("confirm"));
    await waitFor(() => expect(purge).toHaveBeenCalledWith("trash"));
  });
});

it("shows fresh tokens, separate cache, readable costs, and lazy inline bodies", async () => {
  const a = {
    ...session("a"),
    models: ["model-a"],
    messageCount: 33,
    sizeBytes: 11000000,
    usage: {
      models: [],
      inputTokens: 1000,
      outputTokens: 200,
      cacheReadTokens: 5000,
      cacheWriteTokens: 50,
      totalTokens: 1200,
      costUsd: 0.02,
      unpricedModels: 0,
    },
  };
  renderPanel([a, session("b")]);
  expect(screen.getByText("1.2K")).toBeVisible();
  expect(screen.getByText("≈$0.020")).toBeVisible();
  expect(screen.getByLabelText("用量不可用")).toHaveTextContent(/^-$/);
  expect(screen.getByText(/33 条消息 · 11 MB/)).toBeVisible();
  expect(screen.getByText("输入 1.0K · 输出 200 · 缓存 5.0K")).toBeVisible();
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  await waitFor(() =>
    expect(sessionsApi.getTranscript).toHaveBeenCalledWith(
      a.sessionKey,
      undefined,
    ),
  );
});

it("opens only the newest folder and allows revealing older folders", async () => {
  renderPanel([
    { ...session("a"), lastActiveAt: 20 },
    { ...session("b"), projectDir: "/older", lastActiveAt: 10 },
  ]);
  expect(screen.getByText("title-a")).toBeVisible();
  expect(screen.queryByText("title-b")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /older/ }));
  expect(screen.getByText("title-b")).toBeVisible();
});

it("refreshes the index and updates metadata on usage events", async () => {
  const updated = { ...session("a"), title: "updated" };
  const refresh = vi
    .spyOn(sessionsApi, "refreshIndex")
    .mockResolvedValue([updated]);
  renderPanel([session("a")]);
  fireEvent.click(screen.getByText("refresh-page"));
  await waitFor(() => expect(refresh).toHaveBeenCalledTimes(1));
  expect(await screen.findByText("updated")).toBeVisible();
  vi.mocked(sessionsApi.list).mockResolvedValue([
    { ...updated, title: "event-title" },
  ]);
  emitTauriEvent("usage-synced", {});
  expect(await screen.findByText("event-title")).toBeVisible();
});
it("launches safely, exports to the chosen destination, and searches model names", async () => {
  const row = {
    ...session("a"),
    usage: {
      models: [
        {
          model: "special-model",
          requests: 1,
          inputTokens: 1,
          outputTokens: 1,
          cacheReadTokens: 0,
          cacheWriteTokens: 0,
          costUsd: 0.1,
        },
      ],
      inputTokens: 1,
      outputTokens: 1,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      totalTokens: 2,
      costUsd: 0.1,
      unpricedModels: 0,
    },
  };
  vi.mocked(sessionsApi.resume).mockResolvedValue({
    command: "safe command",
    launched: true,
  });
  const exp = vi
    .spyOn(sessionsApi, "exportMarkdown")
    .mockResolvedValue({ filename: "a.md", content: "content" });
  saveDialog.mockResolvedValue("/sandbox/a.md");
  renderPanel([row, session("b")]);
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "special-model" },
  });
  await waitFor(() => expect(screen.queryByText("title-b")).toBeNull());
  fireEvent.click(screen.getByRole("button", { name: "在终端继续" }));
  await waitFor(() =>
    expect(sessionsApi.resume).toHaveBeenCalledWith(row.sessionKey, true),
  );
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "导出 Markdown" }));
  await waitFor(() =>
    expect(exp).toHaveBeenCalledWith(row.sessionKey, "/sandbox/a.md"),
  );
});
it("restores sessions and confirms clearing every trash entry", async () => {
  const entries = ["one", "two"].map((key) => ({
    key,
    providerId: "claude",
    sessionId: key,
    deletedAt: Date.now(),
    state: "trashed" as const,
    itemCount: 1,
  }));
  vi.mocked(sessionsApi.listTrash).mockResolvedValue(entries);
  const restore = vi
    .spyOn(sessionsApi, "restore")
    .mockResolvedValue(entries[0]);
  const purge = vi.spyOn(sessionsApi, "purge").mockResolvedValue();
  renderPanel([]);
  fireEvent.click(screen.getByRole("button", { name: /^回收站/ }));
  fireEvent.click((await screen.findAllByRole("button", { name: "恢复" }))[0]);
  await waitFor(() => expect(restore).toHaveBeenCalledWith("one"));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "清空回收站" })).toBeEnabled(),
  );
  fireEvent.click(screen.getByRole("button", { name: "清空回收站" }));
  expect(purge).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText("confirm"));
  await waitFor(() => expect(purge).toHaveBeenCalledTimes(2));
});

it("refreshes an open transcript when its revision changes", async () => {
  const a = session("a");
  vi.mocked(sessionsApi.getTranscript)
    .mockResolvedValueOnce({
      readRevision: "old",
      messages: [{ role: "user", content: "old body" }],
    })
    .mockResolvedValue({
      readRevision: "new",
      messages: [{ role: "user", content: "new body" }],
    });
  const client = renderPanel([a]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  expect(await screen.findByText("old body")).toBeVisible();
  client.setQueryData(["sessions", "ofox.ai"], [{ ...a, revision: "changed" }]);
  expect(await screen.findByText("new body")).toBeVisible();
  expect(sessionsApi.getTranscript).toHaveBeenCalledTimes(2);
});

it("locates a stable analysis key beyond the first hundred without reading or resuming it", async () => {
  localStorage.setItem("ofox.sessionsAgent", "pi");
  const target = {
    ...session("target", "claude"),
    sessionKey: "analytics-target",
    projectDir: "/older",
    lastActiveAt: 1,
  };
  const rows = Array.from({ length: 102 }, (_, i) => ({
    ...session(`item-${i}`, "claude"),
    projectDir: "/older",
    lastActiveAt: 200 - i,
  }));
  renderPanel([
    { ...session("visible", "pi"), projectDir: "/active", lastActiveAt: 1000 },
    ...rows,
    target,
  ]);
  expect(screen.queryByText("title-target")).toBeNull();
  await userEvent.click(screen.getByText("locate-analysis-session"));
  const button = await screen.findByRole("button", { name: /title-target/ });
  await waitFor(() => expect(button).toHaveFocus());
  expect(button).toHaveAttribute("aria-expanded", "false");
  expect(HTMLElement.prototype.scrollIntoView).toHaveBeenCalled();
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  expect(sessionsApi.resume).not.toHaveBeenCalled();
});

it("opens an independent reader and returns to the unchanged list without eager content reads", async () => {
  vi.mocked(sessionsApi.getTranscript).mockResolvedValue({
    readRevision: "verified",
    messages: [{ role: "user", content: "A readable question" }],
  });
  renderPanel([session("a"), session("b")]);
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "title-a" },
  });
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  expect(
    await screen.findByRole("button", { name: "返回会话列表" }),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: /title-b/ })).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "返回会话列表" }));
  expect(screen.getByRole("searchbox")).toHaveValue("title-a");
  expect(screen.getByRole("button", { name: /title-a/ })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
});

it("requires an explicit content-search action and freezes backend session keys", async () => {
  const row = session("a");
  const prepare = vi
    .spyOn(sessionsApi, "prepareSearch")
    .mockImplementation(async ({ requestId }) => ({
      operationId: "operation",
      requestId,
      candidateCount: 1,
    }));
  const runSearch = vi
    .spyOn(sessionsApi, "runSearch")
    .mockImplementation(async () => ({
      operationId: "operation",
      requestId: prepare.mock.calls[0][0].requestId,
      state: "complete",
      hits: [
        {
          sessionKey: row.sessionKey,
          readRevision: "read-revision",
          messageOrdinal: 0,
          contentHash: "checksum",
          role: "user",
          snippet: "matching question",
        },
      ],
      scanned: 1,
      total: 1,
      changed: 0,
      failed: 0,
      unsupported: 0,
      limitReasons: [],
    }));
  renderPanel([row]);
  fireEvent.click(screen.getByRole("radio", { name: "聊天内容" }));
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "matching" },
  });
  expect(prepare).not.toHaveBeenCalled();
  fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Enter" });
  await waitFor(() =>
    expect(runSearch).toHaveBeenCalledWith("operation", undefined),
  );
  expect(
    await screen.findByText(
      (_, element) =>
        element?.tagName === "SPAN" &&
        element.textContent === "matching question",
    ),
  ).toBeVisible();
  expect(prepare).toHaveBeenCalledWith({
    requestId: expect.any(String),
    query: "matching",
    sessionKeys: [row.sessionKey],
    includeProcess: false,
  });
  expect(runSearch).toHaveBeenCalledWith("operation", undefined);
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
});

it("opens a search hit only through backend revision and message checksum validation", async () => {
  const row = session("a");
  const hit = {
    sessionKey: row.sessionKey,
    readRevision: "searched-revision",
    messageOrdinal: 0,
    contentHash: "checksum",
    role: "user" as const,
    snippet: "matching question",
  };
  const prepare = vi
    .spyOn(sessionsApi, "prepareSearch")
    .mockImplementation(async ({ requestId }) => ({
      operationId: "operation",
      requestId,
      candidateCount: 1,
    }));
  vi.spyOn(sessionsApi, "runSearch").mockImplementation(async () => ({
    operationId: "operation",
    requestId: prepare.mock.calls[0][0].requestId,
    state: "complete",
    hits: [hit],
    scanned: 1,
    total: 1,
    changed: 0,
    failed: 0,
    unsupported: 0,
    limitReasons: [],
  }));
  const read = vi
    .spyOn(sessionsApi, "getTranscript")
    .mockRejectedValue("session.search.changed");
  renderPanel([row]);
  fireEvent.click(screen.getByRole("radio", { name: "聊天内容" }));
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "matching" },
  });
  fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Enter" });
  fireEvent.click(
    await screen.findByRole("button", { name: /title-a.*matching question/ }),
  );
  expect(
    await screen.findByText("会话内容已变化，请重新搜索后定位。"),
  ).toBeVisible();
  expect(read).toHaveBeenCalledWith(row.sessionKey, hit);
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  expect(
    within(screen.getByRole("region", { name: "会话阅读" })).queryByText(
      "matching question",
    ),
  ).toBeNull();
});

it("navigates user questions and local matches inside a virtualized reader", async () => {
  vi.mocked(sessionsApi.getTranscript).mockResolvedValue({
    readRevision: "verified",
    messages: [
      { role: "user", content: "First question" },
      { role: "assistant", content: "Useful answer" },
      { role: "tool", content: "Tool evidence", name: "read" },
      { role: "user", content: "Follow-up question" },
    ],
  });
  renderPanel([session("a")]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  await screen.findByText("First question");
  fireEvent.click(screen.getByRole("button", { name: /提问目录/ }));
  fireEvent.click(screen.getByRole("button", { name: "第 2 条提问" }));
  await waitFor(() =>
    expect(document.querySelector('[data-message-ordinal="3"]')).toHaveFocus(),
  );
  fireEvent.change(screen.getByRole("searchbox", { name: "在本会话中查找" }), {
    target: { value: "Tool evidence" },
  });
  expect(screen.getByText("1 / 1 条匹配消息")).toBeVisible();
  expect(document.querySelector('[data-message-ordinal="2"]')).toHaveClass(
    "ring-brand/40",
  );
  expect(
    document.querySelector('[data-message-ordinal="2"] details'),
  ).toHaveAttribute("open");
});

it("never leaves stale search-hit content visible when a recheck reports a changed source", async () => {
  const row = session("a");
  const hit = {
    sessionKey: row.sessionKey,
    readRevision: "searched-revision",
    messageOrdinal: 0,
    contentHash: "checksum",
    role: "user" as const,
    snippet: "matching question",
  };
  const prepare = vi
    .spyOn(sessionsApi, "prepareSearch")
    .mockImplementation(async ({ requestId }) => ({
      operationId: "operation",
      requestId,
      candidateCount: 1,
    }));
  vi.spyOn(sessionsApi, "runSearch").mockImplementation(async () => ({
    operationId: "operation",
    requestId: prepare.mock.calls[0][0].requestId,
    state: "complete",
    hits: [hit],
    scanned: 1,
    total: 1,
    changed: 0,
    failed: 0,
    unsupported: 0,
    limitReasons: [],
  }));
  const read = vi
    .spyOn(sessionsApi, "getTranscript")
    .mockResolvedValueOnce({
      readRevision: "searched-revision",
      messages: [{ role: "user", content: "Old accepted content" }],
    })
    .mockRejectedValue("session.search.changed");
  const client = renderPanel([row]);
  fireEvent.click(screen.getByRole("radio", { name: "聊天内容" }));
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "matching" },
  });
  fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Enter" });
  fireEvent.click(
    await screen.findByRole("button", { name: /title-a.*matching question/ }),
  );
  expect(await screen.findByText("Old accepted content")).toBeVisible();
  await act(async () =>
    client.invalidateQueries({ queryKey: ["session-messages"] }),
  );
  await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(
    await screen.findByText("会话内容已变化，请重新搜索后定位。"),
  ).toBeVisible();
  expect(screen.queryByText("Old accepted content")).toBeNull();
});

it("keeps focus in the local find field while typing instead of moving it to a result", async () => {
  vi.mocked(sessionsApi.getTranscript).mockResolvedValue({
    readRevision: "verified",
    messages: [{ role: "user", content: "A matching answer" }],
  });
  renderPanel([session("a")]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  await screen.findByText("A matching answer");
  const find = screen.getByRole("searchbox", { name: "在本会话中查找" });
  find.focus();
  fireEvent.change(find, { target: { value: "answer" } });
  await act(
    async () =>
      new Promise((resolve) =>
        window.requestAnimationFrame(() => resolve(undefined)),
      ),
  );
  expect(find).toHaveFocus();
});

it("offers continue and export inside the reader without returning to the list", async () => {
  const row = session("a");
  const exp = vi
    .spyOn(sessionsApi, "exportMarkdown")
    .mockResolvedValue({ filename: "a.md", content: "body" });
  saveDialog.mockResolvedValue(null);
  renderPanel([row]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  const reader = screen.getByRole("region", { name: "会话阅读" });
  fireEvent.click(within(reader).getByRole("button", { name: "在终端继续" }));
  await waitFor(() =>
    expect(sessionsApi.resume).toHaveBeenCalledWith(row.sessionKey, true),
  );
  fireEvent.click(
    within(reader).getByRole("button", { name: "导出 Markdown" }),
  );
  await waitFor(() => expect(exp).toHaveBeenCalledWith(row.sessionKey));
  expect(exp).toHaveBeenCalledOnce();
});

it("labels partial session pricing without presenting the known amount as a complete total", () => {
  const model = {
    model: "known",
    requests: 1,
    inputTokens: 1,
    outputTokens: 1,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    costUsd: 0.02,
  };
  renderPanel([
    {
      ...session("a"),
      usage: {
        models: [model, { ...model, model: "unknown", costUsd: null }],
        inputTokens: 2,
        outputTokens: 2,
        cacheReadTokens: 0,
        cacheWriteTokens: 0,
        totalTokens: 4,
        costUsd: 0.02,
        unpricedModels: 1,
      },
    },
  ]);
  expect(screen.getByText("≈$0.020+")).toBeVisible();
});

it("keeps a copyable command visible when both terminal launch and clipboard fallback fail", async () => {
  vi.mocked(sessionsApi.resume)
    .mockRejectedValueOnce("terminal unavailable")
    .mockResolvedValue({ command: "copy-this-command", launched: false });
  vi.mocked(clipboard.copyText)
    .mockRejectedValueOnce("clipboard unavailable")
    .mockResolvedValue();
  renderPanel([session("a")]);
  fireEvent.click(screen.getByRole("button", { name: "在终端继续" }));
  expect(
    await screen.findByText("未能启动终端，复制也失败；请重试复制下方命令。"),
  ).toBeVisible();
  expect(screen.getByText("copy-this-command")).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "复制继续命令" }));
  expect(clipboard.copyText).toHaveBeenCalledTimes(2);
});

it("opens ordinary sessions through the same bounded verified reader as search hits", async () => {
  const row = session("a");
  const read = vi.spyOn(sessionsApi, "getTranscript").mockResolvedValue({
    readRevision: "verified",
    messages: [{ role: "user", content: "Verified ordinary content" }],
  });
  vi.mocked(sessionsApi.getMessagesByKey).mockResolvedValue([
    { role: "user", content: "Legacy unbounded content" },
  ]);
  renderPanel([row]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  expect(await screen.findByText("Verified ordinary content")).toBeVisible();
  expect(read).toHaveBeenCalledWith(row.sessionKey, undefined);
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  expect(screen.queryByText("Legacy unbounded content")).toBeNull();
});

it("reports a missing verified-reader command without falling back to an unbounded read", async () => {
  vi.spyOn(sessionsApi, "getTranscript").mockRejectedValue(
    "Command get_session_transcript not found",
  );
  renderPanel([session("a")]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  expect(
    await screen.findByText("当前版本不支持安全会话阅读，请更新应用后重试。"),
  ).toBeVisible();
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
});

it("verifies every explicit ordinary open and hides the previous body while the new read is pending", async () => {
  let finish!: (value: {
    readRevision: string;
    messages: { role: "user"; content: string }[];
  }) => void;
  const read = vi
    .spyOn(sessionsApi, "getTranscript")
    .mockResolvedValueOnce({
      readRevision: "first",
      messages: [{ role: "user", content: "Previously verified body" }],
    })
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    );
  renderPanel([session("a")]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  expect(await screen.findByText("Previously verified body")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "返回会话列表" }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  await waitFor(() => expect(read).toHaveBeenCalledTimes(2));
  expect(screen.queryByText("Previously verified body")).toBeNull();
  await act(async () =>
    finish({
      readRevision: "second",
      messages: [{ role: "user", content: "Freshly verified body" }],
    }),
  );
  expect(await screen.findByText("Freshly verified body")).toBeVisible();
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
});

it("explains a database read budget as partial coverage without claiming that scanning can continue", async () => {
  const prepare = vi
    .spyOn(sessionsApi, "prepareSearch")
    .mockImplementation(async ({ requestId }) => ({
      operationId: "database-budget",
      requestId,
      candidateCount: 1,
    }));
  vi.spyOn(sessionsApi, "runSearch").mockImplementation(async () => ({
    operationId: "database-budget",
    requestId: prepare.mock.calls[0][0].requestId,
    state: "limited",
    hits: [],
    scanned: 1,
    total: 1,
    changed: 0,
    failed: 1,
    unsupported: 0,
    limitReasons: ["databaseLimit"],
  }));
  renderPanel([session("a")]);
  fireEvent.click(screen.getByRole("radio", { name: "聊天内容" }));
  fireEvent.change(screen.getByRole("searchbox"), {
    target: { value: "matching" },
  });
  fireEvent.keyDown(screen.getByRole("searchbox"), { key: "Enter" });
  expect(
    await screen.findByText(zh.sessionExperience.limitDatabase),
  ).toBeVisible();
  expect(screen.getByText("当前是部分结果，不代表全部历史。")).toBeVisible();
  expect(screen.queryByRole("button", { name: "继续扫描" })).toBeNull();
});

const regionSession = (apex: string, value: number): ManagedSession => ({
  ...session("region"),
  providerId: "codex",
  usage: {
    models: [],
    inputTokens: 10,
    outputTokens: 2,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
    totalTokens: 12,
    costUsd: value,
    unpricedModels: 0,
  },
  usageDisplay: {
    state: "available",
    basis: "currentOfoxEstimate",
    scope: "nativeThread",
    reasons: [],
    parentSessionId: null,
    billingSessionId: null,
    estimate: {
      knownCostUsd: String(value),
      availability: "known",
      unpricedModelIds: [],
      unpricedRecordCount: 0,
      additionalUnitsUnsupported: false,
    },
    pricingCoverage: {
      policy: "current-ofox-base-v1",
      apex: apex as "ofox.ai" | "ofox.io",
      catalogVersion: "fixture",
      catalogUpdatedAtMs: 1,
      stale: false,
      unpricedModelIds: [],
      unpricedRecordCount: 0,
      additionalUnitsUnsupported: false,
    },
  },
});

it("isolates session estimates by apex and refuses late replies from the previous region", async () => {
  const ai = regionSession("ofox.ai", 1),
    io = regionSession("ofox.io", 2);
  const client = renderPanel([ai]);
  expect(screen.getByText("≈$1.00")).toBeVisible();
  let resolveOld!: (rows: ManagedSession[]) => void;
  vi.mocked(sessionsApi.list)
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveOld = resolve;
        }),
    )
    .mockResolvedValue([io]);
  void client.invalidateQueries({ queryKey: ["sessions"] });
  await waitFor(() => expect(resolveOld).toBeTypeOf("function"));
  act(() => {
    apexState.apex = "ofox.io";
    rerenderPanel();
  });
  expect(await screen.findByText("≈$2.00")).toBeVisible();
  await act(async () => {
    resolveOld([regionSession("ofox.ai", 8)]);
  });
  expect(screen.queryByText("≈$8.00")).toBeNull();
  expect(screen.getByText("≈$2.00")).toBeVisible();
});

it("rejects a listing price receipt from the wrong apex even on the current query", async () => {
  const old = regionSession("ofox.ai", 9);
  apexState.apex = "ofox.io";
  renderPanel([old]);
  expect(await screen.findByText("title-region")).toBeVisible();
  expect(screen.getByLabelText("用量不可用")).toHaveTextContent(/^-$/);
  expect(screen.queryByText("≈$9.00")).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: /title-region/ }));
  expect(
    screen.getByText("价格区域正在更新，请稍后刷新会话用量。"),
  ).toBeVisible();
});

describe("subagent threads", () => {
  const thread = (id: string, parentSessionId?: string, createdAt = 0) => ({
    ...session(id, "codex"),
    parentSessionId,
    createdAt,
  });
  const family = () => [
    thread("p"),
    thread("c", "p", 2),
    thread("b", "p", 1),
    thread("g", "c", 3),
  ];

  it("folds a parent's threads under it, nested and counted in the folder", () => {
    renderPanel(family());
    const folder = screen.getByText("/project").parentElement!;
    expect(within(folder).getByText("4")).toBeVisible();
    expect(screen.queryByText("title-b")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "2 个子代理" }));
    const titles = screen
      .getAllByText(/^title-[bc]$/)
      .map((node) => node.textContent);
    expect(titles).toEqual(["title-b", "title-c"]);
    expect(
      screen.getByRole("checkbox", { name: "选择会话 title-c" }),
    ).toBeEnabled();
    expect(screen.queryByText("title-g")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "1 个子代理" }));
    expect(screen.getByText("title-g")).toBeVisible();
  });

  it("opens the folds while searching, and the folder box selects nested threads", async () => {
    renderPanel(family());
    fireEvent.change(screen.getByRole("searchbox"), {
      target: { value: "title" },
    });
    expect(await screen.findByText("title-g")).toBeVisible();
    fireEvent.click(
      screen.getByRole("checkbox", { name: "选择 /project 中的全部会话" }),
    );
    expect(screen.getByText("已选 4 个")).toBeVisible();
  });

  it("unfolds every session a located thread is under", async () => {
    const target = {
      ...thread("target", "c", 4),
      sessionKey: "analytics-target",
    };
    renderPanel([...family(), target]);
    expect(screen.queryByText("title-target")).toBeNull();
    await userEvent.click(screen.getByText("locate-analysis-session"));
    const button = await screen.findByRole("button", { name: /title-target/ });
    await waitFor(() => expect(button).toHaveFocus());
    expect(screen.getByText("title-c")).toBeVisible();
    expect(screen.queryByText("title-g")).toBeVisible();
  });

  it("lists a Claude session's subagents in its fold and opens one's conversation", async () => {
    const read = vi
      .spyOn(sessionsApi, "getSubagentTranscript")
      .mockResolvedValue({
        readRevision: "verified",
        messages: [{ role: "user", content: "Look for the bug" }],
      });
    const parent = {
      ...session("p"),
      subagents: [
        { id: "a1", title: "Find the bug", kind: "Explore", startedAt: 1 },
        { id: "a3", title: "Go deeper", kind: "general", parent: "a1" },
        { id: "a2", kind: "Plan", startedAt: 2 },
      ],
    };
    renderPanel([parent]);
    expect(screen.queryByText("Find the bug")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "3 个子代理" }));
    const rows = screen
      .getAllByRole("button", { name: /Find the bug|Go deeper|Plan/ })
      .map((row) => row.textContent);
    expect(rows[0]).toMatch(/^Find the bug.*Explore/);
    expect(rows[1]).toMatch(/^Go deeper/);
    expect(rows[2]).toMatch(/^Plan/);
    expect(screen.queryByRole("checkbox", { name: /Find the bug/ })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Find the bug/ }));
    expect(await screen.findByText("Look for the bug")).toBeVisible();
    expect(read).toHaveBeenCalledWith(parent.sessionKey, "a1");
    expect(screen.getByRole("heading", { name: "Find the bug" })).toBeVisible();
    expect(sessionsApi.resume).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("button", { name: "在终端继续" }),
    ).not.toBeInTheDocument();
  });

  it("shows a thread whose parent is not listed on top", () => {
    renderPanel([thread("orphan", "gone")]);
    expect(screen.getByText("title-orphan")).toBeVisible();
    expect(screen.queryByRole("button", { name: /个子代理/ })).toBeNull();
  });
});
