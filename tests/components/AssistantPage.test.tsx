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
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { useAssistant } from "@/components/assistant/useAssistant";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import type {
  AssistantAttachment,
  AssistantBootstrap,
  AssistantEvent,
  AssistantMessage,
  AssistantModel,
  AssistantRunRef,
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
  deleteSession: vi.fn(),
  exportMarkdown: vi.fn(),
  usageSummary: vi.fn(),
  backup: vi.fn(),
  restore: vi.fn(),
  pickAttachments: vi.fn(),
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
const toasts = vi.hoisted(() => ({
  success: vi.fn(),
  warning: vi.fn(),
  error: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: toasts }));

const session: AssistantSession = {
  id: "session-a",
  title: "My conversation",
  modelId: "balanced",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt: 1,
  deletedAt: null,
  activeRun: null,
};
const model = (id: string, name: string): AssistantModel => ({
  id,
  name,
  contextWindow: 128_000,
  maxOutputTokens: 16_000,
  supportsImages: false,
  inputModalities: ["text"],
  supportedEndpoints: ["chat/completions"],
  inputPricePerMillion: null,
  outputPricePerMillion: null,
  pricing: {},
  cost: { input: null, output: null, cacheRead: null, cacheWrite: null },
  pricingKnown: false,
});
const capabilities: AssistantBootstrap = {
  owner: "a".repeat(64),
  authorityGeneration: 1,
  runtime: { available: true, reason: null, version: "test" },
  models: [model("balanced", "Balanced model"), model("fast", "Fast model")],
  defaultModelId: "balanced",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: {
    available: true,
    unitPriceUsd: "0.014",
    model: "openai/gpt-5-nano",
  },
};
let current: AssistantSessionDetail;
const listOf = (
  sessions: AssistantSession[],
  unreadable: AssistantSessionList["unreadable"] = [],
  trashCount = 0,
): AssistantSessionList => ({ sessions, unreadable, trashCount });

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
/** Open the search behind its icon button and type into it. */
async function searchFor(text: string) {
  await userEvent.click(
    screen.getByRole("button", { name: "Search conversations" }),
  );
  await userEvent.type(
    screen.getByRole("searchbox", { name: "Search conversations" }),
    text,
  );
}
/** Pick an entry of the sidebar footer's More actions menu once it can run. */
async function chooseFromMenu(name: string) {
  await userEvent.click(
    screen.getByRole("button", { name: i18n.t("assistant.moreActions") }),
  );
  const item = await screen.findByRole("menuitem", { name });
  await waitFor(() =>
    expect(item).not.toHaveAttribute("aria-disabled", "true"),
  );
  await userEvent.click(item);
}
/** Each toast shown so far: its kind, then its title and description lines. */
const toasted = () =>
  (["success", "warning"] as const).flatMap((kind) =>
    toasts[kind].mock.calls.map(([title, options]) => ({
      kind,
      lines: [
        title,
        ...(options?.description ? options.description.split("\n") : []),
      ],
    })),
  );

afterEach(() => vi.unstubAllEnvs());

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  current = {
    session: { ...session },
    messages: [],
    attachments: [],
    usage: {},
  };
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.listSessions.mockImplementation(async () => listOf([current.session]));
  mocks.createSession.mockImplementation(async () => current.session);
  mocks.getSession.mockImplementation(async () => current);
  mocks.updateSession.mockImplementation(async (patch) => {
    current = { ...current, session: { ...current.session, ...patch } };
    return current.session;
  });
  mocks.deleteSession.mockResolvedValue(undefined);
  mocks.exportMarkdown.mockResolvedValue(true);
  mocks.backup.mockResolvedValue({ sessions: 1, skipped: 0 });
  mocks.restore.mockResolvedValue({
    imported: 1,
    skipped: 0,
    conflicts: 0,
    warning: null,
  });
  mocks.usageSummary.mockResolvedValue({
    inputTokens: 0,
    outputTokens: 0,
    estimatedCostUsd: "0",
    usageCoverage: "complete",
    searchCount: 0,
  });
  mocks.pickAttachments.mockResolvedValue([]);
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

describe("the composer's hint", () => {
  it("is readable: muted text like the other notes, not faint", async () => {
    await openConversation();
    const hint = screen.getByText("Enter to send · Shift+Enter for a new line");
    expect(hint).toHaveClass("text-ink-muted");
    expect(hint).not.toHaveClass("text-ink-faint");
  });
});

describe("the empty state", () => {
  const invitation = "Start a new conversation, or type your question below.";

  it("invites a question while the composer can take one", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    render(page());
    expect(await screen.findByText(invitation)).toBeVisible();
    await waitFor(() => expect(composer()).toBeEnabled());
  });

  it.each([
    [
      "there are no models",
      () =>
        mocks.bootstrap.mockResolvedValue({
          ...capabilities,
          models: [],
          defaultModelId: null,
        }),
    ],
    [
      "the runtime is unavailable",
      () =>
        mocks.bootstrap.mockResolvedValue({
          ...capabilities,
          runtime: { available: false, reason: null, version: null },
        }),
    ],
    [
      "the page can't follow runs",
      () => mocks.subscribe.mockRejectedValue("assistant_storage_unavailable"),
    ],
  ])("doesn't, when %s and the composer is disabled", async (_, setUp) => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    setUp();
    render(page());
    expect(await screen.findByText("How can I help?")).toBeVisible();
    // Let the capabilities and the event subscription settle.
    await waitFor(() => expect(mocks.bootstrap).toHaveBeenCalled());
    await act(async () => undefined);
    expect(composer()).toBeDisabled();
    expect(screen.queryByText(invitation)).toBeNull();
  });
});

describe("history", () => {
  it("searches conversations through the backend filter", async () => {
    render(page());
    await screen.findByRole("button", { name: /My conversation/ });
    await searchFor("notes");
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(false, "notes"),
    );
  });

  it("shows a damaged conversation as a row that says it can't be read and cannot be opened", async () => {
    mocks.listSessions.mockResolvedValue(
      listOf(
        [session],
        [{ id: "damaged-session", error: "assistant_invalid_session" }],
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
        expectedOwner: capabilities.owner,
        expectedGeneration: capabilities.authorityGeneration,
      }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Rename conversation" }),
      ).toHaveFocus(),
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
        expectedOwner: capabilities.owner,
        expectedGeneration: capabilities.authorityGeneration,
      }),
    );
  });
});

