// Automatic checks over the whole style library: every catalog style, its
// adapter, page mapping, native sample pages and theme. Low cost, no model.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  ARCHETYPES,
  FAMILIES,
  loadAdapters,
  mergedCatalog,
} from "../../scripts/presentation-adapters.mjs";
import { jpegSize } from "../../scripts/presentation-resource-coverage.mjs";
import { parseDeck } from "../src/deck/schema.mjs";
import { renderDeck } from "../src/deck/render.mjs";
import { THEME_NAMES, themeFor } from "../src/deck/theme.mjs";
import { PAGE_KINDS } from "../src/presentation-planning.mjs";

const resources = new URL(
  "../skills/ofox-office-pptx/resources/",
  import.meta.url,
);
const json = async (name) =>
  JSON.parse(await readFile(new URL(name, resources), "utf8"));
const catalog = await json("catalog.json");
const coverage = await json("source-coverage.json");
const samples = await json("adaptation-previews.json");
const adapters = await loadAdapters();

const luminance = (hex) => {
  const [r, g, b] = [0, 2, 4].map((at) => {
    const value = parseInt(hex.slice(at, at + 2), 16) / 255;
    return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a, b) => {
  const [high, low] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (high + 0.05) / (low + 0.05);
};

test("every catalog style has an adapter, is merged unchanged and is selectable", async () => {
  assert.equal(catalog.styles.length, 24);
  assert.deepEqual(
    adapters.map((adapter) => adapter.id).sort(),
    catalog.styles.map((style) => style.id).sort(),
  );
  // The catalog on disk is exactly what the adapters produce.
  assert.deepEqual((await mergedCatalog()).catalog, catalog);
  for (const style of catalog.styles) {
    assert.equal(style.status, "enabled", style.id);
    assert.ok(
      style.enabled && style.adapted && style.licenseReviewed,
      style.id,
    );
    assert.ok(THEME_NAMES.includes(style.id), style.id);
    assert.ok(FAMILIES.includes(style.family), style.id);
    // Enabled is never presented as verified quality.
    assert.equal(style.verified, false, style.id);
    for (const language of ["zh", "en", "ja"]) {
      assert.ok(style.names[language].trim(), `${style.id} name ${language}`);
      assert.ok(style.intro[language].trim(), `${style.id} intro ${language}`);
    }
  }
  // Each family the picker groups by has more than one way to look.
  const families = new Set(catalog.styles.map((style) => style.family));
  assert.ok(families.size >= 5, [...families].join(","));
});

test("each page mapping covers every actual source page and never counts an unsupported page as adapted", () => {
  let pages = 0;
  for (const style of catalog.styles) {
    const record = coverage.styles.find((item) => item.id === style.id);
    assert.equal(
      style.pageMapping.length,
      record.sourceDeck.pageCount,
      `${style.id} maps every actual page`,
    );
    pages += style.pageMapping.length;
    for (const page of style.pageMapping) {
      assert.ok(ARCHETYPES.includes(page.archetype), style.id);
      assert.ok(PAGE_KINDS.includes(page.archetype), style.id);
      if (page.status !== "adapted")
        assert.ok(page.differences.trim(), `${style.id}/${page.page}`);
      if (page.status === "unsupported") assert.equal(page.native, "");
    }
    assert.equal(record.nativeAdaptation.status, "page-mapped", style.id);
    assert.equal(
      record.nativeAdaptation.unsupported,
      style.pageMapping.filter((page) => page.status === "unsupported").length,
    );
    // The original deck is fully rendered, separately from the native pages.
    assert.equal(
      record.sourceDeck.preview.pages.length,
      record.sourceDeck.pageCount,
      `${style.id} original pages rendered`,
    );
  }
  assert.equal(pages, 236);
});

test("native sample pages exist for every mapped page, are native objects and are byte-identical to the manifest", async () => {
  for (const style of catalog.styles) {
    const record = samples.styles.find((item) => item.id === style.id);
    assert.ok(record, `${style.id} has native sample pages`);
    const mapped = style.pageMapping.filter(
      (page) => page.status !== "unsupported",
    );
    assert.equal(record.pages.length, mapped.length, style.id);
    assert.equal(record.sourceEquivalent, false);
    assert.equal(record.embeddedMedia, 0, `${style.id} embeds no picture`);
    assert.deepEqual(record.schemaErrors, [], style.id);
    assert.deepEqual(record.lint, [], `${style.id}: ${record.lint.join("; ")}`);
    assert.equal(
      record.adapterSha256,
      createHash("sha256")
        .update(
          JSON.stringify(adapters.find((adapter) => adapter.id === style.id)),
        )
        .digest("hex"),
      `${style.id} sample pages were built from the current adapter`,
    );
    for (const [index, page] of record.pages.entries()) {
      assert.equal(page.sourcePage, mapped[index].page);
      assert.equal(page.archetype, mapped[index].archetype);
      const bytes = await readFile(new URL(page.path, resources));
      assert.equal(
        createHash("sha256").update(bytes).digest("hex"),
        page.sha256,
        page.path,
      );
      assert.deepEqual(jpegSize(bytes), [1280, 720], page.path);
    }
  }
});

test("every theme is drawable by both engines' inputs: readable base colours, defined canvases, shapes on the slide", () => {
  for (const style of catalog.styles) {
    const { colors, decor, fonts, sizes } = style.theme;
    assert.ok(
      contrast(colors.text, colors.background) >= 4.5,
      `${style.id} body text contrast ${contrast(colors.text, colors.background).toFixed(2)}`,
    );
    assert.ok(
      contrast(colors.primary, colors.background) >= 3,
      `${style.id} title contrast`,
    );
    for (const role of ["hero", "statement", "title", "body", "card", "kpi"])
      assert.ok(
        sizes[role].every(
          (size, at) => at === 0 || size <= sizes[role][at - 1],
        ),
        `${style.id} ${role} sizes run large to small`,
      );
    assert.ok(Math.min(...sizes.body) >= 12, `${style.id} body size`);
    for (const language of ["latin", "zh", "ja"])
      assert.ok(fonts[language].title && fonts[language].body, style.id);
    const layout = decor.layout;
    if (decor.composition !== "spec") continue;
    const canvases = new Set(["base", ...Object.keys(layout.canvases ?? {})]);
    for (const tone of Object.values(layout.tones ?? {}).flat())
      assert.ok(canvases.has(tone), `${style.id} tone ${tone}`);
    for (const [name, canvas] of Object.entries(layout.canvases ?? {})) {
      const bg = canvas.bg ?? colors.background;
      for (const role of ["title", "body"]) {
        const ink =
          canvas[role] ?? (role === "title" ? colors.primary : colors.text);
        assert.ok(
          contrast(ink, bg) >= 3,
          `${style.id} canvas ${name} ${role} contrast ${contrast(ink, bg).toFixed(2)}`,
        );
      }
    }
    for (const kind of ["cover", "section", "statement", "content", "closing"])
      for (const shape of layout[kind]?.shapes ?? []) {
        const count = shape.repeat?.count ?? 1;
        assert.ok(count >= 1 && count <= 60, style.id);
        // A rotated shape may reach past the edge; an upright one must start on the slide.
        if (!shape.rot)
          assert.ok(
            shape.x < 13.34 && shape.y < 7.51 && shape.w > 0 && shape.h > 0,
            `${style.id} ${kind} shape off the slide`,
          );
      }
  }
});

test("the deck-object renderer draws every style without the Python engine", () => {
  for (const id of THEME_NAMES) {
    const parsed = parseDeck({
      kind: "deck",
      title: "统一样张",
      language: "zh",
      theme: id,
      slides: [
        { type: "hero", title: "统一样张", subtitle: "自行编写" },
        {
          type: "pillars",
          title: "两种方案",
          pillars: [
            { heading: "全量", body: "每批重建" },
            { heading: "增量", body: "跟踪变更" },
          ],
        },
        { type: "cta", title: "继续验证" },
      ],
    });
    assert.equal(parsed.problems, undefined, id);
    const { commands } = renderDeck(parsed.deck, themeFor(id));
    assert.ok(
      commands.some((command) => command.props?.text === "每批重建"),
      id,
    );
  }
});
