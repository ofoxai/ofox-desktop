import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import {
  assistantErrorKey,
  type AssistantBootstrap,
  type AssistantMessage,
  type AssistantModel,
  type AssistantSession,
  type AssistantSessionDetail,
} from "@/lib/api/assistant";

// Conversation management on the page: pins, the undo after the trash, emptying
// the trash and the jump from a search result. The list is a small fake store
// that sorts, trashes and searches the way the backend does.

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  listSessions: vi.fn(),
  createSession: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  deleteSession: vi.fn(),
  emptyTrash: vi.fn(),
  exportMarkdown: vi.fn(),
  usageSummary: vi.fn(),
  backup: vi.fn(),
  restore: vi.fn(),
  pickAttachments: vi.fn(),
  send: vi.fn(),
  stop: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));
// `toast(...)` is the notice with an action; the kinds hang off it.
const toasts = vi.hoisted(() =>
  Object.assign(vi.fn(), {
    success: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  }),
);
vi.mock("sonner", () => ({ toast: toasts }));

const owner = "a".repeat(64);
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
  owner,
  authorityGeneration: 1,
  runtime: { available: true, reason: null, version: "test" },
  models: [model("balanced", "Balanced model")],
  defaultModelId: "balanced",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: {
    available: true,
    unitPriceUsd: "0.014",
    model: "openai/gpt-5-nano",
  },
};

const NOW = new Date(2026, 9, 29, 12).getTime();
const HOUR = 3_600_000;
const conversation = (
  id: string,
  title: string,
  hoursAgo: number,
): AssistantSession => ({
  id,
  title,
  titleSource: "auto",
  modelId: "balanced",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt: NOW - hoursAgo * HOUR,
  deletedAt: null,
  pinnedAt: null,
  activeRun: null,
});
const message = (
  id: string,
  role: "user" | "assistant",
  content: string,
): AssistantMessage => ({
  id,
  role,
  content,
  createdAt: 1,
  status: "completed",
});

let store: Record<string, AssistantSessionDetail>;
let clock: number;
const sessionOf = (id: string) => store[id].session;
const everySession = () => Object.values(store).map((detail) => detail.session);

function listFromStore(deleted: boolean, search: string) {
  const needle = search.toLowerCase();
  const matching = everySession().filter(
    (item) =>
      (item.deletedAt !== null) === deleted &&
      (!needle ||
        item.title.toLowerCase().includes(needle) ||
        store[item.id].messages.some((m) =>
          m.content.toLowerCase().includes(needle),
        )),
  );
  matching.sort(
    (a, b) =>
      (deleted ? 0 : (b.pinnedAt ?? 0) - (a.pinnedAt ?? 0)) ||
      b.updatedAt - a.updatedAt,
  );
  return {
    sessions: matching,
    unreadable: [],
    trashCount: everySession().filter((item) => item.deletedAt !== null).length,
    // The first message that says it, as the backend names it.
    matches: Object.fromEntries(
      needle
        ? matching.flatMap((item) => {
            const hit = store[item.id].messages.find((m) =>
              m.content.toLowerCase().includes(needle),
            );
            return hit ? [[item.id, hit.id]] : [];
          })
        : [],
    ),
  };
}

function page() {
  return (
    <ShellProvider openSettings={() => undefined}>
      <AssistantPage enabled />
    </ShellProvider>
  );
}
const row = (title: string) =>
  screen.getByTitle(new RegExp(`^${title}`)) as HTMLButtonElement;
const rowMenu = (title: string) =>
  within(row(title).closest("li") as HTMLElement).getByRole("button", {
    name: "Options",
  });
const composer = () =>
  screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;