describe("the model picker", () => {
  it("says a model's context window in compact units and shows its whole note, which may wrap", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [
        {
          ...model("balanced", "Balanced model"),
          contextWindow: 128_000,
          inputPricePerMillion: "0.28",
          outputPricePerMillion: "0.42",
        },
        { ...model("fast", "Fast model"), contextWindow: 1_048_576 },
      ],
    });
    await openConversation();
    await userEvent.click(
      screen.getByRole("combobox", { name: "Choose model" }),
    );
    const balanced = await screen.findByRole("option", {
      name: /Balanced model/,
    });
    const note = within(balanced).getByText(/^Context /);
    expect(note).toHaveTextContent(
      "Context 128K tokens · Input $0.28 / output $0.42 per million tokens",
    );
    expect(note).toHaveClass("break-words");
    expect(note).not.toHaveClass("truncate");
    expect(
      within(screen.getByRole("option", { name: /Fast model/ })).getByText(
        "Context 1M tokens",
      ),
    ).toBeVisible();
    expect(document.body).not.toHaveTextContent("128000");
  });

  it.each([
    ["zh", "上下文 128K Token"],
    ["ja", "コンテキスト 128K トークン"],
  ])("words the context window in %s", async (language, expected) => {
    i18n.addResourceBundle("zh", "translation", zh, true, true);
    i18n.addResourceBundle("ja", "translation", ja, true, true);
    expect(
      i18n.getFixedT(language)("assistant.contextWindow", { tokens: "128K" }),
    ).toBe(expected);
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
      "assistant_key_storage_unavailable",
      "en",
      "The assistant's API key couldn't be read or saved on this computer. Make sure the system credential store is available, then try again.",
    ],
    [
      "assistant_key_unavailable",
      "zh",
      "没能创建助手的 API Key，请检查网络后重试。",
    ],
    [
      // A blocked turn never reached the model: it can only be sent again.
      "assistant_vision_model_required",
      "zh",
      "此模型无法读取图片。请选择能读取图片的模型，然后再发一次这条消息。",
    ],
    [
      "assistant_vision_model_required",
      "en",
      "This model can't read images. Choose a model that can read images, then send the message again.",
    ],
    [
      "assistant_vision_model_required",
      "ja",
      "このモデルは画像を読み取れません。画像を読み取れるモデルを選んでから、メッセージをもう一度送信してください。",
    ],
  ])(
    "localizes the command error %s in %s",
    async (code, language, expected) => {
      // A command rejects with the bare code.
      mocks.send.mockRejectedValueOnce(code);
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

  it.each([
    [
      "assistant_insufficient_balance",
      "en",
      "Your Ofox balance is too low for this reply. Top up your account, then try again.",
    ],
    ["assistant_rate_limited", "zh", "请求太频繁了，请稍等片刻再试。"],
    [
      "assistant_network_error",
      "ja",
      "Ofox のモデルサービスに接続できませんでした。インターネット接続を確認してもう一度お試しください。",
    ],
    [
      "assistant_key_rejected",
      "en",
      "Ofox didn't accept the assistant's API key. Wait a few minutes and try again. If it keeps happening, sign out and sign in again.",
    ],
    [
      "assistant_model_unavailable",
      "en",
      "This model isn't available right now. Choose another model.",
    ],
    [
      "assistant_search_outcome_unknown",
      "zh",
      "无法确认网页搜索是否已执行，回复已停止。这次搜索可能已经计费，因此不会自动重试。如仍需要回答，请重新发送消息。",
    ],
  ])(
    "explains the run failure %s in %s by its code, never with the provider's text",
    async (code, language, expected) => {
      await openConversation();
      i18n.addResourceBundle("zh", "translation", zh, true, true);
      i18n.addResourceBundle("ja", "translation", ja, true, true);
      await act(async () => {
        await i18n.changeLanguage(language);
      });
      await userEvent.type(
        screen.getByRole("textbox", { name: i18n.t("assistant.message") }),
        "Hello",
      );
      await userEvent.click(
        screen.getByRole("button", { name: i18n.t("assistant.send") }),
      );
      await screen.findByRole("button", { name: i18n.t("assistant.stop") });
      const sent = mocks.send.mock.calls[0][0];
      const provider = 'upstream said {"type":"insufficient_credits"}';
      act(() =>
        mocks.listener?.({
          ...sent,
          type: "error",
          data: { code, message: provider },
        }),
      );
      const alert = await screen.findByRole("alert");
      expect(alert).toHaveTextContent(expected);
      expect(document.body).not.toHaveTextContent(provider);
      expect(document.body).not.toHaveTextContent(code);
    },
  );

  it("shows no error for a reply the user stopped", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await userEvent.click(await screen.findByRole("button", { name: "Stop" }));
    const sent = mocks.send.mock.calls[0][0];
    // A stop ends with run_end alone: the host sends no error for it.
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "run_end",
        data: { stopped: true, status: "stopped" },
      }),
    );
    expect(await screen.findByRole("button", { name: "Send" })).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
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

  it("says why the model list is unavailable, in words", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      models: [],
      defaultModelId: null,
      modelUnavailableReason: "assistant_catalog_unavailable",
    });
    render(page());
    const status = await screen.findByText(/Models are unavailable/);
    expect(status).toHaveTextContent(
      "The model list couldn't be loaded. Check your connection and try again.",
    );
    expect(document.body).not.toHaveTextContent("assistant_catalog");
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
        // Written by an older build; the host itself saves only user and
        // assistant messages.
        {
          id: "internal",
          role: "tool",
          content: "assistant_request",
          createdAt: 2,
        } as unknown as AssistantMessage,
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
      // Event types this build does not know are ignored.
      mocks.listener?.({
        sessionId: "session-a",
        runId: "run-a",
        operationId: "op-a",
        type: "operation",
        data: {
          operation: { id: "op-a", title: "assistant_request" },
        },
      } as unknown as AssistantEvent);
      mocks.listener?.({
        sessionId: "session-a",
        runId: "run-a",
        operationId: "op-a",
        type: "tool_start",
        data: { name: "assistant_request" },
      } as unknown as AssistantEvent);
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
        data: { delta: " and live tail", messageId: "reply" },
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
        data: { delta: "STALE REPLY", messageId: "reply" },
      });
      mocks.listener?.({
        ...sent,
        sessionId: "another-session",
        type: "text_delta",
        data: { delta: "WRONG SESSION", messageId: "reply" },
      });
      mocks.listener?.({
        ...sent,
        type: "text_delta",
        data: { delta: "Current reply", messageId: "reply" },
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
      mocks.listener?.({
        ...sent,
        type: "text_delta",
        data: { delta: "Hi", messageId: "reply" },
      }),
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
      mocks.listener?.({
        ...sent,
        type: "run_end",
        data: { stopped: false, status: "completed" },
      }),
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
          nativeEntryId: "entry-original",
          runId: "run-original",
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
          nativeEntryId: "entry-response",
          runId: "run-response",
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

describe("a turn that failed before the model kept it", () => {
  const failedTurn = (status: "error" | "stopped" = "error") => {
    current = {
      ...current,
      messages: [
        {
          id: "lost-question",
          role: "user",
          content: "The lost question",
          createdAt: 1,
          status: "completed",
          nativeEntryId: null,
          runId: "lost-run",
        },
        {
          id: "lost-reply",
          role: "assistant",
          content: "",
          createdAt: 2,
          status,
          nativeEntryId: null,
          runId: "lost-run",
        },
      ],
    };
  };

  it("sends the stored question again as a new turn, keeping the draft", async () => {
    failedTurn();
    await openConversation();
    await userEvent.type(composer(), "Unsent draft");
    await userEvent.click(screen.getByRole("button", { name: "Send again" }));
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
    const request = mocks.send.mock.calls[0][0];
    expect(request).toMatchObject({
      sessionId: "session-a",
      text: "The lost question",
      attachmentIds: [],
    });
    expect(request.editMessageId).toBeUndefined();
    expect(request.regenerateMessageId).toBeUndefined();
    expect(request.runId).not.toBe("lost-run");
    expect(composer()).toHaveValue("Unsent draft");
    await waitFor(() => expect(current.session.draft).toBe("Unsent draft"));
  });

  it("is never offered for a turn the model has, or while a reply runs", async () => {
    failedTurn("stopped");
    current.messages[0].nativeEntryId = "native-question";
    await openConversation();
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
    expect(
      screen.getByRole("button", { name: "Edit and resend" }),
    ).toBeVisible();
  });
});

describe("attachments", () => {
  const pdf: AssistantAttachment = {
    id: "file-pdf",
    name: "report.pdf",
    mimeType: "application/pdf",
    sizeBytes: 2048,
    status: "ready",
    reason: null,
    previewUrl: null,
    details: null,
  };
  const photo: AssistantAttachment = {
    ...pdf,
    id: "file-png",
    name: "photo.png",
    mimeType: "image/png",
    sizeBytes: 3_400_000,
  };
  const locked: AssistantAttachment = {
    ...pdf,
    id: "file-locked",
    name: "locked.pdf",
    status: "unreadable",
    reason: "assistant_attachment_unreadable",
    details: { format: "pdf", reason: "password_protected" },
  };
  const footer = () => composer().closest("footer") as HTMLElement;
  const composerFiles = () =>
    within(footer()).queryByRole("list", { name: "Attached files" });
  const attach = () => screen.getByRole("button", { name: "Attach files" });
  const sendButton = () => screen.getByRole("button", { name: "Send" });
  const withDraftFiles = (
    files: AssistantAttachment[],
    draft = "",
  ): AssistantSessionDetail => ({
    ...current,
    session: {
      ...current.session,
      draft,
      draftAttachmentIds: files.map((file) => file.id),
    },
    attachments: files,
  });
  async function pick(files: AssistantAttachment[]) {
    mocks.pickAttachments.mockResolvedValueOnce(files);
    await userEvent.click(attach());
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent(files[0].name),
    );
  }

  it("attaches picked files to the draft and saves them with it", async () => {
    await openConversation();
    await pick([pdf, photo]);
    expect(mocks.pickAttachments).toHaveBeenCalledWith("session-a");
    const [first, second] = within(composerFiles()!).getAllByRole("listitem");
    expect(first).toHaveTextContent("report.pdf");
    expect(first).toHaveTextContent("2.0 KB");
    expect(second).toHaveTextContent("photo.png");
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          draftAttachmentIds: ["file-pdf", "file-png"],
        }),
      ),
    );
  });

  it("sends files without words, and shows them on the question", async () => {
    await openConversation();
    expect(sendButton()).toBeDisabled();
    await pick([pdf]);
    expect(sendButton()).toBeEnabled();
    await userEvent.click(sendButton());
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          text: "",
          attachmentIds: ["file-pdf"],
        }),
      ),
    );
    expect(composerFiles()).toBeNull();
    const question = screen.getByRole("article", { name: "You" });
    expect(
      within(question).getByRole("list", { name: "Attached files" }),
    ).toHaveTextContent("report.pdf");
  });

  it("restores a saved draft's files, and removes one from the draft", async () => {
    current = withDraftFiles([pdf]);
    await openConversation();
    expect(composerFiles()).toHaveTextContent("report.pdf");
    await userEvent.click(
      screen.getByRole("button", { name: "Remove report.pdf" }),
    );
    expect(composerFiles()).toBeNull();
    expect(sendButton()).toBeDisabled();
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          draftAttachmentIds: [],
        }),
      ),
    );
  });

  it("won't send a file that can't be read, and says why", async () => {
    current = withDraftFiles([locked], "Read this");
    await openConversation();
    expect(composerFiles()).toHaveTextContent(
      "This PDF is password-protected. Remove the password, then attach it again.",
    );
    const hint = screen.getByText(
      "Remove the files that can't be read, then send.",
    );
    expect(hint).toBeVisible();
    expect(hint.closest("[aria-live]")).toHaveAttribute("aria-live", "polite");
    expect(sendButton()).toBeDisabled();
    expect(sendButton()).toHaveAccessibleDescription(
      "Remove the files that can't be read, then send.",
    );
    fireEvent.keyDown(composer(), { key: "Enter" });
    expect(mocks.send).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "Remove locked.pdf" }),
    );
    expect(sendButton()).toBeEnabled();
    expect(
      screen.queryByText("Remove the files that can't be read, then send."),
    ).toBeNull();
  });

  it.each<[AssistantAttachment["status"], string]>([
    ["partial", "Part of this file wasn't read."],
    ["model_unsupported", "Choose a model that can read images"],
    ["too_large", "A message can carry up to 10 images."],
  ])(
    "sends a draft file an earlier turn read as %s, without that turn's words",
    async (status, note) => {
      const earlier = {
        ...photo,
        status,
        details: status === "too_large" ? { limit: "message_images" } : null,
      };
      current = withDraftFiles([earlier], "Look");
      current.session.modelId = "vision";
      mocks.bootstrap.mockResolvedValue({
        ...capabilities,
        models: [
          ...capabilities.models,
          { ...model("vision", "Vision model"), supportsImages: true },
        ],
      });
      await openConversation();
      expect(composerFiles()).toHaveTextContent("photo.png");
      expect(composerFiles()).not.toHaveTextContent(note);
      await userEvent.click(sendButton());
      await waitFor(() =>
        expect(mocks.send).toHaveBeenCalledWith(
          expect.objectContaining({ attachmentIds: ["file-png"] }),
        ),
      );
    },
  );

  describe("an image for a model that can't read images", () => {
    const warning =
      "This model can't read images. Choose a model that can, or remove the image.";
    beforeEach(() => {
      mocks.bootstrap.mockResolvedValue({
        ...capabilities,
        models: [
          ...capabilities.models,
          { ...model("vision", "Vision model"), supportsImages: true },
        ],
      });
    });

    it("is warned about before sending, and can still be sent", async () => {
      await openConversation();
      await pick([photo]);
      expect(screen.getByText(warning)).toBeVisible();
      expect(sendButton()).toBeEnabled();
      await userEvent.click(
        screen.getByRole("combobox", { name: "Choose model" }),
      );
      await userEvent.click(
        await screen.findByRole("option", { name: /Vision model/ }),
      );
      await waitFor(() => expect(screen.queryByText(warning)).toBeNull());
    });

    it("is no longer warned about once the image is removed", async () => {
      await openConversation();
      await pick([pdf, photo]);
      expect(screen.getByText(warning)).toBeVisible();
      await userEvent.click(
        screen.getByRole("button", { name: "Remove photo.png" }),
      );
      expect(screen.queryByText(warning)).toBeNull();
    });

    it("is not warned about for a model that reads images or for other files", async () => {
      current.session.modelId = "vision";
      await openConversation();
      await pick([photo]);
      expect(screen.queryByText(warning)).toBeNull();
      await userEvent.click(
        screen.getByRole("button", { name: "Remove photo.png" }),
      );
      current.session.modelId = "balanced";
      await userEvent.click(
        screen.getByRole("combobox", { name: "Choose model" }),
      );
      await userEvent.click(
        await screen.findByRole("option", { name: /Balanced model/ }),
      );
      await pick([pdf]);
      expect(screen.queryByText(warning)).toBeNull();
    });

    it("is warned about before the first message, for the model chosen", async () => {
      mocks.listSessions.mockResolvedValue(listOf([]));
      render(page());
      await waitFor(() => expect(attach()).toBeEnabled());
      await pick([photo]);
      expect(screen.getByText(warning)).toBeVisible();
    });
  });

  it("returns focus to the message after the file dialog, and after the last file is removed", async () => {
    await openConversation();
    await pick([pdf]);
    await waitFor(() => expect(composer()).toHaveFocus());
    await userEvent.click(
      screen.getByRole("button", { name: "Remove report.pdf" }),
    );
    expect(composer()).toHaveFocus();
  });

  describe("the files and the warnings about them", () => {
    // jsdom has no layout: what scrolls is the element with overflow-y-auto.
    const scrollBox = () =>
      composerFiles()!.closest<HTMLElement>(".overflow-y-auto");
    const many = ["a", "b", "c", "d", "e"].map((letter) => ({
      ...pdf,
      id: `file-${letter}`,
      name: `${letter}.pdf`,
    }));

    it("keeps the reason Send is disabled outside the scrolling list of files", async () => {
      current = withDraftFiles([...many, locked], "Read these");
      await openConversation();
      const hint = screen.getByText(
        "Remove the files that can't be read, then send.",
      );
      expect(scrollBox()).not.toBeNull();
      expect(scrollBox()).toContainElement(composerFiles());
      expect(scrollBox()).not.toContainElement(hint);
      expect(footer()).toContainElement(hint);
    });

    it("keeps the warning about images outside the scrolling list of files", async () => {
      mocks.bootstrap.mockResolvedValue({
        ...capabilities,
        models: [
          ...capabilities.models,
          { ...model("vision", "Vision model"), supportsImages: true },
        ],
      });
      current = withDraftFiles([...many, photo]);
      await openConversation();
      const warning = screen.getByText(
        "This model can't read images. Choose a model that can, or remove the image.",
      );
      expect(scrollBox()).not.toContainElement(warning);
      expect(warning).toBeVisible();
    });

    it("still announces a warning politely, and describes Send by it", async () => {
      current = withDraftFiles([...many, locked], "Read these");
      await openConversation();
      const hint = screen.getByText(
        "Remove the files that can't be read, then send.",
      );
      expect(hint.closest("[aria-live]")).toHaveAttribute(
        "aria-live",
        "polite",
      );
      expect(sendButton()).toHaveAccessibleDescription(
        "Remove the files that can't be read, then send.",
      );
    });
  });

  it("describes Send by the warning about images a model can't read", async () => {
    await openConversation();
    await pick([photo]);
    expect(sendButton()).toHaveAccessibleDescription(
      "This model can't read images. Choose a model that can, or remove the image.",
    );
  });

  it("changes nothing when the file dialog is cancelled", async () => {
    await openConversation();
    await userEvent.click(attach());
    await waitFor(() => expect(mocks.pickAttachments).toHaveBeenCalled());
    await waitFor(() => expect(attach()).toBeEnabled());
    expect(composerFiles()).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it.each([
    [
      "assistant_attachment_too_many",
      "Too many files. Attach up to 10 at a time and up to 200 per conversation.",
    ],
    [
      "assistant_attachment_too_large",
      "A file is too large. Files can be up to 25 MB",
    ],
    [
      "assistant_attachment_dialog_failed",
      "The file picker couldn't be opened. Please try again.",
    ],
  ])("explains a refused pick (%s) in words", async (code, expected) => {
    mocks.pickAttachments.mockRejectedValueOnce(code);
    await openConversation();
    await userEvent.click(attach());
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(expected);
    expect(document.body).not.toHaveTextContent("assistant_");
    expect(composerFiles()).toBeNull();
    expect(attach()).toBeEnabled();
  });

  it("starts a conversation for files picked before there is one", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    render(page());
    await waitFor(() => expect(attach()).toBeEnabled());
    mocks.pickAttachments.mockResolvedValueOnce([pdf]);
    await userEvent.click(attach());
    await waitFor(() =>
      expect(mocks.pickAttachments).toHaveBeenCalledWith("session-a"),
    );
    expect(mocks.createSession).toHaveBeenCalledOnce();
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent("report.pdf"),
    );
  });

  describe("a pick that started a conversation and imported nothing", () => {
    beforeEach(() => {
      mocks.listSessions.mockResolvedValue(listOf([]));
      mocks.createSession.mockImplementation(async (request) => ({
        ...session,
        title: "",
        ...request,
      }));
    });
    const trashed = () =>
      expect(mocks.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        deleted: true,
        expectedOwner: capabilities.owner,
        expectedGeneration: capabilities.authorityGeneration,
      });

    it("leaves no empty conversation behind when the dialog is cancelled", async () => {
      render(page());
      await waitFor(() => expect(attach()).toBeEnabled());
      await userEvent.click(screen.getByRole("button", { name: "Web search" }));
      await userEvent.type(composer(), "Compare these");
      await userEvent.click(attach());
      await waitFor(trashed);
      await waitFor(() => expect(attach()).toBeEnabled());
      // Back to no conversation, with the unsent words and search choice.
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
        "Assistant",
      );
      expect(composer()).toHaveValue("Compare these");
      expect(
        screen.getByRole("button", { name: "Web search" }),
      ).toHaveAttribute("aria-pressed", "true");
      expect(screen.queryByRole("alert")).toBeNull();
      expect(mocks.listSessions.mock.calls.length).toBeGreaterThan(1);
    });

    it("leaves no empty conversation behind when the pick is refused, and says why", async () => {
      mocks.pickAttachments.mockRejectedValueOnce(
        "assistant_attachment_too_many",
      );
      render(page());
      await waitFor(() => expect(attach()).toBeEnabled());
      await userEvent.click(attach());
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "Too many files.",
      );
      await waitFor(trashed);
      expect(screen.getByRole("alert")).toHaveTextContent("Too many files.");
      expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
        "Assistant",
      );
    });

    it("keeps a conversation that was open before the pick", async () => {
      mocks.listSessions.mockResolvedValue(listOf([current.session]));
      await openConversation();
      await userEvent.click(attach());
      await waitFor(() => expect(mocks.pickAttachments).toHaveBeenCalled());
      await waitFor(() => expect(attach()).toBeEnabled());
      expect(mocks.updateSession).not.toHaveBeenCalledWith(
        expect.objectContaining({ deleted: true }),
      );
    });
  });

  it("saves picked files at once, so a page closed right away keeps them", async () => {
    const view = render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() => expect(attach()).toBeEnabled());
    mocks.pickAttachments.mockResolvedValueOnce([pdf]);
    await userEvent.click(attach());
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent("report.pdf"),
    );
    // Closed well before the draft's usual 600 ms pause, which unmounting
    // cancels: only a save made at once can still arrive.
    view.unmount();
    await waitFor(
      () =>
        expect(mocks.updateSession).toHaveBeenCalledWith(
          expect.objectContaining({
            sessionId: "session-a",
            draftAttachmentIds: ["file-pdf"],
          }),
        ),
      { timeout: 2000 },
    );
  });

  it("keeps each conversation's files with its own draft", async () => {
    const other = { ...session, id: "session-b", title: "Other conversation" };
    mocks.listSessions.mockResolvedValue(listOf([session, other]));
    mocks.getSession.mockImplementation(async (id: string) =>
      id === other.id
        ? { ...current, session: other, attachments: [] }
        : current,
    );
    await openConversation();
    await pick([pdf]);
    await userEvent.click(
      screen.getByRole("button", { name: /Other conversation/ }),
    );
    await waitFor(() => expect(composer()).toBeEnabled());
    expect(composerFiles()).toBeNull();
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          draftAttachmentIds: ["file-pdf"],
        }),
      ),
    );
    await userEvent.click(
      screen.getByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent("report.pdf"),
    );
  });

  it("puts the files back in the composer when a send fails", async () => {
    mocks.send.mockRejectedValueOnce("assistant_network_error");
    await openConversation();
    await pick([pdf]);
    await userEvent.type(composer(), "Summarize");
    await userEvent.click(sendButton());
    await screen.findByRole("alert");
    expect(composerFiles()).toHaveTextContent("report.pdf");
    expect(composer()).toHaveValue("Summarize");
    expect(screen.queryByRole("article", { name: "You" })).toBeNull();
  });

  it("can't attach files while a reply runs", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(sendButton());
    await screen.findByRole("button", { name: "Stop" });
    expect(attach()).toBeDisabled();
  });

  it("shows each file's outcome as the run reads it, and why the turn was blocked", async () => {
    await openConversation();
    await pick([photo]);
    await userEvent.type(composer(), "What is this?");
    await userEvent.click(sendButton());
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "attachment",
        data: {
          id: "file-png",
          status: "model_unsupported",
          code: "assistant_vision_model_required",
          details: { format: "image", mimeType: "image/png" },
        },
      }),
    );
    const question = screen.getByRole("article", { name: "You" });
    expect(
      within(question).getByRole("list", { name: "Attached files" }),
    ).toHaveTextContent(
      "This model can't read images. Choose a model that can read images, then try again.",
    );
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "error",
        data: {
          code: "assistant_vision_model_required",
          message:
            "assistant_vision_model_required: This model cannot read images",
        },
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This model can't read images. Choose a model that can read images, then send the message again.",
    );
    expect(document.body).not.toHaveTextContent("assistant_");
  });

  it("says it is reading the files until each one's outcome is in", async () => {
    await openConversation();
    await pick([pdf, photo]);
    await userEvent.click(sendButton());
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    const reply = () => screen.getByRole("article", { name: "Assistant" });
    expect(within(reply()).getByRole("status")).toHaveTextContent(
      "Reading a file…",
    );
    const read = (id: string) =>
      act(() =>
        mocks.listener?.({
          ...sent,
          type: "attachment",
          data: { id, status: "ready", code: null, details: null },
        }),
      );
    read("file-pdf");
    expect(within(reply()).getByRole("status")).toHaveTextContent(
      "Reading a file…",
    );
    read("file-png");
    expect(within(reply()).getByRole("status")).toHaveTextContent("Thinking…");
  });

  describe("stops saying it reads the files once the model is at work, even without their outcome", () => {
    async function sendFiles() {
      await openConversation();
      await pick([pdf]);
      await userEvent.click(sendButton());
      await screen.findByRole("button", { name: "Stop" });
      const sent = mocks.send.mock.calls[0][0];
      const emit = (type: string, data: Record<string, unknown>) =>
        act(() => mocks.listener?.({ ...sent, type, data }));
      expect(
        within(screen.getByRole("article", { name: "Assistant" })).getByRole(
          "status",
        ),
      ).toHaveTextContent("Reading a file…");
      return emit;
    }

    it.each(["text_delta", "thinking_delta"])("on a %s", async (type) => {
      const emit = await sendFiles();
      emit(type, { messageId: "reply", delta: "Let me see" });
      expect(screen.queryByText("Reading a file…")).toBeNull();
    });

    it("on a tool_start, also after the tool ends", async () => {
      const emit = await sendFiles();
      emit("tool_start", {
        messageId: "reply",
        name: "web_search",
        toolCallId: "c1",
        args: { query: "x" },
      });
      expect(screen.getByRole("status")).toHaveTextContent(
        "Searching the web…",
      );
      emit("tool_end", {
        messageId: "reply",
        name: "web_search",
        toolCallId: "c1",
        success: true,
        details: {},
      });
      expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
    });
  });

  it("says it is reading the files of a reply being regenerated", async () => {
    current.messages = [
      {
        id: "q",
        role: "user",
        content: "Summarize",
        createdAt: 1,
        nativeEntryId: "entry-q",
        runId: "run-1",
        attachments: [pdf],
      },
      {
        id: "a",
        role: "assistant",
        content: "Summary",
        createdAt: 2,
        status: "completed",
        nativeEntryId: "entry-a",
        runId: "run-1",
      },
    ];
    await openConversation();
    await userEvent.click(screen.getByRole("button", { name: "Regenerate" }));
    await screen.findByRole("button", { name: "Stop" });
    const streaming = screen.getAllByRole("article", { name: "Assistant" });
    expect(
      within(streaming[streaming.length - 1]).getByRole("status"),
    ).toHaveTextContent("Reading a file…");
  });

  it("ignores the outcome of a run this view does not follow", async () => {
    await openConversation();
    await pick([photo]);
    await userEvent.click(sendButton());
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({
        ...sent,
        runId: "another-run",
        type: "attachment",
        data: {
          id: "file-png",
          status: "unreadable",
          code: "assistant_attachment_unreadable",
          details: { reason: "invalid_image" },
        },
      }),
    );
    expect(screen.getByRole("article", { name: "You" })).not.toHaveTextContent(
      "This image is damaged",
    );
  });

  it("sends a failed question again with the files that can still be read", async () => {
    current.messages = [
      {
        id: "lost-question",
        role: "user",
        content: "Compare these",
        createdAt: 1,
        status: "completed",
        nativeEntryId: null,
        runId: "lost-run",
        attachments: [pdf, locked],
      },
      {
        id: "lost-reply",
        role: "assistant",
        content: "",
        createdAt: 2,
        status: "error",
        nativeEntryId: null,
        runId: "lost-run",
      },
    ];
    await openConversation();
    await userEvent.click(screen.getByRole("button", { name: "Send again" }));
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({
          text: "Compare these",
          attachmentIds: ["file-pdf"],
        }),
      ),
    );
  });

  it("edits a question and sends again the files that can be read", async () => {
    current.messages = [
      {
        id: "original",
        role: "user",
        content: "Compare these",
        createdAt: 1,
        nativeEntryId: "entry-original",
        runId: "run-original",
        attachments: [pdf, locked],
      },
    ];
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Edit and resend" }), {
      target: { value: "Compare again" },
    });
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({
          editMessageId: "original",
          text: "Compare again",
          attachmentIds: ["file-pdf"],
        }),
      ),
    );
  });

  it("sends an explicit empty attachment selection when every file is removed from an edit", async () => {
    current.messages = [
      {
        id: "original",
        role: "user",
        content: "Text only now",
        createdAt: 1,
        nativeEntryId: "entry-original",
        runId: "run-original",
        attachments: [pdf],
      },
    ];
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: `Remove ${pdf.name}` }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    await waitFor(() =>
      expect(mocks.send).toHaveBeenCalledWith(
        expect.objectContaining({
          editMessageId: "original",
          text: "Text only now",
          attachmentIds: [],
          replaceAttachments: true,
        }),
      ),
    );
  });
});

