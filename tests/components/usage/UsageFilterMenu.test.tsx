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
  await userEvent.type(screen.getByRole("textbox"), "entry-299");
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
