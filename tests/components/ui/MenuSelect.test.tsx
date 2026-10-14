import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import i18n from "i18next";
import { useState } from "react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import {
  MenuSelect,
  type MenuSelectOption,
  type MenuSelectProps,
} from "@/components/ui/menu-select";
import zh from "@/i18n/locales/zh.json";

type Agent = "claude" | "codex" | "gemini" | "pi";

const OPTIONS: MenuSelectOption<Agent>[] = [
  { value: "claude", label: "Claude Code", note: "Anthropic", count: 12 },
  { value: "codex", label: "Codex", count: 7 },
  { value: "gemini", label: "Gemini", disabled: true },
  { value: "pi", label: "Pi", count: 2 },
];

type HarnessProps = Partial<MenuSelectProps<Agent>> & {
  initial?: Agent;
  onPick?: (value: Agent) => void;
};

function Harness({ initial = "codex", onPick, ...rest }: HarnessProps) {
  const [value, setValue] = useState<Agent>(initial);
  return (
    <MenuSelect<Agent>
      aria-label="Agent"
      value={value}
      onChange={(next) => {
        onPick?.(next);
        setValue(next);
      }}
      options={OPTIONS}
      {...rest}
    />
  );
}

beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  // cmdk scrolls the highlighted option into view; jsdom has no layout.
  Element.prototype.scrollIntoView = vi.fn();
});

function option(name: RegExp) {
  return screen.getByRole("option", { name });
}

async function openMenu() {
  const user = userEvent.setup();
  const trigger = screen.getByRole("combobox", { name: "Agent" });
  await user.click(trigger);
  return { user, trigger, listbox: screen.getByRole("listbox") };
}