describe("drafts and fencing", () => {
  it("does not revive a finished run when returning during its final draft save", async () => {
    current.session = { ...session, draft: "Question" };
    let release!: () => void;
    const waiting = new Promise<void>((resolve) => {
      release = resolve;
    });
    mocks.updateSession.mockImplementation(async (patch) => {
      if (patch.draft === "") await waiting;
      current.session = { ...current.session, ...patch };
      return current.session;
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    let sent!: Promise<boolean>;
    act(() => {
      sent = hook.result.current.send();
    });
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({ draft: "" }),
      ),
    );
    // The backend has already finished while a post-admission write waits.
    current.session.activeRun = null;
    act(() => hook.result.current.select(null));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    await act(async () => {
      release();
      await sent;
    });
    expect(hook.result.current.run).toBeNull();
    expect(hook.result.current.busy).toBe(false);
  });

  it("applies a committed setting and reports its separate cleanup warning", async () => {
    mocks.updateSession.mockImplementation(async (patch) => ({
      ...session,
      ...patch,
      cleanupWarning: "assistant_attachment_cleanup_failed",
    }));
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    await act(async () => {
      await hook.result.current.update({ webSearch: true });
    });
    expect(hook.result.current.webSearch).toBe(true);
    expect(hook.result.current.error).toBe(
      "assistant_attachment_cleanup_failed",
    );
  });

  const draftFile: AssistantAttachment = {
    id: "draft-file",
    name: "notes.txt",
    mimeType: "text/plain",
    sizeBytes: 12,
    status: "ready",
    reason: null,
    details: null,
  };

  it.each([false, true])(
    "flushes a newer draft after a held send fails (cancelled=%s)",
    async (cancelled) => {
      current.session = {
        ...session,
        draft: "Original",
        draftAttachmentIds: [draftFile.id],
      };
      current.attachments = [draftFile];
      let release!: () => void;
      const waiting = new Promise<void>((resolve) => {
        release = resolve;
      });
      mocks.send.mockImplementation(async () => {
        await waiting;
        throw new Error("assistant_cancelled");
      });
      const hook = renderHook(() => useAssistant(true));
      await waitFor(() => expect(hook.result.current.connected).toBe(true));
      act(() => hook.result.current.select(session.id));
      await waitFor(() =>
        expect(hook.result.current.attachments).toHaveLength(1),
      );
      let sent!: Promise<boolean>;
      act(() => {
        sent = hook.result.current.send();
      });
      await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
      act(() => hook.result.current.setDraft("Newer draft"));
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 700));
      });
      if (cancelled) await act(async () => hook.result.current.stop());
      await act(async () => {
        release();
        await sent;
      });
      expect(current.session.draft).toBe("Newer draft");
      expect(current.session.draftAttachmentIds).toEqual([draftFile.id]);
    },
  );

  it.each(["import", "failed send"])(
    "restores the displayed draft after leaving and returning during %s",
    async (operation) => {
      current.session = {
        ...session,
        draft: "Question",
        draftAttachmentIds: [draftFile.id],
      };
      current.attachments = [draftFile];
      const added = { ...draftFile, id: "added-file", name: "added.txt" };
      let release!: () => void;
      const waiting = new Promise<void>((resolve) => {
        release = resolve;
      });
      mocks.pickAttachments.mockImplementation(async () => {
        await waiting;
        return [added];
      });
      mocks.send.mockImplementation(async () => {
        await waiting;
        throw new Error("assistant_key_unavailable");
      });
      const hook = renderHook(() => useAssistant(true));
      await waitFor(() => expect(hook.result.current.connected).toBe(true));
      act(() => hook.result.current.select(session.id));
      await waitFor(() =>
        expect(hook.result.current.attachments).toHaveLength(1),
      );
      let operationDone!: Promise<unknown>;
      act(() => {
        operationDone =
          operation === "import"
            ? hook.result.current.pickAttachments()
            : hook.result.current.send();
      });
      await waitFor(() =>
        expect(
          operation === "import" ? mocks.pickAttachments : mocks.send,
        ).toHaveBeenCalledOnce(),
      );
      act(() => hook.result.current.select(null));
      act(() => hook.result.current.select(session.id));
      await waitFor(() => expect(hook.result.current.loading).toBe(false));
      expect(hook.result.current.busy).toBe(true);
      await act(async () => {
        release();
        await operationDone;
      });
      const expected =
        operation === "import" ? [draftFile.id, added.id] : [draftFile.id];
      expect(hook.result.current.attachments.map((file) => file.id)).toEqual(
        expected,
      );
      expect(hook.result.current.busy).toBe(false);
      act(() => hook.result.current.setDraft("Next question"));
      await waitFor(() => expect(current.session.draft).toBe("Next question"));
      expect(current.session.draftAttachmentIds).toEqual(expected);
    },
  );

  it.each([false, true])(
    "keeps a sending file while admission waits (leave=%s)",
    async (leave) => {
      current.session = {
        ...session,
        draft: "Question",
        draftAttachmentIds: [draftFile.id],
      };
      current.attachments = [draftFile];
      let release!: () => void;
      const admission = new Promise<void>((resolve) => {
        release = resolve;
      });
      let exists = true;
      let published = false;
      mocks.updateSession.mockImplementation(async (patch) => {
        if (patch.draftAttachmentIds?.length === 0 && !published)
          exists = false;
        current.session = { ...current.session, ...patch };
        return current.session;
      });
      mocks.send.mockImplementation(async () => {
        await admission;
        if (!exists) throw new Error("assistant_attachment_not_found");
        published = true;
        return { runId: "accepted", operationId: "accepted" };
      });
      const hook = renderHook(() => useAssistant(true));
      await waitFor(() => expect(hook.result.current.connected).toBe(true));
      act(() => hook.result.current.select(session.id));
      await waitFor(() =>
        expect(hook.result.current.attachments).toHaveLength(1),
      );
      let submission!: Promise<boolean>;
      act(() => {
        submission = hook.result.current.send();
      });
      await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
      if (leave) act(() => hook.result.current.select(null));
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 700));
      });
      const survived = exists;
      let sent = false;
      await act(async () => {
        release();
        sent = await submission;
      });
      expect(survived).toBe(true);
      expect(sent).toBe(true);
    },
  );

  it("flushes the removed draft before import and never reapplies it to a new file", async () => {
    current.session = { ...session, draftAttachmentIds: [draftFile.id] };
    current.attachments = [draftFile];
    const added = { ...draftFile, id: "new-file", name: "new.txt" };
    let release!: () => void;
    const importing = new Promise<void>((resolve) => {
      release = resolve;
    });
    let copying = false;
    let exists = false;
    mocks.pickAttachments.mockImplementation(async () => {
      copying = true;
      await importing;
      exists = true;
      current.session.draftAttachmentIds = [added.id];
      return [added];
    });
    mocks.updateSession.mockImplementation(async (patch) => {
      if (copying) await importing;
      if (patch.draftAttachmentIds?.length === 0) exists = false;
      current.session = { ...current.session, ...patch };
      return current.session;
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    act(() => hook.result.current.select(session.id));
    await waitFor(() =>
      expect(hook.result.current.attachments).toHaveLength(1),
    );
    act(() => hook.result.current.removeAttachment(draftFile.id));
    let pick!: Promise<void>;
    act(() => {
      pick = hook.result.current.pickAttachments();
    });
    await waitFor(() => expect(mocks.pickAttachments).toHaveBeenCalledOnce());
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 700));
    });
    await act(async () => {
      release();
      await pick;
    });
    expect(exists).toBe(true);
    expect(hook.result.current.attachments.map((file) => file.id)).toEqual([
      added.id,
    ]);
  });

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

  it("refreshes a background conversation's Replying… mark on its run_end", async () => {
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
        type: "run_end",
        data: { stopped: false, status: "completed" },
      }),
    );
    await waitFor(() =>
      expect(within(list).queryByText("Replying…")).toBeNull(),
    );
  });

  it("describes a row the backend could not list by its own code", async () => {
    mocks.listSessions.mockResolvedValue(
      listOf([], [{ id: "x", error: "assistant_storage_unavailable" }]),
    );
    render(page());
    const list = await screen.findByRole("list", { name: "Conversations" });
    await within(list).findByText("Can't be read");
    expect(list).toHaveTextContent(
      "Local assistant data couldn't be read or saved.",
    );
    expect(list).not.toHaveTextContent("damaged");
  });

  it("dates the history in the app's language, not the system's, on hover", async () => {
    // A pinned zone where this moment is already the next day.
    vi.stubEnv("TZ", "Asia/Shanghai");
    const stamp = Date.UTC(2026, 9, 15, 20);
    mocks.listSessions.mockResolvedValue(
      listOf([{ ...session, updatedAt: stamp }]),
    );
    i18n.addResourceBundle("zh", "translation", zh, true, true);
    await i18n.changeLanguage("zh");
    render(page());
    const row = await screen.findByRole("button", { name: "My conversation" });
    // One line: the title, with the full time on hover.
    expect(row).toHaveTextContent(/^My conversation$/);
    expect(row).toHaveAttribute("title", "2026年10月16日 04:00");
  });

  it("marks a conversation that is replying with a spinner that says so", async () => {
    mocks.listSessions.mockResolvedValue(
      listOf([{ ...session, activeRun: { runId: "r", operationId: "o" } }]),
    );
    render(page());
    const row = await screen.findByRole("button", {
      name: "My conversation Replying…",
    });
    expect(row.querySelector("svg.animate-spin")).not.toBeNull();
  });

  it("loads a conversation again when its row is clicked after its load failed", async () => {
    mocks.getSession.mockRejectedValueOnce("assistant_storage_unavailable");
    render(page());
    const row = await screen.findByRole("button", { name: /My conversation/ });
    await userEvent.click(row);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Local assistant data couldn't be read or saved.",
    );
    current.messages = [
      { id: "q", role: "user", content: "Question", createdAt: 1 },
    ];
    await userEvent.click(row);
    expect(await screen.findByText("Question")).toBeVisible();
    expect(mocks.getSession).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("alert")).toBeNull();
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
        data: { delta: "Answer", messageId: "reply" },
      }),
    );
    expect(hook.result.current.stream?.content).toBe("Answer");
  });

  it("keeps a registered run when a read from before its registration answers after the send returned", async () => {
    let accept!: (value: AssistantRunRef) => void;
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
    // Read while the backend has not registered the run yet ...
    let answer!: (value: AssistantSessionDetail) => void;
    mocks.getSession.mockReturnValueOnce(
      new Promise((resolve) => {
        answer = resolve;
      }),
    );
    let reading!: Promise<boolean>;
    act(() => {
      reading = hook.result.current.refreshDetail(session.id);
    });
    // ... answered only after the send returned.
    await act(async () => {
      accept(run);
      await submission;
    });
    await act(async () => {
      answer({ ...current, session: { ...current.session, activeRun: null } });
      await reading;
    });
    expect(hook.result.current.run).toEqual(run);
    act(() =>
      mocks.listener?.({
        sessionId: session.id,
        ...run,
        type: "text_delta",
        data: { delta: "Answer", messageId: "reply" },
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
          } as unknown as AssistantMessage,
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

describe("conversation cost", () => {
  // Scope the assertion to this conversation, apart from the new all-conversation card.
  const usageLine = () => screen.queryByLabelText("This conversation");

  it("shows a saved conversation's tokens and estimated cost", async () => {
    current.usage = {
      inputTokens: 1234,
      outputTokens: 340,
      cacheReadTokens: 0,
      cacheWriteTokens: 0,
      estimatedCostUsd: "0.0123",
      billedCostUsd: null,
      modelRequests: 2,
    };
    await openConversation();
    expect(usageLine()).toHaveTextContent(
      "Input 1.2K · Output 340 tokens · Estimated cost ≈$0.012",
    );
  });

  it.each([
    [
      "a request that may have been billed",
      {
        inputTokens: 900,
        outputTokens: 20,
        estimatedCostUsd: null,
        usageCoverage: "incomplete" as const,
        modelRequests: 1,
      },
    ],
    [
      "a model without a price",
      {
        inputTokens: 900,
        outputTokens: 20,
        estimatedCostUsd: null,
        modelRequests: 1,
      },
    ],
    [
      "a reply stopped before it reported usage",
      { usageCoverage: "incomplete" as const },
    ],
  ])("says the cost is unavailable after %s, never $0", async (_, usage) => {
    current.usage = usage;
    await openConversation();
    expect(usageLine()).toHaveTextContent("Cost unavailable");
    expect(usageLine()).not.toHaveTextContent("$");
  });

  it.each([
    // searchCount counts billed search calls: one search can make two.
    [2, "en", "Estimated cost ≈$0.040 incl. 2 search charges"],
    [1, "en", "Estimated cost ≈$0.040 incl. 1 search charge"],
    [2, "ja", "推定コスト ≈$0.040（検索料金 2 回分を含む）"],
    [1, "ja", "推定コスト ≈$0.040（検索料金 1 回分を含む）"],
    [2, "zh", "预估成本 ≈$0.040（含 2 次搜索计费）"],
    [1, "zh", "预估成本 ≈$0.040（含 1 次搜索计费）"],
  ])(
    "names the search charges a saved cost includes, for a count of %i, in %s",
    async (searchCount, language, expected) => {
      current.usage = {
        inputTokens: 1234,
        outputTokens: 340,
        estimatedCostUsd: "0.0402",
        searchCount,
        searchCostUsd: "0.028",
        modelRequests: 2,
      };
      i18n.addResourceBundle("zh", "translation", zh, true, true);
      i18n.addResourceBundle("ja", "translation", ja, true, true);
      await i18n.changeLanguage(language);
      render(page());
      await userEvent.click(
        await screen.findByRole("button", { name: /My conversation/ }),
      );
      // After the tokens, on the same line.
      const line = await screen.findByText(expected, { exact: false });
      expect(line.textContent?.endsWith(` · ${expected}`)).toBe(true);
    },
  );

  it("says only that the cost is unavailable when a search's outcome is unknown", async () => {
    current.usage = {
      inputTokens: 900,
      outputTokens: 20,
      estimatedCostUsd: null,
      searchCount: 1,
      searchCostUsd: null,
      usageCoverage: "incomplete",
      modelRequests: 1,
    };
    await openConversation();
    expect(usageLine()).toHaveTextContent(
      "Input 900 · Output 20 tokens · Cost unavailable",
    );
    expect(usageLine()).not.toHaveTextContent(/search|\$/);
  });

  it("says nothing about cost before the first request", async () => {
    await openConversation();
    expect(usageLine()).toBeNull();
    expect(screen.queryByText(/Cost/)).toBeNull();
  });

  it("shows running tokens without a cost while a reply runs, then the saved outcome", async () => {
    current.usage = {
      inputTokens: 100,
      outputTokens: 10,
      estimatedCostUsd: "0.01",
      modelRequests: 1,
    };
    await openConversation();
    expect(usageLine()).toHaveTextContent("Estimated cost ≈$0.010");
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "usage",
        data: {
          usage: {
            inputTokens: 300,
            outputTokens: 50,
            estimatedCostUsd: null,
            billedCostUsd: null,
            modelRequests: 1,
          },
        },
      }),
    );
    expect(usageLine()).toHaveTextContent("Input 300 · Output 50 tokens");
    expect(usageLine()).not.toHaveTextContent(/Cost|\$/);
    // The user stops it: the host marks the cost unknown.
    current.usage = {
      inputTokens: 300,
      outputTokens: 50,
      estimatedCostUsd: null,
      usageCoverage: "incomplete",
      modelRequests: 1,
    };
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "run_end",
        data: { stopped: true, status: "stopped" },
      }),
    );
    await waitFor(() =>
      expect(usageLine()).toHaveTextContent(
        "Input 300 · Output 50 tokens · Cost unavailable",
      ),
    );
  });
});

