import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { copyFile, mkdir, readFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { promisify } from "node:util";
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

test("validates clean", options, async (t) => {
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
  assert.deepEqual(lint, []);
  const validation = await office(t, "validate", result.out);
  assert.deepEqual(validation.errors, []);
  const issues = await office(t, "view", result.out, "issues");
  assert.deepEqual(issues.issues, []);
});

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
s = content_slide(prs, "Lint")
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
  const lint = JSON.parse(await readFile(result.lint, "utf8")).join("\n");
  assert.match(lint, /card "A card": text needs/);
  assert.match(lint, /steps: .*lower h/);
  assert.match(lint, /figure "small figure": smallest text/);
  assert.match(lint, /plot "small ticks": .*use at least 10/);
});

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
