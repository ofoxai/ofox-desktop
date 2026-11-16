import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { options, run, withoutDensity } from "./fixtures/python-engine.mjs";

// The five hand-written compositions (split, editorial, course, launch,
// blueprint) are checked on the native objects python-pptx reads back.
const HELPERS = `
def box(item): return (item.left / Inches(1), item.top / Inches(1), item.width / Inches(1), item.height / Inches(1))
def named(slide, prefix): return [item for item in slide.shapes if item.name.startswith(prefix)]
def fill(item): return str(item.fill.fore_color.rgb) if hasattr(item, "fill") and item.fill.type == 1 else None
def ink(item): return str(item.text_frame.paragraphs[0].runs[0].font.color.rgb)
def labelled(slide, value): return next(item for item in slide.shapes if item.has_text_frame and item.text.split("\\n")[0] == value)
def inside(item, region):
    x, y, w, h = box(item)
    return x >= region[0] - 0.01 and x + w <= region[0] + region[1] + 0.01
def paper(slide):
    panel = named(slide, "Alternating panel")[0]; x, _, w, _ = box(panel)
    return ((0, x) if x > 1 else (w, 13.333 - w)), (x, w)
`;

async function passes(t, code) {
  const result = await run(t, HELPERS + code);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(
    withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
    [],
  );
}

test(
  "an architectural cover and chapter keep title, kicker and rule on the paper beside the panel",
  options,
  (t) =>
    passes(
      t,
      `
prs = new_deck(style="architectural-plan")
for page in (1, 2):
    s = cover_slide(prs, "社区读书会年度计划", subtitle="副标题", kicker="示例样张", meta="示例组织")
    main, panel = paper(s)
    for item in s.shapes:
        if item.name in ("Background", "Panel navigation") or item.name.startswith("Alternating panel"): continue
        assert inside(item, main), f"page {page}: {item.name} leaves the paper: {box(item)}"
    assert ink(labelled(s, "示例样张")) == "18293B", "the kicker keeps a readable colour on the paper"
    assert len(s.shapes.title.text_frame.text) == 9 and s.shapes.title.height < Inches(1.2), "the cover title stays on one line"
s = section_slide(prs, "为什么要办读书会", number="01", subtitle="从三个观察说起")
main, panel = paper(s)
assert inside(s.shapes.title, main) and inside(labelled(s, "01"), main) and ink(labelled(s, "01")) == "18293B"
`,
    ),
);

test(
  "architectural content pages follow the source's panel rhythm and fill the panel",
  options,
  (t) =>
    passes(
      t,
      `
prs = new_deck(style="architectural-plan")
cover_slide(prs, "封面")
sides = []
for title in ("三条共读约定", "五件要做的事", "调整前后对比"):
    s = content_slide(prs, title, subtitle="按先后顺序排列")
    main, panel = paper(s); sides.append(named(s, "Alternating panel")[0].name.split()[-1])
    regions = slide_regions(s)
    # The panel is not hollow: the large page numeral, the native title and the subtitle live in it.
    navigation = named(s, "Panel navigation")[0]
    assert navigation.text_frame.paragraphs[0].runs[0].font.size >= Pt(48)
    for item in (navigation, s.shapes.title, labelled(s, "按先后顺序排列")):
        assert inside(item, panel), f"{item.name} is outside the panel"
    # Nothing but the page number placeholder crosses the panel edge: the rule stops there.
    for item in s.shapes:
        if item.name == "Background" or item.is_placeholder or item.name.startswith("Alternating panel"): continue
        assert inside(item, main) or inside(item, panel), f"{item.name} straddles the panel edge: {box(item)}"
    assert main[0] <= regions["main"]["x"] and regions["main"]["x"] + regions["main"]["w"] <= main[0] + main[1] + 0.01
    assert panel[0] <= regions["panel"]["x"] and regions["panel"]["x"] + regions["panel"]["w"] <= panel[0] + panel[1] + 0.01
    assert regions["main"]["h"] >= 5.5, "the body has the whole height of the main region"
    assert regions["panel"]["y"] >= (s.shapes.title.top + s.shapes.title.height) / Inches(1)
    # A short title keeps one line in the panel, with room to spare for bold glyphs.
    size = s.shapes.title.text_frame.paragraphs[0].runs[0].font.size / Pt(1)
    assert len(title) * size / 72 <= panel[1] - 1.4 - 0.3, f"{title} is set at {size} pt"
# The source's first page has the panel on the right, and the sides alternate by page.
assert sides == ["left", "right", "left"], sides
assert named(prs.slides[0], "Alternating panel")[0].name.endswith("right")
forced = panel_slide(prs, "强制左侧", side="left")
assert len(named(forced, "Alternating panel")) == 1 and len(named(forced, "Panel navigation")) == 1
main, panel = paper(forced)
assert panel[0] == 0 and inside(forced.shapes.title, panel) and slide_regions(forced)["main"]["x"] > 5.12
try:
    panel_slide(prs, "错误", side="top"); raise AssertionError("accepted an unknown side")
except ValueError: pass
assert len(prs.slides) == 5, "a rejected side leaves no page"
`,
    ),
);

