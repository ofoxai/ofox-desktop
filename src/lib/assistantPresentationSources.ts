import {
  PRESENTATION_STYLES,
  type PresentationMappingCounts,
} from "@/lib/assistantPresentationStyles";
import manifest from "../../assistant-runtime/skills/ofox-office-pptx/resources/source-coverage.json";

export interface PresentationSourcePage {
  number: number;
  label: string;
  src: string | null;
}
export interface PresentationSource {
  id: string;
  kind: "html" | "officecli-script" | "builtin-master";
  pageCount: number;
  layouts: string[];
  /** `adapted` only when every actual source page is mapped as adapted. */
  nativeStatus: "builtin" | "adapted" | "partial";
  /** Real page-mapping counts; null while the style has no page mapping. */
  nativeCounts: PresentationMappingCounts | null;
  pages: PresentationSourcePage[];
  sourceName: string;
  revision: string | null;
  sourceUrl: string | null;
  licenses: { name: string; url: string }[];
  /** Dependencies pinned only to render this deck's offline preview. */
  dependencies: { name: string; version: string }[];
  findings: { code: string; pages: number[] }[];
}

// Only Vite's bundled, fingerprinted assets reach the UI. Neither original
// filesystem paths nor executable source HTML are exposed to the viewer.
const images = import.meta.glob<string>(
  "../../assistant-runtime/skills/ofox-office-pptx/resources/source-previews/**/*.jpg",
  { eager: true, query: "?url", import: "default" },
);
const assetPrefix =
  "../../assistant-runtime/skills/ofox-office-pptx/resources/";
const githubUrl = (
  repository: string,
  revision: string,
  sourcePath: string,
  tree = false,
) => {
  if (
    !/^https:\/\/github\.com\/[a-zA-Z0-9_.-]+\/[a-zA-Z0-9_.-]+$/.test(
      repository,
    ) ||
    !/^[a-f0-9]{40}$/.test(revision) ||
    sourcePath.split("/").some((part) => part === "..")
  )
    return null;
  return `${repository}/${tree ? "tree" : "blob"}/${revision}/${sourcePath.split("/").map(encodeURIComponent).join("/")}`;
};

interface CoverageFile {
  path: string;
  sourcePath?: string;
  sourceUrl?: string;
}
interface CoverageComponent {
  name: string;
  version: string;
  license: string;
  repository: string;
  revision: string;
  licenseFile?: string;
}
interface CoverageDependency extends Omit<CoverageComponent, "name"> {
  id: string;
  files?: CoverageFile[];
  bundledComponents?: CoverageComponent[];
}
interface CoverageStyle {
  id: string;
  sourceId: string | null;
  sourceDeck: {
    kind: string;
    pageCount: number;
    pages: { number: number; label: string }[];
    layouts: { label: string }[];
    files: CoverageFile[];
    preview: {
      pages?: { number: number; path: string }[];
      dependencyOverrides?: CoverageDependency[];
      visualFindings?: { code: string; pages: number[] }[];
    };
  };
}
interface CoverageSource {
  id: string;
  repository: string;
  revision: string;
  license: string;
  licenseFiles: CoverageFile[];
}
const coverage = manifest as unknown as {
  styles: CoverageStyle[];
  sources: CoverageSource[];
};

/** The public licence page of a pinned dependency, from its recorded files. */
function dependencyLicenseUrl(
  item: Omit<CoverageComponent, "name">,
  files: CoverageFile[],
) {
  const raw = files
    .find((file) => file.path === item.licenseFile)
    ?.sourceUrl?.match(
      /^https:\/\/raw\.githubusercontent\.com\/([^/]+\/[^/]+)\/([a-f0-9]{40})\/(.+)$/,
    );
  return raw && item.repository === `https://github.com/${raw[1]}`
    ? githubUrl(item.repository, raw[2], raw[3])
    : githubUrl(item.repository, item.revision, "LICENSE.md");
}

// The catalog decides which styles are selectable; coverage only describes
// their upstream deck. A page that has no rendered original keeps a null
// image, and nothing else is shown in its place.
export const PRESENTATION_SOURCES: ReadonlyArray<PresentationSource> =
  PRESENTATION_STYLES.flatMap((available) => {
    const entry = coverage.styles.find((style) => style.id === available.id);
    if (!entry) return [];
    const source = coverage.sources.find((item) => item.id === entry.sourceId);
    const sourceFile = entry.sourceDeck.files.find(
      (file) => file.sourcePath && /\.(html|sh)$/.test(file.path),
    );
    const sourceDirectory =
      sourceFile?.sourcePath?.split("/").slice(0, -1).join("/") ?? "";
    const licenses: PresentationSource["licenses"] = [];
    if (source)
      for (const file of source.licenseFiles) {
        const sourcePath = file.sourcePath ?? "";
        const url =
          sourcePath &&
          githubUrl(source.repository, source.revision, sourcePath);
        if (url)
          licenses.push({
            name: `${source.license} · ${source.id}${sourcePath.endsWith("NOTICE") ? " NOTICE" : ""}`,
            url,
          });
      }
    const preview = entry.sourceDeck.preview;
    const dependencies: PresentationSource["dependencies"] = [];
    for (const dependency of preview.dependencyOverrides ?? []) {
      const files = dependency.files ?? [];
      const name = dependency.repository.split("/").pop() || dependency.id;
      dependencies.push({ name, version: dependency.version });
      for (const item of [
        { ...dependency, name },
        ...(dependency.bundledComponents ?? []),
      ]) {
        const url = dependencyLicenseUrl(item, files);
        if (url)
          licenses.push({
            name: `${item.name} ${item.version} · ${item.license}`,
            url,
          });
      }
    }
    const rendered = preview.pages ?? [];
    const counts = available.counts;
    return [
      {
        id: available.id,
        kind: entry.sourceDeck.kind as PresentationSource["kind"],
        pageCount: entry.sourceDeck.pageCount,
        layouts: entry.sourceDeck.layouts.map((layout) => layout.label),
        nativeStatus: available.builtIn
          ? "builtin"
          : counts && counts.source > 0 && counts.adapted === counts.source
            ? "adapted"
            : "partial",
        nativeCounts: counts,
        pages: entry.sourceDeck.pages.map((page) => {
          const raster = rendered.find((item) => item.number === page.number);
          return {
            number: page.number,
            label: page.label,
            src: raster ? images[assetPrefix + raster.path] || null : null,
          };
        }),
        sourceName: source?.id || "Ofox",
        revision: source?.revision || null,
        sourceUrl: source
          ? githubUrl(source.repository, source.revision, sourceDirectory, true)
          : null,
        licenses,
        dependencies,
        findings: (preview.visualFindings ?? []).map((finding) => ({
          code: finding.code,
          pages: finding.pages,
        })),
      },
    ];
  });

export function presentationSource(id: string): PresentationSource | undefined {
  return PRESENTATION_SOURCES.find((source) => source.id === id);
}
