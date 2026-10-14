import { createRef } from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import {
  Dialog,
  DialogContent,
  DialogTitle,
  useDialogContainer,
} from "@/components/ui/dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";

function PickerInside() {
  return (
    <Popover open>
      <PopoverTrigger>pick</PopoverTrigger>
      <PopoverContent>menu</PopoverContent>
    </Popover>
  );
}

function ContainerProbe() {
  const container = useDialogContainer();
  return (
    <span data-testid="probe">{container ? container.tagName : "none"}</span>
  );
}

describe("Dialog", () => {
  it("lets a popover inside it portal into the dialog, not the body", () => {
    render(
      <Dialog open>
        <DialogContent aria-describedby={undefined}>
          <DialogTitle>Edit</DialogTitle>
          <PickerInside />
        </DialogContent>
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog", { name: "Edit" });
    expect(dialog).toContainElement(screen.getByText("menu"));
  });

  it("provides no container outside a dialog", () => {
    render(<ContainerProbe />);
    expect(screen.getByTestId("probe")).toHaveTextContent("none");
  });

  it("still hands its content element to a forwarded ref", () => {
    const ref = createRef<HTMLDivElement>();
    render(
      <Dialog open>
        <DialogContent ref={ref} aria-describedby={undefined}>
          <DialogTitle>Edit</DialogTitle>
        </DialogContent>
      </Dialog>,
    );
    expect(ref.current).toBe(screen.getByRole("dialog"));
  });

  it("is a 14px-radius card with the dialog shadow over a blurred dim", () => {
    render(
      <Dialog open>
        <DialogContent zIndex="alert" aria-describedby={undefined}>
          <DialogTitle>Edit</DialogTitle>
        </DialogContent>
      </Dialog>,
    );
    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveClass("rounded-dialog", "bg-card", "shadow-dialog");
    expect(dialog).toHaveClass("z-[60]");
    const overlay = document.querySelector(".backdrop-blur-\\[8px\\]");
    expect(overlay).not.toBeNull();
    expect(overlay).toHaveClass("bg-black/[.28]", "z-[60]");
  });
});
