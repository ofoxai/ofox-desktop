import { describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import { SHELL_PAGES } from "@/components/shell/ShellContext";

type Tree = { [key: string]: unknown };

function keys(tree: Tree, prefix = ""): string[] {
  return Object.entries(tree).flatMap(([key, value]) =>
    value && typeof value === "object"
      ? keys(value as Tree, `${prefix}${key}.`)
      : [`${prefix}${key}`],
  );
}

describe("assistant navigation", () => {
  it("places the built-in assistant last in the 使用 group, before usage", () => {
    expect(SHELL_PAGES).toEqual([
      "tools",
      "sessions",
      "library",
      "assistant",
      "usage",
    ]);
  });

  it("names the tab and every assistant string in all three locales", () => {
    const locales = { en, ja, zh } as Record<
      string,
      { assistant: Tree; shell: { tabs: Record<string, string> } }
    >;
    const expected = keys(en.assistant).sort();
    expect(expected.length).toBeGreaterThan(0);
    for (const [lang, data] of Object.entries(locales)) {
      expect(keys(data.assistant).sort(), lang).toEqual(expected);
      expect(data.shell.tabs.assistant, lang).toBeTruthy();
    }
  });
});
