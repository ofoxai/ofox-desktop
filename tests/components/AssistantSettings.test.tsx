import { createRef, type ReactNode } from "react";
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
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { AssistantSidebar } from "@/components/assistant/AssistantSidebar";
import { useAssistant } from "@/components/assistant/useAssistant";
import { finishNotice } from "@/components/assistant/useFinishNotice";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import {
  DEFAULT_ASSISTANT_PREFERENCES,
  type AssistantBootstrap,
  type AssistantEvent,
  type AssistantPreferences,
  type AssistantSession,
} from "@/lib/api/assistant";

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  usageSummary: vi.fn(),
  restore: vi.fn(),
  getPreferences: vi.fn(),
  setPreferences: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
  listener: undefined as ((event: AssistantEvent) => void) | undefined,
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
const toasts = vi.hoisted(() =>
  Object.assign(vi.fn(), {
    success: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  }),
);
vi.mock("sonner", () => ({ toast: toasts }));

const conversation = (id: string, title: string): AssistantSession => ({
  id,
  title,
  titleSource: "auto",
  modelId: "balanced",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt: 1,
  deletedAt: null,
  pinnedAt: null,
  activeRun: null,
});
const sessions = [
  conversation("session-a", "Trip to Kyoto"),
  conversation("session-b", "Budget review"),
];
const capabilities: AssistantBootstrap = {
  owner: "a".repeat(64),
  authorityGeneration: 1,
  runtime: { available: true, reason: null, version: "test" },
  models: [],
  defaultModelId: null,
  modelUnavailableReason: null,
  key: { status: "ready" },
};
const saved: AssistantPreferences = {
  instructions: "Answer in English.",
  autoTitle: true,
  notifyOnFinish: true,
};

beforeEach(async () => {
  for (const mock of Object.values(mocks)) {
    if (vi.isMockFunction(mock)) mock.mockReset();
  }
  toasts.mockReset();
  for (const mock of Object.values(toasts)) {
    if (vi.isMockFunction(mock)) mock.mockReset();
  }
  vi.mocked(isPermissionGranted).mockReset();
  vi.mocked(requestPermission).mockReset();
  vi.mocked(sendNotification).mockReset();
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("ja", "translation", ja, true, true);
  await i18n.changeLanguage("en");
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.listSessions.mockResolvedValue({
    sessions,
    unreadable: [],
    trashCount: 0,
    matches: {},
  });
  mocks.getSession.mockImplementation(async (id: string) => ({
    session: sessions.find((item) => item.id === id),
    messages: [],
    attachments: [],
    usage: {},
  }));
  mocks.usageSummary.mockResolvedValue({});
  mocks.restore.mockResolvedValue({
    imported: 0,
    skipped: 1,
    conflicts: 0,
    warning: null,
  });
  mocks.getPreferences.mockResolvedValue(DEFAULT_ASSISTANT_PREFERENCES);
  mocks.setPreferences.mockImplementation(
    async ({ instructions, autoTitle, notifyOnFinish }) => ({
      instructions,
      autoTitle,
      notifyOnFinish,
    }),
  );
  mocks.subscribe.mockImplementation(async (listener) => {
    mocks.listener = listener;
    return () => undefined;
  });
  mocks.onAuthChange.mockResolvedValue(() => undefined);
  vi.mocked(isPermissionGranted).mockResolvedValue(false);
  vi.mocked(requestPermission).mockResolvedValue("granted");
});

afterEach(() => {
  vi.restoreAllMocks();
});

// ── the dialog ───────────────────────────────────────────────────────────

type Assistant = ReturnType<typeof useAssistant>;

function renderSidebar(overrides: Partial<Assistant> = {}) {
  const assistant = {
    sessions: [],
    unreadable: [],
    matches: {},
    search: "",
    setSearch: vi.fn(),
    showTrash: false,
    viewTrash: vi.fn(),
    trashCount: 0,
    selectedId: null,
    busy: false,
    loading: false,
    transferring: false,
    transferResult: null,
    bootstrap: { owner: "a".repeat(64) },
    setPinned: vi.fn(),
    preferences: saved,
    reloadPreferences: vi.fn(async () => saved),
    savePreferences: vi.fn(async (next: AssistantPreferences) => next),
    ...overrides,
  } as unknown as Assistant;
  render(
    <AssistantSidebar
      id="sidebar"
      collapsed={false}
      assistant={assistant}
      canCreate
      onCreate={vi.fn()}
      onOpen={vi.fn()}
      onMoveToTrash={vi.fn()}
      onEmptyTrash={vi.fn()}
      onTransfer={vi.fn()}
      newConversationRef={createRef()}
      trashEntryRef={createRef()}
      trashBackRef={createRef()}
      emptyTrashRef={createRef()}
    />,
  );
  return assistant;
}

