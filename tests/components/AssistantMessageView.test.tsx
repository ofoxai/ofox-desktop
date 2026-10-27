import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantMessageView } from "@/components/assistant/AssistantMessageView";
import type {
  AssistantAttachment,
  AssistantMessage,
  AssistantSource,
} from "@/lib/api/assistant";

const markdown = vi.hoisted(() => vi.fn());
const external = vi.hoisted(() => ({
  openExternal: vi.fn(),
  toastError: vi.fn(),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: external.openExternal },
}));
vi.mock("sonner", () => ({ toast: { error: external.toastError } }));
vi.mock("@/components/sessions/SessionMarkdown", () => ({
  SessionMarkdown: ({ content }: { content: string }) => {
    markdown(content);
    return <p>{content}</p>;
  },
}));

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  external.openExternal.mockReset().mockResolvedValue(undefined);
  external.toastError.mockReset();
});

function view(
  message: Partial<AssistantMessage>,
  handlers: {
    onEdit?: (text: string) => Promise<boolean>;
    onRegenerate?: () => void;
    onRetry?: () => void;
  } = {},
  disabled = false,
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
      onEdit={handlers.onEdit ?? (async () => false)}
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
    expect(onEdit).toHaveBeenCalledWith("Better question");
    expect(
      screen.queryByRole("textbox", { name: "Edit and resend" }),
    ).not.toBeInTheDocument();
  });

  it("keeps a replaced version readable behind a disclosure, without actions", () => {
    render(view({ role: "user", content: "Old question", superseded: true }));
    expect(screen.getByText("View previous version")).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Edit and resend" }),
    ).not.toBeInTheDocument();
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

  it("offers to send a failed question again when the page allows it", async () => {
    const onRetry = vi.fn();
    const failed: Partial<AssistantMessage> = {
      role: "user",
      content: "Lost",
      nativeEntryId: null,
    };
    const rendered = render(view(failed, { onRetry }, true));
    expect(screen.getByRole("button", { name: "Send again" })).toBeDisabled();
    rendered.rerender(view(failed, { onRetry }));
    await userEvent.click(screen.getByRole("button", { name: "Send again" }));
    expect(onRetry).toHaveBeenCalledOnce();
    rendered.rerender(view(failed));
    expect(screen.queryByRole("button", { name: "Send again" })).toBeNull();
  });

  it("marks a stopped or failed reply", () => {
    const rendered = render(view({ content: "Part", status: "stopped" }));
    expect(screen.getByText("Stopped")).toBeVisible();
    rendered.rerender(view({ content: "", status: "error" }));
    expect(screen.getByText("Failed")).toBeVisible();
  });
});

describe("files sent with a question", () => {
  it("shows the files a question was sent with, and that turn's outcome", () => {
    render(
      view({
        role: "user",
        content: "Compare these",
        attachments: [file(), scanned],
      }),
    );
    const files = screen.getByRole("list", { name: "Attached files" });
    const [notes, scan] = within(files).getAllByRole("listitem");
    expect(notes).toHaveTextContent("notes.txt");
    expect(scan).toHaveTextContent(
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
    expect(
      screen.getByText("These files are sent again with your edit."),
    ).toBeVisible();
    await userEvent.clear(
      screen.getByRole("textbox", { name: "Edit and resend" }),
    );
    // The files alone can be sent again.
    await userEvent.click(screen.getByRole("button", { name: "Resend" }));
    expect(onEdit).toHaveBeenCalledWith("");
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
        "These files are sent again with your edit. Files that can't be read are left out.",
      ),
    ).toBeVisible();
    expect(
      screen.getByRole("list", { name: "Attached files" }),
    ).toHaveTextContent("This PDF is password-protected.");
    expect(screen.getByRole("button", { name: "Resend" })).toBeEnabled();
  });

  it("cannot resend an edit when none of its files can be read", async () => {
    render(view({ role: "user", content: "Read it", attachments: [locked] }));
    await userEvent.click(
      screen.getByRole("button", { name: "Edit and resend" }),
    );
    expect(
      screen.getByText(
        "None of this message's files can be read, so it can't be resent. Send a new message instead.",
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Resend" })).toBeDisabled();
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

  it("lists the pages its searches found, numbered with their site, under the reply", () => {
    render(view({ content: "Answer", status: "completed", sources: found }));
    const list = screen.getByRole("list", { name: "Sources" });
    // A styled list keeps its list role for screen readers.
    expect(list).toHaveAttribute("role", "list");
    expect(
      within(list)
        .getAllByRole("button")
        .map((link) => link.textContent),
    ).toEqual(["1. Pi release notes pi.dev", "2. Ofox docs docs.example.com"]);
    expect(
      within(list).getByRole("button", { name: "1. Pi release notes pi.dev" }),
    ).toHaveAttribute("title", "https://pi.dev/releases");
  });

  it("names a site without its www, and wraps a long title inside the reply", () => {
    const long = "https://www.example.org/" + "a".repeat(300);
    render(view({ content: "Answer", sources: [source("s1", long, long)] }));
    const link = screen.getByRole("button", { name: `1. ${long} example.org` });
    expect(within(link).getByText(`1. ${long}`)).toHaveClass(
      "min-w-0",
      "break-words",
    );
  });

  it("opens a source in the browser", async () => {
    render(view({ content: "Answer", sources: found }));
    await userEvent.click(
      screen.getByRole("button", { name: "2. Ofox docs docs.example.com" }),
    );
    expect(external.openExternal).toHaveBeenCalledWith(
      "https://docs.example.com/ofox",
    );
  });

  it("says so when a source can't be opened", async () => {
    external.openExternal.mockRejectedValueOnce(new Error("denied"));
    render(view({ content: "Answer", sources: found }));
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
    render(
      view({
        content: "Answer",
        sources: [
          source("s1", "Plain", "http://plain.example.com/"),
          source("s2", "Script", "javascript:alert(1)"),
          source("s3", "Secure", "https://secure.example.com/"),
        ],
      }),
    );
    expect(
      within(screen.getByRole("list", { name: "Sources" }))
        .getAllByRole("button")
        .map((link) => link.textContent),
    ).toEqual(["1. Secure secure.example.com"]);
  });

  it("shows nothing for a reply without sources", () => {
    render(view({ content: "Answer", sources: [] }));
    expect(screen.queryByText("Sources")).toBeNull();
  });

  it("shows the sources found so far while the reply streams", () => {
    render(view({ status: "streaming", sources: found }));
    expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
    expect(
      screen.getByRole("button", { name: "1. Pi release notes pi.dev" }),
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
