import { createRef } from "react";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import {
  AssistantSidebar,
  type AssistantSidebarHandle,
} from "@/components/assistant/AssistantSidebar";
import type { useAssistant } from "@/components/assistant/useAssistant";
import type { AssistantSession } from "@/lib/api/assistant";

vi.mock("sonner", () => ({ toast: { success: vi.fn(), warning: vi.fn() } }));

const at = (day: number, hour = 12, minute = 0, second = 0) =>
  new Date(2026, 9, day, hour, minute, second).getTime();

const conversation = (
  id: string,
  title: string,
  updatedAt: number,
  pinnedAt: number | null = null,
): AssistantSession => ({
  id,
  title,
  titleSource: "auto",
  modelId: "balanced",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt,
  deletedAt: null,
  pinnedAt,
  activeRun: null,
});

type Assistant = ReturnType<typeof useAssistant>;
const setPinned = vi.fn(async () => true);
const onOpen = vi.fn();
const onEmptyTrash = vi.fn();
const onMoveToTrash = vi.fn();

function fakeAssistant(overrides: Partial<Assistant> = {}) {
  return {
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
    setPinned,
    ...overrides,
  } as unknown as Assistant;
}

function renderSidebar(
  overrides: Partial<Assistant> = {},
  handle = createRef<AssistantSidebarHandle>(),
) {
  const props = {
    id: "sidebar",
    collapsed: false,
    canCreate: true,
    onCreate: vi.fn(),
    onOpen,
    onEmptyTrash,
    onMoveToTrash,
    onTransfer: vi.fn(),
    newConversationRef: createRef<HTMLButtonElement>(),
    trashEntryRef: createRef<HTMLButtonElement>(),
    trashBackRef: createRef<HTMLButtonElement>(),
    emptyTrashRef: createRef<HTMLButtonElement>(),
  };
  const view = (next: Partial<Assistant>) => (
    <AssistantSidebar
      ref={handle}
      {...props}
      assistant={fakeAssistant({ ...overrides, ...next })}
    />
  );
  const result = render(view({}));
  return {
    ...result,
    rerenderWith: (next: Partial<Assistant>) => result.rerender(view(next)),
  };
}

/** Each group heading with the titles listed under it. */
function listed() {
  const list = screen.getByRole("list", { name: "Conversations" });
  return within(list)
    .getAllByRole("heading")
    .map((heading) => {
      const group = heading.closest("li") as HTMLElement;
      return [
        heading.textContent,
        within(group)
          .getAllByRole("button")
          // Each row's own menu is not a conversation.
          .filter((button) => !button.hasAttribute("aria-haspopup"))
          .map((button) => button.textContent),
      ];
    });
}

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("ja", "translation", ja, true, true);
  await i18n.changeLanguage("en");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("date groups", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(at(29, 12));
  });

  it("heads each group, pinned first, and leaves out the empty ones", () => {
    renderSidebar({
      sessions: [
        conversation("p", "Pinned one", at(1), at(28)),
        conversation("t", "Today one", at(29, 9)),
        conversation("y", "Yesterday one", at(28, 9)),
        conversation("w", "Week one", at(25, 9)),
        conversation("e", "Old one", at(1)),
      ],
    });
    expect(listed()).toEqual([
      ["Pinned", ["Pinned one"]],
      ["Today", ["Today one"]],
      ["Yesterday", ["Yesterday one"]],
      ["Past 7 days", ["Week one"]],
      ["Earlier", ["Old one"]],
    ]);
  });

  it("makes the group headings second-level headings", () => {
    renderSidebar({
      sessions: [
        conversation("p", "Pinned one", at(1), at(28)),
        conversation("t", "Today one", at(29, 9)),
      ],
    });
    expect(screen.getAllByRole("heading").map((h) => h.tagName)).toEqual([
      "H2",
      "H2",
    ]);
  });

  it("leaves out the groups nothing is in", () => {
    renderSidebar({ sessions: [conversation("t", "Only today", at(29, 9))] });
    expect(listed()).toEqual([["Today", ["Only today"]]]);
  });

  it("names the groups in each language", async () => {
    const sessions = [
      conversation("p", "A", at(1), at(28)),
      conversation("t", "B", at(29, 9)),
    ];
    for (const [language, tree] of [
      ["zh", zh],
      ["ja", ja],
    ] as const) {
      await i18n.changeLanguage(language);
      const { unmount } = renderSidebar({ sessions });
      const list = screen.getByRole("list", {
        name: tree.assistant.conversations,
      });
      expect(
        within(list)
          .getAllByRole("heading")
          .map((heading) => heading.textContent),
      ).toEqual([tree.assistant.groups.pinned, tree.assistant.groups.today]);
      unmount();
    }
  });

  it("moves a conversation from Today to Yesterday when midnight passes", () => {
    vi.setSystemTime(at(29, 23, 59, 30));
    renderSidebar({
      sessions: [conversation("t", "Late chat", at(29, 22))],
    });
    expect(listed()).toEqual([["Today", ["Late chat"]]]);
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(listed()).toEqual([["Yesterday", ["Late chat"]]]);
  });

  it("looks again when the window comes back after the computer slept", () => {
    vi.setSystemTime(at(29, 22));
    renderSidebar({
      sessions: [conversation("t", "Late chat", at(29, 21))],
    });
    expect(listed()).toEqual([["Today", ["Late chat"]]]);
    // The clock moved on without any timer running.
    vi.setSystemTime(at(30, 8));
    act(() => {
      window.dispatchEvent(new Event("focus"));
    });
    expect(listed()).toEqual([["Yesterday", ["Late chat"]]]);
  });

  it("does not group the trash", () => {
    renderSidebar({
      showTrash: true,
      trashCount: 2,
      sessions: [
        conversation("a", "Trashed one", at(29, 9)),
        conversation("b", "Trashed two", at(1)),
      ],
    });
    const list = screen.getByRole("list", { name: "Trash" });
    expect(within(list).queryAllByRole("heading")).toHaveLength(0);
    expect(
      within(list)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Trashed one", "Trashed two"]);
  });
});

