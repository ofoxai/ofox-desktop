import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { ShellProvider } from "@/components/shell/ShellContext";
import { InteractionScope } from "@/components/ui/interaction-scope";
import type {
  AssistantBootstrap,
  AssistantEvent,
  AssistantModel,
  AssistantSession,
  AssistantSessionDetail,
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
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));
vi.mock("sonner", () => ({
  toast: { success: vi.fn(), warning: vi.fn(), error: vi.fn() },
}));

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
  models: [model("balanced", "Balanced model")],
  defaultModelId: "balanced",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: { available: false, unitPriceUsd: "0", model: "" },
};
let current: AssistantSessionDetail;

function page(active = true) {
  return (
    <InteractionScope active={active}>
      <ShellProvider openSettings={() => undefined}>
        <AssistantPage />
      </ShellProvider>
    </InteractionScope>
  );
}
const composer = () =>
  screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;
async function openConversation(active = true) {
  const rendered = render(page(active));
  await userEvent.click(
    await screen.findByRole("button", { name: /My conversation/ }),
  );
  await screen.findByRole("combobox", { name: "Choose model" });
  await waitFor(() => expect(composer()).toBeEnabled());
  return rendered;
}
/** Send a question and wait until the reply is being written. */
async function startReply() {
  await userEvent.type(composer(), "Hello");
  await userEvent.click(screen.getByRole("button", { name: "Send" }));
  await screen.findByRole("button", { name: "Stop" });
  return mocks.send.mock.calls[0][0] as {
    sessionId: string;
    runId: string;
    operationId: string;
  };
}

/** The user agent the page sees: ⌘ is the shortcut key only on macOS. */
const platformSpies: { mockRestore: () => void }[] = [];
function runOn(system: "mac" | "other") {
  platformSpies.push(
    vi
      .spyOn(navigator, "userAgent", "get")
      .mockReturnValue(system === "mac" ? "Macintosh" : "Windows NT 10.0"),
    vi
      .spyOn(navigator, "platform", "get")
      .mockReturnValue(system === "mac" ? "MacIntel" : "Win32"),
  );
}

/** What the app's own window listener sees of each key (App.tsx's Escape). */
let seenByApp: KeyboardEvent[] = [];
const recordKey = (event: KeyboardEvent) => seenByApp.push(event);

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  seenByApp = [];
  window.addEventListener("keydown", recordKey);
  current = {
    session: { ...session },
    messages: [],
    attachments: [],
    usage: {},
  };
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.listSessions.mockImplementation(async () => ({
    sessions: [current.session],
    unreadable: [],
    trashCount: 0,
  }));
  mocks.createSession.mockImplementation(async () => ({
    ...session,
    id: "session-b",
    title: "",
  }));
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
  mocks.onAuthChange.mockImplementation(async () => () => undefined);
});
afterEach(() => {
  window.removeEventListener("keydown", recordKey);
  for (const spy of platformSpies.splice(0)) spy.mockRestore();
});