describe("web search", () => {
  const toggle = () => screen.getByRole("button", { name: "Web search" });

  it("turns web search on and off for the open conversation", async () => {
    await openConversation();
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
    await userEvent.click(toggle());
    await waitFor(() =>
      expect(toggle()).toHaveAttribute("aria-pressed", "true"),
    );
    expect(mocks.updateSession).toHaveBeenLastCalledWith({
      sessionId: "session-a",
      webSearch: true,
      expectedOwner: capabilities.owner,
      expectedGeneration: capabilities.authorityGeneration,
    });
    await userEvent.click(toggle());
    await waitFor(() =>
      expect(toggle()).toHaveAttribute("aria-pressed", "false"),
    );
    expect(mocks.updateSession).toHaveBeenLastCalledWith(
      expect.objectContaining({ sessionId: "session-a", webSearch: false }),
    );
  });

  it("shows a conversation's saved setting", async () => {
    current.session.webSearch = true;
    await openConversation();
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
  });

  it.each([
    [
      "en",
      "About $0.014–$0.028 per search, plus a little for its tokens. Up to 3 searches per reply; one search can bring back several sources.",
    ],
    [
      "ja",
      "検索 1 回あたり約 $0.014〜$0.028（別途わずかなトークン料金）。1 回の返信で検索は最大 3 回で、1 回の検索で複数の出典が見つかることがあります。",
    ],
    [
      "zh",
      "每次搜索约 $0.014–$0.028，另加少量 Token 费用。每次回复最多搜索 3 次，一次搜索可能带回多个来源。",
    ],
  ])(
    "always shows what a search may cost, one or two billed calls, and how many a reply makes at most, in %s",
    async (language, price) => {
      i18n.addResourceBundle("zh", "translation", zh, true, true);
      i18n.addResourceBundle("ja", "translation", ja, true, true);
      await i18n.changeLanguage(language);
      render(page());
      await userEvent.click(
        await screen.findByRole("button", { name: /My conversation/ }),
      );
      const button = await screen.findByRole("button", {
        name: language === "en" ? "Web search" : /Web|联网/,
      });
      await waitFor(() => expect(button).toBeEnabled());
      expect(button).toHaveAccessibleDescription(price);
      expect(screen.getByText(price)).toBeVisible();
      await userEvent.click(button);
      await waitFor(() =>
        expect(button).toHaveAttribute("aria-pressed", "true"),
      );
      expect(screen.getByText(price)).toBeVisible();
    },
  );

  it("starts the first conversation with web search when it was turned on before the first message", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    render(page());
    await waitFor(() => expect(toggle()).toBeEnabled());
    await userEvent.click(toggle());
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    expect(mocks.createSession).not.toHaveBeenCalled();
    expect(mocks.updateSession).not.toHaveBeenCalled();
    mocks.createSession.mockImplementation(async (request) => ({
      ...session,
      ...request,
    }));
    await userEvent.type(composer(), "What's new today?");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() =>
      expect(mocks.createSession).toHaveBeenCalledWith({
        modelId: "balanced",
        webSearch: true,
      }),
    );
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
  });

  it("leaves web search off in a new conversation until the switch is used", async () => {
    current.session.webSearch = true;
    await openConversation();
    mocks.createSession.mockImplementation(async (request) => ({
      ...session,
      id: "session-b",
      ...request,
    }));
    await userEvent.click(
      screen.getByRole("button", { name: "New conversation" }),
    );
    await waitFor(() =>
      expect(mocks.createSession).toHaveBeenCalledWith({
        modelId: "balanced",
        webSearch: false,
      }),
    );
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
  });

  describe("for a new conversation", () => {
    const newConversation = async () => {
      mocks.createSession.mockImplementation(async (request) => ({
        ...session,
        id: "session-b",
        ...request,
      }));
      await userEvent.click(
        screen.getByRole("button", { name: "New conversation" }),
      );
    };

    it.each([true, false])(
      "starts with the last choice made on the switch (%s)",
      async (on) => {
        current.session.webSearch = !on;
        await openConversation();
        await userEvent.click(toggle());
        await waitFor(() =>
          expect(toggle()).toHaveAttribute("aria-pressed", String(on)),
        );
        await newConversation();
        await waitFor(() =>
          expect(mocks.createSession).toHaveBeenCalledWith({
            modelId: "balanced",
            webSearch: on,
          }),
        );
        expect(toggle()).toHaveAttribute("aria-pressed", String(on));
      },
    );

    it("does not take a refused change as the last choice", async () => {
      await openConversation();
      mocks.updateSession.mockRejectedValueOnce(
        "assistant_storage_unavailable",
      );
      await userEvent.click(toggle());
      await screen.findByRole("alert");
      expect(toggle()).toHaveAttribute("aria-pressed", "false");
      await newConversation();
      await waitFor(() =>
        expect(mocks.createSession).toHaveBeenCalledWith({
          modelId: "balanced",
          webSearch: false,
        }),
      );
      expect(toggle()).toHaveAttribute("aria-pressed", "false");
    });

    it("starts with search off again once the account changed", async () => {
      await openConversation();
      await userEvent.click(toggle());
      await waitFor(() =>
        expect(toggle()).toHaveAttribute("aria-pressed", "true"),
      );
      act(() => mocks.auth?.("expired"));
      act(() => mocks.auth?.("restored"));
      await screen.findByRole("button", { name: /My conversation/ });
      await waitFor(() => expect(toggle()).toBeEnabled());
      expect(toggle()).toHaveAttribute("aria-pressed", "false");
      // Leaving a conversation for a new one starts from the last choice:
      // the old account's must be gone, not only the switch's state.
      await userEvent.click(
        screen.getByRole("button", { name: /My conversation/ }),
      );
      await waitFor(() => expect(composer()).toBeEnabled());
      await newConversation();
      await waitFor(() =>
        expect(mocks.createSession).toHaveBeenCalledWith({
          modelId: "balanced",
          webSearch: false,
        }),
      );
    });
  });

  it("can be changed while a reply runs, which goes on", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    expect(toggle()).toBeEnabled();
    await userEvent.click(toggle());
    await waitFor(() =>
      expect(toggle()).toHaveAttribute("aria-pressed", "true"),
    );
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeVisible();
  });

  describe("for the reply being written", () => {
    const skipped =
      "Web search was unavailable for this reply, so it uses the model's own knowledge.";
    const nextMessage = "Your web search change applies to your next message.";
    async function sendWithSearch(on: boolean, got: boolean) {
      current.session.webSearch = on;
      await openConversation();
      await userEvent.type(composer(), "What's new?");
      await userEvent.click(screen.getByRole("button", { name: "Send" }));
      await screen.findByRole("button", { name: "Stop" });
      const sent = mocks.send.mock.calls[0][0];
      act(() =>
        mocks.listener?.({
          ...sent,
          type: "run_start",
          data: { messageId: "reply", webSearch: got },
        }),
      );
      return sent;
    }
    const end = (sent: AssistantRunRef & { sessionId: string }) => {
      current = {
        ...current,
        messages: [
          { id: "q", role: "user", content: "What's new?", createdAt: 1 },
          {
            id: "reply",
            role: "assistant",
            content: "Nothing yet.",
            status: "completed",
            createdAt: 2,
            runId: sent.runId,
          },
        ],
      };
      act(() =>
        mocks.listener?.({
          ...sent,
          type: "run_end",
          data: { stopped: false, status: "completed" },
        }),
      );
    };

    it("says it ran without web search when search was unavailable", async () => {
      const sent = await sendWithSearch(true, false);
      expect(screen.getByText(skipped)).toBeVisible();
      expect(screen.queryByText(nextMessage)).toBeNull();
      end(sent);
      expect(await screen.findByText("Nothing yet.")).toBeVisible();
      await waitFor(() =>
        expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(),
      );
      expect(
        within(screen.getByRole("article", { name: "Assistant" })).getByText(
          skipped,
        ),
      ).toBeVisible();
    });

    it.each([
      [true, true],
      [false, false],
    ])(
      "says nothing more when the reply got what was asked (search on: %s)",
      async (on, got) => {
        const sent = await sendWithSearch(on, got);
        act(() =>
          mocks.listener?.({
            ...sent,
            type: "text_delta",
            data: { delta: "Working on it", messageId: "reply" },
          }),
        );
        expect(screen.getByText("Working on it")).toBeVisible();
        expect(screen.queryByText(skipped)).toBeNull();
        expect(screen.queryByText(nextMessage)).toBeNull();
      },
    );

    it.each([
      ["on", false, false],
      ["off", true, true],
    ])(
      "says turning search %s while it runs applies to the next message",
      async (_, on, got) => {
        const sent = await sendWithSearch(on, got);
        await userEvent.click(toggle());
        expect(await screen.findByText(nextMessage)).toBeVisible();
        await userEvent.click(toggle());
        await waitFor(() => expect(screen.queryByText(nextMessage)).toBeNull());
        await userEvent.click(toggle());
        expect(await screen.findByText(nextMessage)).toBeVisible();
        end(sent);
        await waitFor(() =>
          expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(),
        );
        expect(screen.queryByText(nextMessage)).toBeNull();
      },
    );

    it("needs no next-message note for turning off search the reply never got", async () => {
      await sendWithSearch(true, false);
      await userEvent.click(toggle());
      await waitFor(() =>
        expect(toggle()).toHaveAttribute("aria-pressed", "false"),
      );
      expect(screen.queryByText(nextMessage)).toBeNull();
    });

    it("still says so after another conversation was open", async () => {
      const other = {
        ...session,
        id: "session-b",
        title: "Other conversation",
      };
      mocks.listSessions.mockResolvedValue(listOf([current.session, other]));
      const sent = await sendWithSearch(true, false);
      end(sent);
      await screen.findByText("Nothing yet.");
      const saved = current;
      mocks.getSession.mockImplementation(async (id: string) =>
        id === other.id ? { ...saved, session: other, messages: [] } : saved,
      );
      await userEvent.click(
        screen.getByRole("button", { name: /Other conversation/ }),
      );
      await userEvent.click(
        screen.getByRole("button", { name: /My conversation/ }),
      );
      expect(await screen.findByText("Nothing yet.")).toBeVisible();
      expect(screen.getByText(skipped)).toBeVisible();
    });
  });

  it("shows a search's sources as they arrive, and the saved ones once the reply ends", async () => {
    current.session.webSearch = true;
    await openConversation();
    await userEvent.type(composer(), "What's new in Pi?");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    const notes = {
      id: "s1",
      title: "Pi release notes",
      url: "https://pi.dev/releases",
      excerpt: "Pi 2 is out.",
    };
    const changelog = {
      id: "s2",
      title: "Pi changelog",
      url: "https://pi.dev/changelog",
      excerpt: "",
    };
    const search = { messageId: "reply", name: "web_search", toolCallId: "c1" };
    act(() => {
      mocks.listener?.({
        ...sent,
        type: "tool_start",
        data: { ...search, args: { query: "pi release" } },
      });
      mocks.listener?.({
        ...sent,
        type: "sources",
        data: { messageId: "reply", sources: [notes] },
      });
    });
    // Nothing written yet cites them: they are offered behind a disclosure.
    expect(screen.getByRole("button", { name: "Show 1 source" })).toBeVisible();
    // Each event holds all of the reply's sources so far.
    act(() => {
      mocks.listener?.({
        ...sent,
        type: "sources",
        data: { messageId: "reply", sources: [notes, changelog] },
      });
      mocks.listener?.({
        ...sent,
        type: "text_delta",
        data: {
          messageId: "reply",
          delta: "Pi 2 is out ([notes](https://pi.dev/releases)).",
        },
      });
      mocks.listener?.({
        ...sent,
        type: "tool_end",
        data: { ...search, success: true, code: null },
      });
    });
    expect(
      within(screen.getByRole("list", { name: "Sources" }))
        .getAllByRole("button")
        .map((link) => link.textContent),
    ).toEqual(["1. Pi release notes pi.dev"]);
    expect(screen.getByRole("button", { name: "1 more source" })).toBeVisible();
    current = {
      ...current,
      messages: [
        { id: "q", role: "user", content: "What's new in Pi?", createdAt: 1 },
        {
          id: "reply",
          role: "assistant",
          content:
            "Pi 2 is out ([notes](https://pi.dev/releases), [log](https://pi.dev/changelog)).",
          status: "completed",
          createdAt: 2,
          sources: [notes, changelog],
        },
      ],
    };
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "run_end",
        data: { stopped: false, status: "completed" },
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(),
    );
    await waitFor(() =>
      expect(screen.getByRole("link", { name: "log" })).toBeVisible(),
    );
    expect(screen.getAllByRole("list", { name: "Sources" })).toHaveLength(1);
    expect(
      within(
        screen.getByRole("region", { name: "Assistant conversation" }),
      ).queryByRole("button", { expanded: false }),
    ).toBeNull();
    expect(
      screen.getByRole("button", { name: "2. Pi changelog pi.dev" }),
    ).toBeVisible();
  });

  it("says what the reply is doing while a tool runs, never naming the tool", async () => {
    current.session.webSearch = true;
    await openConversation();
    await userEvent.type(composer(), "Compare these pages");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    // The progress lines on screen (each message's copy button has a
    // status of its own).
    const status = () =>
      [
        "Thinking…",
        "Searching the web…",
        "Reading a page…",
        "Reading a file…",
      ].filter((text) => screen.queryByText(text));
    const tool = (name: string, toolCallId: string) => ({
      messageId: "reply",
      name,
      toolCallId,
    });
    const start = (name: string, id: string, args = {}) =>
      act(() =>
        mocks.listener?.({
          ...sent,
          type: "tool_start",
          data: { ...tool(name, id), args },
        }),
      );
    const end = (name: string, id: string, code: string | null = null) =>
      act(() =>
        mocks.listener?.({
          ...sent,
          type: "tool_end",
          data: { ...tool(name, id), success: code === null, code },
        }),
      );
    start("web_search", "c1", { query: "pi release" });
    expect(status()).toEqual(["Searching the web…"]);
    end("web_search", "c1", "search_failed");
    expect(status()).toEqual(["Thinking…"]);
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "text_delta",
        data: { delta: "Let me read them.", messageId: "reply" },
      }),
    );
    start("web_fetch", "c2", { url: "https://pi.dev/" });
    expect(status()).toEqual(["Reading a page…"]);
    start("read_attachment", "c3", { id: "file-1" });
    expect(status()).toEqual(["Reading a file…"]);
    end("read_attachment", "c3");
    expect(status()).toEqual(["Reading a page…"]);
    end("web_fetch", "c2");
    expect(status()).toEqual([]);
    // A tool this build doesn't know shows nothing.
    start("shell_exec", "c4");
    expect(status()).toEqual([]);
    for (const internal of [
      "web_search",
      "web_fetch",
      "read_attachment",
      "shell_exec",
      "search_failed",
    ])
      expect(document.body).not.toHaveTextContent(internal);
    // A run that ends while a tool still runs leaves no progress behind.
    start("web_search", "c5");
    expect(status()).toEqual(["Searching the web…"]);
    current = {
      ...current,
      messages: [
        { id: "q", role: "user", content: "Compare these pages", createdAt: 1 },
        {
          id: "reply",
          role: "assistant",
          content: "Let me read them.",
          status: "stopped",
          createdAt: 2,
        },
      ],
    };
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "run_end",
        data: { stopped: true, status: "stopped" },
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(),
    );
    expect(status()).toEqual([]);
  });

  it("says why a change was refused and keeps the saved setting", async () => {
    await openConversation();
    mocks.updateSession.mockRejectedValueOnce("assistant_storage_unavailable");
    await userEvent.click(toggle());
    expect(await screen.findByRole("alert")).not.toHaveTextContent(
      "assistant_",
    );
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
  });

  it("ignores the progress, sources and search flag of a run this view does not follow", async () => {
    current.session.webSearch = true;
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const other = { ...mocks.send.mock.calls[0][0], runId: "another-run" };
    act(() => {
      mocks.listener?.({
        ...other,
        type: "run_start",
        data: { messageId: "reply", webSearch: false },
      });
      mocks.listener?.({
        ...other,
        type: "tool_start",
        data: {
          messageId: "reply",
          name: "web_search",
          toolCallId: "c1",
          args: { query: "x" },
        },
      });
      mocks.listener?.({
        ...other,
        type: "sources",
        data: {
          messageId: "reply",
          sources: [
            {
              id: "s1",
              title: "Elsewhere",
              url: "https://a.example/",
              excerpt: "",
            },
          ],
        },
      });
    });
    expect(screen.queryByText("Searching the web…")).toBeNull();
    expect(screen.queryByRole("list", { name: "Sources" })).toBeNull();
    expect(screen.queryByRole("button", { name: /source/ })).toBeNull();
    expect(screen.queryByText(/Web search was unavailable/)).toBeNull();
    expect(screen.getByText("Thinking…")).toBeVisible();
  });

  it("can't be turned on, and says why, when search is unavailable", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      search: {
        available: false,
        unitPriceUsd: null,
        model: "openai/gpt-5-nano",
      },
    });
    await openConversation();
    expect(toggle()).toBeDisabled();
    expect(toggle()).toHaveAccessibleDescription(
      "Web search is unavailable right now. Replies use the model's own knowledge.",
    );
    expect(
      screen.getByText(
        "Web search is unavailable right now. Replies use the model's own knowledge.",
      ),
    ).toBeVisible();
    expect(document.body).not.toHaveTextContent("per search");
  });

  it.each([
    [
      "is unavailable",
      { available: false, unitPriceUsd: null, model: "openai/gpt-5-nano" },
    ],
    ["is unknown to the backend", undefined],
  ])(
    "can still be turned off, never on, while search %s",
    async (_, search) => {
      mocks.bootstrap.mockResolvedValue({ ...capabilities, search });
      current.session.webSearch = true;
      await openConversation();
      expect(toggle()).toHaveAttribute("aria-pressed", "true");
      expect(toggle()).toBeEnabled();
      await userEvent.click(toggle());
      await waitFor(() =>
        expect(toggle()).toHaveAttribute("aria-pressed", "false"),
      );
      expect(mocks.updateSession).toHaveBeenLastCalledWith(
        expect.objectContaining({ sessionId: "session-a", webSearch: false }),
      );
      expect(toggle()).toBeDisabled();
    },
  );

  describe("the switch's look", () => {
    const lit = () => expect(toggle()).toHaveClass("bg-brand-soft");
    const unlit = () => expect(toggle()).not.toHaveClass("bg-brand-soft");

    it("is lit while search is on and offered, and plain while it is off", async () => {
      await openConversation();
      unlit();
      await userEvent.click(toggle());
      await waitFor(() =>
        expect(toggle()).toHaveAttribute("aria-pressed", "true"),
      );
      lit();
    });

    it.each([
      [
        "is unavailable",
        { available: false, unitPriceUsd: null, model: "openai/gpt-5-nano" },
      ],
      ["is unknown to the backend", undefined],
    ])(
      "is not lit while search %s, though the saved setting is on and can be turned off",
      async (_, search) => {
        mocks.bootstrap.mockResolvedValue({ ...capabilities, search });
        current.session.webSearch = true;
        await openConversation();
        expect(toggle()).toHaveAttribute("aria-pressed", "true");
        expect(toggle()).toBeEnabled();
        unlit();
        await userEvent.click(toggle());
        await waitFor(() =>
          expect(toggle()).toHaveAttribute("aria-pressed", "false"),
        );
        expect(mocks.updateSession).toHaveBeenLastCalledWith(
          expect.objectContaining({ webSearch: false }),
        );
      },
    );

    it("is lit for a conversation saved with search on while search is offered", async () => {
      current.session.webSearch = true;
      await openConversation();
      lit();
    });
  });

  it("can be turned off before the first message while search is unavailable", async () => {
    mocks.listSessions.mockResolvedValue(listOf([]));
    function Refresh() {
      const shell = useShell();
      return (
        <>
          <button onClick={() => shell.setActive("assistant")}>
            Open assistant
          </button>
          <button onClick={() => void shell.refresh()}>
            Refresh assistant
          </button>
        </>
      );
    }
    render(
      <ShellProvider openSettings={() => undefined}>
        <Refresh />
        <AssistantPage />
      </ShellProvider>,
    );
    await waitFor(() => expect(toggle()).toBeEnabled());
    await userEvent.click(toggle());
    expect(toggle()).toHaveAttribute("aria-pressed", "true");
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      search: {
        available: false,
        unitPriceUsd: null,
        model: "openai/gpt-5-nano",
      },
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Open assistant" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Refresh assistant" }),
    );
    await waitFor(() =>
      expect(screen.getByText(/Web search is unavailable/)).toBeVisible(),
    );
    await userEvent.click(toggle());
    expect(toggle()).toHaveAttribute("aria-pressed", "false");
    expect(toggle()).toBeDisabled();
  });

  it("can't be changed for a conversation that hasn't loaded", async () => {
    mocks.getSession.mockRejectedValue("assistant_storage_unavailable");
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await screen.findByRole("alert");
    expect(toggle()).toBeDisabled();
    expect(mocks.updateSession).not.toHaveBeenCalled();
  });

  it("takes a backend that says nothing about search as search unavailable", async () => {
    const { search: _, ...older } = capabilities;
    mocks.bootstrap.mockResolvedValue(older);
    await openConversation();
    expect(toggle()).toBeDisabled();
    expect(
      screen.getByText(
        "Web search is unavailable right now. Replies use the model's own knowledge.",
      ),
    ).toBeVisible();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
  });
});

