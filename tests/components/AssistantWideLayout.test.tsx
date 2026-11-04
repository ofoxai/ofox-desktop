import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import {
  AssistantLoginRequired,
  AssistantPage,
} from "@/components/assistant/AssistantPage";
import { ShellProvider } from "@/components/shell/ShellContext";
import type {
  AssistantBootstrap,
  AssistantMessage,
  AssistantModel,
  AssistantSession,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

// The assistant page of a wide window: no frame, the title only where the
// list does not show it, and a new conversation's composer in the middle.

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
const platform = vi.hoisted(() => ({ mac: false }));
vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => platform.mac,
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
const model: AssistantModel = {
  id: "balanced",
  name: "Balanced model",
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
};
const capabilities: AssistantBootstrap = {
  owner,
  authorityGeneration: 1,
  runtime: { available: true, reason: null, version: "test" },
  models: [model],
  defaultModelId: "balanced",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: { available: true, unitPriceUsd: "0.014", model: "x/search" },
};
const session = (title = "Rewrites"): AssistantSession => ({
  id: "session-a",
  title,
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
});
const messages: AssistantMessage[] = [
  {
    id: "q1",
    role: "user",
    content: "Shorten this",
    createdAt: 1,
    status: "completed",
    runId: "run-1",
  },
  {
    id: "a1",
    role: "assistant",
    content: "Done.",
    createdAt: 2,
    status: "completed",
    runId: "run-1",
    modelId: "balanced",
  },
];
let detail: AssistantSessionDetail;

const jsdomWidth = window.innerWidth;
beforeEach(async () => {
  for (const mock of Object.values(mocks)) mock.mockReset();
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  detail = { session: session(), messages, attachments: [], usage: {} };
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
  mocks.updateSession.mockImplementation(async (patch) => {
    detail = { ...detail, session: { ...detail.session, ...patch } };
    return detail.session;
  });
  mocks.usageSummary.mockResolvedValue({});
  mocks.subscribe.mockResolvedValue(() => undefined);
  mocks.onAuthChange.mockResolvedValue(() => undefined);
  mocks.onDrag.mockResolvedValue(() => undefined);
  mocks.onDrop.mockResolvedValue(() => undefined);
});
afterEach(() => {
  window.innerWidth = jsdomWidth;
  platform.mac = false;
});
const resizeTo = (width: number) =>
  act(() => {
    window.innerWidth = width;
    window.dispatchEvent(new Event("resize"));
  });

function page(width: number) {
  window.innerWidth = width;
  return render(
    <ShellProvider openSettings={() => undefined}>
      <AssistantPage enabled />
    </ShellProvider>,
  );
}
async function open(width = 1512) {
  page(width);
  await userEvent.click(
    await screen.findByTitle(
      // The list row (its title holds the date), not "New conversation (Ctrl+N)".
      new RegExp(`^(Rewrites|${en.assistant.untitled})\\s.*\\d{4}`),
    ),
  );
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "Message" })).toBeEnabled(),
  );
  await screen.findByText("Done.");
}
const chat = () => screen.getByRole("region", { name: en.assistant.chat });
const topRow = () => chat().querySelector("header")!;