/** The titles listed under a group's heading. */
const group = (name: string) => {
  const heading = screen.getByRole("heading", { name });
  return within(heading.closest("li") as HTMLElement)
    .getAllByRole("button")
    .filter((button) => !button.hasAttribute("aria-haspopup"))
    .map((button) => button.textContent);
};
async function open(title: string) {
  await userEvent.click(await screen.findByTitle(new RegExp(`^${title}`)));
  await screen.findByRole("heading", { level: 1, name: title });
  await waitFor(() => expect(composer()).toBeEnabled());
}
/** Pick an item of the header's menu once it can run. */
async function chooseFromHeaderMenu(name: string) {
  await userEvent.click(
    await screen.findByRole("button", { name: "Conversation actions" }),
  );
  const item = await screen.findByRole("menuitem", { name });
  await waitFor(() =>
    expect(item).not.toHaveAttribute("aria-disabled", "true"),
  );
  await userEvent.click(item);
}
/** Open the trash from its entry, once the entry shows how many it holds. */
async function openTrash(entry: string | RegExp = /^Trash · \d/) {
  await userEvent.click(await screen.findByRole("button", { name: entry }));
  await screen.findByRole("button", { name: "Back to conversations" });
}
/** Type into the search, and let the list it asks for come back. */
async function searchFor(text: string) {
  await userEvent.click(
    screen.getByRole("button", { name: "Search conversations" }),
  );
  await userEvent.type(
    screen.getByRole("searchbox", { name: "Search conversations" }),
    text,
  );
  await waitFor(() =>
    expect(mocks.listSessions).toHaveBeenLastCalledWith(false, text),
  );
  await act(async () => undefined);
}

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  // Only the date is held: timers stay real for the user-event delays.
  vi.useFakeTimers({ toFake: ["Date"] });
  vi.setSystemTime(NOW);
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  clock = 1_000;
  store = {
    a: {
      session: conversation("a", "Trip plan", 1),
      messages: [
        message("a-1", "user", "Help me plan a trip to Kyoto"),
        message("a-2", "assistant", "A good plan starts with the season."),
        message("a-3", "user", "And the budget?"),
      ],
      attachments: [],
      usage: {},
    },
    b: {
      session: conversation("b", "Weekly report", 2),
      messages: [message("b-1", "user", "A weekly plan")],
      attachments: [],
      usage: {},
    },
    c: {
      session: conversation("c", "Old notes", 24 * 20),
      messages: [],
      attachments: [],
      usage: {},
    },
  };
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.listSessions.mockImplementation(async (deleted = false, search = "") =>
    listFromStore(deleted, search),
  );
  mocks.getSession.mockImplementation(async (id: string) => store[id]);
  mocks.updateSession.mockImplementation(async (patch) => {
    const detail = store[patch.sessionId];
    const next = { ...detail.session };
    if (patch.pinned !== undefined)
      next.pinnedAt = patch.pinned ? (next.pinnedAt ?? ++clock) : null;
    if (patch.deleted !== undefined)
      next.deletedAt = patch.deleted ? NOW : null;
    if (patch.draft !== undefined) next.draft = patch.draft;
    store[patch.sessionId] = { ...detail, session: next };
    return next;
  });
  mocks.emptyTrash.mockImplementation(async () => {
    const trashed = everySession().filter((item) => item.deletedAt !== null);
    for (const item of trashed) delete store[item.id];
    return { deleted: trashed.length, failed: 0 };
  });
  mocks.usageSummary.mockResolvedValue({});
  mocks.pickAttachments.mockResolvedValue([]);
  mocks.subscribe.mockResolvedValue(() => undefined);
  mocks.onAuthChange.mockResolvedValue(() => undefined);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("pins", () => {
  it("pins from a row's menu: the row moves up under Pinned and keeps focus", async () => {
    render(page());
    await screen.findByTitle(/^Weekly report/);
    expect(group("Today")).toEqual(["Trip plan", "Weekly report"]);
    await userEvent.click(rowMenu("Weekly report"));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Pin" }));
    expect(mocks.updateSession).toHaveBeenCalledWith({
      sessionId: "b",
      pinned: true,
      expectedOwner: owner,
      expectedGeneration: 1,
    });
    await waitFor(() => expect(group("Pinned")).toEqual(["Weekly report"]));
    expect(group("Today")).toEqual(["Trip plan"]);
    await waitFor(() => expect(row("Weekly report")).toHaveFocus());
  });

  it("unpins a pinned row back to its date", async () => {
    sessionOf("c").pinnedAt = 5;
    render(page());
    await screen.findByTitle(/^Old notes/);
    expect(group("Pinned")).toEqual(["Old notes"]);
    await userEvent.click(rowMenu("Old notes"));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "Unpin" }),
    );
    expect(mocks.updateSession).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "c", pinned: false }),
    );
    await waitFor(() => expect(group("Earlier")).toEqual(["Old notes"]));
    expect(screen.queryByRole("heading", { name: "Pinned" })).toBeNull();
  });

  it("pins and unpins the open conversation from the header's menu", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Pin");
    expect(mocks.updateSession).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "a", pinned: true }),
    );
    await waitFor(() => expect(group("Pinned")).toEqual(["Trip plan"]));
    // The menu now offers the way back, and focus is on the menu's button.
    expect(
      screen.getByRole("button", { name: "Conversation actions" }),
    ).toHaveFocus();
    await chooseFromHeaderMenu("Unpin");
    expect(mocks.updateSession).toHaveBeenLastCalledWith(
      expect.objectContaining({ sessionId: "a", pinned: false }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("heading", { name: "Pinned" })).toBeNull(),
    );
  });

  it("does not pin a conversation in the trash", async () => {
    sessionOf("a").deletedAt = NOW;
    render(page());
    await openTrash();
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await userEvent.click(
      await screen.findByRole("button", { name: "Conversation actions" }),
    );
    expect(
      await screen.findByRole("menuitem", { name: "Pin" }),
    ).toHaveAttribute("aria-disabled", "true");
  });

  it("says why when a pin could not be saved", async () => {
    mocks.updateSession.mockRejectedValueOnce("assistant_storage_unavailable");
    render(page());
    await screen.findByTitle(/^Weekly report/);
    await userEvent.click(rowMenu("Weekly report"));
    await userEvent.click(await screen.findByRole("menuitem", { name: "Pin" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      i18n.t(assistantErrorKey("assistant_storage_unavailable")),
    );
    expect(screen.queryByRole("heading", { name: "Pinned" })).toBeNull();
  });
});

