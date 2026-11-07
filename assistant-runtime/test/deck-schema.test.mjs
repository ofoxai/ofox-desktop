import test from "node:test";
import assert from "node:assert/strict";
import {
  LIMITS,
  PAGE_TYPES,
  deckOf,
  parseDeck,
  textLength,
} from "../src/deck/schema.mjs";

const zh = (count) => "字".repeat(count);

const HERO = { type: "hero", title: "认识 Ofox" };
const STATEMENT = { type: "statement", statement: "为什么现在需要 agent" };
const PILLARS = {
  type: "pillars",
  title: "三项核心能力",
  pillars: [
    { heading: "感知", body: "多模态输入" },
    { heading: "推理", body: "思维链" },
  ],
};
const CTA = { type: "cta", title: "开始构建" };

// A valid deck around the slides under test; the minimum is three pages.
function deckWith(...slides) {
  const filler = [HERO, STATEMENT, CTA];
  return {
    kind: "deck",
    title: "认识 Ofox",
    slides: [...slides, ...filler.slice(0, Math.max(0, 3 - slides.length))],
  };
}

const showcase = (media, extra = {}) => ({
  type: "showcase",
  title: "收入",
  media,
  ...extra,
});

const chart = (extra = {}) => ({
  kind: "bar",
  categories: ["Q1", "Q2"],
  series: [{ name: "收入", values: [12, 15] }],
  ...extra,
});

const PIXEL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

function problemsOf(deck, options) {
  const result = parseDeck(deck, options);
  assert.ok(result.problems, "expected the deck to be rejected");
  return result.problems;
}

function parsed(deck, options) {
  const result = parseDeck(deck, options);
  assert.equal(result.problems, undefined, result.problems?.join("\n"));
  return result;
}

test("textLength counts CJK as one and Latin as half", () => {
  assert.equal(textLength("Transformer 入门"), 8);
  assert.equal(textLength(""), 0);
});

test("textLength ignores line breaks and rounds up", () => {
  assert.equal(textLength("字\n字\r\n字"), 3);
  assert.equal(textLength("abc"), 2);
  assert.equal(textLength("，。ＡＢ"), 4);
  assert.equal(textLength("한글 豈"), 4);
});

test("the page types and limits are the documented ones", () => {
  assert.deepEqual(PAGE_TYPES, [
    "hero",
    "statement",
    "pillars",
    "evidence",
    "timeline",
    "comparison",
    "grid",
    "quote",
    "cta",
    "showcase",
  ]);
  assert.deepEqual(LIMITS.deck, { slides: [3, 30], notes: 300, title: 22 });
  assert.deepEqual(LIMITS.pillars, {
    count: [2, 4],
    heading: 12,
    body: 60,
  });
});

test("deckOf finds a deck object and its one-element wrapper", () => {
  const deck = { kind: "deck", slides: [] };
  assert.equal(deckOf(deck), deck);
  assert.equal(deckOf([deck]), deck);
  assert.equal(deckOf([{ command: "add", type: "slide" }]), null);
  assert.equal(deckOf("deck"), null);
  assert.equal(deckOf([deck, deck]), null);
  assert.equal(deckOf(null), null);
  assert.equal(deckOf([]), null);
});

function fullDeck() {
  return {
    kind: "deck",
    title: zh(22),
    language: "zh",
    theme: "navy",
    slides: [
      { type: "hero", title: zh(18), subtitle: zh(30), meta: zh(30) },
      { type: "statement", statement: zh(40), note: zh(60), notes: zh(300) },
      {
        type: "pillars",
        title: zh(22),
        pillars: Array.from({ length: 4 }, () => ({
          heading: zh(12),
          body: zh(60),
        })),
      },
      {
        type: "evidence",
        title: zh(22),
        metrics: Array.from({ length: 4 }, () => ({
          value: "118.5%",
          label: zh(12),
          note: zh(30),
        })),
        detail: [zh(40), zh(40), zh(40)],
      },
      {
        type: "timeline",
        title: zh(22),
        steps: Array.from({ length: 6 }, () => ({
          heading: zh(10),
          body: zh(30),
        })),
      },
      {
        type: "comparison",
        title: zh(22),
        left: { heading: zh(12), points: Array(4).fill(zh(30)) },
        right: { heading: zh(12), points: Array(4).fill(zh(30)) },
      },
      {
        type: "grid",
        title: zh(22),
        items: Array.from({ length: 8 }, () => ({
          heading: zh(10),
          body: zh(30),
        })),
      },
      { type: "quote", quote: zh(60), source: zh(20) },
      { type: "cta", title: zh(18), subtitle: zh(40), contact: zh(30) },
      showcase(
        {
          table: {
            columns: Array(6).fill(zh(8)),
            rows: Array.from({ length: 5 }, () => Array(6).fill(zh(12))),
          },
        },
        { caption: zh(30), body: zh(100) },
      ),
    ],
  };
}

