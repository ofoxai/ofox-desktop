import type { TFunction } from "i18next";
import type { AssistantPresentationStyleId } from "@/lib/api/assistant";
import catalog from "../../assistant-runtime/skills/ofox-office-pptx/resources/catalog.json";
import adaptations from "../../assistant-runtime/skills/ofox-office-pptx/resources/adaptation-previews.json";

export const PRESENTATION_SAMPLE_PAGE_KINDS = [
  "cover",
  "outline",
  "content",
  "comparison",
  "chart",
  "process",
  "equation",
  "closing",
] as const;
export type PresentationSamplePageKind =
  (typeof PRESENTATION_SAMPLE_PAGE_KINDS)[number];
/** One page of the built-in style's self-written example. */
export interface PresentationSamplePage {
  kind: PresentationSamplePageKind;
  src: string;
  thumbnail: string;
}

export type PresentationLanguage = "zh" | "en" | "ja";
type LocalizedText = Partial<Record<PresentationLanguage, string>>;
export type PresentationScheme = "light" | "dark" | "mixed";
export type PresentationMappingStatus =
  | "adapted"
  | "approximate"
  | "unsupported";

/** How one actual source page is carried by the native generator. */
export interface PresentationNativePage {
  sourcePage: number;
  archetype: string;
  status: PresentationMappingStatus;
  /** Upstream-vs-native difference as recorded in the catalog (English). */
  differences: string;
  /** Bundled native sample page; never set for an unsupported page. */
  src: string | null;
}
export interface PresentationMappingCounts {
  source: number;
  adapted: number;
  approximate: number;
  unsupported: number;
}
export interface PresentationStyle {
  id: string;
  /** The Ofox master, which has no upstream deck. */
  builtIn: boolean;
  family: string;
  scheme: PresentationScheme | null;
  scenarios: string[];
  /** Catalog text; absent until a style has been through the adapter step. */
  names: LocalizedText | null;
  intro: LocalizedText | null;
  /** Whether `scenarios` are routing ids rather than pre-adapter metadata. */
  routed: boolean;
  fallbackName: string;
  fallbackIntro: string;
  /** Card image: the first native sample page, or the built-in cover. */
  cover: string | null;
  /** The built-in style's self-written example; empty for every other style. */
  pages: ReadonlyArray<PresentationSamplePage>;
  /** One entry per actual source page, or null while nothing is mapped. */
  nativePages: ReadonlyArray<PresentationNativePage> | null;
  /** Whether a native adaptation sample has been published for this style. */
  nativeSample: boolean;
  counts: PresentationMappingCounts | null;
}

interface CatalogStyle {
  id: string;
  name?: string;
  description?: string;
  status?: string;
  enabled?: boolean;
  adapted?: boolean;
  licenseReviewed?: boolean;
  theme?: unknown;
  scenarios?: string[];
  scheme?: string;
  family?: string;
  names?: LocalizedText;
  intro?: LocalizedText;
  routing?: { scenarios?: string[]; scheme?: string };
  pageMapping?: {
    page: number;
    archetype?: string;
    status?: string;
    differences?: string;
  }[];
}
interface AdaptationRecord {
  id: string;
  sourcePages?: number;
  unsupportedPages?: number[];
  pages?: {
    sourcePage: number;
    archetype?: string;
    status?: string;
    path: string;
  }[];
}

export const BUILT_IN_PRESENTATION_STYLE = "navy";
const DEFAULT_FAMILY = "business";
/** Display order of the family filter; only families with styles are shown. */
const FAMILY_ORDER = [
  "editorial",
  "business",
  "poster",
  "color-block",
  "playful",
  "teaching",
  "technical",
];
const SCHEMES: readonly string[] = ["light", "dark", "mixed"];
const STATUS_RANK: Record<PresentationMappingStatus, number> = {
  adapted: 0,
  approximate: 1,
  unsupported: 2,
};

