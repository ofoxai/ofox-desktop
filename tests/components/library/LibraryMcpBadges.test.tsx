import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act } from "react";
import { render, screen, within } from "@testing-library/react";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { LibraryDiscover } from "@/components/library/LibraryDiscover";

beforeEach(() => {
  for (const [language, copy] of Object.entries({ zh, en, ja }))
    i18n.addResourceBundle(language, "translation", copy, true, true);
});

afterEach(async () => {
  await act(async () => {
    await i18n.changeLanguage("zh");
  });
});

describe("MCP preparation badges", () => {
  for (const [language, login, apiKey] of [
    ["zh", "需登录", "需要 API key"],
    ["en", "Sign-in required", "API key required"],
    ["ja", "ログインが必要", "API キーが必要"],
  ]) {
    it(`shows sign-in and API key badges without opening details in ${language}`, async () => {
      await act(async () => {
        await i18n.changeLanguage(language);
      });
      const { container } = render(
        <LibraryDiscover kind="mcp" onChoose={vi.fn()} />,
      );
      const notion = screen.getByRole("article", { name: "Notion" });
      const tavily = screen.getByRole("article", { name: "Tavily" });
      const fetch = screen.getByRole("article", { name: "Fetch" });
      expect(within(notion).getByText(login)).toBeVisible();
      expect(within(notion).queryByText(apiKey)).not.toBeInTheDocument();
      expect(within(tavily).getByText(apiKey)).toBeVisible();
      expect(within(tavily).queryByText(login)).not.toBeInTheDocument();
      expect(within(fetch).queryByText(login)).not.toBeInTheDocument();
      expect(within(fetch).queryByText(apiKey)).not.toBeInTheDocument();
      expect(container.textContent).not.toMatch(/library\.discover/);
      expect(
        container.querySelector(
          "select,input[type=checkbox],input[type=radio]",
        ),
      ).toBeNull();
    });
  }
});
