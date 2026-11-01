import {
  act,
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
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { useAssistant } from "@/components/assistant/useAssistant";
import { ShellProvider } from "@/components/shell/ShellContext";
import type {
  AssistantBootstrap,
  AssistantMessage,
  AssistantModel,
  AssistantSession,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

// The versions of a turn on the page: which ones are shown, the switcher that
// brings another one back, and the line under a reply. The backend is a fake
// that flips which version is superseded, as its navigate does.

const mocks = vi.hoisted(() => ({
  bootstrap: vi.fn(),
  listSessions: vi.fn(),
  getSession: vi.fn(),
  updateSession: vi.fn(),
  switchVersion: vi.fn(),
  send: vi.fn(),
  usageSummary: vi.fn(),
  getPreferences: vi.fn(),
  setPreferences: vi.fn(),
  subscribe: vi.fn(),
  onAuthChange: vi.fn(),
  onDrag: vi.fn(),
  onDrop: vi.fn(),
}));
vi.mock("@/lib/api/assistant", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/assistant")>()),
  assistantApi: mocks,
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), {
    success: vi.fn(),
    warning: vi.fn(),
    error: vi.fn(),
  }),
}));
vi.mock("@/components/sessions/SessionMarkdown", () => ({
  SessionMarkdown: (props: { content: string }) => <p>{props.content}</p>,
}));

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
const session: AssistantSession = {
  id: "session-a",
  title: "Rewrites",
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
};

let clock = 0;
const turn = (
  id: string,
  role: "user" | "assistant",
  content: string,
  run: string,
  over: Partial<AssistantMessage> = {},
): AssistantMessage => ({
  id,
  role,
  content,
  createdAt: ++clock,
  status: "completed",
  nativeEntryId: `entry-${id}`,
  runId: run,
  superseded: false,
  ...(role === "user" ? { branchFrom: null } : { modelId: "balanced" }),
  ...over,
});

/** Two versions of the first turn: the second is shown. */
function twoVersions(): AssistantMessage[] {
  clock = 0;
  return [
    turn("q1", "user", "First wording", "run-1", { superseded: true }),
    turn("a1", "assistant", "Reply to the first wording", "run-1", {
      superseded: true,
    }),
    turn("q2", "user", "Second wording", "run-2"),
    turn("a2", "assistant", "Reply to the second wording", "run-2", {
      completedAt: new Date(2026, 9, 29, 14, 32).getTime(),
      usage: { inputTokens: 3, outputTokens: 312, estimatedCostUsd: "0.0004" },
    }),
  ];
}

let detail: AssistantSessionDetail;
/** The fake backend: shows the version whose question is `messageId`. */
function showVersionOf(messageId: string): AssistantSessionDetail {
  const shown = detail.messages.find((message) => message.id === messageId)!;
  const runs = new Set(
    detail.messages
      .filter((message) => message.role === "user")
      .map((message) => message.runId),
  );
  return {
    ...detail,
    messages: detail.messages.map((message) => ({
      ...message,
      superseded: runs.has(message.runId) && message.runId !== shown.runId,
    })),
  };
}

beforeEach(async () => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  detail = {
    session,
    messages: twoVersions(),
    attachments: [],
    usage: {},
  };
  mocks.bootstrap.mockResolvedValue(capabilities);
  mocks.getPreferences.mockResolvedValue({
    instructions: "",
    autoTitle: true,
    notifyOnFinish: true,
  });
  mocks.listSessions.mockImplementation(async () => ({
    sessions: [detail.session],
    unreadable: [],
    trashCount: 0,
    matches: {},
  }));
  mocks.getSession.mockImplementation(async () => detail);
  mocks.switchVersion.mockImplementation(async ({ messageId }) => {
    detail = showVersionOf(messageId);
    return detail;
  });
  mocks.usageSummary.mockResolvedValue({});
  mocks.subscribe.mockResolvedValue(() => undefined);
  mocks.onAuthChange.mockResolvedValue(() => undefined);
  mocks.onDrag.mockResolvedValue(() => undefined);
  mocks.onDrop.mockResolvedValue(() => undefined);
});

function page() {
  return (
    <ShellProvider openSettings={() => undefined}>
      <AssistantPage enabled />
    </ShellProvider>
  );
}
async function open() {
  render(page());
  await userEvent.click(await screen.findByTitle(/^Rewrites/));
  await screen.findByRole("heading", { level: 1, name: "Rewrites" });
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Message" })).toBeEnabled(),
  );
}
const switcher = () => screen.getByRole("group", { name: "Versions" });
const previous = () =>
  within(switcher()).getByRole("button", { name: "Previous version" });
const next = () =>
  within(switcher()).getByRole("button", { name: "Next version" });

