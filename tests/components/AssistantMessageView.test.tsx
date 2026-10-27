import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AssistantMessageView } from "@/components/assistant/AssistantMessageView";

const markdown = vi.hoisted(() => vi.fn());
vi.mock("@/components/sessions/SessionMarkdown", () => ({
  SessionMarkdown: ({ content }: { content: string }) => {
    markdown(content);
    return <p>{content}</p>;
  },
}));

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
        onError={() => undefined}
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
        onError={() => undefined}
      />
    </>
  );
}

describe("assistant transcript rendering", () => {
  it("keeps completed Markdown parsed once while another reply streams", () => {
    const view = render(transcript("First fragment"));
    for (let index = 0; index < 20; index++)
      view.rerender(transcript(`Streaming fragment ${index}`));
    expect(screen.getByText("Saved long document analysis")).toBeVisible();
    expect(screen.getByText("Streaming fragment 19")).toBeVisible();
    expect(
      markdown.mock.calls.filter(
        ([content]) => content === "Saved long document analysis",
      ),
    ).toHaveLength(1);
  });
});
