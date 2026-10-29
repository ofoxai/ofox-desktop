import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import type { ReactNode } from "react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { useAssistant } from "@/components/assistant/useAssistant";
import { MAX_QUEUED_MESSAGES } from "@/components/assistant/useMessageQueue";
import { InteractionScope } from "@/components/ui/interaction-scope";
import { ShellProvider } from "@/components/shell/ShellContext";
import type {
  AssistantBootstrap,
  AssistantMessage,
  AssistantEvent,
  AssistantRunRef,
  AssistantSendRequest,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  getPreferences: vi.fn(),
  send: vi.fn(),
  stop: vi.fn(),
  steer: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
  onDrag: vi.fn(),
  onDrop: vi.fn(),
  listener: undefined as ((event: AssistantEvent) => void) | undefined,
  auth: undefined as ((state: "expired" | "restored") => void) | undefined,
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(false),
  requestPermission: vi.fn().mockResolvedValue("denied"),
  sendNotification: vi.fn(),
}));

const initialRun: AssistantRunRef = {
  runId: "initial-run",
  operationId: "initial-operation",
};
const bootstrap: AssistantBootstrap = {
  owner: "a".repeat(64),
  authorityGeneration: 1,
  runtime: { available: true, version: "test", reason: null },
  models: [
    {
      id: "chat-model",
      name: "Chat model",
      kind: "chat",
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
    },
  ],
  defaultModelId: "chat-model",
  modelUnavailableReason: null,
  key: { status: "ready" },
};
let current: AssistantSessionDetail;

function page(active = true) {
  return (
    <ShellProvider openSettings={() => undefined}>
      <button>Other page control</button>
      <InteractionScope active={active}>
        <AssistantPage />
      </InteractionScope>
    </ShellProvider>
  );
}
const composer = () => screen.getByRole("textbox", { name: "Message" });
const queue = () => screen.getByRole("region", { name: "Message queue" });

async function openConversation() {
  const view = render(page());
  await userEvent.click(
    await screen.findByRole("button", { name: /Queue conversation/ }),
  );
  await screen.findByRole("button", { name: "Stop" });
  return view;
}

function enqueue(text: string) {
  fireEvent.change(composer(), { target: { value: text } });
  fireEvent.keyDown(composer(), { key: "Enter" });
}

function finish(
  status: "completed" | "stopped" | "error" | "queued" = "completed",
  reference = current.session.activeRun!,
) {
  current = {
    ...current,
    session: { ...current.session, activeRun: null },
    messages: current.messages.map((message) =>
      message.status === "streaming" ? { ...message, status } : message,
    ),
  };
  act(() =>
    mocks.listener?.({
      ...reference,
      sessionId: current.session.id,
      type: "run_end",
      data: { stopped: status === "stopped", status },
    }),
  );
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

async function openHook(
  wrapper?: (props: { children: ReactNode }) => JSX.Element,
) {
  const hook = renderHook(() => useAssistant(true), { wrapper });
  await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
  act(() => hook.result.current.select(current.session.id));
  await waitFor(() => expect(hook.result.current.loading).toBe(false));
  return hook;
}

function addToHook(
  hook: { result: { current: ReturnType<typeof useAssistant> } },
  text = "Queued question",
) {
  act(() => {
    hook.result.current.setDraft(text);
    expect(hook.result.current.enqueue()).toBe(true);
  });
}

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  current = {
    session: {
      id: "session-queue",
      title: "Queue conversation",
      titleSource: "user",
      modelId: "chat-model",
      webSearch: false,
      draft: "",
      draftAttachmentIds: [],
      createdAt: 1,
      updatedAt: 1,
      deletedAt: null,
      pinnedAt: null,
      activeRun: initialRun,
    },
    messages: [
      {
        id: "first-question",
        runId: initialRun.runId,
        role: "user",
        content: "First question",
        createdAt: 1,
      },
      {
        id: "first-answer",
        runId: initialRun.runId,
        role: "assistant",
        content: "Answer in progress",
        createdAt: 2,
        status: "streaming",
      },
    ],
    attachments: [],
    usage: {},
  };
  mocks.bootstrap.mockResolvedValue(bootstrap);
  mocks.getPreferences.mockResolvedValue({
    instructions: "",
    autoTitle: false,
    notifyOnFinish: false,
  });
  mocks.listSessions.mockImplementation(async () => ({
    sessions: [current.session],
    unreadable: [],
    trashCount: 0,
    matches: {},
  }));
  mocks.getSession.mockImplementation(async () => current);
  mocks.updateSession.mockImplementation(async (patch) => {
    current = { ...current, session: { ...current.session, ...patch } };
    return current.session;
  });
  mocks.send.mockImplementation(async (request: AssistantSendRequest) => {
    const reference = {
      runId: request.runId,
      operationId: request.operationId,
    };
    current = {
      ...current,
      session: { ...current.session, activeRun: reference },
      messages: [
        ...current.messages,
        {
          id: `question-${request.runId}`,
          runId: request.runId,
          role: "user",
          content: request.text,
          createdAt: 3,
        },
        {
          id: `answer-${request.runId}`,
          runId: request.runId,
          role: "assistant",
          content: "Next answer",
          status: "streaming",
          createdAt: 4,
        },
      ],
    };
    return reference;
  });
  mocks.stop.mockResolvedValue(undefined);
  mocks.subscribe.mockImplementation(async (listener) => {
    mocks.listener = listener;
    return () => undefined;
  });
  mocks.onAuthChange.mockImplementation(async (listener) => {
    mocks.auth = listener;
    return () => undefined;
  });
  mocks.onDrag.mockResolvedValue(() => undefined);
  mocks.onDrop.mockResolvedValue(() => undefined);
});

