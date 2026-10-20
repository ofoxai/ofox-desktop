import { useState } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageFilterToolbar } from "@/components/usage-overview/UsageFilterToolbar";

const tools = [
  { value: "claude", label: "Claude Code" },
  { value: "codex", label: "Codex" },
];
const manyTools = [
  { value: "claude", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "dsh", label: "DeepSeek Harness" },
  { value: "gemini", label: "Gemini CLI" },
  { value: "opencode", label: "OpenCode" },
  { value: "workbuddy", label: "WorkBuddy" },
  ...Array.from({ length: 6 }, (_, i) => ({
    value: `extra-${i}`,
    label:
      i === 5 ? `LongTool-${"VeryLongName".repeat(16)}` : `Extra tool ${i}`,
  })),
];
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
});
function Fixture() {
  const [selected, setSelected] = useState<string[]>([]);
  const [model, setModel] = useState(true);
  return (
    <UsageFilterToolbar
      tools={tools}
      selectedTools={selected}
      onToolsChange={setSelected}
      disabled={false}
      chips={[
        ...selected.map((id) => ({
          key: id,
          name: "Tool",
          value: id,
          remove: () =>
            setSelected((old) => old.filter((value) => value !== id)),
        })),
        ...(model
          ? [
              {
                key: "model",
                name: "Model",
                value: "model-a",
                remove: () => setModel(false),
              },
            ]
          : []),
      ]}
      onReset={() => {
        setSelected([]);
        setModel(false);
      }}
    >
      {null}
    </UsageFilterToolbar>
  );
}
it("keeps tool selection additive and all-tools clears only the tool scope", async () => {
  render(<Fixture />);
  const group = within(screen.getByRole("group", { name: "工具筛选" }));
  await userEvent.click(group.getByRole("button", { name: "Claude Code" }));
  await userEvent.click(group.getByRole("button", { name: "Codex" }));
  expect(group.getByRole("button", { name: "Claude Code" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(group.getByRole("button", { name: "Codex" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await userEvent.click(group.getByRole("button", { name: "全部工具" }));
  expect(group.getByRole("button", { name: "全部工具" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(
    screen.getByRole("button", { name: "移除Model：model-a" }),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: "移除Tool：codex" })).toBeNull();
});
it("removes one condition, keeps the others and gives the next chip focus without scrolling", async () => {
  render(<Fixture />);
  await userEvent.click(screen.getByRole("button", { name: "Codex" }));
  const focus = vi.spyOn(HTMLElement.prototype, "focus");
  await userEvent.click(
    screen.getByRole("button", { name: "移除Tool：codex" }),
  );
  const model = screen.getByRole("button", { name: "移除Model：model-a" });
  await waitFor(() => expect(model).toHaveFocus());
  expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  expect(screen.getByRole("button", { name: "全部工具" })).not.toHaveFocus();
  focus.mockRestore();
  await userEvent.click(model);
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "全部工具" })).toHaveFocus(),
  );
  expect(screen.queryByRole("group", { name: "已选筛选" })).toBeNull();
});
it("offers a one-stop reset and arrow-key tool navigation without changing selection", async () => {
  render(<Fixture />);
  const all = screen.getByRole("button", { name: "全部工具" });
  all.focus();
  await userEvent.keyboard("{ArrowRight}");
  expect(screen.getByRole("button", { name: "Claude Code" })).toHaveFocus();
  expect(all).toHaveAttribute("aria-pressed", "true");
  await userEvent.keyboard("{End}{Enter}");
  expect(screen.getByRole("button", { name: "Codex" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await userEvent.click(screen.getByRole("button", { name: "重置条件" }));
  await waitFor(() => expect(all).toHaveFocus());
  expect(screen.queryByRole("group", { name: "已选筛选" })).toBeNull();
});
it("does not move focus or scroll when a ranking updates the filters externally", async () => {
  const props = {
    tools,
    selectedTools: [],
    onToolsChange: vi.fn(),
    disabled: false,
    onReset: vi.fn(),
    children: null,
  };
  const view = render(
    <>
      <button>Rank</button>
      <UsageFilterToolbar {...props} chips={[]} />
    </>,
  );
  const rank = screen.getByRole("button", { name: "Rank" });
  rank.focus();
  view.rerender(
    <>
      <button>Rank</button>
      <UsageFilterToolbar
        {...props}
        selectedTools={["codex"]}
        chips={[
          { key: "codex", name: "Tool", value: "Codex", remove: vi.fn() },
        ]}
      />
    </>,
  );
  expect(rank).toHaveFocus();
  expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "移除Tool：Codex" })).toBeVisible();
});

function DenseToolbar({ count }: { count: number }) {
  const options = manyTools.slice(0, count);
  const [selected, setSelected] = useState([
    options[0].value,
    options.at(-1)!.value,
  ]);
  return (
    <UsageFilterToolbar
      tools={options}
      selectedTools={selected}
      onToolsChange={setSelected}
      disabled={false}
      chips={selected.map((value) => ({
        key: value,
        name: "Tool",
        value,
        remove: () => setSelected((old) => old.filter((id) => id !== value)),
      }))}
      onReset={() => setSelected([])}
    >
      <button>Model filter</button>
      <button>Project filter</button>
      <button>Hermes statistics</button>
    </UsageFilterToolbar>
  );
}

it.each([6, 12])(
  "keeps all %i tool choices and their selection independent of secondary filters with keyboard access to the last long-named tool",
  async (count) => {
    render(<DenseToolbar count={count} />);
    const toolGroup = screen.getByRole("group", { name: "工具筛选" });
    const secondaryGroup = screen.getByRole("group", {
      name: "模型、项目及其他筛选",
    });
    expect(toolGroup).not.toContainElement(secondaryGroup);
    for (const name of ["Model filter", "Project filter", "Hermes statistics"])
      expect(
        within(secondaryGroup).getByRole("button", { name }),
      ).toBeEnabled();
    const choices = within(toolGroup).getAllByRole("button");
    expect(choices).toHaveLength(count + 1);
    for (const option of manyTools.slice(0, count))
      expect(
        within(toolGroup).getByRole("button", { name: option.label }),
      ).toHaveTextContent(option.label);
    const first = within(toolGroup).getByRole("button", { name: "全部工具" });
    const lastOption = manyTools[count - 1];
    const last = within(toolGroup).getByRole("button", {
      name: lastOption.label,
    });
    expect(last).toHaveAttribute("aria-pressed", "true");
    first.focus();
    await userEvent.keyboard("{End}");
    expect(last).toHaveFocus();
    expect(choices.filter((choice) => choice.tabIndex === 0)).toHaveLength(1);
    await userEvent.keyboard("{Enter}");
    expect(last).toHaveAttribute("aria-pressed", "false");
    expect(
      within(toolGroup).getByRole("button", { name: "Claude Code" }),
    ).toHaveAttribute("aria-pressed", "true");
    await userEvent.tab();
    expect(
      within(secondaryGroup).getByRole("button", { name: "Model filter" }),
    ).toHaveFocus();
    expect(
      screen.getByRole("button", { name: "移除Tool：claude" }),
    ).toBeInTheDocument();
  },
);