test("an architectural deck closes on a whole dark page", options, (t) =>
  passes(
    t,
    `
prs = new_deck(style="architectural-plan")
cover_slide(prs, "封面")
s = closing_slide(prs, "谢谢", subtitle="欢迎提问与建议", meta="联系方式")
assert fill(named(s, "Background")[0]) == "18293B"
assert ink(s.shapes.title) == "FFFFFF" and ink(labelled(s, "欢迎提问与建议")) != "18293B"
assert fill(named(s, "Alternating panel")[0]) not in ("B5D5E3", "18293B")
# A page after it starts from the deck's own light colours again.
after = content_slide(prs, "后续")
assert fill(named(after, "Background")[0]) == "F4F7F9" and ink(after.shapes.title) == "18293B"
`,
  ),
);

test(
  "architectural content over the side panel is refused, with the regions to use instead",
  options,
  async (t) => {
    // Found in a real deck: cards drawn at full-width coordinates ran into
    // the panel and over its title; the warning did not stop the delivery.
    const page = (body) => `
prs = new_deck(style="architectural-plan", language="zh")
cover_slide(prs, "封面")
s = content_slide(prs, "四个服务", subtitle="按边界拆分")
${body}`;
    const crossing = await run(
      t,
      page('card(s, 0.72, 4.6, 5.6, 1.6, "用户服务", "账号与登录")'),
    );
    assert.equal(crossing.status, 1, "a card across the panel edge is refused");
    assert.match(
      crossing.stderr,
      /slide 2: "用户服务"[^;]*crosses the side panel's edge/,
    );
    // The regions are named with their numbers, so the fix needs no guessing.
    assert.match(
      crossing.stderr,
      /main x 5\.57–12\.61, y 1\.00–6\.70; panel x 0\.70–4\.42 from y \d\.\d\d/,
    );
    const title = await run(t, page('text(s, "补充说明", 0.8, 2.4, 3.2, 0.5)'));
    assert.equal(title.status, 1, "text over the panel's title is refused");
    assert.match(title.stderr, /"补充说明"[^;]*covers the side panel's title/);
    // Content in the main region, and in the panel below its title, is fine.
    await passes(
      t,
      page(`
b = slide_regions(s)["main"]
card(s, b["x"], b["y"] + 0.4, b["w"], 1.8, "用户服务", "账号与登录")
p = slide_regions(s)["panel"]
text(s, "边界先于代码", p["x"], p["y"], p["w"], 0.5)
`),
    );
  },
);

test(
  "a launch statement centres its kicker above the statement",
  options,
  (t) =>
    passes(
      t,
      `
prs = new_deck(style="product-launch")
s = statement_slide(prs, "读得慢一点，才能想得深一点。", kicker="核心主张")
kicker = labelled(s, "核心主张"); title = s.shapes.title
assert kicker.text_frame.paragraphs[0].alignment == PP_ALIGN.CENTER
assert (kicker.left, kicker.width) == (title.left, title.width) and kicker.top < title.top
# A left-aligned composition keeps its kicker on the left.
other = new_deck(style="cartesian")
s = statement_slide(other, "读得慢一点。", kicker="核心主张")
assert labelled(s, "核心主张").text_frame.paragraphs[0].alignment == PP_ALIGN.LEFT
`,
    ),
);

test(
  "a launch deck has whole dark content pages whose helpers stay readable",
  options,
  (t) =>
    passes(
      t,
      `
prs = new_deck(style="product-launch")
cover_slide(prs, "封面")
pages = []
for number in (2, 3, 4, 5):
    s = content_slide(prs, f"第 {number} 页", subtitle="副标题"); pages.append(s)
    card(s, 0.72, 1.9, 3.6, 1.6, "卡片", "卡片正文", size=14)
    kpi(s, 4.6, 1.9, 3.6, 1.6, "72", "人次")
    bullets(s, ["要点一", "要点二"], 8.6, 1.9, 4, 1.0, size=16)
    steps(s, [{"title": "选书", "body": "投票"}, {"title": "讨论", "body": "线下"}], 0.72, 3.8, 5.6, 1.6)
    native_chart(s, "bar", ["一", "二"], [{"name": "示例", "values": [3, 5]}], 6.8, 3.6, 5.8, 3.0)
light, _, dark, after = pages
# Every fourth page is wholly dark; its neighbours keep the light page with a dark title band.
assert fill(named(dark, "Background")[0]) == "0A0A12"
assert [fill(named(page, "Background")[0]) for page in (light, after)] == ["FFFFFF", "FFFFFF"]
assert not any(fill(item) == "0A0A12" and item.name != "Background" for item in dark.shapes), "a dark page needs no title band"
for value in ("卡片", "卡片正文", "72", "人次", "要点一", "选书", "投票", "副标题"):
    assert ink(labelled(dark, value)) in ("FFFFFF", "E4EAF0"), f"{value} is not light on the dark page"
    if value != "副标题": assert ink(labelled(after, value)) not in ("FFFFFF", "E4EAF0"), f"{value} stays dark on the next light page"
assert ink(dark.shapes.title) == "FFFFFF"
fills = {fill(item) for item in dark.shapes if not item.has_chart and item.name != "Background"}
assert not fills & {"FFF0E9", "FFFFFF"}, f"light containers on the dark page: {fills}"
chart = next(item for item in dark.shapes if item.has_chart).chart
assert str(chart.value_axis.tick_labels.font.color.rgb) == "FFFFFF"
assert str(chart.series[0].format.fill.fore_color.rgb) != "101018", "a near-black series disappears on the dark page"
`,
    ),
);

test(
  "each hand-written composition draws steps in its own card language",
  options,
  (t) =>
    passes(
      t,
      `
from pptx.enum.shapes import MSO_SHAPE_TYPE
items = [{"title": "选书", "body": "成员提名并投票"}, {"title": "导读", "body": "发出三个问题"}, {"title": "讨论", "body": "线下九十分钟"}]
looks = {}
for style in ("blue-professional", "cartesian", "course-module", "architectural-plan", "product-launch", "navy"):
    prs = new_deck(style=style); s = content_slide(prs, "步骤"); r = slide_regions(s)["main"]
    before = len(s.shapes); steps(s, items, r["x"], r["y"] + 0.3, r["w"], 1.6, highlight=1)
    drawn = list(s.shapes)[before:]
    kinds = [item.auto_shape_type for item in drawn if item.shape_type == MSO_SHAPE_TYPE.AUTO_SHAPE]
    looks[style] = {"arrows": sum(item.shape_type == MSO_SHAPE_TYPE.LINE for item in drawn), "kinds": set(kinds),
                    "boxes": [item for item in drawn if item.shape_type == MSO_SHAPE_TYPE.AUTO_SHAPE and item.height > Inches(1)],
                    "texts": [item.text for item in drawn if item.has_text_frame and item.text]}
    assert len(named(s, "Step")) == 3
assert looks["navy"]["arrows"] == 2 and looks["navy"]["kinds"] == {MSO_SHAPE.ROUNDED_RECTANGLE}, "the legacy template keeps its boxes"
for style in ("blue-professional", "cartesian", "course-module", "architectural-plan", "product-launch"):
    assert looks[style]["arrows"] == 0, f"{style} keeps generic arrows"
for style in ("blue-professional", "cartesian"):
    # Open ruled columns with numerals: rules only, no boxes.
    assert looks[style]["kinds"] == {MSO_SHAPE.RECTANGLE} and not looks[style]["boxes"], style
    assert "01" in looks[style]["texts"] and "03" in looks[style]["texts"]
for style in ("course-module", "architectural-plan"):
    assert looks[style]["kinds"] == {MSO_SHAPE.RECTANGLE} and len(looks[style]["boxes"]) >= 3 and "02" in looks[style]["texts"], style
assert MSO_SHAPE.OVAL in looks["product-launch"]["kinds"] and "2" in looks["product-launch"]["texts"]
assert fill(looks["product-launch"]["boxes"][1]) == "FF5A36", "the launch highlight is a bold accent block"
`,
    ),
);

test(
  "the blue professional quotation and closing are centred on plain paper",
  options,
  (t) =>
    passes(
      t,
      `
prs = new_deck(style="blue-professional")
quote = statement_slide(prs, "“每月一本书，比一年十二本更容易坚持。”", kicker="引语", attribution="示例成员")
closing = closing_slide(prs, "谢谢", subtitle="欢迎提问与建议", meta="联系方式")
for s, texts in ((quote, ("引语", "示例成员")), (closing, ("欢迎提问与建议", "联系方式"))):
    assert not any(fill(item) == "1E2BFA" and item.width > Inches(1) for item in s.shapes), "the cobalt panel is still drawn"
    title = s.shapes.title
    assert title.text_frame.paragraphs[0].alignment == PP_ALIGN.CENTER
    assert abs((title.left + title.width / 2) / Inches(1) - 13.333 / 2) < 0.01
    for value in texts:
        item = labelled(s, value)
        assert item.text_frame.paragraphs[0].alignment == PP_ALIGN.CENTER and (item.left, item.width) == (title.left, title.width)
assert quote.shapes.title.height < Inches(0.9), "the quotation is sized to avoid a one-word last line"
# The cover and chapter keep the cobalt panel.
for s in (cover_slide(prs, "封面"), section_slide(prs, "章节", number="01")):
    assert any(fill(item) == "1E2BFA" and item.width > Inches(3) for item in s.shapes)
`,
    ),
);

test(
  "a lesson rail lists objectives with progress dots and keeps a single sentence unchanged",
  options,
  (t) =>
    passes(
      t,
      `
prs = new_deck(style="course-module")
objectives = ["定义递归", "写出基线条件", "画出调用栈", "判断何时使用"]
s = lesson_slide(prs, "如何提出一个好问题", objectives, current=1)
rail = named(s, "Lesson objective region")[0]
dots = named(s, "Lesson objective dot"); current = named(s, "Lesson objective current")
assert len(dots) == 3 and len(current) == 1
assert all(dot.width <= Inches(0.14) and inside(dot, (box(rail)[0], box(rail)[2])) for dot in dots + current)
assert fill(current[0]) == "D88A3A" and fill(dots[0]) == "2D7D6E" and fill(dots[1]) is None and fill(dots[2]) is None
rows = [labelled(s, value) for value in objectives]
assert [row.top for row in rows] == sorted(row.top for row in rows) and len({row.top for row in rows}) == 4
assert all(inside(row, (box(rail)[0], box(rail)[2])) for row in rows)
assert rows[1].text_frame.paragraphs[0].runs[0].font.bold and not rows[0].text_frame.paragraphs[0].runs[0].font.bold
assert abs(current[0].top + current[0].height / 2 - rows[1].top - Inches(0.12)) < Inches(0.08)
assert slide_regions(s)["main"]["x"] > box(rail)[0] + box(rail)[2]
plain = lesson_slide(prs, "无进度", objectives)
assert len(named(plain, "Lesson objective dot")) == 4 and all(fill(dot) == "2D7D6E" for dot in named(plain, "Lesson objective dot"))
single = lesson_slide(prs, "一句话", "读完本页，你能写出一个开放式导读问题。")
sentence = labelled(single, "读完本页，你能写出一个开放式导读问题。")
assert not named(single, "Lesson objective dot") and sentence.text_frame.paragraphs[0].runs[0].font.size == Pt(16)
assert [round(value, 2) for value in box(sentence)] == [0.94, 2.9, 2.01, 3.35]
for bad in (lambda: lesson_slide(prs, "空", []), lambda: lesson_slide(prs, "混合", ["有效", {"text": "无效"}]),
            lambda: lesson_slide(prs, "越界", objectives, current=4), lambda: lesson_slide(prs, "单句", "一句话", current=0)):
    before = len(prs.slides)
    try:
        bad(); raise AssertionError("accepted an invalid objective")
    except ValueError: pass
    assert len(prs.slides) == before, "a rejected lesson left a partial slide"
`,
    ),
);