describe("ordinary chat queue", () => {
  it("interrupts the running sidecar without consuming its waiting queue", async () => {
    mocks.steer.mockResolvedValue(undefined);
    await openConversation();
    enqueue("Later queued message");
    fireEvent.change(composer(), { target: { value: "Change direction now" } });
    await userEvent.click(screen.getByRole("button", { name: "Interrupt" }));
    await waitFor(() => expect(mocks.steer).toHaveBeenCalledOnce());
    expect(mocks.steer).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionId: current.session.id,
        ...initialRun,
        text: "Change direction now",
        expectedOwner: bootstrap.owner,
        expectedGeneration: 1,
        id: expect.any(String),
      }),
    );
    expect(mocks.send).not.toHaveBeenCalled();
    expect(within(queue()).getByText("Later queued message")).toBeVisible();
    await waitFor(() => expect(composer()).toHaveValue(""));
  });

  it("keeps a refused interruption draft and never retries it", async () => {
    mocks.steer.mockRejectedValue("assistant_session_busy");
    await openConversation();
    fireEvent.change(composer(), {
      target: { value: "Keep this interruption" },
    });
    await userEvent.click(screen.getByRole("button", { name: "Interrupt" }));
    await waitFor(() => expect(mocks.steer).toHaveBeenCalledOnce());
    expect(composer()).toHaveValue("Keep this interruption");
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("rotates streaming replies when the sidecar consumes an interruption", async () => {
    const hook = await openHook();
    act(() => hook.result.current.setDraft("An unrelated newer draft"));
    act(() =>
      mocks.listener?.({
        sessionId: current.session.id,
        ...initialRun,
        type: "thinking_start",
        data: { messageId: "first-answer" },
      }),
    );
    expect(hook.result.current.thinking).not.toBeNull();
    const previous: AssistantMessage = {
      ...current.messages[1],
      status: "completed",
    };
    const user: AssistantMessage = {
      id: "interrupted-user",
      role: "user",
      runId: "child",
      content: "New direction",
      createdAt: 3,
      status: "completed",
    };
    const reply: AssistantMessage = {
      id: "interrupted-reply",
      role: "assistant",
      runId: "child",
      content: "",
      createdAt: 4,
      status: "streaming",
    };
    act(() =>
      mocks.listener?.({
        sessionId: current.session.id,
        ...initialRun,
        type: "steer_applied",
        data: { messageId: reply.id, messages: [previous, user, reply] },
      }),
    );
    expect(hook.result.current.detail?.messages.map((m) => m.id)).toEqual([
      "first-question",
      "first-answer",
      user.id,
    ]);
    expect(hook.result.current.thinking).toBeNull();
    expect(hook.result.current.stream?.id).toBe(reply.id);
    expect(hook.result.current.draft).toBe("An unrelated newer draft");
    act(() =>
      mocks.listener?.({
        sessionId: current.session.id,
        ...initialRun,
        type: "text_delta",
        data: { delta: "New response", messageId: reply.id },
      }),
    );
    await waitFor(() =>
      expect(hook.result.current.stream?.content).toBe("New response"),
    );
  });

  it("preserves text typed while an interruption is being admitted", async () => {
    const pending = deferred<null>();
    mocks.steer.mockReturnValueOnce(pending.promise);
    const hook = await openHook();
    act(() => hook.result.current.setDraft("Interrupt now"));
    let admitted: Promise<boolean>;
    act(() => {
      admitted = hook.result.current.steer();
    });
    await waitFor(() => expect(mocks.steer).toHaveBeenCalledOnce());
    act(() => hook.result.current.setDraft("New draft"));
    await act(async () => {
      pending.resolve(null);
      await admitted!;
    });
    expect(hook.result.current.draft).toBe("New draft");
  });

  it("queues with Enter and the queue button, then sends one ordinary run at a time in order", async () => {
    await openConversation();
    enqueue("Second question");
    expect(within(queue()).getByText("Second question")).toBeVisible();
    expect(composer()).toHaveValue("");
    fireEvent.change(composer(), { target: { value: "Third question" } });
    await userEvent.click(
      screen.getByRole("button", { name: "Queue message" }),
    );
    expect(within(queue()).getByText("Third question")).toBeVisible();
    expect(mocks.send).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Stop" })).toBeEnabled();

    finish();
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(1));
    expect(mocks.send.mock.calls[0][0]).toMatchObject({
      sessionId: current.session.id,
      text: "Second question",
      attachmentIds: [],
      expectedOwner: bootstrap.owner,
      expectedGeneration: bootstrap.authorityGeneration,
    });
    await waitFor(() =>
      expect(within(queue()).queryByText("Second question")).toBeNull(),
    );
    expect(within(queue()).getByText("Third question")).toBeVisible();
    finish();
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    expect(mocks.send.mock.calls[1][0].text).toBe("Third question");
    expect(mocks.send.mock.calls[1][0].runId).not.toBe(
      mocks.send.mock.calls[0][0].runId,
    );
  });

  it("pauses immediately on Stop even when a successful completion wins the backend race", async () => {
    await openConversation();
    enqueue("Do not send after Stop");
    await userEvent.click(screen.getByRole("button", { name: "Stop" }));
    finish();
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(),
    );
    expect(mocks.send).not.toHaveBeenCalled();
    expect(within(queue()).getByText("Queue paused")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Resume queue" }));
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
  });

  it("keeps a queue paused after leaving the retained assistant page and returning", async () => {
    const view = await openConversation();
    enqueue("Wait for my return");
    view.rerender(page(false));
    finish();
    await act(async () => undefined);
    view.rerender(page());
    expect(mocks.send).not.toHaveBeenCalled();
    expect(within(queue()).getByText("Queue paused")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Resume queue" }));
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
  });

  it.each(["stopped", "error", "queued"] as const)(
    "does not drain after a %s run end",
    async (status) => {
      const hook = await openHook();
      addToHook(hook);
      finish(status);
      await waitFor(() => expect(hook.result.current.stream).toBeNull());
      expect(hook.result.current.queue?.paused).toBe(true);
      expect(hook.result.current.queue?.messages).toHaveLength(1);
      expect(mocks.send).not.toHaveBeenCalled();
    },
  );

  it("pauses on an error event even if the eventual end claims success", async () => {
    const hook = await openHook();
    addToHook(hook);
    act(() =>
      mocks.listener?.({
        ...initialRun,
        sessionId: current.session.id,
        type: "error",
        data: { code: "assistant_timeout" },
      }),
    );
    expect(hook.result.current.queue?.paused).toBe(true);
    finish();
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("ignores another run's events and duplicate completion while history is loading", async () => {
    const hook = await openHook();
    addToHook(hook, "Only once");
    act(() =>
      mocks.listener?.({
        ...initialRun,
        operationId: "another-operation",
        sessionId: current.session.id,
        type: "run_end",
        data: { status: "completed", stopped: false },
      }),
    );
    expect(mocks.send).not.toHaveBeenCalled();
    const history = deferred<AssistantSessionDetail>();
    mocks.getSession.mockReturnValueOnce(history.promise);
    finish();
    finish("completed", initialRun);
    expect(mocks.send).not.toHaveBeenCalled();
    await act(async () => history.resolve(current));
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
  });

  it("requires explicit resume after the completed history could not be read", async () => {
    const hook = await openHook();
    addToHook(hook);
    mocks.getSession.mockRejectedValueOnce("assistant_storage_unavailable");
    finish();
    await waitFor(() => expect(hook.result.current.queue?.paused).toBe(true));
    expect(mocks.send).not.toHaveBeenCalled();
    await act(async () => hook.result.current.resumeQueue());
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
  });

  it("keeps new submissions behind an idle paused queue rather than overtaking it", async () => {
    const hook = await openHook();
    addToHook(hook, "Older entry");
    finish("stopped");
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    addToHook(hook, "Newer entry");
    expect(mocks.send).not.toHaveBeenCalled();
    expect(hook.result.current.queue?.paused).toBe(true);
    await act(async () => hook.result.current.resumeQueue());
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
    expect(mocks.send.mock.calls[0][0].text).toBe("Older entry");
  });

  it("leaves the composer's newer draft untouched while an entry is automatically sent", async () => {
    const hook = await openHook();
    addToHook(hook);
    act(() => hook.result.current.setDraft("Still writing this"));
    finish();
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
    await waitFor(() => expect(hook.result.current.busy).toBe(false));
    expect(hook.result.current.draft).toBe("Still writing this");
    expect(current.session.draft).toBe("Still writing this");
    expect(mocks.send.mock.calls[0][0].text).toBe("Queued question");
  });

  it("keeps a refused entry in place, pauses its siblings and never retries automatically", async () => {
    const hook = await openHook();
    addToHook(hook, "First queued");
    addToHook(hook, "Second queued");
    mocks.send.mockRejectedValueOnce("assistant_key_unavailable");
    finish();
    await waitFor(() => expect(hook.result.current.queue?.paused).toBe(true));
    expect(
      hook.result.current.queue?.messages.map((item) => item.text),
    ).toEqual(["First queued", "Second queued"]);
    expect(mocks.send).toHaveBeenCalledOnce();
    await act(async () => hook.result.current.resumeQueue());
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    expect(mocks.send.mock.calls[1][0].text).toBe("First queued");
  });

  it("does not requeue an admitted run when it fails, and pauses only the remaining entries", async () => {
    const hook = await openHook();
    addToHook(hook, "Already admitted");
    addToHook(hook, "Not admitted yet");
    finish();
    await waitFor(() =>
      expect(hook.result.current.queue?.messages).toHaveLength(1),
    );
    finish("error");
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    expect(hook.result.current.queue?.messages[0].text).toBe(
      "Not admitted yet",
    );
    expect(hook.result.current.queue?.paused).toBe(true);
    expect(mocks.send).toHaveBeenCalledOnce();
  });

  it("handles a completed run arriving before the send acknowledgement without skipping an entry", async () => {
    const hook = await openHook();
    addToHook(hook, "Quick answer");
    addToHook(hook, "Following answer");
    const admit = mocks.send.getMockImplementation()!;
    const acknowledgement = deferred<void>();
    mocks.send.mockImplementationOnce(async (request) => {
      const reference = await admit(request);
      finish("completed", reference);
      await acknowledgement.promise;
      return reference;
    });
    finish();
    await waitFor(() => expect(hook.result.current.run).toBeNull());
    await waitFor(() => expect(hook.result.current.queue?.ready).toBe(true));
    expect(mocks.send).toHaveBeenCalledOnce();
    await act(async () => acknowledgement.resolve());
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    expect(mocks.send.mock.calls.map(([request]) => request.text)).toEqual([
      "Quick answer",
      "Following answer",
    ]);
  });

  it("pauses before editing, preserves queue order and leaves the ordinary draft alone", async () => {
    await openConversation();
    enqueue("First queued");
    enqueue("Second queued");
    fireEvent.change(composer(), {
      target: { value: "Unsent composer draft" },
    });
    await userEvent.click(
      screen.getByRole("button", { name: "Edit queued message 2" }),
    );
    const edit = screen.getByRole("textbox", { name: "Edit queued message" });
    expect(edit).toHaveFocus();
    fireEvent.change(edit, { target: { value: "Edited second" } });
    finish();
    await act(async () => undefined);
    expect(mocks.send).not.toHaveBeenCalled();
    fireEvent.keyDown(edit, { key: "Enter" });
    expect(composer()).toHaveValue("Unsent composer draft");
    expect(composer()).toHaveFocus();
    expect(
      within(queue())
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["1First queued", "2Edited second"]);
    await userEvent.click(screen.getByRole("button", { name: "Resume queue" }));
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
    expect(mocks.send.mock.calls[0][0].text).toBe("First queued");
    finish();
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    expect(mocks.send.mock.calls[1][0].text).toBe("Edited second");
  });

  it("Escape cancels a queue edit without stopping the running reply", async () => {
    await openConversation();
    enqueue("Keep this version");
    await userEvent.click(
      screen.getByRole("button", { name: "Edit queued message 1" }),
    );
    const edit = screen.getByRole("textbox", { name: "Edit queued message" });
    fireEvent.change(edit, { target: { value: "Abandoned edit" } });
    fireEvent.keyDown(edit, { key: "Escape" });
    expect(within(queue()).getByText("Keep this version")).toBeVisible();
    expect(composer()).toHaveFocus();
    expect(mocks.stop).not.toHaveBeenCalled();
  });

  it("removes or clears unsent entries without changing the draft or sending anything", async () => {
    await openConversation();
    enqueue("Remove me");
    enqueue("Clear me");
    fireEvent.change(composer(), { target: { value: "Keep my draft" } });
    await userEvent.click(
      screen.getByRole("button", { name: "Remove queued message 1" }),
    );
    expect(within(queue()).queryByText("Remove me")).toBeNull();
    expect(within(queue()).getByText("Clear me")).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Clear queue" }));
    expect(screen.queryByRole("region", { name: "Message queue" })).toBeNull();
    expect(composer()).toHaveValue("Keep my draft");
    expect(composer()).toHaveFocus();
    finish();
    await act(async () => undefined);
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("bounds queue length, keeps refused text, and respects Shift+Enter and IME Enter", async () => {
    await openConversation();
    fireEvent.change(composer(), { target: { value: "Not submitted" } });
    for (const key of [
      { shiftKey: true },
      { isComposing: true },
      { keyCode: 229 },
    ])
      fireEvent.keyDown(composer(), { key: "Enter", ...key });
    expect(screen.queryByRole("region", { name: "Message queue" })).toBeNull();
    for (let index = 0; index < MAX_QUEUED_MESSAGES; index += 1)
      enqueue(`Question ${index + 1}`);
    enqueue("Keep overflow text");
    expect(within(queue()).getAllByRole("listitem")).toHaveLength(
      MAX_QUEUED_MESSAGES,
    );
    expect(composer()).toHaveValue("Keep overflow text");
    const add = screen.getByRole("button", { name: "Queue message" });
    expect(add).toBeDisabled();
    expect(
      document.getElementById(add.getAttribute("aria-describedby")!),
    ).toHaveTextContent("The queue holds up to 10 messages");
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("rejects oversized UTF-8 text without discarding it", async () => {
    const hook = await openHook();
    const text = "界".repeat(42_667);
    act(() => hook.result.current.setDraft(text));
    act(() => expect(hook.result.current.enqueue()).toBe(false));
    expect(hook.result.current.draft).toBe(text);
    expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
  });

  it("pauses when leaving and returning to the same conversation during a history refresh", async () => {
    const hook = await openHook();
    addToHook(hook);
    const history = deferred<AssistantSessionDetail>();
    mocks.getSession.mockReturnValueOnce(history.promise);
    finish();
    act(() => hook.result.current.select(null));
    act(() => hook.result.current.select(current.session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    await act(async () => history.resolve(current));
    expect(mocks.send).not.toHaveBeenCalled();
    expect(hook.result.current.queue?.paused).toBe(true);
    await act(async () => hook.result.current.resumeQueue());
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
  });

  it("revokes a pending resume when the page is hidden, even if it is reopened before the read finishes", async () => {
    let active = true;
    const hook = await openHook(({ children }) => (
      <InteractionScope active={active}>{children}</InteractionScope>
    ));
    addToHook(hook);
    finish("stopped");
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    const history = deferred<AssistantSessionDetail>();
    mocks.getSession.mockReturnValueOnce(history.promise);
    let resuming!: Promise<void>;
    act(() => {
      resuming = hook.result.current.resumeQueue();
    });
    active = false;
    hook.rerender();
    active = true;
    hook.rerender();
    await act(async () => {
      history.resolve(current);
      await resuming;
    });
    expect(mocks.send).not.toHaveBeenCalled();
    expect(hook.result.current.queue?.paused).toBe(true);
  });

  it.each(["expired", "restored"] as const)(
    "clears all old queued work on auth %s, including delayed history callbacks",
    async (state) => {
      const hook = await openHook();
      addToHook(hook, "Old account text");
      const history = deferred<AssistantSessionDetail>();
      mocks.getSession.mockReturnValueOnce(history.promise);
      finish();
      mocks.bootstrap.mockResolvedValue({
        ...bootstrap,
        owner: "b".repeat(64),
        authorityGeneration: 2,
      });
      act(() => mocks.auth?.(state));
      if (state === "expired") act(() => mocks.auth?.("restored"));
      await waitFor(() =>
        expect(hook.result.current.bootstrap?.owner).toBe("b".repeat(64)),
      );
      act(() => hook.result.current.select(current.session.id));
      await waitFor(() => expect(hook.result.current.loading).toBe(false));
      await act(async () => history.resolve(current));
      expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
      expect(mocks.send).not.toHaveBeenCalled();
    },
  );

  it.each(["owner", "generation"] as const)(
    "also clears the queue when a bootstrap refresh discovers a changed %s",
    async (change) => {
      const hook = await openHook();
      addToHook(hook);
      mocks.bootstrap.mockResolvedValue({
        ...bootstrap,
        ...(change === "owner"
          ? { owner: "b".repeat(64) }
          : { authorityGeneration: 2 }),
      });
      await act(async () => {
        await hook.result.current.loadBootstrap();
      });
      finish();
      await act(async () => undefined);
      expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
      expect(mocks.send).not.toHaveBeenCalled();
    },
  );

  it("does not revive the queue when an old account's pending send is refused", async () => {
    const hook = await openHook();
    addToHook(hook, "Old head");
    addToHook(hook, "Old tail");
    const pending = deferred<AssistantRunRef>();
    mocks.send.mockReturnValueOnce(pending.promise);
    finish();
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
    act(() => mocks.auth?.("expired"));
    await act(async () => pending.reject("assistant_identity_changed"));
    expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
    expect(hook.result.current.error).toBeNull();
    expect(mocks.send).toHaveBeenCalledOnce();
  });

  it("does not dispatch after unmount or restore a queue on a fresh mount", async () => {
    const hook = await openHook();
    addToHook(hook);
    const history = deferred<AssistantSessionDetail>();
    mocks.getSession.mockReturnValueOnce(history.promise);
    finish();
    hook.unmount();
    await act(async () => history.resolve(current));
    const reopened = await openHook();
    expect(reopened.result.current.queue?.messages.length ?? 0).toBe(0);
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("does not pull focus back from another page when a cancelled resume finishes", async () => {
    const view = await openConversation();
    enqueue("Wait here");
    finish("stopped");
    const resume = screen.getByRole("button", { name: "Resume queue" });
    await waitFor(() => expect(resume).toBeEnabled());
    const history = deferred<AssistantSessionDetail>();
    mocks.getSession.mockReturnValueOnce(history.promise);
    await userEvent.click(resume);
    view.rerender(page(false));
    const elsewhere = screen.getByRole("button", {
      name: "Other page control",
    });
    act(() => elsewhere.focus());
    await act(async () => history.resolve(current));
    expect(elsewhere).toHaveFocus();
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("preserves each conversation's queue and never drains a background conversation", async () => {
    const first = current;
    const second: AssistantSessionDetail = {
      ...first,
      session: {
        ...first.session,
        id: "other-session",
        title: "Other conversation",
      },
    };
    mocks.getSession.mockImplementation(async (id) =>
      id === first.session.id ? first : second,
    );
    const hook = await openHook();
    addToHook(hook, "First conversation queue");
    act(() => hook.result.current.select(second.session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
    addToHook(hook, "Second conversation queue");
    finish(); // The first conversation completes out of view.
    expect(mocks.send).not.toHaveBeenCalled();
    expect(hook.result.current.queue?.messages[0].text).toBe(
      "Second conversation queue",
    );
    act(() => hook.result.current.select(first.session.id));
    await waitFor(() => expect(hook.result.current.loading).toBe(false));
    expect(hook.result.current.queue?.messages[0].text).toBe(
      "First conversation queue",
    );
    expect(hook.result.current.queue?.paused).toBe(true);
  });

  it("keeps a queue added after Stop paused, even when it was empty at the time of Stop", async () => {
    const hook = await openHook();
    await act(async () => hook.result.current.stop());
    addToHook(hook);
    finish();
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    expect(hook.result.current.queue?.paused).toBe(true);
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it("lets a new manual run start a fresh queue after an empty queue was paused", async () => {
    const hook = await openHook();
    await act(async () => hook.result.current.stop());
    finish("stopped");
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    act(() => hook.result.current.setDraft("Manual new question"));
    await act(async () => expect(await hook.result.current.send()).toBe(true));
    addToHook(hook, "Follow the new run");
    expect(hook.result.current.queue?.paused).toBe(false);
    finish();
    await waitFor(() => expect(mocks.send).toHaveBeenCalledTimes(2));
    expect(mocks.send.mock.calls[1][0].text).toBe("Follow the new run");
  });

  it("a repeated Resume press starts only one ordinary send", async () => {
    const hook = await openHook();
    addToHook(hook);
    finish("stopped");
    await waitFor(() => expect(hook.result.current.stream).toBeNull());
    await act(async () => {
      await Promise.all([
        hook.result.current.resumeQueue(),
        hook.result.current.resumeQueue(),
      ]);
    });
    await waitFor(() => expect(mocks.send).toHaveBeenCalledOnce());
  });

  it("does not drop attached files or text when refusing to queue an attachment draft", async () => {
    const file = {
      id: "file",
      name: "notes.txt",
      mimeType: "text/plain",
      sizeBytes: 12,
      status: "ready" as const,
      reason: null,
      details: null,
    };
    current.session.draftAttachmentIds = [file.id];
    current.attachments = [file];
    const hook = await openHook();
    act(() => hook.result.current.setDraft("Send with this file"));
    act(() => expect(hook.result.current.enqueue()).toBe(false));
    expect(hook.result.current.draft).toBe("Send with this file");
    expect(hook.result.current.attachments[0].id).toBe(file.id);
    expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
    expect(mocks.send).not.toHaveBeenCalled();
  });

  it.each(["image", "video"] as const)(
    "never queues a %s request",
    async (kind) => {
      mocks.bootstrap.mockResolvedValue({
        ...bootstrap,
        models: [{ ...bootstrap.models[0], kind }],
      });
      const hook = await openHook();
      act(() => hook.result.current.setDraft("Media prompt"));
      act(() => expect(hook.result.current.enqueue()).toBe(false));
      expect(hook.result.current.draft).toBe("Media prompt");
      expect(hook.result.current.queue?.messages.length ?? 0).toBe(0);
    },
  );
});
