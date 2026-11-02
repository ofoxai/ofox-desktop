import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantLocalPanel } from "@/components/assistant/AssistantLocalPanel";
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("@/components/ConfirmDialog", () => ({
  ConfirmDialog: ({ isOpen, onConfirm, onCancel }: any) =>
    isOpen ? (
      <div>
        <button onClick={onConfirm}>confirm full</button>
        <button onClick={onCancel}>cancel full</button>
      </div>
    ) : null,
}));
const base = {
  mode: "autoConfig" as const,
  locked: false,
  approvals: [],
  changes: [],
  plan: [],
  intent: null,
  warning: null,
  cost: 0,
  onPermission: vi.fn().mockResolvedValue(true),
  onApproval: vi.fn().mockResolvedValue(true),
  onRestore: vi.fn().mockResolvedValue(true),
};
describe("local configuration controls", () => {
  beforeEach(()=>{
    Element.prototype.hasPointerCapture=()=>false;
    Element.prototype.setPointerCapture=()=>{};
    Element.prototype.releasePointerCapture=()=>{};
    Element.prototype.scrollIntoView=()=>{};
  });
  it("requires a separate full-access confirmation and cancellation does not change permissions", async () => {
    const onPermission = vi.fn().mockResolvedValue(true);
    render(<AssistantLocalPanel {...base} onPermission={onPermission} />);
    await userEvent.click(screen.getByRole("combobox"));
    await userEvent.click(screen.getByRole("option",{name:"assistant.local.modes.fullAccess"}));
    expect(onPermission).not.toHaveBeenCalled();
    fireEvent.click(screen.getByText("cancel full"));
    expect(onPermission).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("combobox"));
    await userEvent.click(screen.getByRole("option",{name:"assistant.local.modes.fullAccess"}));
    fireEvent.click(screen.getByText("confirm full"));
    await waitFor(() =>
      expect(onPermission).toHaveBeenCalledWith("fullAccess", true),
    );
  });
  it("offers no session grant for irreversible actions and only restores when idle", () => {
    render(
      <AssistantLocalPanel
        {...base}
        locked
        approvals={[
          {
            approvalId: "a",
            name: "install_tool",
            details: "install codex",
            allowSession: false,
          },
        ]}
        changes={[
          {
            id: "c",
            operation: "write",
            paths: ["/config"],
            restored: false,
            outcomeKnown: true,
            changed: true,
          },
        ]}
      />,
    );
    expect(
      screen.queryByText("assistant.local.allowSession"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("assistant.local.restore")).toBeDisabled();
    fireEvent.click(screen.getByText("assistant.local.deny"));
    expect(base.onApproval).toHaveBeenCalledWith("a", "deny");
  });
});
