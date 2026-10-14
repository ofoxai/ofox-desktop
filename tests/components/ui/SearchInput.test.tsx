import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import i18n from "i18next";
import { useState } from "react";
import { beforeAll, describe, expect, it, vi } from "vitest";
import { SearchInput } from "@/components/ui/search-input";
import zh from "@/i18n/locales/zh.json";

beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

function Filter({
  initial = "",
  onChange,
}: {
  initial?: string;
  onChange?: (value: string) => void;
}) {
  const [value, setValue] = useState(initial);
  return (
    <SearchInput
      value={value}
      placeholder="筛选会话"
      onChange={(next) => {
        onChange?.(next);
        setValue(next);
      }}
    />
  );
}

describe("SearchInput", () => {
  it("is a 26px searchbox with a search icon that turns its border brand on focus", () => {
    render(<Filter />);
    const box = screen.getByRole("searchbox", { name: "筛选会话" });
    const frame = box.parentElement!;
    expect(frame).toHaveClass(
      "h-[26px]",
      "rounded-ctl",
      "border-line",
      "bg-card",
      "text-ctl",
      "focus-within:border-brand",
    );
    expect(frame.querySelector("svg.lucide-search")).not.toBeNull();
  });

  it("reports what is typed", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Filter onChange={onChange} />);
    await user.type(screen.getByRole("searchbox"), "ab");
    expect(onChange).toHaveBeenLastCalledWith("ab");
  });

  it("offers a clear button only while there is text, and clearing keeps focus in the field", async () => {
    const user = userEvent.setup();
    render(<Filter />);
    expect(screen.queryByRole("button", { name: "清除搜索" })).toBeNull();
    const box = screen.getByRole("searchbox");
    await user.type(box, "codex");
    await user.click(screen.getByRole("button", { name: "清除搜索" }));
    expect(box).toHaveValue("");
    expect(box).toHaveFocus();
    expect(screen.queryByRole("button", { name: "清除搜索" })).toBeNull();
  });

  it("clears with Escape", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<Filter initial="codex" onChange={onChange} />);
    screen.getByRole("searchbox").focus();
    await user.keyboard("{Escape}");
    expect(onChange).toHaveBeenCalledWith("");
    expect(screen.getByRole("searchbox")).toHaveValue("");

    onChange.mockClear();
    await user.keyboard("{Escape}");
    expect(onChange).not.toHaveBeenCalled();
  });
});
