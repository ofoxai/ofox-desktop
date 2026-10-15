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
  client.setQueryData(["sessions"], sessions);
  vi.spyOn(sessionsApi, "list").mockResolvedValue(sessions);
  render(
    <QueryClientProvider client={client}>
      <ShellProvider openSettings={() => undefined}>
        <RefreshControl />
        <SessionsPage className="h-full" />
      </ShellProvider>
    </QueryClientProvider>,
  );
  return client;
}

beforeEach(() => {
  vi.restoreAllMocks();
  localStorage.clear();
  HTMLElement.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  vi.spyOn(sessionsApi, "resume").mockResolvedValue({
    command: "safe command",
    launched: false,
  });
  vi.spyOn(sessionsApi, "getMessagesByKey").mockResolvedValue([]);
  vi.spyOn(sessionsApi, "listTrash").mockResolvedValue([]);
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
          .getQueryData<ManagedSession[]>(["sessions"])
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
    fireEvent.click(await screen.findByRole("button", { name: "继续对话" }));
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
  expect(screen.getAllByText("—").length).toBeGreaterThan(0);
  expect(screen.getByText(/33 条消息.*11 MB/)).toBeVisible();
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  expect(sessionsApi.getMessagesByKey).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  await waitFor(() =>
    expect(sessionsApi.getMessagesByKey).toHaveBeenCalledWith(a.sessionKey),
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
  fireEvent.click(screen.getByRole("button", { name: "继续对话" }));
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
  vi.mocked(sessionsApi.getMessagesByKey)
    .mockResolvedValueOnce([{ role: "user", content: "old body" }])
    .mockResolvedValue([{ role: "user", content: "new body" }]);
  const client = renderPanel([a]);
  fireEvent.click(screen.getByRole("button", { name: /title-a/ }));
  fireEvent.click(screen.getByRole("button", { name: "查看会话" }));
  expect(await screen.findByText("old body")).toBeVisible();
  client.setQueryData(["sessions"], [{ ...a, revision: "changed" }]);
  expect(await screen.findByText("new body")).toBeVisible();
  expect(sessionsApi.getMessagesByKey).toHaveBeenCalledTimes(2);
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
