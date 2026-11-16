// The deck object: what the model sends instead of officecli commands. It
// names a page type per slide and fills in plain text; the renderer lays
// the pages out. This module reads that object.
//
// Reading is loose about shape and strict about size. A string where a list
// belongs is split into lines, a number where text belongs becomes text, an
// unknown field is dropped with a warning. Text is only ever trimmed: a deck
// whose text is too long is refused as a whole, with one sentence per
// problem naming the slide, field and limit, so the model can shorten it and
// send the whole deck again. Nothing is truncated, substituted or reworded.
// A line break takes no room in the length count but a line on the slide,
// so a short field (a title, heading, label, metric note, list item, table
// cell or chart label) must be one line and a body-like field holds at most
// two line breaks (LIMITS.lineBreaks).

import { DEFAULT_THEME, THEME_NAMES } from "./theme.mjs";

export const PAGE_TYPES = [
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
];

// Limits are in Chinese-character units (see textLength): a KPI value of 3
// is 6 Latin characters or 3 CJK, which keeps it on one line of 48pt in a
// quarter of the content width. `count: [min, max]` bounds a list; `detail` and `points` read as
// `[at most this many, each at most this long]`. A table's `rows` counts
// the header row too. `lineBreaks` bounds the line breaks inside one
// text field, `short` for one-line fields and `body` for body-like ones
// (see LINE_BREAKS).
export const LIMITS = {
  deck: { slides: [3, 30], notes: 300, title: 22 },
  lineBreaks: { short: 0, body: 2 },
  hero: { title: 18, subtitle: 30, meta: 30 },
  statement: { statement: 40, note: 60 },
  pillars: { count: [2, 4], heading: 12, body: 60 },
  evidence: {
    count: [1, 4],
    value: 3,
    label: 12,
    note: 30,
    detail: [3, 40],
  },
  timeline: { count: [3, 6], heading: 10, body: 30 },
  comparison: { heading: 12, points: [4, 30] },
  grid: { count: [3, 8], heading: 10, body: 30 },
  quote: { quote: 60, source: 20 },
  cta: { title: 18, subtitle: 40, contact: 30 },
  showcase: {
    caption: 30,
    body: 100,
    chart: { categories: 8, series: 3, pie: [2, 6], label: 12 },
    table: { columns: 6, rows: 6, header: 8, cell: 10 },
  },
};

const LANGUAGES = ["zh", "en", "ja"];
const CHART_KINDS = ["bar", "line", "pie"];
const MEDIA = ["image", "chart", "table"];
const IMAGE_URI = /^data:image\/(png|jpeg|svg\+xml);base64,/;
// officecli separates chart categories, series and values with these.
const CHART_SEPARATORS = /[,;:]/;

// The fields each page type reads; anything else is ignored.
const FIELDS = {
  hero: ["title", "subtitle", "meta"],
  statement: ["statement", "note"],
  pillars: ["title", "pillars"],
  evidence: ["title", "metrics", "detail"],
  timeline: ["title", "steps"],
  comparison: ["title", "left", "right"],
  grid: ["title", "items"],
  quote: ["quote", "source"],
  cta: ["title", "subtitle", "contact"],
  showcase: ["title", "media", "caption", "body"],
};

const CJK = /[\u2e80-\u9fff\uac00-\ud7af\uf900-\ufaff\uff00-\uffef]/u;

// The width of a text in Chinese-character units: a CJK character is one,
// anything else half, rounded up; line breaks take no room.
export function textLength(text) {
  let units = 0;
  for (const char of text) {
    if (char === "\n" || char === "\r") continue;
    units += CJK.test(char) ? 1 : 0.5;
  }
  return Math.ceil(units);
}

const isObject = (value) =>
  typeof value === "object" && value !== null && !Array.isArray(value);

// An object that is a deck: its `kind` is "deck" in any case, or it has no
// `kind` and a `slides` list. A batch command has neither.
function deckShaped(value) {
  if (!isObject(value)) return false;
  if (value.kind === undefined) return Array.isArray(value.slides);
  return (
    typeof value.kind === "string" && value.kind.trim().toLowerCase() === "deck"
  );
}

