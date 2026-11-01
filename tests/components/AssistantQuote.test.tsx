import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AssistantQuote } from "@/components/assistant/AssistantQuote";
import { insertAssistantQuote } from "@/utils/assistantQuote";

describe("assistant selected text quote", () => {
  it("inserts multiline quotation at the saved composer selection without losing either side", () => {
    expect(
      insertAssistantQuote("before selected after", 7, 15, "first\nsecond"),
    ).toEqual({
      text: "before \n> first\n> second\n\n after",
      cursor: 26,
    });
    expect(insertAssistantQuote("", 0, 0, " hello ").text).toBe("> hello\n\n");
  });
  it("offers only a selection contained in the assistant text and preserves selection on press", () => {
    const quote = vi.fn();
    render(
      <>
        <p>outside</p>
        <AssistantQuote onQuote={quote}>
          <p>answer text</p>
        </AssistantQuote>
      </>,
    );
    const text = screen.getByText("answer text");
    const range = document.createRange();
    range.selectNodeContents(text);
    range.getBoundingClientRect = () => ({ left: 10, bottom: 20 }) as DOMRect;
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(range);
    fireEvent.pointerUp(text);
    const button = screen.getByRole("button", {
      name: /quote|assistant.quote/i,
    });
    fireEvent.pointerDown(button);
    fireEvent.click(button);
    expect(quote).toHaveBeenCalledWith("answer text");
    expect(
      screen.queryByRole("button", { name: /quote|assistant.quote/i }),
    ).not.toBeInTheDocument();
    range.selectNodeContents(screen.getByText("outside"));
    selection.removeAllRanges();
    selection.addRange(range);
    fireEvent.pointerUp(text);
    expect(
      screen.queryByRole("button", { name: /quote|assistant.quote/i }),
    ).not.toBeInTheDocument();
    range.selectNodeContents(text);
    selection.removeAllRanges();
    selection.addRange(range);
    fireEvent.pointerUp(text);
    expect(
      screen.getByRole("button", { name: /quote|assistant.quote/i }),
    ).toBeInTheDocument();
    expect(fireEvent.keyDown(document, { key: "Escape" })).toBe(false);
    expect(
      screen.queryByRole("button", { name: /quote|assistant.quote/i }),
    ).not.toBeInTheDocument();
    selection.removeAllRanges();
  });
});
