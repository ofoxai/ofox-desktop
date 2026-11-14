import test from "node:test";
import assert from "node:assert/strict";
import { layoutBox, slideText } from "../src/slide-text.mjs";

// A text box the way officecli's `view svg` writes one: a foreignObject in
// a translated group, a flex container, then one div per paragraph.
const textBox = (paragraphs, { justify = "flex-start", at = "0,0" } = {}) =>
  `<g transform="translate(${at})"><rect width="500" height="100" fill="none"/>` +
  `<foreignObject x="14.4" y="7.2" width="400" height="80">` +
  `<div xmlns="http://www.w3.org/1999/xhtml" style="width:100%;height:100%;overflow:hidden;display:flex;flex-direction:column;justify-content:${justify};line-height:1">` +
  paragraphs +
  `</div></foreignObject></g>`;
const slide = (
  body,
  size = 'width="1920" height="1080" viewBox="0 0 1920 1080"',
) =>
  `<svg xmlns="http://www.w3.org/2000/svg" ${size}>` +
  `<rect width="1920" height="1080" fill="#1E3A5F"/>${body}</svg>`;

test("slideText reads a text box at its place on the slide, with pt as 2 px", () => {
  const { width, height, boxes } = slideText(
    slide(
      textBox(
        `<div style="white-space:pre-wrap;margin:0;font-size:18pt;text-align:center;line-height:1.15">` +
          `<span style="font-family:'Arial';font-size:18pt;font-weight:bold;color:#FFFFFF">标题 Title</span></div>`,
        { justify: "center", at: "132.09,340.16" },
      ),
    ),
  );
  assert.deepEqual([width, height], [1920, 1080]);
  assert.equal(boxes.length, 1);
  const [box] = boxes;
  assert.ok(Math.abs(box.x - 146.49) < 1e-9, `x ${box.x}`);
  assert.ok(Math.abs(box.y - 347.36) < 1e-9, `y ${box.y}`);
  assert.deepEqual([box.width, box.height, box.valign], [400, 80, "middle"]);
  const [paragraph] = box.paragraphs;
  assert.deepEqual(
    [paragraph.align, paragraph.lineHeight, paragraph.size],
    ["center", 1.15, 36],
  );
  assert.deepEqual(paragraph.runs, [
    {
      text: "标题 Title",
      size: 36,
      family: "'Arial'",
      bold: true,
      italic: false,
      color: "#FFFFFF",
    },
  ]);
});

test("nested groups add up, as a table's cells do", () => {
  const { boxes } = slideText(
    slide(
      `<g transform="translate(850.4,56.69)">` +
        textBox(`<div style="font-size:16pt"><span>45%</span></div>`, {
          at: "0,226.76",
        }) +
        `</g>`,
    ),
  );
  assert.ok(Math.abs(boxes[0].y - (56.69 + 226.76 + 7.2)) < 1e-9);
  assert.ok(Math.abs(boxes[0].x - (850.4 + 14.4)) < 1e-9);
  assert.equal(boxes[0].paragraphs[0].runs[0].size, 32);
});

test("a list paragraph keeps its spacing and hanging indent in SVG units", () => {
  const { boxes } = slideText(
    slide(
      textBox(
        `<div style="font-size:18pt;margin-bottom:6pt;line-height:1.15;text-indent:-72px;margin-left:72px">` +
          `<span>• </span><span style="color:#333333">第一点</span></div>` +
          `<div style="font-size:18pt;margin-bottom:6pt;line-height:1.15;text-indent:-72px;margin-left:72px">` +
          `<span>• </span><span style="color:#333333">第二点</span></div>`,
      ),
    ),
  );
  const paragraphs = boxes[0].paragraphs;
  assert.equal(paragraphs.length, 2);
  assert.deepEqual(
    [
      paragraphs[0].marginBottom,
      paragraphs[0].marginLeft,
      paragraphs[0].textIndent,
    ],
    [12, 72, -72],
  );
  assert.deepEqual(
    paragraphs[1].runs.map((run) => [run.text, run.color, run.size]),
    [
      ["• ", "#000000", 36],
      ["第二点", "#333333", 36],
    ],
  );
});

test("a box under a rotation or inside a chart's nested svg is left out", () => {
  const paragraph = `<div style="font-size:18pt"><span>文字</span></div>`;
  const { boxes } = slideText(
    slide(
      // officecli writes a rotated shape's translate and rotate together.
      `<g transform="translate(793.7,283.46) rotate(30,226.77,56.7)">${textBox(paragraph)}</g>` +
        `<g transform="translate(10,10)"><svg x="0" y="0" width="100" height="100" viewBox="0 0 50 50">${textBox(paragraph)}</svg></g>` +
        textBox(paragraph, { at: "5,5" }),
    ),
  );
  assert.equal(boxes.length, 1);
  assert.ok(Math.abs(boxes[0].x - 19.4) < 1e-9);
});

test("whitespace between paragraphs is not a paragraph", () => {
  const { boxes } = slideText(
    slide(
      textBox(
        `\n  <div style="font-size:18pt"><span>一</span></div>\n  ` +
          `<div style="font-size:18pt"><span>二</span></div>\n`,
      ),
    ),
  );
  assert.deepEqual(
    boxes[0].paragraphs.map((paragraph) =>
      paragraph.runs.map((run) => run.text).join(""),
    ),
    ["一", "二"],
  );
});