// create_document's content is a deck object, or that object alone in an
// array (some models wrap a single object), or that object serialized as a
// JSON string; a batch of commands is none of these.
export function deckOf(content) {
  let given = content;
  if (typeof given === "string") {
    try {
      given = JSON.parse(given);
    } catch {
      return null;
    }
  }
  const candidate =
    Array.isArray(given) && given.length === 1 ? given[0] : given;
  return deckShaped(candidate) ? candidate : null;
}

// A literal \n or \r\n (a backslash and a letter), as a model writes a line
// break when it escapes twice, is a line break, as the host does: it reads
// them so in text and notes (create.rs unescape_newlines), so every string
// of the deck (titles too) reads them so here, before any limit or estimate
// sees it. A lone \r is left as it is (`C:\reports`), as the host leaves it.
const LITERAL_NEWLINE = /\\r\\n|\\n/g;

// `value` with every literal \n and \r\n in its strings (at any depth)
// turned into a line break; nothing else changes and `value` is not
// modified.
export function normalizeLiteralNewlines(value) {
  if (typeof value === "string") return value.replace(LITERAL_NEWLINE, "\n");
  if (Array.isArray(value)) return value.map(normalizeLiteralNewlines);
  if (isObject(value))
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [
        key,
        normalizeLiteralNewlines(item),
      ]),
    );
  return value;
}

// Problems and warnings name where they are: "deck", "slide 3" and, once the
// page type is known, "slide 3 (pillars)". Slides count from 1 as a reader
// would; list items count from 0 as in the JSON the model wrote.
const problem = (ctx, message) =>
  ctx.problems.push(
    `${ctx.scope}${ctx.type ? ` (${ctx.type})` : ""}: ${message}`,
  );

const warn = (ctx, message) => ctx.warnings.push(`${ctx.scope}: ${message}`);

const compact = (object) =>
  Object.fromEntries(
    Object.entries(object).filter(([, value]) => value !== undefined),
  );

// Unknown fields are dropped, not kept and not fatal.
function ignore(ctx, object, known, path = "") {
  for (const key of Object.keys(object)) {
    if (!known.includes(key))
      warn(ctx, `ignored field "${path ? `${path}.` : ""}${key}"`);
  }
}

// Which LIMITS.lineBreaks bound a text field by its name (the last part of
// its path). List items, table cells and chart labels, whose paths end in
// an index, and a metric's note, named like a statement's, are short too:
// they say so with str's `breaks`. Speaker notes are not bounded.
const LINE_BREAKS = {
  title: "short",
  subtitle: "short",
  meta: "short",
  heading: "short",
  statement: "short",
  quote: "short",
  source: "short",
  contact: "short",
  label: "short",
  value: "short",
  caption: "short",
  body: "body",
  note: "body",
};

const lineBreaks = (text) => text.match(/\r\n|\r|\n/g)?.length ?? 0;

// A text field: a string or a number, trimmed, within the limit and its
// line-break bound, which `breaks` names when the field's name does not
// (see LINE_BREAKS). A blank optional field is absent.
function str(ctx, value, { field, limit, required = false, breaks }) {
  if (value === undefined || value === null) {
    if (required) problem(ctx, `${field} is missing`);
    return undefined;
  }
  const given =
    typeof value === "number" && Number.isFinite(value) ? String(value) : value;
  if (typeof given !== "string") {
    problem(ctx, `${field} must be text`);
    return undefined;
  }
  const text = given.trim();
  if (!text) {
    if (required) problem(ctx, `${field} is empty`);
    return undefined;
  }
  const size = textLength(text);
  if (size > limit)
    problem(ctx, `${field} is ${size} characters; the limit is ${limit}`);
  const bound = breaks ?? LINE_BREAKS[field.split(".").at(-1)];
  const allowed = LIMITS.lineBreaks[bound];
  const count = lineBreaks(text);
  if (allowed !== undefined && count > allowed)
    problem(
      ctx,
      `${field} has ${count} line break${count === 1 ? "" : "s"}; the limit is ${allowed}`,
    );
  return text;
}