describe("undo after moving to the trash", () => {
  /** The notice the last move to the trash showed. */
  const undoNotice = () => {
    const [text, options] = toasts.mock.calls[toasts.mock.calls.length - 1];
    return { text, options };
  };
  const pressUndo = async () => {
    await act(async () => {
      undoNotice().options.action.onClick();
    });
  };

  it("offers Undo, with the title, after the move", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalledTimes(1));
    expect(undoNotice().text).toBe("Moved “Trip plan” to trash");
    expect(undoNotice().options.action.label).toBe("Undo");
    // Long enough to reach it from the keyboard.
    expect(undoNotice().options.duration).toBeGreaterThanOrEqual(6000);
  });

  it("shortens a long title in the notice", async () => {
    const long = "T".repeat(80);
    sessionOf("a").title = long;
    render(page());
    await userEvent.click(await screen.findByTitle(new RegExp(`^${long}`)));
    await screen.findByRole("heading", { level: 1, name: long });
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalledTimes(1));
    expect(undoNotice().text).toBe(`Moved “${"T".repeat(40)}…” to trash`);
  });

  it("makes no offer when the move failed", async () => {
    render(page());
    await open("Trip plan");
    mocks.updateSession.mockRejectedValueOnce("assistant_storage_unavailable");
    await chooseFromHeaderMenu("Move to trash");
    await screen.findByRole("alert");
    expect(toasts).not.toHaveBeenCalled();
  });

  it("brings the conversation back where it was, and reopens it when nothing else is open", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalled());
    expect(screen.queryByTitle(/^Trip plan/)).toBeNull();
    await pressUndo();
    expect(mocks.updateSession).toHaveBeenLastCalledWith({
      sessionId: "a",
      deleted: false,
      expectedOwner: owner,
      expectedGeneration: 1,
    });
    expect(
      await screen.findByRole("heading", { level: 1, name: "Trip plan" }),
    ).toBeVisible();
    // Focus goes back to writing, not to the page.
    await waitFor(() => expect(composer()).toHaveFocus());
    expect(group("Today")).toContain("Trip plan");
  });

  it("restores the right conversation after the selection moved, leaving the selection alone", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalled());
    await open("Weekly report");
    const writes = mocks.updateSession.mock.calls.length;
    await pressUndo();
    // The trashed conversation's id, not the open one's.
    expect(mocks.updateSession.mock.calls.slice(writes)).toEqual([
      [
        {
          sessionId: "a",
          deleted: false,
          expectedOwner: owner,
          expectedGeneration: 1,
        },
      ],
    ]);
    expect(sessionOf("b").deletedAt).toBeNull();
    // Still on the report, and the restored conversation is in the list,
    // holding focus where the toast's button has gone.
    expect(
      screen.getByRole("heading", { level: 1, name: "Weekly report" }),
    ).toBeVisible();
    await waitFor(() => expect(row("Trip plan")).toHaveFocus());
    expect(row("Weekly report")).toHaveAttribute("aria-current", "page");
  });

  it("restores into the trash view's list without leaving it", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalled());
    await userEvent.click(
      await screen.findByRole("button", { name: /^Trash/ }),
    );
    const back = await screen.findByRole("button", {
      name: "Back to conversations",
    });
    await pressUndo();
    // The trash is empty again, and the view is where the reader left it.
    expect(await screen.findByText("Trash is empty")).toBeVisible();
    await waitFor(() => expect(back).toHaveFocus());
    expect(sessionOf("a").deletedAt).toBeNull();
  });

  it("says why when the undo failed", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalled());
    mocks.updateSession.mockRejectedValueOnce("assistant_storage_unavailable");
    await pressUndo();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      i18n.t(assistantErrorKey("assistant_storage_unavailable")),
    );
    expect(sessionOf("a").deletedAt).not.toBeNull();
  });
});

