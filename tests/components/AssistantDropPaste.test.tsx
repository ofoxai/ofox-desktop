import { StrictMode, type ReactNode } from "react";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import { ShellProvider } from "@/components/shell/ShellContext";
import { InteractionScope } from "@/components/ui/interaction-scope";
import { MAX_PASTED_FILES, pastedFiles } from "@/utils/assistantPaste";
import type {
  AssistantAttachment,
  AssistantBootstrap,
  AssistantDrop,
  AssistantDragState,
  AssistantModel,
  AssistantSession,
  AssistantSessionDetail,
  AssistantSessionList,
} from "@/lib/api/assistant";

const api = vi.hoisted(() => ({
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
  importDrop: vi.fn(),
  importPasted: vi.fn(),
  send: vi.fn(),
  stop: vi.fn(),
  getPreferences: vi.fn(),
  setPreferences: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
  onDrag: vi.fn(),
  onDrop: vi.fn(),
}));
/** The backend's drag and drop listeners, as the page registered them. */
const listeners = vi.hoisted(() => ({
  drag: new Set<(state: "enter" | "leave") => void>(),
  drop: new Set<(drop: { token: string; count: number }) => void>(),
  unlistened: 0,
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: api,
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));
const toasts = vi.hoisted(() =>
  Object.assign(vi.fn(), {
    success: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  }),
);
vi.mock("sonner", () => ({ toast: toasts }));

const session: AssistantSession = {
  id: "session-a",
  title: "My conversation",
  titleSource: "auto",
  assistantId: "ofox",
  capabilities: [],
  modelId: "balanced",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt: 1,
  deletedAt: null,
  pinnedAt: null,
  activeRun: null,
};
const model: AssistantModel = {
  id: "balanced",
  name: "Balanced model",
  contextWindow: 128_000,
  maxOutputTokens: 16_000,
  supportsImages: true,
  inputModalities: ["text", "image"],
  supportedEndpoints: ["chat/completions"],
  inputPricePerMillion: null,
  outputPricePerMillion: null,
  pricing: {},
  cost: { input: null, output: null, cacheRead: null, cacheWrite: null },
  pricingKnown: false,
};
const capabilities: AssistantBootstrap = {
  owner: "a".repeat(64),
  authorityGeneration: 3,
  runtime: { available: true, reason: null, version: "test" },
  models: [model],
  defaultModelId: "balanced",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: { available: false, unitPriceUsd: "0", model: "" },
};
const identity = {
  expectedOwner: capabilities.owner,
  expectedGeneration: capabilities.authorityGeneration,
};
const attachment = (
  id: string,
  name: string,
  mimeType = "application/pdf",
): AssistantAttachment => ({
  id,
  name,
  mimeType,
  sizeBytes: 2048,
  status: "ready",
  reason: null,
  previewUrl: null,
  details: null,
});
const pdf = attachment("file-pdf", "report.pdf");

let current: AssistantSessionDetail;
const listOf = (sessions: AssistantSession[]): AssistantSessionList => ({
  sessions,
  unreadable: [],
  trashCount: 0,
  matches: {},
});

function page(active = true, beside: ReactNode = null) {
  return (
    <>
      <ShellProvider openSettings={() => undefined}>
        <InteractionScope active={active}>
          <AssistantPage />
        </InteractionScope>
      </ShellProvider>
      {beside}
    </>
  );
}
/** A modal dialog of the app, as another part of the app would open one. */
const dialog = (open: boolean) => (
  <ConfirmDialog
    isOpen={open}
    title="Sure?"
    message="Really?"
    onConfirm={() => undefined}
    onCancel={() => undefined}
  />
);
const composer = () =>
  screen.getByRole("textbox", { name: "Message" }) as HTMLTextAreaElement;
const composerFiles = () =>
  within(composer().closest("footer") as HTMLElement).queryByRole("list", {
    name: "Attached files",
  });
const overlay = () => screen.queryByTestId("assistant-drop-overlay");

async function openConversation(active = true) {
  const view = render(page(active));
  await userEvent.click(
    await screen.findByRole("button", { name: /My conversation/ }),
  );
  await screen.findByRole("combobox", { name: "Choose model" });
  await waitFor(() => expect(composer()).toBeEnabled());
  return view;
}
/** The page with no conversation open: the first message makes one. */
async function openNewConversation() {
  api.listSessions.mockResolvedValue(listOf([]));
  api.createSession.mockImplementation(async (request) => ({
    ...session,
    title: "",
    ...request,
  }));
  render(page());
  await waitFor(() => expect(composer()).toBeEnabled());
}
/** A reply is running: Send has become Stop. */
async function startReply() {
  await userEvent.type(composer(), "Hello");
  await userEvent.click(screen.getByRole("button", { name: "Send" }));
  await screen.findByRole("button", { name: "Stop" });
}
const drag = (state: AssistantDragState) =>
  act(() => listeners.drag.forEach((callback) => callback(state)));
const drop = (token = "token-1", count = 1) =>
  act(() =>
    listeners.drop.forEach((callback) =>
      callback({ token, count } as AssistantDrop),
    ),
  );

interface Clipboard {
  files?: File[];
  text?: string;
  html?: string;
}
/** Paste into the message box; false when the page took the paste over. */
function paste({ files = [], text = "", html = "" }: Clipboard) {
  const data: Record<string, string> = {
    "text/plain": text,
    "text/html": html,
  };
  return fireEvent.paste(composer(), {
    clipboardData: { files, getData: (type: string) => data[type] ?? "" },
  });
}
const image = (name = "shot.png") =>
  new File([new Uint8Array([1, 2, 3])], name, { type: "image/png" });
/** Every pasted file is imported as an attachment of its own name. */
const importsAsItsName = () =>
  api.importPasted.mockImplementation(async (file: File) => [
    attachment(`id-${file.name}`, file.name, "image/png"),
  ]);

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  Object.values(api).forEach((mock) => mock.mockReset());
  [toasts, toasts.success, toasts.warning, toasts.error].forEach((mock) =>
    mock.mockReset(),
  );
  listeners.drag.clear();
  listeners.drop.clear();
  listeners.unlistened = 0;
  current = {
    session: { ...session },
    messages: [],
    attachments: [],
    usage: {},
  };
  api.bootstrap.mockResolvedValue(capabilities);
  api.getPreferences.mockResolvedValue({
    instructions: "",
    autoTitle: true,
    notifyOnFinish: true,
  });
  api.listSessions.mockImplementation(async () => listOf([current.session]));
  api.createSession.mockImplementation(async () => current.session);
  api.getSession.mockImplementation(async () => current);
  api.updateSession.mockImplementation(async (patch) => {
    current = { ...current, session: { ...current.session, ...patch } };
    return current.session;
  });
  api.deleteSession.mockResolvedValue(undefined);
  api.exportMarkdown.mockResolvedValue(true);
  api.usageSummary.mockResolvedValue({
    inputTokens: 0,
    outputTokens: 0,
    estimatedCostUsd: "0",
    usageCoverage: "complete",
    searchCount: 0,
  });
  api.pickAttachments.mockResolvedValue([]);
  api.send.mockResolvedValue({ runId: "run-1", operationId: "operation-1" });
  api.stop.mockResolvedValue(undefined);
  api.subscribe.mockResolvedValue(() => undefined);
  api.onAuthChange.mockResolvedValue(() => undefined);
  api.onDrag.mockImplementation(async (callback) => {
    listeners.drag.add(callback);
    return () => {
      listeners.drag.delete(callback);
      listeners.unlistened += 1;
    };
  });
  api.onDrop.mockImplementation(async (callback) => {
    listeners.drop.add(callback);
    return () => {
      listeners.drop.delete(callback);
      listeners.unlistened += 1;
    };
  });
});