describe("assistant key and authority", () => {
  it("stops saying a key will be created once the first message is sent", async () => {
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      key: { status: "missing" },
    });
    await openConversation();
    const hint =
      "Your first message creates a dedicated Ofox API key for the assistant.";
    expect(screen.getByText(hint)).toBeVisible();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    await waitFor(() => expect(screen.queryByText(hint)).toBeNull());
  });

  it("reads the authority again when the backend says it moved on, and writes with the new one", async () => {
    mocks.bootstrap
      .mockResolvedValueOnce(capabilities)
      .mockResolvedValue({ ...capabilities, authorityGeneration: 2 });
    mocks.updateSession.mockRejectedValueOnce("assistant_identity_changed");
    await openConversation();
    const rename = async () => {
      await userEvent.click(
        screen.getByRole("button", { name: "Rename conversation" }),
      );
      const dialog = await screen.findByRole("dialog");
      await userEvent.type(
        within(dialog).getByRole("textbox", { name: "Conversation title" }),
        "!",
      );
      await userEvent.click(
        within(dialog).getByRole("button", { name: "Save" }),
      );
    };
    await rename();
    await waitFor(() => expect(mocks.bootstrap).toHaveBeenCalledTimes(2));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The account or region changed. Please try again.",
    );
    await rename();
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenLastCalledWith(
        expect.objectContaining({ expectedGeneration: 2 }),
      ),
    );
  });
});