describe("moving a conversation to the trash from its row", () => {
  const undoNotice = () => {
    const [text, options] = toasts.mock.calls[toasts.mock.calls.length - 1];
    return { text, options };
  };
  const trashFromRow = async (title: string) => {
    await userEvent.click(rowMenu(title));
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "Move to trash" }),
    );
  };

  it("trashes the row's conversation, not the open one, and offers Undo for it", async () => {
    render(page());
    await open("Trip plan");
    await trashFromRow("Weekly report");
    expect(mocks.updateSession).toHaveBeenCalledWith({
      sessionId: "b",
      deleted: true,
      expectedOwner: owner,
      expectedGeneration: 1,
    });
    await waitFor(() => expect(toasts).toHaveBeenCalledTimes(1));
    expect(undoNotice().text).toBe("Moved “Weekly report” to trash");
    expect(undoNotice().options.action.label).toBe("Undo");
    // The open conversation stays open; the other is gone from the list.
    expect(
      screen.getByRole("heading", { level: 1, name: "Trip plan" }),
    ).toBeVisible();
    await waitFor(() =>
      expect(screen.queryByTitle(/^Weekly report/)).toBeNull(),
    );
    expect(sessionOf("a").deletedAt).toBeNull();
    // Focus goes to the trash, where it went.
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /^Trash/ })).toHaveFocus(),
    );
  });

  it("undoes it from the notice, back in the list with focus, the open conversation untouched", async () => {
    render(page());
    await open("Trip plan");
    await trashFromRow("Weekly report");
    await waitFor(() => expect(toasts).toHaveBeenCalled());
    await act(async () => {
      undoNotice().options.action.onClick();
    });
    expect(sessionOf("b").deletedAt).toBeNull();
    await waitFor(() => expect(row("Weekly report")).toHaveFocus());
    expect(
      screen.getByRole("heading", { level: 1, name: "Trip plan" }),
    ).toBeVisible();
  });

  it("closes the conversation when it is the open one's row", async () => {
    render(page());
    await open("Trip plan");
    await trashFromRow("Trip plan");
    await waitFor(() => expect(toasts).toHaveBeenCalledTimes(1));
    expect(undoNotice().text).toBe("Moved “Trip plan” to trash");
    await waitFor(() => expect(screen.queryByTitle(/^Trip plan/)).toBeNull());
    expect(
      screen.queryByRole("heading", { level: 1, name: "Trip plan" }),
    ).toBeNull();
  });

  it("says why and offers no Undo when it could not be moved", async () => {
    render(page());
    await screen.findByTitle(/^Weekly report/);
    mocks.updateSession.mockRejectedValueOnce("assistant_storage_unavailable");
    await trashFromRow("Weekly report");
    expect(await screen.findByRole("alert")).toHaveTextContent(
      i18n.t(assistantErrorKey("assistant_storage_unavailable")),
    );
    expect(toasts).not.toHaveBeenCalled();
    expect(screen.getByTitle(/^Weekly report/)).toBeVisible();
  });
});

