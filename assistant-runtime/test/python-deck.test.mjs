import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { copyFile, mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { promisify } from "node:util";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import { thumbnailFromSvg } from "../src/thumbnails.mjs";
import {
  officecli,
  options,
  requireEngine,
  root,
  run,
  scratch,
  zipEntries,
} from "./fixtures/python-engine.mjs";

// What the model's code gets from `from deck import *`: the 16:9 template and
// the helpers. Decks are built by the runner and read back with officecli,
// the same checker the host runs on them.

const EMU_PER_INCH = 914400;

/**
 * Runs officecli the way the host does: a private HOME, no resident process
 * (it would cache a file's view by its path) and no update check. Every deck
 * is written to a path of its own, so nothing is ever served from a cache.
 */
async function office(t, ...args) {
  const home = path.join(await scratch(t), "home");
  await mkdir(home);
  const env = {
    PATH: process.env.PATH,
    HOME: home,
    OFFICECLI_NO_AUTO_RESIDENT: "1",
    OFFICECLI_SKIP_UPDATE: "1",
  };
  // officecli exits non-zero when validate finds errors but still answers in
  // JSON; the caller reads the verdict from the data.
  const answer = await promisify(execFile)(officecli, [...args, "--json"], {
    env,
    timeout: 60_000,
  }).catch((error) => error);
  try {
    return JSON.parse(answer.stdout).data;
  } catch {
    assert.fail(
      `officecli ${args.join(" ")}: ${answer.stderr || answer.message}`,
    );
  }
}

/** The text a preview SVG draws, one string per line. */
const svgText = (svg) =>
  svg
    .replace(/<[^>]*>/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

async function build(t, code) {
  await requireEngine(officecli);
  const result = await run(t, code);
  assert.equal(result.status, 0, result.stderr);
  return result;
}

const slideNames = (entries) =>
  [...entries.keys()]
    .map((name) => name.match(/^ppt\/slides\/slide(\d+)\.xml$/))
    .filter(Boolean)
    .map(([name, n]) => ({ name, n: Number(n) }))
    .sort((a, b) => a.n - b.n);

const text = (xml) =>
  [...xml.matchAll(/<a:t>([^<]*)<\/a:t>/g)].map((m) => m[1]);

/** The layout name of slide `n`, read through its relationships. */
function layoutOf(entries, n) {
  const rels = entries.get(`ppt/slides/_rels/slide${n}.xml.rels`).toString();
  const target = rels.match(
    /Target="\.\.\/slideLayouts\/(slideLayout\d+\.xml)"/,
  );
  assert.ok(target, `slide ${n} has no layout`);
  const layout = entries.get(`ppt/slideLayouts/${target[1]}`).toString();
  return layout.match(/<p:cSld name="([^"]*)"/)[1];
}

/** The placeholders of a slide: their type and the text they carry. */
function placeholders(xml) {
  return [...xml.matchAll(/<p:sp>[\s\S]*?<\/p:sp>/g)]
    .map(([shape]) => shape)
    .filter((shape) => shape.includes("<p:ph"))
    .map((shape) => ({
      type: shape.match(/<p:ph[^>]*\btype="([^"]*)"/)?.[1] ?? "body",
      text: text(shape).join(""),
    }));
}

const OUTLINE = `
prs = new_deck(title="Outline", footer="Ofox quarterly")
cover_slide(prs, "A cover", subtitle="With a subtitle")
section_slide(prs, "A section", number="01")
content_slide(prs, "First content")
content_slide(prs, "Second content", subtitle="One claim")
`;

test("uses the 16:9 template", options, async (t) => {
  const result = await build(t, OUTLINE);
  const entries = await zipEntries(result.out);
  const size = entries
    .get("ppt/presentation.xml")
    .toString()
    .match(/<p:sldSz cx="(\d+)" cy="(\d+)"/);
  assert.ok(size, "presentation.xml has no slide size");
  assert.ok(Math.abs(size[1] / EMU_PER_INCH - 13.333) < 0.001, size[1]);
  assert.ok(Math.abs(size[2] / EMU_PER_INCH - 7.5) < 0.001, size[2]);

  const slides = slideNames(entries);
  assert.deepEqual(
    slides.map(({ n }) => layoutOf(entries, n)),
    ["Cover", "Section", "Content", "Content"],
  );
  // The template carries its three layouts and python-pptx's own are gone.
  const layouts = [...entries.keys()]
    .filter((name) => /^ppt\/slideLayouts\/slideLayout\d+\.xml$/.test(name))
    .map(
      (name) =>
        entries
          .get(name)
          .toString()
          .match(/<p:cSld name="([^"]*)"/)[1],
    );
  assert.deepEqual(layouts.sort(), ["Content", "Cover", "Section"]);

  const outline = await office(t, "view", result.out, "outline");
  assert.deepEqual(
    outline.slides.map((slide) => slide.title),
    ["A cover", "A section", "First content", "Second content"],
  );
});