describe("the drop overlay", () => {
  it("shows while files are dragged over a conversation that can take them, and goes when they leave", async () => {
    await openConversation();
    expect(overlay()).toBeNull();
    drag("enter");
    expect(overlay()).toHaveTextContent("Drop to add files");
    drag("leave");
    expect(overlay()).toBeNull();
  });

  it("shows over the transcript and the composer, not the header", async () => {
    await openConversation();
    drag("enter");
    const box = overlay() as HTMLElement;
    const area = box.parentElement as HTMLElement;
    expect(area).toContainElement(composer());
    expect(area).toContainElement(
      screen.getByRole("heading", { name: "How can I help?" }),
    );
    expect(area).not.toContainElement(
      screen.getByRole("heading", { level: 1 }),
    );
  });

  it("goes with the drop too: no leave follows it", async () => {
    await openConversation();
    api.importDrop.mockResolvedValue([pdf]);
    drag("enter");
    expect(overlay()).not.toBeNull();
    drop();
    expect(overlay()).toBeNull();
    await waitFor(() => expect(composerFiles()).not.toBeNull());
    // Once the import is over the page takes files again: still no overlay.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Add content or a feature" }),
      ).toBeEnabled(),
    );
    expect(overlay()).toBeNull();
  });

  it("is only a picture: no alert, no pointer events, no announcement, focus stays", async () => {
    await openConversation();
    composer().focus();
    drag("enter");
    const box = overlay() as HTMLElement;
    expect(box).toHaveClass("pointer-events-none");
    expect(box).toHaveAttribute("aria-hidden", "true");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
    expect(box.querySelector("button, a, input, [tabindex]")).toBeNull();
    expect(composer()).toHaveFocus();
    // Its fade is the app's own, which index.css turns off for reduced motion.
    expect(box).toHaveClass("animate-fade");
  });

  it("does not show on a page that is not the one on screen", async () => {
    const view = await openConversation(false);
    drag("enter");
    expect(overlay()).toBeNull();
    // Dragging on when the page comes to the front shows it.
    view.rerender(page(true));
    expect(overlay()).not.toBeNull();
  });

  it("does not show while a reply runs", async () => {
    await openConversation();
    await startReply();
    drag("enter");
    expect(overlay()).toBeNull();
  });

  it("does not show for a conversation in the trash", async () => {
    current.session.deletedAt = 5;
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await screen.findByRole("button", { name: "Restore" });
    drag("enter");
    expect(overlay()).toBeNull();
  });

  it("does not show while the assistant is unavailable", async () => {
    api.bootstrap.mockResolvedValue({
      ...capabilities,
      runtime: { available: false, reason: "missing", version: null },
    });
    render(page());
    await screen.findByText("Assistant runtime unavailable");
    drag("enter");
    expect(overlay()).toBeNull();
  });
});