test("parseDeck accepts every page type at its limits", () => {
  const { deck, warnings } = parsed(fullDeck());
  assert.equal(deck.slides.length, 10);
  assert.deepEqual(
    deck.slides.map((slide) => slide.type),
    PAGE_TYPES,
  );
  assert.deepEqual(warnings, []);
});

test("parseDeck accepts each showcase medium at its limits", () => {
  const labels = Array.from({ length: 8 }, (_, i) =>
    `类别${i}`.padEnd(12, "字"),
  );
  const eight = Array(8).fill(1);
  const { deck, warnings } = parsed(
    deckWith(
      showcase({
        chart: chart({
          kind: "line",
          categories: labels,
          series: ["甲", "乙", "丙"].map((name) => ({
            name: name.padEnd(12, "字"),
            values: eight,
          })),
        }),
      }),
      showcase({
        chart: chart({
          kind: "pie",
          categories: ["a", "b", "c", "d", "e", "f"],
          series: [{ name: "份额", values: [1, 2, 3, 4, 5, 6] }],
        }),
      }),
      showcase({ image: PIXEL }),
    ),
  );
  assert.equal(deck.slides.length, 3);
  assert.deepEqual(warnings, []);
});

test("parseDeck tolerates loose input", () => {
  const { deck, warnings } = parsed({
    kind: "deck",
    title: "认识 Ofox",
    slides: [
      HERO,
      {
        type: "Pillars",
        title: "三项核心能力",
        pillars: "感知: 多模态\n推理: 思维链",
        color: "red",
      },
      {
        type: "evidence",
        title: "数据",
        metrics: [{ value: 118, label: "NRR" }],
        detail: [{ text: "EMEA 三月上线" }, "续约率 +12%"],
      },
      showcase({
        chart: chart({ series: [{ name: "收入", values: "12, 15" }] }),
      }),
    ],
  });
  assert.equal(deck.slides[1].type, "pillars");
  assert.deepEqual(deck.slides[1].pillars, [
    { heading: "感知", body: "多模态" },
    { heading: "推理", body: "思维链" },
  ]);
  assert.equal(deck.slides[2].metrics[0].value, "118");
  assert.deepEqual(deck.slides[2].detail, ["EMEA 三月上线", "续约率 +12%"]);
  assert.deepEqual(deck.slides[3].media.chart.series[0].values, [12, 15]);
  assert.ok(warnings.includes('slide 2: ignored field "color"'));
});

test("parseDeck splits list fields given as one string by line", () => {
  const { deck } = parsed(
    deckWith({
      type: "comparison",
      title: "RNN 与 Transformer",
      left: { heading: "RNN", points: "逐词计算\n\n长句衰减\n" },
      right: { heading: "Transformer", points: ["并行计算"] },
    }),
  );
  assert.deepEqual(deck.slides[0].left.points, ["逐词计算", "长句衰减"]);
});

test("parseDeck accepts numbers where text is expected", () => {
  const { deck } = parsed(
    deckWith(
      showcase({
        table: { columns: ["年份", "收入"], rows: [[2024, 12.5]] },
      }),
      showcase({
        chart: chart({ categories: [2023, 2024] }),
      }),
    ),
  );
  assert.deepEqual(deck.slides[0].media.table.rows, [["2024", "12.5"]]);
  assert.deepEqual(deck.slides[1].media.chart.categories, ["2023", "2024"]);
});

test("parseDeck defaults the theme and falls back for the language", () => {
  const plain = parsed(deckWith(PILLARS)).deck;
  assert.equal(plain.theme, "navy");
  assert.equal(plain.language, "zh");

  const odd = parsed(
    { ...deckWith(PILLARS), language: "fr" },
    {
      fallbackLanguage: "en",
    },
  );
  assert.equal(odd.deck.language, "en");
  assert.deepEqual(odd.warnings, []);

  const japanese = parsed({ ...deckWith(PILLARS), language: "ja" }).deck;
  assert.equal(japanese.language, "ja");
});

