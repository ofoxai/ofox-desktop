import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import {
  LocalApprovalCard,
  LocalChangesLine,
  LocalConfigControl,
  LocalTaskProgress,
  type LocalApproval,
} from "@/components/assistant/AssistantLocalConfig";
import type { AssistantLocalChange } from "@/lib/api/assistant";

beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

const permission = () => screen.getByRole("combobox", { name: "Permission" });
const pick = async (name: RegExp) => {
  await userEvent.click(permission());
  await userEvent.click(await screen.findByRole("option", { name }));
};

describe("the composer's local configuration control", () => {
  const props = () => ({
    on: true,
    mode: "default" as const,
    disabled: false,
    onToggle: vi.fn(),
    onMode: vi.fn().mockResolvedValue(true),
  });

  it("is a plain toggle while off", async () => {
    const p = { ...props(), on: false };
    render(<LocalConfigControl {...p} />);
    const toggle = screen.getByRole("button", { name: "Local configuration" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");
    expect(screen.queryByRole("combobox")).toBeNull();
    await userEvent.click(toggle);
    expect(p.onToggle).toHaveBeenCalledWith(true);
  });

  it("names the mode while on, and its name turns it off", async () => {
    const p = props();
    render(<LocalConfigControl {...p} />);
    expect(permission()).toHaveTextContent("Default");
    const toggle = screen.getByRole("button", { name: "Local configuration" });
    expect(toggle).toHaveAttribute("aria-pressed", "true");
    await userEvent.click(toggle);
    expect(p.onToggle).toHaveBeenCalledWith(false);
  });

  it("offers the three modes and turning it off", async () => {
    const p = props();
    render(<LocalConfigControl {...p} />);
    await userEvent.click(permission());
    const names = (await screen.findAllByRole("option")).map(
      (option) => option.textContent,
    );
    expect(names).toEqual([
      expect.stringMatching(/^Default/),
      expect.stringMatching(/^Auto edit/),
      expect.stringMatching(/^Full auto/),
      "Turn off local configuration",
    ]);
    await userEvent.click(screen.getByRole("option", { name: /^Auto edit/ }));
    expect(p.onMode).toHaveBeenCalledWith("autoEdit", undefined);
    await pick(/^Turn off/);
    expect(p.onToggle).toHaveBeenCalledWith(false);
  });

  it("asks before full auto, and cancelling changes nothing", async () => {
    const p = props();
    render(<LocalConfigControl {...p} />);
    await pick(/^Full auto/);
    const dialog = await screen.findByRole("dialog", {
      name: "Turn on full auto?",
    });
    expect(dialog).toHaveTextContent(/install, update or remove tools/);
    expect(dialog).not.toHaveTextContent(/separate approval/);
    await userEvent.click(
      within(dialog).getByRole("button", { name: "Cancel" }),
    );
    expect(p.onMode).not.toHaveBeenCalled();
    await pick(/^Full auto/);
    await userEvent.click(
      await screen.findByRole("button", { name: "Turn on full auto" }),
    );
    await waitFor(() =>
      expect(p.onMode).toHaveBeenCalledWith("fullAuto", true),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("can't be changed while locked", () => {
    render(<LocalConfigControl {...props()} disabled />);
    expect(
      screen.getByRole("button", { name: "Local configuration" }),
    ).toBeDisabled();
    expect(permission()).toBeDisabled();
  });
});

describe("an approval in the transcript", () => {
  const approval: LocalApproval = {
    approvalId: "a",
    toolCallId: "t",
    name: "rebind_tool",
    category: "write_config",
    details: '{"tool":"opencode"}',
    allowSession: true,
    overwrites: [
      "~/.config/opencode/opencode.json · provider.ofox-opencode.options.baseURL",
    ],
    expectedOwner: "o",
    expectedGeneration: 1,
  };

  it("says what it does and which edited fields it sets back", () => {
    render(<LocalApprovalCard approval={approval} onRespond={vi.fn()} />);
    const card = screen.getByRole("group", { name: "Approve operation" });
    expect(card).toHaveTextContent("Repair binding");
    expect(card).toHaveTextContent(
      "These edited fields will be set back to Ofox’s values for the current region:",
    );
    expect(within(card).getByRole("listitem")).toHaveTextContent(
      "provider.ofox-opencode.options.baseURL",
    );
    expect(within(card).getByText('{"tool":"opencode"}')).not.toBeVisible();
  });

  it.each([
    ["Allow once", "once"],
    ["Always allow in this conversation", "session"],
    ["Deny", "deny"],
  ])("answers %s", async (name, choice) => {
    const onRespond = vi.fn().mockResolvedValue(true);
    render(
      <LocalApprovalCard
        approval={{ ...approval, overwrites: undefined }}
        onRespond={onRespond}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name }));
    expect(onRespond).toHaveBeenCalledWith("a", choice);
  });
});

describe("the changes line", () => {
  const change = (
    id: string,
    paths: string[],
    restored = false,
  ): AssistantLocalChange => ({
    id,
    operation: "write",
    paths,
    restored,
    outcomeKnown: true,
    changed: true,
  });

  it("is absent once every change is restored", () => {
    const { container } = render(
      <LocalChangesLine
        changes={[change("c", ["/a"], true)]}
        disabled={false}
        onRestore={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("counts the files, lists them on click and restores", async () => {
    const onRestore = vi.fn().mockResolvedValue(true);
    render(
      <LocalChangesLine
        changes={[
          change("c1", ["/a", "/b"]),
          change("c2", ["/b"]),
          change("c3", ["/old"], true),
        ]}
        disabled={false}
        onRestore={onRestore}
      />,
    );
    const count = screen.getByRole("button", { name: "Changed 2 files" });
    expect(screen.queryByText("/a")).toBeNull();
    await userEvent.click(count);
    expect(count).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getAllByRole("listitem").map((item) => item.textContent),
    ).toEqual(["/a", "/b"]);
    await userEvent.click(
      screen.getByRole("button", {
        name: "Restore this conversation’s changes",
      }),
    );
    expect(onRestore).toHaveBeenCalledOnce();
  });

  it("can't restore while a run is under way", () => {
    render(
      <LocalChangesLine
        changes={[change("c", ["/a"])]}
        disabled
        onRestore={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("button", {
        name: "Restore this conversation’s changes",
      }),
    ).toBeDisabled();
  });
});

describe("a local run's progress", () => {
  it("shows the step, warning and plan", () => {
    render(
      <LocalTaskProgress
        intent="Reading the Codex config"
        warning="repeat_warning"
        plan={[
          { step: "Inspect", status: "completed" },
          { step: "Repair", status: "in_progress" },
        ]}
      />,
    );
    expect(screen.getByText("Reading the Codex config")).toBeVisible();
    expect(screen.getByRole("status")).toHaveTextContent(
      /repeated three times/,
    );
    expect(
      within(screen.getByRole("list", { name: "Task steps" }))
        .getAllByRole("listitem")
        .map((item) => item.textContent),
    ).toEqual(["✓ Inspect", "→ Repair"]);
  });

  it("is nothing without any of them", () => {
    const { container } = render(
      <LocalTaskProgress intent={null} warning={null} plan={[]} />,
    );
    expect(container).toBeEmptyDOMElement();
  });
});
