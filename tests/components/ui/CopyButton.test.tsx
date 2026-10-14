import { act, fireEvent, render, screen } from "@testing-library/react";
import i18n from "i18next";
import { toast } from "sonner";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { CopyButton } from "@/components/ui/copy-button";
import zh from "@/i18n/locales/zh.json";

const clipboard = vi.hoisted(() => ({ copyText: vi.fn() }));
vi.mock("@/lib/clipboard", () => clipboard);

beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

afterEach(() => {
  vi.useRealTimers();
});

async function click(button: HTMLElement) {
  await act(async () => {
    fireEvent.click(button);
  });
}

describe("CopyButton", () => {
  it("is a 22px icon button named by its aria-label, or 复制 by default", () => {
    render(
      <>
        <CopyButton text="abc" aria-label="复制会话 ID" />
        <CopyButton text="xyz" />
      </>,
    );
    expect(screen.getByRole("button", { name: "复制会话 ID" })).toHaveClass(
      "h-[22px]",
      "w-[22px]",
      "text-ink-muted",
    );
    expect(screen.getByRole("button", { name: "复制" })).toBeInTheDocument();
  });

  it("copies its text and shows a green check, announced as copied, for 1.4 seconds", async () => {
    vi.useFakeTimers();
    clipboard.copyText.mockResolvedValue(undefined);
    render(<CopyButton text="session-123" aria-label="复制会话 ID" />);
    const button = screen.getByRole("button", { name: "复制会话 ID" });

    await click(button);
    expect(clipboard.copyText).toHaveBeenCalledWith("session-123");
    expect(button).toHaveAttribute("data-copied", "true");
    expect(button).toHaveClass("bg-ok-soft", "text-ok");
    expect(button.querySelector("svg.lucide-check")).not.toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("已复制");
    // the name stays put, so the button can still be found and pressed again
    expect(button).toHaveAccessibleName("复制会话 ID");

    act(() => vi.advanceTimersByTime(1399));
    expect(button).toHaveAttribute("data-copied", "true");
    act(() => vi.advanceTimersByTime(1));
    expect(button).not.toHaveAttribute("data-copied");
    expect(button.querySelector("svg.lucide-copy")).not.toBeNull();
    expect(screen.getByRole("status")).toHaveTextContent("");
  });

  it("says the copy failed and shows no check when the clipboard refuses", async () => {
    const error = vi.spyOn(toast, "error").mockImplementation(() => "");
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    clipboard.copyText.mockRejectedValue(new Error("denied"));
    render(<CopyButton text="abc" />);
    const button = screen.getByRole("button", { name: "复制" });

    await click(button);
    expect(error).toHaveBeenCalledWith("复制失败，请手动复制");
    expect(button).not.toHaveAttribute("data-copied");
    expect(button.querySelector("svg.lucide-check")).toBeNull();
  });
});