describe("pins", () => {
  const sessions = [
    conversation("p", "Pinned one", at(1), at(28)),
    conversation("t", "Plain one", at(29, 9)),
  ];

  it("leaves the pin icon off the rows: the group heading says it", () => {
    vi.useFakeTimers();
    vi.setSystemTime(at(29, 12));
    renderSidebar({ sessions });
    expect(
      screen.getByRole("heading", { name: "Pinned" }).closest("li"),
    ).toHaveTextContent("Pinned one");
    for (const name of ["Pinned one", "Plain one"])
      expect(
        screen.getByRole("button", { name }).querySelector(".lucide-pin"),
      ).toBeNull();
  });

  it("pins from the row's menu, and unpins a pinned row", async () => {
    renderSidebar({ sessions });
    const [pinnedMenu, plainMenu] = screen.getAllByRole("button", {
      name: "Options",
    });
    await userEvent.click(plainMenu);
    await userEvent.click(await screen.findByRole("menuitem", { name: "Pin" }));
    expect(setPinned).toHaveBeenLastCalledWith("t", true);
    await userEvent.click(pinnedMenu);
    await userEvent.click(
      await screen.findByRole("menuitem", { name: "Unpin" }),
    );
    expect(setPinned).toHaveBeenLastCalledWith("p", false);
  });

  it("offers no row menu in the trash", () => {
    renderSidebar({ sessions, showTrash: true, trashCount: 2 });
    expect(screen.queryByRole("button", { name: "Options" })).toBeNull();
  });

  it("moves a conversation to the trash from the row's menu", async () => {
    renderSidebar({ sessions });
    const [pinnedMenu] = screen.getAllByRole("button", { name: "Options" });
    await userEvent.click(pinnedMenu);
    const items = await screen.findAllByRole("menuitem");
    expect(items.map((item) => item.textContent)).toEqual([
      "Unpin",
      "Move to trash",
    ]);
    await userEvent.click(
      screen.getByRole("menuitem", { name: "Move to trash" }),
    );
    expect(onMoveToTrash).toHaveBeenCalledWith(sessions[0]);
    expect(setPinned).not.toHaveBeenCalled();
  });

  it("does not offer the trash while another operation is running", async () => {
    renderSidebar({ sessions, busy: true });
    await userEvent.click(
      screen.getAllByRole("button", { name: "Options" })[1],
    );
    expect(
      await screen.findByRole("menuitem", { name: "Move to trash" }),
    ).toHaveAttribute("aria-disabled", "true");
    expect(screen.getByRole("menuitem", { name: "Pin" })).not.toHaveAttribute(
      "aria-disabled",
      "true",
    );
  });

  it("makes room for the row's menu button while it shows, so the title ends before it", () => {
    renderSidebar({ sessions });
    const row = screen.getByRole("button", { name: "Plain one" });
    for (const reserved of [
      "group-hover:pr-8",
      "group-focus-within:pr-8",
      "group-has-[[data-state=open]]:pr-8",
    ])
      expect(row).toHaveClass(reserved);
  });

  it("gives focus to a pinned row once it is listed in its new place", async () => {
    const { rerenderWith } = renderSidebar({ sessions });
    const [, plainMenu] = screen.getAllByRole("button", {
      name: "Options",
    });
    await userEvent.click(plainMenu);
    await userEvent.click(await screen.findByRole("menuitem", { name: "Pin" }));
    // Not yet listed as pinned: the row that is still there is not the one.
    await waitFor(() => expect(setPinned).toHaveBeenCalled());
    expect(screen.getByRole("button", { name: "Plain one" })).not.toHaveFocus();
    rerenderWith({
      sessions: [
        conversation("t", "Plain one", at(29, 9), at(29, 10)),
        sessions[0],
      ],
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Plain one" })).toHaveFocus(),
    );
  });
});

describe("search", () => {
  it("highlights the search in titles", () => {
    renderSidebar({
      search: "plan",
      sessions: [conversation("a", "Trip planning", at(29, 9))],
    });
    // The title attribute is the row's whole title: its name is split by the marks.
    const marks = screen.getByTitle(/^Trip planning/).querySelectorAll("mark");
    expect([...marks].map((mark) => mark.textContent)).toEqual(["plan"]);
  });

  it("marks nothing for a search that is only spaces", () => {
    renderSidebar({
      search: " ",
      sessions: [conversation("a", "Trip planning", at(29, 9))],
    });
    expect(
      screen.getByTitle(/^Trip planning/).querySelectorAll("mark"),
    ).toHaveLength(0);
  });

  it("opens a conversation with the message its search found, if any", async () => {
    renderSidebar({
      search: "plan",
      matches: { a: "message-2" },
      sessions: [
        conversation("a", "Trip", at(29, 9)),
        conversation("b", "Weekly plan", at(29, 8)),
      ],
    });
    await userEvent.click(screen.getByRole("button", { name: "Trip" }));
    expect(onOpen).toHaveBeenLastCalledWith("a", "message-2");
    await userEvent.click(screen.getByRole("button", { name: "Weekly plan" }));
    expect(onOpen).toHaveBeenLastCalledWith("b", undefined);
  });
});

describe("the trash's header", () => {
  it("keeps the search above the way back and Empty", () => {
    renderSidebar({ showTrash: true, trashCount: 1 });
    const search = screen.getByRole("button", {
      name: "Search conversations",
    });
    const back = screen.getByRole("button", { name: "Back to conversations" });
    const empty = screen.getByRole("button", { name: "Empty trash" });
    expect(
      search.compareDocumentPosition(back) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(
      back.compareDocumentPosition(empty) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("asks to empty the trash when there is something in it", async () => {
    renderSidebar({ showTrash: true, trashCount: 3 });
    await userEvent.click(screen.getByRole("button", { name: "Empty trash" }));
    expect(onEmptyTrash).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["it is empty", { trashCount: 0 }],
    ["a backup is running", { trashCount: 2, transferring: true }],
    ["an operation is running", { trashCount: 2, busy: true }],
    ["a conversation is loading", { trashCount: 2, loading: true }],
  ])("cannot empty the trash while %s", (_, state) => {
    renderSidebar({ showTrash: true, ...state });
    expect(screen.getByRole("button", { name: "Empty trash" })).toBeDisabled();
  });

  it("has no Empty button among the conversations", () => {
    renderSidebar({ trashCount: 3 });
    expect(screen.queryByRole("button", { name: "Empty trash" })).toBeNull();
  });
});
