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
  AssistantSessionList,
} from "@/lib/api/assistant";

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  listSessions: vi.fn(),
  createSession: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  send: vi.fn(),
  stop: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
  listener: undefined as ((event: AssistantEvent) => void) | undefined,
  auth: undefined as ((state: "expired" | "restored") => void) | undefined,
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));

const session: AssistantSession = {
  id: "session-a",
  title: "My conversation",
  modelId: "balanced",
  webSearch: false,
  createdAt: 1,
  updatedAt: 1,
};
const capabilities: AssistantBootstrap = {
  runtime: { available: true, version: "test" },
  models: [
    { id: "balanced", name: "Balanced model" },
    { id: "fast", name: "Fast model" },
  ],
  defaultModelId: "balanced",
  key: { status: "ready" },
};
let current: AssistantSessionDetail;
const listOf = (
  sessions: AssistantSession[],
  unreadable: AssistantSessionList["unreadable"] = [],
): AssistantSessionList => ({ sessions, unreadable });

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
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Message" })).toBeEnabled(),
  );
}
const composer = () =>
  screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  current = { session: { ...session }, messages: [], attachments: [] };
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.listSessions.mockImplementation(async () => listOf([current.session]));
  mocks.createSession.mockImplementation(async () => current.session);
  mocks.getSession.mockImplementation(async () => current);
  mocks.updateSession.mockImplementation(async (patch) => {
    current = { ...current, session: { ...current.session, ...patch } };
    return current.session;
  });
  mocks.send.mockResolvedValue({ runId: "unused", operationId: "unused" });
  mocks.stop.mockResolvedValue(undefined);
  mocks.subscribe.mockImplementation(async (listener) => {
    mocks.listener = listener;
    return () => undefined;
  });
  mocks.onAuthChange.mockImplementation(async (callback) => {
    mocks.auth = callback;
    return () => undefined;
  });
});

describe("signed out", () => {
  it("keeps the page free of assistant refresh and loading, and only asks to sign in", async () => {
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
});

describe("starting a conversation", () => {
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
      mocks.listSessions.mockResolvedValue(listOf([]));
      render(page());
      await userEvent.click(await screen.findByRole("button", { name: label }));
      expect(composer()).toHaveValue(prompt);
      expect(composer()).toHaveFocus();
      expect(composer().selectionStart).toBe(prompt.length);
      expect(
        screen.queryByRole("button", { name: label }),
      ).not.toBeInTheDocument();
      expect(mocks.send).not.toHaveBeenCalled();
      expect(mocks.createSession).not.toHaveBeenCalled();
    },
  );

  it("preserves an existing draft without offering starters that could replace it", async () => {
    current.session.draft = "Existing private writing";
    await openConversation();
    expect(composer()).toHaveValue("Existing private writing");
    expect(
      screen.queryByRole("button", { name: "Summarize content" }),
    ).not.toBeInTheDocument();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("lets the user choose a catalog model before the first conversation exists", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [{ id: "other/model", name: "Available model" }],
      defaultModelId: null,
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
    await userEvent.type(composer(), "First question");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() =>
      expect(mocks.createSession).toHaveBeenCalledWith({
        modelId: "other/model",
        webSearch: false,
      }),
    );
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
  });

  it("creates a conversation with the default model from the sidebar button", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    render(page());
    const create = await screen.findByRole("button", {
      name: "New conversation",
    });
    await waitFor(() => expect(create).toBeEnabled());
    await userEvent.click(create);
    await waitFor(() =>
      expect(mocks.createSession).toHaveBeenCalledWith({
        modelId: "balanced",
        webSearch: false,
      }),
    );
  });

  it("says the first message creates the assistant's own API key", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      key: { status: "missing" },
    });
    render(page());
    expect(
      await screen.findByText(
        "Your first message creates a dedicated Ofox API key for the assistant.",
      ),
    ).toBeVisible();
  });
});

describe("history", () => {
  it("searches conversations through the backend filter", async () => {
    render(page());
    await screen.findByRole("button", { name: /My conversation/ });
    await userEvent.type(
      screen.getByRole("searchbox", { name: "Search conversations" }),
      "notes",
    );
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(false, "notes"),
    );
  });

  it("shows a damaged conversation as a row that says it can't be read and cannot be opened", async () => {
    mocks.listSessions.mockResolvedValue(
      listOf(
        [session],
        [{ id: "damaged-session", code: "assistant_invalid_session" }],
      ),
    );
    render(page());
    const list = await screen.findByRole("list", { name: "Conversations" });
    const damaged = await within(list).findByText("Can't be read");
    const row = damaged.closest("li")!;
    expect(within(row).queryByRole("button")).toBeNull();
    expect(row).toHaveTextContent(
      "This conversation's saved file is damaged and can't be opened.",
    );
    expect(list).not.toHaveTextContent("assistant_invalid_session");
    await userEvent.click(damaged);
    expect(mocks.getSession).not.toHaveBeenCalled();
  });

  it("renames the open conversation", async () => {
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Rename conversation" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Rename conversation",
    });
    const title = within(dialog).getByRole("textbox", {
      name: "Conversation title",
    });
    await userEvent.clear(title);
    await userEvent.type(title, "Trip plan");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        title: "Trip plan",
      }),
    );
  });

  it("switches the model of an open conversation", async () => {
    await openConversation();
    await userEvent.click(
      screen.getByRole("combobox", { name: "Choose model" }),
    );
    await userEvent.click(
      await screen.findByRole("option", { name: /Fast model/ }),
    );
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        modelId: "fast",
      }),
    );
  });
});