async function openSettings() {
  await userEvent.click(screen.getByRole("button", { name: "More actions" }));
  await userEvent.click(
    await screen.findByRole("menuitem", { name: "Assistant settings…" }),
  );
  return screen.findByRole("dialog", { name: "Assistant settings" });
}

const instructionsBox = (dialog: HTMLElement) =>
  within(dialog).getByRole("textbox", { name: "Custom instructions" });

describe("assistant settings dialog", () => {
  it("opens from the sidebar's More actions with the saved settings read again", async () => {
    const assistant = renderSidebar();
    const dialog = await openSettings();
    expect(assistant.reloadPreferences).toHaveBeenCalledTimes(1);
    await waitFor(() =>
      expect(instructionsBox(dialog)).toHaveValue("Answer in English."),
    );
    expect(within(dialog).getByText("18 / 4,000")).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        "Replies in every conversation follow these, starting with your next message.",
      ),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole("switch", { name: "Generate titles" }),
    ).toBeChecked();
    expect(
      within(dialog).getByRole("switch", {
        name: "Notify when a reply finishes",
      }),
    ).toBeChecked();
  });

  it("saves the instructions and both switches together when Save is pressed", async () => {
    const assistant = renderSidebar();
    const dialog = await openSettings();
    const box = instructionsBox(dialog);
    await waitFor(() => expect(box).toHaveValue("Answer in English."));
    await userEvent.clear(box);
    await userEvent.type(box, "Be brief.");
    await userEvent.click(
      within(dialog).getByRole("switch", { name: "Generate titles" }),
    );
    // Nothing is saved before Save.
    expect(assistant.savePreferences).not.toHaveBeenCalled();
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(assistant.savePreferences).toHaveBeenCalledWith({
      instructions: "Be brief.",
      autoTitle: false,
      notifyOnFinish: true,
    });
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Assistant settings" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("counts the characters and refuses to save over 4,000", async () => {
    renderSidebar();
    const dialog = await openSettings();
    const box = instructionsBox(dialog);
    await waitFor(() => expect(box).toHaveValue("Answer in English."));
    fireEvent.change(box, { target: { value: "a".repeat(4001) } });
    expect(within(dialog).getByText("4,001 / 4,000")).toBeInTheDocument();
    expect(
      within(dialog).getByText(
        "Custom instructions can have at most 4,000 characters. Shorten them.",
      ),
    ).toBeInTheDocument();
    expect(box).toHaveAttribute("aria-invalid", "true");
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeDisabled();
    fireEvent.change(box, { target: { value: "a".repeat(4000) } });
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeEnabled();
  });

  it("keeps the dialog and the edits when saving fails, and Cancel discards them", async () => {
    const assistant = renderSidebar({
      savePreferences: vi.fn(async () => {
        throw "assistant_storage_unavailable";
      }),
    });
    const dialog = await openSettings();
    const box = instructionsBox(dialog);
    await waitFor(() => expect(box).toHaveValue("Answer in English."));
    await userEvent.type(box, " Always.");
    await userEvent.click(within(dialog).getByRole("button", { name: "Save" }));
    expect(
      await within(dialog).findByText(
        "Local assistant data couldn't be read or saved. Check disk space and permissions.",
      ),
    ).toBeInTheDocument();
    expect(box).toHaveValue("Answer in English. Always.");
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Assistant settings" }),
      ).not.toBeInTheDocument(),
    );
    const again = await openSettings();
    await waitFor(() =>
      expect(instructionsBox(again)).toHaveValue("Answer in English."),
    );
    expect(assistant.reloadPreferences).toHaveBeenCalledTimes(2);
  });

  it("counts emoji as code points, like the Rust and runtime limits", async () => {
    renderSidebar();
    const dialog = await openSettings();
    const input = instructionsBox(dialog);
    await waitFor(() => expect(input).toBeEnabled());
    fireEvent.change(input, { target: { value: "🦊".repeat(4000) } });
    expect(within(dialog).getByText("4,000 / 4,000")).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeEnabled();
    fireEvent.change(input, { target: { value: "🦊".repeat(4001) } });
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeDisabled();
  });

  it("waits for the settings before they can be edited", async () => {
    let finish: (value: AssistantPreferences) => void = () => undefined;
    renderSidebar({
      reloadPreferences: vi.fn(
        () =>
          new Promise<AssistantPreferences>((resolve) => {
            finish = resolve;
          }),
      ),
    });
    const dialog = await openSettings();
    expect(within(dialog).getByText("Loading settings…")).toBeInTheDocument();
    expect(instructionsBox(dialog)).toBeDisabled();
    expect(within(dialog).getByRole("button", { name: "Save" })).toBeDisabled();
    await act(async () => finish(saved));
    expect(instructionsBox(dialog)).toBeEnabled();
    expect(instructionsBox(dialog)).toHaveValue("Answer in English.");
    expect(
      within(dialog).queryByText("Loading settings…"),
    ).not.toBeInTheDocument();
  });

  it("tells a failed read and offers nothing to save", async () => {
    renderSidebar({
      reloadPreferences: vi.fn(async () => {
        throw "assistant_sign_in_required";
      }),
    });
    const failed = await openSettings();
    expect(
      await within(failed).findByText(
        i18n.t("assistant.errors.signInRequired"),
      ),
    ).toBeInTheDocument();
    expect(within(failed).getByRole("button", { name: "Save" })).toBeDisabled();
  });
});