// The text field `key` of `source`, named `path` in messages.
const required = (ctx, source, key, limit, path = key) =>
  str(ctx, source[key], { field: path, limit, required: true });
const optional = (ctx, source, key, limit, path = key) =>
  str(ctx, source[key], { field: path, limit });

// A list field: an array, or a string with one item per line. `each` turns
// an element into its normalized form and reports what is wrong with it.
function list(ctx, value, { field, count: [min, max], each }) {
  if (value === undefined || value === null) {
    if (min > 0) problem(ctx, `${field} is missing`);
    return undefined;
  }
  const elements =
    typeof value === "string"
      ? value
          .split(/\r?\n/)
          .map((line) => line.trim())
          .filter(Boolean)
      : value;
  if (!Array.isArray(elements)) {
    problem(ctx, `${field} must be a list`);
    return undefined;
  }
  if (elements.length < min || elements.length > max)
    problem(
      ctx,
      `${elements.length} ${field}; the limit is ${min > 0 ? `${min}–${max}` : max}`,
    );
  if (elements.length === 0 && min === 0) return undefined;
  return elements.map((element, index) => each(element, `${field}[${index}]`));
}

// A list of one-line items; an element may be { text } instead of a string.
function bullets(ctx, value, { field, count, limit }) {
  return list(ctx, value, {
    field,
    count,
    each: (element, path) => {
      if (isObject(element)) ignore(ctx, element, ["text"], path);
      return str(ctx, isObject(element) ? element.text : element, {
        field: path,
        limit,
        required: true,
        breaks: "short",
      });
    },
  });
}

// "感知: 多模态" is shorthand for { heading: "感知", body: "多模态" }.
function fromLine(line) {
  const at = line.search(/[:：]/);
  return at < 0
    ? { heading: line }
    : { heading: line.slice(0, at), body: line.slice(at + 1) };
}

// Pillars, timeline steps and grid items share one shape.
function headed(ctx, value, { field, count, heading, body }) {
  return list(ctx, value, {
    field,
    count,
    each: (element, path) => {
      const source = typeof element === "string" ? fromLine(element) : element;
      if (!isObject(source)) {
        problem(ctx, `${path} must be an object with heading and body`);
        return undefined;
      }
      ignore(ctx, source, ["heading", "body"], path);
      return {
        heading: required(ctx, source, "heading", heading, `${path}.heading`),
        body: required(ctx, source, "body", body, `${path}.body`),
      };
    },
  });
}

const titleOf = (ctx, slide) =>
  required(ctx, slide, "title", LIMITS.deck.title);

function normalizeHero(slide, ctx) {
  const limits = LIMITS.hero;
  return {
    title: required(ctx, slide, "title", limits.title),
    subtitle: optional(ctx, slide, "subtitle", limits.subtitle),
    meta: optional(ctx, slide, "meta", limits.meta),
  };
}

function normalizeStatement(slide, ctx) {
  const limits = LIMITS.statement;
  return {
    statement: required(ctx, slide, "statement", limits.statement),
    note: optional(ctx, slide, "note", limits.note),
  };
}

function normalizePillars(slide, ctx) {
  return {
    title: titleOf(ctx, slide),
    pillars: headed(ctx, slide.pillars, {
      field: "pillars",
      ...LIMITS.pillars,
    }),
  };
}

function normalizeEvidence(slide, ctx) {
  const limits = LIMITS.evidence;
  const [detailCount, detailLimit] = limits.detail;
  return {
    title: titleOf(ctx, slide),
    metrics: list(ctx, slide.metrics, {
      field: "metrics",
      count: limits.count,
      each: (element, path) => {
        if (!isObject(element)) {
          problem(ctx, `${path} must be an object with value and label`);
          return undefined;
        }
        ignore(ctx, element, ["value", "label", "note"], path);
        // A metric's note is a one-line annotation, unlike a statement's.
        return compact({
          value: required(ctx, element, "value", limits.value, `${path}.value`),
          label: required(ctx, element, "label", limits.label, `${path}.label`),
          note: str(ctx, element.note, {
            field: `${path}.note`,
            limit: limits.note,
            breaks: "short",
          }),
        });
      },
    }),
    detail: bullets(ctx, slide.detail, {
      field: "detail",
      count: [0, detailCount],
      limit: detailLimit,
    }),
  };
}