describe("under a modal dialog", () => {
  it("shows no overlay and takes no drop while a dialog is open over the page", async () => {
    const view = await openConversation();
    view.rerender(page(true, dialog(true)));
    await screen.findByRole("dialog");
    drag("enter");
    expect(overlay()).toBeNull();
    drop();
    await act(async () => undefined);
    expect(api.importDrop).not.toHaveBeenCalled();
    // Closed again, the page takes files as before.
    view.rerender(page(true, dialog(false)));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    drag("enter");
    expect(overlay()).not.toBeNull();
  });

  it("treats any modal dialog on the document as covering the page", async () => {
    await openConversation();
    const modal = document.createElement("div");
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-modal", "true");
    document.body.append(modal);
    try {
      drag("enter");
      expect(overlay()).toBeNull();
      drop();
      await act(async () => undefined);
      expect(api.importDrop).not.toHaveBeenCalled();
    } finally {
      modal.remove();
    }
    drag("enter");
    expect(overlay()).not.toBeNull();
  });
});

describe("listening to the backend", () => {
  it("listens once under StrictMode and lets go of every listener on unmount", async () => {
    const view = render(<StrictMode>{page()}</StrictMode>);
    await waitFor(() => expect(listeners.drag.size).toBe(1));
    expect(listeners.drop.size).toBe(1);
    // The listener the first mount made is gone, not left beside the second.
    await waitFor(() =>
      expect(listeners.unlistened).toBe(
        api.onDrag.mock.calls.length + api.onDrop.mock.calls.length - 2,
      ),
    );
    view.unmount();
    expect(listeners.drag.size).toBe(0);
    expect(listeners.drop.size).toBe(0);
  });

  it("imports a drop once under StrictMode", async () => {
    api.importDrop.mockResolvedValue([pdf]);
    render(<StrictMode>{page()}</StrictMode>);
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await waitFor(() => expect(composer()).toBeEnabled());
    await waitFor(() => expect(listeners.drop.size).toBe(1));
    drop();
    await waitFor(() => expect(composerFiles()).not.toBeNull());
    expect(api.importDrop).toHaveBeenCalledTimes(1);
  });

  it("says why when the backend's events can't be heard", async () => {
    api.onDrag.mockRejectedValue("assistant_storage_unavailable");
    render(page());
    expect(await screen.findByRole("alert")).toBeVisible();
  });
});