describe("sign-in changes", () => {
  it("lists nothing for a background run that ends after sign-in expired, and again once it is restored", async () => {
    const running = {
      ...session,
      activeRun: { runId: "bg-run", operationId: "bg-op" },
    };
    mocks.listSessions.mockResolvedValue(listOf([running]));
    render(page());
    const list = await screen.findByRole("list", { name: "Conversations" });
    await within(list).findByText("Replying…");
    act(() => mocks.auth?.("expired"));
    const listed = mocks.listSessions.mock.calls.length;
    // The backend ends the account's runs when sign-in expires.
    const runEnd: AssistantEvent = {
      sessionId: session.id,
      runId: "bg-run",
      operationId: "bg-op",
      type: "run_end",
      data: { stopped: true, status: "stopped" },
    };
    act(() => mocks.listener?.(runEnd));
    await act(async () => undefined);
    expect(mocks.listSessions).toHaveBeenCalledTimes(listed);
    expect(within(list).queryByText("My conversation")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();

    act(() => mocks.auth?.("restored"));
    await within(list).findByText("Replying…");
    mocks.listSessions.mockResolvedValue(listOf([session]));
    act(() => mocks.listener?.(runEnd));
    await waitFor(() =>
      expect(within(list).queryByText("Replying…")).toBeNull(),
    );
  });

  it("reads the authority once more when sign-in completes during the first read", async () => {
    // ofox-auth-restored arrives before the sign-in has finished: the first
    // read can be refused as the backend moves its authority on.
    mocks.bootstrap
      .mockRejectedValueOnce("assistant_identity_changed")
      .mockResolvedValue({ ...capabilities, authorityGeneration: 2 });
    await openConversation();
    expect(mocks.bootstrap).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("saves a refused draft again under the new authority without an error", async () => {
    mocks.bootstrap
      .mockResolvedValueOnce(capabilities)
      .mockResolvedValue({ ...capabilities, authorityGeneration: 2 });
    mocks.updateSession.mockRejectedValueOnce("assistant_identity_changed");
    await openConversation();
    await userEvent.type(composer(), "Kept draft");
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenLastCalledWith(
        expect.objectContaining({
          draft: "Kept draft",
          expectedGeneration: 2,
        }),
      ),
    );
    expect(current.session.draft).toBe("Kept draft");
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("never sends a message again by itself after the authority moved on", async () => {
    mocks.bootstrap
      .mockResolvedValueOnce(capabilities)
      .mockResolvedValue({ ...capabilities, authorityGeneration: 2 });
    mocks.send.mockRejectedValueOnce("assistant_identity_changed");
    await openConversation();
    await userEvent.type(composer(), "Paid question");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The account or region changed. Please try again.",
    );
    await waitFor(() => expect(mocks.bootstrap).toHaveBeenCalledTimes(2));
    expect(mocks.send).toHaveBeenCalledOnce();
    expect(composer()).toHaveValue("Paid question");
  });

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

  it("drops the composer's files when sign-in expires, and shows only the saved ones once restored", async () => {
    const file: AssistantAttachment = {
      id: "file-1",
      name: "private.pdf",
      mimeType: "application/pdf",
      sizeBytes: 2048,
      status: "ready",
      reason: null,
      previewUrl: null,
      details: null,
    };
    const files = () =>
      within(composer().closest("footer") as HTMLElement).queryByRole("list", {
        name: "Attached files",
      });
    await openConversation();
    mocks.pickAttachments.mockResolvedValueOnce([file]);
    await userEvent.click(screen.getByRole("button", { name: "Attach files" }));
    await waitFor(() => expect(files()).toHaveTextContent("private.pdf"));
    // The account's saved draft names no file.
    mocks.getSession.mockResolvedValue({
      ...current,
      session: { ...current.session, draftAttachmentIds: [] },
      attachments: [file],
    });
    act(() => mocks.auth?.("expired"));
    expect(files()).toBeNull();
    act(() => mocks.auth?.("restored"));
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() => expect(composer()).toBeEnabled());
    expect(files()).toBeNull();
  });
});

describe("conversation trash", () => {
  it("explains an empty trash view without presenting it as an active conversation", async () => {
    render(page());
    await userEvent.click(await screen.findByRole("button", { name: "Trash" }));
    expect(
      await screen.findByText(
        "Choose a conversation from the trash to view or restore it.",
      ),
    ).toBeVisible();
    expect(
      screen.queryByText(
        "This conversation is in the trash. Restore it to continue chatting.",
      ),
    ).toBeNull();
    expect(screen.queryByText("How can I help?")).toBeNull();
  });

  beforeEach(() => {
    mocks.listSessions.mockImplementation(async (deleted = false) =>
      listOf(
        Boolean(current.session.deletedAt) === deleted ? [current.session] : [],
      ),
    );
    mocks.updateSession.mockImplementation(async (patch) => {
      current.session = {
        ...current.session,
        ...patch,
        ...(patch.deleted === undefined
          ? {}
          : { deletedAt: patch.deleted ? 10 : null }),
      };
      return current.session;
    });
  });

  it.each([true, false])(
    "refreshes the currently selected list after a delayed trash update (deleted=%s)",
    async (deleted) => {
      current.session.deletedAt = deleted ? null : 10;
      let release!: () => void;
      const waiting = new Promise<void>((resolve) => {
        release = resolve;
      });
      mocks.updateSession.mockImplementation(async (patch) => {
        if (patch.deleted !== undefined) await waiting;
        current.session = {
          ...current.session,
          ...patch,
          ...(patch.deleted === undefined
            ? {}
            : { deletedAt: patch.deleted ? 10 : null }),
        };
        return current.session;
      });
      const hook = renderHook(() => useAssistant(true));
      await waitFor(() => expect(hook.result.current.connected).toBe(true));
      if (!deleted) act(() => hook.result.current.viewTrash(true));
      act(() => hook.result.current.select(session.id));
      await waitFor(() => expect(hook.result.current.loading).toBe(false));
      let saving!: Promise<boolean>;
      act(() => {
        saving = hook.result.current.setDeleted(deleted);
      });
      await waitFor(() =>
        expect(mocks.updateSession).toHaveBeenCalledWith(
          expect.objectContaining({ deleted }),
        ),
      );
      act(() => hook.result.current.viewTrash(deleted));
      await waitFor(() =>
        expect(mocks.listSessions).toHaveBeenLastCalledWith(deleted, ""),
      );
      await act(async () => {
        release();
        await saving;
      });
      await waitFor(() => expect(hook.result.current.sessions).toHaveLength(1));
      expect(hook.result.current.showTrash).toBe(deleted);
      expect(mocks.listSessions).toHaveBeenLastCalledWith(deleted, "");
    },
  );

  it("saves the draft before trashing, shows it read-only, and restores the conversation", async () => {
    await openConversation();
    fireEvent.change(composer(), { target: { value: "Keep this draft" } });
    await userEvent.click(
      screen.getByRole("button", { name: "Move to trash" }),
    );
    await waitFor(() => expect(current.session.deletedAt).toBe(10));
    expect(current.session.draft).toBe("Keep this draft");
    await userEvent.click(screen.getByRole("button", { name: "Trash" }));
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() => expect(composer()).toHaveValue("Keep this draft"));
    expect(composer()).toHaveAttribute("readonly");
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Restore conversation" }),
    );
    await waitFor(() => expect(current.session.deletedAt).toBeNull());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() => expect(composer()).not.toHaveAttribute("readonly"));
    expect(composer()).toHaveValue("Keep this draft");
  });

  it("requires an explicit confirmation before deleting files and history permanently", async () => {
    current.session.deletedAt = 10;
    mocks.deleteSession.mockImplementation(async () => {
      mocks.listSessions.mockResolvedValue(listOf([]));
    });
    render(page());
    await userEvent.click(await screen.findByRole("button", { name: "Trash" }));
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Delete permanently" }),
    );
    const dialog = screen.getByRole("dialog", {
      name: "Delete conversation permanently?",
    });
    expect(
      within(dialog).getByText(/attachments and conversation history/),
    ).toBeVisible();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    expect(mocks.deleteSession).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Delete permanently" }),
      ).toHaveFocus(),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Delete permanently" }),
    );
    await userEvent.click(
      within(screen.getByRole("dialog")).getByRole("button", {
        name: "Delete permanently",
      }),
    );
    await waitFor(() =>
      expect(mocks.deleteSession).toHaveBeenCalledWith({
        sessionId: session.id,
        expectedOwner: capabilities.owner,
        expectedGeneration: capabilities.authorityGeneration,
      }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: /My conversation/ }),
      ).toBeNull(),
    );
  });
});

describe("permanently deleting a trashed conversation", () => {
  beforeEach(() => {
    current.session.deletedAt = 10;
    mocks.listSessions.mockImplementation(async (deleted = false) =>
      listOf(
        Boolean(current.session.deletedAt) === deleted ? [current.session] : [],
      ),
    );
  });
  async function openDeleteDialog() {
    render(page());
    await userEvent.click(await screen.findByRole("button", { name: "Trash" }));
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Delete permanently" }),
    );
    return screen.getByRole("dialog", {
      name: "Delete conversation permanently?",
    });
  }

  it("says why it failed inside the dialog, once, and lets the user retry or cancel", async () => {
    mocks.deleteSession.mockRejectedValueOnce("assistant_storage_unavailable");
    const dialog = await openDeleteDialog();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Delete permanently" }),
    );
    const alert = await within(dialog).findByRole("alert");
    expect(alert).toHaveTextContent(
      "Local assistant data couldn't be read or saved.",
    );
    // The page's own banner sits behind the dialog and is not announced too.
    expect(screen.getAllByRole("alert")).toEqual([alert]);
    expect(
      within(dialog).getByRole("button", { name: "Cancel" }),
    ).toHaveFocus();
    expect(
      within(dialog).getByRole("button", { name: "Delete permanently" }),
    ).toBeEnabled();

    // A second try clears the old message while it runs and succeeds.
    mocks.deleteSession.mockImplementationOnce(async () => {
      mocks.listSessions.mockResolvedValue(listOf([]));
    });
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Delete permanently" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(mocks.deleteSession).toHaveBeenCalledTimes(2);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("leaves no stale failure behind when the dialog is cancelled", async () => {
    mocks.deleteSession.mockRejectedValueOnce("assistant_storage_unavailable");
    const dialog = await openDeleteDialog();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Delete permanently" }),
    );
    await within(dialog).findByRole("alert");
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.queryByRole("alert")).toBeNull();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Delete permanently" }),
      ).toHaveFocus(),
    );
  });

  it("closes the dialog and says so in the page when only the disk cleanup is left to retry", async () => {
    mocks.deleteSession.mockRejectedValueOnce("assistant_delete_pending");
    const dialog = await openDeleteDialog();
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Delete permanently" }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The conversation was removed, but some files could not be deleted.",
    );
  });
});