function normalizeTimeline(slide, ctx) {
  return {
    title: titleOf(ctx, slide),
    steps: headed(ctx, slide.steps, { field: "steps", ...LIMITS.timeline }),
  };
}

function side(ctx, value, field) {
  const limits = LIMITS.comparison;
  if (value === undefined || value === null) {
    problem(ctx, `${field} is missing`);
    return undefined;
  }
  if (!isObject(value)) {
    problem(ctx, `${field} must be an object with heading and points`);
    return undefined;
  }
  ignore(ctx, value, ["heading", "points"], field);
  const [pointCount, pointLimit] = limits.points;
  return {
    heading: required(
      ctx,
      value,
      "heading",
      limits.heading,
      `${field}.heading`,
    ),
    points: bullets(ctx, value.points, {
      field: `${field}.points`,
      count: [1, pointCount],
      limit: pointLimit,
    }),
  };
}

function normalizeComparison(slide, ctx) {
  return {
    title: titleOf(ctx, slide),
    left: side(ctx, slide.left, "left"),
    right: side(ctx, slide.right, "right"),
  };
}

function normalizeGrid(slide, ctx) {
  return {
    title: titleOf(ctx, slide),
    items: headed(ctx, slide.items, { field: "items", ...LIMITS.grid }),
  };
}

function normalizeQuote(slide, ctx) {
  const limits = LIMITS.quote;
  return {
    quote: required(ctx, slide, "quote", limits.quote),
    source: optional(ctx, slide, "source", limits.source),
  };
}

function normalizeCta(slide, ctx) {
  const limits = LIMITS.cta;
  return {
    title: required(ctx, slide, "title", limits.title),
    subtitle: optional(ctx, slide, "subtitle", limits.subtitle),
    contact: optional(ctx, slide, "contact", limits.contact),
  };
}

// The first table header, chart category and series name start the
// value officecli reads as a whole, and the host refuses one that starts
// like a mention or a path (create.rs data_reaches_out); a drive only
// reaches a table header, since chart labels hold no colon.
const PATH_START = /^(?:@|[/\\]|\.{1,2}[/\\]|~[/\\]|[A-Za-z]:)/;

// A problem when `text`, the first cell of an officecli value, starts like
// a mention or a path.
function firstCell(ctx, text, field) {
  if (text !== undefined && PATH_START.test(text))
    problem(ctx, `${field} must not start with "@" or a path prefix`);
}

// A chart label ends up inside an officecli argument, so it must not hold
// the characters that argument is split on.
function label(ctx, value, path) {
  const text = str(ctx, value, {
    field: path,
    limit: LIMITS.showcase.chart.label,
    required: true,
    breaks: "short",
  });
  if (text !== undefined && CHART_SEPARATORS.test(text))
    problem(ctx, `${path} is "${text}"; it must not contain , ; or :`);
  return text;
}

// Series values are numbers, or a string of numbers separated by commas.
function numbers(ctx, value, field, categories) {
  if (value === undefined || value === null) {
    problem(ctx, `${field} is missing`);
    return undefined;
  }
  const parts = typeof value === "string" ? value.split(",") : value;
  if (!Array.isArray(parts)) {
    problem(ctx, `${field} must be a list of numbers`);
    return undefined;
  }
  if (categories !== undefined && parts.length !== categories)
    problem(
      ctx,
      `${field} has ${parts.length} values; the chart has ${categories} categories`,
    );
  return parts.map((part, index) => {
    const given = typeof part === "string" ? part.trim() : part;
    const number = typeof given === "string" && given ? Number(given) : given;
    if (typeof number === "number" && Number.isFinite(number)) return number;
    problem(
      ctx,
      `${field}[${index}] is ${JSON.stringify(given)}; it must be a number`,
    );
    return undefined;
  });
}

