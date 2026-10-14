import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Switch } from "@/components/ui/switch";

describe("Switch", () => {
  it("is brand orange when on, never emerald", () => {
    render(<Switch aria-label="Auto update" checked />);
    const toggle = screen.getByRole("switch", { name: "Auto update" });
    expect(toggle).toHaveAttribute("aria-checked", "true");
    expect(toggle).toHaveClass("data-[state=checked]:bg-brand");
    expect(toggle.className).not.toMatch(/emerald/);
  });
});