describe("a drop", () => {
  it("adds the files to the draft with the backend's own token, and saves them with it", async () => {
    api.importDrop.mockResolvedValue([pdf]);
    await openConversation();
    drop("token-7", 1);
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent("report.pdf"),
    );
    expect(api.importDrop).toHaveBeenCalledWith({
      sessionId: "session-a",
      token: "token-7",
      ...identity,
    });
    await waitFor(() =>
      expect(api.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          draftAttachmentIds: ["file-pdf"],
        }),
      ),
    );
  });

  it("shows an imported file's notes like a picked one", async () => {
    api.importDrop.mockResolvedValue([
      {
        ...pdf,
        status: "unreadable",
        reason: "assistant_attachment_unreadable",
      },
    ]);
    await openConversation();
    drop();
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent("This file couldn't be read."),
    );
    expect(screen.getByRole("button", { name: "Send" })).toBeDisabled();
  });

  it("gives focus to the message when the import left it on the page", async () => {
    api.importDrop.mockResolvedValue([pdf]);
    await openConversation();
    (document.activeElement as HTMLElement | null)?.blur();
    drop();
    await waitFor(() => expect(composerFiles()).not.toBeNull());
    await waitFor(() => expect(composer()).toHaveFocus());
  });

  it("starts the conversation first when none is open", async () => {
    api.importDrop.mockResolvedValue([pdf]);
    await openNewConversation();
    drop();
    await waitFor(() =>
      expect(composerFiles()).toHaveTextContent("report.pdf"),
    );
    expect(api.createSession).toHaveBeenCalledOnce();
    expect(api.importDrop).toHaveBeenCalledWith(
      expect.objectContaining({ sessionId: "session-a", token: "token-1" }),
    );
    expect(api.createSession.mock.invocationCallOrder[0]).toBeLessThan(
      api.importDrop.mock.invocationCallOrder[0],
    );
  });

  it.each([
    [
      "assistant_drop_unavailable",
      "The dropped files are no longer available. Drop them again.",
    ],
    [
      "assistant_attachment_too_many",
      "Too many files. Attach up to 10 at a time and up to 200 per conversation.",
    ],
    [
      "assistant_attachment_unreadable",
      "A file couldn't be read. It may have been moved",
    ],
  ])("explains a refused drop (%s) in words", async (code, expected) => {
    api.importDrop.mockRejectedValueOnce(code);
    await openConversation();
    drop();
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(expected);
    expect(document.body).not.toHaveTextContent("assistant_");
    expect(composerFiles()).toBeNull();
    // Nothing was held back: the next drop can be tried.
    api.importDrop.mockResolvedValueOnce([pdf]);
    drop("token-2");
    await waitFor(() => expect(composerFiles()).not.toBeNull());
  });

  it("leaves no empty conversation behind when a drop made for a new one is refused", async () => {
    api.importDrop.mockRejectedValueOnce("assistant_drop_unavailable");
    await openNewConversation();
    drop();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "The dropped files are no longer available.",
    );
    await waitFor(() =>
      expect(api.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        deleted: true,
        ...identity,
      }),
    );
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent(
      "Assistant",
    );
  });

  it("is ignored when the page is not the one on screen", async () => {
    await openConversation(false);
    drop();
    // Give an import the time it would take to begin.
    await act(async () => undefined);
    expect(api.importDrop).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("is ignored while a reply runs", async () => {
    await openConversation();
    await startReply();
    drop();
    await act(async () => undefined);
    expect(api.importDrop).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("is ignored when no conversation can be made for it yet", async () => {
    api.listSessions.mockResolvedValue(listOf([]));
    api.bootstrap.mockResolvedValue({
      ...capabilities,
      runtime: { available: false, reason: "missing", version: null },
    });
    render(page());
    await screen.findByText("Assistant runtime unavailable");
    drop();
    await act(async () => undefined);
    expect(api.createSession).not.toHaveBeenCalled();
    expect(api.importDrop).not.toHaveBeenCalled();
  });

  it("is ignored for a conversation in the trash", async () => {
    current.session.deletedAt = 5;
    render(page());
    await userEvent.click(
      await screen.findByRole("button", { name: /My conversation/ }),
    );
    await screen.findByRole("button", { name: "Restore" });
    drop();
    await act(async () => undefined);
    expect(api.importDrop).not.toHaveBeenCalled();
  });

  it("does not run a second drop beside one that is being imported", async () => {
    api.importDrop.mockResolvedValue([pdf]);
    await openConversation();
    // Both arrive before the page has drawn the first one's busy state.
    act(() =>
      listeners.drop.forEach((callback) => {
        callback({ token: "token-1", count: 1 });
        callback({ token: "token-2", count: 1 });
      }),
    );
    await waitFor(() => expect(composerFiles()).not.toBeNull());
    // The second token is left to expire, without a word about it.
    expect(screen.queryByRole("alert")).toBeNull();
    expect(api.importDrop).toHaveBeenCalledTimes(1);
    expect(api.importDrop).toHaveBeenCalledWith(
      expect.objectContaining({ token: "token-1" }),
    );
  });
});

describe("without a known account", () => {
  // A bootstrap without an owner can't name the account a write is for.
  const unknownAccount = () =>
    api.bootstrap.mockResolvedValue({ ...capabilities, owner: "" });
  const identityChanged = "The account or region changed. Please try again.";

  it("refuses a drop, says so, and reads the authority again", async () => {
    unknownAccount();
    await openConversation();
    const reads = api.bootstrap.mock.calls.length;
    drop();
    expect(await screen.findByRole("alert")).toHaveTextContent(identityChanged);
    expect(api.importDrop).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(api.bootstrap.mock.calls.length).toBeGreaterThan(reads),
    );
    expect(composerFiles()).toBeNull();
  });

  it("refuses a paste, says so, and reads the authority again", async () => {
    unknownAccount();
    await openConversation();
    const reads = api.bootstrap.mock.calls.length;
    paste({ files: [image()] });
    expect(await screen.findByRole("alert")).toHaveTextContent(identityChanged);
    expect(api.importPasted).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(api.bootstrap.mock.calls.length).toBeGreaterThan(reads),
    );
    expect(composerFiles()).toBeNull();
  });

  it("leaves no empty conversation behind when the drop made for a new one is refused", async () => {
    unknownAccount();
    await openNewConversation();
    drop();
    expect(await screen.findByRole("alert")).toHaveTextContent(identityChanged);
    await waitFor(() =>
      expect(api.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({ sessionId: "session-a", deleted: true }),
      ),
    );
    expect(api.importDrop).not.toHaveBeenCalled();
  });
});