describe("undo when something unsent is in the composer", () => {
  it("restores the conversation but keeps the text, without reopening it", async () => {
    render(page());
    await open("Trip plan");
    await chooseFromHeaderMenu("Move to trash");
    await waitFor(() => expect(toasts).toHaveBeenCalled());
    // Nothing is open: this is the new-conversation composer.
    await userEvent.type(composer(), "words not sent yet");
    const [, options] = toasts.mock.calls[toasts.mock.calls.length - 1];
    await act(async () => {
      options.action.onClick();
    });
    expect(sessionOf("a").deletedAt).toBeNull();
    await waitFor(() => expect(row("Trip plan")).toHaveFocus());
    expect(composer()).toHaveValue("words not sent yet");
    expect(row("Trip plan")).not.toHaveAttribute("aria-current");
    expect(
      screen.queryByRole("heading", { level: 1, name: "Trip plan" }),
    ).toBeNull();
  });
});

describe("emptying the trash", () => {
  beforeEach(() => {
    sessionOf("a").deletedAt = NOW;
    sessionOf("c").deletedAt = NOW;
  });
  const emptyButton = () => screen.getByRole("button", { name: "Empty trash" });
  const dialog = () => screen.findByRole("dialog");
  const confirm = async () =>
    userEvent.click(
      within(await dialog()).getByRole("button", { name: "Empty trash" }),
    );

  it("confirms with the number of conversations, then tells the result", async () => {
    render(page());
    await openTrash();
    await userEvent.click(emptyButton());
    const asking = await dialog();
    expect(asking).toHaveTextContent("Empty the trash?");
    expect(asking).toHaveTextContent("all 2 conversations");
    expect(mocks.emptyTrash).not.toHaveBeenCalled();
    await confirm();
    expect(mocks.emptyTrash).toHaveBeenCalledWith({
      expectedOwner: owner,
      expectedGeneration: 1,
    });
    await waitFor(() =>
      expect(toasts.success).toHaveBeenCalledWith(
        "Permanently deleted 2 conversations.",
      ),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByText("Trash is empty")).toBeVisible();
    // The button is out of work: focus goes to the way back.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Back to conversations" }),
      ).toHaveFocus(),
    );
  });

  it("does nothing when the question is cancelled, and focus returns to Empty", async () => {
    render(page());
    await openTrash();
    await userEvent.click(emptyButton());
    await userEvent.click(
      within(await dialog()).getByRole("button", { name: "Cancel" }),
    );
    expect(mocks.emptyTrash).not.toHaveBeenCalled();
    await waitFor(() => expect(emptyButton()).toHaveFocus());
    expect(toasts.success).not.toHaveBeenCalled();
  });

  it("says how many stayed when some could not be deleted", async () => {
    mocks.emptyTrash.mockResolvedValueOnce({ deleted: 1, failed: 1 });
    render(page());
    await openTrash();
    await userEvent.click(emptyButton());
    await confirm();
    await waitFor(() =>
      expect(toasts.warning).toHaveBeenCalledWith(
        "Deleted 1; 1 couldn't be deleted. Try again later.",
        expect.anything(),
      ),
    );
    expect(toasts.success).not.toHaveBeenCalled();
  });

  it("says so when every conversation is still busy", async () => {
    mocks.emptyTrash.mockResolvedValueOnce({ deleted: 0, failed: 2 });
    render(page());
    await openTrash();
    await userEvent.click(emptyButton());
    await confirm();
    await waitFor(() =>
      expect(toasts.warning).toHaveBeenCalledWith(
        "Deleted 0; 2 couldn't be deleted. Try again later.",
        expect.anything(),
      ),
    );
  });

  it("holds the dialog still while it works", async () => {
    let finish: (report: { deleted: number; failed: number }) => void = () =>
      undefined;
    mocks.emptyTrash.mockReturnValueOnce(
      new Promise((resolve) => (finish = resolve)),
    );
    render(page());
    await openTrash();
    await userEvent.click(emptyButton());
    await confirm();
    const working = await dialog();
    await waitFor(() =>
      expect(
        within(working).getByRole("button", { name: "Empty trash" }),
      ).toBeDisabled(),
    );
    expect(
      within(working).getByRole("button", { name: "Cancel" }),
    ).toBeDisabled();
    await act(async () => finish({ deleted: 2, failed: 0 }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("keeps the dialog open with the reason when it failed", async () => {
    mocks.emptyTrash.mockRejectedValueOnce("assistant_storage_unavailable");
    render(page());
    await openTrash();
    await userEvent.click(emptyButton());
    await confirm();
    const failed = await dialog();
    expect(await within(failed).findByRole("alert")).toHaveTextContent(
      i18n.t(assistantErrorKey("assistant_storage_unavailable")),
    );
    // The page's banner would tell it a second time.
    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(toasts.success).not.toHaveBeenCalled();
    expect(toasts.warning).not.toHaveBeenCalled();
  });

  it("closes the trashed conversation that was open", async () => {
    render(page());
    await openTrash();
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await screen.findByRole("heading", { level: 1, name: "Trip plan" });
    await userEvent.click(emptyButton());
    await confirm();
    await waitFor(() => expect(toasts.success).toHaveBeenCalled());
    expect(
      await screen.findByText(
        "Choose a conversation from the trash to view or restore it.",
      ),
    ).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("can't be asked for while the trash is empty", async () => {
    delete store.a;
    delete store.c;
    render(page());
    await openTrash("Trash");
    expect(emptyButton()).toBeDisabled();
  });
});

describe("opening a search result", () => {
  const scrolled = () => {
    const scrollIntoView = Element.prototype.scrollIntoView as ReturnType<
      typeof vi.fn
    >;
    return scrollIntoView.mock.contexts.flatMap((element, index) =>
      element instanceof HTMLElement && element.dataset.messageId
        ? [[element.dataset.messageId, scrollIntoView.mock.calls[index][0]]]
        : [],
    );
  };

  it("scrolls to the first message found, centred, and marks the words", async () => {
    render(page());
    await screen.findByTitle(/^Trip plan/);
    await searchFor("plan");
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await screen.findByRole("heading", { level: 1, name: "Trip plan" });
    await waitFor(() =>
      expect(scrolled()).toEqual([["a-1", { block: "center" }]]),
    );
    const found = document.querySelector(
      '[data-message-id="a-1"]',
    ) as HTMLElement;
    expect(
      [...found.querySelectorAll("mark")].map((mark) => mark.textContent),
    ).toEqual(["plan"]);
    // Every message shows the words, not only the one scrolled to.
    const reply = document.querySelector(
      '[data-message-id="a-2"]',
    ) as HTMLElement;
    expect(reply.querySelectorAll("mark")).toHaveLength(1);
    const unrelated = document.querySelector(
      '[data-message-id="a-3"]',
    ) as HTMLElement;
    expect(unrelated.querySelectorAll("mark")).toHaveLength(0);
  });

  it("opens a conversation found only by its title without scrolling", async () => {
    render(page());
    await screen.findByTitle(/^Weekly report/);
    await searchFor("report");
    await userEvent.click(await screen.findByTitle(/^Weekly report/));
    await screen.findByRole("heading", { level: 1, name: "Weekly report" });
    await waitFor(() => expect(composer()).toBeEnabled());
    expect(scrolled()).toEqual([]);
  });

  it("scrolls again when the open conversation's row is chosen again", async () => {
    render(page());
    await screen.findByTitle(/^Trip plan/);
    await searchFor("plan");
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await waitFor(() => expect(scrolled()).toHaveLength(1));
    await userEvent.click(row("Trip plan"));
    await waitFor(() => expect(scrolled()).toHaveLength(2));
  });

  it("does not scroll to a result when another conversation is opened", async () => {
    render(page());
    await screen.findByTitle(/^Trip plan/);
    await searchFor("plan");
    // Chosen, then another before its transcript arrives.
    let arrive: () => void = () => undefined;
    mocks.getSession.mockImplementationOnce(
      (id: string) =>
        new Promise((resolve) => {
          arrive = () => resolve(store[id]);
        }),
    );
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await userEvent.click(await screen.findByTitle(/^Weekly report/));
    await screen.findByRole("heading", { level: 1, name: "Weekly report" });
    await act(async () => arrive());
    // Only the one that is open, to its own result.
    await waitFor(() =>
      expect(scrolled()).toEqual([["b-1", { block: "center" }]]),
    );
  });

  it("marks nothing for a search that is only spaces", async () => {
    render(page());
    await screen.findByTitle(/^Trip plan/);
    await searchFor(" ");
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await screen.findByRole("heading", { level: 1, name: "Trip plan" });
    await waitFor(() =>
      expect(document.querySelector('[data-message-id="a-1"]')).not.toBeNull(),
    );
    expect(document.querySelectorAll("mark")).toHaveLength(0);
  });

  it("forgets the jump when the conversation could not be loaded, so a later load does not scroll", async () => {
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
        <AssistantPage enabled />
      </ShellProvider>,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Open assistant" }),
    );
    await screen.findByTitle(/^Trip plan/);
    await searchFor("plan");
    mocks.getSession.mockRejectedValueOnce("assistant_storage_unavailable");
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await screen.findByRole("alert");
    // The header's refresh reads the open conversation again.
    await userEvent.click(
      screen.getByRole("button", { name: "Refresh assistant" }),
    );
    await waitFor(() =>
      expect(document.querySelector('[data-message-id="a-1"]')).not.toBeNull(),
    );
    expect(scrolled()).toEqual([]);
  });

  it("takes the marks away with the search", async () => {
    render(page());
    await screen.findByTitle(/^Trip plan/);
    await searchFor("plan");
    await userEvent.click(await screen.findByTitle(/^Trip plan/));
    await screen.findByRole("heading", { level: 1, name: "Trip plan" });
    await waitFor(() =>
      expect(
        document.querySelectorAll("[data-message-id] mark"),
      ).not.toHaveLength(0),
    );
    await userEvent.clear(
      screen.getByRole("searchbox", { name: "Search conversations" }),
    );
    await waitFor(() =>
      expect(document.querySelectorAll("[data-message-id] mark")).toHaveLength(
        0,
      ),
    );
  });
});
