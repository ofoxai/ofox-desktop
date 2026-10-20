import { act, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { LibraryInventoryView } from "@/components/library/LibraryInventoryView";
import {
  libraryApi,
  type LibraryInventory,
  type LibraryKind,
} from "@/lib/api/library";

afterEach(async () => {
  await act(async () => {
    await i18n.changeLanguage("zh");
  });
});

const languages = [
  {
    language: "zh",
    locale: zh,
    instruction: "原生指令文件",
    legacy: "旧提示词数据",
    mcp: "原生 MCP 配置",
    inline: "内联 MCP 配置",
    skill: "本机 Skills 目录",
    fallback: "本机资源",
  },
  {
    language: "en",
    locale: en,
    instruction: "Native instruction files",
    legacy: "Legacy prompt data",
    mcp: "Native MCP configuration",
    inline: "Inline MCP configuration",
    skill: "Local Skills directories",
    fallback: "Local resources",
  },
  {
    language: "ja",
    locale: ja,
    instruction: "ネイティブ指示ファイル",
    legacy: "旧プロンプトデータ",
    mcp: "ネイティブ MCP 設定",
    inline: "インライン MCP 設定",
    skill: "ローカル Skills ディレクトリ",
    fallback: "ローカルリソース",
  },
];

function view(kind: LibraryKind, labels: string[], sameEntry = false) {
  const inventory: LibraryInventory = {
    ticketId: "ticket",
    kind,
    expiresAtMs: Date.now() + 60000,
    sources: labels.map((label, index) => ({
      sourceId: `opaque-source-${index}`,
      toolId: "codex",
      label,
      state: "ready",
      count: 1,
    })),
    entries: (sameEntry ? [labels] : labels.map((label) => [label])).map(
      (_, index) => ({
        entryId: `entry-${index}`,
        kind,
        name: "Same name",
        sources: sameEntry
          ? labels.map((_, index) => ({
              sourceId: `opaque-source-${index}`,
              toolId: "codex",
            }))
          : [{ sourceId: `opaque-source-${index}`, toolId: "codex" }],
        matchedResourceIds: [],
        conflict: false,
      }),
    ),
  };
  vi.spyOn(libraryApi, "inventory").mockResolvedValue(inventory);
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  const imported = vi.spyOn(libraryApi, "importInventory");
  render(
    <LibraryInventoryView
      kind={kind}
      epoch={1}
      onChanged={vi.fn()}
      onBusyChange={vi.fn()}
    />,
  );
  return imported;
}

it.each(languages)(
  "distinguishes same-named native instructions and legacy data in $language",
  async (copy) => {
    i18n.addResourceBundle(
      copy.language,
      "translation",
      copy.locale,
      true,
      true,
    );
    await i18n.changeLanguage(copy.language);
    const imported = view("instructions", [
      "nativeInstructions",
      "legacyPrompts",
    ]);
    const choices = await screen.findAllByRole("checkbox");
    expect(choices[0]).toHaveAccessibleDescription(
      new RegExp(copy.instruction),
    );
    expect(choices[1]).toHaveAccessibleDescription(new RegExp(copy.legacy));
    expect(screen.getByText(new RegExp(copy.instruction))).toBeVisible();
    expect(screen.getByText(new RegExp(copy.legacy))).toBeVisible();
    expect(imported).not.toHaveBeenCalled();
  },
);

it.each(languages)(
  "keeps every merged MCP source category visible in $language",
  async (copy) => {
    i18n.addResourceBundle(
      copy.language,
      "translation",
      copy.locale,
      true,
      true,
    );
    await i18n.changeLanguage(copy.language);
    view("mcp", ["nativeMcp", "nativeMcpInline"], true);
    const choice = await screen.findByRole("checkbox");
    expect(choice).toHaveAccessibleDescription(new RegExp(copy.mcp));
    expect(choice).toHaveAccessibleDescription(new RegExp(copy.inline));
    expect(screen.queryByText(/opaque-source-/)).not.toBeInTheDocument();
  },
);

it.each(languages)(
  "deduplicates Skills source labels and safely handles unknown metadata in $language",
  async (copy) => {
    i18n.addResourceBundle(
      copy.language,
      "translation",
      copy.locale,
      true,
      true,
    );
    await i18n.changeLanguage(copy.language);
    view(
      "skills",
      ["nativeSkills", "nativeSkills", "private-unknown-source"],
      true,
    );
    const choice = await screen.findByRole("checkbox");
    expect(choice).toHaveAccessibleDescription(new RegExp(copy.skill));
    expect(choice).toHaveAccessibleDescription(new RegExp(copy.fallback));
    expect(choice.getAttribute("aria-describedby")).toBeTruthy();
    expect(
      screen.queryByText(/private-unknown-source|opaque-source-/),
    ).not.toBeInTheDocument();
    const description = document.getElementById(
      choice.getAttribute("aria-describedby")!,
    )!;
    expect(description).toHaveAttribute("title", description.textContent);
    expect(description.textContent?.split(copy.skill)).toHaveLength(2);
  },
);
