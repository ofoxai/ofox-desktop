import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  options,
  run,
  zipEntries,
  withoutDensity,
} from "./fixtures/python-engine.mjs";

const slideXml = (entries, index) =>
  entries.get(`ppt/slides/slide${index}.xml`).toString();

test(
  "a declarative composition draws its own canvases, type and page kinds as native objects",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="创意提案", style="studio", language="en")
cover_slide(prs, "Proposal", subtitle="A self-written sample", meta="Sample studio")
section_slide(prs, "Who we are", number="01")
statement_slide(prs, "Great work is planned", kicker="Belief", attribution="Sample author")
s = content_slide(prs, "The studio")
kpi(s, 0.72, 2.0, 3.6, 2.2, "12", "Years")
card(s, 4.8, 2.0, 3.6, 2.2, "Approach", "Strategy before aesthetics.")
s2 = content_slide(prs, "What we offer")
steps(s2, [{"title": "Listen"}, {"title": "Shape"}, {"title": "Ship"}], 0.72, 2.2, 11.9, 1.8)
closing_slide(prs, "Any questions", meta="Sample studio")
closing_slide(prs, "A = P(1+r)^n")
`,
    );
    assert.equal(result.status, 0, result.stderr);
    // A formula keeps its letters: r and n are not R and N.
    assert.ok(
      zipEntries &&
        (await zipEntries(result.out))
          .get("ppt/slides/slide7.xml")
          .toString()
          .includes("A = P(1+r)^n"),
    );
    assert.deepEqual(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
      [],
    );
    const entries = await zipEntries(result.out);
    // Cover: near-black canvas, uppercase native title in electric yellow.
    const cover = slideXml(entries, 1);
    assert.match(cover, /name="Background"[\s\S]*?srgbClr val="1C1C1C"/);
    assert.ok(cover.includes("PROPOSAL") && cover.includes('type="title"'));
    // Chapter and closing use the yellow canvas with dark type.
    for (const index of [2, 6]) {
      const page = slideXml(entries, index);
      assert.match(page, /name="Background"[\s\S]*?srgbClr val="F5D200"/);
      assert.ok(page.includes('val="1C1C1C"'), `dark type on page ${index}`);
      assert.ok(page.includes('type="title"'));
    }
    assert.ok(slideXml(entries, 3).includes("GREAT WORK IS PLANNED"));
    assert.ok(slideXml(entries, 3).includes("Sample author"));
    // Content canvases alternate; helper colours follow each page's canvas.
    assert.match(
      slideXml(entries, 4),
      /name="Background"[\s\S]*?srgbClr val="F5D200"/,
    );
    assert.match(
      slideXml(entries, 5),
      /name="Background"[\s\S]*?srgbClr val="1C1C1C"/,
    );
    assert.ok(!slideXml(entries, 4).includes("roundRect"));
    // Numbered steps are bare numerals, not rounded boxes.
    const steps = slideXml(entries, 5);
    assert.ok(steps.includes(">01<") && !steps.includes("roundRect"));
    assert.ok(
      ![...entries.keys()].some((name) => name.startsWith("ppt/media/")),
    );
  },
);

test(
  "statement and closing pages stay available to every other style",
  options,
  async (t) => {
    const result = await run(
      t,
      `
for style in ("navy", "cartesian"):
    prs = new_deck(title="样张", style=style, language="zh")
    statement_slide(prs, "一个清晰的主张", attribution="自行编写")
    closing_slide(prs, "谢谢", subtitle="欢迎提问")
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    assert.ok(slideXml(entries, 1).includes('type="title"'));
    assert.ok(slideXml(entries, 2).includes("谢谢"));
  },
);

test(
  "helpers on an earlier page keep that page's canvas, white stays white and code is monospaced",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="Sample", style="studio", language="en")
first = content_slide(prs, "Yellow page")
second = content_slide(prs, "Dark page")
text(second, "on dark", 0.72, 2.0, 5, 0.5, size=18)
text(first, "back on yellow", 0.72, 2.0, 5, 0.5, size=18)
assert T.white == "FFFFFF", T.white
code(first, "x = 1", 0.72, 3.0, 5, 1.0)
prs2 = new_deck(title="Sample", style="editorial-forest", language="en")
page = content_slide(prs2, "Template")
block = code(page, "x = 1", 0.72, 2.0, 5, 1.0)
assert block.text_frame.paragraphs[0].runs[0].font.name == "Consolas"
cover = new_deck(title="Sample", style="block-frame", language="en")
c = cover_slide(cover, "Title", art='<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#000000"/></svg>')
title = c.shapes.title
assert (title.left + title.width) / 914400 <= 8.3, "cover title must stay clear of the art column"
`,
    );
    assert.equal(result.status, 0, result.stderr);
  },
);
