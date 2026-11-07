import test from "node:test";
import assert from "node:assert/strict";
import { PAGE_TYPES, parseDeck } from "../src/deck/schema.mjs";
import { themeFor } from "../src/deck/theme.mjs";
import { CANVAS, CONTENT, GAP_CM } from "../src/deck/layout.mjs";
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

// `count` CJK units of English text (two Latin characters to a unit, with
// spaces), so a Latin text sits exactly at a limit.
const en = (count) =>
  Array.from({ length: 2 * count }, (_, index) => "Slide text "[index % 11])
    .join("")
    .replace(/ $/, "x");

const headed = (count, heading, body, text = zh) =>
  Array.from({ length: count }, () => ({
    heading: text(heading),
    body: text(body),
  }));

const PIXEL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const PNG_4_BY_3 =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAEElEQVR4nGOQs4qHIwacHACkYQiVfa22owAAAABJRU5ErkJggg==";
const svgUri = (attributes) =>
  `data:image/svg+xml;base64,${Buffer.from(
    `<svg xmlns="http://www.w3.org/2000/svg" ${attributes}/>`,
  ).toString("base64")}`;

// The three showcase media, each at its limits, in `text`.
const MEDIA_AT_LIMITS = {
  table: (text) => ({
    table: {
      columns: Array(6).fill(text(8)),
      rows: Array.from({ length: 5 }, () => Array(6).fill(text(10))),
    },
  }),
  chart: (text) => ({
    chart: {
      kind: "bar",
      categories: Array.from({ length: 8 }, () => text(12)),
      series: Array.from({ length: 3 }, () => ({
        name: text(12),
        values: [1, 2, 3, 4, 5, 6, 7, 8],
      })),
    },
  }),
  image: () => ({ image: PIXEL }),
};

// One page of each of the ten types, every field at its limit, in `text`
// (zh or en); the showcase holds `media`.
const tenAtLimits = (text, media = "table") => [
  { type: "hero", title: text(18), subtitle: text(30), meta: text(30) },
  { type: "statement", statement: text(40), note: text(60) },
  { type: "pillars", title: text(22), pillars: headed(4, 12, 60, text) },
  {
    type: "evidence",
    title: text(22),
    metrics: Array.from({ length: 4 }, () => ({
      value: text(3),
      label: text(12),
      note: text(30),
    })),
    detail: [text(40), text(40), text(40)],
  },
  { type: "timeline", title: text(22), steps: headed(6, 10, 30, text) },
  {
    type: "comparison",
    title: text(22),
    left: { heading: text(12), points: Array(4).fill(text(30)) },
    right: { heading: text(12), points: Array(4).fill(text(30)) },
  },
  { type: "grid", title: text(22), items: headed(8, 10, 30, text) },
  { type: "quote", quote: text(60), source: text(20) },
  { type: "cta", title: text(18), subtitle: text(40), contact: text(30) },
  {
    type: "showcase",
    title: text(22),
    media: MEDIA_AT_LIMITS[media](text),
    caption: text(30),
    body: text(100),
  },
];

const zhTen = () => deck(tenAtLimits(zh));
const enTen = () => deck(tenAtLimits(en), "en");

// A three-page deck around one page under test, which is slide 2.
const around = (page, language = "zh") =>
  deck(
    [{ type: "hero", title: "封面" }, page, { type: "cta", title: "谢谢" }],
    language,
  );

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

const added = (commands, type) =>
  commands.filter(
    (command) => command.command === "add" && command.type === type,
  );
const shapes = (commands) => added(commands, "shape");
// Every added element that sits at a place on the slide.
const placed = (commands) =>
  commands.filter(
    (command) => command.command === "add" && command.props?.x !== undefined,
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
  for (const make of [sixPages, atLimits, zhTen]) {
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
  for (const commands of [english, enTen()].map(
    (made) => renderDeck(made, navy).commands,
  )) {
    for (const shape of textShapes(commands)) {
      assert.equal(shape.props.font, "Arial", shape.props.name);
      assert.equal(shape.props["font.latin"], undefined, shape.props.name);
    }
  }
});