describe("the versions of a turn", () => {
  it("shows only the version that is on screen, with no disclosure for the others", async () => {
    await open();
    expect(screen.getByText("Second wording")).toBeVisible();
    expect(screen.getByText("Reply to the second wording")).toBeVisible();
    expect(screen.queryByText("First wording")).toBeNull();
    expect(screen.queryByText("Reply to the first wording")).toBeNull();
    expect(screen.queryByText("View previous version")).toBeNull();
    expect(within(switcher()).getByText("2/2")).toBeVisible();
  });

  it("shows another version through the backend, and leaves the focus on the switcher", async () => {
    await open();
    previous().focus();
    await userEvent.click(previous());
    expect(mocks.switchVersion).toHaveBeenCalledWith({
      sessionId: "session-a",
      messageId: "q1",
      expectedOwner: owner,
      expectedGeneration: 1,
    });
    expect(await screen.findByText("First wording")).toBeVisible();
    expect(screen.getByText("Reply to the first wording")).toBeVisible();
    expect(screen.queryByText("Second wording")).toBeNull();
    expect(within(switcher()).getByText("1/2")).toBeVisible();
    // The same button, still focused (it is now the end of the range).
    await waitFor(() =>
      expect(previous()).toHaveAttribute("aria-disabled", "true"),
    );
    expect(previous()).toHaveFocus();
    expect(next()).not.toHaveAttribute("aria-disabled");
    await userEvent.click(next());
    expect(await screen.findByText("Second wording")).toBeVisible();
    expect(mocks.switchVersion).toHaveBeenLastCalledWith(
      expect.objectContaining({ messageId: "q2" }),
    );
    expect(next()).toHaveFocus();
  });

  it("takes no second switch while one is under way", async () => {
    await open();
    let finish: (value: AssistantSessionDetail) => void = () => undefined;
    mocks.switchVersion.mockImplementationOnce(
      () => new Promise((resolve) => (finish = resolve)),
    );
    await userEvent.click(previous());
    await waitFor(() =>
      expect(previous()).toHaveAttribute("aria-disabled", "true"),
    );
    expect(next()).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(next());
    await userEvent.click(previous());
    expect(mocks.switchVersion).toHaveBeenCalledOnce();
    await act(async () => finish(showVersionOf("q1")));
    expect(await screen.findByText("First wording")).toBeVisible();
  });

  it("is off while a reply runs", async () => {
    detail = {
      ...detail,
      session: {
        ...session,
        activeRun: { runId: "run-3", operationId: "op-3" },
      },
    };
    await open();
    expect(previous()).toHaveAttribute("aria-disabled", "true");
    expect(next()).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(previous());
    expect(mocks.switchVersion).not.toHaveBeenCalled();
  });

  it.each([
    ["assistant_version_unavailable", en.assistant.errors.versionUnavailable],
    [
      "assistant_version_switch_failed",
      en.assistant.errors.versionSwitchFailed,
    ],
    ["assistant_session_busy", en.assistant.errors.sessionBusy],
  ])(
    "says why a switch failed (%s) and keeps the version it was on",
    async (code, message) => {
      await open();
      mocks.switchVersion.mockRejectedValueOnce(code);
      await userEvent.click(previous());
      expect(await screen.findByRole("alert")).toHaveTextContent(message);
      expect(screen.getByText("Second wording")).toBeVisible();
      expect(within(switcher()).getByText("2/2")).toBeVisible();
      // Ready for another try.
      await waitFor(() =>
        expect(previous()).not.toHaveAttribute("aria-disabled"),
      );
    },
  );

  it("has no switcher for a turn that has one version", async () => {
    detail = { ...detail, messages: detail.messages.slice(2) };
    await open();
    expect(screen.getByText("Second wording")).toBeVisible();
    expect(screen.queryByRole("group", { name: "Versions" })).toBeNull();
  });
});

describe("the line under a reply, on the page", () => {
  it("names the model as the catalog does, with the time, and opens what the reply used", async () => {
    await open();
    // The picker in the header names the model too: this is the reply's own.
    const reply = screen.getByRole("article", { name: "Assistant" });
    expect(within(reply).getByText("Balanced model")).toBeVisible();
    expect(within(reply).getByText(/\d:32/)).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Reply details" }),
    );
    expect(await screen.findByRole("dialog")).toHaveTextContent(
      "Input 3 · Output 312 tokens · Estimated cost <$0.001",
    );
  });
});

describe("switching without the page", () => {
  // The page switches the switcher off in the trash; the hook refuses too,
  // as `send` does, so no other caller can reach the backend from there.
  it.each([
    ["an open conversation", null, true],
    ["a conversation in the trash", 5, false],
  ])(
    "on %s: asks the backend only when it can",
    async (_name, deletedAt, asked) => {
      detail = { ...detail, session: { ...session, deletedAt } };
      const { result } = renderHook(() => useAssistant(true));
      await waitFor(() => expect(result.current.bootstrap).not.toBeNull());
      act(() => result.current.select("session-a"));
      await waitFor(() => expect(result.current.loading).toBe(false));
      expect(result.current.detail?.session.id).toBe("session-a");
      let switched: boolean | undefined;
      await act(async () => {
        switched = await result.current.switchVersion("q1");
      });
      expect(switched).toBe(asked);
      expect(mocks.switchVersion).toHaveBeenCalledTimes(asked ? 1 : 0);
    },
  );
});
