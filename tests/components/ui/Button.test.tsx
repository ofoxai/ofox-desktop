import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Button, buttonVariants } from "@/components/ui/button";

const classesOf = (
  variant: Parameters<typeof buttonVariants>[0] extends infer P
    ? P extends { variant?: infer V }
      ? V
      : never
    : never,
) => buttonVariants({ variant });

describe("Button", () => {
  it("draws the primary action in brand orange with on-brand text", () => {
    render(<Button variant="primary">Save</Button>);
    const button = screen.getByRole("button", { name: "Save" });
    expect(button).toHaveClass("bg-brand", "text-brand-on", "font-medium");
  });

  it("defaults to the primary look", () => {
    render(<Button>Go</Button>);
    expect(screen.getByRole("button", { name: "Go" })).toHaveClass("bg-brand");
  });

  it("draws secondary as a 27px card-coloured button with a hairline border", () => {
    render(<Button variant="secondary">Manage</Button>);
    const button = screen.getByRole("button", { name: "Manage" });
    expect(button).toHaveClass("h-[27px]", "bg-card", "border", "border-line");
  });

  it("keeps the old variant names as aliases of the new ones", () => {
    expect(classesOf("default")).toBe(classesOf("primary"));
    expect(classesOf("outline")).toBe(classesOf("secondary"));
    expect(classesOf("destructive")).toBe(classesOf("danger"));
  });

  it("fills danger with the danger colour", () => {
    render(<Button variant="danger">Delete</Button>);
    expect(screen.getByRole("button", { name: "Delete" })).toHaveClass(
      "bg-danger",
    );
  });

  it("turns dangerGhost red on danger-soft only on hover", () => {
    render(<Button variant="dangerGhost">Remove</Button>);
    const button = screen.getByRole("button", { name: "Remove" });
    expect(button).toHaveClass(
      "text-ink-ctl",
      "hover:bg-danger-soft",
      "hover:text-danger",
    );
  });

  it("draws ghost as quiet 12px control text that gains a pill on hover", () => {
    render(<Button variant="ghost">More</Button>);
    const button = screen.getByRole("button", { name: "More" });
    expect(button).toHaveClass("text-ctl", "text-ink-ctl", "hover:bg-pill");
  });

  it("sizes pill, icon and copy buttons whatever size is passed", () => {
    render(
      <>
        <Button variant="pill">Upgrade</Button>
        <Button variant="icon" aria-label="Refresh" size="lg" />
        <Button variant="copy" aria-label="Copy" />
      </>,
    );
    expect(screen.getByRole("button", { name: "Upgrade" })).toHaveClass(
      "h-6",
      "bg-pill",
      "text-sub",
      "font-medium",
    );
    const icon = screen.getByRole("button", { name: "Refresh" });
    expect(icon).toHaveClass("h-[26px]", "w-[26px]", "rounded-ctl");
    expect(icon).not.toHaveClass("h-8");
    expect(screen.getByRole("button", { name: "Copy" })).toHaveClass(
      "h-[22px]",
      "w-[22px]",
      "rounded-[5px]",
    );
  });

  it("has 27 / 24 / 32 / 26px sizes", () => {
    expect(buttonVariants({ size: "default" })).toContain("h-[27px]");
    expect(buttonVariants({ size: "sm" })).toContain("h-6");
    expect(buttonVariants({ size: "lg" })).toContain("h-8");
    expect(buttonVariants({ size: "icon" })).toContain("h-[26px] w-[26px]");
  });

  it("gives a little when pressed", () => {
    render(<Button>Press</Button>);
    expect(screen.getByRole("button", { name: "Press" })).toHaveClass(
      "active:scale-[.96]",
    );
  });

  it("has no blue in any variant", () => {
    const variants = [
      "primary",
      "secondary",
      "ghost",
      "danger",
      "dangerGhost",
      "pill",
      "icon",
      "copy",
      "link",
      "default",
      "outline",
      "destructive",
      "mcp",
    ] as const;
    for (const variant of variants) {
      expect(classesOf(variant), variant).not.toMatch(/blue|sky|indigo/);
    }
  });
});
