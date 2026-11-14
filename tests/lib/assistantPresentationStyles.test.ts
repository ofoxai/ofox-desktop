import { describe, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import catalog from "../../assistant-runtime/skills/ofox-office-pptx/resources/catalog.json";
import adaptations from "../../assistant-runtime/skills/ofox-office-pptx/resources/adaptation-previews.json";
import {
  PRESENTATION_FAMILIES,
  PRESENTATION_STYLES,
  isPresentationStyleAvailable,
  presentationArchetypeName,
  presentationFamilyName,
  presentationLanguage,
  presentationStyle,
  presentationStyleIntro,
  presentationStyleName,
  presentationStyleScenarios,
} from "@/lib/assistantPresentationStyles";

interface CatalogStyle {
  id: string;
  status?: string;
  enabled?: boolean;
  adapted?: boolean;
  licenseReviewed?: boolean;
  theme?: unknown;
  family?: string;
  names?: Record<string, string>;
  intro?: Record<string, string>;
  routing?: { scenarios?: string[]; scheme?: string };
  pageMapping?: { page: number; archetype: string; status: string }[];
}
interface AdaptationRecord {
  id: string;
  unsupportedPages: number[];
  pages: { sourcePage: number; status: string; path: string }[];
}
const styles = (catalog as unknown as { styles: CatalogStyle[] }).styles;
const records = (adaptations as unknown as { styles: AdaptationRecord[] })
  .styles;
const enabled = styles.filter(
  (style) =>
    style.status === "enabled" &&
    style.enabled &&
    style.adapted &&
    style.licenseReviewed &&
    style.theme,
);
const LOCALES = { zh, en, ja } as const;
const languages = ["zh", "en", "ja"] as const;
const presentation = (locale: (typeof LOCALES)[keyof typeof LOCALES]) =>
  locale.assistant.presentation as unknown as Record<
    string,
    Record<string, string>
  >;

async function translator(language: string) {
  for (const [code, bundle] of Object.entries(LOCALES))
    i18n.addResourceBundle(code, "translation", bundle, true, true);
  await i18n.changeLanguage(language);
  return i18n.getFixedT(language);
}

describe("the visible presentation catalog", () => {
  it("lists the built-in master first, then exactly the catalog's selectable styles", () => {
    expect(PRESENTATION_STYLES[0].id).toBe("navy");
    expect(PRESENTATION_STYLES[0].builtIn).toBe(true);
    expect(PRESENTATION_STYLES.slice(1).map((style) => style.id)).toEqual(
      enabled.map((style) => style.id),
    );
    expect(enabled.length).toBeGreaterThan(0);
    for (const style of PRESENTATION_STYLES.slice(1))
      expect(style.builtIn).toBe(false);
  });

  it("never treats an unknown, historical, candidate or malformed id as available", () => {
    for (const style of PRESENTATION_STYLES)
      expect(isPresentationStyleAvailable(style.id)).toBe(true);
    expect(isPresentationStyleAvailable("auto")).toBe(true);
    const candidates = styles.filter((style) => !enabled.includes(style));
    for (const style of candidates)
      expect(isPresentationStyleAvailable(style.id)).toBe(false);
    for (const value of [
      "unknown-imported-style",
      "",
      "../navy",
      "Navy",
      undefined,
      null,
      7,
      {},
    ])
      expect(isPresentationStyleAvailable(value)).toBe(false);
  });

  it.each(languages)(
    "gives every selectable style a non-empty localized name, intro and scenario line in %s",
    async (language) => {
      const t = await translator(language);
      for (const style of PRESENTATION_STYLES) {
        const name = presentationStyleName(style.id, t, language);
        const intro = presentationStyleIntro(style, t, language);
        const scenarios = presentationStyleScenarios(style, t);
        for (const text of [name, intro, scenarios]) {
          expect(text.trim(), `${style.id}/${language}`).not.toBe("");
          expect(text, `${style.id}/${language}`).not.toContain(
            "assistant.presentation.",
          );
        }
        const source = styles.find((item) => item.id === style.id);
        if (source?.names) expect(name).toBe(source.names[language]);
        if (source?.intro) expect(intro).toBe(source.intro[language]);
        expect(presentationFamilyName(style.family, t)).toBe(
          presentation(LOCALES[language]).families[style.family],
        );
      }
    },
  );

  it("reads catalog text in English for any other UI language and keeps the built-in strings", async () => {
    expect(presentationLanguage("zh-CN")).toBe("zh");
    expect(presentationLanguage("ja")).toBe("ja");
    expect(presentationLanguage("fr")).toBe("en");
    expect(presentationLanguage(undefined)).toBe("en");
    const t = await translator("en");
    const named = styles.find(
      (style) => enabled.includes(style) && style.names,
    );
    if (named)
      expect(presentationStyleName(named.id, t, "fr")).toBe(named.names!.en);
    expect(presentationStyleName("navy", t, "en")).toBe(
      en.assistant.presentation.names.navy,
    );
    expect(presentationStyleName("auto", t, "en")).toBe(
      en.assistant.presentation.auto,
    );
    const zhT = await translator("zh");
    expect(presentationStyleName("navy", zhT, "zh")).toBe(
      zh.assistant.presentation.names.navy,
    );
    expect(presentationStyleIntro(presentationStyle("navy")!, zhT, "zh")).toBe(
      zh.assistant.presentation.descriptions.navy,
    );
  });

  it("has a label in every language for each family, scenario, scheme, page kind and mapping status in the catalog", () => {
    const families = new Set(PRESENTATION_STYLES.map((style) => style.family));
    const scenarios = new Set<string>();
    const archetypes = new Set<string>();
    for (const style of styles) {
      for (const id of style.routing?.scenarios ?? []) scenarios.add(id);
      for (const page of style.pageMapping ?? [])
        archetypes.add(page.archetype);
    }
    for (const language of languages) {
      const text = presentation(LOCALES[language]);
      for (const id of families) expect(text.families[id], id).toBeTruthy();
      for (const id of scenarios)
        expect(text.scenarioLabels[id], id).toBeTruthy();
      for (const id of archetypes) expect(text.archetypes[id], id).toBeTruthy();
      for (const id of ["light", "dark", "mixed"])
        expect(text.schemes[id], id).toBeTruthy();
      for (const id of ["adapted", "approximate", "unsupported"])
        expect(text.mappingStatus[id], id).toBeTruthy();
      expect(Object.keys(text.families).sort()).toEqual(
        Object.keys(presentation(en).families).sort(),
      );
      expect(Object.keys(text.archetypes).sort()).toEqual(
        Object.keys(presentation(en).archetypes).sort(),
      );
      expect(Object.keys(text.scenarioLabels).sort()).toEqual(
        Object.keys(presentation(en).scenarioLabels).sort(),
      );
    }
  });

  it("offers only families that have styles and falls back to business for unclassified ones", async () => {
    expect([...PRESENTATION_FAMILIES].sort()).toEqual(
      [...new Set(PRESENTATION_STYLES.map((style) => style.family))].sort(),
    );
    for (const style of PRESENTATION_STYLES.slice(1))
      expect(style.family).toBe(
        styles.find((item) => item.id === style.id)?.family ?? "business",
      );
    expect(presentationStyle("navy")!.family).toBe("business");
    const t = await translator("en");
    expect(presentationArchetypeName("not-a-kind", t)).toBe("not-a-kind");
  });

  it("bundles only the built-in example from src/assets and gives it all eight self-written pages", () => {
    const expected = [
      "cover",
      "outline",
      "content",
      "comparison",
      "chart",
      "process",
      "equation",
      "closing",
    ];
    const navy = presentationStyle("navy")!;
    expect(navy.pages.map((page) => page.kind)).toEqual(expected);
    for (const page of navy.pages) {
      expect(page.src).toMatch(new RegExp(`navy-${page.kind}\\.jpg`));
      expect(page.thumbnail).toMatch(
        new RegExp(`navy-${page.kind}-thumb\\.jpg`),
      );
    }
    expect(navy.cover).toMatch(/navy-cover\.jpg/);
    expect(navy.nativePages).toBeNull();
    expect(navy.counts).toBeNull();
    for (const style of PRESENTATION_STYLES.slice(1)) {
      expect(style.pages).toEqual([]);
      expect(style.cover ?? "").not.toContain("assets/presentation-styles");
    }
  });

  it("reports real mapping counts and never gives an unsupported source page a native image", () => {
    for (const style of PRESENTATION_STYLES.slice(1)) {
      const source = styles.find((item) => item.id === style.id)!;
      const record = records.find((item) => item.id === style.id);
      expect(style.nativeSample).toBe(Boolean(record));
      if (!source.pageMapping && !record) {
        expect(style.nativePages).toBeNull();
        expect(style.counts).toBeNull();
        expect(style.cover).toBeNull();
        continue;
      }
      const pages = style.nativePages!;
      const counts = style.counts!;
      expect(counts.adapted + counts.approximate + counts.unsupported).toBe(
        pages.length,
      );
      if (source.pageMapping) {
        expect(counts.source).toBe(source.pageMapping.length);
        expect(pages.map((page) => page.sourcePage)).toEqual(
          source.pageMapping.map((page) => page.page).sort((a, b) => a - b),
        );
        // Never more optimistic than the catalog's own mapping.
        for (const mapped of source.pageMapping) {
          const page = pages.find((item) => item.sourcePage === mapped.page)!;
          if (mapped.status !== "adapted")
            expect(page.status).not.toBe("adapted");
          if (mapped.status === "unsupported")
            expect(page.status).toBe("unsupported");
        }
      }
      for (const number of record?.unsupportedPages ?? [])
        expect(pages.find((page) => page.sourcePage === number)?.status).toBe(
          "unsupported",
        );
      for (const page of pages) {
        if (page.status === "unsupported") expect(page.src).toBeNull();
        if (page.src) {
          expect(page.src).toContain(`adaptation-previews/${style.id}/page-`);
          expect(page.src).not.toMatch(/file:|\/Users\//);
        }
        if (!record) expect(page.src).toBeNull();
      }
      if (record)
        expect(style.cover).toBe(pages.find((page) => page.src)?.src ?? null);
    }
  });
});