describe("New conversation shortcut", () => {
  it.each([
    ["Command+N on macOS", "mac", { metaKey: true }],
    ["Ctrl+N elsewhere", "other", { ctrlKey: true }],
  ] as const)("starts a conversation with %s", async (_, system, modifier) => {
    runOn(system);
    await openConversation();
    const prevented = !fireEvent.keyDown(composer(), { key: "n", ...modifier });
    expect(prevented).toBe(true);
    await waitFor(() => expect(mocks.createSession).toHaveBeenCalledOnce());
    // Ready to type into.
    await waitFor(() => expect(composer()).toHaveFocus());
  });

  it.each([
    ["Ctrl+N on macOS, where it moves the cursor to the next line", "mac", { ctrlKey: true }],
    ["Command+N elsewhere, where the Windows key is not ours", "other", { metaKey: true }],
  ] as const)("is not %s", async (_, system, modifier) => {
    runOn(system);
    await openConversation();
    expect(fireEvent.keyDown(composer(), { key: "n", ...modifier })).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
  });

  it("ignores a key that is held down", async () => {
    await openConversation();
    expect(
      fireEvent.keyDown(composer(), { key: "n", ctrlKey: true, repeat: true }),
    ).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
  });

  it("ignores a key pressed in a menu, which is drawn outside the workspace", async () => {
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Conversation actions" }),
    );
    const item = await screen.findByRole("menuitem", { name: "Rename" });
    expect(fireEvent.keyDown(item, { key: "n", ctrlKey: true })).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
  });

  it("leaves a title being edited alone instead of dropping the edit", async () => {
    await openConversation();
    await userEvent.dblClick(screen.getByRole("heading", { level: 1 }));
    const title = await screen.findByRole("textbox", {
      name: "Conversation title",
    });
    expect(fireEvent.keyDown(title, { key: "n", ctrlKey: true })).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
    expect(title).toBeInTheDocument();
  });

  it("leaves a message being edited alone, wherever the key is pressed", async () => {
    current.messages = [
      {
        id: "original",
        role: "user",
        content: "Original question",
        createdAt: 1,
        nativeEntryId: "entry-original",
        runId: "run-original",
      },
    ];
    await openConversation();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    const editor = screen.getByRole("textbox", { name: "Edit and resend" });
    expect(fireEvent.keyDown(editor, { key: "n", ctrlKey: true })).toBe(true);
    // Not from the conversation list either.
    const row = screen.getByRole("button", { name: /My conversation/ });
    expect(fireEvent.keyDown(row, { key: "n", ctrlKey: true })).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
    expect(editor).toBeInTheDocument();
    // Once the edit is over the shortcut works again.
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(fireEvent.keyDown(composer(), { key: "n", ctrlKey: true })).toBe(
      false,
    );
    await waitFor(() => expect(mocks.createSession).toHaveBeenCalledOnce());
  });

  it("works from anywhere in the workspace, such as the conversation list", async () => {
    runOn("mac");
    await openConversation();
    const row = screen.getByRole("button", { name: /My conversation/ });
    row.focus();
    fireEvent.keyDown(row, { key: "N", metaKey: true });
    await waitFor(() => expect(mocks.createSession).toHaveBeenCalledOnce());
  });

  it("leaves other combinations alone", async () => {
    await openConversation();
    for (const combination of [
      { key: "n" },
      { key: "n", ctrlKey: true, shiftKey: true },
      { key: "n", ctrlKey: true, altKey: true },
      { key: "m", ctrlKey: true },
    ])
      expect(fireEvent.keyDown(composer(), combination)).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
  });

  it.each([
    ["isComposing", { isComposing: true }],
    ["keyCode 229", { keyCode: 229 }],
  ])("is not for a key that belongs to an input method (%s)", async (_, ime) => {
    await openConversation();
    expect(
      fireEvent.keyDown(composer(), { key: "n", ctrlKey: true, ...ime }),
    ).toBe(true);
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
  });

  it("does nothing while the page is not the one on screen", async () => {
    await openConversation(false);
    expect(fireEvent.keyDown(composer(), { key: "n", ctrlKey: true })).toBe(
      true,
    );
    await act(async () => undefined);
    expect(mocks.createSession).not.toHaveBeenCalled();
  });
});

