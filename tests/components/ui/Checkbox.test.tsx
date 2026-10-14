import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Checkbox } from "@/components/ui/checkbox";

describe("Checkbox", () => {
  it("is a 14px box with a 4px radius that turns brand when checked", () => {
    render(<Checkbox aria-label="Pick" />);
    const box = screen.getByRole("checkbox", { name: "Pick" });
    expect(box).toHaveClass(
      "h-3.5",
      "w-3.5",
      "rounded-[4px]",
      "data-[state=checked]:bg-brand",
    );
  });

  it("shows a tick when checked", () => {
    render(<Checkbox aria-label="Pick" checked />);
    const box = screen.getByRole("checkbox", { name: "Pick" });
    expect(box).toHaveAttribute("aria-checked", "true");
    expect(box.querySelector("svg.lucide-check")).not.toBeNull();
    expect(box.querySelector("svg.lucide-minus")).toBeNull();
  });

  it("reads as mixed and shows a dash when indeterminate", () => {
    render(<Checkbox aria-label="All" checked="indeterminate" />);
    const box = screen.getByRole("checkbox", { name: "All" });
    expect(box).toHaveAttribute("aria-checked", "mixed");
    expect(box).toHaveClass("data-[state=indeterminate]:bg-brand");
    expect(box.querySelector("svg.lucide-minus")).not.toBeNull();
    expect(box.querySelector("svg.lucide-check")).toBeNull();
  });

  it("asks to become checked when an indeterminate box is clicked", () => {
    const onCheckedChange = vi.fn();
    render(
      <Checkbox
        aria-label="All"
        checked="indeterminate"
        onCheckedChange={onCheckedChange}
      />,
    );
    fireEvent.click(screen.getByRole("checkbox", { name: "All" }));
    expect(onCheckedChange).toHaveBeenCalledWith(true);
  });

  it("shows nothing inside when unchecked", () => {
    render(<Checkbox aria-label="Pick" checked={false} />);
    const box = screen.getByRole("checkbox", { name: "Pick" });
    expect(box).toHaveAttribute("aria-checked", "false");
    expect(box.querySelector("svg")).toBeNull();
  });
});
