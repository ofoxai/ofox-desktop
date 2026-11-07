import test from "node:test";
import assert from "node:assert/strict";
import { PAGE_TYPES, parseDeck } from "../src/deck/schema.mjs";
import { themeFor } from "../src/deck/theme.mjs";
import { CANVAS, CONTENT } from "../src/deck/layout.mjs";
import { heightCm, lineCount } from "../src/deck/text.mjs";
import { RENDERERS, renderDeck } from "../src/deck/render.mjs";

const navy = themeFor("navy");

// `count` CJK characters, so a text sits exactly at a limit.
const zh = (count) =>
  Array.from(
    { length: count },
    (_, index) => "幻灯片排版测试文字"[index % 9],
  ).join("");

// A deck as parseDeck hands it to the renderer.
function deck(slides, language = "zh") {
  const parsed = parseDeck({ kind: "deck", title: "测试", language, slides });
  assert.deepEqual(parsed.problems, undefined);
  return parsed.deck;
}

const sixPages = () =>
  deck([
    {
      type: "hero",
      title: "认识 Ofox",
      subtitle: "让想法更快变成答案与作品",
      meta: "团队分享 · 2026 年 11 月",
    },
    {
      type: "statement",
      statement: "从自动化到自主：为什么现在需要 agent",
      note: "可选的一句补充",
    },
    {
      type: "pillars",
      title: "三项核心能力",
      pillars: [
        { heading: "感知", body: "文本、图像、语音多模态输入" },
        { heading: "推理", body: "拆解任务，规划每一步" },
        { heading: "执行", body: "调用工具，交付结果" },
      ],
      notes: "先讲感知，再讲推理和执行。\n最后留一分钟提问。",
    },
    {
      type: "grid",
      title: "今天讲什么",
      items: [
        { heading: "背景", body: "为什么需要 agent" },
        { heading: "机制", body: "感知、推理、执行" },
        { heading: "结构", body: "模型加工具" },
        { heading: "应用", body: "写作、分析、编程" },
      ],
    },
    {
      type: "quote",
      quote: "把任务说清楚，剩下的交给它。",
      source: "用户反馈",
    },
    {
      type: "cta",
      title: "开始构建你的 agent",
      subtitle: "app.ofox.ai",
      contact: "hello@ofox.ai",
    },
  ]);

const headed = (count, heading, body) =>
  Array.from({ length: count }, () => ({
    heading: zh(heading),
    body: zh(body),
  }));

// Every field of the six text page types at its limit.
const atLimits = () =>
  deck([
    { type: "hero", title: zh(18), subtitle: zh(30), meta: zh(30) },
    { type: "statement", statement: zh(40), note: zh(60) },
    { type: "pillars", title: zh(22), pillars: headed(4, 12, 60) },
    { type: "grid", title: zh(22), items: headed(8, 10, 30) },
    { type: "quote", quote: zh(60), source: zh(20) },
    { type: "cta", title: zh(18), subtitle: zh(40), contact: zh(30) },
  ]);

const pillarsDeck = (count) =>
  deck([
    { type: "hero", title: "封面" },
    { type: "pillars", title: zh(22), pillars: headed(count, 12, 60) },
    { type: "cta", title: "谢谢" },
  ]);

const cm = (value) => {
  assert.match(value, /^-?\d+(\.\d+)?cm$/);
  return Number.parseFloat(value);
};

const shapes = (commands) =>
  commands.filter(
    (command) => command.command === "add" && command.type === "shape",
  );
const textShapes = (commands) =>
  shapes(commands).filter((command) => command.props.text !== undefined);
const onSlide = (commands, n) =>
  commands.filter((command) => command.parent === `/slide[${n}]`);
const named = (commands, name) => {
  const found = shapes(commands).find((command) => command.props.name === name);
  assert.ok(found, `no shape named ${name}`);
  return found;
};
const titleOf = (commands, n) =>
  commands.find(
    (command) =>
      command.command === "set" &&
      command.path === `/slide[${n}]/placeholder[title]`,
  );

// The height text.mjs gives a text shape's text, from what the renderer wrote.
function estimate(shape) {
  const { text, width, size, spaceAfter } = shape.props;
  const pt = Number(size);
  return heightCm(lineCount(text, cm(width), pt), pt, {
    paragraphs: text.split(/\r?\n/).length,
    spaceAfterPt: spaceAfter === undefined ? 0 : Number.parseFloat(spaceAfter),
  });
}

