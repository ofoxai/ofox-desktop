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
  SessionsPanel,
  groupSessions,
} from "@/components/sessions/SessionsPanel";
import { sessionsApi, type ManagedSession } from "@/lib/api/sessions";
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
function renderPanel(sessions: ManagedSession[]) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, staleTime: Infinity } },
  });
  client.setQueryData(["sessions"], sessions);
  vi.spyOn(sessionsApi, "list").mockResolvedValue(sessions);
  render(
    <QueryClientProvider client={client}>
      <SessionsPanel className="h-full" />
    </QueryClientProvider>,
  );
  return client;
}

beforeEach(() => {
  vi.restoreAllMocks();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  vi.spyOn(sessionsApi, "getMessagesByKey").mockResolvedValue([]);
  vi.spyOn(sessionsApi, "listTrash").mockResolvedValue([]);
});

describe("SessionsPanel capabilities and reversible actions", () => {
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
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "pi" } });
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
      screen.getByRole("checkbox", { name: "选择项目 /project" }),
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
      name: "选择项目 /project",
    }) as HTMLInputElement;
    expect(project.indeterminate).toBe(true);
    fireEvent.click(screen.getByRole("checkbox", { name: "选择会话 title-b" }));
    expect(project).toBeChecked();
    fireEvent.change(screen.getByRole("combobox", { name: "工具" }), {
      target: { value: "claude" },
    });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "移至回收站（1）" }),
      ).toBeEnabled(),
    );
  });

  it("copies a backend-generated command when macOS terminal launch fails", async () => {
    vi.spyOn(platform, "isMac").mockReturnValue(true);
    const writeText = vi.fn().mockResolvedValue(undefined);
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
    fireEvent.click(screen.getByRole("button", { name: "回收站" }));
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
