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

// Limits are in Chinese-character units (see textLength) except where noted.
// `count: [min, max]` bounds a list; `detail` and `points` read as
// `[at most this many, each at most this long]`. A table's `rows` counts
// the header row too.
export const LIMITS = {
  deck: { slides: [3, 30], notes: 300, title: 22 },
  hero: { title: 18, subtitle: 30, meta: 30 },
  statement: { statement: 40, note: 60 },
  pillars: { count: [2, 4], heading: 12, body: 60 },
  evidence: {
    count: [1, 4],
    value: 6,
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
    table: { columns: 6, rows: 6, header: 8, cell: 12 },
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

// create_document's content is a deck object, or that object alone in an
// array (some models wrap a single object); a batch of commands is neither.
export function deckOf(content) {
  const candidate =
    Array.isArray(content) && content.length === 1 ? content[0] : content;
  return isObject(candidate) && candidate.kind === "deck" ? candidate : null;
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

const characters = (text) => [...text].length;

// A text field: a string or a number, trimmed, within the limit. A blank
// optional field is absent.
function str(
  ctx,
  value,
  { field, limit, required = false, measure = textLength },
) {
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
  const size = measure(text);
  if (size > limit)
    problem(ctx, `${field} is ${size} characters; the limit is ${limit}`);
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

// A list of short lines; an element may be { text } instead of a string.
function bullets(ctx, value, { field, count, limit }) {
  return list(ctx, value, {
    field,
    count,
    each: (element, path) => {
      if (!isObject(element))
        return str(ctx, element, { field: path, limit, required: true });
      ignore(ctx, element, ["text"], path);
      return required(ctx, element, "text", limit, path);
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
        // The value is a few characters wide on the slide whatever the script.
        return compact({
          value: str(ctx, element.value, {
            field: `${path}.value`,
            limit: limits.value,
            required: true,
            measure: characters,
          }),
          label: required(ctx, element, "label", limits.label, `${path}.label`),
          note: optional(ctx, element, "note", limits.note, `${path}.note`),
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

// A chart label ends up inside an officecli argument, so it must not hold
// the characters that argument is split on.
function label(ctx, value, path) {
  const text = str(ctx, value, {
    field: path,
    limit: LIMITS.showcase.chart.label,
    required: true,
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
      str(ctx, element, { field: path, limit: limits.header, required: true }),
  });
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
          str(ctx, cell, { field: `${path}[${index}]`, limit: limits.cell }) ??
          "",
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
// when anything is wrong, every problem found and no deck.
export function parseDeck(value, { fallbackLanguage = "zh" } = {}) {
  if (!isObject(value)) return { problems: ["deck must be an object"] };
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
