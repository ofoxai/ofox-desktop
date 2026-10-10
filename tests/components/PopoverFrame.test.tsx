import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import PopoverFrame from "@/components/tray/PopoverFrame";

const platform = vi.hoisted(() => ({ mac: true }));

vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => platform.mac,
}));

function frameOf(content: HTMLElement): HTMLElement {
  return content.parentElement as HTMLElement;
}

describe("tray PopoverFrame", () => {
  it("floats a translucent card inside the transparent macOS window", () => {
    platform.mac = true;
    render(
      <PopoverFrame>
        <p>content</p>
      </PopoverFrame>,
    );
    const card = frameOf(screen.getByText("content"));
    expect(card).toHaveClass("rounded-xl");
    expect(card.parentElement).toHaveClass("p-4");
  });

  it("fills the window with an opaque panel elsewhere", () => {
    platform.mac = false;
    render(
      <PopoverFrame>
        <p>content</p>
      </PopoverFrame>,
    );
    const panel = frameOf(screen.getByText("content"));
    expect(panel).toHaveClass("h-screen", "bg-popover");
    expect(panel).not.toHaveClass("rounded-xl");
    expect(panel.style.backgroundColor).toBe("");
    expect(panel.parentElement?.className ?? "").not.toContain("p-4");
  });
});