describe("Escape while a reply is running", () => {
  it("stops the reply, and the app's own Escape does not follow", async () => {
    await openConversation();
    const sent = await startReply();
    composer().focus();
    const prevented = !fireEvent.keyDown(composer(), { key: "Escape" });
    expect(prevented).toBe(true);
    expect(mocks.stop).toHaveBeenCalledOnce();
    expect(mocks.stop).toHaveBeenCalledWith(
      sent.sessionId,
      sent.runId,
      sent.operationId,
    );
    // What App.tsx's handler sees: handled, so it keeps the page.
    expect(seenByApp.at(-1)?.defaultPrevented).toBe(true);
    await waitFor(() => expect(composer()).toHaveFocus());
  });

  it("works from other places in the workspace", async () => {
    await openConversation();
    await startReply();
    const header = screen.getByRole("heading", { level: 1 });
    expect(fireEvent.keyDown(header, { key: "Escape" })).toBe(false);
    expect(mocks.stop).toHaveBeenCalledOnce();
  });

  it("leaves Escape to the app when no reply is running", async () => {
    await openConversation();
    expect(fireEvent.keyDown(composer(), { key: "Escape" })).toBe(true);
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(seenByApp.at(-1)?.defaultPrevented).toBe(false);
  });

  it("leaves Escape to the app once the reply is over", async () => {
    await openConversation();
    const sent = await startReply();
    current = {
      ...current,
      messages: [
        { id: "q", role: "user", content: "Hello", createdAt: 1 },
        {
          id: "a",
          role: "assistant",
          content: "Hi",
          status: "completed",
          createdAt: 2,
        },
      ],
    };
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "run_end",
        data: { status: "completed" },
      } as unknown as AssistantEvent),
    );
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Stop" })).toBeNull(),
    );
    expect(fireEvent.keyDown(composer(), { key: "Escape" })).toBe(true);
    expect(mocks.stop).not.toHaveBeenCalled();
  });

  it.each([
    ["isComposing", { isComposing: true }],
    ["keyCode 229", { keyCode: 229 }],
  ])(
    "does not stop it for the Escape that cancels an input method candidate (%s)",
    async (_, ime) => {
      await openConversation();
      await startReply();
      expect(fireEvent.keyDown(composer(), { key: "Escape", ...ime })).toBe(
        true,
      );
      expect(mocks.stop).not.toHaveBeenCalled();
    },
  );

  it("does not stop it when something inside already used the Escape", async () => {
    await openConversation();
    await startReply();
    // The title field drops its edit with Escape.
    await userEvent.dblClick(screen.getByRole("heading", { level: 1 }));
    const title = await screen.findByRole("textbox", {
      name: "Conversation title",
    });
    expect(fireEvent.keyDown(title, { key: "Escape" })).toBe(false);
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(await screen.findByRole("heading", { level: 1 })).toBeVisible();
  });

  it("does not stop it for an Escape pressed outside the workspace", async () => {
    await openConversation();
    await startReply();
    const outside = document.body.appendChild(document.createElement("button"));
    fireEvent.keyDown(outside, { key: "Escape" });
    outside.remove();
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(seenByApp.at(-1)?.defaultPrevented).toBe(false);
  });

  it("still stops it after a click on the reply's own text, which has no focus to give", async () => {
    await openConversation();
    const sent = await startReply();
    act(() =>
      mocks.listener?.({
        ...sent,
        type: "text_delta",
        data: { delta: "Some reply words", messageId: "reply" },
      } as unknown as AssistantEvent),
    );
    await userEvent.click(await screen.findByText("Some reply words"));
    // Focus stays in the workspace instead of falling to the page.
    expect(document.activeElement).not.toBe(document.body);
    await userEvent.keyboard("{Escape}");
    expect(mocks.stop).toHaveBeenCalledOnce();
    expect(seenByApp.at(-1)?.defaultPrevented).toBe(true);
  });

  it("stops it with the first Escape while the Stop button's own tooltip is open", async () => {
    await openConversation();
    await startReply();
    // Keyboard focus opens the tooltip, which would otherwise take the Escape.
    act(() => screen.getByRole("button", { name: "Stop" }).focus());
    await screen.findByRole("tooltip");
    await userEvent.keyboard("{Escape}");
    expect(mocks.stop).toHaveBeenCalledOnce();
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  });

  it.each([
    ["isComposing", { isComposing: true }],
    ["keyCode 229", { keyCode: 229 }],
  ])(
    "does not stop it for the Escape that cancels an input method candidate while the tooltip is open (%s)",
    async (_, ime) => {
      await openConversation();
      await startReply();
      const stop = screen.getByRole("button", { name: "Stop" });
      act(() => stop.focus());
      await screen.findByRole("tooltip");
      fireEvent.keyDown(stop, { key: "Escape", ...ime });
      expect(mocks.stop).not.toHaveBeenCalled();
    },
  );

  it("does nothing while the page is not the one on screen", async () => {
    const rendered = await openConversation();
    await startReply();
    rendered.rerender(page(false));
    expect(fireEvent.keyDown(composer(), { key: "Escape" })).toBe(true);
    expect(mocks.stop).not.toHaveBeenCalled();
  });
});
