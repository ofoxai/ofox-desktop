# Native adaptation records

One JSON file per catalog style. `scripts/presentation-adapters.mjs` checks each
file against the pinned source coverage and merges it into `../catalog.json`,
the single file the Python helper, the Node runtime, the Rust host and the UI read.
An adapter describes how Ofox draws that style with **native PPTX objects**; it
never embeds a page picture and never runs the vendored HTML or scripts.

`studio.json` is the reference example.

## Fields

| Field         | Meaning                                                                                                              |
| ------------- | -------------------------------------------------------------------------------------------------------------------- |
| `id`          | Catalog style id; the file is `<id>.json`.                                                                           |
| `family`      | One of `editorial`, `business`, `poster`, `color-block`, `playful`, `teaching`, `technical`; used for grouping and sampling. |
| `names`       | Display name in `zh`, `en`, `ja` (at most 40 characters).                                                            |
| `intro`       | One or two sentences in `zh`, `en`, `ja` (at most 200 characters): look, and what it suits.                          |
| `routing`     | `scenarios`, `density`, `scheme`, `tone`, `suits` (0 poor, 1 fair, 2 good for data, formulas, images, longText), `avoid`. |
| `theme`       | Colours, fonts, type sizes and `decor`. Omit to keep the theme already in the catalog.                               |
| `recipe`      | `composition`, `components`, `narrative`, `nativeCoverage` text the model reads. Omit to keep the catalog's.         |
| `pageMapping` | One entry for **every actual source page**, in order.                                                                |

`routing.scenarios` values: academic, research, teaching, workshop, consulting,
business-review, product-launch, creative, technical, general.

### pageMapping

```json
{ "page": 4, "archetype": "split-image", "native": "content_slide + bullets + image", "status": "approximate", "differences": "…" }
```

- `archetype`: cover, chapter, agenda, statement, quote, list, split-image, cards,
  stats, comparison, table, chart-bar, chart-line, chart-pie, process, timeline,
  cycle, hierarchy, diagram, code, team, lesson, pricing, closing.
- `native`: helpers joined by `+`, from: cover_slide, section_slide,
  statement_slide, closing_slide, content_slide, panel_slide, lesson_slide,
  bullets, card, kpi, steps, timeline, callout, relation, fork_join,
  native_chart, table, pyramid, code, image, equation, text. Empty for `unsupported`.
- `status`: `adapted` (same page kind and structure as native objects),
  `approximate` (same page kind, a named structural difference), `unsupported`
  (no native equivalent; the page is listed, not counted as adapted).
- `differences`: required unless `adapted`. Name what the source has that the
  native page does not: an effect, a decoration, a font, a placement.

Judge the status from the rendered source page and the rendered native page side
by side. A page is not `adapted` because its colours match.

## theme

Colours are upper-case `RRGGBB`. Required: background, card, primary, accent,
text, textSecondary, textMuted, dark, onDark, onDarkMuted, chart (six series
colours). Optional: secondary, line, accentTint, surface (fill of plain boxes),
palette (block colours for `palette:N` and `"fill": "palette"`).

Fonts name **system** fonts only; no font file is bundled. Keep the source's
serif / sans / mono roles: Georgia or Times New Roman for serif display, Arial
for grotesque, Consolas or Courier New for mono; `zh` SimSun (serif) or
Microsoft YaHei (sans); `ja` Yu Mincho or Yu Gothic.

`sizes` are point sizes from large to small for hero, statement, title,
subtitle, body, card, caption, kpi. Page titles (hero, statement, title) take
the first size that fits their box; kpi starts from the smallest listed value
and card, caption, body and subtitle use the smallest listed value.

### decor.layout (composition `"spec"`)

The slide is 13.333 × 7.5 inches. Body content normally sits between y = 1.7 and
y = 6.85 with x from 0.72 to 12.61; keep a content page's title block above 1.6
and its decoration out of that region unless it sits behind the content.