// Keep all sample bytes local and let Vite fingerprint them with the renderer.
// Only the built-in example is bundled from src/assets; every other style's
// pages come from the published native adaptation manifest.
const BUILT_IN_IMAGES = import.meta.glob<string>(
  "../assets/presentation-styles/navy-*.jpg",
  { eager: true, query: "?url", import: "default" },
);
const ADAPTATION_IMAGES = import.meta.glob<string>(
  "../../assistant-runtime/skills/ofox-office-pptx/resources/adaptation-previews/**/*.jpg",
  { eager: true, query: "?url", import: "default" },
);
const RESOURCES = "../../assistant-runtime/skills/ofox-office-pptx/resources/";

function builtInImage(kind: PresentationSamplePageKind, thumbnail = false) {
  const key = `../assets/presentation-styles/${BUILT_IN_PRESENTATION_STYLE}-${kind}${thumbnail ? "-thumb" : ""}.jpg`;
  const url = BUILT_IN_IMAGES[key];
  if (!url)
    throw new Error(
      `Missing bundled presentation sample: ${kind}${thumbnail ? "/thumbnail" : ""}`,
    );
  return url;
}

/** An unrecognised status is never shown as adapted. */
function mappingStatus(value: string | undefined): PresentationMappingStatus {
  return value === "adapted" || value === "approximate" ? value : "unsupported";
}

function nativePages(
  style: CatalogStyle,
  record: AdaptationRecord | undefined,
): PresentationNativePage[] | null {
  const mapping = Array.isArray(style.pageMapping) ? style.pageMapping : null;
  if (!mapping && !record) return null;
  const rendered = record?.pages ?? [];
  const refused = record?.unsupportedPages ?? [];
  const numbers = mapping
    ? mapping.map((page) => page.page)
    : [...rendered.map((page) => page.sourcePage), ...refused];
  return [...new Set(numbers)]
    .sort((a, b) => a - b)
    .map((sourcePage) => {
      const mapped = mapping?.find((page) => page.page === sourcePage);
      const sample = rendered.find((page) => page.sourcePage === sourcePage);
      // The catalog and the rendered sample may disagree while both are being
      // regenerated: the more cautious claim wins.
      const claims = [
        ...(mapped ? [mappingStatus(mapped.status)] : []),
        ...(sample ? [mappingStatus(sample.status)] : []),
        ...(refused.includes(sourcePage) ? ["unsupported" as const] : []),
      ];
      const status = claims.reduce<PresentationMappingStatus>(
        (worst, claim) =>
          STATUS_RANK[claim] > STATUS_RANK[worst] ? claim : worst,
        "adapted",
      );
      return {
        sourcePage,
        archetype: mapped?.archetype || sample?.archetype || "",
        status,
        differences: mapped?.differences?.trim() || "",
        src:
          status !== "unsupported" && sample
            ? ADAPTATION_IMAGES[RESOURCES + sample.path] || null
            : null,
      };
    });
}

function mappingCounts(
  pages: PresentationNativePage[] | null,
  record: AdaptationRecord | undefined,
  mapped: boolean,
): PresentationMappingCounts | null {
  if (!pages) return null;
  const count = (status: PresentationMappingStatus) =>
    pages.filter((page) => page.status === status).length;
  return {
    source: mapped
      ? pages.length
      : Math.max(record?.sourcePages ?? 0, pages.length),
    adapted: count("adapted"),
    approximate: count("approximate"),
    unsupported: count("unsupported"),
  };
}

function builtInStyle(): PresentationStyle {
  return {
    id: BUILT_IN_PRESENTATION_STYLE,
    builtIn: true,
    family: DEFAULT_FAMILY,
    scheme: null,
    scenarios: ["general"],
    names: null,
    intro: null,
    routed: false,
    fallbackName: BUILT_IN_PRESENTATION_STYLE,
    fallbackIntro: "",
    cover: builtInImage("cover"),
    pages: PRESENTATION_SAMPLE_PAGE_KINDS.map((kind) => ({
      kind,
      src: builtInImage(kind),
      thumbnail: builtInImage(kind, true),
    })),
    nativePages: null,
    nativeSample: false,
    counts: null,
  };
}

const CATALOG_STYLES = (catalog as unknown as { styles: CatalogStyle[] })
  .styles;
const ADAPTATION_RECORDS = (
  adaptations as unknown as { styles: AdaptationRecord[] }
).styles;

