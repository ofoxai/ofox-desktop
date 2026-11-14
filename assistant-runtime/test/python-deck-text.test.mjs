import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { options, run, zipEntries } from "./fixtures/python-engine.mjs";

test("PPT guidance keeps core formulas native on diagram-led pages and makes text types explicit", async () => {
  const skill = await readFile(
    new URL("../skills/ofox-office-pptx/SKILL.md", import.meta.url),
    "utf8",
  );
  assert.match(skill, /Core formulas use `equation\(\)` even on diagram pages/);
  assert.match(skill, /Text fields must be strings; objects fail/);
  assert.match(skill, /relation[^\n]+string labels/);
});

test(
  "relation rejects structured objects instead of publishing dictionary representations",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(style="course-module")
s = content_slide(prs, "原生流程")
for items in [
    [{"label":"1", "title":"Attention", "body":""}, {"label":"2", "title":"Norm", "body":""}],
    ["合法第一项", {"text":"错误第二项"}], ["甲", 2], ["甲", None], "甲乙",
]:
    before = len(s.shapes)
    try:
        relation(s, items, 0.8, 2, 11.5, 1.2)
        raise AssertionError("relation accepted non-string labels")
    except ValueError as error:
        assert "relation" in str(error) and "string" in str(error)
    assert len(s.shapes) == before, "rejected relation left partial objects"
relation(s, ["Attention", "Norm"], 0.8, 2, 11.5, 1.2)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    assert.doesNotMatch(xml, /\{'label':|\{'text':/);
    assert.match(xml, /<a:t>Attention<\/a:t>/);
  },
);

test(
  "text validates every rich-text item before adding a textbox",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "类型检查")
for content in [{"text":"不是顶层富文本"}, 123, None, ["先合法", {"text":{"label":"泄漏"}}], ["先合法", {"text":["嵌套"]}], ["先合法", {"body":"缺少text"}], ["先合法", 42]]:
    before = len(s.shapes)
    try:
        text(s, content, 1, 2, 10, 1)
        raise AssertionError("text accepted a non-string payload")
    except ValueError as error:
        assert "text" in str(error) and "string" in str(error)
    assert len(s.shapes) == before, "rejected text left a partial textbox"
text(s, "合法正文", 1, 2, 10, 1)
`,
    );
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "composed text helpers reject wrong body types before adding decorations or slides",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(style="course-module")
s = content_slide(prs, "组合正文")
bad = {"label":"不要转为字典字符串"}
draws = [
    lambda: card(s, 1, 2, 5, 2, "标题", body=bad),
    lambda: card(s, 1, 2, 5, 2, "标题", points=["有效", {"text":bad}]),
    lambda: bullets(s, ["有效", {"text":bad}], 1, 2, 5, 2),
    lambda: steps(s, [{"title":"一", "body":"正文"}, {"title":"二", "body":bad}], 1, 2, 10, 2),
    lambda: timeline(s, [{"label":"1", "title":"一", "body":"正文"}, {"label":"2", "title":"二", "body":bad}], 1, 2, 10, 2),
    lambda: lesson_slide(prs, "目标", objective=bad),
    lambda: callout(s, 2, bad),
    lambda: kpi(s, 1, 2, 5, 2, "42", "数字", note=bad),
]
for draw in draws:
    before_shapes, before_slides = len(s.shapes), len(prs.slides)
    try:
        draw()
        raise AssertionError("body helper accepted a dictionary")
    except ValueError as error:
        assert "string" in str(error)
    assert len(s.shapes) == before_shapes, "rejected helper left partial objects"
    assert len(prs.slides) == before_slides, "rejected lesson left a partial slide"
`,
    );
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "valid rich text, structured step fields, numeric KPIs and intentional literal text remain native text",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "合法文本")
text(s, ["首段", {"text":"富文本", "bold":True, "color":"D9822B"}], 0.8, 1.8, 4, 0.8)
text(s, r"用户原文 {'label': 'literal'} \\n 字面标记", 0.8, 2.8, 8, 0.5)
bullets(s, ["要点", {"text":"强调", "bold":True}], 0.8, 3.4, 4, 0.8)
card(s, 5.2, 1.8, 6.5, 1.8, "标题", body="正文")
card(s, 5.2, 3.8, 6.5, 1.8, "要点卡", points=["字符串", {"text":"富文本要点"}])
kpi(s, 0.8, 4.5, 3.8, 2, 42, "给定数字")
s2 = content_slide(prs, "结构化字段")
steps(s2, [{"title":"起点", "body":"已知输入"}, {"title":"终点", "body":"得到输出"}], 0.8, 1.8, 11.5, 1.3)
timeline(s2, [{"label":"1", "title":"第一阶段", "body":"已知"}, {"label":"2", "title":"第二阶段", "body":"输出"}], 0.8, 3.4, 11.5, 2.8)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    assert.match(xml, /<a:t>富文本<\/a:t>/);
    assert.match(xml, /b="1"/);
    assert.match(xml, /val="D9822B"/);
    assert.match(xml, /用户原文 \{'label': 'literal'\} \\n 字面标记/);
    assert.match(xml, /<a:t>42<\/a:t>/);
    assert.doesNotMatch(xml, /<a:t>\{'text':/);
  },
);
