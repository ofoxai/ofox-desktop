import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import i18n from "i18next";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import { ThinkingMenu } from "@/components/assistant/ThinkingMenu";
import type {
  AssistantModel,
  AssistantThinkingLevel,
} from "@/lib/api/assistant";

const reasoning = (levels: AssistantThinkingLevel[]) =>
  ({ id: "a/b", name: "A B", thinkingLevels: levels }) as AssistantModel;

beforeAll(() => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("ja", "translation", ja, true, true);
  Element.prototype.scrollIntoView = vi.fn();
});
beforeEach(async () => {
  await i18n.changeLanguage("en");
});

function menu(props: Partial<Parameters<typeof ThinkingMenu>[0]> = {}) {
  return (
    <ThinkingMenu
      model={reasoning(["low", "medium", "high"])}
      level={null}
      disabled={false}
      onChange={vi.fn()}
      {...props}
    />
  );
}
const trigger = (name = "Thinking") => screen.getByRole("combobox", { name });
const optionNames = () =>
  screen.getAllByRole("option").map((option) => option.textContent);

describe("the thinking menu", () => {
  it("says the conversation's level on its trigger, the default when none was chosen", () => {
    render(menu());
    expect(trigger()).toHaveTextContent("Thinking: Default");
  });

  it("offers the default and the levels the model lists, in its order", async () => {
    render(menu());
    await userEvent.click(trigger());
    expect(optionNames()).toEqual(["Default", "Low", "Medium", "High"]);
  });

  it("offers off, and the higher levels, only for a model that lists them", async () => {
    render(
      menu({
        model: reasoning(["off", "minimal", "low", "xhigh", "max"]),
      }),
    );
    await userEvent.click(trigger());
    expect(optionNames()).toEqual([
      "Default",
      "Off",
      "Minimal",
      "Low",
      "Extra high",
      "Max",
    ]);
  });

  it("says what deeper thinking costs at the head of the menu", async () => {
    render(menu());
    await userEvent.click(trigger());
    expect(
      within(
        screen.getByRole("listbox").parentElement as HTMLElement,
      ).getByText(
        "Deeper thinking uses more output tokens, billed at the output price, and replies take longer.",
      ),
    ).toBeVisible();
  });

  it("marks the chosen level and shows it on the trigger", async () => {
    render(menu({ level: "medium" }));
    expect(trigger()).toHaveTextContent("Thinking: Medium");
    await userEvent.click(trigger());
    expect(screen.getByRole("option", { name: "Medium" })).toHaveAttribute(
      "data-checked",
      "true",
    );
  });

  it("reads a level the model no longer offers as the default", () => {
    render(menu({ level: "xhigh" }));
    expect(trigger()).toHaveTextContent("Thinking: Default");
  });

  it("reports the choice, and 'default' to clear it", async () => {
    const onChange = vi.fn();
    render(menu({ level: "high", onChange }));
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "Low" }));
    expect(onChange).toHaveBeenLastCalledWith("low");
    await userEvent.click(trigger());
    await userEvent.click(screen.getByRole("option", { name: "Default" }));
    expect(onChange).toHaveBeenLastCalledWith("default");
  });

  it("is hidden for a model that offers no levels, or none is known", () => {
    const { rerender } = render(menu({ model: reasoning([]) }));
    expect(screen.queryByRole("combobox")).toBeNull();
    rerender(
      menu({ model: { id: "a/b", name: "A B" } as unknown as AssistantModel }),
    );
    expect(screen.queryByRole("combobox")).toBeNull();
    rerender(menu({ model: undefined }));
    expect(screen.queryByRole("combobox")).toBeNull();
  });

  it("looks like the Web search toggle beside it, even with a level chosen", () => {
    render(menu({ level: "high" }));
    expect(trigger()).toHaveClass(
      "h-[27px]",
      "px-2",
      "border-transparent",
      "bg-transparent",
      "text-ink-ctl",
      "hover:bg-pill",
      "hover:text-ink",
    );
    expect(trigger()).not.toHaveClass("text-ink");
  });

  it("can be disabled", () => {
    render(menu({ disabled: true }));
    expect(trigger()).toBeDisabled();
  });

  it.each([
    ["zh", "思考", "思考：默认", ["默认", "低", "中", "高"]],
    ["ja", "思考", "思考：既定", ["既定", "低", "中", "高"]],
  ])("is worded in %s", async (language, label, triggerText, options) => {
    await i18n.changeLanguage(language);
    render(menu());
    expect(trigger(label)).toHaveTextContent(triggerText);
    await userEvent.click(trigger(label));
    expect(optionNames()).toEqual(options);
  });

  it("says the cost note in Chinese as the product words it", () => {
    expect(i18n.getFixedT("zh")("assistant.thinking.costNote")).toBe(
      "思考越深输出 token 越多，按输出单价计费，回复更慢",
    );
  });
});
