import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import { isDeepStrictEqual } from "node:util";
import { JSDOM } from "jsdom";

const ROOT = path.resolve(fileURLToPath(new URL("../", import.meta.url)));
export const RESOURCE_PATH =
  "assistant-runtime/skills/ofox-office-pptx/resources";
const ORIGINAL_INVENTORY_HASH =
  "eecb37a4c4d3a50821c6d3b340635bc5c04c64b9f85c600dc689c4c8c1bdbcb6";
const PREVIEW_DEPENDENCIES_HASH =
  "e249bc54e8bb1a6b54735a3e359832f0cd47c462d93ae2ae816c7a46a2aa0fb6";
const PINNED = {
  "beautiful-html-templates": [
    "e5e204fb1f3b06290846e7dcd7aceddabeceec8c",
    "MIT",
  ],
  "open-design-html-ppt": ["07a86b1041531bfa0bec1006d9c0993985fee560", "MIT"],
  officecli: ["c56512c8c7b76fe284f09940e4cd492470099d3e", "Apache-2.0"],
};
export const hashBytes = (bytes) =>
  createHash("sha256").update(bytes).digest("hex");

export function assertOriginalBytes(bytes, resource) {
  if (hashBytes(bytes) !== resource.sha256)
    throw new Error(`Original source hash mismatch: ${resource.path}`);
}

export function jpegSize(bytes) {
  for (let offset = 2; offset + 8 < bytes.length; ) {
    if (bytes[offset++] !== 0xff) throw new Error("Invalid JPEG marker");
    while (bytes[offset] === 0xff) offset++;
    const marker = bytes[offset++];
    if ([0xc0, 0xc1, 0xc2].includes(marker))
      return [bytes.readUInt16BE(offset + 5), bytes.readUInt16BE(offset + 3)];
    if (marker === 0xd9 || marker === 0xda) break;
    const length = bytes.readUInt16BE(offset);
    if (length < 2) break;
    offset += length;
  }
  throw new Error("JPEG dimensions unavailable");
}

function safePath(base, relative) {
  if (typeof relative !== "string" || path.isAbsolute(relative))
    throw new Error("Invalid relative resource path");
  const resolved = path.resolve(base, relative);
  if (!resolved.startsWith(base + path.sep))
    throw new Error(`Resource path escapes root: ${relative}`);
  return resolved;
}

const lineAt = (text, offset) => text.slice(0, offset).split("\n").length;