test("renderDeck makes one slide per page in order", () => {
  const { commands, slides } = renderDeck(sixPages(), navy);
  assert.equal(slides, 6);
  const added = commands.filter(
    (command) => command.command === "add" && command.type === "slide",
  );
  assert.equal(added.length, 6);
  for (const slide of added) assert.equal(slide.parent, "/");
  assert.deepEqual(
    added.map((slide) => slide.props.title),
    [
      "认识 Ofox",
      undefined,
      "三项核心能力",
      "今天讲什么",
      undefined,
      "开始构建你的 agent",
    ],
  );
  // Each command after the k-th `add slide` works on /slide[k].
  let current = 0;
  for (const command of commands) {
    if (command.command === "add" && command.type === "slide") {
      current += 1;
      continue;
    }
    const target = command.parent ?? command.path;
    assert.ok(
      target.startsWith(`/slide[${current}]`),
      `${target} follows slide ${current}`,
    );
  }
});

test("every text shape carries explicit font, size and color", () => {
  for (const make of [sixPages, atLimits]) {
    const { commands } = renderDeck(make(), navy);
    const texts = textShapes(commands);
    assert.ok(texts.length > 0);
    for (const shape of texts) {
      const { name, font, size, color } = shape.props;
      assert.ok(font, `${name} font`);
      assert.match(size, /^\d+$/, `${name} size`);
      assert.match(color, /^[0-9A-F]{6}$/, `${name} color`);
      assert.equal(shape.props["font.latin"], "Arial", `${name} font.latin`);
    }
  }
});

test("an English deck sets text in Arial with no separate Latin font", () => {
  const english = deck(
    [
      {
        type: "hero",
        title: "Meet Ofox",
        subtitle: "Ideas to answers, faster",
      },
      {
        type: "pillars",
        title: "Three abilities",
        pillars: [
          { heading: "Sense", body: "Text, images and voice in" },
          { heading: "Act", body: "Tools called, work delivered" },
        ],
      },
      { type: "cta", title: "Start building" },
    ],
    "en",
  );
  for (const shape of textShapes(renderDeck(english, navy).commands)) {
    assert.equal(shape.props.font, "Arial", shape.props.name);
    assert.equal(shape.props["font.latin"], undefined, shape.props.name);
  }
});

test("every shape stays inside the canvas", () => {
  for (const make of [sixPages, atLimits]) {
    const { commands } = renderDeck(make(), navy);
    const placed = [
      ...shapes(commands),
      ...commands.filter(
        (command) => command.command === "set" && command.props.x !== undefined,
      ),
    ];
    for (const command of placed) {
      const { x, y, width, height } = command.props;
      const where = command.props.name ?? command.path;
      assert.ok(cm(x) >= 0, `${where} x`);
      assert.ok(cm(y) >= 0, `${where} y`);
      assert.ok(cm(x) + cm(width) <= CANVAS.wCm, `${where} right edge`);
      assert.ok(cm(y) + cm(height) <= CANVAS.hCm, `${where} bottom edge`);
    }
  }
});

test("names are unique within a slide", () => {
  for (const make of [sixPages, atLimits]) {
    const { commands, slides } = renderDeck(make(), navy);
    for (let n = 1; n <= slides; n += 1) {
      const names = shapes(onSlide(commands, n)).map(
        (command) => command.props.name,
      );
      assert.equal(new Set(names).size, names.length, `slide ${n}: ${names}`);
      for (const name of names)
        assert.match(name, new RegExp(`^s${n}-[a-z]+(-[a-z]+)*(-\\d+)?$`));
    }
  }
});

test("dark pages use light text", () => {
  const { commands } = renderDeck(sixPages(), navy);
  const added = commands.filter(
    (command) => command.command === "add" && command.type === "slide",
  );
  for (const n of [1, 6]) {
    assert.equal(added[n - 1].props.background, "1E3A5F", `slide ${n}`);
    assert.equal(titleOf(commands, n).props.color, "FFFFFF", `slide ${n}`);
    for (const shape of textShapes(onSlide(commands, n)))
      assert.ok(
        [navy.colors.onDark, navy.colors.onDarkMuted].includes(
          shape.props.color,
        ),
        `${shape.props.name} is ${shape.props.color}`,
      );
  }
});

test("pillars at the limit fit without shrinking below 14pt", () => {
  const { commands } = renderDeck(pillarsDeck(4), navy);
  for (let i = 1; i <= 4; i += 1) {
    const heading = named(commands, `s2-heading-${i}`);
    const body = named(commands, `s2-body-${i}`);
    assert.ok(Number(body.props.size) >= 14, `body ${i} is ${body.props.size}`);
    assert.ok(
      estimate(heading) + estimate(body) <= CONTENT.bottom - CONTENT.top,
      `column ${i}`,
    );
  }
});

