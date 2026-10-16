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
