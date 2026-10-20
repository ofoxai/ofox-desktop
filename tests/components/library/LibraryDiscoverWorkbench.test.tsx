import zhLocale from "@/i18n/locales/zh.json";
import enLocale from "@/i18n/locales/en.json";
import jaLocale from "@/i18n/locales/ja.json";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { act } from "react";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import { LibraryDiscover } from "@/components/library/LibraryDiscover";
import { LibraryResourceIcon } from "@/components/library/LibraryResourceIcon";
import { LIBRARY_RECOMMENDATIONS } from "@/components/library/libraryRecommendations";
import { LIBRARY_DISCOVER_COPY } from "@/components/library/libraryDiscoverCopy";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";

beforeEach(() => {
  for (const [lang, copy] of Object.entries({
    zh: zhLocale,
    en: enLocale,
    ja: jaLocale,
  }))
    i18n.addResourceBundle(lang, "translation", copy, true, true);
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle(
    "zh",
    "translation",
    {
      library: {
        ...LIBRARY_ADD_COPY.zh,
        ...LIBRARY_DISCOVER_COPY.zh,
      },
    },
    true,
    true,
  );
});

describe("curated resource workbench", () => {
  it("contains eight MCP recipes, six selected skill directories and three templates", () => {
    expect(
      LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "mcp"),
    ).toHaveLength(8);
    expect(
      LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "skills"),
    ).toHaveLength(6);
    expect(
      LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "instructions"),
    ).toHaveLength(3);
    for (const id of ["theme-factory", "canvas-design"]) {
      expect(LIBRARY_RECOMMENDATIONS.find((r) => r.id === id)).toMatchObject({
        kind: "skills",
        repository: { directory: `skills/${id}` },
      });
    }
  });

  it("prefills documented remote endpoints and keeps GitHub credentials out of the recipe", () => {
    const expected = {
      "microsoft-learn": "https://learn.microsoft.com/api/mcp",
      deepwiki: "https://mcp.deepwiki.com/mcp",
      github: "https://api.githubcopilot.com/mcp/readonly",
    };
    for (const [id, url] of Object.entries(expected)) {
      const recipe = LIBRARY_RECOMMENDATIONS.find((r) => r.id === id);
      expect(recipe).toMatchObject({
        kind: "mcp",
        initialDraft: { transport: "http", url, headers: [] },
      });
    }
    const playwright = LIBRARY_RECOMMENDATIONS.find(
      (r) => r.id === "playwright",
    );
    expect(playwright).toMatchObject({
      initialDraft: { command: "npx", argsText: "-y\n@playwright/mcp@latest" },
    });
  });

  it("searches within the active resource kind and resets an empty search without choosing anything", async () => {
    const choose = vi.fn();
    render(<LibraryDiscover kind="mcp" onChoose={choose} />);
    fireEvent.change(
      screen.getByRole("searchbox", { name: "搜索可添加资源" }),
      {
        target: { value: "microsoft" },
      },
    );
    expect(
      screen.getByRole("article", { name: "Microsoft Learn" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("article", { name: "Context7" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("article", { name: "frontend-design" }),
    ).not.toBeInTheDocument();
    fireEvent.change(
      screen.getByRole("searchbox", { name: "搜索可添加资源" }),
      {
        target: { value: "" },
      },
    );
    expect(screen.getAllByRole("article")).toHaveLength(8);
    expect(choose).not.toHaveBeenCalled();
  });

  it("filters by purpose without executing or selecting a recipe", async () => {
    const choose = vi.fn();
    render(<LibraryDiscover kind="mcp" onChoose={choose} />);
    const category = screen.getByRole("combobox", { name: "用途分类" });
    act(() => category.focus());
    fireEvent.keyDown(category, { key: "ArrowDown" });
    const option = screen.getByRole("option", { name: "浏览器" });
    act(() => option.focus());
    fireEvent.keyDown(option, { key: "Enter" });
    expect(screen.getAllByRole("article")).toHaveLength(2);
    expect(
      screen.getByRole("article", { name: "Chrome DevTools" }),
    ).toBeVisible();
    expect(screen.getByRole("article", { name: "Playwright" })).toBeVisible();
    expect(choose).not.toHaveBeenCalled();
    expect(
      document.querySelector("select,input[type=checkbox],input[type=radio]"),
    ).toBeNull();
  });

  it("locates an exact backend-confirmed match instead of creating a duplicate", async () => {
    const choose = vi.fn();
    const locate = vi.fn();
    const match = {
      state: "exactLibrary" as const,
      resourceId: "saved-context7",
    };
    render(
      <LibraryDiscover
        kind="mcp"
        onChoose={choose}
        matches={{ context7: match }}
        onLocateExisting={locate}
      />,
    );
    const card = screen.getByRole("article", { name: "Context7" });
    await userEvent.click(
      within(card).getByRole("button", { name: "查看已有配置" }),
    );
    expect(locate).toHaveBeenCalledWith(match);
    expect(choose).not.toHaveBeenCalled();
  });

  it("keeps native-other configuration distinct and leaves unconfirmed recipes selectable", async () => {
    const choose = vi.fn();
    const locate = vi.fn();
    const match = { state: "nativeOther" as const, entryId: "native-context7" };
    render(
      <LibraryDiscover
        kind="mcp"
        onChoose={choose}
        matches={{ context7: match }}
        onLocateExisting={locate}
      />,
    );
    const card = screen.getByRole("article", { name: "Context7" });
    await userEvent.click(
      within(card).getByRole("button", { name: "配置详情" }),
    );
    expect(within(card).getByText("本机有其他配置")).toBeVisible();
    await userEvent.click(
      within(card).getByRole("button", { name: "查看本机配置" }),
    );
    expect(locate).toHaveBeenCalledWith(match);
    await userEvent.click(
      within(card).getByRole("button", { name: "选择 Context7" }),
    );
    expect(choose).toHaveBeenCalledWith(
      expect.objectContaining({ id: "context7" }),
      expect.any(HTMLButtonElement),
    );
    expect(
      within(screen.getByRole("article", { name: "Filesystem" })).queryByText(
        "本机有其他配置",
      ),
    ).not.toBeInTheDocument();
  });

  it("explains GitHub read-only limits and native checks in details rather than claiming a tested connection", async () => {
    render(<LibraryDiscover kind="mcp" onChoose={vi.fn()} />);
    const card = screen.getByRole("article", { name: "GitHub" });
    await userEvent.click(
      within(card).getByRole("button", { name: "配置详情" }),
    );
    expect(within(card).getByText("仅提供只读工具集")).toBeVisible();
    expect(
      within(card).getByText("仍需凭据；实际访问权限由凭据决定。"),
    ).toBeVisible();
    const playwright = screen.getByRole("article", { name: "Playwright" });
    await userEvent.click(
      within(playwright).getByRole("button", { name: "配置详情" }),
    );
    expect(within(playwright).getByText(/请在对应工具中检查/)).toBeVisible();
    expect(screen.queryByText("已实测连接成功")).not.toBeInTheDocument();
  });
});

afterEach(async () => {
  await act(async () => {
    await i18n.changeLanguage("zh");
  });
});

describe("curated evidence and keyboard boundaries", () => {
  it("never infers absent or collected state when backend evidence is missing", async () => {
    const choose = vi.fn();
    render(<LibraryDiscover kind="mcp" onChoose={choose} />);
    const card = screen.getByRole("article", { name: "Context7" });
    expect(within(card).queryByText("未收录")).not.toBeInTheDocument();
    expect(within(card).queryByText("已收录")).not.toBeInTheDocument();
    await userEvent.click(
      within(card).getByRole("button", { name: "选择 Context7" }),
    );
    expect(choose).toHaveBeenCalledWith(
      expect.objectContaining({ id: "context7" }),
      expect.any(HTMLButtonElement),
    );
  });

  it("requires an explicit source action and respects busy state", async () => {
    const open = vi.fn();
    const choose = vi.fn();
    const { rerender } = render(
      <LibraryDiscover kind="mcp" onChoose={choose} onOpenSource={open} />,
    );
    const card = screen.getByRole("article", { name: "Microsoft Learn" });
    await userEvent.click(
      within(card).getByRole("button", { name: "配置详情" }),
    );
    expect(open).not.toHaveBeenCalled();
    await userEvent.click(
      within(card).getByRole("button", { name: "查看来源" }),
    );
    expect(open).toHaveBeenCalledWith(
      "https://learn.microsoft.com/en-us/training/support/mcp",
    );
    rerender(
      <LibraryDiscover kind="mcp" busy onChoose={choose} onOpenSource={open} />,
    );
    expect(
      within(card).getByRole("button", { name: "选择 Microsoft Learn" }),
    ).toBeDisabled();
    expect(
      within(card).getByRole("button", { name: "查看来源" }),
    ).toBeDisabled();
  });

  it("moves focus to the surviving search input when clearing empty results", async () => {
    render(<LibraryDiscover kind="mcp" onChoose={vi.fn()} />);
    const search = screen.getByRole("searchbox", { name: "搜索可添加资源" });
    fireEvent.change(search, { target: { value: "no-such-curated-recipe" } });
    await userEvent.click(screen.getByRole("button", { name: "清除筛选" }));
    expect(search).toHaveFocus();
    expect(screen.getAllByRole("article")).toHaveLength(8);
  });

  it("uses only bundled trusted icons and falls back for unknown IDs", () => {
    const { container, rerender } = render(
      <LibraryResourceIcon
        kind="mcp"
        recipeId="https://untrusted.invalid/logo.svg"
      />,
    );
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector("svg")).not.toBeNull();
    rerender(<LibraryResourceIcon kind="mcp" recipeId="__proto__" />);
    expect(container.querySelector("img")).toBeNull();
    rerender(<LibraryResourceIcon kind="mcp" recipeId="github" />);
    expect(container.querySelector("img")?.getAttribute("src")).not.toMatch(
      /^https?:/,
    );
  });

  for (const language of ["zh", "en", "ja"] as const) {
    for (const kind of ["mcp", "skills", "instructions"] as const) {
      it(`localizes ${kind} metadata and detail actions in ${language}`, async () => {
        i18n.addResourceBundle(
          language,
          "translation",
          {
            library: {
              ...LIBRARY_ADD_COPY[language],
              ...LIBRARY_DISCOVER_COPY[language],
            },
          },
          true,
          true,
        );
        await act(async () => {
          await i18n.changeLanguage(language);
        });
        const { container } = render(
          <LibraryDiscover kind={kind} onChoose={vi.fn()} />,
        );
        const card = screen.getAllByRole("article")[0];
        await userEvent.click(
          within(card).getByRole("button", {
            name: LIBRARY_DISCOVER_COPY[language].discover2.details,
          }),
        );
        expect(container.textContent).not.toMatch(/library\.discover/);
        expect(
          within(card).getByText(
            LIBRARY_DISCOVER_COPY[language].discover2.reviewBoundary,
          ),
        ).toBeVisible();
      });
    }
  }

  it("keeps locale keys and placeholders equal across all languages", () => {
    function leaves(value: unknown, prefix = ""): Record<string, string> {
      return Object.fromEntries(
        Object.entries(value as Record<string, unknown>).flatMap(
          ([key, child]) =>
            typeof child === "string"
              ? [[`${prefix}${key}`, child]]
              : Object.entries(leaves(child, `${prefix}${key}.`)),
        ),
      );
    }
    const reference = leaves(LIBRARY_DISCOVER_COPY.zh);
    for (const locale of [LIBRARY_DISCOVER_COPY.en, LIBRARY_DISCOVER_COPY.ja]) {
      const values = leaves(locale);
      expect(Object.keys(values).sort()).toEqual(Object.keys(reference).sort());
      for (const key of Object.keys(reference))
        expect((values[key].match(/{{[^}]+}}/g) ?? []).sort()).toEqual(
          (reference[key].match(/{{[^}]+}}/g) ?? []).sort(),
        );
    }
  });
});