test("pillar body size follows the column count at the limit", () => {
  const bodySize = (count) =>
    named(renderDeck(pillarsDeck(count), navy).commands, "s2-body-1").props
      .size;
  assert.equal(bodySize(2), "18");
  assert.equal(bodySize(3), "16");
  assert.equal(bodySize(4), "14");
});

test("grid with eight items uses two rows", () => {
  const { commands } = renderDeck(atLimits(), navy);
  const cell = (i) => named(commands, `s4-card-${i}`).props;
  for (let i = 1; i <= 4; i += 1) {
    assert.equal(cell(i).y, cell(1).y, `cell ${i} is on the first row`);
    assert.ok(
      cm(cell(i + 4).y) >= cm(cell(1).y) + cm(cell(1).height),
      `cell ${i + 4} is below the first row`,
    );
    assert.equal(cell(i + 4).x, cell(i).x, `cell ${i + 4} is under cell ${i}`);
  }
});

test("notes become speaker notes", () => {
  const { commands } = renderDeck(sixPages(), navy);
  const notes = commands.filter((command) => command.type === "notes");
  assert.equal(notes.length, 1);
  assert.equal(notes[0].command, "add");
  assert.equal(notes[0].parent, "/slide[3]");
  assert.equal(
    notes[0].props.text,
    "先讲感知，再讲推理和执行。\n最后留一分钟提问。",
  );
});

test("text boxes are tall enough for their text at the limits", () => {
  const { commands } = renderDeck(atLimits(), navy);
  for (const shape of textShapes(commands))
    assert.ok(
      estimate(shape) <= cm(shape.props.height),
      `${shape.props.name}: ${estimate(shape)} > ${shape.props.height}`,
    );
});

test("only content pages carry the title rule and the page number", () => {
  const { commands } = renderDeck(sixPages(), navy);
  const has = (n, role) =>
    shapes(onSlide(commands, n)).some(
      (command) => command.props.name === `s${n}-${role}`,
    );
  for (const n of [3, 4]) {
    assert.ok(has(n, "title-rule"), `slide ${n} rule`);
    assert.equal(named(commands, `s${n}-page`).props.text, `${n} / 6`);
  }
  for (const n of [1, 2, 5, 6]) {
    assert.ok(!has(n, "title-rule"), `slide ${n} rule`);
    assert.ok(!has(n, "page"), `slide ${n} page number`);
  }
  const rule = named(commands, "s3-title-rule").props;
  assert.deepEqual(
    [rule.x, rule.y, rule.width, rule.height, rule.fill, rule.text],
    ["2.33cm", "4.4cm", "2cm", "0.12cm", "1E3A5F", undefined],
  );
});

test("there is a renderer for every page type", () => {
  assert.deepEqual(Object.keys(RENDERERS).sort(), [...PAGE_TYPES].sort());
});

test("renderDeck refuses a batch over 2000 commands", () => {
  const grid = deck([
    { type: "hero", title: "封面" },
    { type: "grid", title: zh(10), items: headed(8, 10, 30) },
    { type: "cta", title: "谢谢" },
  ]).slides[1];
  // Not a deck parseDeck would pass (it allows 30 pages): the guard is
  // defensive, so it is reached with more pages than that.
  const huge = { ...sixPages(), slides: Array(100).fill(grid) };
  assert.throws(
    () => renderDeck(huge, navy),
    /^Error: deck too large: \d+ commands$/,
  );
});

test("renderDeck fails fast on content that does not fit the page", () => {
  // Hand-built pages parseDeck would refuse (too many line breaks): row 2
  // of the grid would end past 19.05 cm, and the quote would start above 0.
  const lines = (count) => Array(count).fill("项目").join("\n");
  const grid = {
    type: "grid",
    title: "今天讲什么",
    items: Array.from({ length: 8 }, () => ({
      heading: "背景",
      body: lines(8),
    })),
  };
  const quote = { type: "quote", quote: lines(20) };
  const hero = { type: "hero", title: "封面" };
  assert.throws(
    () => renderDeck({ ...sixPages(), slides: [hero, grid] }, navy),
    /^Error: slide 2 \(grid\): content does not fit the page$/,
  );
  assert.throws(
    () => renderDeck({ ...sixPages(), slides: [hero, hero, quote] }, navy),
    /^Error: slide 3 \(quote\): content does not fit the page$/,
  );
});