test("renders no stray placeholders", options, async (t) => {
  const result = await build(t, OUTLINE);
  const entries = await zipEntries(result.out);
  for (const { name } of slideNames(entries)) {
    const xml = entries.get(name).toString();
    assert.doesNotMatch(xml, /type="dt"/, `${name} has a date placeholder`);
    for (const placeholder of placeholders(xml))
      assert.notEqual(
        placeholder.text,
        "",
        `${name} has an empty ${placeholder.type} placeholder`,
      );
  }

  // What the preview draws is what the file carries: no date and none of
  // the master's prompt text on the cover.
  const svg = await office(
    t,
    "view",
    result.out,
    "svg",
    "--start",
    "1",
    "--end",
    "1",
  );
  const drawn = svgText(svg).join("\n");
  assert.match(drawn, /A cover/);
  assert.doesNotMatch(drawn, /\d{1,4}[/.-]\d{1,2}[/.-]\d{1,4}/);
  assert.doesNotMatch(drawn, /Click to edit/);
});

test(
  "puts the footer and the page number on every content slide",
  options,
  async (t) => {
    const result = await build(t, OUTLINE);
    const entries = await zipEntries(result.out);
    const slides = slideNames(entries);
    const kinds = slides.map(({ name }) =>
      placeholders(entries.get(name).toString()).map(({ type }) => type),
    );
    // Cover and section slides stay clean; content slides carry both.
    for (const cleaned of kinds.slice(0, 2)) {
      assert.ok(!cleaned.includes("ftr"), cleaned.join());
      assert.ok(!cleaned.includes("sldNum"), cleaned.join());
    }
    for (const index of [2, 3]) {
      const found = placeholders(entries.get(slides[index].name).toString());
      const footer = found.find(({ type }) => type === "ftr");
      const number = found.find(({ type }) => type === "sldNum");
      assert.equal(footer?.text, "Ofox quarterly", `slide ${index + 1} footer`);
      assert.equal(
        number?.text,
        String(index + 1),
        `slide ${index + 1} number`,
      );
    }
    // The preview draws both, too. (It draws a slide-number field blank, which
    // is why the number is plain text in the slide-number placeholder.)
    const svg = await office(
      t,
      "view",
      result.out,
      "svg",
      "--start",
      "3",
      "--end",
      "3",
    );
    const drawn = svgText(svg);
    assert.ok(drawn.includes("Ofox quarterly"), drawn.join("|"));
    assert.ok(drawn.includes("3"), drawn.join("|"));
  },
);

test("leaves the footer out when there is none", options, async (t) => {
  const result = await build(
    t,
    'prs = new_deck(title="t")\ncontent_slide(prs, "Only")\n',
  );
  const entries = await zipEntries(result.out);
  const found = placeholders(entries.get("ppt/slides/slide1.xml").toString());
  assert.deepEqual(found.map(({ type }) => type).sort(), ["sldNum", "title"]);
});

/**
 * The RGB pixels of a page as the model's overview draws it: officecli's SVG
 * for the page, through the runtime's thumbnail code. officecli paints every
 * page white and ignores a slide's own background, so what a slide shows
 * there is what its shapes draw.
 */
async function previewPixels(t, file, page) {
  const svg = await office(
    t,
    "view",
    file,
    "svg",
    "--start",
    page,
    "--end",
    page,
  );
  const thumbnail = await thumbnailFromSvg(Buffer.from(svg));
  const image = await loadImage(Buffer.from(thumbnail.data, "base64"));
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  const { data } = context.getImageData(0, 0, image.width, image.height);
  const at = (x, y) => [
    ...data.subarray((y * image.width + x) * 4, (y * image.width + x) * 4 + 3),
  ];
  // How many pixels in a box are light (text on a dark slide) or dark.
  const count = ([x, y, w, h], keep) => {
    let n = 0;
    for (let row = y; row < y + h; row++)
      for (let column = x; column < x + w; column++)
        if (keep(at(column, row))) n += 1;
    return n;
  };
  return {
    width: image.width,
    height: image.height,
    at,
    light: (box) => count(box, (rgb) => rgb.every((v) => v > 200)),
    dark: (box) => count(box, (rgb) => rgb.every((v) => v < 100)),
  };
}

