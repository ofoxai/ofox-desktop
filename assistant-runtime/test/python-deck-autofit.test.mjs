import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import {
  options,
  run,
  zipEntries,
  withoutDensity,
} from "./fixtures/python-engine.mjs";

const EMU = 914400;
// Heights (inches) of the drawn container shapes on a slide, in order.
const boxHeights = (xml, preset) =>
  [...xml.matchAll(/<p:sp>[\s\S]*?<\/p:sp>/g)]
    .map(([shape]) => shape)
    .filter((shape) => shape.includes(`prst="${preset}"`))
    .map((shape) => Number(/<a:ext cx="\d+" cy="(\d+)"/.exec(shape)[1]) / EMU);

test(
  "a card or step box that is more than half empty is drawn at its content's height and still reported",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="样张", style="navy", language="zh")
s = content_slide(prs, "三条约定")
card(s, 0.72, 2.0, 3.6, 4.2, "读完再来", "没读完也欢迎。", size=13)
card(s, 4.6, 2.0, 3.6, 1.6, "只谈文本", "观点要能回到书里。", size=13)
s2 = content_slide(prs, "四个步骤")
steps(s2, [{"title": "选书"}, {"title": "导读"}, {"title": "讨论"}], 0.72, 2.0, 11.9, 3.6)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = withoutDensity(
      JSON.parse(await readFile(result.lint, "utf8")),
    );
    assert.equal(lint.length, 2, lint.join("\n"));
    assert.match(lint[0], /card "读完再来".*drawn at/);
    assert.match(lint[1], /steps.*drawn at/);
    const entries = await zipEntries(result.out);
    const cards = boxHeights(
      entries.get("ppt/slides/slide1.xml").toString(),
      "roundRect",
    );
    // The half-empty card no longer keeps its 4.2 in box; the fitted one is unchanged.
    assert.ok(cards[0] < 1.8, String(cards[0]));
    assert.ok(Math.abs(cards[1] - 1.6) < 0.01, String(cards[1]));
    const boxes = boxHeights(
      entries.get("ppt/slides/slide2.xml").toString(),
      "roundRect",
    );
    assert.equal(boxes.length, 3);
    assert.ok(
      boxes.every((height) => height < 1.5),
      boxes.join(","),
    );
  },
);

test(
  "cards in one row keep one height: a fuller sibling restores the row",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="样张", style="navy", language="zh")
s = content_slide(prs, "三条约定")
card(s, 0.72, 2.0, 3.6, 3.0, "读完再来", "一行。", size=13)
card(s, 4.6, 2.0, 3.6, 3.0, "只谈文本", points=["观点要能回到书里的某一页", "每次只谈一个问题", "轮流发言", "不打断别人", "记录分歧"], size=13)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const heights = boxHeights(
      (await zipEntries(result.out)).get("ppt/slides/slide1.xml").toString(),
      "roundRect",
    );
    assert.deepEqual(
      heights.map((height) => Math.round(height * 100) / 100),
      [3, 3],
    );
  },
);

test(
  "a content page whose content stops in the upper half is reported, a filled page is not",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="样张", style="studio", language="zh")
cover_slide(prs, "封面")
s = content_slide(prs, "偏空的一页")
bullets(s, ["一条", "两条"], 0.72, 1.9, 6, 1.0, size=14)
s = content_slide(prs, "填满的一页")
bullets(s, ["一条", "两条", "三条", "四条"], 0.72, 1.9, 6, 4.2, size=24, gap=18)
statement_slide(prs, "一句主张")
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.equal(lint.length, 1, lint.join("\n"));
    assert.match(lint[0], /^slide 2: .*lower .* empty/);
  },
);

test(
  "a box drawn over a page's title is reported with the page and the title",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(title="样张", style="knowledge-arch-blueprint", language="zh")
s = closing_slide(prs, "今天想请大家讨论的四件事")
t = s.shapes.title
y = t.top / 914400
card(s, 0.72, y, 5.6, 1.6, "第一件", "一句说明。")
s2 = content_slide(prs, "标题之下的内容")
card(s2, 0.72, 2.0, 5.6, 4.6, "正常", points=["一", "二", "三", "四", "五", "六"])
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    const covering = lint.filter((line) => /covers the title/.test(line));
    assert.equal(covering.length, 1, lint.join("\n"));
    assert.match(covering[0], /^slide 1: .*今天想请大家讨论的四件事/);
  },
);
