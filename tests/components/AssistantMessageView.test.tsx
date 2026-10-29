import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantMessageView } from "@/components/assistant/AssistantMessageView";
import { TooltipProvider } from "@/components/ui/tooltip";
import { chipNotes } from "../utils/chipNotes";
import type {
  AssistantAttachment,
  AssistantMessage,
  AssistantSource,
} from "@/lib/api/assistant";

const markdown = vi.hoisted(() => vi.fn());
const markdownProps = vi.hoisted(() => vi.fn());
const copyButtonRenders = vi.hoisted(() => vi.fn());
const external = vi.hoisted(() => ({
  openExternal: vi.fn(),
  toastError: vi.fn(),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: external.openExternal },
}));
vi.mock("sonner", () => ({ toast: { error: external.toastError } }));
// Counts how often a message draws its actions, to tell a message that is
// drawn again from one that is not.
vi.mock("@/components/ui/copy-button", async (importOriginal) => {
  const original =
    await importOriginal<typeof import("@/components/ui/copy-button")>();
  return {
    ...original,
    CopyButton: (props: Parameters<typeof original.CopyButton>[0]) => {
      copyButtonRenders(props.text);
      return <original.CopyButton {...props} />;
    },
  };
});
vi.mock("@/components/sessions/SessionMarkdown", () => ({
  SessionMarkdown: (props: { content: string }) => {
    markdown(props.content);
    markdownProps(props);
    return <p>{props.content}</p>;
  },
}));

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  external.openExternal.mockReset().mockResolvedValue(undefined);
  external.toastError.mockReset();
  copyButtonRenders.mockReset();
});

function view(
  message: Partial<AssistantMessage>,
  handlers: {
    onEdit?: (
      messageId: string,
      text: string,
      attachments: AssistantAttachment[],
    ) => Promise<boolean>;
    onEditingChange?: (editing: boolean) => void;
    onRegenerate?: (messageId: string) => void;
    onRetry?: (messageId: string, modelId?: string) => Promise<boolean> | void;
  } = {},
  disabled = false,
  latest = false,
) {
  return (
    <AssistantMessageView
      message={{
        id: "m",
        role: "assistant",
        content: "",
        createdAt: 1,
        nativeEntryId: "entry",
        runId: "run",
        ...message,
      }}
      disabled={disabled}
      latest={latest}
      onEdit={handlers.onEdit ?? (async () => false)}
      onEditingChange={handlers.onEditingChange}
      onRegenerate={handlers.onRegenerate ?? (() => undefined)}
      onRetry={handlers.onRetry}
    />
  );
}

const file = (
  over: Partial<AssistantAttachment> = {},
): AssistantAttachment => ({
  id: "f1",
  name: "notes.txt",
  mimeType: "text/plain",
  sizeBytes: 1200,
  status: "ready",
  reason: null,
  previewUrl: null,
  details: null,
  ...over,
});
const scanned = file({
  id: "f2",
  name: "scan.pdf",
  mimeType: "application/pdf",
  status: "partial",
  reason: "assistant_attachment_partial",
  details: { format: "pdf", pageCount: 3, pagesWithoutText: [3] },
});
const locked = file({
  id: "f3",
  name: "locked.pdf",
  mimeType: "application/pdf",
  status: "unreadable",
  reason: "assistant_attachment_unreadable",
  details: { format: "pdf", reason: "password_protected" },
});

function transcript(partial: string) {
  return (
    <>
      <AssistantMessageView
        message={{
          id: "saved",
          role: "assistant",
          content: "Saved long document analysis",
          createdAt: 1,
        }}
        disabled
        onEdit={async () => false}
        onRegenerate={() => undefined}
      />
      <AssistantMessageView
        message={{
          id: "stream",
          role: "assistant",
          content: partial,
          createdAt: 2,
          status: "streaming",
        }}
        disabled
        onEdit={async () => false}
        onRegenerate={() => undefined}
      />
    </>
  );
}

