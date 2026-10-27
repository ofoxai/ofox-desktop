import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { useAssistant } from "@/components/assistant/useAssistant";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import type {
  AssistantBootstrap,
  AssistantEvent,
  AssistantSession,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  backup: vi.fn(),
  previewRestore: vi.fn(),
  restore: vi.fn(),
  authStart: vi.fn(),
  authPoll: vi.fn(),
  authCancel: vi.fn(),
  listSessions: vi.fn(),
  createSession: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  send: vi.fn(),
  stop: vi.fn(),
  pickAttachments: vi.fn(),
  exportSession: vi.fn(),
  purgeSession: vi.fn(),
  saveMemory: vi.fn(),
  deleteMemory: vi.fn(),
  respondApproval: vi.fn(),
  subscribe: vi.fn(),
  listener: undefined as ((event: AssistantEvent) => void) | undefined,
}));
vi.mock("@/lib/api/assistant", () => ({ assistantApi: mocks }));
vi.mock("@/lib/api/ofoxAuth", () => ({
  ofoxStartLogin: mocks.authStart,
  ofoxPollForToken: mocks.authPoll,
  ofoxCancelLogin: mocks.authCancel,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(() => undefined),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));

const session: AssistantSession = {
  id: "session-a",
  kind: "general",
  title: "My conversation",
  modelId: "balanced",
  permissionMode: "ask",
  webSearch: false,
  createdAt: 1,
  updatedAt: 1,
};
const capabilities: AssistantBootstrap = {
  runtime: { available: true, version: "test" },
  models: [{ id: "balanced", name: "Balanced model" }],
  defaultModelId: "balanced",
  search: { available: true, currency: "USD", unitPrice: "0.01" },
};
let current: AssistantSessionDetail;

function page(enabled = true) {
  return (
    <ShellProvider openSettings={() => undefined}>
      <AssistantPage enabled={enabled} />
    </ShellProvider>
  );
}
async function openConversation() {
  render(page());
  await userEvent.click(
    await screen.findByRole("button", { name: /My conversation/ }),
  );
  await screen.findByRole("combobox", { name: "Choose model" });
}

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  current = { session: { ...session }, messages: [], memories: [] };
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.backup.mockResolvedValue({ path: null });
  mocks.previewRestore.mockResolvedValue(null);
  mocks.restore.mockResolvedValue({ importedRecords: 1 });
  mocks.authStart.mockResolvedValue({
    device_code: "assistant-grant",
    user_code: "ABCD-EFGH",
    verification_uri: "https://example.test/verify",
    verification_uri_complete: null,
    expires_in: 600,
    interval: 5,
  });
  mocks.authPoll.mockResolvedValue(null);
  mocks.authCancel.mockResolvedValue(undefined);
  mocks.listSessions.mockImplementation(async () => [current.session]);
  mocks.createSession.mockImplementation(async () => current.session);
  mocks.getSession.mockImplementation(async () => current);
  mocks.updateSession.mockImplementation(async (patch) => {
    current = {
      ...current,
      session: {
        ...current.session,
        ...patch,
        deletedAt:
          patch.deleted === false
            ? null
            : patch.deleted
              ? 2
              : current.session.deletedAt,
      },
    };
    return current.session;
  });
  mocks.send.mockResolvedValue({ runId: "unused", operationId: "unused" });
  mocks.stop.mockResolvedValue(undefined);
  mocks.pickAttachments.mockResolvedValue([]);
  mocks.exportSession.mockResolvedValue({ path: null });
  mocks.purgeSession.mockResolvedValue(undefined);
  mocks.subscribe.mockImplementation(async (listener) => {
    mocks.listener = listener;
    return () => undefined;
  });
  mocks.saveMemory.mockImplementation(async ({ content, id }) => ({
    id: id || "memory-a",
    content,
    updatedAt: 1,
  }));
  mocks.deleteMemory.mockResolvedValue(undefined);
});