describe("errors", () => {
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
    ["assistant_sign_in_required", "zh", "请先登录 Ofox 再使用助手。"],
    [
      "assistant_unknown_apex",
      "en",
      "The selected Ofox region isn't recognized. Choose a region in Settings and try again.",
    ],
    [
      "assistant_invalid_session",
      "ja",
      "この会話の保存ファイルが破損しているため開けません。",
    ],
    [
      "assistant_vision_model_required",
      "en",
      "This content requires image understanding. Switch to a model that supports images and retry. Your message is kept.",
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
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent(expected);
      expect(alert).not.toHaveTextContent("assistant_");
      expect(
        screen.getByRole("textbox", { name: i18n.t("assistant.message") }),
      ).toHaveValue("Test question");
    },
  );

  it("falls back to one generic localized message for an unknown code and never shows it", async () => {
    mocks.send.mockRejectedValueOnce("assistant_brand_new_failure");
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "The assistant ran into a problem. Please try again.",
    );
    expect(document.body).not.toHaveTextContent("assistant_brand_new_failure");
  });

  it("shows a run's error event as a localized message and ignores a user stop", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "error",
        data: { code: "stopped", message: "aborted" },
      }),
    );
    expect(screen.queryByRole("alert")).toBeNull();
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "error",
        data: { code: "runtime_error", message: "upstream 503 body" },
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The model service couldn't complete the reply. Please try again.",
    );
    expect(document.body).not.toHaveTextContent("upstream 503 body");
  });

  it("preserves a draft after a failed send", async () => {
    mocks.send.mockRejectedValueOnce(new Error("Provider unavailable"));
    await openConversation();
    await userEvent.type(composer(), "Read this please");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("alert");
    expect(composer()).toHaveValue("Read this please");
    expect(mocks.send).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "session-a",
        text: "Read this please",
        attachmentIds: [],
      }),
    );
  });

  it("explains an unavailable runtime and does not simulate a working assistant", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [],
      runtime: { available: false, reason: "/private/path/node is missing" },
    });
    render(page());
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Assistant runtime unavailable",
    );
    expect(screen.queryByText(/private\/path/)).not.toBeInTheDocument();
    expect(composer()).toBeDisabled();
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    expect(mocks.send).not.toHaveBeenCalled();
  });
});

describe("internal records", () => {
  it("never renders assistant_request records and shows progress as a typing state", async () => {
    current = {
      ...current,
      session: {
        ...session,
        activeRun: { runId: "run-a", operationId: "op-a" },
      },
      messages: [
        { id: "q", role: "user", content: "Question", createdAt: 1 },
        {
          id: "internal",
          role: "tool",
          content: "assistant_request",
          createdAt: 2,
        },
        {
          id: "reply",
          role: "assistant",
          content: "",
          status: "streaming",
          createdAt: 3,
        },
      ],
      // A detail written by an older build may still carry its run ledger.
      ...({
        operations: [
          { id: "op-a", title: "assistant_request", status: "running" },
        ],
      } as object),
    };
    await openConversation();
    expect(await screen.findByText("Thinking…")).toBeVisible();
    act(() => {
      mocks.listener?.({
        sessionId: "session-a",
        runId: "run-a",
        operationId: "op-a",
        type: "operation",
        data: {
          operation: { id: "op-a", title: "assistant_request" },
        },
      });
      mocks.listener?.({
        sessionId: "session-a",
        runId: "run-a",
        operationId: "op-a",
        type: "tool_start",
        data: { name: "assistant_request" },
      });
    });
    expect(screen.getByText("Question")).toBeVisible();
    expect(document.body).not.toHaveTextContent("assistant_request");
  });
});