test("parseDeck warns about an unknown theme and uses navy", () => {
  const { deck, warnings } = parsed({ ...deckWith(PILLARS), theme: "neon" });
  assert.equal(deck.theme, "navy");
  assert.deepEqual(warnings, ['deck: unknown theme "neon"; using navy']);
});

test("parseDeck warns about unknown fields at every level", () => {
  const { warnings } = parsed({
    ...deckWith({
      ...PILLARS,
      pillars: [
        { heading: "感知", body: "多模态", icon: "eye" },
        { heading: "推理", body: "思维链" },
      ],
    }),
    author: "me",
  });
  assert.deepEqual(warnings, [
    'deck: ignored field "author"',
    'slide 1: ignored field "pillars[0].icon"',
  ]);
});

test("parseDeck rejects an unknown page type and lists the types", () => {
  const problems = problemsOf(deckWith(HERO, { type: "bullets" }, CTA));
  assert.equal(
    problems[0],
    'slide 2: unknown type "bullets"; use one of hero, statement, pillars, evidence, timeline, comparison, grid, quote, cta, showcase',
  );
  assert.equal(problems.length, 1);
});

test("parseDeck rejects a slide without a type or that is not an object", () => {
  const problems = problemsOf(deckWith(HERO, { title: "x" }, "text"));
  assert.match(problems[0], /^slide 2: type is missing; use one of hero,/);
  assert.equal(problems[1], "slide 3: must be an object");
});

test("parseDeck rejects overlong text and names the slide, field and limit", () => {
  const long = {
    ...PILLARS,
    pillars: [
      { heading: "感知", body: "多模态" },
      { heading: "推理", body: zh(61) },
    ],
  };
  assert.deepEqual(problemsOf(deckWith(long)), [
    "slide 1 (pillars): pillars[1].body is 61 characters; the limit is 60",
  ]);
});

test("parseDeck counts Latin text at half width when it checks the limit", () => {
  const latin = { ...HERO, title: "a".repeat(36) };
  parsed(deckWith(latin));
  assert.deepEqual(problemsOf(deckWith({ ...HERO, title: "a".repeat(38) })), [
    "slide 1 (hero): title is 19 characters; the limit is 18",
  ]);
});

test("parseDeck counts an evidence value in characters, not CJK units", () => {
  const evidence = (value) => ({
    type: "evidence",
    title: "数据",
    metrics: [{ value, label: "NRR" }],
  });
  parsed(deckWith(evidence("118.5%")));
  assert.deepEqual(problemsOf(deckWith(evidence("1,118.5%"))), [
    "slide 1 (evidence): metrics[0].value is 8 characters; the limit is 6",
  ]);
});

test("parseDeck rejects a missing or empty required field", () => {
  assert.deepEqual(
    problemsOf(
      deckWith({
        type: "pillars",
        pillars: [
          { heading: "感知", body: "  " },
          { heading: "推理", body: "思维链" },
        ],
      }),
    ),
    [
      "slide 1 (pillars): title is missing",
      "slide 1 (pillars): pillars[0].body is empty",
    ],
  );
});

test("parseDeck rejects text that is neither a string nor a number", () => {
  assert.deepEqual(problemsOf(deckWith({ ...HERO, title: { text: "x" } })), [
    "slide 1 (hero): title must be text",
  ]);
});

test("parseDeck rejects too many or too few items", () => {
  const many = (count) =>
    Array.from({ length: count }, (_, i) => ({ heading: `h${i}`, body: "b" }));

  assert.deepEqual(
    problemsOf(
      deckWith(HERO, STATEMENT, CTA, { ...PILLARS, pillars: many(5) }),
    ),
    ["slide 4 (pillars): 5 pillars; the limit is 2–4"],
  );
  assert.deepEqual(
    problemsOf(deckWith({ type: "timeline", title: "节奏", steps: many(2) })),
    ["slide 1 (timeline): 2 steps; the limit is 3–6"],
  );
  assert.deepEqual(
    problemsOf(
      deckWith({
        type: "evidence",
        title: "数据",
        metrics: [{ value: "1", label: "a" }],
        detail: ["1", "2", "3", "4"],
      }),
    ),
    ["slide 1 (evidence): 4 detail; the limit is 3"],
  );
  assert.deepEqual(problemsOf(deckWith({ ...PILLARS, pillars: [] })), [
    "slide 1 (pillars): 0 pillars; the limit is 2–4",
  ]);
});