describe("pasted files", () => {
  it("adds a pasted image to the draft and keeps the paste from the text field", async () => {
    importsAsItsName();
    await openConversation();
    const shot = image();
    expect(paste({ files: [shot] })).toBe(false);
    await waitFor(() => expect(composerFiles()).toHaveTextContent("shot.png"));
    expect(api.importPasted).toHaveBeenCalledOnce();
    expect(api.importPasted).toHaveBeenCalledWith(shot, {
      sessionId: "session-a",
      ...identity,
    });
    await waitFor(() =>
      expect(api.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({
          sessionId: "session-a",
          draftAttachmentIds: ["id-shot.png"],
        }),
      ),
    );
    expect(composer()).toHaveValue("");
  });

  it("imports several pasted files in order, each as its own file", async () => {
    importsAsItsName();
    await openConversation();
    const files = [image("a.png"), image("b.png"), image("c.png")];
    paste({ files });
    await waitFor(() =>
      expect(within(composerFiles()!).getAllByRole("listitem")).toHaveLength(3),
    );
    expect(api.importPasted.mock.calls.map(([file]) => file)).toEqual(files);
  });

  it("never reads a file's type: the backend reads the bytes", async () => {
    importsAsItsName();
    await openConversation();
    const shot = image();
    Object.defineProperty(shot, "type", {
      get() {
        throw new Error("the type was read");
      },
    });
    expect(paste({ files: [shot] })).toBe(false);
    await waitFor(() => expect(composerFiles()).not.toBeNull());
  });

  it("leaves a text paste to the text field", async () => {
    await openConversation();
    expect(paste({ text: "some words" })).toBe(true);
    expect(api.importPasted).not.toHaveBeenCalled();
  });

  it("leaves a paste of files and text to the text field: the words win", async () => {
    await openConversation();
    expect(paste({ files: [image()], text: "a caption" })).toBe(true);
    expect(api.importPasted).not.toHaveBeenCalled();
    expect(composerFiles()).toBeNull();
  });

  it("takes a file copied in the file manager, whose plain text is only its name", async () => {
    importsAsItsName();
    await openConversation();
    expect(paste({ files: [image("shot.png")], text: "shot.png" })).toBe(false);
    await waitFor(() => expect(composerFiles()).toHaveTextContent("shot.png"));
    expect(composer()).toHaveValue("");
  });

  it("takes files that come with markup but no plain text (an image copied from a page)", async () => {
    importsAsItsName();
    await openConversation();
    expect(
      paste({
        files: [image()],
        html: '<img src="https://example.test/a.png">',
      }),
    ).toBe(false);
    await waitFor(() => expect(composerFiles()).not.toBeNull());
  });

  it("imports the first 10 of a bigger paste and says how many were skipped", async () => {
    importsAsItsName();
    await openConversation();
    const files = Array.from({ length: 12 }, (_, index) =>
      image(`p${index + 1}.png`),
    );
    paste({ files });
    await waitFor(() =>
      expect(within(composerFiles()!).getAllByRole("listitem")).toHaveLength(
        MAX_PASTED_FILES,
      ),
    );
    expect(api.importPasted.mock.calls.map(([file]) => file)).toEqual(
      files.slice(0, 10),
    );
    expect(toasts.warning).toHaveBeenCalledOnce();
    expect(toasts.warning).toHaveBeenCalledWith(
      "Only the first 10 pasted files are added. 2 files were skipped; paste them again to add them.",
    );
  });

  it("says nothing about skipped files when the paste fits", async () => {
    importsAsItsName();
    await openConversation();
    paste({
      files: Array.from({ length: 10 }, (_, index) => image(`p${index}.png`)),
    });
    await waitFor(() =>
      expect(within(composerFiles()!).getAllByRole("listitem")).toHaveLength(
        10,
      ),
    );
    expect(toasts.warning).not.toHaveBeenCalled();
  });

  it("starts the conversation first when none is open", async () => {
    importsAsItsName();
    await openNewConversation();
    paste({ files: [image()] });
    await waitFor(() => expect(composerFiles()).toHaveTextContent("shot.png"));
    expect(api.createSession).toHaveBeenCalledOnce();
    expect(api.importPasted).toHaveBeenCalledWith(
      expect.anything(),
      expect.objectContaining({ sessionId: "session-a" }),
    );
  });

  it("keeps the files that came in before one failed, says why, and tries no more", async () => {
    api.importPasted
      .mockResolvedValueOnce([attachment("id-a", "a.png", "image/png")])
      .mockRejectedValueOnce("assistant_attachment_too_large");
    await openConversation();
    paste({ files: [image("a.png"), image("big.png"), image("c.png")] });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "A file is too large.",
    );
    expect(composerFiles()).toHaveTextContent("a.png");
    expect(within(composerFiles()!).getAllByRole("listitem")).toHaveLength(1);
    expect(api.importPasted).toHaveBeenCalledTimes(2);
    expect(document.body).not.toHaveTextContent("assistant_");
    // What did come in is saved with the draft.
    await waitFor(() =>
      expect(api.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({ draftAttachmentIds: ["id-a"] }),
      ),
    );
  });

  it("leaves no empty conversation behind when the first pasted file is refused", async () => {
    api.importPasted.mockRejectedValueOnce("assistant_attachment_unsupported");
    await openNewConversation();
    paste({ files: [image("movie.mov")] });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "This file type can't be read.",
    );
    await waitFor(() =>
      expect(api.updateSession).toHaveBeenCalledWith({
        sessionId: "session-a",
        deleted: true,
        ...identity,
      }),
    );
    expect(composerFiles()).toBeNull();
  });

  it("says nothing about skipped files for a paste that was not started", async () => {
    importsAsItsName();
    await openConversation();
    // The second paste arrives before the page has drawn the first one's busy
    // state: it is not run beside the first.
    act(() => {
      paste({ files: [image("first.png")] });
      paste({
        files: Array.from({ length: 12 }, (_, index) => image(`p${index}.png`)),
      });
    });
    await waitFor(() => expect(composerFiles()).not.toBeNull());
    expect(api.importPasted).toHaveBeenCalledTimes(1);
    expect(api.importPasted.mock.calls[0][0]).toHaveProperty(
      "name",
      "first.png",
    );
    expect(toasts.warning).not.toHaveBeenCalled();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("is not imported while a reply runs, and the field still gets nothing", async () => {
    await openConversation();
    await startReply();
    expect(paste({ files: [image()] })).toBe(false);
    await act(async () => undefined);
    expect(api.importPasted).not.toHaveBeenCalled();
    expect(composerFiles()).toBeNull();
  });
});