// ── finish notifications ─────────────────────────────────────────────────

function shell({ children }: { children: ReactNode }) {
  return (
    <ShellProvider openSettings={() => undefined}>{children}</ShellProvider>
  );
}

/** The assistant and the app's tabs, once the page is listening for runs. */
async function workspace(preferences = DEFAULT_ASSISTANT_PREFERENCES) {
  mocks.getPreferences.mockResolvedValue(preferences);
  const hook = renderHook(
    () => ({ assistant: useAssistant(true), shell: useShell() }),
    { wrapper: shell },
  );
  await waitFor(() => {
    expect(hook.result.current.assistant.connected).toBe(true);
    expect(hook.result.current.assistant.sessions).toHaveLength(2);
    expect(hook.result.current.assistant.preferences).toEqual(preferences);
  });
  return hook;
}

let runs = 0;
function end(sessionId: string, status: "completed" | "stopped" | "error") {
  runs += 1;
  act(() =>
    mocks.listener?.({
      type: "run_end",
      sessionId,
      runId: `run-${runs}`,
      operationId: `operation-${runs}`,
      data: { stopped: status === "stopped", status },
    }),
  );
}

const focused = (value: boolean) =>
  vi.spyOn(document, "hasFocus").mockReturnValue(value);

describe("finish notifications", () => {
  it("decides by the switch, the window's focus, the page and the conversation", () => {
    const base = {
      status: "completed" as const,
      enabled: true,
      focused: true,
      onAssistantPage: true,
      openSessionId: "a",
      sessionId: "a",
    };
    expect(finishNotice(base)).toBeNull();
    expect(finishNotice({ ...base, focused: false })).toBe("system");
    expect(finishNotice({ ...base, onAssistantPage: false })).toBe("toast");
    expect(finishNotice({ ...base, openSessionId: "b" })).toBe("toast");
    expect(finishNotice({ ...base, openSessionId: null })).toBe("toast");
    expect(finishNotice({ ...base, status: "error", focused: false })).toBe(
      "system",
    );
    // A stopped run was stopped by the user, or ended for them (a model
    // change, the trash, a sign-out).
    expect(
      finishNotice({ ...base, status: "stopped", focused: false }),
    ).toBeNull();
    expect(
      finishNotice({ ...base, enabled: false, focused: false }),
    ).toBeNull();
  });

  it("asks for permission at the first notice only, and tells the system the title alone", async () => {
    focused(false);
    await workspace();
    // Never at startup.
    expect(isPermissionGranted).not.toHaveBeenCalled();
    expect(requestPermission).not.toHaveBeenCalled();
    end("session-a", "completed");
    await waitFor(() =>
      expect(sendNotification).toHaveBeenCalledWith({
        title: "Trip to Kyoto",
        body: "Reply finished",
      }),
    );
    end("session-b", "error");
    await waitFor(() =>
      expect(sendNotification).toHaveBeenLastCalledWith({
        title: "Budget review",
        body: "Reply failed",
      }),
    );
    expect(isPermissionGranted).toHaveBeenCalledTimes(1);
    expect(requestPermission).toHaveBeenCalledTimes(1);
    end("session-a", "stopped");
    await act(async () => undefined);
    expect(sendNotification).toHaveBeenCalledTimes(2);
    expect(toasts).not.toHaveBeenCalled();
  });

  it.each(["sign-out", "unmount", "disabled"])(
    "discards a pending notification after %s",
    async (change) => {
      focused(false);
      let permit!: (permission: "granted") => void;
      vi.mocked(requestPermission).mockImplementation(
        () =>
          new Promise((resolve) => {
            permit = resolve;
          }),
      );
      const hook = await workspace();
      end("session-a", "completed");
      await waitFor(() => expect(requestPermission).toHaveBeenCalledTimes(1));
      if (change === "sign-out") {
        act(() => mocks.onAuthChange.mock.calls[0][0]("expired"));
      } else if (change === "unmount") {
        hook.unmount();
      } else {
        await act(async () => {
          await hook.result.current.assistant.savePreferences({
            ...saved,
            notifyOnFinish: false,
          });
        });
      }
      await act(async () => permit("granted"));
      expect(sendNotification).not.toHaveBeenCalled();
    },
  );

  it("sends nothing once permission is refused, and does not ask again", async () => {
    focused(false);
    vi.mocked(requestPermission).mockResolvedValue("denied");
    await workspace();
    end("session-a", "completed");
    end("session-b", "completed");
    await waitFor(() => expect(requestPermission).toHaveBeenCalledTimes(1));
    await act(async () => undefined);
    expect(isPermissionGranted).toHaveBeenCalledTimes(1);
    expect(sendNotification).not.toHaveBeenCalled();
  });

  it("shows a notice with View while the user is on another page or conversation", async () => {
    focused(true);
    const hook = await workspace();
    expect(hook.result.current.shell.active).toBe("tools");
    end("session-a", "completed");
    await waitFor(() => expect(toasts).toHaveBeenCalledTimes(1));
    const [title, options] = toasts.mock.calls[0];
    expect(title).toBe("Trip to Kyoto");
    expect(options.description).toBe("Reply finished");
    expect(options.action.label).toBe("View");
    expect(sendNotification).not.toHaveBeenCalled();
    act(() => options.action.onClick());
    await waitFor(() => {
      expect(hook.result.current.shell.active).toBe("assistant");
      expect(hook.result.current.assistant.selectedId).toBe("session-a");
    });
    // On screen: nothing to tell. Another conversation: a notice.
    end("session-a", "completed");
    end("session-b", "error");
    await waitFor(() => expect(toasts).toHaveBeenCalledTimes(2));
    expect(toasts.mock.calls[1][0]).toBe("Budget review");
    expect(toasts.mock.calls[1][1].description).toBe("Reply failed");
  });

  it("tells nothing while the switch is off", async () => {
    focused(false);
    await workspace({
      ...DEFAULT_ASSISTANT_PREFERENCES,
      notifyOnFinish: false,
    });
    end("session-a", "completed");
    await act(async () => undefined);
    expect(isPermissionGranted).not.toHaveBeenCalled();
    expect(sendNotification).not.toHaveBeenCalled();
    expect(toasts).not.toHaveBeenCalled();
  });

  it("saves the settings for the page's account and reads them again after a restore", async () => {
    focused(true);
    const hook = await workspace();
    await act(async () => {
      await hook.result.current.assistant.savePreferences(saved);
    });
    expect(mocks.setPreferences).toHaveBeenCalledWith({
      ...saved,
      expectedOwner: "a".repeat(64),
      expectedGeneration: 1,
    });
    expect(hook.result.current.assistant.preferences).toEqual(saved);
    mocks.getPreferences.mockResolvedValue({ ...saved, autoTitle: false });
    await act(async () => {
      await hook.result.current.assistant.transfer("restore");
    });
    await waitFor(() =>
      expect(hook.result.current.assistant.preferences).toEqual({
        ...saved,
        autoTitle: false,
      }),
    );
  });
});