test("every shape stays inside the canvas", () => {
  for (const make of [sixPages, atLimits, zhTen, enTen]) {
    const { commands } = renderDeck(make(), navy);
    const boxes = [
      ...placed(commands),
      ...commands.filter(
        (command) => command.command === "set" && command.props.x !== undefined,
      ),
    ];
    for (const command of boxes) {
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
  for (const make of [sixPages, atLimits, zhTen]) {
    const { commands, slides } = renderDeck(make(), navy);
    for (let n = 1; n <= slides; n += 1) {
      const names = placed(onSlide(commands, n)).map(
        (command) => command.props.name,
      );
      assert.equal(new Set(names).size, names.length, `slide ${n}: ${names}`);
      for (const name of names)
        assert.match(
          name,
          new RegExp(`^s${n}-[a-z]+(-[a-z]+)*(-\\d+(-[a-z]+)?)?$`),
        );
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

// --- Data pages: evidence, timeline, comparison, showcase -------------------

const near = (actual, expected, message) =>
  assert.ok(
    Math.abs(actual - expected) < 0.002,
    `${message}: ${actual} is not ${expected}`,
  );
const bottomOf = (props) => cm(props.y) + cm(props.height);

const evidence = (count) =>
  around({
    type: "evidence",
    title: "关键数据",
    metrics: Array.from({ length: count }, (_, index) => ({
      value: "118.5%",
      label: `指标 ${index + 1}`,
    })),
  });

test("evidence picks the KPI size by count and fits the value", () => {
  const valueOf = (made) =>
    named(renderDeck(made, navy).commands, "s2-value-1").props;
  assert.equal(valueOf(evidence(4)).size, "48");
  assert.equal(valueOf(evidence(3)).size, "60");
  assert.equal(valueOf(evidence(2)).size, "72");
  assert.equal(valueOf(evidence(1)).size, "72");
  // Past the schema's limit (3 units), six CJK characters need
  // 6 × 72 × 0.0353 = 15.2 cm, more than a 14.2 cm column holds, so the
  // value steps down the KPI ladder to stay on one line.
  const wide = evidence(2);
  wide.slides[1].metrics[0].value = "一百一十八万";
  assert.equal(valueOf(wide).size, "60");
});

test("evidence at every limit keeps its values on one line above the content foot", () => {
  // Four metrics: 3-unit values (3 CJK, or 6 Latin), 12-unit labels,
  // 30-unit notes and three 40-unit detail lines.
  for (const made of [zhTen(), enTen()]) {
    const { commands } = renderDeck(made, navy);
    const value = named(commands, "s4-value-1").props;
    assert.equal(value.size, "48");
    assert.equal(lineCount(value.text, cm(value.width), 48), 1, value.text);
    const detail = named(commands, "s4-detail").props;
    assert.ok(
      bottomOf(detail) <= CONTENT.bottom,
      `ends at ${bottomOf(detail)}`,
    );
  }
});

test("evidence stacks value, label and note and lists the detail under them", () => {
  const page = {
    type: "evidence",
    title: "关键数据",
    metrics: [
      { value: "118%", label: "净收入留存", note: "同比" },
      { value: "3.2亿", label: "年度收入" },
    ],
  };
  const alone = renderDeck(around(page), navy).commands;
  const value = named(alone, "s2-value-1").props;
  const label = named(alone, "s2-label-1").props;
  const note = named(alone, "s2-note-1").props;
  assert.equal(value.color, navy.colors.primary);
  assert.deepEqual([label.size, label.bold], ["16", "true"]);
  assert.deepEqual([note.size, note.color], ["14", navy.colors.textSecondary]);
  near(cm(label.y), bottomOf(value), "label under value");
  near(cm(note.y), bottomOf(label), "note under label");
  assert.ok(!shapes(alone).some((shape) => shape.props.name === "s2-note-2"));
  // Without detail the metrics sit in the middle of the content area.
  near(
    (cm(value.y) + bottomOf(note)) / 2,
    (CONTENT.top + CONTENT.bottom) / 2,
    "metrics centered",
  );

  const detailed = renderDeck(
    around({ ...page, detail: ["EMEA 三月上线", "续约率 +12%"] }),
    navy,
  ).commands;
  const detail = named(detailed, "s2-detail").props;
  assert.deepEqual(
    [detail.text, detail.list, detail.size, detail.spaceAfter],
    ["EMEA 三月上线\n续约率 +12%", "bullet", "18", "6pt"],
  );
  // With detail the list sits a gap under the metrics, and the two are
  // centered together in the content area.
  const top = named(detailed, "s2-value-1").props;
  const under = named(detailed, "s2-note-1").props;
  near(cm(detail.y), bottomOf(under) + GAP_CM, "detail a gap under them");
  near(
    (cm(top.y) + bottomOf(detail)) / 2,
    (CONTENT.top + CONTENT.bottom) / 2,
    "metrics and detail centered",
  );
});

test("timeline draws one numbered card per step on a track", () => {
  const steps = Array.from({ length: 6 }, (_, index) => ({
    heading: `第 ${index + 1} 步`,
    body: "运营牵头",
  }));
  const { commands } = renderDeck(
    around({ type: "timeline", title: "推进节奏", steps }),
    navy,
  );
  const page = shapes(onSlide(commands, 2));
  const numbers = [1, 2, 3, 4, 5, 6];
  const marks = page.filter((shape) => shape.props.preset === "ellipse");
  assert.deepEqual(
    marks.map((mark) => [mark.props.name, mark.props.text]),
    numbers.map((i) => [`s2-step-${i}-no`, String(i)]),
  );
  for (const { props } of marks)
    assert.deepEqual(
      [props.fill, props.color, props.size, props.bold, props.width],
      [navy.colors.primary, "FFFFFF", "20", "true", "1.5cm"],
    );
  const headings = numbers.map((i) => named(commands, `s2-heading-${i}`));
  for (const { props } of headings)
    assert.deepEqual([props.size, props.bold], ["16", "true"]);
  assert.equal(named(commands, "s2-body-1").props.size, "14");

  const cards = numbers.map((i) => named(commands, `s2-card-${i}`).props);
  cards.forEach((card, index) => {
    const mark = marks[index].props;
    if (index > 0) assert.ok(cm(card.x) > cm(cards[index - 1].x), "x grows");
    near(
      cm(mark.x) + cm(mark.width) / 2,
      cm(card.x) + cm(card.width) / 2,
      `mark ${index + 1} centered over its card`,
    );
    assert.ok(bottomOf(mark) <= cm(card.y), `mark ${index + 1} above card`);
  });

  const tracks = page.filter((shape) => shape.props.name === "s2-track");
  assert.equal(tracks.length, 1);
  const track = tracks[0].props;
  const [first, last] = [marks[0].props, marks[5].props];
  assert.deepEqual(
    [track.fill, track.height],
    [navy.colors.secondary, "0.12cm"],
  );
  near(
    cm(track.y) + cm(track.height) / 2,
    cm(first.y) + cm(first.height) / 2,
    "track at the mark centers",
  );
  near(cm(track.x), cm(first.x) + 0.75, "track starts at the first center");
  near(
    cm(track.x) + cm(track.width),
    cm(last.x) + 0.75,
    "track ends at the last center",
  );
  // Added first, so the marks are drawn over it.
  assert.ok(page.indexOf(tracks[0]) < page.indexOf(marks[0]));
});

test("comparison puts left and right headings on contrasting bars", () => {
  const { commands } = renderDeck(
    around({
      type: "comparison",
      title: "RNN 与 Transformer",
      left: { heading: "RNN", points: ["逐词计算", "长句衰减"] },
      right: { heading: "Transformer", points: ["并行计算", "全句可见"] },
    }),
    navy,
  );
  const { colors } = navy;
  const left = named(commands, "s2-left-heading").props;
  const right = named(commands, "s2-right-heading").props;
  assert.deepEqual(
    [left.text, left.fill, left.color],
    ["RNN", colors.primary, colors.onDark],
  );
  assert.deepEqual(
    [right.text, right.fill, right.color],
    ["Transformer", colors.secondary, colors.text],
  );
  for (const bar of [left, right])
    assert.deepEqual([bar.size, bar.bold, bar.height], ["20", "true", "1.4cm"]);
  assert.ok(cm(right.x) > cm(left.x) + cm(left.width), "two columns");

  const points = named(commands, "s2-left-points").props;
  assert.deepEqual(
    [points.text, points.list, points.size],
    ["逐词计算\n长句衰减", "bullet", "18"],
  );
  assert.ok(cm(points.y) >= bottomOf(left), "points under the bar");
  assert.equal(named(commands, "s2-right-points").props.x, right.x);

  const vs = named(commands, "s2-vs");
  assert.deepEqual(
    [vs.props.text, vs.props.preset, vs.props.fill, vs.props.color],
    ["VS", "ellipse", colors.card, colors.primary],
  );
  assert.deepEqual(
    [vs.props.size, vs.props.bold, vs.props.width, vs.props.height],
    ["20", "true", "2cm", "2cm"],
  );
  near(
    cm(vs.props.x) + 1,
    CONTENT.x + CONTENT.w / 2,
    "VS centered between the columns",
  );
  near(
    cm(vs.props.y) + 1,
    cm(left.y) + cm(left.height) / 2,
    "VS at the heading height",
  );
  // Added after both bars, so it is drawn over their inner ends.
  const order = shapes(commands);
  assert.ok(
    order.indexOf(vs) > order.indexOf(named(commands, "s2-right-heading")),
  );
});

const showcasePage = (media, extra = {}) =>
  around({ type: "showcase", title: "收入", media, ...extra });

test("showcase chart maps kinds, colors and data", () => {
  const chartOf = (chart) =>
    added(renderDeck(showcasePage({ chart }), navy).commands, "chart")[0].props;
  const bar = chartOf({
    kind: "bar",
    categories: ["Q1", "Q2"],
    series: [{ name: "收入", values: [12, 15] }],
  });
  assert.deepEqual(
    [bar.name, bar.chartType, bar.categories, bar.data, bar.colors],
    ["s2-chart", "column", "Q1,Q2", "收入:12,15", "1E3A5F"],
  );
  assert.deepEqual(
    [bar.legend, bar.title, bar.dataLabels],
    ["none", "none", undefined],
  );

  const line = chartOf({
    kind: "line",
    categories: ["Q1", "Q2"],
    series: [
      { name: "收入", values: [12, 15] },
      { name: "成本", values: [8, 9.5] },
    ],
  });
  assert.deepEqual(
    [line.chartType, line.data, line.colors, line.legend],
    ["line", "收入:12,15;成本:8,9.5", "1E3A5F,D4A84B", "bottom"],
  );

  // A pie colors its slices, names them in the legend and labels values.
  const pie = chartOf({
    kind: "pie",
    categories: ["华东", "华南", "华北", "西部"],
    series: [{ name: "份额", values: [40, 30, 20, 10] }],
  });
  assert.deepEqual(
    [pie.chartType, pie.colors, pie.legend, pie.dataLabels],
    ["pie", "1E3A5F,D4A84B,3498DB,7F9CC0", "bottom", "value"],
  );
});

test("showcase table quotes cells with separators and sizes by column count", () => {
  const tableOf = (columns, rows) => {
    const { commands } = renderDeck(
      showcasePage({ table: { columns, rows } }),
      navy,
    );
    const path = "/slide[2]/table[@name=s2-table]";
    const set = (target) =>
      commands.find(
        (command) => command.command === "set" && command.path === target,
      );
    return {
      table: added(commands, "table")[0].props,
      style: set(path)?.props,
      header: set(`${path}/tr[1]`)?.props,
    };
  };
  const six = ["A", "B", "C", "D", "E", "F"];
  const { table, style, header } = tableOf(six, [
    ["a,b", 'say "hi"', "x;y", "1", "2", "3"],
  ]);
  assert.equal(table.name, "s2-table");
  assert.equal(table.data, 'A,B,C,D,E,F;"a,b","say ""hi""","x;y",1,2,3');
  assert.equal(table.colWidths, Array(6).fill("3.333cm").join(","));
  assert.deepEqual(
    [table.headerFill, table.bodyFill],
    [navy.colors.primary, navy.colors.card],
  );
  assert.deepEqual(style, {
    font: "Microsoft YaHei",
    "font.latin": "Arial",
    size: "13",
    color: navy.colors.text,
    valign: "middle",
  });
  assert.deepEqual(header, { bold: "true", color: "FFFFFF" });

  const sizeAt = (count) =>
    tableOf(six.slice(0, count), [six.slice(0, count)]).style.size;
  assert.equal(sizeAt(1), "16");
  assert.equal(sizeAt(4), "16");
  assert.equal(sizeAt(5), "14");
});

test("showcase table rows are tall enough for the longest cell", () => {
  const { commands } = renderDeck(
    showcasePage(MEDIA_AT_LIMITS.table(zh)),
    navy,
  );
  const table = added(commands, "table")[0].props;
  // A 10-character cell takes two lines of 13pt in a 3.33 cm column:
  // 2 × 13 × 0.0466 + 0.254 = 1.466 cm, so six rows of 1.5 cm.
  assert.equal(lineCount(zh(10), 20 / 6, 13), 2);
  assert.equal(table.height, "9cm");
  assert.ok(bottomOf(table) <= CONTENT.top + 11);
});

test("showcase image passes the data URI and an alt", () => {
  const imageOf = (extra) =>
    added(
      renderDeck(showcasePage({ image: PIXEL }, extra), navy).commands,
      "picture",
    )[0].props;
  const captioned = imageOf({ caption: "新版首页" });
  assert.deepEqual(
    [captioned.name, captioned.src, captioned.alt],
    ["s2-image", PIXEL, "新版首页"],
  );
  assert.equal(imageOf({}).alt, "收入");
});

test("showcase fits a picture in the media box by its proportions", () => {
  const boxOf = (image, extra) => {
    const { props } = added(
      renderDeck(showcasePage({ image }, extra), navy).commands,
      "picture",
    )[0];
    return [props.x, props.y, props.width, props.height];
  };
  const beside = { body: "说明" };
  // 4:3 beside a body: 11 cm tall, 11 × 4 / 3 = 14.667 cm wide, centered
  // across the 20 cm box at x 2.33.
  assert.deepEqual(boxOf(PNG_4_BY_3, beside), [
    "4.997cm",
    "5cm",
    "14.667cm",
    "11cm",
  ]);
  // 3:1: 20 cm wide, 6.667 cm tall, centered down the 11 cm box.
  assert.deepEqual(boxOf(svgUri('viewBox="0 0 300 100"'), beside), [
    "2.33cm",
    "7.167cm",
    "20cm",
    "6.667cm",
  ]);
  // 1:1 without a body: an 11 cm square in the middle of the centered box.
  assert.deepEqual(boxOf(PIXEL), ["11.435cm", "5cm", "11cm", "11cm"]);
  // A picture whose size cannot be read fills the box.
  assert.deepEqual(boxOf(svgUri(""), beside), [
    "2.33cm",
    "5cm",
    "20cm",
    "11cm",
  ]);
});

test("showcase puts the body beside the media and the caption under it", () => {
  const chart = {
    kind: "bar",
    categories: ["Q1", "Q2"],
    series: [{ name: "收入", values: [12, 15] }],
  };
  const extra = { caption: "单位：百万", body: zh(100) };
  const { commands } = renderDeck(showcasePage({ chart }, extra), navy);
  const media = added(commands, "chart")[0].props;
  assert.deepEqual(
    [media.x, media.y, media.width, media.height],
    ["2.33cm", "5cm", "20cm", "11cm"],
  );
  const body = named(commands, "s2-body").props;
  assert.deepEqual([body.x, body.y, body.width], ["23.09cm", "5cm", "8.45cm"]);
  assert.ok(navy.sizes.body.includes(Number(body.size)), body.size);
  assert.ok(cm(body.height) <= 11, body.height);
  const caption = named(commands, "s2-caption").props;
  assert.deepEqual(
    [caption.x, caption.y, caption.width, caption.size, caption.color],
    ["2.33cm", "16.3cm", "20cm", "14", navy.colors.textMuted],
  );

  // Without a body the media and its caption move to the middle.
  const alone = renderDeck(
    showcasePage({ chart }, { caption: "单位：百万" }),
    navy,
  ).commands;
  const middle = CONTENT.x + (CONTENT.w - 20) / 2;
  near(cm(added(alone, "chart")[0].props.x), middle, "media centered");
  near(cm(named(alone, "s2-caption").props.x), middle, "caption follows");
});

test("thirty full slides stay within the batch limits", () => {
  const pages = ["table", "chart", "image"].flatMap((media) =>
    tenAtLimits(zh, media).map((page) => ({ ...page, notes: zh(300) })),
  );
  const { commands, slides } = renderDeck(deck(pages), navy);
  assert.equal(slides, 30);
  assert.ok(commands.length <= 2000, `${commands.length} commands`);
  const bytes = Buffer.byteLength(JSON.stringify(commands));
  assert.ok(bytes <= 2 * 1024 * 1024, `${bytes} bytes`);
});

test("every renderer fits its content at the limits", () => {
  const decks = [
    zhTen(),
    enTen(),
    deck(tenAtLimits(zh, "chart")),
    deck(tenAtLimits(en, "image"), "en"),
  ];
  for (const made of decks) {
    const { commands } = renderDeck(made, navy);
    for (const shape of textShapes(commands))
      assert.ok(
        estimate(shape) <= cm(shape.props.height),
        `${shape.props.name}: ${estimate(shape)} > ${shape.props.height}`,
      );
    // On the content pages (pillars, evidence, timeline, comparison, grid
    // and showcase) every box but the title rule and the page number lies
    // within the content area.
    for (const n of [3, 4, 5, 6, 7, 10])
      for (const { props } of placed(onSlide(commands, n))) {
        if ([`s${n}-title-rule`, `s${n}-page`].includes(props.name)) continue;
        assert.ok(
          cm(props.y) >= CONTENT.top,
          `${props.name} starts at ${props.y}`,
        );
        assert.ok(
          bottomOf(props) <= CONTENT.bottom,
          `${props.name} ends at ${bottomOf(props)}`,
        );
      }
  }
});