describe("the assistant page of a wide window", () => {
  it("shows no title in the top row while the list is open", async () => {
    await open();
    // Named for a screen reader only: the list shows which one it is.
    expect(screen.getByRole("heading", { level: 1 })).toHaveClass("sr-only");
    expect(
      within(topRow()).getByRole("combobox", { name: /model/i }),
    ).toBeInTheDocument();
    const [picker, menu] = [
      within(topRow()).getByRole("combobox", { name: /model/i }),
      within(topRow()).getByRole("button", { name: "Conversation actions" }),
    ];
    expect(
      picker.compareDocumentPosition(menu) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("shows the title in the top row once the list is collapsed", async () => {
    await open();
    await userEvent.click(
      screen.getByRole("button", { name: en.assistant.sidebar }),
    );
    expect(
      within(topRow()).getByRole("heading", { level: 1, name: "Rewrites" }),
    ).not.toHaveClass("sr-only");
  });

  it("renames in the top row even while the list is open", async () => {
    await open();
    await userEvent.click(
      screen.getByRole("button", { name: "Conversation actions" }),
    );
    await userEvent.click(screen.getByRole("menuitem", { name: "Rename" }));
    const field = within(topRow()).getByRole("textbox", {
      name: en.assistant.conversationTitle,
    });
    await userEvent.clear(field);
    await userEvent.type(field, "Shorter{Enter}");
    await waitFor(() =>
      expect(mocks.updateSession).toHaveBeenCalledWith(
        expect.objectContaining({ title: "Shorter" }),
      ),
    );
  });

  it("falls back to the page name for an untitled conversation when collapsed", async () => {
    detail = { ...detail, session: session("") };
    await open();
    await userEvent.click(
      screen.getByRole("button", { name: en.assistant.sidebar }),
    );
    expect(
      within(topRow()).getByRole("heading", { level: 1 }),
    ).toHaveTextContent(en.assistant.title);
  });

  it("drops the frame and sets the roomier sizes", async () => {
    await open();
    const root = chat().parentElement!;
    expect(root).toHaveAttribute("data-layout", "wide");
    expect(root).not.toHaveClass("rounded-list");
    expect(root.className).toContain("[--fs-ui:15px]");
    expect(root.className).toContain("[--icon-btn:30px]");
  });

  it("starts a new conversation with its composer in the middle, the starters under it", async () => {
    page(1512);
    const welcome = await screen.findByRole("heading", {
      level: 2,
      name: en.assistant.welcome,
    });
    const composer = await screen.findByRole("textbox", { name: "Message" });
    const body = chat().querySelector("[data-centred]")!;
    expect(body).toHaveAttribute("data-centred", "true");
    expect(body).toContainElement(welcome);
    expect(body).toContainElement(composer);
    const starter = screen.getAllByRole("button", {
      name: en.assistant.starters.summary.label,
    })[0];
    expect(
      composer.compareDocumentPosition(starter) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("is unchanged in a narrow window", async () => {
    await open(1000);
    const root = chat().parentElement!;
    expect(root).toHaveAttribute("data-layout", "narrow");
    expect(root).toHaveClass("rounded-list", "border", "border-line");
    expect(
      within(topRow()).getByRole("heading", { level: 1, name: "Rewrites" }),
    ).not.toHaveClass("sr-only");
  });
});

it("switches back to the narrow page when the window shrinks", async () => {
  await open();
  act(() => {
    window.innerWidth = 1000;
    window.dispatchEvent(new Event("resize"));
  });
  expect(chat().parentElement).toHaveAttribute("data-layout", "narrow");
  expect(screen.getByText("Done.")).toBeVisible();
});

describe("switching between the narrow and the wide page", () => {
  it("drags the window from the empty top of a wide chat on macOS", async () => {
    platform.mac = true;
    await open();
    const empty = [
      ...topRow().querySelectorAll<HTMLElement>("[data-tauri-drag-region]"),
    ].filter((element) => element !== topRow());
    expect(empty.some((element) => element.classList.contains("flex-1"))).toBe(
      true,
    );
  });

  it("keeps the model picker as it was", async () => {
    await open();
    const picker = within(topRow()).getByRole("combobox", { name: /model/i });
    picker.focus();
    resizeTo(1000);
    resizeTo(1512);
    const after = within(topRow()).getByRole("combobox", { name: /model/i });
    expect(after).toBe(picker);
    expect(after).toHaveFocus();
  });

  it("keeps a transcript that followed its end at the end", async () => {
    await open();
    const transcript = chat().querySelector<HTMLElement>("[aria-busy]")!;
    Object.defineProperty(transcript, "scrollHeight", {
      configurable: true,
      value: 2400,
    });
    transcript.scrollTop = 0;
    resizeTo(1000);
    expect(transcript.scrollTop).toBe(2400);
  });

  it("starts a wide composer taller", async () => {
    await open();
    const composer = screen.getByRole("textbox", { name: "Message" });
    expect(composer.style.height).toBe("96px");
    resizeTo(1000);
    expect(composer.style.height).toBe("76px");
  });
});

it("frames a signed-out assistant only in a narrow window", () => {
  window.innerWidth = 1512;
  const { unmount } = render(
    <ShellProvider openSettings={() => undefined}>
      <AssistantLoginRequired onLoginRequested={() => undefined} />
    </ShellProvider>,
  );
  const wide = screen.getByRole("heading", { level: 1 }).closest("section")!;
  expect(wide).not.toHaveClass("rounded-list");
  unmount();
  window.innerWidth = 1000;
  render(
    <ShellProvider openSettings={() => undefined}>
      <AssistantLoginRequired onLoginRequested={() => undefined} />
    </ShellProvider>,
  );
  expect(
    screen.getByRole("heading", { level: 1 }).closest("section"),
  ).toHaveClass("rounded-list", "border");
});