const isDark = (rgb) => rgb.every((v) => v < 100);
const isWhite = (rgb) => rgb.every((v) => v > 240);

test(
  "draws cover and section slides dark in the preview",
  options,
  async (t) => {
    const result = await build(
      t,
      `
prs = new_deck(title="Dark", footer="Ofox quarterly")
cover_slide(prs, "A cover", subtitle="With a subtitle")
section_slide(prs, "A section", number="01")
content_slide(prs, "A white content slide")
`,
    );
    const [cover, section, content] = [
      await previewPixels(t, result.out, 1),
      await previewPixels(t, result.out, 2),
      await previewPixels(t, result.out, 3),
    ];
    // The corners and the middle of both dark slides are dark.
    for (const [name, page] of [
      ["cover", cover],
      ["section", section],
    ])
      for (const [x, y] of [
        [2, 2],
        [page.width - 3, 2],
        [2, page.height - 3],
        [page.width - 3, page.height - 3],
        [page.width >> 1, page.height >> 1],
      ])
        assert.ok(
          isDark(page.at(x, y)),
          `${name} at ${x},${y}: ${page.at(x, y)}`,
        );
    // The cover's title (white, 44 pt) sits on that dark: its box (x 0.72 in,
    // y 1.9 in, 57.6 px per inch) has light pixels and mostly dark ones.
    const title = [41, 109, 400, 40];
    assert.ok(cover.light(title) > 300, `light pixels: ${cover.light(title)}`);
    assert.ok(cover.dark(title) > 8000, `dark pixels: ${cover.dark(title)}`);
    // A normal content slide stays white.
    assert.ok(isWhite(content.at(2, 2)), `${content.at(2, 2)}`);
    assert.ok(isWhite(content.at(content.width - 3, content.height - 3)));
  },
);

test(
  "background() colours any slide in the preview and in the file",
  options,
  async (t) => {
    const result = await build(
      t,
      `
prs = new_deck(title="Background", footer="Ofox quarterly")
cover_slide(prs, "A cover")
s = content_slide(prs, "A coloured slide")
background(s, T.primary)
s.shapes.title.text_frame.paragraphs[0].runs[0].font.color.rgb = rgb(T.white)
text(s, "Readable on the colour", 0.72, 2.0, 8, 0.5, size=18, color=T.white)
content_slide(prs, "A white content slide")
`,
    );
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide2.xml").toString();
    // For PowerPoint: the slide's own background; for the preview: a full-slide
    // rectangle that is the first shape, with no line.
    assert.match(xml, /<p:bg><p:bgPr><a:solidFill><a:srgbClr val="1E3A5F"\/>/);
    const first = xml.match(/<p:sp>[\s\S]*?<\/p:sp>/)[0];
    assert.match(
      first,
      /<a:off x="0" y="0"\/><a:ext cx="12192000" cy="6858000"\/>/,
    );
    assert.match(first, /<a:srgbClr val="1E3A5F"\/>/);
    assert.match(first, /<a:ln><a:noFill\/><\/a:ln>/);

    const coloured = await previewPixels(t, result.out, 2);
    for (const [x, y] of [
      [2, 2],
      [coloured.width - 3, coloured.height - 3],
      [coloured.width >> 1, coloured.height >> 1],
    ])
      assert.ok(
        isDark(coloured.at(x, y)),
        `at ${x},${y}: ${coloured.at(x, y)}`,
      );
    // The text on it is drawn light.
    assert.ok(coloured.light([41, 115, 300, 25]) > 100, "text on the colour");
    const white = await previewPixels(t, result.out, 3);
    assert.ok(isWhite(white.at(2, 2)), `${white.at(2, 2)}`);

    assert.deepEqual((await office(t, "validate", result.out)).errors, []);
    assert.deepEqual(
      (await office(t, "view", result.out, "issues")).issues,
      [],
    );
  },
);

