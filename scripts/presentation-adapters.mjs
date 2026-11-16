// Native adaptation records: one reviewed JSON file per catalog style under
// resources/adapters. This script checks each record against the pinned source
// coverage and merges it into catalog.json, the single file every runtime reads.
// It never reads or executes the vendored HTML/scripts.
import { rmdirSync } from "node:fs";
import { mkdir, readFile, readdir, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";

const ROOT = path.resolve(fileURLToPath(new URL("../", import.meta.url)));
export const RESOURCES = "assistant-runtime/skills/ofox-office-pptx/resources";

// Page kinds found in the pinned source decks. A mapping names one per page.
export const ARCHETYPES = [
  "cover",
  "chapter",
  "agenda",
  "statement",
  "quote",
  "list",
  "split-image",
  "cards",
  "stats",
  "comparison",
  "table",
  "chart-bar",
  "chart-line",
  "chart-pie",
  "process",
  "timeline",
  "cycle",
  "hierarchy",
  "diagram",
  "code",
  "team",
  "lesson",
  "pricing",
  "closing",
];
export const MAPPING_STATUS = ["adapted", "approximate", "unsupported"];
// Helpers a mapping may name; each must exist in the Python deck helper.
export const NATIVE_HELPERS = [
  "cover_slide",
  "section_slide",
  "statement_slide",
  "closing_slide",
  "content_slide",
  "panel_slide",
  "lesson_slide",
  "bullets",
  "card",
  "kpi",
  "steps",
  "timeline",
  "callout",
  "relation",
  "fork_join",
  "native_chart",
  "table",
  "pyramid",
  "code",
  "image",
  "equation",
  "text",
];
// Style families: the picker groups by them and acceptance samples across them.
export const FAMILIES = [
  "editorial",
  "business",
  "poster",
  "color-block",
  "playful",
  "teaching",
  "technical",
];
const LANGUAGES = ["zh", "en", "ja"];
export const SCENARIOS = [
  "academic",
  "research",
  "teaching",
  "workshop",
  "consulting",
  "business-review",
  "product-launch",
  "creative",
  "technical",
  "general",
];
const HEX = /^[0-9A-F]{6}$/;

function fail(id, why) {
  throw new Error(`Invalid presentation adapter ${id}: ${why}`);
}

function localized(id, value, name, max) {
  if (
    !value ||
    !isDeepStrictEqual(Object.keys(value).sort(), [...LANGUAGES].sort()) ||
    LANGUAGES.some(
      (language) =>
        typeof value[language] !== "string" ||
        !value[language].trim() ||
        value[language].length > max,
    )
  )
    fail(id, `${name} needs nonempty zh, en and ja text`);
}

/** Checks one adapter against its catalog style and actual source page count. */
export function checkAdapter(adapter, style, pageCount, helperSource) {
  const id = adapter?.id;
  if (!style || id !== style.id) fail(id, "no matching catalog style");
  localized(id, adapter.names, "names", 40);
  localized(id, adapter.intro, "intro", 200);
  if (!FAMILIES.includes(adapter.family))
    fail(id, `family must be one of ${FAMILIES.join(", ")}`);
  const routing = adapter.routing;
  if (
    !routing ||
    !Array.isArray(routing.scenarios) ||
    !routing.scenarios.length ||
    routing.scenarios.some((scenario) => !SCENARIOS.includes(scenario)) ||
    !["low", "medium", "high"].includes(routing.density) ||
    !["light", "dark", "mixed"].includes(routing.scheme) ||
    !routing.suits ||
    !isDeepStrictEqual(Object.keys(routing.suits).sort(), [
      "data",
      "formulas",
      "images",
      "longText",
    ]) ||
    Object.values(routing.suits).some((value) => ![0, 1, 2].includes(value)) ||
    typeof routing.avoid !== "string"
  )
    fail(id, "routing");
  const theme = adapter.theme ?? style.theme;
  if (!theme?.colors || !theme.fonts || !theme.sizes || !theme.decor)
    fail(id, "theme");
  for (const [name, value] of Object.entries(theme.colors))
    if (
      Array.isArray(value)
        ? value.some((item) => !HEX.test(item))
        : !HEX.test(value)
    )
      fail(id, `color ${name} must be upper-case RRGGBB`);
  for (const key of [
    "background",
    "card",
    "primary",
    "accent",
    "text",
    "textSecondary",
    "textMuted",
    "dark",
    "onDark",
    "onDarkMuted",
    "chart",
  ])
    if (!(key in theme.colors)) fail(id, `missing color ${key}`);
  for (const language of ["latin", "zh", "ja"])
    if (!theme.fonts[language]?.title || !theme.fonts[language]?.body)
      fail(id, `fonts.${language}`);
  const layout = theme.decor.layout;
  if (theme.decor.composition === "spec") {
    if (!layout) fail(id, "a spec composition needs decor.layout");
    for (const [name, canvas] of Object.entries(layout.canvases ?? {}))
      for (const [role, value] of Object.entries(canvas))
        if (
          Array.isArray(value)
            ? value.some((item) => !HEX.test(item))
            : !HEX.test(value)
        )
          fail(id, `canvas ${name}.${role} must be upper-case RRGGBB`);
    for (const [kind, tone] of Object.entries(layout.tones ?? {}))
      if (Array.isArray(tone) ? !tone.length : typeof tone !== "string")
        fail(id, `tones.${kind} must be a canvas name or a nonempty list`);
    for (const kind of ["cover", "section", "statement", "content", "closing"])
      for (const shape of layout[kind]?.shapes ?? [])
        if (
          !["rect", "round", "oval"].includes(shape.kind ?? "rect") ||
          [shape.x, shape.y, shape.w, shape.h].some(
            (value) => typeof value !== "number",
          ) ||
          (shape.repeat && !Number.isInteger(shape.repeat.count))
        )
          fail(id, `${kind} shape`);
    if (
      layout.kpi &&
      !["rule", "box", "bare", "block"].includes(layout.kpi.style)
    )
      fail(id, "kpi.style");
    if (
      layout.steps &&
      !["box", "numbered", "block"].includes(layout.steps.style)
    )
      fail(id, "steps.style");
  }
  for (const key of ["composition", "components", "narrative", "nativeCoverage"])
    if (typeof (adapter.recipe ?? style.recipe)?.[key] !== "string")
      fail(id, `recipe.${key}`);
  const mapping = adapter.pageMapping;
  if (!Array.isArray(mapping) || mapping.length !== pageCount)
    fail(id, `pageMapping must list all ${pageCount} actual source pages`);
  mapping.forEach((entry, index) => {
    if (
      entry.page !== index + 1 ||
      !ARCHETYPES.includes(entry.archetype) ||
      !MAPPING_STATUS.includes(entry.status) ||
      typeof entry.native !== "string" ||
      typeof entry.differences !== "string" ||
      (entry.status !== "adapted" && !entry.differences.trim())
    )
      fail(id, `pageMapping entry ${index + 1}`);
    const helpers = entry.native.split("+").map((name) => name.trim());
    if (
      entry.status === "unsupported"
        ? entry.native !== ""
        : !helpers.length ||
          helpers.some(
            (name) =>
              !NATIVE_HELPERS.includes(name) ||
              !new RegExp(`^def ${name}\\(`, "m").test(helperSource),
          )
    )
      fail(id, `pageMapping entry ${index + 1} names an unknown native helper`);
  });
}

export async function loadAdapters(root = ROOT, only) {
  const directory = path.join(root, RESOURCES, "adapters");
  const names = (await readdir(directory))
    .filter(
      (name) =>
        name.endsWith(".json") && (!only || only.includes(name.slice(0, -5))),
    )
    .sort();
  return Promise.all(
    names.map(async (name) => {
      const adapter = JSON.parse(
        await readFile(path.join(directory, name), "utf8"),
      );
      if (adapter.id + ".json" !== name)
        fail(adapter.id, "file name must be <id>.json");
      return adapter;
    }),
  );
}

const fontPolicy = (style) => ({
  sourceFamilies: style.typography ?? {},
  runtime:
    "Intentional system-font substitutions retain serif/sans/data roles. No external font binary is downloaded or embedded. Native app fallback and CJK metrics require visual verification on each OS.",
  externalFontRedistribution: "not approved",
});

/** The catalog with every adapter merged in; the input objects are not changed. */
export async function mergedCatalog(root = ROOT, only) {
  const base = path.join(root, RESOURCES);
  const catalog = JSON.parse(
    await readFile(path.join(base, "catalog.json"), "utf8"),
  );
  const coverage = JSON.parse(
    await readFile(path.join(base, "source-coverage.json"), "utf8"),
  );
  const helperSource = await readFile(
    path.join(root, "assistant-runtime/src/python/deck.py"),
    "utf8",
  );
  const adapters = await loadAdapters(root, only);
  for (const adapter of adapters) {
    const style = catalog.styles.find((item) => item.id === adapter.id);
    const pages = coverage.styles.find((item) => item.id === adapter.id)
      ?.sourceDeck.pageCount;
    checkAdapter(adapter, style, pages, helperSource);
    Object.assign(style, {
      status: "enabled",
      adapted: true,
      enabled: true,
      family: adapter.family,
      names: adapter.names,
      intro: adapter.intro,
      routing: adapter.routing,
      scenarios: adapter.routing.scenarios,
      density: adapter.routing.density,
      scheme: adapter.routing.scheme,
      theme: adapter.theme ?? style.theme,
      recipe: adapter.recipe ?? style.recipe,
      fontPolicy: style.fontPolicy ?? fontPolicy(style),
      pageMapping: adapter.pageMapping,
      // Enabling a style never upgrades its acceptance state.
      validation: {
        visualQuality: "pending",
        powerPointRoundTrip: "pending",
        modelABCD: "pending",
        ...style.validation,
      },
      modifications: [
        "Ofox native theme/recipe and bounded layout adaptation added; vendor source bytes unchanged. Not a complete HTML-to-PPTX converter.",
      ],
    });
  }
  const enabled = catalog.styles.filter((style) => style.enabled).length;
  catalog.runtimeScope = `${enabled} of ${catalog.styles.length} public sources are experimentally selectable via native adapters with per-page mapping records. Enabled is not verified quality or PowerPoint editing acceptance; unmapped or unsupported pages are listed per style.`;
  return { catalog, adapters };
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  // --only=a,b merges just those adapters and leaves every other catalog
  // entry as it is, so styles can be worked on independently.
  const only = process.argv
    .find((arg) => arg.startsWith("--only="))
    ?.slice(7)
    .split(",");
  const file = path.join(ROOT, RESOURCES, "catalog.json");
  // One writer at a time: a merge reads the catalog, changes it and replaces it.
  const lock = file + ".lock";
  for (let attempt = 0; ; attempt++) {
    try {
      await mkdir(lock);
      break;
    } catch (error) {
      if (error.code !== "EEXIST" || attempt >= 300) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
  process.on("exit", () => rmdirSync(lock));
  const { catalog, adapters } = await mergedCatalog(ROOT, only);
  // Same formatter as the repository gate, so unchanged entries keep their bytes.
  const prettier = await import("prettier");
  const text = await prettier.format(JSON.stringify(catalog), {
    ...(await prettier.resolveConfig(file)),
    filepath: file,
  });
  const current = await readFile(file, "utf8");
  if (process.argv.includes("--write")) {
    const temporary = `${file}.${process.pid}.tmp`;
    await writeFile(temporary, text);
    await rename(temporary, file);
  }
  else if (current !== text)
    throw new Error(
      "catalog.json is out of date with resources/adapters; run with --write after review",
    );
  console.log(
    JSON.stringify({
      ok: true,
      adapters: adapters.length,
      enabled: catalog.styles.filter((style) => style.enabled).length,
      written: process.argv.includes("--write"),
    }),
  );
}
