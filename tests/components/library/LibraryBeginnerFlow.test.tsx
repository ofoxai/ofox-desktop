import { render, screen, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import { LibraryNativeSources } from "@/components/library/LibraryNativeSources";
import type { LibraryInventory, LibraryKind } from "@/lib/api/library";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

it.each<LibraryKind>(["instructions", "mcp", "skills"])(
  "%s discovery shows only new configurations while preserving different-content variants",
  (kind) => {
    const inventory: LibraryInventory = {
      kind,
      ticketId: "read-only",
      expiresAtMs: Date.now() + 60000,
      sources: [
        {
          sourceId: "s",
          toolId: "codex",
          label: "nativeMcp",
          state: "ready",
          count: 3,
        },
      ],
      entries: [
        {
          entryId: "added",
          kind,
          name: "Already added",
          sources: [{ sourceId: "s", toolId: "codex" }],
          matchedResourceIds: ["mine"],
          conflict: false,
        },
        {
          entryId: "new",
          kind,
          name: "New resource",
          sources: [{ sourceId: "s", toolId: "codex" }],
          matchedResourceIds: [],
          conflict: false,
        },
        {
          entryId: "variant",
          kind,
          name: "Different configuration",
          sources: [{ sourceId: "s", toolId: "codex" }],
          matchedResourceIds: [],
          conflict: true,
        },
      ],
    };
    const importResource = vi.fn();
    render(
      <LibraryNativeSources
        inventory={inventory}
        loading={false}
        failed={false}
        search=""
        tool="all"
        status="all"
        busy={false}
        onRescan={vi.fn()}
        onImport={importResource}
        onSource={vi.fn()}
        onLocate={vi.fn()}
        onCompare={vi.fn()}
      />,
    );
    const discovery = screen.getByRole("region", { name: "工具中发现" });
    expect(within(discovery).getByText("New resource")).toBeVisible();
    expect(
      within(discovery).getByText("Different configuration"),
    ).toBeVisible();
    expect(within(discovery).queryByText("Already added")).toBeNull();
    expect(
      within(discovery).queryByRole("button", { name: /已收录|已加入 ·/ }),
    ).toBeNull();
    expect(importResource).not.toHaveBeenCalled();
  },
);

it.each([
  ["zh", zh],
  ["en", en],
  ["ja", ja],
] as const)(
  "%s uses the Ofox product spelling without changing OFOX_ environment identifiers",
  (_, locale) => {
    const strings = (value: unknown): string[] =>
      typeof value === "string"
        ? [value]
        : value && typeof value === "object"
          ? Object.values(value).flatMap(strings)
          : [];
    expect(
      strings(locale).filter((value) => /\bOFox\b|\bOFOX\b/.test(value)),
    ).toEqual([]);
  },
);

it("calls the managed list My resources and separates adding from applying", () => {
  expect(zh.library.workbench.inLibrary).toBe("我的资源");
  expect(zh.library.interaction.collect).toBe("加入");
  expect(zh.library.interaction.savedLocated).toContain("已加入");
  expect(zh.library.interaction.savedLocated).toContain("预览应用");
});