describe("Markdown export", () => {
  it.each(["selection", "account"])(
    "does not export a snapshot that arrived after the %s changed",
    async (change) => {
      await openConversation();
      let resolve!: (detail: AssistantSessionDetail) => void;
      mocks.getSession.mockImplementationOnce(
        () =>
          new Promise<AssistantSessionDetail>((done) => {
            resolve = done;
          }),
      );
      const reads = mocks.getSession.mock.calls.length;
      await userEvent.click(
        screen.getByRole("button", { name: "Export Markdown" }),
      );
      await waitFor(() =>
        expect(mocks.getSession).toHaveBeenCalledTimes(reads + 1),
      );
      if (change === "account") act(() => mocks.auth?.("expired"));
      else await userEvent.click(screen.getByRole("button", { name: "Trash" }));
      await act(async () => resolve(current));
      expect(mocks.exportMarkdown).not.toHaveBeenCalled();
      expect(screen.queryByText("Markdown saved")).not.toBeInTheDocument();
    },
  );

  it("flushes the current draft and exports a fresh snapshot with confirmed account authority", async () => {
    await openConversation();
    fireEvent.change(composer(), { target: { value: "latest unsent draft" } });
    current = {
      ...current,
      messages: [
        {
          id: "fresh",
          role: "assistant",
          content: "Saved response from disk",
          status: "completed",
          createdAt: 2,
        },
      ],
    };
    await userEvent.click(
      screen.getByRole("button", { name: "Export Markdown" }),
    );
    await waitFor(() => expect(mocks.exportMarkdown).toHaveBeenCalledTimes(1));
    const request = mocks.exportMarkdown.mock.calls[0][0];
    expect(request).toMatchObject({
      sessionId: session.id,
      expectedOwner: capabilities.owner,
      expectedGeneration: capabilities.authorityGeneration,
    });
    expect(request.markdown).toContain("latest unsent draft");
    expect(request.markdown).toContain("Saved response from disk");
    expect(await screen.findByText("Markdown saved")).toHaveAttribute(
      "role",
      "status",
    );
  });

  it("keeps cancellation quiet and the draft editable", async () => {
    mocks.exportMarkdown.mockResolvedValue(false);
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Export Markdown" }),
    );
    await waitFor(() => expect(mocks.exportMarkdown).toHaveBeenCalled());
    await waitFor(() => expect(composer()).toBeEnabled());
    expect(screen.queryByText("Markdown saved")).not.toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("does not open the native dialog when saving the draft fails", async () => {
    await openConversation();
    mocks.updateSession.mockRejectedValue("assistant_storage_unavailable");
    fireEvent.change(composer(), { target: { value: "keep this" } });
    await userEvent.click(
      screen.getByRole("button", { name: "Export Markdown" }),
    );
    await screen.findByRole("alert");
    expect(mocks.exportMarkdown).not.toHaveBeenCalled();
    expect(composer()).toHaveValue("keep this");
  });
});

describe("complete conversation backups", () => {
  it("does not rewrite an untouched conversation before restoring an identical backup", async () => {
    await openConversation();
    mocks.restore.mockResolvedValue({
      imported: 0,
      skipped: 1,
      conflicts: 0,
      warning: null,
    });
    await chooseFromMenu("Restore…");
    await waitFor(() =>
      expect(toasted()).toEqual([
        {
          kind: "success",
          lines: ["1 conversation was already on this device."],
        },
      ]),
    );
    expect(mocks.updateSession).not.toHaveBeenCalled();
  });

  it("preserves a new unsent draft when selection is attempted during backup creation", async () => {
    let resolve!: (value: AssistantSession) => void;
    mocks.createSession.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
    act(() => hook.result.current.setDraft("New unsent backup draft"));
    let transfer!: Promise<void>;
    act(() => {
      transfer = hook.result.current.transfer("backup");
    });
    await waitFor(() => expect(mocks.createSession).toHaveBeenCalled());
    act(() => hook.result.current.select(session.id));
    expect(hook.result.current.selectedId).toBeNull();
    expect(hook.result.current.draft).toBe("New unsent backup draft");
    await act(async () => {
      resolve({ ...session, id: "created-for-backup" });
      await transfer;
    });
    expect(mocks.updateSession).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: "created-for-backup",
        draft: "New unsent backup draft",
      }),
    );
    expect(mocks.backup).toHaveBeenCalled();
    expect(hook.result.current.error).toBeNull();
  });

  it("uses the restored default model for the next new conversation", async () => {
    mocks.restore.mockImplementation(async () => {
      mocks.bootstrap.mockResolvedValue({
        ...capabilities,
        defaultModelId: "fast",
      });
      return { imported: 1, skipped: 0, conflicts: 0, warning: null };
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
    await act(async () => {
      await hook.result.current.transfer("restore");
    });
    await act(async () => {
      await hook.result.current.create();
    });
    expect(mocks.createSession).toHaveBeenCalledWith(
      expect.objectContaining({ modelId: "fast" }),
    );
  });

  it("does not overwrite restored preferences when their bootstrap reload fails", async () => {
    mocks.restore.mockImplementation(async () => {
      mocks.bootstrap.mockRejectedValue("assistant_network_error");
      return { imported: 1, skipped: 0, conflicts: 0, warning: null };
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
    await act(async () => {
      await hook.result.current.transfer("restore");
    });
    expect(hook.result.current.bootstrap).toBeNull();
    expect(hook.result.current.error).not.toBeNull();
    await act(async () => {
      await hook.result.current.create();
    });
    expect(mocks.createSession).not.toHaveBeenCalled();
    mocks.bootstrap.mockResolvedValue({
      ...capabilities,
      defaultModelId: "fast",
    });
    await act(async () => {
      await hook.result.current.loadBootstrap();
    });
    await act(async () => {
      await hook.result.current.create();
    });
    expect(mocks.createSession).toHaveBeenCalledWith(
      expect.objectContaining({ modelId: "fast" }),
    );
  });

  it("flushes unsent draft before opening the account backup save dialog", async () => {
    await openConversation();
    fireEvent.change(composer(), {
      target: { value: "Draft in complete backup" },
    });
    mocks.backup.mockImplementation(async () => {
      expect(current.session.draft).toBe("Draft in complete backup");
      return { sessions: 1, skipped: 0 };
    });
    await chooseFromMenu("Back up…");
    await waitFor(() =>
      expect(mocks.backup).toHaveBeenCalledWith({
        expectedOwner: capabilities.owner,
        expectedGeneration: capabilities.authorityGeneration,
      }),
    );
    await waitFor(() =>
      expect(toasted()).toEqual([
        { kind: "success", lines: ["Backed up 1 conversation."] },
      ]),
    );
  });
  it("keeps native backup cancellation quiet", async () => {
    mocks.backup.mockResolvedValue(null);
    render(page());
    await chooseFromMenu("Back up…");
    await waitFor(() => expect(mocks.backup).toHaveBeenCalled());
    await act(async () => undefined);
    expect(toasted()).toEqual([]);
  });
  it("reports partial restore counts and its warning without discarding the current conversation", async () => {
    mocks.restore.mockResolvedValue({
      imported: 2,
      skipped: 1,
      conflicts: 0,
      warning: "assistant_backup_conflict",
    });
    await openConversation();
    await chooseFromMenu("Restore…");
    await waitFor(() =>
      expect(mocks.restore).toHaveBeenCalledWith({
        expectedOwner: capabilities.owner,
        expectedGeneration: capabilities.authorityGeneration,
      }),
    );
    await waitFor(() =>
      expect(toasted()).toEqual([
        {
          kind: "success",
          lines: [
            "Restored 2 conversations.",
            "1 conversation was already on this device.",
          ],
        },
      ]),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent("same ID");
    expect(screen.getByRole("heading", { name: session.title })).toBeVisible();
  });
  it("does not report an old account's completed backup after sign-in expires", async () => {
    let resolve!: (saved: { sessions: number; skipped: number }) => void;
    mocks.backup.mockImplementation(
      () =>
        new Promise((done) => {
          resolve = done;
        }),
    );
    await openConversation();
    await chooseFromMenu("Back up…");
    await waitFor(() => expect(mocks.backup).toHaveBeenCalled());
    act(() => mocks.auth?.("expired"));
    await act(async () => resolve({ sessions: 1, skipped: 0 }));
    expect(toasted()).toEqual([]);
  });
});

describe("a failed reply", () => {
  const reason = /Your Ofox balance is too low for this reply/;
  const failedMessage: AssistantMessage = {
    id: "reply",
    role: "assistant",
    content: "Half an answ",
    status: "error",
    errorCode: "assistant_insufficient_balance",
    createdAt: 2,
  };

  it("is announced once: the banner alerts, the saved reason of the reply is plain text", async () => {
    await openConversation();
    await userEvent.type(composer(), "Hello");
    await userEvent.click(screen.getByRole("button", { name: "Send" }));
    await screen.findByRole("button", { name: "Stop" });
    const sent = mocks.send.mock.calls[0][0];
    act(() => {
      mocks.listener?.({
        ...sent,
        type: "message",
        data: { message: failedMessage },
      });
      mocks.listener?.({
        ...sent,
        type: "error",
        data: { code: "assistant_insufficient_balance" },
      });
    });
    await screen.findByRole("alert");
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    // The reply still says why, for whoever reads the transcript.
    expect(screen.getAllByText(reason)).toHaveLength(2);
  });

  it("announces nothing when a conversation with a saved failure is opened", async () => {
    current.messages = [failedMessage];
    await openConversation();
    expect(screen.getByText(reason)).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
  });
});

describe("conversation trash, in detail", () => {
  const hint =
    "This conversation is in the trash. Restore it to continue chatting.";
  beforeEach(() => {
    current.session.deletedAt = 10;
    mocks.listSessions.mockImplementation(async (deleted = false) =>
      listOf(
        Boolean(current.session.deletedAt) === deleted ? [current.session] : [],
      ),
    );
    mocks.updateSession.mockImplementation(async (patch) => {
      current.session = {
        ...current.session,
        ...patch,
        ...(patch.deleted === undefined
          ? {}
          : { deletedAt: patch.deleted ? 10 : null }),
      };
      return current.session;
    });
  });
  async function openTrashed() {
    render(page());
    await userEvent.click(await screen.findByRole("button", { name: "Trash" }));
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await screen.findByRole("button", { name: "Restore conversation" });
  }

  it("is read-only: the title can't be renamed until it is restored", async () => {
    await openTrashed();
    expect(
      screen.getByRole("button", { name: "Rename conversation" }),
    ).toBeDisabled();
    await userEvent.click(
      screen.getByRole("button", { name: "Restore conversation" }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Rename conversation" }),
      ).toBeEnabled(),
    );
  });

  it("says it is in the trash once when it has no messages", async () => {
    await openTrashed();
    expect(screen.getAllByText(hint)).toHaveLength(1);
  });

  it("says it is in the trash once when it has messages", async () => {
    current.messages = [
      { id: "q", role: "user", content: "Old question", createdAt: 1 },
    ];
    await openTrashed();
    await screen.findByText("Old question");
    expect(screen.getAllByText(hint)).toHaveLength(1);
  });

  it("goes back to the conversations without listing the trash again for it", async () => {
    await openTrashed();
    const reads = mocks.listSessions.mock.calls.length;
    await userEvent.click(
      screen.getByRole("button", { name: "Restore conversation" }),
    );
    await waitFor(() => expect(current.session.deletedAt).toBeNull());
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(false, ""),
    );
    // A read of the trash begun for the restore would flash the restored
    // conversation among the trashed ones.
    expect(
      mocks.listSessions.mock.calls
        .slice(reads)
        .every(([deleted]) => deleted === false),
    ).toBe(true);
    expect(
      await screen.findByRole("button", { name: /My conversation/ }),
    ).toBeVisible();
  });
});

describe("the labels of the sidebar", () => {
  it.each(["en", "zh", "ja"])(
    "names backup and restore by what they show, and describes what a backup holds (%s)",
    async (language) => {
      i18n.addResourceBundle("zh", "translation", zh, true, true);
      i18n.addResourceBundle("ja", "translation", ja, true, true);
      await act(async () => {
        await i18n.changeLanguage(language);
      });
      render(page());
      await userEvent.click(
        await screen.findByRole("button", {
          name: i18n.t("assistant.moreActions"),
        }),
      );
      for (const key of ["assistant.backupMenu", "assistant.restoreMenu"]) {
        const item = await screen.findByRole("menuitem", { name: i18n.t(key) });
        expect(item).toHaveAccessibleDescription(
          i18n.t("assistant.backupHint"),
        );
      }
    },
  );

  it("groups the conversation's usage under its name", async () => {
    current.usage = { inputTokens: 10, outputTokens: 5 };
    await openConversation();
    expect(
      screen.getByRole("group", { name: "This conversation" }),
    ).toHaveTextContent("Input 10 · Output 5 tokens");
  });
});

describe("what a backup or restore reports", () => {
  // The menu waits for its entries to be enabled.
  const ready = async () => void render(page());
  const toastedOnce = () =>
    waitFor(() => {
      expect(toasted()).toHaveLength(1);
      return toasted()[0];
    });

  it("says how many conversations a backup holds", async () => {
    mocks.backup.mockResolvedValue({ sessions: 3, skipped: 0 });
    await ready();
    await chooseFromMenu("Back up…");
    expect(await toastedOnce()).toEqual({
      kind: "success",
      lines: ["Backed up 3 conversations."],
    });
  });

  it("warns, in the same toast, about damaged conversations left out of it", async () => {
    mocks.backup.mockResolvedValue({ sessions: 3, skipped: 2 });
    await ready();
    await chooseFromMenu("Back up…");
    expect(await toastedOnce()).toEqual({
      kind: "warning",
      lines: [
        "Backed up 3 conversations.",
        "2 conversations couldn't be read and were left out of the backup.",
      ],
    });
  });

  it("keeps a result in view long enough to read its lines", async () => {
    mocks.backup.mockResolvedValue({ sessions: 3, skipped: 2 });
    await ready();
    await chooseFromMenu("Back up…");
    await toastedOnce();
    const [, options] = toasts.warning.mock.calls[0];
    expect(options.duration).toBeGreaterThanOrEqual(6000);
    expect(options.classNames.description).toContain("whitespace-pre-line");
  });

  it.each([
    [1, "Backed up 1 conversation."],
    [2, "Backed up 2 conversations."],
  ])(
    "counts %i backed up conversation(s) in the singular or plural",
    async (sessions, text) => {
      mocks.backup.mockResolvedValue({ sessions, skipped: 0 });
      await ready();
      await chooseFromMenu("Back up…");
      expect((await toastedOnce()).lines).toEqual([text]);
    },
  );

  it("says what a restore imported, already had, and left unchanged, each with its own count", async () => {
    mocks.restore.mockResolvedValue({
      imported: 1,
      skipped: 3,
      conflicts: 2,
      warning: null,
    });
    await ready();
    await chooseFromMenu("Restore…");
    expect(await toastedOnce()).toEqual({
      kind: "warning",
      lines: [
        "Restored 1 conversation.",
        "3 conversations were already on this device.",
        "2 conversations on this device are different or in use, so they were left unchanged.",
      ],
    });
  });

  it("names only conflicts when nothing else happened, and never claims a restore", async () => {
    mocks.restore.mockResolvedValue({
      imported: 0,
      skipped: 0,
      conflicts: 1,
      warning: null,
    });
    await ready();
    await chooseFromMenu("Restore…");
    expect(await toastedOnce()).toEqual({
      kind: "warning",
      lines: [
        "1 conversation on this device is different or in use, so it was left unchanged.",
      ],
    });
  });

  it("says so when the backup had nothing to restore", async () => {
    mocks.restore.mockResolvedValue({
      imported: 0,
      skipped: 0,
      conflicts: 0,
      warning: null,
    });
    await ready();
    await chooseFromMenu("Restore…");
    expect(await toastedOnce()).toEqual({
      kind: "warning",
      lines: ["The backup has no conversations to restore."],
    });
  });

  it.each([
    [
      "zh",
      ["已备份 3 个对话。", "2 个对话无法读取，未放进备份。"],
      [
        "已恢复 1 个对话。",
        "3 个对话已在本机，已跳过。",
        "2 个对话与本机已有的内容不同或正在使用，已保留本机的版本。",
      ],
    ],
    [
      "ja",
      [
        "3 件の会話をバックアップしました。",
        "2 件の会話を読み取れなかったため、バックアップに含まれていません。",
      ],
      [
        "1 件の会話を復元しました。",
        "3 件の会話は既にこの端末にあるため、スキップしました。",
        "2 件の会話は、この端末に別の内容で存在するか使用中のため、そのままにしました。",
      ],
    ],
  ])("tells the results in %s", async (language, backed, restored) => {
    i18n.addResourceBundle("zh", "translation", zh, true, true);
    i18n.addResourceBundle("ja", "translation", ja, true, true);
    await act(async () => {
      await i18n.changeLanguage(language);
    });
    mocks.backup.mockResolvedValue({ sessions: 3, skipped: 2 });
    mocks.restore.mockResolvedValue({
      imported: 1,
      skipped: 3,
      conflicts: 2,
      warning: null,
    });
    await ready();
    await chooseFromMenu(i18n.t("assistant.backupMenu"));
    await waitFor(() => expect(toasted()).toHaveLength(1));
    await chooseFromMenu(i18n.t("assistant.restoreMenu"));
    await waitFor(() => expect(toasted()).toHaveLength(2));
    expect(toasted().map((told) => told.lines)).toEqual([backed, restored]);
  });

  it("refuses with its own message while another conversation sends or imports, and does not open the dialog", async () => {
    mocks.send.mockImplementation(() => new Promise(() => undefined));
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.connected).toBe(true));
    await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
    act(() => hook.result.current.select(session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    act(() => hook.result.current.setDraft("A question"));
    act(() => {
      void hook.result.current.send();
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalled());
    // Leave for no conversation: the send goes on in the background.
    act(() => hook.result.current.select(null));
    await act(async () => {
      await hook.result.current.transfer("backup");
    });
    expect(hook.result.current.error).toBe("assistant_transfer_busy");
    expect(mocks.backup).not.toHaveBeenCalled();
    await act(async () => {
      await hook.result.current.transfer("restore");
    });
    expect(hook.result.current.error).toBe("assistant_transfer_busy");
    expect(mocks.restore).not.toHaveBeenCalled();
  });

  it("keeps the model the user chose when a restore imported nothing", async () => {
    mocks.restore.mockResolvedValue({
      imported: 0,
      skipped: 2,
      conflicts: 0,
      warning: null,
    });
    const hook = renderHook(() => useAssistant(true));
    await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
    act(() => hook.result.current.chooseNewModel("fast"));
    const reads = mocks.bootstrap.mock.calls.length;
    await act(async () => {
      await hook.result.current.transfer("restore");
    });
    expect(hook.result.current.newModelId).toBe("fast");
    expect(mocks.bootstrap).toHaveBeenCalledTimes(reads);
  });

  it("goes to the conversations when a new unsent draft is saved for a backup from the trash, and is not called trashed", async () => {
    render(page());
    await waitFor(() => expect(composer()).toBeEnabled());
    fireEvent.change(composer(), { target: { value: "Unsent words" } });
    await userEvent.click(screen.getByRole("button", { name: "Trash" }));
    expect(composer()).toHaveAttribute("readonly");
    expect(composer()).toHaveValue("Unsent words");
    await chooseFromMenu("Back up…");
    await waitFor(() => expect(mocks.backup).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "Trash" })).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Back to conversations" }),
    ).toBeNull();
    await waitFor(() => expect(composer()).not.toHaveAttribute("readonly"));
    expect(composer()).toHaveValue("Unsent words");
    expect(
      screen.queryByText(
        "This conversation is in the trash. Restore it to continue chatting.",
      ),
    ).toBeNull();
    expect(mocks.updateSession).not.toHaveBeenCalledWith(
      expect.objectContaining({ deleted: true }),
    );
  });
});