test("parseDeck rejects a deck with too many or too few slides", () => {
  const slides = (count) => ({
    kind: "deck",
    title: "认识 Ofox",
    slides: Array(count).fill(HERO),
  });
  assert.deepEqual(problemsOf(slides(31)), [
    "deck: 31 slides; the limit is 3–30; split it into two decks",
  ]);
  assert.deepEqual(problemsOf(slides(2)), [
    "deck: 2 slides; the limit is 3–30",
  ]);
  parsed(slides(3));
  parsed(slides(30));
});

test("parseDeck rejects a deck without slides, title or an object", () => {
  assert.deepEqual(problemsOf({ kind: "deck", title: "认识 Ofox" }), [
    "deck: slides is missing",
  ]);
  assert.deepEqual(
    problemsOf({ kind: "deck", title: "认识 Ofox", slides: "hero" }),
    ["deck: slides must be a list of slide objects"],
  );
  assert.deepEqual(problemsOf({ kind: "deck", slides: [HERO, HERO, HERO] }), [
    "deck: title is missing",
  ]);
  assert.deepEqual(problemsOf(null), ["deck must be an object"]);
});

test("parseDeck rejects overlong speaker notes", () => {
  assert.deepEqual(problemsOf(deckWith({ ...HERO, notes: zh(301) })), [
    "slide 1 (hero): notes is 301 characters; the limit is 300",
  ]);
});

test("parseDeck reports every problem in one pass", () => {
  const problems = problemsOf(
    deckWith(
      { type: "bullets" },
      { ...PILLARS, pillars: PILLARS.pillars.slice(0, 1) },
      { ...HERO, title: zh(19) },
    ),
  );
  assert.equal(problems.length, 3);
  assert.match(problems[0], /^slide 1: unknown type/);
  assert.match(problems[1], /^slide 2 \(pillars\): 1 pillars/);
  assert.match(problems[2], /^slide 3 \(hero\): title is 19/);
});

test("parseDeck rejects chart labels with separators", () => {
  const problems = problemsOf(
    deckWith(
      showcase({
        chart: chart({
          categories: ["Q1,Q2", "H1;H2"],
          series: [
            { name: "收入:预算", values: [1, 2] },
            { name: "利润", values: [1, 2] },
          ],
        }),
      }),
    ),
  );
  assert.deepEqual(problems, [
    'slide 1 (showcase): media.chart.categories[0] is "Q1,Q2"; it must not contain , ; or :',
    'slide 1 (showcase): media.chart.categories[1] is "H1;H2"; it must not contain , ; or :',
    'slide 1 (showcase): media.chart.series[0].name is "收入:预算"; it must not contain , ; or :',
  ]);
});

test("parseDeck rejects a chart with an unknown kind, or limits exceeded", () => {
  const kind = problemsOf(
    deckWith(showcase({ chart: chart({ kind: "scatter" }) })),
  );
  assert.deepEqual(kind, [
    'slide 1 (showcase): media.chart.kind is "scatter"; use one of bar, line, pie',
  ]);

  const nine = Array.from({ length: 9 }, (_, i) => `c${i}`);
  const four = ["a", "b", "c", "d"].map((name) => ({
    name,
    values: Array(9).fill(1),
  }));
  assert.deepEqual(
    problemsOf(
      deckWith(showcase({ chart: chart({ categories: nine, series: four }) })),
    ),
    [
      "slide 1 (showcase): 9 media.chart.categories; the limit is 1–8",
      "slide 1 (showcase): 4 media.chart.series; the limit is 1–3",
    ],
  );

  const pie = (categories) =>
    deckWith(
      showcase({
        chart: chart({
          kind: "pie",
          categories,
          series: [{ name: "份额", values: categories.map(() => 1) }],
        }),
      }),
    );
  assert.deepEqual(problemsOf(pie(["a"])), [
    "slide 1 (showcase): 1 media.chart.categories; the limit is 2–6",
  ]);
  assert.deepEqual(problemsOf(pie(["a", "b", "c", "d", "e", "f", "g"])), [
    "slide 1 (showcase): 7 media.chart.categories; the limit is 2–6",
  ]);
});

test("parseDeck rejects chart values that do not match the categories", () => {
  assert.deepEqual(
    problemsOf(
      deckWith(
        showcase({
          chart: chart({
            series: [
              { name: "收入", values: [1, 2, 3] },
              { name: "利润", values: "1, abc" },
            ],
          }),
        }),
      ),
    ),
    [
      "slide 1 (showcase): media.chart.series[0].values has 3 values; the chart has 2 categories",
      'slide 1 (showcase): media.chart.series[1].values[1] is "abc"; it must be a number',
    ],
  );
});

