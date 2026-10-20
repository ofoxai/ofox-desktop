import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { Input } from "@/components/ui/input";

function Menu() {
  return (
    <DropdownMenu open modal={false}>
      <DropdownMenuTrigger>more</DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuItem>Rename</DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const FROSTED_POP = [
  "bg-pop-bg",
  "backdrop-blur-xl",
  "rounded-pop",
  "p-1",
  "shadow-pop",
  "animate-pop-in",
];

describe("popups", () => {
  it("draws a dropdown menu as a frosted pop with 7px, 12.5px items", () => {
    render(<Menu />);
    expect(screen.getByRole("menu")).toHaveClass(...FROSTED_POP);
    expect(screen.getByRole("menuitem", { name: "Rename" })).toHaveClass(
      "rounded-ctl",
      "text-nav",
      "focus:bg-pill",
    );
  });

  it("portals a dropdown menu opened in a dialog into that dialog", () => {
    render(
      <Dialog open>
        <DialogContent aria-describedby={undefined}>
          <DialogTitle>Tool</DialogTitle>
          <Menu />
        </DialogContent>
      </Dialog>,
    );
    expect(screen.getByRole("dialog", { name: "Tool" })).toContainElement(
      screen.getByRole("menu"),
    );
  });

  it("draws a popover as the same frosted pop", () => {
    render(
      <Popover open>
        <PopoverTrigger>pick</PopoverTrigger>
        <PopoverContent>list</PopoverContent>
      </Popover>,
    );
    expect(screen.getByText("list")).toHaveClass(...FROSTED_POP);
  });

  it("draws a tooltip as an ink chip, not in the brand colour", () => {
    render(
      <TooltipProvider>
        <Tooltip open>
          <TooltipTrigger>hover</TooltipTrigger>
          <TooltipContent>Copied</TooltipContent>
        </Tooltip>
      </TooltipProvider>,
    );
    const tip = screen.getAllByText("Copied")[0];
    expect(tip).toHaveClass("bg-ink", "text-canvas", "text-sub");
    expect(tip.className).not.toMatch(/bg-primary/);
  });

  it("draws an input 26px high with a hairline that turns brand on focus", () => {
    render(<Input aria-label="Name" />);
    const input = screen.getByRole("textbox", { name: "Name" });
    expect(input).toHaveClass(
      "h-[26px]",
      "rounded-ctl",
      "border-line",
      "focus:border-brand",
    );
    expect(input.className).not.toMatch(/blue/);
  });
});