describe("built-in assistant", () => {
  it("keeps the signed-out page free of assistant refresh and invokes only the requested login flow", async () => {
    const login = vi.fn();
    function RefreshControl() {
      const shell = useShell();
      return (
        <>
          <button onClick={() => shell.setActive("assistant")}>
            Open assistant
          </button>
          <button
            disabled={!shell.canRefresh}
            onClick={() => void shell.refresh()}
          >
            Refresh assistant
          </button>
        </>
      );
    }
    render(
      <ShellProvider openSettings={() => undefined}>
        <RefreshControl />
        <AssistantPage enabled={false} onLoginRequested={login} />
      </ShellProvider>,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Open assistant" }),
    );
    expect(
      screen.getByRole("button", { name: "Refresh assistant" }),
    ).toBeDisabled();
    expect(
      screen.queryByRole("textbox", { name: "Message" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Sign in to Ofox" }),
    );
    expect(login).toHaveBeenCalledOnce();
    expect(mocks.bootstrap).not.toHaveBeenCalled();
    expect(mocks.listSessions).not.toHaveBeenCalled();
    expect(mocks.subscribe).not.toHaveBeenCalled();
  });

  it.each([
    [
      "Summarize content",
      "Please summarize the following content, highlight the key points, and list any action items:\n\n",
    ],
    [
      "Polish writing",
      "Please polish the following text to make it clearer and more natural:\n\n",
    ],
    [
      "Explain a question",
      "Please explain the following question in plain language and give an example:\n\n",
    ],
  ])(
    "uses the %s starter to prepare an editable draft without sending or creating a conversation",
    async (label, prompt) => {
      mocks.listSessions.mockResolvedValue([]);
      render(page());
      await userEvent.click(await screen.findByRole("button", { name: label }));
      const composer = screen.getByRole("textbox", {
        name: "Message",
      }) as HTMLTextAreaElement;
      expect(composer).toHaveValue(prompt);
      expect(composer).toHaveFocus();
      expect(composer.selectionStart).toBe(prompt.length);
      expect(
        screen.queryByRole("button", { name: label }),
      ).not.toBeInTheDocument();
      expect(mocks.send).not.toHaveBeenCalled();
      expect(mocks.createSession).not.toHaveBeenCalled();
    },
  );

  it.each([
    [
      "Check tool status",
      "Please check the status of my AI tools and explain anything that needs attention.",
    ],
    [
      "Diagnose a connection",
      "Please help me diagnose this connection problem. Explain your findings before changing any settings:\n\n",
    ],
    [
      "Choose a model",
      "Please explain my current model settings and help me choose a suitable model.",
    ],
  ])(
    "prepares the configuration starter %s without starting a process or changing settings",
    async (label, prompt) => {
      mocks.listSessions.mockResolvedValue([]);
      render(page());
      await userEvent.click(
        screen.getByRole("combobox", { name: "Choose assistant" }),
      );
      await userEvent.click(
        screen.getByRole("option", { name: /Configuration Assistant/ }),
      );
      await userEvent.click(await screen.findByRole("button", { name: label }));
      expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
        prompt,
      );
      expect(screen.getByRole("textbox", { name: "Message" })).toHaveFocus();
      expect(mocks.send).not.toHaveBeenCalled();
      expect(mocks.createSession).not.toHaveBeenCalled();
      expect(mocks.updateSession).not.toHaveBeenCalled();
      expect(mocks.respondApproval).not.toHaveBeenCalled();
    },
  );

  it("preserves an existing draft without offering starters that could replace it", async () => {
    current.session.draft = "Existing private writing";
    await openConversation();
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Existing private writing",
    );
    expect(
      screen.queryByRole("button", { name: "Summarize content" }),
    ).not.toBeInTheDocument();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it.each([
    ["assistant_signing_out", "zh", "正在退出账号，已停止接受新的助手任务。"],
    [
      "assistant_signing_out",
      "en",
      "Signing out. New assistant tasks are no longer accepted.",
    ],
    [
      "assistant_signing_out",
      "ja",
      "ログアウト中です。新しいアシスタントタスクの受付を停止しました。",
    ],
    ["assistant_shutting_down", "zh", "应用正在退出，已停止接受新的助手任务。"],
    [
      "assistant_shutting_down",
      "en",
      "The app is closing. New assistant tasks are no longer accepted.",
    ],
    [
      "assistant_shutting_down",
      "ja",
      "アプリを終了中です。新しいアシスタントタスクの受付を停止しました。",
    ],
    ["assistant_sign_in_required", "zh", "请先登录后再使用助手。"],
    ["assistant_sign_in_required", "en", "Sign in before using the assistant."],
    [
      "assistant_sign_in_required",
      "ja",
      "アシスタントを使うにはログインしてください。",
    ],
    [
      "assistant_search_outcome_unknown",
      "zh",
      "搜索可能已完成或计费，已停止自动重试。请先确认结果再决定是否重试。",
    ],
    [
      "assistant_search_outcome_unknown",
      "en",
      "The search may have completed or incurred a charge. Automatic retries have stopped. Check the outcome before retrying.",
    ],
    [
      "assistant_search_outcome_unknown",
      "ja",
      "検索が完了または課金された可能性があります。自動再試行は停止しました。再試行する前に結果を確認してください。",
    ],
    [
      "assistant_vision_model_required",
      "zh",
      "当前内容需要图片理解能力，请切换到支持图片的模型后重试。原消息和附件会保留。",
    ],
    [
      "assistant_vision_model_required",
      "en",
      "This content requires image understanding. Switch to a model that supports images and retry. Your message and attachments are kept.",
    ],
    [
      "assistant_vision_model_required",
      "ja",
      "この内容には画像理解が必要です。画像対応モデルに切り替えて再試行してください。メッセージと添付ファイルは保持されます。",
    ],
  ])(
    "localizes a wrapped assistant error %s in %s",
    async (code, language, expected) => {
      mocks.send.mockRejectedValueOnce(
        new Error(`assistant_model_request_failed: ${code}`),
      );
      await openConversation();
      i18n.addResourceBundle("zh", "translation", zh, true, true);
      i18n.addResourceBundle("ja", "translation", ja, true, true);
      await act(async () => {
        await i18n.changeLanguage(language);
      });
      await userEvent.type(
        screen.getByRole("textbox", { name: i18n.t("assistant.message") }),
        "Test question",
      );
      await userEvent.click(
        screen.getByRole("button", { name: i18n.t("assistant.send") }),
      );
      expect(await screen.findByRole("alert")).toHaveTextContent(expected);
      expect(
        screen.getByRole("textbox", { name: i18n.t("assistant.message") }),
      ).toHaveValue("Test question");
    },
  );

  it("stops a pending send before a delayed draft write can dispatch a paid request", async () => {
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    mocks.updateSession.mockImplementation(async (patch) => {
      if (patch.draft === "Question") await pending;
      return { ...session, ...patch };
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    act(() => hook.result.current.setDraft("Question"));
    await waitFor(() => expect(mocks.updateSession).toHaveBeenCalled());
    let submission!: Promise<boolean>;
    act(() => {
      submission = hook.result.current.send();
    });
    await waitFor(() => expect(hook.result.current.run).not.toBeNull());
    await act(async () => hook.result.current.stop());
    await act(async () => {
      release();
      await submission;
    });
    expect(mocks.send).not.toHaveBeenCalled();
    expect(hook.result.current.draft).toBe("Question");
    expect(hook.result.current.run).toBeNull();
  });

  it("revokes queued draft writes on unmount instead of dispatching them under another identity", async () => {
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    mocks.updateSession.mockImplementation(async ({ draft }) => {
      if (draft === "First draft") await pending;
      return { ...session, draft };
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    act(() => hook.result.current.setDraft("First draft"));
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({ draft: "First draft" }),
      ),
    );
    act(() => hook.result.current.setDraft("Old identity's latest draft"));
    hook.unmount();
    await act(async () => {
      release();
      await pending;
    });
    expect(mocks.updateSession).not.toHaveBeenCalledWith(
      expect.objectContaining({ draft: "Old identity's latest draft" }),
    );
  });

  it("orders a send after an in-flight draft write so sent text cannot reappear as a draft", async () => {
    let release!: () => void;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    mocks.updateSession.mockImplementation(async ({ draft, ...patch }) => {
      if (draft === "Question") await pending;
      current.session = { ...current.session, ...patch, draft };
      return current.session;
    });
    mocks.send.mockImplementation(async () => {
      current.session.draft = "";
      return { runId: "sent", operationId: "sent-op" };
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    act(() => hook.result.current.setDraft("Question"));
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({ draft: "Question" }),
      ),
    );
    let submission!: Promise<boolean>;
    act(() => {
      submission = hook.result.current.send();
    });
    await act(async () => {
      release();
      await submission;
    });
    expect(current.session.draft).toBe("");
  });

  it("does not overwrite a draft when leaving a conversation before it loads", async () => {
    let finish!: (value: AssistantSessionDetail) => void;
    const pending = new Promise<AssistantSessionDetail>((resolve) => {
      finish = resolve;
    });
    mocks.listSessions.mockResolvedValue([
      session,
      { ...session, id: "session-b", title: "Other conversation" },
    ]);
    mocks.getSession.mockImplementation((id) =>
      id === session.id
        ? pending
        : Promise.resolve({
            ...current,
            session: { ...session, id, title: "Other conversation" },
          }),
    );
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: /Other conversation/ }),
    );
    await waitFor(() =>
      expect(screen.getByRole("textbox", { name: "Message" })).toBeEnabled(),
    );
    await act(async () =>
      finish({
        ...current,
        session: { ...session, draft: "Keep the server draft" },
      }),
    );
    expect(mocks.updateSession).not.toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: session.id, draft: "" }),
    );
  });

  it("restores an attachment draft after switching conversations and remounting", async () => {
    const a = { ...session };
    const b = { ...session, id: "session-b", title: "Other conversation" };
    const saved = new Map<string, AssistantSessionDetail>([
      [a.id, { session: a, messages: [], memories: [], attachments: [] }],
      [b.id, { session: b, messages: [], memories: [] }],
    ]);
    mocks.listSessions.mockImplementation(async () =>
      [...saved.values()].map((d) => d.session),
    );
    mocks.getSession.mockImplementation(async (id) => saved.get(id));
    mocks.updateSession.mockImplementation(async ({ sessionId, ...patch }) => {
      const d = saved.get(sessionId)!;
      d.session = { ...d.session, ...patch };
      return d.session;
    });
    mocks.pickAttachments.mockImplementation(async () => {
      const file = {
        id: "draft-file",
        name: "draft.pdf",
        mimeType: "application/pdf",
        sizeBytes: 10,
        status: "ready" as const,
      };
      saved.get(a.id)!.attachments = [file];
      return [file];
    });
    const view = render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await screen.findByRole("combobox", { name: "Choose model" });
    await userEvent.click(screen.getByRole("button", { name: "Attach files" }));
    await screen.findByText("draft.pdf");
    await userEvent.click(
      screen.getByRole("button", { name: /Other conversation/ }),
    );
    await screen.findByRole("combobox", { name: "Choose model" });
    await userEvent.click(
      screen.getByRole("button", { name: /My conversation/ }),
    );
    expect(await screen.findByText("draft.pdf")).toBeVisible();
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: a.id,
          draftAttachmentIds: ["draft-file"],
        }),
      ),
    );
    view.unmount();
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    expect(await screen.findByText("draft.pdf")).toBeVisible();
  });

  it("marks incomplete conversation usage and leaves unknown token counts unavailable", async () => {
    current.usage = { usageCoverage: "incomplete", estimatedCostUsd: null };
    await openConversation();
    expect(
      screen.getByText(
        "Known usage only; this conversation has incomplete usage records.",
      ),
    ).toBeVisible();
    expect(screen.getByText(/Input —/)).toBeVisible();
    expect(screen.queryByText(/Input 0/)).not.toBeInTheDocument();
  });

  it("previews conflicts before confirming a restore and refreshes local history", async () => {
    let generation = 1;
    mocks.bootstrap.mockImplementation(async () => ({
      ...capabilities,
      owner: "test-owner",
      authorityGeneration: generation,
    }));
    mocks.restore.mockImplementation(async () => {
      generation++;
      return { importedRecords: 1 };
    });
    mocks.previewRestore.mockResolvedValue({
      receipt: "restore-receipt",
      sessionCount: 1,
      fileCount: 2,
      newRecords: 1,
      skippedRecords: 0,
      conflicts: ["files:kept-local"],
    });
    render(page());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Assistant backup and restore",
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Preview restore" }),
    );
    expect(await screen.findByText("files:kept-local")).toBeInTheDocument();
    expect(mocks.restore).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "Restore validated backup" }),
    );
    await waitFor(() =>
      expect(mocks.restore).toHaveBeenCalledWith("restore-receipt"),
    );
    expect(await screen.findByText("Restored 1 records.")).toBeInTheDocument();
    await waitFor(() => expect(mocks.bootstrap).toHaveBeenCalledTimes(2));
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", { name: "Close" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Assistant backup and restore" }),
      ).toHaveFocus(),
    );
    await userEvent.click(
      screen.getByRole("button", { name: /My conversation/ }),
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "Draft after restore",
    );
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          draft: "Draft after restore",
          expectedOwner: "test-owner",
          expectedGeneration: 2,
        }),
      ),
    );
  });
  it("cancels a separate assistant grant while retaining the existing history", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      reauthorizationRequired: true,
    });
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: "Authorize assistant" }),
    );
    expect(await screen.findByText("ABCD-EFGH")).toBeInTheDocument();
    expect(mocks.authStart).toHaveBeenCalledWith(true);
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Cancel",
      }),
    );
    await waitFor(() =>
      expect(mocks.authCancel).toHaveBeenCalledWith("assistant-grant"),
    );
    expect(
      await screen.findByRole("button", { name: /My conversation/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Assistant backup and restore" }),
    ).toBeEnabled();
  });

  it("offers complete backup and restore while model authorization is unavailable", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [],
      reauthorizationRequired: true,
      runtime: { available: false },
    });
    render(page());
    expect(
      await screen.findByRole("button", {
        name: "Assistant backup and restore",
      }),
    ).toBeEnabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Assistant backup and restore" }),
    );
    expect(
      await screen.findByRole("button", { name: "Create complete backup" }),
    ).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "Preview restore" }),
    ).toBeEnabled();
  });

  it("lets the user lower permissions while a configuration run is active", async () => {
    current = {
      ...current,
      session: {
        ...session,
        kind: "configuration",
        permissionMode: "fullAccess",
        activeRun: { runId: "active-run", operationId: "active-operation" },
      },
    };
    render(page());
    await userEvent.click(
      screen.getByRole("combobox", { name: "Choose assistant" }),
    );
    await userEvent.click(
      await screen.findByRole("option", { name: /Configuration Assistant/ }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    const permissions = await screen.findByRole("combobox", {
      name: "Permissions",
    });
    expect(permissions).toBeEnabled();
    await userEvent.click(permissions);
    await userEvent.click(
      await screen.findByRole("option", { name: /Read only/ }),
    );
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        permissionMode: "readOnly",
      }),
    );
  });

  it("continues a saved in-flight reply as one message when returning to a running conversation", async () => {
    current = {
      ...current,
      session: {
        ...session,
        activeRun: { runId: "resumed-run", operationId: "resumed-operation" },
      },
      messages: [
        {
          id: "running-reply",
          role: "assistant",
          content: "Saved prefix",
          status: "streaming",
          createdAt: 1,
        },
      ],
    };
    await openConversation();
    await screen.findByText("Saved prefix");
    act(() =>
      mocks.listener?.({
        sessionId: "session-a",
        runId: "resumed-run",
        operationId: "resumed-operation",
        type: "text_delta",
        data: { delta: " and live tail" },
      }),
    );
    expect(await screen.findByText("Saved prefix and live tail")).toBeVisible();
  });

  it("does not jump back to a new session after the user selects another conversation", async () => {
    let resolve!: (session: AssistantSession) => void;
    mocks.createSession.mockReturnValue(
      new Promise<AssistantSession>((done) => {
        resolve = done;
      }),
    );
    const { result } = renderHook(() => useAssistant(true));
    let creating!: Promise<void>;
    act(() => {
      creating = result.current.create();
    });
    act(() => result.current.select("session-a"));
    await waitFor(() =>
      expect(result.current.detail?.session.id).toBe("session-a"),
    );
    await act(async () => {
      resolve({ ...session, id: "late-created" });
      await creating;
    });
    expect(result.current.selectedId).toBe("session-a");
    expect(result.current.error).toBeNull();
  });

  it("loads persisted drafts without replacing them with an empty selection draft", async () => {
    current = {
      ...current,
      session: { ...session, draft: "Saved across restart" },
    };
    await openConversation();
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Saved across restart",
    );
  });

  it("does not create another session while the selected conversation is loading", async () => {
    let resolve!: (detail: AssistantSessionDetail) => void;
    mocks.getSession.mockReturnValue(
      new Promise<AssistantSessionDetail>((done) => {
        resolve = done;
      }),
    );
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    const composer = screen.getByRole("textbox", { name: "Message" });
    fireEvent.change(composer, { target: { value: "Still loading" } });
    fireEvent.keyDown(composer, { key: "Enter" });
    expect(mocks.createSession).not.toHaveBeenCalled();
    expect(mocks.send).not.toHaveBeenCalled();
    await act(async () => {
      resolve(current);
    });
  });

  it("restores an outstanding approval when returning to its conversation", async () => {
    current = {
      ...current,
      session: {
        ...session,
        activeRun: { runId: "run-a", operationId: "op-a" },
      },
      pendingApproval: {
        id: "approval-a",
        title: "Install Codex",
        description: "Needs approval",
      },
    };
    await openConversation();
    expect(
      await screen.findByRole("region", { name: "Waiting for approval" }),
    ).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Deny" }));
    expect(mocks.respondApproval).toHaveBeenCalledWith(
      expect.objectContaining({
        approvalId: "approval-a",
        runId: "run-a",
        operationId: "op-a",
        decision: "deny",
      }),
    );
  });

  it("rejects Enter submission while an attachment is still processing", async () => {
    mocks.pickAttachments.mockResolvedValue([
      {
        id: "file-a",
        name: "notes.pdf",
        mimeType: "application/pdf",
        sizeBytes: 42,
        status: "processing",
      },
    ]);
    await openConversation();
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "Read this",
    );
    await userEvent.click(screen.getByRole("button", { name: "Attach files" }));
    await screen.findByText("notes.pdf");
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Attach files" }),
      ).toBeEnabled(),
    );
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Message" }), {
      key: "Enter",
    });
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("does not overwrite a newer setting when update replies arrive out of order", async () => {
    const { result } = renderHook(() => useAssistant(true));
    act(() => result.current.select("session-a"));
    await waitFor(() =>
      expect(result.current.detail?.session.id).toBe("session-a"),
    );
    let first!: (value: AssistantSession) => void;
    let second!: (value: AssistantSession) => void;
    mocks.updateSession
      .mockReturnValueOnce(
        new Promise<AssistantSession>((done) => {
          first = done;
        }),
      )
      .mockReturnValueOnce(
        new Promise<AssistantSession>((done) => {
          second = done;
        }),
      );
    let a!: Promise<AssistantSession | undefined>;
    let b!: Promise<AssistantSession | undefined>;
    act(() => {
      a = result.current.update({ modelId: "model-a" });
      b = result.current.update({ modelId: "model-b" });
    });
    await act(async () => {
      second({ ...session, modelId: "model-b" });
      await b;
    });
    await act(async () => {
      first({ ...session, modelId: "model-a" });
      await a;
    });
    expect(result.current.detail?.session.modelId).toBe("model-b");
  });

  it("discards the previous account's transcript and draft when access is disabled", async () => {
    const view = render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await screen.findByRole("combobox", { name: "Choose model" });
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "Private draft",
    );
    view.rerender(page(false));
    expect(
      screen.queryByRole("textbox", { name: "Message" }),
    ).not.toBeInTheDocument();
    current = {
      ...current,
      session: { ...session, id: "session-b", title: "Other account" },
    };
    view.rerender(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /Other account/ }),
    );
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue("");
  });

  it("explains an unavailable runtime and does not simulate a working assistant", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [],
      runtime: { available: false, reason: "Bundled executable is missing" },
    });
    render(page());
    expect(
      await screen.findByText("Bundled executable is missing"),
    ).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Message" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(mocks.send).not.toHaveBeenCalled();
    expect(screen.queryByText("0", { exact: true })).not.toBeInTheDocument();
  });

  it("preserves a draft and attachments after a failed send", async () => {
    mocks.pickAttachments.mockResolvedValue([
      {
        id: "file-a",
        name: "notes.pdf",
        mimeType: "application/pdf",
        sizeBytes: 42,
        status: "ready",
      },
    ]);
    mocks.send.mockRejectedValueOnce(new Error("Provider unavailable"));
    await openConversation();
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "Read this please",
    );
    await userEvent.click(screen.getByRole("button", { name: "Attach files" }));
    await screen.findByText("notes.pdf");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("alert");
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Read this please",
    );
    expect(screen.getByText("notes.pdf")).toBeVisible();
    expect(mocks.send).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "session-a",
        text: "Read this please",
        attachmentIds: ["file-a"],
      }),
    );
  });

  it("ignores stale events and stops only the currently displayed run", async () => {
    await openConversation();
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "Hello",
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() => {
      mocks.listener?.({
        ...sent,
        operationId: "old-operation",
        type: "text_delta",
        data: { delta: "STALE REPLY" },
      });
      mocks.listener?.({
        ...sent,
        sessionId: "another-session",
        type: "text_delta",
        data: { delta: "WRONG SESSION" },
      });
      mocks.listener?.({
        ...sent,
        type: "text_delta",
        data: { delta: "Current reply" },
      });
    });
    expect(await screen.findByText("Current reply")).toBeVisible();
    expect(screen.queryByText("STALE REPLY")).not.toBeInTheDocument();
    expect(screen.queryByText("WRONG SESSION")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.stop).toHaveBeenCalledWith(
      "session-a",
      sent.runId,
      sent.operationId,
    );
  });

  it("requires an explicit credential-boundary confirmation before enabling full access", async () => {
    current = { ...current, session: { ...session, kind: "configuration" } };
    await openConversation();
    await userEvent.click(
      screen.getByRole("combobox", { name: "Choose assistant" }),
    );
    await userEvent.click(
      await screen.findByRole("option", { name: /Configuration Assistant/ }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await userEvent.click(
      await screen.findByRole("combobox", { name: "Permissions" }),
    );
    const option = await screen.findByRole("option", { name: /Full access/ });
    await userEvent.click(option);
    const dialog = await screen.findByRole("dialog", {
      name: "Enable full access?",
    });
    expect(
      within(dialog).getByText(
        /model cannot be guaranteed never to access credentials/,
      ),
    ).toBeVisible();
    expect(mocks.updateSession).not.toHaveBeenCalledWith(
      expect.objectContaining({ permissionMode: "fullAccess" }),
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Confirm full access" }),
    );
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        permissionMode: "fullAccess",
        fullAccessAcknowledged: true,
      }),
    );
  });

  it("restores trashed conversations and confirms permanent deletion", async () => {
    current = { ...current, session: { ...session, deletedAt: 2 } };
    await openConversation();
    expect(
      screen.getByText(
        "This conversation is in the trash. Restore it to continue.",
      ),
    ).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Delete permanently" }),
    );
    expect(mocks.purgeSession).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog", {
      name: "Delete permanently",
    });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Restore" }));
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        deleted: false,
      }),
    );
  });

  it("does not treat a missing web capability or price as free search", async () => {
    mocks.bootstrap.mockResolvedValue({ ...capabilities, search: undefined });
    await openConversation();
    expect(screen.getByRole("switch", { name: "Web search" })).toBeDisabled();
    expect(screen.getByText("Web search unavailable")).toBeVisible();
    expect(screen.queryByText(/free/i)).not.toBeInTheDocument();
  });

  it("only stores memories the user explicitly adds", async () => {
    await openConversation();
    expect(mocks.saveMemory).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "Manual memory" }),
    );
    const dialog = await screen.findByRole("dialog", { name: "Manual memory" });
    await userEvent.type(
      within(dialog).getByRole("textbox", { name: "Memory content" }),
      "Use concise answers",
    );
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Add memory" }),
    );
    expect(
      await within(dialog).findByText("Use concise answers"),
    ).toBeVisible();
    expect(mocks.saveMemory).toHaveBeenCalledWith({
      sessionId: "session-a",
      id: undefined,
      content: "Use concise answers",
    });
  });

  it("keeps original messages visible while submitting an edited version", async () => {
    current = {
      ...current,
      messages: [
        {
          id: "original",
          role: "user",
          content: "Original question",
          createdAt: 1,
        },
      ],
    };
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    const editor = screen.getByRole("textbox", { name: "Edit and resend" });
    fireEvent.change(editor, { target: { value: "Updated question" } });
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({
          editMessageId: "original",
          text: "Updated question",
        }),
      ),
    );
    expect(screen.getByText("Original question")).toBeVisible();
  });

  it("edits an attachment-only message without consuming the composer draft", async () => {
    const attachment = {
      id: "image-a",
      name: "picture.png",
      mimeType: "image/png",
      sizeBytes: 42,
      status: "ready" as const,
    };
    current.messages = [
      {
        id: "original",
        role: "user",
        content: "",
        createdAt: 1,
        attachments: [attachment],
      },
    ];
    await openConversation();
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "Unsent draft",
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({
          editMessageId: "original",
          text: "",
          attachmentIds: ["image-a"],
        }),
      ),
    );
    expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
      "Unsent draft",
    );
  });

  it.each(["error", "stopped"] as const)(
    "regenerates an empty %s response while retaining the composer draft",
    async (status) => {
      current.messages = [
        { id: "question", role: "user", content: "Question", createdAt: 1 },
        {
          id: "response",
          role: "assistant",
          content: "",
          createdAt: 2,
          status,
        },
      ];
      await openConversation();
      await userEvent.type(
        screen.getByRole("textbox", { name: "Message" }),
        "Unsent draft",
      );
      await userEvent.click(screen.getByRole("button", { name: "Regenerate" }));
      await waitFor(() =>
        expect(mocks.send).toHaveBeenCalledWith(
          expect.objectContaining({
            regenerateMessageId: "response",
            text: "",
            attachmentIds: [],
          }),
        ),
      );
      expect(screen.getByRole("textbox", { name: "Message" })).toHaveValue(
        "Unsent draft",
      );
    },
  );

  it("allows choosing a catalog model before the first session when the balanced model is absent", async () => {
    mocks.listSessions.mockResolvedValue([]);
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [{ id: "other/model", name: "Available model" }],
      defaultModelId: null,
      defaultModelIds: { general: null, configuration: null },
    });
    mocks.createSession.mockImplementation(async (request) => {
      if (!request.modelId) throw new Error("assistant_choose_model");
      return { ...session, modelId: request.modelId };
    });
    render(page());
    const model = await screen.findByRole("combobox", { name: "Choose model" });
    await userEvent.click(model);
    await userEvent.click(
      await screen.findByRole("option", { name: /Available model/ }),
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "Message" }),
      "First question",
    );
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() =>
      expect(mocks.createSession).toHaveBeenCalledWith(
        expect.objectContaining({ kind: "general", modelId: "other/model" }),
      ),
    );
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
  });
});