function normalizeChart(ctx, value) {
  const field = "media.chart";
  const limits = LIMITS.showcase.chart;
  if (!isObject(value)) {
    problem(ctx, `${field} must be an object with kind, categories and series`);
    return undefined;
  }
  ignore(ctx, value, ["kind", "categories", "series"], field);

  const named =
    typeof value.kind === "string"
      ? value.kind.trim().toLowerCase()
      : value.kind;
  const kind = CHART_KINDS.includes(named) ? named : undefined;
  if (kind === undefined)
    problem(
      ctx,
      value.kind === undefined || value.kind === null
        ? `${field}.kind is missing; use one of ${CHART_KINDS.join(", ")}`
        : `${field}.kind is ${JSON.stringify(value.kind)}; use one of ${CHART_KINDS.join(", ")}`,
    );

  const categories = list(ctx, value.categories, {
    field: `${field}.categories`,
    count: kind === "pie" ? limits.pie : [1, limits.categories],
    each: (element, path) => label(ctx, element, path),
  });
  const series = list(ctx, value.series, {
    field: `${field}.series`,
    count: [1, limits.series],
    each: (element, path) => {
      if (!isObject(element)) {
        problem(ctx, `${path} must be an object with name and values`);
        return undefined;
      }
      ignore(ctx, element, ["name", "values"], path);
      return {
        name: label(ctx, element.name, `${path}.name`),
        values: numbers(
          ctx,
          element.values,
          `${path}.values`,
          categories?.length,
        ),
      };
    },
  });
  firstCell(ctx, categories?.[0], `${field}.categories[0]`);
  firstCell(ctx, series?.[0]?.name, `${field}.series[0].name`);
  // A pie draws one series; the others would be dropped, so it is refused.
  if (kind === "pie" && series !== undefined && series.length > 1)
    problem(ctx, `a pie chart takes one series; got ${series.length}`);
  return { kind, categories, series };
}

function normalizeTable(ctx, value) {
  const field = "media.table";
  const limits = LIMITS.showcase.table;
  if (!isObject(value)) {
    problem(ctx, `${field} must be an object with columns and rows`);
    return undefined;
  }
  ignore(ctx, value, ["columns", "rows"], field);

  const columns = list(ctx, value.columns, {
    field: `${field}.columns`,
    count: [1, limits.columns],
    each: (element, path) =>
      str(ctx, element, {
        field: path,
        limit: limits.header,
        required: true,
        breaks: "short",
      }),
  });
  firstCell(ctx, columns?.[0], `${field}.columns[0]`);
  // The limit on rows counts the header, which is `columns` here.
  const rows = list(ctx, value.rows, {
    field: `${field}.rows`,
    count: [1, limits.rows - 1],
    each: (row, path) => {
      if (!Array.isArray(row)) {
        problem(ctx, `${path} must be a list of cells`);
        return undefined;
      }
      if (columns !== undefined && row.length !== columns.length)
        problem(
          ctx,
          `${path} has ${row.length} cells; the table has ${columns.length} columns`,
        );
      return row.map(
        (cell, index) =>
          str(ctx, cell, {
            field: `${path}[${index}]`,
            limit: limits.cell,
            breaks: "short",
          }) ?? "",
      );
    },
  });
  return { columns, rows };
}

// The one picture, chart or table a showcase page shows.
function normalizeMedia(ctx, value) {
  const given = isObject(value)
    ? MEDIA.filter((key) => value[key] !== undefined && value[key] !== null)
    : [];
  if (given.length !== 1) {
    problem(
      ctx,
      `media must hold exactly one of image, chart or table; it holds ${given.length ? given.join(", ") : "none"}`,
    );
    return undefined;
  }
  ignore(ctx, value, MEDIA, "media");

  const [kind] = given;
  if (kind === "chart") return { chart: normalizeChart(ctx, value.chart) };
  if (kind === "table") return { table: normalizeTable(ctx, value.table) };
  const image = String(value.image).trim();
  if (!IMAGE_URI.test(image))
    problem(
      ctx,
      `media.image must be a base64 data URI of a png, jpeg or svg+xml image; it starts with "${image.slice(0, 24)}"`,
    );
  return { image };
}

