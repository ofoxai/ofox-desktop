import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { InteractionScope } from "@/components/ui/interaction-scope";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

it("releases portaled dialog focus and overlay when its retained page is suspended", () => {
  const content = (active: boolean) => (
    <InteractionScope active={active}>
      <Dialog open>
        <DialogContent>
          <DialogTitle>Fixture</DialogTitle>
          <button>inside</button>
        </DialogContent>
      </Dialog>
    </InteractionScope>
  );
  const { rerender } = render(content(true));
  expect(screen.getByRole("dialog")).toBeVisible();
  rerender(content(false));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.body.style.pointerEvents).not.toBe("none");
});

it("suspends a portaled menu while keeping ordinary page state mounted", () => {
  const content = (active: boolean) => (
    <InteractionScope active={active}>
      <Popover>
        <PopoverTrigger>open</PopoverTrigger>
        <PopoverContent>menu contents</PopoverContent>
      </Popover>
    </InteractionScope>
  );
  const { rerender } = render(content(true));
  fireEvent.click(screen.getByText("open"));
  expect(screen.getByText("menu contents")).toBeVisible();
  rerender(content(false));
  expect(screen.queryByText("menu contents")).toBeNull();
});
