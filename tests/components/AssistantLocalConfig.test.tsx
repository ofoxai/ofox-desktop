import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import {
  LocalApprovalCard,
  LocalChangesLine,
  PermissionControl,
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

describe("the composer's permission control", () => {
  const props = () => ({
    mode: "default" as const,
    disabled: false,
    onMode: vi.fn().mockResolvedValue(true),
  });

  it("names the mode, with no switch to turn local configuration off", () => {
    render(<PermissionControl {...props()} />);
    expect(permission()).toHaveTextContent("Permission: Default");
    expect(
      screen.queryByRole("button", { name: "Local configuration" }),
    ).toBeNull();
  });

  it("offers the three modes only", async () => {
    const p = props();
    render(<PermissionControl {...p} />);
    await userEvent.click(permission());
    const names = (await screen.findAllByRole("option")).map(
      (option) => option.textContent,
    );
    expect(names).toEqual([
      expect.stringMatching(/^Default/),
      expect.stringMatching(/^Auto edit/),
      expect.stringMatching(/^Full auto/),
    ]);
    await userEvent.click(screen.getByRole("option", { name: /^Auto edit/ }));
    expect(p.onMode).toHaveBeenCalledWith("autoEdit", undefined);
  });

  it("asks before full auto, and cancelling changes nothing", async () => {
    const p = props();
    render(<PermissionControl {...p} />);
    await pick(/^Full auto/);
    const dialog = await screen.findByRole("dialog", {
      name: "Turn on full auto?",
    });
    expect(dialog).toHaveTextContent(/install, update or remove tools/);
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
    render(<PermissionControl {...props()} disabled />);
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

it("shows the actual MCP approval details and all three ordinary choices", async () => {
  const respond = vi.fn().mockResolvedValue(true);
  const base: LocalApproval = {
    approvalId: "mcp",
    toolCallId: "tool",
    name: "mcp__custom__write",
    category: "mcp",
    details: { server: "custom", tool: "mcp__custom__write" },
    allowSession: true,
    expectedOwner: "owner",
    expectedGeneration: 1,
  };
  const result = render(
    <LocalApprovalCard approval={base} onRespond={respond} />,
  );
  expect(screen.getByText("Allow this MCP operation?")).toBeVisible();
  expect(screen.getByText("custom · mcp__custom__write")).toBeVisible();
  const buttons = screen.getAllByRole("button");
  expect(buttons).toHaveLength(3);
  for (const [index, choice] of ["once", "session", "deny"].entries()) {
    await userEvent.click(buttons[index]);
    await waitFor(() =>
      expect(respond).toHaveBeenLastCalledWith("mcp", choice),
    );
    result.rerender(<LocalApprovalCard approval={base} onRespond={respond} />);
  }
});

it("renders MCP argument previews as text with an explicit complete-call truncation warning", async () => {
  const details = {
    server: "custom",
    tool: "mcp__custom__write",
    argumentsPreview: JSON.stringify(
      { target: "draft-inbox", body: "<img src=x onerror=alert(1)>" },
      null,
      2,
    ),
    argumentsTruncated: true,
  };
  const approval: LocalApproval = {
    approvalId: "preview",
    toolCallId: "preview-call",
    name: details.tool,
    category: "mcp",
    details,
    argumentsPreview: details.argumentsPreview,
    argumentsTruncated: details.argumentsTruncated,
    allowSession: true,
    expectedOwner: "owner",
    expectedGeneration: 1,
  };
  const rendered = render(
    <LocalApprovalCard
      approval={approval}
      onRespond={vi.fn().mockResolvedValue(true)}
    />,
  );
  await userEvent.click(screen.getByText("Arguments preview"));
  expect(screen.getByText(/draft-inbox/)).toBeVisible();
  expect(screen.getByText(/Approval runs the complete call/)).toBeVisible();
  expect(rendered.container.querySelector("img")).toBeNull();
  expect(rendered.container.querySelector("pre")?.textContent).toContain(
    "<img src=x onerror=alert(1)>",
  );
});
