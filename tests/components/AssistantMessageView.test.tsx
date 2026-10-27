import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantMessageView } from "@/components/assistant/AssistantMessageView";
import type { AssistantMessage } from "@/lib/api/assistant";

const markdown = vi.hoisted(() => vi.fn());
vi.mock("@/components/sessions/SessionMarkdown", () => ({
  SessionMarkdown: ({ content }: { content: string }) => {
    markdown(content);
    return <p>{content}</p>;
  },
}));

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
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
