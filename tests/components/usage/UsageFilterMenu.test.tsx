import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});
import { UsageFilterMenu } from "@/components/usage-overview/UsageFilterMenu";

const options = Array.from({ length: 300 }, (_, n) => ({
  value: `id-${n}`,
  label: `entry-${n}`,
}));
it("bounds a large analysis filter, expands on demand, and still searches the full option set", async () => {
  const onChange = vi.fn();
  render(
    <UsageFilterMenu
      label="Projects"
      options={options}
      selected={[]}
      onChange={onChange}
      pageSize={50}
    />,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Projects" }));
  expect(screen.getAllByRole("option")).toHaveLength(50);
  await userEvent.click(
    screen.getByRole("button", { name: "显示更多（剩余 250）" }),
  );
  expect(screen.getAllByRole("option")).toHaveLength(100);
  await userEvent.type(screen.getByRole("searchbox"), "entry-299");
  expect(screen.getAllByRole("option")).toHaveLength(1);
  await userEvent.click(screen.getByRole("option", { name: "entry-299" }));
  expect(onChange).toHaveBeenCalledWith(["id-299"]);
});
it("preserves existing unbounded menus when no page size is requested", async () => {
  render(
    <UsageFilterMenu
      label="Legacy"
      options={options}
      selected={[]}
      onChange={vi.fn()}
    />,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Legacy" }));
  expect(screen.getAllByRole("option")).toHaveLength(300);
});

it("shows the selected model name on the compact trigger instead of only an opaque count", () => {
  render(
    <UsageFilterMenu
      label="Models"
      options={options}
      selected={["id-299"]}
      onChange={vi.fn()}
      pageSize={50}
    />,
  );
  expect(screen.getByRole("combobox", { name: "Models" })).toHaveTextContent(
    "entry-299",
  );
});

it("offers an explicit clear action and keeps a selected item beyond the first page discoverable", async () => {
  const change = vi.fn();
  render(
    <UsageFilterMenu
      label="Models"
      options={options}
      selected={["id-299"]}
      onChange={change}
      pageSize={50}
    />,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Models" }));
  expect(screen.getByRole("option", { name: "entry-299" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await userEvent.click(screen.getByRole("button", { name: "清空Models" }));
  expect(change).toHaveBeenCalledWith([]);
});

it("focuses search, reports a selection count, and uses arrows without moving selected rows", async () => {
  render(
    <UsageFilterMenu
      label="Models"
      options={options}
      selected={["id-299"]}
      onChange={vi.fn()}
      pageSize={50}
    />,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Models" }));
  expect(screen.getByRole("searchbox")).toHaveFocus();
  expect(screen.getByText("1 / 300 已选")).toBeVisible();
  await userEvent.keyboard("{ArrowDown}");
  expect(screen.getByRole("option", { name: "entry-299" })).toHaveFocus();
  await userEvent.keyboard("{End}");
  expect(screen.getByRole("option", { name: "entry-48" })).toHaveFocus();
  await userEvent.keyboard("{Escape}");
  expect(screen.getByRole("combobox", { name: "Models" })).toHaveFocus();
});

it("makes the clear and all actions keyboard reachable before tabbing out of the menu", async () => {
  const change = vi.fn();
  render(
    <>
      <button>Before</button>
      <UsageFilterMenu
        label="Models"
        options={options}
        selected={["id-299"]}
        onChange={change}
        pageSize={50}
      />
      <button>After</button>
    </>,
  );
  await userEvent.click(screen.getByRole("combobox", { name: "Models" }));
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "清空Models" })).toHaveFocus();
  await userEvent.keyboard("{Enter}");
  expect(change).toHaveBeenCalledWith([]);
  expect(screen.getByRole("searchbox")).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "清空Models" })).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "全部" })).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("option", { name: "entry-299" })).toHaveFocus();
  await userEvent.tab();
  expect(
    screen.getByRole("button", { name: "显示更多（剩余 250）" }),
  ).toHaveFocus();
  await userEvent.tab();
  expect(screen.getByRole("button", { name: "After" })).toHaveFocus();
  expect(screen.queryByRole("listbox")).toBeNull();
});

it.each(["clear", "all"] as const)(
  "returns focus to search after %s clears controlled selection",
  async (action) => {
    function ControlledMenu() {
      const [selected, setSelected] = useState(["id-299"]);
      return (
        <UsageFilterMenu
          label="Projects"
          options={options}
          selected={selected}
          onChange={setSelected}
          pageSize={50}
        />
      );
    }
    render(<ControlledMenu />);
    await userEvent.click(screen.getByRole("combobox", { name: "Projects" }));
    const search = screen.getByRole("searchbox");
    expect(search).toHaveFocus();
    await userEvent.tab();
    expect(screen.getByRole("button", { name: "清空Projects" })).toHaveFocus();
    if (action === "all") {
      await userEvent.tab();
      expect(screen.getByRole("button", { name: "全部" })).toHaveFocus();
    }
    await userEvent.keyboard("{Enter}");
    expect(screen.getByText("0 / 300 已选")).toBeVisible();
    expect(screen.getByRole("button", { name: "清空Projects" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "全部" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(search).toHaveFocus();
    await userEvent.keyboard("{ArrowDown}{Enter}");
    expect(screen.getByText("1 / 300 已选")).toBeVisible();
    expect(screen.getByRole("option", { name: "entry-299" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  },
);