function normalizeShowcase(slide, ctx) {
  const limits = LIMITS.showcase;
  return {
    title: titleOf(ctx, slide),
    media: normalizeMedia(ctx, slide.media),
    caption: optional(ctx, slide, "caption", limits.caption),
    body: optional(ctx, slide, "body", limits.body),
  };
}

const NORMALIZERS = {
  hero: normalizeHero,
  statement: normalizeStatement,
  pillars: normalizePillars,
  evidence: normalizeEvidence,
  timeline: normalizeTimeline,
  comparison: normalizeComparison,
  grid: normalizeGrid,
  quote: normalizeQuote,
  cta: normalizeCta,
  showcase: normalizeShowcase,
};

function normalizeSlide(slide, index, problems, warnings) {
  const ctx = { scope: `slide ${index + 1}`, problems, warnings };
  if (!isObject(slide)) {
    problem(ctx, "must be an object");
    return undefined;
  }
  const type =
    typeof slide.type === "string"
      ? slide.type.trim().toLowerCase()
      : undefined;
  if (!PAGE_TYPES.includes(type)) {
    problem(
      ctx,
      `${slide.type === undefined ? "type is missing" : `unknown type ${JSON.stringify(slide.type)}`}; use one of ${PAGE_TYPES.join(", ")}`,
    );
    return undefined;
  }
  ctx.type = type;
  ignore(ctx, slide, ["type", "notes", ...FIELDS[type]]);
  return compact({
    type,
    ...NORMALIZERS[type](slide, ctx),
    notes: optional(ctx, slide, "notes", LIMITS.deck.notes),
  });
}

// Reads a deck object. Returns the normalized copy and the warnings, or,
// when anything is wrong, every problem found and no deck. Literal \n
// sequences are line breaks first (see normalizeLiteralNewlines).
export function parseDeck(given, { fallbackLanguage = "zh" } = {}) {
  if (!isObject(given)) return { problems: ["deck must be an object"] };
  const value = normalizeLiteralNewlines(given);
  const problems = [];
  const warnings = [];
  const ctx = { scope: "deck", problems, warnings };
  ignore(ctx, value, ["kind", "title", "language", "theme", "slides"]);

  const title = titleOf(ctx, value);

  const spoken =
    typeof value.language === "string"
      ? value.language.trim().toLowerCase()
      : "";
  const language = LANGUAGES.includes(spoken) ? spoken : fallbackLanguage;

  const named =
    typeof value.theme === "string"
      ? value.theme.trim().toLowerCase()
      : value.theme;
  let theme = DEFAULT_THEME;
  if (THEME_NAMES.includes(named)) theme = named;
  else if (named !== undefined && named !== null && named !== "")
    warn(
      ctx,
      `unknown theme ${JSON.stringify(value.theme)}; using ${DEFAULT_THEME}`,
    );

  let slides;
  if (value.slides === undefined || value.slides === null) {
    problem(ctx, "slides is missing");
  } else if (!Array.isArray(value.slides)) {
    problem(ctx, "slides must be a list of slide objects");
  } else {
    const [min, max] = LIMITS.deck.slides;
    const count = value.slides.length;
    if (count < min || count > max)
      problem(
        ctx,
        `${count} slides; the limit is ${min}–${max}${count > max ? "; split it into two decks" : ""}`,
      );
    slides = value.slides.map((slide, index) =>
      normalizeSlide(slide, index, problems, warnings),
    );
  }

  if (problems.length > 0) return { problems };
  return { deck: { kind: "deck", title, language, theme, slides }, warnings };
}