test(
  "background() on a cover replaces its background, not hides behind it",
  options,
  async (t) => {
    const result = await build(
      t,
      `
prs = new_deck(title="Recoloured")
s = cover_slide(prs, "A black cover", subtitle="Recoloured once")
background(s, "000000")
s = section_slide(prs, "A black section", number="02")
background(s, "000000")
background(s, "000000")
`,
    );
    const entries = await zipEntries(result.out);
    for (const page of [1, 2]) {
      const xml = entries.get(`ppt/slides/slide${page}.xml`).toString();
      // One Background shape, the first one in the tree, in the latest colour,
      // however many times the colour was set; the slide's own fill agrees.
      assert.equal(xml.match(/name="Background"/g)?.length, 1, `slide ${page}`);
      const first = xml.match(/<p:sp>[\s\S]*?<\/p:sp>/)[0];
      assert.match(first, /name="Background"/);
      assert.match(first, /<a:solidFill><a:srgbClr val="000000"\/>/);
      assert.doesNotMatch(xml, /val="152B47"/);
      assert.match(
        xml,
        /<p:bg><p:bgPr><a:solidFill><a:srgbClr val="000000"\/>/,
      );
      // What the preview draws is black, not the navy the slide started with.
      const preview = await previewPixels(t, result.out, page);
      for (const [x, y] of [
        [2, 2],
        [preview.width - 3, preview.height - 3],
        [preview.width >> 1, preview.height >> 1],
      ])
        assert.ok(
          preview.at(x, y).every((v) => v < 12),
          `page ${page} at ${x},${y}: ${preview.at(x, y)}`,
        );
    }
    assert.deepEqual((await office(t, "validate", result.out)).errors, []);
    assert.deepEqual(
      (await office(t, "view", result.out, "issues")).issues,
      [],
    );
  },
);

test(
  "validates structurally and reports the legacy formula text panel",
  options,
  async (t) => {
    const result = await build(
      t,
      `
prs = new_deck(title="Clean", footer="Ofox quarterly")
cover_slide(prs, "A cover", subtitle="With a subtitle", kicker="KICKER", meta="Prepared for the team", notes="Open with the claim.")
section_slide(prs, "A section", number="01", subtitle="Where it starts", notes="Pause here.")

s = content_slide(prs, "Results at a glance", subtitle="Three numbers carry the story", notes="Walk the numbers.")
w = (G.CW - 2 * G.GAP) / 3
kpi(s, G.X, 1.7, w, 2.1, "42%", "Growth", note="Year over year", highlight=True)
kpi(s, G.X + w + G.GAP, 1.7, w, 2.1, "1.8M", "Users", note="Monthly active")
kpi(s, G.X + 2 * (w + G.GAP), 1.7, w, 2.1, "9 days", "Lead time", note="From idea to ship")
card(s, G.X, 4.0, 5.7, 2.1, "Why it works", points=["Small steps", "Fast feedback", "Clear owners"])
formula(s, 6.72, 4.0, 5.893, 1.2, ["growth = users x rate", "rate = share / time"])
callout(s, 6.2, "Ship the smallest thing that teaches us something.")

s = content_slide(prs, "How it fits together", notes="One step at a time.")
steps(
    s,
    [
        {"title": "Plan", "body": "Pick one claim per slide"},
        {"title": "Build", "body": "Write the code once"},
        {"title": "Check", "body": "Read the preview"},
    ],
    G.X, 1.7, G.CW, 1.0, highlight=1,
)
timeline(
    s,
    [
        {"label": "Q1", "title": "Prototype", "body": "A first deck"},
        {"label": "Q2", "title": "Beta", "body": "Ten teams"},
        {"label": "Q3", "title": "Launch", "body": "Everyone"},
    ],
    G.X, 2.85, G.CW, 1.9, highlight=2,
)
fig, ax = plt.subplots()
ax.bar(["A", "B", "C"], [3, 5, 4], color="#1E3A5F")
ax.set_title("Plotted")
fig.tight_layout()
plot(s, fig, G.X, 4.9, 5.7, 1.95, alt="Plotted bars")
data = CategoryChartData()
data.categories = ["A", "B", "C"]
data.add_series("Sales", (1, 2, 3))
s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(6.72), Inches(4.9), Inches(5.893), Inches(1.95), data)
`,
    );
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.equal(lint.length, 1);
    assert.match(
      lint[0],
      /slide 3: legacy formula: ordinary text, not typeset mathematics/,
    );
    const validation = await office(t, "validate", result.out);
    assert.deepEqual(validation.errors, []);
    const issues = await office(t, "view", result.out, "issues");
    assert.deepEqual(issues.issues, []);
  },
);

test("validates the template itself", options, async (t) => {
  await requireEngine(officecli);
  // A copy at a path of its own: officecli caches results by path.
  const copy = path.join(await scratch(t), "template.pptx");
  await copyFile(path.join(root, "src/python/template.pptx"), copy);
  const validation = await office(t, "validate", copy);
  assert.deepEqual(validation.errors, []);
});

