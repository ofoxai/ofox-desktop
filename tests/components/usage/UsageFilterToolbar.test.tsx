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
const toolsMenu = () => screen.getByRole("combobox", { name: "工具筛选" });
const pickTool = async (name: string) => {
  if (toolsMenu().getAttribute("aria-expanded") !== "true")
    await userEvent.click(toolsMenu());
  await userEvent.click(screen.getByRole("option", { name }));
};

it("filters tools from a menu: picks add up, and 全部工具 clears only the tool scope", async () => {
  render(<Fixture />);
  expect(toolsMenu()).toHaveTextContent("全部工具");
  await pickTool("Claude Code");
  await pickTool("Codex");
  expect(screen.getByRole("option", { name: "Claude Code" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  expect(screen.getByRole("option", { name: "Codex" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await userEvent.click(screen.getByRole("button", { name: "全部工具" }));
  expect(
    screen.getByRole("button", { name: "移除Model：model-a" }),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: "移除Tool：codex" })).toBeNull();
});

it("shows each tool's badge in the menu", async () => {
  render(<Fixture />);
  await userEvent.click(toolsMenu());
  expect(
    screen
      .getByRole("option", { name: "Codex" })
      .querySelector("[data-tool-badge], svg, img"),
  ).not.toBeNull();
});

it("removes one condition, keeps the others and gives the next chip focus without scrolling", async () => {
  render(<Fixture />);
  await pickTool("Codex");
  await userEvent.keyboard("{Escape}");
  const focus = vi.spyOn(HTMLElement.prototype, "focus");
  await userEvent.click(
    screen.getByRole("button", { name: "移除Tool：codex" }),
  );
  const model = screen.getByRole("button", { name: "移除Model：model-a" });
  await waitFor(() => expect(model).toHaveFocus());
  expect(focus).toHaveBeenCalledWith({ preventScroll: true });
  focus.mockRestore();
  await userEvent.click(model);
  await waitFor(() => expect(toolsMenu()).toHaveFocus());
  expect(screen.queryByRole("group", { name: "已选筛选" })).toBeNull();
});

it("offers a one-stop reset that returns focus to the tools menu", async () => {
  render(<Fixture />);
  await pickTool("Codex");
  await userEvent.keyboard("{Escape}");
  await userEvent.click(screen.getByRole("button", { name: "重置条件" }));
  await waitFor(() => expect(toolsMenu()).toHaveFocus());
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
  "offers all %i tools in the menu, apart from the secondary filters",
  async (count) => {
    render(<DenseToolbar count={count} />);
    const secondaryGroup = screen.getByRole("group", {
      name: "模型、项目及其他筛选",
    });
    expect(secondaryGroup).not.toContainElement(toolsMenu());
    for (const name of ["Model filter", "Project filter", "Hermes statistics"])
      expect(
        within(secondaryGroup).getByRole("button", { name }),
      ).toBeEnabled();
    await userEvent.click(toolsMenu());
    for (const option of manyTools.slice(0, count))
      expect(screen.getByRole("option", { name: option.label })).toBeVisible();
    const last = screen.getByRole("option", {
      name: manyTools[count - 1].label,
    });
    expect(last).toHaveAttribute("aria-selected", "true");
    await userEvent.click(last);
    expect(last).toHaveAttribute("aria-selected", "false");
    expect(
      screen.getByRole("button", { name: "移除Tool：claude" }),
    ).toBeInTheDocument();
  },
);