test("parseDeck rejects a table whose rows do not match the columns", () => {
  const table = (rows) =>
    deckWith(showcase({ table: { columns: ["年份", "收入"], rows } }));
  assert.deepEqual(problemsOf(table([["2024", "12"], ["2025"]])), [
    "slide 1 (showcase): media.table.rows[1] has 1 cells; the table has 2 columns",
  ]);
  assert.deepEqual(problemsOf(table([["2024", "12", "x"]])), [
    "slide 1 (showcase): media.table.rows[0] has 3 cells; the table has 2 columns",
  ]);
});

test("parseDeck rejects a table that is too big", () => {
  const cell = (n) => Array(n).fill("a");
  const sevenColumns = problemsOf(
    deckWith(showcase({ table: { columns: cell(7), rows: [cell(7)] } })),
  );
  assert.deepEqual(sevenColumns, [
    "slide 1 (showcase): 7 media.table.columns; the limit is 1–6",
  ]);
  const sixRows = problemsOf(
    deckWith(
      showcase({
        table: {
          columns: cell(2),
          rows: Array.from({ length: 6 }, () => cell(2)),
        },
      }),
    ),
  );
  assert.deepEqual(sixRows, [
    "slide 1 (showcase): 6 media.table.rows; the limit is 1–5",
  ]);
  assert.deepEqual(
    problemsOf(
      deckWith(
        showcase({
          table: { columns: [zh(9)], rows: [[zh(13)]] },
        }),
      ),
    ),
    [
      "slide 1 (showcase): media.table.columns[0] is 9 characters; the limit is 8",
      "slide 1 (showcase): media.table.rows[0][0] is 13 characters; the limit is 12",
    ],
  );
});

test("parseDeck requires exactly one of image, chart or table", () => {
  const none = problemsOf(deckWith(showcase({})));
  assert.deepEqual(none, [
    "slide 1 (showcase): media must hold exactly one of image, chart or table; it holds none",
  ]);
  const two = problemsOf(deckWith(showcase({ image: PIXEL, chart: chart() })));
  assert.deepEqual(two, [
    "slide 1 (showcase): media must hold exactly one of image, chart or table; it holds image, chart",
  ]);
  assert.deepEqual(problemsOf(deckWith(showcase("chart"))), [
    "slide 1 (showcase): media must hold exactly one of image, chart or table; it holds none",
  ]);
});

test("parseDeck accepts only base64 png, jpeg or svg images", () => {
  for (const mime of ["png", "jpeg", "svg+xml"]) {
    parsed(deckWith(showcase({ image: `data:image/${mime};base64,AAAA` })));
  }
  for (const image of [
    "attachment:abc",
    "data:image/gif;base64,AAAA",
    "data:image/png,AAAA",
  ]) {
    const problems = problemsOf(deckWith(showcase({ image })));
    assert.equal(problems.length, 1);
    assert.match(
      problems[0],
      /^slide 1 \(showcase\): media\.image must be a base64 data URI of a png, jpeg or svg\+xml image; it starts with "/,
    );
  }
});

test("parseDeck never rewrites text", () => {
  const input = deckWith({
    ...HERO,
    title: "  Transformer  入门  ",
    subtitle: "第一行\n第二行 ",
    notes: "\n 讲稿  内容 ",
  });
  const snapshot = structuredClone(input);
  const { deck } = parsed(input);
  assert.equal(deck.slides[0].title, "Transformer  入门");
  assert.equal(deck.slides[0].subtitle, "第一行\n第二行");
  assert.equal(deck.slides[0].notes, "讲稿  内容");
  assert.deepEqual(input, snapshot);
});

test("parseDeck output is itself a deck that parses to the same deck", () => {
  const first = parsed(fullDeck());
  assert.ok(deckOf(first.deck));
  const second = parsed(first.deck);
  assert.deepEqual(second.deck, first.deck);
  assert.deepEqual(second.warnings, []);
});

test("parseDeck drops optional fields that are empty", () => {
  const { deck } = parsed(deckWith({ ...HERO, subtitle: "  ", meta: "" }));
  assert.deepEqual(deck.slides[0], { type: "hero", title: "认识 Ofox" });
});
