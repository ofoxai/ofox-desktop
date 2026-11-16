// Writes the frozen library inventory for this round from the files that are
// actually bundled: catalog.json, source-coverage.json, the source-preview
// evidence and the native adaptation sample manifest. Nothing is inferred from
// upstream READMEs. Run after the coverage manifest has been regenerated.
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { RESOURCES } from "./presentation-adapters.mjs";

const root = path.resolve(fileURLToPath(new URL("../", import.meta.url)));
const read = async (name) =>
  JSON.parse(await readFile(path.join(root, RESOURCES, name), "utf8"));

export async function libraryInventory() {
  const [catalog, coverage, samples] = await Promise.all([
    read("catalog.json"),
    read("source-coverage.json"),
    read("adaptation-previews.json"),
  ]);
  return catalog.styles.map((style) => {
    const source = catalog.sources.find((item) => item.id === style.sourceId);
    const record = coverage.styles.find((item) => item.id === style.id);
    const sample = samples.styles.find((item) => item.id === style.id);
    const deck = record.sourceDeck;
    const mapping = style.pageMapping || [];
    const count = (status) =>
      mapping.filter((page) => page.status === status).length;
    return {
      id: style.id,
      family: style.family ?? null,
      source: source.repository,
      revision: source.revision,
      upstream: source.upstream ?? null,
      directory: style.sourcePath,
      codeLicense: source.license,
      licenseFiles: style.licenseFiles.map((file) => file.path),
      thirdPartyAssets: {
        fonts: record.licenses.fonts.status,
        images: record.licenses.images.status,
        brands: record.licenses.brands.status,
        fontBinariesBundled: false,
        imageBinariesBundled: false,
        declaredFontFamilies: Object.values(style.typography || {}).filter(
          (value) => typeof value === "string" && value.length < 40,
        ),
      },
      resourceType:
        deck.kind === "html" ? "complete HTML deck" : "OfficeCLI build script",
      files: style.resources.map((file) => file.path),
      actualPages: deck.pageCount,
      pageKinds: [...new Set(mapping.map((page) => page.archetype))],
      runtimeDependencies: deck.dependencies.map(
        (dependency) => `${dependency.kind}: ${dependency.url}`,
      ),
      originalPreview: {
        status: deck.preview.status,
        renderedPages: deck.preview.pages.length,
        activation: deck.preview.renderer?.activationMode ?? "script-rebuild",
        findings: (deck.preview.visualFindings || []).map(
          (finding) => finding.code,
        ),
      },
      nativeAdaptation: {
        status: record.nativeAdaptation.status,
        adapted: count("adapted"),
        approximate: count("approximate"),
        unsupported: count("unsupported"),
        unsupportedPages: mapping
          .filter((page) => page.status === "unsupported")
          .map((page) => ({ page: page.page, why: page.differences })),
        samplePages: sample?.pages.length ?? 0,
        sampleLint: sample?.lint.length ?? null,
        engine:
          style.theme?.decor?.composition === "spec"
            ? "declarative composition (Python helper); base-canvas editorial fallback in the Node deck renderer"
            : `hand-written "${style.theme?.decor?.composition}" composition (Python and Node)`,
        coverageNote: style.recipe?.nativeCoverage ?? null,
      },
      discoverable: style.enabled === true && !!style.names && !!style.intro,
      selectable: style.enabled === true,
      autoRouting: style.enabled === true && !!style.routing,
      scenarios: style.scenarios,
      density: style.density,
      scheme: style.scheme,
      validation: style.validation,
    };
  });
}