describe("MenuSelect", () => {
  it("is a combobox showing the chosen option's label and count that opens a listbox", async () => {
    render(<Harness />);
    const trigger = screen.getByRole("combobox", { name: "Agent" });
    expect(trigger).toHaveAttribute("aria-haspopup", "listbox");
    expect(trigger).toHaveAttribute("aria-expanded", "false");
    expect(trigger).toHaveTextContent("Codex");
    expect(trigger).toHaveTextContent("7");

    const { listbox } = await openMenu();
    expect(trigger).toHaveAttribute("aria-expanded", "true");
    expect(listbox).toHaveAccessibleName("Agent");
    expect(within(listbox).getAllByRole("option")).toHaveLength(4);
  });

  it("draws the trigger as magpie's 26px picker with a chevron that turns when open", () => {
    render(<Harness />);
    const trigger = screen.getByRole("combobox", { name: "Agent" });
    expect(trigger).toHaveClass(
      "h-[26px]",
      "max-w-[180px]",
      "gap-1",
      "rounded-ctl",
      "border",
      "border-line",
      "bg-card",
      "text-ctl",
      "text-ink",
    );
    expect(trigger.querySelector("svg.lucide-chevron-down")).toHaveClass(
      "group-data-[state=open]:rotate-180",
    );
  });

  it("shows the placeholder in the control colour when no option holds the value", () => {
    render(
      <MenuSelect
        aria-label="Model"
        value=""
        onChange={vi.fn()}
        options={[{ value: "a", label: "A" }]}
        placeholder="Pick one"
      />,
    );
    const trigger = screen.getByRole("combobox", { name: "Model" });
    expect(trigger).toHaveTextContent("Pick one");
    expect(trigger).toHaveClass("text-ink-ctl");
    expect(trigger).not.toHaveClass("text-ink");
  });

  it("draws the menu as a 262px frosted pop with a line border", async () => {
    render(<Harness head="Show sessions from" />);
    const { listbox } = await openMenu();
    const pop = listbox.closest("[data-radix-popper-content-wrapper] > *");
    expect(pop).toHaveClass(
      "rounded-pop",
      "bg-pop-bg",
      "backdrop-blur-xl",
      "border",
      "border-line",
      "p-1",
      "shadow-pop",
      "animate-pop-in",
      "max-h-[min(360px,70vh)]",
    );
    expect(pop).toHaveStyle({ width: "262px" });
    expect(screen.getByText("Show sessions from")).toHaveClass(
      "text-note",
      "text-ink-muted",
    );
  });

  it("ticks the chosen option and highlights it when the menu opens", async () => {
    render(<Harness />);
    await openMenu();
    const codex = option(/^Codex/);
    expect(codex).toHaveAttribute("data-checked", "true");
    expect(codex.querySelector("svg.lucide-check")).not.toBeNull();
    expect(codex).toHaveAttribute("aria-selected", "true");
    // focus sits on the option itself, so a screen reader names it at once
    expect(codex).toHaveFocus();
    const claude = option(/^Claude Code/);
    expect(claude).not.toHaveAttribute("data-checked");
    expect(claude.querySelector("svg.lucide-check")).toBeNull();
    expect(claude).toHaveTextContent("Anthropic");
  });

  it("moves with the arrow keys past disabled options, wrapping at both ends, and jumps with Home and End", async () => {
    render(<Harness />);
    const { user } = await openMenu();

    await user.keyboard("{ArrowDown}");
    expect(option(/^Pi/)).toHaveAttribute("aria-selected", "true");
    expect(option(/^Pi/)).toHaveFocus();
    await user.keyboard("{ArrowDown}");
    expect(option(/^Claude Code/)).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{ArrowUp}");
    expect(option(/^Pi/)).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{Home}");
    expect(option(/^Claude Code/)).toHaveAttribute("aria-selected", "true");
    await user.keyboard("{End}");
    expect(option(/^Pi/)).toHaveAttribute("aria-selected", "true");
  });

  it("chooses the highlighted option with Enter, closes, and puts focus back on the trigger", async () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick} />);
    const { user, trigger } = await openMenu();

    await user.keyboard("{ArrowDown}{Enter}");
    expect(onPick).toHaveBeenCalledWith("pi");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveTextContent("Pi");
  });

  it("closes with Escape without choosing and puts focus back on the trigger", async () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick} />);
    const { user, trigger } = await openMenu();

    await user.keyboard("{ArrowDown}{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(onPick).not.toHaveBeenCalled();
    expect(trigger).toHaveFocus();
    expect(trigger).toHaveAttribute("aria-expanded", "false");
  });

  it("opens from the keyboard with the down arrow", async () => {
    const user = userEvent.setup();
    render(<Harness />);
    const trigger = screen.getByRole("combobox", { name: "Agent" });
    trigger.focus();
    await user.keyboard("{ArrowDown}");
    expect(screen.getByRole("listbox")).toBeInTheDocument();
  });

  it("chooses an option on click", async () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick} />);
    const { user, trigger } = await openMenu();
    await user.click(option(/^Claude Code/));
    expect(onPick).toHaveBeenCalledWith("claude");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(trigger).toHaveTextContent("Claude Code");
  });

  it("does not report the option that is already chosen", async () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick} />);
    const { user } = await openMenu();
    await user.click(option(/^Codex/));
    expect(onPick).not.toHaveBeenCalled();
    expect(screen.queryByRole("listbox")).toBeNull();
  });

  it("never chooses a disabled option", async () => {
    const onPick = vi.fn();
    render(<Harness onPick={onPick} />);
    const { user } = await openMenu();
    const gemini = option(/^Gemini/);
    expect(gemini).toHaveAttribute("aria-disabled", "true");
    await user.click(gemini);
    expect(onPick).not.toHaveBeenCalled();
    expect(screen.getByRole("listbox")).toBeInTheDocument();
  });

  it("filters by label and note as the user types when searchable, and Enter takes the first match", async () => {
    const onPick = vi.fn();
    render(
      <Harness onPick={onPick} searchable searchPlaceholder="Filter agents" />,
    );
    const { user } = await openMenu();
    const search = screen.getByRole("combobox", { name: "Filter agents" });
    expect(search).toHaveFocus();
    // the filter points at the option Enter would take
    await waitFor(() =>
      expect(search).toHaveAttribute(
        "aria-activedescendant",
        option(/^Claude Code/).id,
      ),
    );

    await user.keyboard("anthro");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(option(/^Claude Code/)).toBeInTheDocument();

    await user.keyboard("{Enter}");
    expect(onPick).toHaveBeenCalledWith("claude");
  });

  it("points the filter at the first match as the user types and at each option the arrows reach", async () => {
    render(<Harness searchable searchPlaceholder="Filter agents" />);
    const { user } = await openMenu();
    const search = screen.getByRole("combobox", { name: "Filter agents" });

    await user.keyboard("ex");
    expect(screen.getAllByRole("option")).toHaveLength(1);
    expect(search).toHaveAttribute(
      "aria-activedescendant",
      option(/^Codex/).id,
    );
    expect(option(/^Codex/)).toHaveAttribute("aria-selected", "true");

    await user.clear(search);
    await user.keyboard("{ArrowDown}");
    expect(search).toHaveAttribute(
      "aria-activedescendant",
      option(/^Codex/).id,
    );
    expect(search).toHaveFocus();
  });

  it("says so when nothing matches the search", async () => {
    render(<Harness searchable searchPlaceholder="Filter agents" />);
    const { user } = await openMenu();
    await user.keyboard("zzz");
    expect(screen.queryAllByRole("option")).toHaveLength(0);
    expect(screen.getByText("无匹配项")).toBeInTheDocument();
  });

  it("opens inside the enclosing dialog so the dialog keeps its scroll and focus", async () => {
    render(
      <Dialog open>
        <DialogContent aria-describedby={undefined}>
          <DialogTitle>Tool</DialogTitle>
          <Harness />
        </DialogContent>
      </Dialog>,
    );
    const { listbox } = await openMenu();
    expect(screen.getByRole("dialog", { name: "Tool" })).toContainElement(
      listbox,
    );
  });

  it("keeps the dialog open when Escape closes the menu inside it", async () => {
    function ClosableDialog() {
      const [open, setOpen] = useState(true);
      return (
        <Dialog open={open} onOpenChange={setOpen}>
          <DialogContent aria-describedby={undefined}>
            <DialogTitle>Tool</DialogTitle>
            <Harness />
          </DialogContent>
        </Dialog>
      );
    }
    render(<ClosableDialog />);
    const { user, trigger } = await openMenu();
    await user.keyboard("{Escape}");
    expect(screen.queryByRole("listbox")).toBeNull();
    expect(screen.getByRole("dialog", { name: "Tool" })).toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });
});