test("a soft break officecli writes after its paragraph's runs adds no line", () => {
  const { boxes } = slideText(
    slide(
      textBox(
        `<div style="font-size:18pt"><span>soft</span><span>break</span><br/></div>`,
      ),
    ),
  );
  assert.deepEqual(
    boxes[0].paragraphs[0].runs.map((run) => run.text),
    ["soft", "break", "\n"],
  );
  const lines = layoutBox({ ...boxes[0], width: 1000 }, measure);
  assert.equal(lines.length, 1);
});

test("the slide's size is its viewBox, else its width and height", () => {
  const viewBoxOnly = slideText(slide("", 'viewBox="0 0 960 540"'));
  assert.deepEqual([viewBoxOnly.width, viewBoxOnly.height], [960, 540]);
  const sized = slideText(slide("", 'width="1440" height="1080"'));
  assert.deepEqual([sized.width, sized.height], [1440, 1080]);
});

test("SVG that is not XML is refused", () => {
  assert.throws(() => slideText("not an svg"));
  assert.throws(() => slideText('<svg><div style="x"></svg>'));
});

// Every character 10 wide; a 10-unit font has an 8 ascent and 2 descent.
const measure = (text, run) => ({
  width: [...text].length * run.size,
  ascent: run.size * 0.8,
  descent: run.size * 0.2,
});
const style = {
  size: 10,
  family: undefined,
  bold: false,
  italic: false,
  color: "#000",
};
const box = (texts, options = {}) => ({
  x: 100,
  y: 200,
  width: options.width ?? 50,
  height: options.height ?? 100,
  valign: options.valign ?? "top",
  paragraphs: texts.map((text) => ({
    align: options.align ?? "left",
    lineHeight: 1.5,
    size: 10,
    marginTop: 0,
    marginBottom: options.marginBottom ?? 0,
    marginLeft: options.marginLeft ?? 0,
    textIndent: options.textIndent ?? 0,
    runs: [{ ...style, text }],
  })),
});
const lineTexts = (lines) =>
  lines.map((line) => line.items.map((item) => item.text).join(""));

test("CJK text wraps by character at the box width", () => {
  const lines = layoutBox(box(["一二三四五六七"]), measure);
  assert.deepEqual(lineTexts(lines), ["一二三四五", "六七"]);
  // 15-unit lines with the 10-unit em box centered: baselines 2.5 + 8 down.
  assert.deepEqual(
    lines.map((line) => line.y),
    [210.5, 225.5],
  );
  assert.equal(lines[0].items[0].x, 100);
});

test("closing punctuation never starts a line", () => {
  const lines = layoutBox(box(["一二三四五、六"]), measure);
  assert.deepEqual(lineTexts(lines), ["一二三四", "五、六"]);
});

test("punctuation stays with Latin words and numbers too", () => {
  // Five 10-unit characters to a line.
  assert.deepEqual(lineTexts(layoutBox(box(["使用GPU，速度快"]), measure)), [
    "使用",
    "GPU，速",
    "度快",
  ]);
  // Six to a line, so the bracketed year fits one line and moves whole.
  assert.deepEqual(
    lineTexts(layoutBox(box(["阅读（2017）好"], { width: 60 }), measure)),
    ["阅读", "（2017）", "好"],
  );
});

test("Latin text wraps between words and centers without the trailing space", () => {
  const lines = layoutBox(
    box(["ab cd efgh"], { width: 60, align: "center" }),
    measure,
  );
  assert.deepEqual(lineTexts(lines), ["ab cd", "efgh"]);
  assert.equal(lines[0].items[0].x, 100 + (60 - 50) / 2);
  assert.equal(lines[1].items[0].x, 100 + (60 - 40) / 2);
});

test("a word wider than the box breaks inside it", () => {
  const lines = layoutBox(box(["abcdefgh"], { width: 30 }), measure);
  assert.deepEqual(lineTexts(lines), ["abc", "def", "gh"]);
});

test("a hanging indent starts the first line left of the rest", () => {
  const lines = layoutBox(
    box(["• 一二三四五六"], { width: 60, marginLeft: 20, textIndent: -20 }),
    measure,
  );
  assert.deepEqual(lineTexts(lines), ["• 一二三四", "五六"]);
  assert.equal(lines[0].items[0].x, 100);
  assert.equal(lines[1].items[0].x, 120);
});

test("a box's lines sit at its top, middle or bottom", () => {
  const firstY = (valign) =>
    layoutBox(box(["一", "二"], { valign, marginBottom: 5 }), measure)[0].y;
  // Two 15-unit lines with 5 after each: 40 of the box's 100.
  assert.equal(firstY("top"), 210.5);
  assert.equal(firstY("middle"), 210.5 + 30);
  assert.equal(firstY("bottom"), 210.5 + 60);
});

test("a hard line break starts a new line", () => {
  const lines = layoutBox(box(["一\n二"]), measure);
  assert.deepEqual(lineTexts(lines), ["一", "二"]);
});