describe("what a paste is", () => {
  const files = (count: number) =>
    Array.from({ length: count }, (_, index) => image(`f${index}.png`));
  const named = (...names: string[]) => names.map((name) => image(name));
  const clipboard = (list: File[], text = "") =>
    ({
      files: list as unknown as FileList,
      getData: (type: string) => (type === "text/plain" ? text : ""),
    }) as Pick<DataTransfer, "files" | "getData">;

  it("is nothing for the composer without a clipboard or without files", () => {
    expect(pastedFiles(null)).toBeNull();
    expect(pastedFiles(clipboard([], "words"))).toBeNull();
  });

  it.each([
    ["no plain text", ""],
    ["blank plain text", "  \n\t \r\n "],
  ])("is a file paste with %s", (_name, text) => {
    const list = files(2);
    expect(pastedFiles(clipboard(list, text))).toEqual({
      files: list,
      skipped: 0,
    });
  });

  // The file manager puts the names of the copied files in the plain text.
  it.each([
    ["one file", ["photo.png"], "photo.png"],
    ["two files, a line each", ["a.png", "b.png"], "a.png\nb.png"],
    ["two files, CRLF lines", ["a.png", "b.png"], "a.png\r\nb.png"],
    ["two files, CR lines", ["a.png", "b.png"], "a.png\rb.png"],
    ["two files in another order", ["a.png", "b.png"], "b.png\na.png"],
    [
      "a name with spaces around it and a final line break",
      ["a.png"],
      " a.png \n",
    ],
    [
      "a decomposed name for a composed file name",
      ["\u00e9.png"],
      "e\u0301.png",
    ],
    [
      "a composed name for a decomposed file name",
      ["e\u0301.png"],
      "\u00e9.png",
    ],
  ])(
    "is a file paste when the text is only the names of the files: %s",
    (_name, names, text) => {
      const list = named(...names);
      expect(pastedFiles(clipboard(list, text))).toEqual({
        files: list,
        skipped: 0,
      });
    },
  );

  it.each([
    ["a caption", ["image.png"], "A cat on a sofa"],
    [
      "a title, then the name of a file that isn't there",
      ["other.png"],
      "Title\nimage.png",
    ],
    ["a name among other words", ["photo.png"], "photo.png from the trip"],
    ["a name with a caption line", ["photo.png"], "photo.png\nfrom the trip"],
    ["the name of a file that isn't on the clipboard", ["a.png"], "b.png"],
  ])(
    "is a text paste when the text says more than the names: %s",
    (_name, names, text) => {
      expect(pastedFiles(clipboard(named(...names), text))).toBeNull();
    },
  );

  it("compares the text with the names of every file, also those a paste leaves out", () => {
    const list = files(11);
    const text = list.map((file) => file.name).join("\n");
    expect(pastedFiles(clipboard(list, text))).toEqual({
      files: list.slice(0, 10),
      skipped: 1,
    });
  });

  it("takes at most 10 files and counts the rest as skipped", () => {
    const list = files(11);
    expect(pastedFiles(clipboard(list))).toEqual({
      files: list.slice(0, 10),
      skipped: 1,
    });
  });
});
