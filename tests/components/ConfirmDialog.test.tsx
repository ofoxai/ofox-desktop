import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ConfirmDialog } from "@/components/ConfirmDialog";

function renderConfirm(
  props: Partial<React.ComponentProps<typeof ConfirmDialog>> = {},
) {
  const onConfirm = vi.fn();
  const onCancel = vi.fn();
  render(
    <ConfirmDialog
      isOpen
      title="Remove Codex?"
      message="Its config goes back to how it was."
      confirmText="Remove"
      cancelText="Keep"
      onConfirm={onConfirm}
      onCancel={onCancel}
      {...props}
    />,
  );
  return { onConfirm, onCancel };
}

describe("ConfirmDialog", () => {
  it("confirms and cancels", () => {
    const { onConfirm, onCancel } = renderConfirm();
    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(onConfirm).toHaveBeenCalledTimes(1);
    fireEvent.click(screen.getByRole("button", { name: "Keep" }));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("fills a destructive confirm with the danger colour", () => {
    renderConfirm();
    expect(screen.getByRole("button", { name: "Remove" })).toHaveClass(
      "bg-danger",
    );
  });

  it("uses the brand primary for an info confirm, with no blue icon", () => {
    renderConfirm({ variant: "info", confirmText: "OK" });
    expect(screen.getByRole("button", { name: "OK" })).toHaveClass("bg-brand");
    expect(screen.getByRole("dialog").innerHTML).not.toMatch(/blue/);
  });

  it("shows details under the message", () => {
    renderConfirm({
      details: (
        <ul aria-label="files">
          <li>config.toml</li>
        </ul>
      ),
    });
    expect(screen.getByRole("list", { name: "files" })).toHaveTextContent(
      "config.toml",
    );
  });

  it("while busy, disables both buttons, marks the confirm busy and ignores Escape", () => {
    const { onConfirm, onCancel } = renderConfirm({ busy: true });
    const confirm = screen.getByRole("button", { name: "Remove" });
    expect(confirm).toBeDisabled();
    expect(confirm).toHaveAttribute("aria-busy", "true");
    expect(screen.getByRole("button", { name: "Keep" })).toBeDisabled();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onCancel).not.toHaveBeenCalled();
    expect(onConfirm).not.toHaveBeenCalled();
  });

  it("cancels on Escape when not busy", () => {
    const { onCancel } = renderConfirm();
    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
    expect(onCancel).toHaveBeenCalledTimes(1);
  });
});
