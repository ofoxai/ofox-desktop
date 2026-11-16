import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { promisify } from "node:util";
import {
  officecli,
  options,
  run,
  scratch,
  zipEntries,
  withoutDensity,
} from "./fixtures/python-engine.mjs";

async function validate(t, file) {
  const home = await scratch(t);
  const result = await promisify(execFile)(
    officecli,
    ["validate", file, "--json"],
    {
      env: {
        PATH: process.env.PATH,
        HOME: home,
        OFFICECLI_NO_AUTO_RESIDENT: "1",
        OFFICECLI_SKIP_UPDATE: "1",
      },
      timeout: 60_000,
    },
  );
  return JSON.parse(result.stdout);
}

for (const style of [
  "blue-professional",
  "cartesian",
  "course-module",
  "product-launch",
  "architectural-plan",
]) {
  test(
    `Python ${style} emits native titles, distinct font slots and a valid PPTX`,
    options,
    async (t) => {
      const result = await run(
        t,
        `
prs = new_deck(title="每周复盘", style="${style}", language="zh")
cover_slide(prs, "每周复盘", subtitle="统一内容", kicker="复盘")
section_slide(prs, "证据与建议", number="01")
s = content_slide(prs, "收入持续增长", notes="来源：自行编写")
# Content goes in the page's body region; some compositions put the title in a side panel.
r = slide_regions(s)["main"]; half = (r["w"] - 0.3) / 2
card(s, r["x"], r["y"], half, 2.3, "增长来源", "解释变化原因并提出行动。")
kpi(s, r["x"] + half + 0.3, r["y"], half, 2.3, "24%", "增长率", note="统一样张数据")
`,
      );
      assert.equal(result.status, 0, result.stderr);
      const entries = await zipEntries(result.out);
      for (const index of [1, 2, 3]) {
        const slide = entries.get(`ppt/slides/slide${index}.xml`).toString();
        assert.ok(
          slide.includes('type="title"'),
          `native title on slide ${index}`,
        );
        assert.ok(
          slide.includes("<a:latin") && slide.includes("<a:ea"),
          `font slots on slide ${index}`,
        );
      }
      assert.ok(
        ![...entries.keys()].some((name) => name.startsWith("ppt/media/")),
      );
      assert.deepEqual(
        withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
        [],
      );
      const verdict = await validate(t, result.out);
      assert.equal(verdict.success, true, JSON.stringify(verdict));
      assert.deepEqual(verdict.data.errors, []);
    },
  );
}

test(
  "Python selects a public style without the fixed navy layout decoration",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="课程复盘", footer="Public style smoke", style="cartesian", language="zh")
cover_slide(prs, "课程复盘", subtitle="同内容质量验证", kicker="课程设计")
section_slide(prs, "证据与建议", number="01")
s = content_slide(prs, "收入持续增长", subtitle="数据来自统一样张", notes="来源：自行编写的统一样张")
card(s, 0.72, 2.0, 5.7, 2.3, "学习目标", "解释变化原因并提出行动。")
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const slide = entries.get("ppt/slides/slide3.xml").toString();
    assert.match(slide, /type="title"/);
    assert.match(slide, /收入持续增长/);
    assert.ok(
      [...entries].some(
        ([name, bytes]) =>
          /^ppt\/notesSlides\/notesSlide\d+\.xml$/.test(name) &&
          bytes.toString().includes("自行编写"),
      ),
      "native speaker notes retain the declared source",
    );
    const layout = entries.get("ppt/slideLayouts/slideLayout3.xml").toString();
    assert.doesNotMatch(layout, /val="D9822B"|val="D5DDE6"/);
    assert.doesNotMatch(slide, /val="D9822B"/);
    assert.deepEqual(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
      [],
    );
  },
);

test(
  "Python refuses unadapted resources instead of silently returning navy",
  options,
  async (t) => {
    const result = await run(t, 'prs = new_deck(style="not-an-enabled-style")');
    assert.equal(result.status, 1);
    assert.match(result.stderr, /unknown or disabled style/);
  },
);