describe("assistant transcript rendering", () => {
  it.each([
    ["assistant_insufficient_balance", en.assistant.errors.insufficientBalance],
    [
      "assistant_search_outcome_unknown",
      en.assistant.errors.searchOutcomeUnknown,
    ],
    ["untrusted provider text with a credential", en.assistant.errors.generic],
  ])(
    "shows the saved failure %s as plain text that is not announced again",
    (errorCode, reason) => {
      const saved = { status: "error" as const, errorCode };
      render(view(saved));
      // The live failure was announced by the page's banner when it
      // happened; opening a saved conversation announces nothing.
      expect(screen.queryByRole("alert")).not.toBeInTheDocument();
      const text = screen.getByText(reason);
      expect(text).toBeVisible();
      expect(text.closest("[role], [aria-live]")).toBeNull();
      expect(screen.queryByText(errorCode)).not.toBeInTheDocument();
    },
  );

  it("keeps old failed messages readable when they have no saved reason", () => {
    render(view({ status: "error" }));
    expect(screen.getByText(en.assistant.failed)).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps completed Markdown parsed once while another reply streams", () => {
    const rendered = render(transcript("First fragment"));
    for (let index = 0; index < 20; index++)
      rendered.rerender(transcript(`Streaming fragment ${index}`));
    expect(screen.getByText("Saved long document analysis")).toBeVisible();
    expect(screen.getByText("Streaming fragment 19")).toBeVisible();
    expect(
      markdown.mock.calls.filter(
        ([content]) => content === "Saved long document analysis",
      ),
    ).toHaveLength(1);
  });

  it("shows a typing state until the first words of a reply arrive", () => {
    const rendered = render(view({ status: "streaming" }));
    expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
    expect(
      screen.queryByRole("button", { name: "Regenerate" }),
    ).not.toBeInTheDocument();
    rendered.rerender(view({ status: "streaming", content: "Hello" }));
    expect(screen.queryByText("Thinking…")).toBeNull();
    expect(screen.getByText("Hello")).toBeVisible();
  });

  it("offers copy and regenerate on a finished reply", async () => {
    const onRegenerate = vi.fn();
    render(view({ content: "Answer", status: "completed" }, { onRegenerate }));
    expect(screen.getByRole("button", { name: "Copy" })).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Regenerate" }));
    expect(onRegenerate).toHaveBeenCalledOnce();
    expect(onRegenerate).toHaveBeenCalledWith("m");
  });

  it("draws math in a reply and copies it as written", () => {
    const content = "Area \\(\\pi r^2\\)";
    render(view({ content, status: "completed" }));
    expect(markdownProps).toHaveBeenCalledWith(
      expect.objectContaining({ content, math: true }),
    );
    expect(copyButtonRenders).toHaveBeenCalledWith(content);
  });

  it("shows a question as it was typed, without drawing math", () => {
    const content = "find . \\( -name a \\)";
    render(view({ role: "user", content }));
    expect(markdownProps).toHaveBeenCalledWith(
      expect.objectContaining({ content, math: false }),
    );
  });

  it("edits a question in place and resends it", async () => {
    const onEdit = vi.fn(async () => true);
    render(view({ role: "user", content: "Original" }, { onEdit }));
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    const editor = screen.getByRole("textbox", { name: "Edit and resend" });
    expect(editor).toHaveFocus();
    await userEvent.clear(editor);
    await userEvent.type(editor, "Better question");
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(onEdit).toHaveBeenCalledWith("m", "Better question", []);
    expect(
      screen.queryByRole("textbox", { name: "Edit and resend" }),
    ).not.toBeInTheDocument();
  });

  it("tells the page while a question is being edited, and when it no longer is", async () => {
    const onEditingChange = vi.fn();
    const rendered = render(
      view({ role: "user", content: "Original" }, { onEditingChange }),
    );
    expect(onEditingChange).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    expect(onEditingChange).toHaveBeenLastCalledWith(true);
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(onEditingChange).toHaveBeenLastCalledWith(false);
    // Gone with its edit still open: that edit is over too.
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    expect(onEditingChange).toHaveBeenLastCalledWith(true);
    rendered.unmount();
    expect(onEditingChange).toHaveBeenLastCalledWith(false);
    expect(onEditingChange).toHaveBeenCalledTimes(4);
  });

  it.each(["user", "assistant"] as const)(
    "offers no edit or regenerate on a %s message the model never received",
    (role) => {
      // The backend can branch only from a turn in the model's history.
      render(view({ role, content: "Kept", nativeEntryId: null }));
      expect(screen.getByRole("button", { name: "Copy" })).toBeVisible();
      expect(
        screen.queryByRole("button", { name: /Edit and resend|Regenerate/ }),
      ).toBeNull();
    },
  );

  it("marks a stopped or failed reply", () => {
    const rendered = render(view({ content: "Part", status: "stopped" }));
    expect(screen.getByText("Stopped")).toBeVisible();
    rendered.rerender(view({ content: "", status: "error" }));
    expect(screen.getByText("Failed")).toBeVisible();
  });

  it("keeps the words of a stopped reply on screen", () => {
    render(view({ content: "Half of the answer", status: "stopped" }));
    expect(screen.getByText("Half of the answer")).toBeVisible();
    expect(screen.getByText("Stopped")).toBeVisible();
  });
});