describe("the sidebar", () => {
  it("keeps to the list: no view tabs, totals of all conversations or disclaimer", async () => {
    render(page());
    await screen.findByRole("button", { name: /My conversation/ });
    expect(screen.queryByRole("group", { name: /view/i })).toBeNull();
    expect(
      screen.queryByText(/Includes conversations in the trash/),
    ).toBeNull();
    expect(screen.queryByText(/Replies come from the model/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Conversations" })).toBeNull();
    expect(mocks.usageSummary).not.toHaveBeenCalled();
  });

  it("starts a new conversation from its icon button", async () => {
    render(page());
    const create = await screen.findByRole("button", {
      name: "New conversation",
    });
    expect(create).toHaveTextContent("");
    await waitFor(() => expect(create).toBeEnabled());
    await userEvent.click(create);
    await waitFor(() => expect(mocks.createSession).toHaveBeenCalled());
  });

  describe("search", () => {
    it("shows only an icon until it is opened, then a focused field", async () => {
      render(page());
      await screen.findByRole("button", { name: /My conversation/ });
      const open = screen.getByRole("button", { name: "Search conversations" });
      expect(open).toHaveTextContent("");
      expect(screen.queryByRole("searchbox")).toBeNull();
      await userEvent.click(open);
      expect(
        screen.getByRole("searchbox", { name: "Search conversations" }),
      ).toHaveFocus();
    });

    it("closes and clears with Escape, back on its button, without leaving the page", async () => {
      const left = vi.fn();
      window.addEventListener("keydown", left);
      try {
        render(page());
        await screen.findByRole("button", { name: /My conversation/ });
        await searchFor("notes");
        await waitFor(() =>
          expect(mocks.listSessions).toHaveBeenLastCalledWith(false, "notes"),
        );
        await userEvent.keyboard("{Escape}");
        expect(screen.queryByRole("searchbox")).toBeNull();
        await waitFor(() =>
          expect(
            screen.getByRole("button", { name: "Search conversations" }),
          ).toHaveFocus(),
        );
        await waitFor(() =>
          expect(mocks.listSessions).toHaveBeenLastCalledWith(false, ""),
        );
        // The app's Escape handler sees it as handled.
        expect(left.mock.calls.at(-1)?.[0].defaultPrevented).toBe(true);
      } finally {
        window.removeEventListener("keydown", left);
      }
    });

    it("closes when it loses focus empty, and stays open with its filter", async () => {
      render(page());
      await screen.findByRole("button", { name: /My conversation/ });
      await userEvent.click(
        screen.getByRole("button", { name: "Search conversations" }),
      );
      await userEvent.tab();
      expect(screen.queryByRole("searchbox")).toBeNull();
      await searchFor("notes");
      await userEvent.tab();
      expect(
        screen.getByRole("searchbox", { name: "Search conversations" }),
      ).toHaveValue("notes");
    });

    it("filters the trash too, and the conversations again on the way back", async () => {
      render(page());
      await screen.findByRole("button", { name: /My conversation/ });
      await searchFor("notes");
      await waitFor(() =>
        expect(mocks.listSessions).toHaveBeenLastCalledWith(false, "notes"),
      );
      await userEvent.click(screen.getByRole("button", { name: "Trash" }));
      await waitFor(() =>
        expect(mocks.listSessions).toHaveBeenLastCalledWith(true, "notes"),
      );
      await userEvent.click(
        screen.getByRole("button", { name: "Back to conversations" }),
      );
      await waitFor(() =>
        expect(mocks.listSessions).toHaveBeenLastCalledWith(false, "notes"),
      );
    });
  });

  describe("the trash", () => {
    it("counts every trashed conversation in its entry, whatever the search", async () => {
      mocks.listSessions.mockImplementation(async (_deleted, search) =>
        listOf(search ? [] : [session], [], 3),
      );
      render(page());
      const entry = await screen.findByRole("button", { name: "Trash · 3" });
      await searchFor("nothing like it");
      await waitFor(() =>
        expect(mocks.listSessions).toHaveBeenLastCalledWith(
          false,
          "nothing like it",
        ),
      );
      await screen.findByText("No matching conversations");
      expect(entry).toHaveTextContent("Trash · 3");
    });

    it("shows its conversations under a way back that takes focus, and returns to its entry", async () => {
      current.session.deletedAt = 10;
      mocks.listSessions.mockImplementation(async (deleted = false) =>
        listOf(deleted ? [current.session] : [], [], 1),
      );
      render(page());
      await userEvent.click(
        await screen.findByRole("button", { name: "Trash · 1" }),
      );
      const back = screen.getByRole("button", {
        name: "Back to conversations",
      });
      await waitFor(() => expect(back).toHaveFocus());
      const sidebar = screen.getByRole("complementary", {
        name: "Conversations",
      });
      expect(
        within(sidebar).getByRole("heading", { name: "Trash" }),
      ).toBeVisible();
      const list = within(sidebar).getByRole("list", { name: "Trash" });
      expect(
        await within(list).findByRole("button", { name: "My conversation" }),
      ).toBeVisible();
      expect(screen.queryByRole("button", { name: "Trash · 1" })).toBeNull();
      await userEvent.click(back);
      await waitFor(() =>
        expect(screen.getByRole("button", { name: "Trash · 1" })).toHaveFocus(),
      );
      expect(
        screen.queryByRole("button", { name: "Back to conversations" }),
      ).toBeNull();
    });

    it("gives focus to its entry once the open conversation is moved there, and once it is restored", async () => {
      mocks.listSessions.mockImplementation(async (deleted = false) =>
        listOf(
          Boolean(current.session.deletedAt) === deleted
            ? [current.session]
            : [],
        ),
      );
      mocks.updateSession.mockImplementation(async (patch) => {
        current.session = {
          ...current.session,
          ...patch,
          ...(patch.deleted === undefined
            ? {}
            : { deletedAt: patch.deleted ? 10 : null }),
        };
        return current.session;
      });
      await openConversation();
      await userEvent.click(
        screen.getByRole("button", { name: "Move to trash" }),
      );
      await waitFor(() =>
        expect(screen.getByRole("button", { name: "Trash" })).toHaveFocus(),
      );
      await userEvent.click(screen.getByRole("button", { name: "Trash" }));
      await userEvent.click(
        await screen.findByRole("button", { name: "My conversation" }),
      );
      await userEvent.click(
        await screen.findByRole("button", { name: "Restore conversation" }),
      );
      await waitFor(() =>
        expect(screen.getByRole("button", { name: "Trash" })).toHaveFocus(),
      );
    });
  });

  describe("collapsing", () => {
    afterEach(() => localStorage.clear());

    it("hides the list behind the header toggle, says so to assistive tech, and remembers it", async () => {
      const view = render(page());
      const toggle = await screen.findByRole("button", {
        name: "Conversation list",
      });
      const sidebar = screen.getByRole("complementary", {
        name: "Conversations",
      });
      expect(toggle).toHaveAttribute("aria-expanded", "true");
      expect(toggle).toHaveAttribute("aria-controls", sidebar.id);
      await userEvent.click(toggle);
      expect(toggle).toHaveAttribute("aria-expanded", "false");
      expect(sidebar).not.toBeVisible();
      expect(toggle).toHaveAttribute("title", "Show the conversation list");
      view.unmount();

      render(page());
      expect(
        await screen.findByRole("button", { name: "Conversation list" }),
      ).toHaveAttribute("aria-expanded", "false");
      expect(
        screen.queryByRole("complementary", { name: "Conversations" }),
      ).toBeNull();
    });

    it("still works when storage can't be used", async () => {
      const read = vi
        .spyOn(Storage.prototype, "getItem")
        .mockImplementation(() => {
          throw new Error("blocked");
        });
      const write = vi
        .spyOn(Storage.prototype, "setItem")
        .mockImplementation(() => {
          throw new Error("blocked");
        });
      try {
        render(page());
        const toggle = await screen.findByRole("button", {
          name: "Conversation list",
        });
        expect(toggle).toHaveAttribute("aria-expanded", "true");
        await userEvent.click(toggle);
        expect(toggle).toHaveAttribute("aria-expanded", "false");
      } finally {
        read.mockRestore();
        write.mockRestore();
      }
    });
  });
});

describe("focus after a button's action is over", () => {
  // A browser drops the focus of a button that is disabled while its action
  // runs; jsdom doesn't, and can't blur a disabled button, so move it to the
  // body instead.
  const dropFocus = () => {
    document.body.tabIndex = -1;
    document.body.focus();
    document.body.removeAttribute("tabindex");
    expect(document.body).toHaveFocus();
  };

  it("gives it back to Export Markdown once the action was cancelled or failed", async () => {
    await openConversation();
    for (const outcome of [
      () => false,
      () => Promise.reject("assistant_export_failed"),
    ]) {
      mocks.exportMarkdown.mockImplementationOnce(async () => {
        dropFocus();
        return outcome();
      });
      const button = screen.getByRole("button", { name: "Export Markdown" });
      const calls = mocks.exportMarkdown.mock.calls.length;
      await userEvent.click(button);
      await waitFor(() =>
        expect(mocks.exportMarkdown).toHaveBeenCalledTimes(calls + 1),
      );
      await waitFor(() => expect(button).toBeEnabled());
      await waitFor(() => expect(button).toHaveFocus());
    }
  });

  it.each([
    ["Back up…", "backup", "assistant_backup_failed"],
    ["Restore…", "restore", "assistant_backup_invalid"],
  ] as const)(
    "gives it back to the menu button once %s was cancelled or failed",
    async (item, command, failure) => {
      await openConversation();
      for (const outcome of [() => null, () => Promise.reject(failure)]) {
        mocks[command].mockImplementationOnce(async () => {
          dropFocus();
          return outcome();
        });
        const calls = mocks[command].mock.calls.length;
        await chooseFromMenu(item);
        await waitFor(() =>
          expect(mocks[command]).toHaveBeenCalledTimes(calls + 1),
        );
        await waitFor(() =>
          expect(
            screen.getByRole("button", { name: "More actions" }),
          ).toHaveFocus(),
        );
      }
    },
  );

  it("gives it back to Move to trash when moving the conversation failed", async () => {
    await openConversation();
    mocks.updateSession.mockImplementation(async (patch) => {
      if (patch.deleted === undefined) return current.session;
      dropFocus();
      throw "assistant_storage_unavailable";
    });
    const button = screen.getByRole("button", { name: "Move to trash" });
    await userEvent.click(button);
    await screen.findByRole("alert");
    await waitFor(() => expect(button).toBeEnabled());
    await waitFor(() => expect(button).toHaveFocus());
  });

  it("leaves focus where the user has put it meanwhile", async () => {
    await openConversation();
    mocks.exportMarkdown.mockImplementation(async () => {
      composer().focus();
      return false;
    });
    const button = screen.getByRole("button", { name: "Export Markdown" });
    await userEvent.click(button);
    await waitFor(() => expect(mocks.exportMarkdown).toHaveBeenCalled());
    await waitFor(() => expect(button).toBeEnabled());
    expect(composer()).toHaveFocus();
  });
});

describe("a pick that imported files", () => {
  it("keeps the conversation it created when saving the draft after the import fails", async () => {
    const pdf: AssistantAttachment = {
      id: "file-pdf",
      name: "report.pdf",
      mimeType: "application/pdf",
      sizeBytes: 2048,
      status: "ready",
      reason: null,
      previewUrl: null,
      details: null,
    };
    mocks.listSessions.mockResolvedValue(listOf([]));
    mocks.createSession.mockImplementation(async (request) => ({
      ...session,
      title: "",
      ...request,
    }));
    mocks.updateSession.mockImplementation(async (patch) => {
      if (patch.draftAttachmentIds?.length)
        throw "assistant_storage_unavailable";
      return current.session;
    });
    mocks.pickAttachments.mockResolvedValueOnce([pdf]);
    render(page());
    const attach = screen.getByRole("button", { name: "Attach files" });
    await waitFor(() => expect(attach).toBeEnabled());
    await userEvent.click(attach);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "couldn't be read or saved",
    );
    expect(mocks.updateSession).not.toHaveBeenCalledWith(
      expect.objectContaining({ deleted: true }),
    );
    expect(
      within(composer().closest("footer") as HTMLElement).getByRole("list", {
        name: "Attached files",
      }),
    ).toHaveTextContent("report.pdf");
  });
});