async function htmlDeck(base, file) {
  const html = await readFile(safePath(base, file.path), "utf8");
  // Parsing only: jsdom never runs upstream scripts or fetches linked resources.
  const dom = new JSDOM(html, { includeNodeLocations: true });
  const document = dom.window.document;
  const selector = document.querySelector("deck-stage")
    ? "deck-stage > section"
    : document.querySelector(".deck > .slide")
      ? ".deck > .slide"
      : document.querySelector(".presentation > .slide")
        ? ".presentation > .slide"
        : ".slide";
  const nodes = [...document.querySelectorAll(selector)];
  if (!nodes.length)
    throw new Error(`No original pages detected: ${file.path}`);
  const pages = nodes.map((node, index) => ({
    id: `page-${String(index + 1).padStart(2, "0")}`,
    number: index + 1,
    label: (
      node.getAttribute("data-title") ||
      node.getAttribute("data-screen-label") ||
      node.querySelector("h1,h2,h3,blockquote")?.textContent ||
      node.className ||
      `Page ${index + 1}`
    )
      .replace(/\s+/g, " ")
      .trim()
      .slice(0, 180),
    layout: node.className.replace(/\b(active|is-active)\b/g, "").trim(),
    sourcePath: file.path,
    line: dom.nodeLocation(node).startLine,
    selector,
    index,
  }));
  const dependencies = [];
  const queue = [];
  const visited = new Set([file.path]);
  function addDependency(value, kind, from, line) {
    if (!value || value.startsWith("#") || value.startsWith("data:")) return;
    if (/^https?:\/\//.test(value)) {
      dependencies.push({
        kind: /fonts\.(googleapis|gstatic)\.com/.test(value)
          ? "remote-font"
          : kind,
        url: value,
        from,
        line,
        status: "not-fetched",
      });
      return;
    }
    if (/^[a-z]+:/i.test(value) || value.startsWith("//"))
      throw new Error(`Unsupported source dependency: ${value}`);
    const resolved = path.posix.normalize(
      path.posix.join(path.posix.dirname(from), value.split(/[?#]/)[0]),
    );
    safePath(base, resolved);
    dependencies.push({ kind, path: resolved, from, line, status: "local" });
    if (!visited.has(resolved)) {
      visited.add(resolved);
      queue.push(resolved);
    }
  }
  for (const node of document.querySelectorAll(
    "script[src],link[href],img[src],iframe[src]",
  )) {
    if (
      node.tagName === "LINK" &&
      !["stylesheet", "preconnect"].includes(node.getAttribute("rel"))
    )
      continue;
    const kind =
      node.tagName === "SCRIPT"
        ? "script"
        : node.tagName === "LINK"
          ? node.getAttribute("rel")
          : "image";
    addDependency(
      node.getAttribute("src") || node.getAttribute("href"),
      kind,
      file.path,
      dom.nodeLocation(node).startLine,
    );
  }
  const inline = [...document.querySelectorAll("style,script:not([src])")].map(
    (node) => ({
      kind: node.tagName === "STYLE" ? "inline-css" : "inline-script",
      sourcePath: file.path,
      line: dom.nodeLocation(node).startLine,
      sha256: hashBytes(node.textContent),
    }),
  );
  const dependencyFiles = [];
  while (queue.length) {
    const relative = queue.shift();
    const bytes = await readFile(safePath(base, relative));
    dependencyFiles.push({ path: relative, sha256: hashBytes(bytes) });
    if (relative.endsWith(".css")) {
      const css = bytes.toString("utf8");
      for (const match of css.matchAll(/url\(\s*['"]?([^'"\s)]+)['"]?\s*\)/g))
        addDependency(
          match[1],
          "css-resource",
          relative,
          lineAt(css, match.index),
        );
    }
  }
  dom.window.close();
  return {
    kind: "html",
    pageCount: pages.length,
    layoutCount: 0,
    pages,
    layouts: [],
    inline,
    dependencies,
    dependencyFiles,
  };
}

async function scriptDeck(base, file) {
  const text = await readFile(safePath(base, file.path), "utf8");
  const calls = [...text.matchAll(/^sl\s+--prop\s+background=([^\n]+)/gm)];
  const headings = [...text.matchAll(/^# SLIDE (\d+)\s*[—-]\s*(.+)/gm)];
  if (calls.length !== 6 || headings.length !== 6)
    throw new Error("Architectural source slide structure changed");
  return {
    kind: "officecli-script",
    pageCount: calls.length,
    layoutCount: 0,
    pages: calls.map((call, index) => ({
      id: `page-${String(index + 1).padStart(2, "0")}`,
      number: index + 1,
      label: headings[index][2].trim(),
      layout:
        index === 5
          ? "dark-closing"
          : index % 2 === 0
            ? "content-left-panel-right"
            : "content-right-panel-left",
      sourcePath: file.path,
      line: lineAt(text, call.index),
      selector: null,
      index,
    })),
    layouts: [],
    inline: [],
    dependencyFiles: [],
    dependencies: [
      {
        kind: "executable",
        name: "officecli",
        status: "execution-not-required-for-audit",
      },
      {
        kind: "font",
        name: "OfficeCLI default/local fallback; no explicit font family in script",
        status: "not-embedded",
      },
    ],
    cautions: [
      "style.md page names and palette differ from actual build.sh; script is the page inventory authority.",
      "Original image regions are native shape placeholders, not downloaded architecture photographs.",
      "Related AionCore index warns against verbatim coordinate reuse for different content.",
    ],
  };
}

async function builtinDeck(root) {
  const relative = "assistant-runtime/src/python/template.pptx";
  const bytes = await readFile(safePath(root, relative));
  const JSZip = createRequire(import.meta.resolve("pptxgenjs"))("jszip");
  const zip = await JSZip.loadAsync(bytes);
  const slideNames = Object.keys(zip.files).filter((name) =>
    /^ppt\/slides\/slide\d+\.xml$/.test(name),
  );
  const layoutNames = Object.keys(zip.files)
    .filter((name) => /^ppt\/slideLayouts\/slideLayout\d+\.xml$/.test(name))
    .sort();
  const layouts = [];
  for (const name of layoutNames) {
    const xml = await zip.file(name).async("string");
    layouts.push({
      id: name,
      label: /<p:cSld\b[^>]*\bname="([^"]+)"/.exec(xml)?.[1] || "Unnamed",
    });
  }
  return {
    kind: "builtin-master",
    pageCount: slideNames.length,
    layoutCount: layouts.length,
    pages: [],
    layouts,
    files: [
      { path: relative, sha256: hashBytes(bytes), relativeTo: "repository" },
    ],
    dependencies: [
      {
        kind: "font",
        name: "Microsoft YaHei / system fallback",
        status: "not-embedded",
      },
    ],
    sourceGenerator: "scripts/assistant-deck-template.mjs",
  };
}

async function previewEvidence(base) {
  try {
    return JSON.parse(
      await readFile(path.join(base, "source-preview-evidence.json"), "utf8"),
    );
  } catch (error) {
    if (error.code === "ENOENT") return { styles: [] };
    throw error;
  }
}

export async function loadPreviewDependencies(root = ROOT) {
  const base = path.join(root, RESOURCE_PATH);
  const value = JSON.parse(
    await readFile(path.join(base, "preview-dependencies.json"), "utf8"),
  );
  if (hashBytes(JSON.stringify(value)) !== PREVIEW_DEPENDENCIES_HASH)
    throw new Error("Unreviewed preview dependency metadata");
  for (const dependency of value.dependencies)
    for (const file of dependency.files)
      assertOriginalBytes(await readFile(safePath(base, file.path)), file);
  return value.dependencies;
}

export async function buildCoverage(root = ROOT, { sourceOnly = false } = {}) {
  const base = path.join(root, RESOURCE_PATH);
  const catalog = JSON.parse(
    await readFile(path.join(base, "catalog.json"), "utf8"),
  );
  const originals = new Map();
  let originalBytes = 0;
  for (const owner of [...catalog.sources, ...catalog.styles])
    for (const file of [
      ...(owner.resources || []),
      ...(owner.licenseFiles || []),
    ]) {
      if (
        originals.has(file.path) &&
        originals.get(file.path).sha256 !== file.sha256
      )
        throw new Error(`Conflicting original hashes: ${file.path}`);
      originals.set(file.path, file);
    }
  for (const file of originals.values()) {
    const bytes = await readFile(safePath(base, file.path));
    assertOriginalBytes(bytes, file);
    originalBytes += bytes.length;
  }
  if (originals.size !== 143)
    throw new Error("Pinned original inventory must contain 143 unique files");
  const inventoryHash = hashBytes(
    JSON.stringify(
      [...originals]
        .map(([name, file]) => [name, file.sha256])
        .sort(([a], [b]) => (a === b ? 0 : a < b ? -1 : 1)),
    ),
  );
  if (inventoryHash !== ORIGINAL_INVENTORY_HASH)
    throw new Error(
      "Pinned original inventory changed; reviewing a new upstream revision is required",
    );
  const sources = catalog.sources.map((source) => {
    if (
      !PINNED[source.id] ||
      !isDeepStrictEqual(PINNED[source.id], [source.revision, source.license])
    )
      throw new Error(`Unreviewed source revision/license: ${source.id}`);
    return {
      id: source.id,
      repository: source.repository,
      revision: source.revision,
      license: source.license,
      licenseFiles: source.licenseFiles,
      scope: source.licenseScope,
    };
  });
  const renders = sourceOnly ? { styles: [] } : await previewEvidence(base);
  let samples = { styles: [] };
  if (!sourceOnly)
    try {
      samples = JSON.parse(
        await readFile(path.join(base, "adaptation-previews.json"), "utf8"),
      );
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  const previewDependencies = await loadPreviewDependencies(root);
  if (
    !Array.isArray(renders.styles) ||
    new Set(renders.styles.map((style) => style.id)).size !==
      renders.styles.length ||
    renders.styles.some(
      (render) => !catalog.styles.some((style) => style.id === render.id),
    )
  )
    throw new Error("Unknown or duplicate original preview evidence");
  const nativeSample = {
    kind: "selfAuthoredExample",
    pageCount: 8,
    sourceEquivalent: false,
    notice: "src/assets/presentation-styles/NOTICE.md",
    pageKinds: [
      "cover",
      "outline",
      "content",
      "comparison",
      "chart",
      "process",
      "equation",
      "closing",
    ],
  };
  const styles = [
    {
      id: "navy",
      availability: "enabled",
      sourceId: null,
      sourceDeck: {
        ...(await builtinDeck(root)),
        preview: {
          status: "not-applicable",
          pages: [],
          reason: "A zero-content-page master is not an original example deck.",
        },
      },
      nativeAdaptation: {
        status: "builtin",
        coverage:
          "Ofox baseline master and native helper layouts; no upstream complete deck is claimed.",
      },
      selfAuthoredPreview: nativeSample,
      licenses: {
        code: { status: "project-owned", license: "MIT" },
        fonts: { status: "system-fallback-not-embedded" },
        images: { status: "none-imported" },
        brands: { status: "independent-sample" },
      },
    },
  ];
  for (const style of catalog.styles) {
    const html = style.resources.find((file) => file.path.endsWith(".html"));
    const script = style.resources.find((file) =>
      file.path.endsWith("build.sh"),
    );
    const deck = html
      ? await htmlDeck(base, html)
      : await scriptDeck(base, script);
    const render = renders.styles.find((entry) => entry.id === style.id);
    if (render) {
      const source = html || script;
      if (
        render.sourceSha256 !== source.sha256 ||
        render.pageCount !== deck.pageCount
      )
        throw new Error(`Stale original preview: ${style.id}`);
      const inputs = [...style.resources, ...deck.dependencyFiles].map(
        ({ path: filePath, sha256 }) => ({ path: filePath, sha256 }),
      );
      if (!isDeepStrictEqual(render.inputFiles, inputs))
        throw new Error(`Stale original preview dependencies: ${style.id}`);
      if (
        !["partially-rendered", "rendered-with-local-fonts"].includes(
          render.status,
        )
      )
        throw new Error(`Unknown original preview status: ${style.id}`);
      const numbers = new Set();
      for (const page of render.pages) {
        if (
          !Number.isInteger(page.number) ||
          page.number < 1 ||
          page.number > deck.pageCount
        )
          throw new Error(`Invalid preview page: ${style.id}`);
        if (
          numbers.has(page.number) ||
          !new RegExp(
            `^source-previews/${style.id}/page-${String(page.number).padStart(2, "0")}(?:-${page.sha256})?\\.jpg$`,
          ).test(page.path)
        )
          throw new Error(
            `Invalid/duplicate original preview path: ${style.id}`,
          );
        numbers.add(page.number);
        const jpeg = await readFile(safePath(base, page.path));
        assertOriginalBytes(jpeg, page);
        if (jpeg[0] !== 0xff || jpeg[1] !== 0xd8 || jpeg[2] !== 0xff)
          throw new Error(`Original preview must be JPEG: ${page.path}`);
        if (
          !isDeepStrictEqual(jpegSize(jpeg), [page.width, page.height]) ||
          page.width !== 1920 ||
          page.height !== 1080
        )
          throw new Error(`Original preview dimensions mismatch: ${page.path}`);
        if (
          !LEGACY_ACTIVATION.has(style.id) &&
          (render.renderer?.activationMode !== "native-navigation" ||
            !activationProofAccepted(page.activationProof))
        )
          throw new Error(
            `Original page lacks proof of visible activation: ${style.id}/${page.number}`,
          );
        if (style.id === "cartesian" && [4, 7].includes(page.number)) {
          const data =
            page.number === 4
              ? [
                  [2.4, 3.1, 3.8, 4.5],
                  [2.5, 3, 3.5, 4.2],
                ]
              : [
                  [12, 18, 25, 34, 42, 51],
                  [10, 14, 19, 24, 30, 36],
                ];
          if (
            page.chartEvidence?.libraryVersion !== "4.6.1" ||
            page.chartEvidence.nonTransparentPixels <= 1000 ||
            !isDeepStrictEqual(
              page.chartEvidence.datasets?.map((dataset) => dataset.data),
              data,
            ) ||
            !isDeepStrictEqual(render.dependencyOverrides, previewDependencies)
          )
            throw new Error(
              "Original Cartesian canvas evidence is missing or stale",
            );
        }
      }
      if (
        render.status === "rendered-with-local-fonts" &&
        numbers.size !== deck.pageCount
      )
        throw new Error(`Incomplete original preview: ${style.id}`);
      for (const page of render.omittedPages || []) {
        if (
          !Number.isInteger(page.number) ||
          page.number < 1 ||
          page.number > deck.pageCount ||
          numbers.has(page.number) ||
          !page.reason
        )
          throw new Error(`Invalid omitted preview page: ${style.id}`);
        numbers.add(page.number);
      }
      if (numbers.size !== deck.pageCount)
        throw new Error(
          `Original preview page partition is incomplete: ${style.id}`,
        );
      if (render.rebuiltSourceDeck)
        assertOriginalBytes(
          await readFile(safePath(base, render.rebuiltSourceDeck.path)),
          render.rebuiltSourceDeck,
        );
    }
    styles.push({
      id: style.id,
      availability: style.enabled ? "enabled" : "candidate",
      sourceId: style.sourceId,
      sourceDeck: {
        ...deck,
        files: style.resources,
        preview: render || {
          status: "not-rendered",
          pageCount: deck.pageCount,
          pages: [],
          reason:
            "Source inventory is verified; actual original-deck rendering is separate evidence.",
        },
      },
      nativeAdaptation: nativeAdaptation(style, deck.pageCount),
      // Catalog styles show native adaptation sample pages (adaptation-previews.json),
      // one per mapped source page kind; only navy keeps the eight-page example.
      selfAuthoredPreview: null,
      nativeAdaptationSample: await adaptationSample(base, style, samples),
      licenses: {
        code: {
          status: style.licenseReviewed ? "reviewed" : "pending",
          license: sources.find((source) => source.id === style.sourceId)
            .license,
          files: style.licenseFiles,
        },
        fonts: {
          status: "pending",
          policy:
            "No remote font binary fetched or embedded; rendered previews may use local fallback.",
        },
        images: {
          status: "pending",
          policy: "Repository code license is not third-party image clearance.",
        },
        brands: {
          status: "pending",
          policy:
            "Upstream sample claims/brands/testimonials are examples, not Ofox endorsements or verified facts.",
        },
      },
    });
  }
  return {
    schemaVersion: 1,
    artifactKinds: [
      {
        id: "originalSourceDeck",
        staticResources: true,
        meaning:
          "Pinned upstream HTML/build source pages or the explicitly identified built-in master.",
      },
      {
        id: "nativeAdaptation",
        staticResources: true,
        meaning:
          "Implemented Ofox native theme/composition coverage; partial unless page mapping is separately evidenced.",
      },
      {
        id: "selfAuthoredExample",
        staticResources: true,
        meaning:
          "Independent eight-page Ofox demonstrations; never a substitute for upstream deck coverage.",
      },
      {
        id: "generatedUserFile",
        staticResources: false,
        meaning:
          "Session-authorized runtime output, never a static source resource or a promised existing user deck.",
      },
    ],
    pageDetection: {
      html: "Parse without running scripts: direct .deck/.presentation > .slide when present, otherwise .slide; deck-stage > section for custom-element sources. Direct-child scopes exclude runtime overview clones. Count exact nodes and retain original line/index.",
      officecli:
        "Count actual top-level sl --prop background calls and matching SLIDE headings, not README examples.",
      builtin:
        "Inspect PPTX ZIP slide/layout XML parts; master layouts are not content pages.",
    },
    originalFileCount: originals.size,
    originalBytes,
    originalInventorySha256: inventoryHash,
    sources,
    previewDependencies,
    styles,
  };
}

/** Counts of a style's page mapping; "partial" until every page is mapped. */
function nativeAdaptation(style, pageCount) {
  const mapping = style.pageMapping;
  if (!style.adapted)
    return {
      status: "not-adapted",
      coverage: "Candidate source only; no native page mapping implemented.",
      originalPagesIndividuallyMapped: 0,
    };
  if (!mapping)
    return {
      status: "partial",
      coverage: style.recipe?.nativeCoverage,
      originalPagesIndividuallyMapped: 0,
    };
  if (mapping.length !== pageCount)
    throw new Error(`Page mapping does not cover the source: ${style.id}`);
  const count = (status) =>
    mapping.filter((page) => page.status === status).length;
  return {
    status: "page-mapped",
    coverage: style.recipe?.nativeCoverage,
    originalPagesIndividuallyMapped: count("adapted") + count("approximate"),
    adapted: count("adapted"),
    approximate: count("approximate"),
    unsupported: count("unsupported"),
    unsupportedPages: mapping
      .filter((page) => page.status === "unsupported")
      .map((page) => page.page),
  };
}

/** The published native sample pages of a style, checked byte for byte. */
async function adaptationSample(base, style, samples) {
  const record = samples.styles.find((item) => item.id === style.id);
  if (!record) return null;
  const expected = (style.pageMapping || []).filter(
    (page) => page.status !== "unsupported",
  );
  if (
    record.sourceEquivalent !== false ||
    record.embeddedMedia !== 0 ||
    record.schemaErrors.length ||
    record.pages.length !== expected.length ||
    record.pages.some(
      (page, index) =>
        page.sourcePage !== expected[index].page ||
        page.archetype !== expected[index].archetype ||
        page.status !== expected[index].status,
    )
  )
    throw new Error(`Stale native adaptation sample: ${style.id}`);
  for (const page of record.pages) {
    if (
      !new RegExp(
        `^adaptation-previews/${style.id}/page-\\d{2}-${page.sha256}\\.jpg$`,
      ).test(page.path)
    )
      throw new Error(`Invalid native sample path: ${style.id}`);
    assertOriginalBytes(await readFile(safePath(base, page.path)), page);
  }
  return {
    kind: "nativeAdaptationSample",
    sourceEquivalent: false,
    pageCount: record.pages.length,
    lintFindings: record.lint.length,
    manifest: "adaptation-previews.json",
  };
}

// Decks whose forced-display captures were individually reviewed on
// 2026-11-10. Every later capture must prove its page was really shown.
export const LEGACY_ACTIVATION = new Set([
  "blue-professional",
  "cartesian",
  "course-module",
  "product-launch",
  "architectural-plan",
]);

/**
 * A page counts only when it was on screen and most of its text was visible.
 * A page shorter than the viewport (a deck's own short closing page) counts
 * when the whole of it was on screen and it spans the viewport's width.
 */
export function activationProofAccepted(proof) {
  return (
    !!proof &&
    (proof.viewportCoverage >= 0.9 ||
      (proof.ownAreaVisible >= 0.98 &&
        proof.viewportCoverage >= 0.4 &&
        proof.rect?.[2] >= 1900)) &&
    Number.isInteger(proof.textElements) &&
    Number.isInteger(proof.visibleTextElements) &&
    proof.visibleTextElements >= 1 &&
    proof.visibleTextElements >= proof.textElements * 0.6
  );
}

export async function verifyCoverage(root = ROOT, supplied) {
  const expected = await buildCoverage(root);
  const actual =
    supplied ||
    JSON.parse(
      await readFile(
        path.join(root, RESOURCE_PATH, "source-coverage.json"),
        "utf8",
      ),
    );
  if (!isDeepStrictEqual(actual, expected))
    throw new Error(
      "Source coverage manifest drift: regenerate after checking page counts, source hashes, provenance and preview evidence",
    );
  return {
    ok: true,
    originalFiles: expected.originalFileCount,
    styles: expected.styles.length,
    originalEnabledPages: expected.styles
      .filter((style) => style.availability === "enabled" && style.sourceId)
      .reduce((sum, style) => sum + style.sourceDeck.pageCount, 0),
    candidates: expected.styles.filter(
      (style) => style.availability === "candidate",
    ).length,
  };
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const root = ROOT;
  if (process.argv.includes("--source-only")) {
    if (process.argv.includes("--write"))
      throw new Error(
        "Source-only verification cannot rewrite preview evidence",
      );
    const sources = await buildCoverage(root, { sourceOnly: true });
    console.log(
      JSON.stringify({
        ok: true,
        originalFiles: sources.originalFileCount,
        styles: sources.styles.length,
        previewChecked: false,
      }),
    );
    process.exit(0);
  }
  if (process.argv.includes("--write")) {
    const value = await buildCoverage(root);
    await writeFile(
      path.join(root, RESOURCE_PATH, "source-coverage.json"),
      JSON.stringify(value, null, 2) + "\n",
    );
  }
  console.log(JSON.stringify(await verifyCoverage(root)));
}