test(
  "Python emits native OMML fractions and relation shapes without raster media",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "原生对象")
equation(s, 0.8, 2.0, 4, 1, {"fraction": ["收益", "投入"]})
relation(s, ["输入", "处理", "输出"], 0.8, 4.0, 11.5, 1.1)
formula(s, 0.8, 5.7, 4, 0.7, ["legacy = text"])
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const slide = entries.get("ppt/slides/slide1.xml").toString();
    assert.match(slide, /<m:oMath/);
    assert.match(slide, /<m:f>/);
    assert.match(slide, /<mc:Choice Requires="a14">/);
    assert.match(slide, /name="Equation fallback \(text only\)"/);
    assert.match(slide, /<m:t\b[^>]*>收益<\/m:t>/);
    assert.match(slide, /<p:cxnSp>/);
    assert.match(slide, /<a:tailEnd type="triangle"/);
    assert.doesNotMatch(slide, /<a:headEnd/);
    assert.match(slide, /legacy = text/);
    assert.ok(
      ![...entries.keys()].some((name) => name.startsWith("ppt/media/")),
    );
    const verdict = await validate(t, result.out);
    assert.equal(verdict.success, true, JSON.stringify(verdict));
    assert.deepEqual(verdict.data.errors, []);
  },
);

test(
  "interleaved decks retain their own component palette",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(style="blue-professional")
s = content_slide(prs, "One deck")
other = new_deck(style="cartesian")
callout(s, 2, "Callout palette")
content_slide(other, "Other deck")
steps(s, [{"title": "A"}, {"title": "B"}], 0.8, 3.0, 11, 0.8)
content_slide(other, "Other page")
formula(s, 0.8, 4.2, 4, 0.7, ["a + b"])
content_slide(other, "Another page")
timeline(s, [{"label": "Q1", "title": "A"}, {"label": "Q2", "title": "B"}], 0.8, 5.2, 11, 1.5)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const slide = entries.get("ppt/slides/slide1.xml").toString();
    assert.match(slide, /val="F0ECD9"/);
    assert.match(slide, /val="1E2BFA"/);
    assert.doesNotMatch(slide, /val="E2DBD1"|val="1A1A1A"/);
  },
);

test(
  "architecture preserves alternating native panels and lesson pages have a separate objective region",
  options,
  async (t) => {
    const result = await run(
      t,
      `
architecture = new_deck(style="architectural-plan")
panel_slide(architecture, "现状", subtitle="右侧证据")
panel_slide(architecture, "建议", subtitle="左侧证据")
prs = new_deck(style="course-module")
s = lesson_slide(prs, "学习目标", objective="解释收入变化，并区分事实和假设。")
text(s, "主内容区", 3.65, 2.1, 8.5, 1, size=20)
# Switching back to another deck must restore its colors and independent page count.
panel_slide(architecture, "行动")
assert architecture.slides[2].shapes.title.text_frame.paragraphs[0].runs[0].font.color.rgb == rgb("18293B")
assert len(architecture.slides) == 3
panels = [next(sh for sh in page.shapes if sh.name.startswith("Alternating panel")) for page in architecture.slides]
# The source's first page has its panel on the right; the sides alternate by page.
assert panels[1].left == 0
assert panels[0].left == panels[2].left > Inches(8)
assert all(abs(panel.width / Inches(1) - 5.12) < 0.01 for panel in panels)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const slide = entries.get("ppt/slides/slide1.xml").toString();
    assert.match(slide, /name="Lesson objective region"/);
    assert.match(slide, /解释收入变化，并区分事实和假设。/);
    assert.match(slide, /主内容区/);
    assert.match(slide, /<a:ea typeface="SimSun"/);
    assert.match(slide, /<a:ea typeface="Microsoft YaHei"/);
    assert.match(slide, /<a:latin typeface="Georgia"/);
  },
);