describe("files sent with a question", () => {
  it("can remove every file from an edit without changing the original", async () => {
    const onEdit = vi.fn(async () => false);
    render(
      view(
        { role: "user", content: "Read it", attachments: [file()] },
        { onEdit },
      ),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Remove notes.txt" }),
    );
    const editor = screen.getByRole("textbox", { name: "Edit and resend" });
    expect(editor).toHaveFocus();
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(onEdit).toHaveBeenCalledWith("m", "Read it", []);
    await userEvent.clear(editor);
    expect(screen.getByRole("button", { name: "Resend" })).toBeDisabled();
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(
      screen.getByRole("list", { name: "Attached files" }),
    ).toHaveTextContent("notes.txt");
  });

  it("shows the files a question was sent with, and that turn's outcome", async () => {
    // The outcome's words are in a tooltip, which the page's provider serves.
    render(
      view({
        role: "user",
        content: "Compare these",
        attachments: [file(), scanned],
      }),
      { wrapper: TooltipProvider },
    );
    const files = screen.getByRole("list", { name: "Attached files" });
    const [notes, scan] = within(files).getAllByRole("listitem");
    expect(notes).toHaveTextContent("notes.txt");
    expect(await chipNotes(scan)).toContain(
      "Page 3 has no text and wasn't read. Choose a model that can read images to read it.",
    );
    expect(document.body).not.toHaveTextContent("assistant_");
  });

  it("shows a question sent with files alone without an empty bubble", () => {
    render(view({ role: "user", content: "", attachments: [file()] }));
    expect(screen.getByRole("list", { name: "Attached files" })).toBeVisible();
    expect(markdown).not.toHaveBeenCalled();
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
  });

  it("keeps the files of an edited question and says so", async () => {
    const onEdit = vi.fn(async () => true);
    render(
      view(
        { role: "user", content: "Read it", attachments: [file()] },
        { onEdit },
      ),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    const files = screen.getByRole("list", { name: "Attached files" });
    expect(files).toHaveTextContent("notes.txt");
    expect(screen.getByText(en.assistant.editKeepsFiles)).toBeVisible();
    await userEvent.clear(
      screen.getByRole("textbox", { name: "Edit and resend" }),
    );
    // The files alone can be sent again.
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(onEdit).toHaveBeenCalledWith("m", "", [file()]);
  });

  it("says an edit leaves out the files that can't be read", async () => {
    render(
      view({
        role: "user",
        content: "Read both",
        attachments: [file(), locked],
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    expect(
      screen.getByText(
        `${en.assistant.editKeepsFiles} ${en.assistant.editDropsFiles}`,
      ),
    ).toBeVisible();
    expect(
      screen.getByRole("list", { name: "Attached files" }),
    ).not.toHaveTextContent("locked.pdf");
    expect(screen.getByRole("button", { name: "Resend" })).toBeEnabled();
  });

  it("can resend only the text when none of its files can be read", async () => {
    const onEdit = vi.fn(async () => true);
    render(
      view(
        { role: "user", content: "Read it", attachments: [locked] },
        { onEdit },
      ),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    expect(screen.getByText(en.assistant.editDropsFiles)).toBeVisible();
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(onEdit).toHaveBeenCalledWith("m", "Read it", []);
  });
});

describe("a reply's sources", () => {
  const source = (id: string, title: string, url: string): AssistantSource => ({
    id,
    title,
    url,
    excerpt: "",
  });
  const found = [
    source("s1", "Pi release notes", "https://pi.dev/releases"),
    source("s2", "Ofox docs", "https://docs.example.com/ofox"),
  ];
  /** An answer that links to each of `urls`, as a model cites a source. */
  const citing = (...urls: string[]) =>
    `Answer ${urls.map((url, index) => `([${index + 1}](${url}))`).join(" ")}`;
  const titles = (list: HTMLElement) =>
    within(list)
      .getAllByRole("button")
      .map((link) => link.textContent);

  it("lists the pages the answer cites, numbered with their site, under the reply", () => {
    render(
      view({
        content: citing(...found.map((item) => item.url)),
        status: "completed",
        sources: found,
      }),
    );
    const list = screen.getByRole("list", { name: "Sources" });
    // A styled list keeps its list role for screen readers.
    expect(list).toHaveAttribute("role", "list");
    expect(titles(list)).toEqual([
      "1. Pi release notes pi.dev",
      "2. Ofox docs docs.example.com",
    ]);
    expect(
      within(list).getByRole("button", { name: "1. Pi release notes pi.dev" }),
    ).toHaveAttribute("title", "https://pi.dev/releases");
    expect(screen.queryByRole("button", { expanded: false })).toBeNull();
  });

  it("shows the cited sources first and keeps the rest behind a disclosure", async () => {
    const other = source("s3", "Other page", "https://other.example/x");
    render(
      view({
        // Cited with tracking and a fragment, as a model may link a source.
        content:
          "See [docs](https://docs.example.com/ofox?utm_source=openai#a).",
        sources: [...found, other],
      }),
    );
    expect(titles(screen.getByRole("list", { name: "Sources" }))).toEqual([
      "1. Ofox docs docs.example.com",
    ]);
    const more = screen.getByRole("button", { name: "2 more sources" });
    expect(more).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByRole("list", { name: "2 more sources" })).toBeNull();
    expect(screen.queryByText(/Pi release notes/)).toBeNull();
    await userEvent.click(more);
    expect(more).toHaveAttribute("aria-expanded", "true");
    const rest = screen.getByRole("list", { name: "2 more sources" });
    expect(more).toHaveAttribute("aria-controls", rest.id);
    expect(titles(rest)).toEqual([
      "2. Pi release notes pi.dev",
      "3. Other page other.example",
    ]);
    await userEvent.click(more);
    expect(more).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText(/Pi release notes/)).toBeNull();
  });

  it.each([
    ["，", "见 https://docs.example.com/ofox，然后"],
    ["）", "见（https://docs.example.com/ofox）。"],
    ["。", "详见https://docs.example.com/ofox。"],
    ["_。", "见 _https://docs.example.com/ofox_。后"],
    ["**。", "见 **https://docs.example.com/ofox**。后"],
  ])("counts a page cited in running text before %s as cited", (_, content) => {
    // The saved source carries tracking, so only the cleaned URLs match.
    const tracked = source(
      "s2",
      "Ofox docs",
      "https://docs.example.com/ofox?utm_source=openai",
    );
    render(view({ content, sources: [found[0], tracked] }));
    expect(titles(screen.getByRole("list", { name: "Sources" }))).toEqual([
      "1. Ofox docs docs.example.com",
    ]);
    expect(
      screen.getByRole("button", { name: "1 more source" }),
    ).toBeInTheDocument();
  });

  it("offers every source behind the disclosure when the answer cites none, by keyboard too", async () => {
    render(view({ content: "Answer", sources: found }));
    expect(screen.queryByRole("list", { name: "Sources" })).toBeNull();
    const show = screen.getByRole("button", { name: "Show 2 sources" });
    show.focus();
    await userEvent.keyboard("{Enter}");
    expect(show).toHaveAttribute("aria-expanded", "true");
    expect(
      titles(screen.getByRole("list", { name: "Show 2 sources" })),
    ).toEqual(["1. Pi release notes pi.dev", "2. Ofox docs docs.example.com"]);
  });

  it("names a site without its www, and wraps a long title inside the reply", () => {
    const long = "https://www.example.org/" + "a".repeat(300);
    render(
      view({ content: citing(long), sources: [source("s1", long, long)] }),
    );
    const link = screen.getByRole("button", { name: `1. ${long} example.org` });
    expect(within(link).getByText(`1. ${long}`)).toHaveClass(
      "min-w-0",
      "break-words",
    );
  });

  it("lines a source's icon and site up with the first line of its wrapped title", () => {
    const long = "https://www.example.org/" + "a".repeat(300);
    render(
      view({ content: citing(long), sources: [source("s1", long, long)] }),
    );
    const link = screen.getByRole("button", { name: `1. ${long} example.org` });
    expect(link).toHaveClass("items-start");
    expect(link).not.toHaveClass("items-center");
    // Half the gap between the line (1.4 x 12px) and the 12px icon.
    expect(link.querySelector("svg")).toHaveClass("mt-0.5");
  });

  it("opens a source in the browser", async () => {
    render(
      view({
        content: citing("https://docs.example.com/ofox"),
        sources: found,
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "1. Ofox docs docs.example.com" }),
    );
    expect(external.openExternal).toHaveBeenCalledWith(
      "https://docs.example.com/ofox",
    );
  });

  it("says so when a source can't be opened", async () => {
    external.openExternal.mockRejectedValueOnce(new Error("denied"));
    render(
      view({ content: citing("https://pi.dev/releases"), sources: found }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "1. Pi release notes pi.dev" }),
    );
    await vi.waitFor(() =>
      expect(external.toastError).toHaveBeenCalledWith(
        "Couldn't open the link",
      ),
    );
  });

  it("shows only HTTPS pages", () => {
    const urls = [
      "http://plain.example.com/",
      "javascript:alert(1)",
      "https://secure.example.com/",
    ];
    render(
      view({
        content: citing(...urls),
        sources: [
          source("s1", "Plain", urls[0]),
          source("s2", "Script", urls[1]),
          source("s3", "Secure", urls[2]),
        ],
      }),
    );
    expect(titles(screen.getByRole("list", { name: "Sources" }))).toEqual([
      "1. Secure secure.example.com",
    ]);
    expect(screen.queryByRole("button", { expanded: false })).toBeNull();
  });

  it("shows nothing for a reply without sources", () => {
    render(view({ content: "Answer", sources: [] }));
    expect(screen.queryByText("Sources")).toBeNull();
  });

  it("offers the sources found so far while the reply streams", () => {
    render(view({ status: "streaming", sources: found }));
    expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
    expect(
      screen.getByRole("button", { name: "Show 2 sources" }),
    ).toBeVisible();
  });
});

describe("what a streaming reply is doing", () => {
  const streaming = (
    content: string,
    activity: "searching" | "readingPage" | "readingFile" | null,
    status: AssistantMessage["status"] = "streaming",
  ) => (
    <AssistantMessageView
      message={{ id: "m", role: "assistant", content, createdAt: 1, status }}
      activity={activity}
      disabled
      onEdit={async () => false}
      onRegenerate={() => undefined}
    />
  );

  it.each([
    ["searching", "Searching the web…"],
    ["readingPage", "Reading a page…"],
    ["readingFile", "Reading a file…"],
  ] as const)("says %s instead of thinking", (activity, text) => {
    render(streaming("", activity));
    expect(screen.getByRole("status")).toHaveTextContent(text);
    expect(screen.queryByText("Thinking…")).toBeNull();
  });

  it("says it below the words already written", () => {
    render(streaming("Let me check.", "searching"));
    expect(screen.getByText("Let me check.")).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Searching the web…");
  });

  it("says nothing once the reply is finished", () => {
    render(streaming("Answer", "searching", "completed"));
    expect(screen.queryByText("Searching the web…")).toBeNull();
  });
});

describe("a message's layout", () => {
  it("shows a question as a bubble on the right, with its files under it", () => {
    render(
      view({ role: "user", content: "Polish this", attachments: [file()] }),
    );
    const article = screen.getByRole("article", { name: "You" });
    expect(article).toHaveClass("items-end");
    const bubble = screen.getByText("Polish this").parentElement!;
    expect(bubble).toHaveClass("bg-pill", "max-w-[80%]");
    const files = screen.getByRole("list", { name: "Attached files" });
    expect(files).toHaveClass("max-w-[80%]", "justify-end");
    expect(
      bubble.compareDocumentPosition(files) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("gives a reply the full width, with no bubble", () => {
    render(view({ content: "A long answer", status: "completed" }));
    const article = screen.getByRole("article", { name: "Assistant" });
    expect(article).not.toHaveClass("items-end");
    const body = screen.getByText("A long answer").parentElement!;
    expect(body).not.toHaveClass("bg-pill");
    expect(body).not.toHaveClass("max-w-[80%]");
  });

  it.each([
    ["user", "You"],
    ["assistant", "Assistant"],
  ] as const)(
    "names a %s message for assistive tech only, with no role row",
    (role, name) => {
      render(view({ role, content: "Words", status: "completed" }));
      const article = screen.getByRole("article", { name });
      const heading = within(article).getByRole("heading", { name });
      expect(heading).toHaveClass("sr-only");
      // The name is nowhere else on the message, and no avatar is drawn.
      expect(within(article).getAllByText(name)).toEqual([heading]);
      for (const icon of article.querySelectorAll("svg"))
        expect(icon.closest("button")).not.toBeNull();
    },
  );
});

describe("a message's actions", () => {
  const toolbarOf = (name: string) =>
    screen.getByRole("button", { name }).parentElement!;

  it("keeps the newest reply's actions in view", () => {
    render(view({ content: "Newest", status: "completed" }, {}, false, true));
    expect(toolbarOf("Copy")).not.toHaveClass("opacity-0");
  });

  it("shows another message's actions while the pointer or focus is on it", async () => {
    render(view({ content: "Older", status: "completed" }));
    const toolbar = toolbarOf("Copy");
    expect(toolbar).toHaveClass(
      "opacity-0",
      "group-hover:opacity-100",
      "focus-within:opacity-100",
    );
    expect(screen.getByRole("article", { name: "Assistant" })).toHaveClass(
      "group",
    );
    // Hidden only by opacity: the keyboard still reaches them.
    await userEvent.tab();
    expect(screen.getByRole("button", { name: "Copy" })).toHaveFocus();
    await userEvent.tab();
    expect(screen.getByRole("button", { name: "Regenerate" })).toHaveFocus();
  });

  it("marks a stopped or failed reply with a badge in the actions' row", () => {
    const rendered = render(view({ content: "Part", status: "stopped" }));
    const badge = screen.getByText("Stopped");
    const row = badge.parentElement!;
    expect(row).toContainElement(screen.getByRole("button", { name: "Copy" }));
    // Said whether or not the actions are in view.
    expect(toolbarOf("Copy")).not.toContainElement(badge);
    expect(badge).toHaveClass("rounded-full");
    rendered.rerender(view({ content: "Part", status: "error" }));
    expect(screen.getByText("Failed")).toHaveClass("text-danger");
    expect(screen.getByText("Failed").parentElement).toContainElement(
      screen.getByRole("button", { name: "Copy" }),
    );
  });

  it("has no actions row while the reply is still being written", () => {
    render(view({ content: "So far", status: "streaming" }, {}, false, true));
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
  });
});

describe("drawing a message again", () => {
  function Transcript({
    message = saved,
    onEdit,
    onRegenerate,
    tick,
  }: {
    message?: AssistantMessage;
    onEdit: () => Promise<boolean>;
    onRegenerate: () => void;
    tick: number;
  }) {
    return (
      <>
        <span data-tick={tick} />
        <AssistantMessageView
          message={message}
          disabled={false}
          onEdit={onEdit}
          onRegenerate={onRegenerate}
        />
      </>
    );
  }
  const saved: AssistantMessage = {
    id: "saved",
    role: "assistant",
    content: "Saved answer",
    createdAt: 1,
  };

  it("leaves a message that did not change alone while the page draws again", () => {
    const onEdit = async () => false;
    const onRegenerate = () => undefined;
    const rendered = render(
      <Transcript onEdit={onEdit} onRegenerate={onRegenerate} tick={1} />,
    );
    expect(copyButtonRenders).toHaveBeenCalledTimes(1);
    for (let tick = 2; tick < 12; tick++)
      rendered.rerender(
        <Transcript onEdit={onEdit} onRegenerate={onRegenerate} tick={tick} />,
      );
    expect(copyButtonRenders).toHaveBeenCalledTimes(1);
  });

  it("draws it again once its message or a callback changes", () => {
    const onEdit = async () => false;
    const onRegenerate = () => undefined;
    const rendered = render(
      <Transcript onEdit={onEdit} onRegenerate={onRegenerate} tick={1} />,
    );
    rendered.rerender(
      <Transcript onEdit={onEdit} onRegenerate={() => undefined} tick={1} />,
    );
    expect(copyButtonRenders).toHaveBeenCalledTimes(2);
    rendered.rerender(
      <Transcript
        message={{ ...saved, content: "Changed answer" }}
        onEdit={onEdit}
        onRegenerate={onRegenerate}
        tick={1}
      />,
    );
    expect(copyButtonRenders).toHaveBeenCalledTimes(3);
    expect(copyButtonRenders).toHaveBeenLastCalledWith("Changed answer");
  });

  it("names the message in each callback, so one function serves every message", async () => {
    const onEdit = vi.fn(async () => true);
    const onRegenerate = vi.fn();
    render(
      <>
        <AssistantMessageView
          message={{
            id: "q1",
            role: "user",
            content: "First",
            createdAt: 1,
            nativeEntryId: "e1",
          }}
          disabled={false}
          onEdit={onEdit}
          onRegenerate={onRegenerate}
        />
        <AssistantMessageView
          message={{
            id: "a1",
            role: "assistant",
            content: "Answer",
            createdAt: 2,
            nativeEntryId: "e2",
            runId: "run",
          }}
          disabled={false}
          onEdit={onEdit}
          onRegenerate={onRegenerate}
        />
      </>,
    );
    await userEvent.click(screen.getByRole("button", { name: "Regenerate" }));
    expect(onRegenerate).toHaveBeenCalledWith("a1");
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(onEdit).toHaveBeenCalledWith("q1", "First", []);
  });
});

describe("a reply's thinking", () => {
  const reply = (
    over: Partial<AssistantMessage> = {},
    thinking: { doneMs: number; since: number | null } | null = null,
  ) => (
    <AssistantMessageView
      message={{
        id: "m",
        role: "assistant",
        content: "",
        createdAt: 1,
        ...over,
      }}
      thinking={thinking && { runId: "run", ...thinking }}
      disabled
      onEdit={async () => false}
      onRegenerate={() => undefined}
    />
  );

  it("says how long a saved reply thought, with its reasoning behind it", () => {
    render(
      reply({
        content: "Answer",
        reasoning: "Because",
        reasoningMs: 7600,
        status: "completed",
      }),
    );
    const caption = screen.getByText("Thought for 8 seconds");
    expect(caption.closest("details")).toHaveTextContent("Because");
    expect(screen.queryByText("Reasoning")).toBeNull();
  });

  it("says nothing of thinking for a reply that has no reasoning", () => {
    render(reply({ content: "Answer", status: "completed" }));
    expect(screen.queryByText(/Thought for|Thinking for|Reasoning/)).toBeNull();
  });

  it("says nothing of thinking on a question", () => {
    render(reply({ role: "user", content: "Hello", status: "completed" }));
    expect(screen.queryByText(/Thought for|Thinking for/)).toBeNull();
  });

  it("shows the live caption in place of the plain Thinking… line while it thinks", () => {
    render(reply({ status: "streaming" }, { doneMs: 0, since: Date.now() }));
    expect(screen.getByText("Thinking for 1 second")).toBeVisible();
    // The plain line would say the same twice.
    expect(screen.queryByText("Thinking…", { selector: "p" })).toBeNull();
  });

  it("goes back to the plain Thinking… line once the thinking is over and nothing was said", () => {
    render(reply({ status: "streaming" }, { doneMs: 3000, since: null }));
    expect(screen.getByText("Thought for 3 seconds")).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
  });

  it("keeps the activity line while a tool runs, even though it was thinking before", () => {
    render(
      <AssistantMessageView
        message={{
          id: "m",
          role: "assistant",
          content: "",
          createdAt: 1,
          status: "streaming",
        }}
        activity="searching"
        thinking={{ runId: "run", doneMs: 2000, since: null }}
        disabled
        onEdit={async () => false}
        onRegenerate={() => undefined}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("Searching the web…");
  });
});