const NOT_INCLUDED = `## Not included in this round

| Resource | Why it is not counted |
| --- | --- |
| \`zarazhangrui/beautiful-html-templates\` — the other 17 upstream templates (8-bit-orbit, biennale-yellow, coral, editorial-tri-tone, emerald-editorial, grove, long-table, mat, neo-grid-bold, peoples-platform, pin-and-paper, pink-script, playful, raw-grid, retro-windows, retro-zine, soft-editorial) | The upstream index lists 34; 17 were selected on 2026-11-10 as the de-duplicated candidate roster. The rest were never vendored or reviewed, so they have no pinned bytes, page count or licence check here. Adding them is a new intake, not part of this frozen list. |
| \`nexu-io/open-design\` \`design-templates/html-ppt-zhangzara-*\` and \`zarazhangrui/frontend-slides\` bold-template-pack | Mirrors or derivatives of beautiful-html-templates; counted once under the canonical source. |
| \`nexu-io/open-design\` html-ppt shared themes, animation modules and single-page layouts | Supporting package code for the six vendored full decks, not decks themselves. |
| \`iOfficeAI/OfficeCLI\` other morph-ppt styles (a local public copy has 51 style notes, 27 build scripts, 27 PPTX) | Only \`dark--architectural-plan\` was reviewed. The upstream index warns that scripts overlap and coordinates cannot be copied as-is; the sample PPTX binaries carry unreviewed fonts and images. |
| \`manalkaff/opendesign\` | A deck-making skill specification (workflow text). It contains no template deck; it is cited as a workflow reference only. |
| PPT Master, Presenton, PPTAgent | Reused for planning, slot-binding and review rules (see the workflow reuse record), not as template sources. Their templates, fonts and applications are not bundled. |
| frontend-slides (other content) | Style-preset descriptions without complete decks beyond the mirrored pack. |
| Closed-source ofox-agent | Never read or copied; capability split and workflow were the only reference. |
| Built-in \`navy\` | Ofox's own master (0 content pages, 3 layouts). It is selectable but is not an upstream template and has no source deck. |
`;

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const styles = await libraryInventory();
  const total = (pick) => styles.reduce((sum, style) => sum + pick(style), 0);
  const lines = [
    "# PPT style library inventory (frozen 2026-11-11)",
    "",
    "Generated by `scripts/presentation-library-inventory.mjs` from the bundled catalog, source coverage, preview evidence and native sample manifest. Page counts are the pages actually found in the pinned source files and actually rendered, not README figures.",
    "",
    `**Scope**: ${styles.length} public catalog styles plus the built-in \`navy\`. Actual source pages: **${total((s) => s.actualPages)}**; original pages rendered: **${total((s) => s.originalPreview.renderedPages)}**; native mapping: **${total((s) => s.nativeAdaptation.adapted)} adapted**, **${total((s) => s.nativeAdaptation.approximate)} approximate**, **${total((s) => s.nativeAdaptation.unsupported)} unsupported**; selectable: **${styles.filter((s) => s.selectable).length}**; with automatic routing data: **${styles.filter((s) => s.autoRouting).length}**.`,
    "",
    "Four different things are counted separately: upstream original pages, Ofox's native adaptation (page mapping and native sample pages), Ofox's own example for the built-in master, and files a user generates. A native sample page is built by Ofox's generator from Ofox's own sample text; it is never a picture of the upstream page. `adapted` means the same page kind and structure as native objects; `approximate` names a structural difference; `unsupported` pages are listed and not counted as adapted.",
    "",
    "Licences: the code/text licence below covers the vendored source files only. No font or image binary is bundled; fonts are replaced by system fonts, and upstream sample brands, people and figures are not reused. Font and image clearance stays `pending` for every style.",
    "",
    "| Style | Family | Source @ commit | Directory | Licence | Type | Pages | Original rendered | Adapted / approx. / unsupported | Native sample (pages, lint) | Selectable / routed |",
    "| --- | --- | --- | --- | --- | --- | ---: | ---: | --- | --- | --- |",
    ...styles.map(
      (s) =>
        `| \`${s.id}\` | ${s.family ?? "—"} | ${s.source.replace("https://github.com/", "")} @ \`${s.revision.slice(0, 12)}\` | \`${s.directory}\` | ${s.codeLicense} | ${s.resourceType} | ${s.actualPages} | ${s.originalPreview.renderedPages} | ${s.nativeAdaptation.adapted} / ${s.nativeAdaptation.approximate} / ${s.nativeAdaptation.unsupported} | ${s.nativeAdaptation.samplePages}, ${s.nativeAdaptation.sampleLint ?? "—"} | ${s.selectable ? "yes" : "no"} / ${s.autoRouting ? "yes" : "no"} |`,
    ),
    "",
    "## Per style: page kinds, dependencies, unsupported pages",
    "",
    ...styles.flatMap((s) => [
      `### ${s.id}`,
      "",
      `- Page kinds in the source: ${s.pageKinds.join(", ") || "not mapped"}.`,
      `- Scenarios: ${s.scenarios.join(", ")}; density ${s.density}; scheme ${s.scheme}.`,
      `- Declared source fonts (not bundled): ${s.thirdPartyAssets.declaredFontFamilies.join(", ") || "none declared"}.`,
      `- Source runtime dependencies (not fetched at run time): ${s.runtimeDependencies.join("; ") || "none"}.`,
      `- Original preview: ${s.originalPreview.status}, ${s.originalPreview.activation}${s.originalPreview.findings.length ? `; findings: ${s.originalPreview.findings.join(", ")}` : ""}.`,
      `- Native engine: ${s.nativeAdaptation.engine}.`,
      `- Native coverage note: ${s.nativeAdaptation.coverageNote ?? "—"}`,
      ...(s.nativeAdaptation.unsupportedPages.length
        ? s.nativeAdaptation.unsupportedPages.map(
            (page) => `- Unsupported source page ${page.page}: ${page.why}`,
          )
        : ["- Unsupported source pages: none listed."]),
      "",
    ]),
    NOT_INCLUDED,
  ];
  await writeFile(
    path.join(
      root,
      "docs/superpowers/research/2026-11-11-ppt-library-inventory.md",
    ),
    lines.join("\n"),
  );
  await writeFile(
    path.join(
      root,
      ".real-eval/ppt-library-completion-2026-11-11/library-inventory.json",
    ),
    JSON.stringify({ generated: "2026-11-11", styles }, null, 2) + "\n",
  );
  console.log(JSON.stringify({ styles: styles.length }));
}