describe("running a reply", () => {
  it("sends with Enter, keeps focus in the composer, and returns focus after Stop", async () => {
    await openConversation();
    await userEvent.click(composer());
    await userEvent.keyboard("Hello{Enter}");
    const stop = await screen.findByRole("button", { name: "Stop" });
    await waitFor(() => expect(composer()).toHaveFocus());
    stop.focus();
    await userEvent.keyboard("{Enter}");
    expect(mocks.stop).toHaveBeenCalledOnce();
    await waitFor(() => expect(composer()).toHaveFocus());
  });

  it("reaches the model picker from the keyboard", async () => {
    await openConversation();
    const picker = screen.getByRole("combobox", { name: "Choose model" });
    picker.focus();
    await userEvent.keyboard("{Enter}");
    expect(
      await screen.findByRole("option", { name: /Fast model/ }),
    ).toBeVisible();
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

  it("ignores stale events and stops only the currently displayed run", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
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

  it("replaces the streamed reply with the saved transcript when the run ends", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({ ...sent, type: "text_delta", data: { delta: "Hi" } }),
    );
    current = {
      ...current,
      messages: [
        { id: "u", role: "user", content: "Hello", createdAt: 1 },
        {
          id: "a",
          role: "assistant",
          content: "Hi there",
          status: "completed",
          createdAt: 2,
        },
      ],
    };
    act(() =>
      mocks.listener?.({ ...sent, type: "run_end", data: { stopped: false } }),
    );
    expect(await screen.findByText("Hi there")).toBeVisible();
    expect(await screen.findByRole("button", { name: "Send" })).toBeVisible();
    expect(screen.queryByText("Thinking…")).toBeNull();
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
      await userEvent.type(composer(), "Unsent draft");
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
      expect(composer()).toHaveValue("Unsent draft");
    },
  );
});

describe("drafts and fencing", () => {
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

  it("sends the signed-in owner and generation with every draft write", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      owner: "test-owner",
      authorityGeneration: 2,
    });
    await openConversation();
    await userEvent.type(composer(), "Draft for this owner");
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        draft: "Draft for this owner",
        expectedOwner: "test-owner",
        expectedGeneration: 2,
      }),
    );
  });

  it("does not overwrite a draft when leaving a conversation before it loads", async () => {
    let finish!: (value: AssistantSessionDetail) => void;
    const pending = new Promise<AssistantSessionDetail>((resolve) => {
      finish = resolve;
    });
    mocks.listSessions.mockResolvedValue(
      listOf([
        session,
        { ...session, id: "session-b", title: "Other conversation" },
      ]),
    );
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
    await waitFor(() => expect(composer()).toBeEnabled());
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

  it("does not jump back to a new session after the user selects another conversation", async () => {
    let resolve!: (session: AssistantSession) => void;
    mocks.createSession.mockReturnValue(
      new Promise<AssistantSession>((done) => {
        resolve = done;
      }),
    );
    const { result } = renderHook(() => useAssistant(true));
    await waitFor(() => expect(result.current.bootstrap).not.toBeNull());
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
    expect(composer()).toHaveValue("Saved across restart");
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
    fireEvent.change(composer(), { target: { value: "Still loading" } });
    fireEvent.keyDown(composer(), { key: "Enter" });
    expect(mocks.createSession).not.toHaveBeenCalled();
    expect(mocks.send).not.toHaveBeenCalled();
    await act(async () => {
      resolve(current);
    });
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
    await waitFor(() => expect(composer()).toBeEnabled());
    await userEvent.type(composer(), "Private draft");
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
    expect(composer()).toHaveValue("");
  });
});

describe("selection and history edges", () => {
  it("keeps an open conversation on screen when its row is clicked again", async () => {
    current.messages = [
      { id: "q", role: "user", content: "Question", createdAt: 1 },
    ];
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: /My conversation/ }),
    );
    expect(screen.queryByText("Loading…")).toBeNull();
    expect(screen.getByText("Question")).toBeVisible();
    expect(mocks.getSession).toHaveBeenCalledTimes(1);
  });

  it("carries a draft typed before any conversation into the new one", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    render(page());
    const create = await screen.findByRole("button", {
      name: "New conversation",
    });
    await waitFor(() => expect(create).toBeEnabled());
    await userEvent.type(composer(), "Unsent idea");
    await userEvent.click(create);
    await waitFor(() => expect(mocks.createSession).toHaveBeenCalled());
    expect(composer()).toHaveValue("Unsent idea");
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          draft: "Unsent idea",
        }),
      ),
    );
  });

  it.each(["run_end", "error"])(
    "refreshes a background conversation's Replying… mark on its %s",
    async (type) => {
      const other = { ...session, id: "session-b", title: "Other" };
      mocks.listSessions.mockResolvedValue(
        listOf([
          { ...session, activeRun: { runId: "bg-run", operationId: "bg-op" } },
          other,
        ]),
      );
      render(page());
      const list = await screen.findByRole("list", { name: "Conversations" });
      await within(list).findByText("Replying…");
      mocks.listSessions.mockResolvedValue(listOf([session, other]));
      act(() =>
        mocks.listener?.({
          sessionId: "session-a",
          runId: "bg-run",
          operationId: "bg-op",
          type,
          data: {},
        }),
      );
      await waitFor(() =>
        expect(within(list).queryByText("Replying…")).toBeNull(),
      );
    },
  );

  it("describes a row the backend could not list by its own code", async () => {
    mocks.listSessions.mockResolvedValue(
      listOf([], [{ id: "x", code: "assistant_storage_unavailable" }]),
    );
    render(page());
    const list = await screen.findByRole("list", { name: "Conversations" });
    await within(list).findByText("Can't be read");
    expect(list).toHaveTextContent(
      "Local assistant data couldn't be read or saved.",
    );
    expect(list).not.toHaveTextContent("damaged");
  });

  it("dates the history in the app's language, not the system's", async () => {
    const stamp = Date.UTC(2026, 9, 15, 12);
    mocks.listSessions.mockResolvedValue(
      listOf([{ ...session, updatedAt: stamp }]),
    );
    i18n.addResourceBundle("zh", "translation", zh, true, true);
    await i18n.changeLanguage("zh");
    render(page());
    expect(await screen.findByText("2026/10/15")).toBeVisible();
  });
});