test("lints what will not fit", options, async (t) => {
  const result = await build(
    t,
    `
prs = new_deck(title="Lint")
cover_slide(prs, "Cover")
s = content_slide(prs, "Lint")
need("my box", 2, 1)
need("my box", 2, 1, slide=s)
card(s, 0.72, 1.7, 5, 0.8, "A card", points=["First point of the card", "Second point of the card", "Third point of the card"])
steps(
    s,
    [{"title": "One"}, {"title": "Two"}, {"title": "Three"}],
    0.72, 2.8, 11, 1.7,
)
figure(
    s,
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300">'
    '<text x="20" y="40" font-size="12" fill="#1F2A37">Small</text></svg>',
    6, 1.7, 4, 3, alt="small figure",
)
fig, ax = plt.subplots()
ax.plot([1, 2, 3])
ax.tick_params(labelsize=8)
plot(s, fig, 0.72, 4.7, 5, 2, alt="small ticks")
`,
  );
  const lines = JSON.parse(await readFile(result.lint, "utf8"));
  const lint = lines.join("\n");
  // Each helper line names its slide: the content slide is the second.
  assert.match(lint, /^slide 2: card "A card": text needs/m);
  assert.match(lint, /^slide 2: steps: .*drawn at/m);
  assert.match(lint, /^slide 2: figure "small figure": smallest text/m);
  // The model's own need() names a slide when it is given one.
  assert.deepEqual(
    lines.filter((line) => line.includes("my box")),
    [
      "my box: text needs 2.00 in, box has 1.00 in",
      "slide 2: my box: text needs 2.00 in, box has 1.00 in",
    ],
  );
  // One line for all the small ticks, with the way to fix them.
  assert.deepEqual(
    lines.filter((line) => line.includes('plot "small ticks"')),
    [
      'slide 2: plot "small ticks": tick labels are 8 pt; use at least 10 (ax.tick_params(labelsize=10))',
    ],
  );
});

test(
  "lints small plot text once per group, and only what is drawn",
  options,
  async (t) => {
    const result = await build(
      t,
      `
prs = new_deck(title="Plots")
s = content_slide(prs, "Plots")

fig, ax = plt.subplots()
ax.plot([1, 2, 3])
ax.tick_params(labelsize=8)
plot(s, fig, 0.72, 1.7, 5, 2, alt="small ticks")

# Two axes, everything 8 pt: still one line per group, not one per axes.
fig, axes = plt.subplots(1, 2)
for ax in axes:
    ax.plot([1, 2, 3])
    ax.set_title("Title", fontsize=8)
    ax.set_xlabel("Label", fontsize=8)
    ax.tick_params(labelsize=8)
plot(s, fig, 6.2, 1.7, 5, 2, alt="small all")

# The tick at 5 is outside the view limits: matplotlib never draws it.
fig, ax = plt.subplots()
ax.plot([0, 1, 2])
ax.set_xticks([0, 1, 2, 5])
ax.set_xlim(0, 2.1)
ax.get_xticklabels()[-1].set_fontsize(8)
plot(s, fig, 0.72, 4.2, 5, 2, alt="far tick")

fig, ax = plt.subplots()
ax.plot([1, 2, 3])
plot(s, fig, 6.2, 4.2, 5, 2, alt="fine")
`,
    );
    const lines = JSON.parse(await readFile(result.lint, "utf8"));
    assert.deepEqual(lines, [
      'slide 1: plot "small ticks": tick labels are 8 pt; use at least 10 (ax.tick_params(labelsize=10))',
      'slide 1: plot "small all": title is 8 pt; use at least 10 (ax.set_title(text, fontsize=12))',
      'slide 1: plot "small all": axis labels are 8 pt; use at least 10 (ax.set_xlabel(text, fontsize=10))',
      'slide 1: plot "small all": tick labels are 8 pt; use at least 10 (ax.tick_params(labelsize=10))',
    ]);
  },
);

test("draws Chinese in plots", options, async (t) => {
  // matplotlib reports a glyph its font lacks as a UserWarning and draws a
  // box; the bundled font must cover the title, the labels and the ticks.
  await build(
    t,
    `
import warnings

prs = new_deck(title="t")
s = content_slide(prs, "Chinese")
with warnings.catch_warnings(record=True) as caught:
    warnings.simplefilter("always")
    fig, ax = plt.subplots()
    ax.bar(["一季度", "二季度", "三季度"], [3, 5, 4])
    ax.set_title("销售额与增长")
    ax.set_ylabel("万元")
    plot(s, fig, 0.72, 1.7, 6, 4, alt="中文图表")
missing = [str(w.message) for w in caught if "missing from font" in str(w.message)]
if missing:
    raise AssertionError("missing glyphs: " + "; ".join(missing[:3]))
`,
  );
});