/** The built-in master first, then every selectable catalog style. */
export const PRESENTATION_STYLES: ReadonlyArray<PresentationStyle> = [
  builtInStyle(),
  ...CATALOG_STYLES.filter(
    (style) =>
      style.id !== BUILT_IN_PRESENTATION_STYLE &&
      style.id !== "auto" &&
      style.status === "enabled" &&
      style.enabled &&
      style.adapted &&
      style.licenseReviewed &&
      style.theme,
  ).map((style): PresentationStyle => {
    const record = ADAPTATION_RECORDS.find((item) => item.id === style.id);
    const pages = nativePages(style, record);
    const scheme = style.routing?.scheme ?? style.scheme;
    return {
      id: style.id,
      builtIn: false,
      family: style.family || DEFAULT_FAMILY,
      scheme:
        scheme && SCHEMES.includes(scheme)
          ? (scheme as PresentationScheme)
          : null,
      scenarios: style.routing?.scenarios ?? style.scenarios ?? [],
      names: style.names ?? null,
      intro: style.intro ?? null,
      routed: Boolean(style.routing?.scenarios),
      fallbackName: style.name || style.id,
      fallbackIntro: style.description || "",
      cover: pages?.find((page) => page.src)?.src ?? null,
      pages: [],
      nativePages: pages,
      nativeSample: Boolean(record),
      counts: mappingCounts(pages, record, Array.isArray(style.pageMapping)),
    };
  }),
];

/** Families that have at least one style, in their display order. */
export const PRESENTATION_FAMILIES: ReadonlyArray<string> = [
  ...FAMILY_ORDER,
  ...PRESENTATION_STYLES.map((style) => style.family),
].filter(
  (family, index, all) =>
    all.indexOf(family) === index &&
    PRESENTATION_STYLES.some((style) => style.family === family),
);

export function presentationStyle(id: unknown): PresentationStyle | undefined {
  return PRESENTATION_STYLES.find((style) => style.id === id);
}

/** Catalog text exists in zh/en/ja; every other UI language reads English. */
export function presentationLanguage(
  language: string | undefined,
): PresentationLanguage {
  const base = (language || "").toLowerCase().split(/[-_]/)[0];
  return base === "zh" || base === "ja" ? base : "en";
}

export function presentationStyleName(
  id: AssistantPresentationStyleId,
  t: TFunction,
  language: string | undefined,
): string {
  if (id === "auto") return t("assistant.presentation.auto");
  const style = presentationStyle(id);
  return (
    style?.names?.[presentationLanguage(language)] ||
    style?.names?.en ||
    t(`assistant.presentation.names.${id}`, {
      defaultValue: style?.fallbackName ?? id,
    })
  );
}

export function presentationStyleIntro(
  style: PresentationStyle,
  t: TFunction,
  language: string | undefined,
): string {
  return (
    style.intro?.[presentationLanguage(language)] ||
    style.intro?.en ||
    t(`assistant.presentation.descriptions.${style.id}`, {
      defaultValue: style.fallbackIntro,
    })
  );
}

export function presentationFamilyName(family: string, t: TFunction): string {
  return t(`assistant.presentation.families.${family}`, {
    defaultValue: family,
  });
}

export function presentationArchetypeName(
  archetype: string,
  t: TFunction,
): string {
  return t(`assistant.presentation.archetypes.${archetype}`, {
    defaultValue: archetype,
  });
}

/** Scenario labels in the UI language, joined for a card or for search. */
export function presentationStyleScenarios(
  style: PresentationStyle,
  t: TFunction,
): string {
  const labels = style.scenarios
    .map((id) =>
      t(`assistant.presentation.scenarioLabels.${id}`, { defaultValue: id }),
    )
    .join(" · ");
  // Styles that predate the adapter step keep their hand-written line.
  return style.routed
    ? labels
    : t(`assistant.presentation.scenarios.${style.id}`, {
        defaultValue: labels,
      });
}

/** Historical/imported metadata is displayable only while its choice is known. */
export function isPresentationStyleAvailable(
  value: unknown,
): value is AssistantPresentationStyleId {
  return value === "auto" || presentationStyle(value) !== undefined;
}