```json
"decor": {
  "titleRule": "none", "pageNumber": true, "composition": "spec",
  "layout": {
    "upper": false,
    "canvases": { "<name>": { "bg": "…", "title": "…", "body": "…", "muted": "…", "faint": "…",
                               "accent": "…", "line": "…", "card": "…", "surface": "…",
                               "accentTint": "…", "chart": ["…"] } },
    "tones": { "cover": "base", "section": "<name>", "statement": "base",
               "content": ["base", "<name>"], "closing": "<name>" },
    "cover":     { "title": {…}, "kicker": {…}, "subtitle": {…}, "meta": {…}, "shapes": […] },
    "section":   { "title": {…}, "number": {…}, "subtitle": {…}, "shapes": […] },
    "statement": { "title": {…}, "kicker": {…}, "attribution": {…}, "shapes": […] },
    "content":   { "title": {…}, "subtitle": {…}, "body": {"x","y","w","h"}, "shapes": […] },
    "closing":   { "title": {…}, "subtitle": {…}, "meta": {…}, "shapes": […] },
    "card":  { "shape": "rect|round", "radius": 0.12, "fill": "card|none|palette|<colour>", "line": "line|none|<colour>",
               "lineW": 0.75, "shadow": [0.08, 0.08, "<colour>"], "bar": "top|left|none", "barColor": "title", "barW": 0.05 },
    "kpi":   { "style": "rule|box|bare|block", "valueColor": "accent", "fill": "palette" },
    "steps": { "style": "box|numbered|block", "arrow": true, "numberColor": "accent", "fill": "palette", "gap": 0 }
  }
}
```

- **Canvas**: a named set of colour roles for a page. `base` is the theme
  itself. `tones` picks the canvas per page kind; a list alternates by page
  number. Helpers (cards, text, charts, footers) follow the page's canvas.
- **Text block** (`title`, `kicker`, `subtitle`, `attribution`, `meta`,
  `number`): `x`, `y`, `w`, optional `h`, `align` (left, center, right),
  `size`, `color`. A title also takes `anchor` (top, middle, bottom: where the
  text sits inside its `h`), `sizes` and `upper: false`. Blocks left out get a
  sensible place next to the title.
- **Shape**: `kind` (rect, round, oval), `x`, `y`, `w`, `h`, `fill`, `line`,
  `lineW`, `radius`, `rot` (degrees), `name`, and `repeat: {"count", "dx", "dy"}`
  for rules, stripes or dots. Shapes are drawn before the text.
- **Colour reference**: a canvas role (bg, title, body, muted, faint, accent,
  line, card, surface, accentTint), a theme colour name, `palette:N`, or `RRGGBB`.
- `layout.titleBold: false` (or `bold: false` on one title block) sets titles in
  regular weight. `card.ink`, `kpi.ink`, `steps.ink` name the text colour on that
  style's filled blocks; without it the helper picks dark or light by contrast.
- `card.fill: "palette"` gives each block on a page the next palette colour and
  picks a readable text colour for it. `kpi.style: "block"` and
  `steps.style: "block"` take their own `fill`, `line`, `shape` the same way.

Keep what makes the source recognisable: where the title sits and how large it
is, the serif/sans contrast, which pages turn dark or saturated, the block or
rule language. Different styles must not collapse into the same card grid in
different colours.

## Check a style

```sh
node scripts/presentation-adapters.mjs --only=<id> --write      # validate and merge that adapter
node scripts/render-presentation-adaptations.mjs --styles=<id> --no-publish
```

The second command builds a native deck that reproduces each mapped page kind
with Ofox's own sample content and writes `page-NN.jpg` under
`.real-eval/ppt-library-completion-2026-11-11/adaptations/<id>/`. It must
report `"lint":0,"schemaErrors":0`; look at every page next to the source page
in `../source-previews/<id>/`.