describe("run edges", () => {
  it("keeps a question being prepared when the conversation is refreshed before the backend registers its run", async () => {
    let accept!: (value: { runId: string; operationId: string }) => void;
    mocks.send.mockReturnValue(
      new Promise((resolve) => {
        accept = resolve;
      }),
    );
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    act(() => hook.result.current.setDraft("Question"));
    let submission!: Promise<boolean>;
    act(() => {
      submission = hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    const run = hook.result.current.run!;
    expect(run).not.toBeNull();
    await act(async () => {
      await hook.result.current.refreshDetail(session.id);
    });
    expect(hook.result.current.run).toEqual(run);
    expect(
      hook.result.current.detail?.messages.map((message) => message.content),
    ).toContain("Question");
    await act(async () => {
      accept(run);
      await submission;
    });
    act(() =>
      mocks.listener?.({
        sessionId: session.id,
        ...run,
        type: "text_delta",
        data: { delta: "Answer" },
      }),
    );
    expect(hook.result.current.stream?.content).toBe("Answer");
  });

  it("does not send the Enter that confirms an input-method candidate", async () => {
    await openConversation();
    fireEvent.change(composer(), { target: { value: "にほんご" } });
    fireEvent.keyDown(composer(), { key: "Enter", keyCode: 229 });
    fireEvent.keyDown(composer(), { key: "Enter", isComposing: true });
    await act(async () => undefined);
    expect(mocks.send).not.toHaveBeenCalled();
    expect(composer()).toHaveValue("にほんご");
    fireEvent.keyDown(composer(), { key: "Enter", keyCode: 13 });
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({ text: "にほんご" }),
      ),
    );
  });

  it("shows only an assistant reply from the final message event", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "message",
        data: {
          message: {
            id: "internal",
            role: "tool",
            content: "assistant_request",
            createdAt: 1,
          },
        },
      }),
    );
    expect(document.body).not.toHaveTextContent("assistant_request");
    expect(screen.getByText("Thinking…")).toBeVisible();
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "message",
        data: {
          message: {
            id: "reply",
            role: "assistant",
            content: "Final answer",
            status: "completed",
            createdAt: 2,
          },
        },
      }),
    );
    expect(await screen.findByText("Final answer")).toBeVisible();
  });

  it("keeps the run stoppable when an error arrives without its run end", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "error",
        data: { code: "assistant_run_timeout" },
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The reply took too long and was stopped. Please try again.",
    );
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    expect(mocks.stop).toHaveBeenCalledWith(
      "session-a",
      sent.runId,
      sent.operationId,
    );
  });
});

describe("sign-in changes", () => {
  it("drops the conversation and draft when sign-in expires, and reloads when it is restored", async () => {
    await openConversation();
    await userEvent.type(composer(), "Private draft");
    expect(mocks.bootstrap).toHaveBeenCalledTimes(1);
    act(() => mocks.auth?.("expired"));
    expect(composer()).toHaveValue("");
    expect(composer()).toBeDisabled();
    expect(
      screen.queryByRole("button", { name: /My conversation/ }),
    ).not.toBeInTheDocument();
    act(() => mocks.auth?.("restored"));
    await waitFor(() => expect(mocks.bootstrap).toHaveBeenCalledTimes(2));
    expect(
      await screen.findByRole("button", { name: /My conversation/ }),
    ).toBeVisible();
    expect(composer()).toHaveValue("");
    expect(mocks.updateSession).not.toHaveBeenCalledWith(
      expect.objectContaining({ draft: "Private draft" }),
    );
  });
});
