// Images a model is shown of a file: a rendered PDF page, a thumbnail of a
// generated document's first page or slide, or a contact sheet of a deck's
// slides. The native canvas addon loads on first use, not with the runtime.
import { drawSlideText, slideText } from "./slide-text.mjs";
import { SaxesParser } from "saxes";
import {
  CJK_FONT_FAMILIES,
  cjkFontFamilies,
  hasCjkText,
} from "./text-fonts.mjs";

const THUMBNAIL_EDGE = 768;
const MAX_THUMBNAIL_BYTES = 200 * 1024;
const THUMBNAIL_ENCODINGS = [85, 60, 40].map((quality) => [
  "image/jpeg",
  quality,
]);

/**
 * The first of `encodings` ([mimeType, quality?] pairs, tried in order) that
 * encodes the canvas in at most maxBytes, as { data: base64, mimeType }, or
 * null when none does.
 */
export function encodeWithin(canvas, encodings, maxBytes) {
  for (const [mimeType, quality] of encodings) {
    const bytes = canvas.toBuffer(mimeType, quality);
    if (bytes.length <= maxBytes)
      return { data: bytes.toString("base64"), mimeType };
  }
  return null;
}

/**
 * A JPEG of a canvas or loaded image, scaled down to maxEdge on its long side
 * over white (JPEG has no transparency), at the best quality step within
 * maxBytes; null when even the lowest step is over.
 */
export async function encodeThumbnail(
  source,
  { maxBytes = MAX_THUMBNAIL_BYTES, maxEdge = THUMBNAIL_EDGE } = {},
) {
  const { createCanvas } = await import("@napi-rs/canvas");
  const scale = Math.min(1, maxEdge / Math.max(source.width, source.height));
  const width = Math.max(1, Math.round(source.width * scale));
  const height = Math.max(1, Math.round(source.height * scale));
  const canvas = createCanvas(width, height);
  const context = canvas.getContext("2d");
  context.fillStyle = "#fff";
  context.fillRect(0, 0, width, height);
  context.drawImage(source, 0, 0, width, height);
  return encodeWithin(canvas, THUMBNAIL_ENCODINGS, maxBytes);
}

/** A thumbnail of PNG bytes; rejects bytes the canvas cannot decode. */
export async function thumbnailFromPng(bytes, options) {
  const { loadImage } = await import("@napi-rs/canvas");
  return encodeThumbnail(await loadImage(bytes), options);
}

// A slide's SVG read and its text lifted out of foreignObject (see
// slide-text.mjs); rejects when the canvas cannot read the SVG or the SVG
// is not XML: a slide drawn without its words would tell the model it is
// blank.
// officecli writes chart labels as native <text> with no font family. The
// canvas's SVG reader then uses one default font and no per-glyph fallback,
// so Chinese or Japanese labels drew as missing-glyph boxes. It takes the
// first installed family of a list, so name CJK-capable system fonts first.
export const SVG_TEXT_FONTS = `${CJK_FONT_FAMILIES}, sans-serif`;
const FONT_DECLARATION = /(^|;)(\s*font-family\s*:\s*)[^;]*/i;
const xmlAttribute = (value) =>
  value
    .replaceAll("&", "&amp;")
    .replaceAll('"', "&quot;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;");

// The XML parser has already validated this opening tag. Skip whole quoted
// values so metadata containing "font-family='...'" cannot become an attribute.
function setAttribute(tag, wanted, value) {
  let index = 1;
  while (index < tag.length && !/[\s/>]/.test(tag[index])) index++;
  while (index < tag.length) {
    while (/\s/.test(tag[index] ?? "")) index++;
    if (index >= tag.length || tag[index] === "/" || tag[index] === ">") break;
    const start = index;
    while (index < tag.length && !/[\s=]/.test(tag[index])) index++;
    const name = tag.slice(start, index);
    while (/\s/.test(tag[index] ?? "")) index++;
    if (tag[index++] !== "=") throw new Error("Invalid SVG attribute");
    while (/\s/.test(tag[index] ?? "")) index++;
    const quote = tag[index++];
    if (quote !== '"' && quote !== "'")
      throw new Error("Invalid SVG attribute");
    const end = tag.indexOf(quote, index) + 1;
    if (end === 0) throw new Error("Invalid SVG attribute");
    if (name === wanted)
      return (
        tag.slice(0, start) +
        `${wanted}="${xmlAttribute(value)}"` +
        tag.slice(end)
      );
    index = end;
  }
  return tag.replace(
    /\/?>$/,
    (ending) => ` ${wanted}="${xmlAttribute(value)}"${ending}`,
  );
}

const SVG_NAMESPACE = "http://www.w3.org/2000/svg";
const CHART_VIEWPORT_ATTRIBUTES = new Set([
  "x",
  "y",
  "width",
  "height",
  "viewBox",
  "preserveAspectRatio",
]);
const CHART_LABEL_ATTRIBUTES = new Set([
  "x",
  "y",
  "fill",
  "font-size",
  "text-anchor",
  "font-style",
  "font-weight",
]);
const CHART_LINE_ATTRIBUTES = new Set([
  "x1",
  "y1",
  "x2",
  "y2",
  "stroke",
  "stroke-width",
]);
const OPAQUE_CHART_COLOR = /^#(?:[\da-f]{3}|[\da-f]{6})$/i;

function chartNumber(attributes, name) {
  const value = attributes[name]?.value ?? "";
  return /^-?\d+(?:\.\d+)?$/.test(value) ? Number(value) : NaN;
}

function chartViewport({ name, uri, attributes }, inChartGroup) {
  if (
    name !== "svg" ||
    uri !== SVG_NAMESPACE ||
    !inChartGroup ||
    !Object.keys(attributes).every((name) =>
      CHART_VIEWPORT_ATTRIBUTES.has(name),
    ) ||
    attributes.x?.value !== "0" ||
    attributes.preserveAspectRatio?.value !== "xMidYMid meet" ||
    !["width", "height"].every((axis) => {
      const size = chartNumber(attributes, axis);
      return size > 0 && size <= 8192;
    })
  )
    return null;
  const viewBox = attributes.viewBox?.value.match(/^0 0 (\d+) (\d+)$/);
  if (!viewBox) return null;
  const [width, height] = viewBox.slice(1).map(Number);
  if (![width, height].every((size) => size > 0 && size <= 8192)) return null;
  return { width, height, lines: [], labels: [] };
}

function axisSpacing(chart) {
  const { width, height, labels, lines } = chart;
  const titles = labels.filter(
    ({ x, y }) => x === width / 2 && y === height - 2,
  );
  if (titles.length !== 1) return null;
  const title = titles[0];
  const horizontalRows = new Set(
    lines
      .filter(([x1, y1, x2, y2]) => y1 === y2 && x1 < x2)
      .map((line) => line[1]),
  );
  // OfficeCLI draws a four-unit tick and places its label 16 units from
  // the horizontal axis. Use both strokes to distinguish actual tick labels
  // from series labels or ordinary text near the bottom of a nested SVG.
  const ticks = new Set(
    lines
      .filter(
        ([x1, y1, x2, y2]) =>
          x1 === x2 && y2 === y1 + 4 && horizontalRows.has(y1),
      )
      .map(([x, y]) => `${x},${y + 16}`),
  );
  let titleY = title.y;
  for (const tick of labels) {
    if (tick.y < title.y && ticks.has(`${tick.x},${tick.y}`)) {
      titleY = Math.max(
        titleY,
        tick.y + Math.ceil(Math.max(title.size, tick.size)) + 2,
      );
    }
  }
  if (titleY === title.y) return null;
  const viewHeight = Math.max(height, titleY + Math.ceil(title.size / 4));
  return viewHeight <= 8192 ? { title, titleY, viewHeight } : null;
}

// The pinned OfficeCLI chart viewport can place the bottom title on the
// same ink row as the ticks. Reserve space inside the chart's existing
// frame before rasterization. The native slide and plot coordinates stay intact.
function withChartAxisSpacing(source) {
  if (!source.includes("xMidYMid meet")) return source;
  const parser = new SaxesParser({ xmlns: true });
  const stack = [];
  const edits = [];
  let parseFailed = false;
  parser.on("error", () => {
    parseFailed = true;
  });
  parser.on("opentag", (tag) => {
    const parent = stack.at(-1);
    if (parent?.label) parent.label = null;
    const end = parser.position;
    const isChartGroup =
      tag.name === "g" &&
      tag.uri === SVG_NAMESPACE &&
      /^translate\([^()]+\)$/.test(tag.attributes.transform?.value ?? "") &&
      Object.keys(tag.attributes).length === 1;
    const node = {
      isChartGroup,
      start: source.lastIndexOf("<", end - 1),
      end,
      chart: chartViewport(tag, parent?.isChartGroup),
    };
    if (parent?.chart && tag.uri === SVG_NAMESPACE) {
      const value = (name) => chartNumber(tag.attributes, name);
      if (
        tag.name === "line" &&
        OPAQUE_CHART_COLOR.test(tag.attributes.stroke?.value ?? "") &&
        value("stroke-width") > 0 &&
        Object.keys(tag.attributes).every((name) =>
          CHART_LINE_ATTRIBUTES.has(name),
        )
      ) {
        const line = ["x1", "y1", "x2", "y2"].map(value);
        if (line.every(Number.isFinite)) parent.chart.lines.push(line);
      }
      if (
        tag.name === "text" &&
        tag.attributes["text-anchor"]?.value === "middle" &&
        OPAQUE_CHART_COLOR.test(tag.attributes.fill?.value ?? "") &&
        Object.keys(tag.attributes).every((name) =>
          CHART_LABEL_ATTRIBUTES.has(name),
        )
      ) {
        const [x, y, size] = ["x", "y", "font-size"].map(value);
        if (
          [x, y, size].every(Number.isFinite) &&
          size > 0 &&
          size <= parent.chart.height
        ) {
          node.label = { x, y, size, start: node.start, end };
        }
      }
    }
    stack.push(node);
  });
  parser.on("text", (value) => {
    const label = stack.at(-1)?.label;
    if (label && value.trim()) label.hasText = true;
  });
  parser.on("closetag", () => {
    const node = stack.pop();
    if (node?.label?.hasText) stack.at(-1)?.chart?.labels.push(node.label);
    if (!node?.chart) return;
    const spacing = axisSpacing(node.chart);
    if (!spacing) return;
    edits.push(
      {
        start: node.start,
        end: node.end,
        name: "viewBox",
        value: `0 0 ${node.chart.width} ${spacing.viewHeight}`,
      },
      {
        start: spacing.title.start,
        end: spacing.title.end,
        name: "y",
        value: String(spacing.titleY),
      },
    );
  });
  parser.write(source).close();
  if (parseFailed || !edits.length) return source;
  const parts = [];
  let cursor = 0;
  for (const { start, end, name, value } of edits.sort(
    (a, b) => a.start - b.start,
  )) {
    parts.push(
      source.slice(cursor, start),
      setAttribute(source.slice(start, end), name, value),
    );
    cursor = end;
  }
  parts.push(source.slice(cursor));
  return parts.join("");
}

export function withTextFonts(source) {
  const records = [],
    stack = [];
  const parser = new SaxesParser();
  parser.on("opentag", ({ name, attributes }) => {
    const local = name.split(":").at(-1),
      style = attributes.style ?? "";
    const declaration = [
      ...style.matchAll(new RegExp(FONT_DECLARATION.source, "gi")),
    ].at(-1);
    const ownFamily = declaration
      ? declaration[0].replace(/^;?\s*font-family\s*:\s*/i, "")
      : attributes["font-family"];
    const parent = stack.at(-1);
    const family =
      ownFamily === undefined || /^(?:inherit|unset)$/i.test(ownFamily.trim())
        ? parent?.family
        : ownFamily;
    const inText = local === "text" || !!parent?.inText;
    let record;
    if (inText) {
      const end = parser.position;
      record = {
        start: source.lastIndexOf("<", end - 1),
        end,
        style,
        declaration: !!declaration,
        family,
        local,
        cjk: false,
      };
      records.push(record);
    }
    stack.push({ record, cjk: false, family, inText });
  });
  const text = (value) => {
    if (stack.length && hasCjkText(value)) stack.at(-1).cjk = true;
  };
  parser.on("text", text);
  parser.on("cdata", text);
  parser.on("closetag", () => {
    const node = stack.pop();
    if (node.record) node.record.cjk = node.cjk;
    // A nested text run handles its own CJK face; it must not replace a Latin
    // parent's face or an independently authored CJK serif face.
    else if (node.cjk && stack.length) stack.at(-1).cjk = true;
  });
  parser.write(source).close();
  // Opening-tag offsets and decoded text come from the XML parser, so comments,
  // numeric entities and quoted '>' cannot select or relabel another element.
  const parts = [];
  let cursor = 0;
  for (const record of records) {
    if (!record.cjk && (record.family !== undefined || record.local !== "text"))
      continue;
    const families = record.cjk
      ? cjkFontFamilies(record.family)
      : SVG_TEXT_FONTS;
    let tag = source.slice(record.start, record.end);
    if (record.declaration) {
      const style = record.style.replace(
        new RegExp(FONT_DECLARATION.source, "gi"),
        (_all, separator, key) => separator + key + families,
      );
      tag = setAttribute(tag, "style", style);
    } else {
      tag = setAttribute(tag, "font-family", families);
    }
    parts.push(source.slice(cursor, record.start), tag);
    cursor = record.end;
  }
  parts.push(source.slice(cursor));
  return parts.join("");
}

async function loadSlide(bytes) {
  const { loadImage } = await import("@napi-rs/canvas");
  const source = Buffer.from(bytes).toString("utf8");
  const text = slideText(source);
  const drawn = withTextFonts(withChartAxisSpacing(source));
  return {
    image: await loadImage(drawn === source ? bytes : Buffer.from(drawn)),
    text,
  };
}

// Draws a loaded slide over white in the box at (x, y), its shapes first and
// its text on top. The image is sized to the box so that the vector is
// drawn at full resolution, not scaled up from its small default size.
function drawSlide(context, { image, text }, x, y, width, height) {
  context.save();
  context.translate(x, y);
  context.fillStyle = "#fff";
  context.fillRect(0, 0, width, height);
  image.width = width;
  image.height = height;
  context.drawImage(image, 0, 0, width, height);
  if (text.boxes.length > 0 && text.width > 0 && text.height > 0) {
    context.scale(width / text.width, height / text.height);
    drawSlideText(context, text.boxes);
  }
  context.restore();
}

/**
 * A thumbnail of SVG bytes, drawn at the full long edge: a vector loses
 * nothing when enlarged, and an SVG sized only by its viewBox loads small.
 * The canvas's SVG reader skips foreignObject, where officecli puts a
 * slide's text, so that text is drawn on top (slide-text.mjs). Rejects when
 * this canvas build cannot read the SVG, or when the SVG is not XML.
 */
export async function thumbnailFromSvg(bytes, options = {}) {
  const { createCanvas } = await import("@napi-rs/canvas");
  const slide = await loadSlide(bytes);
  const maxEdge = options.maxEdge ?? THUMBNAIL_EDGE;
  const scale = maxEdge / Math.max(slide.image.width, slide.image.height);
  if (!Number.isFinite(scale) || scale <= 0)
    throw new Error("The SVG has no size");
  const width = Math.max(1, Math.round(slide.image.width * scale));
  const height = Math.max(1, Math.round(slide.image.height * scale));
  const canvas = createCanvas(width, height);
  drawSlide(canvas.getContext("2d"), slide, 0, 0, width, height);
  return encodeThumbnail(canvas, options);
}

const SHEET_GAP = 24;
const SHEET_LABEL = 32;
const MAX_SHEET_BYTES = 600 * 1024;
const SHEET_ENCODINGS = [85, 65, 45].map((quality) => ["image/jpeg", quality]);

// The cell of a slide that could not be drawn: a grey box that says so.
function drawPlaceholder(context, x, y, width, height) {
  context.save();
  context.fillStyle = "#94a3b8";
  context.fillRect(x, y, width, height);
  context.fillStyle = "#f8fafc";
  context.font = `bold ${SHEET_LABEL - 8}px sans-serif`;
  context.textAlign = "center";
  context.textBaseline = "middle";
  context.fillText("no picture", x + width / 2, y + height / 2);
  context.restore();
}

/**
 * One JPEG of a deck's slides, for a model to review it at a glance:
 * `slides` is [{ label, svg }], each slide's number and its SVG bytes (null
 * when they could not be read), drawn with their text (as thumbnailFromSvg
 * does) in `columns` columns over a light background, each under its label.
 * A slide that cannot be drawn keeps its cell, as a placeholder. A cell is
 * (width - gaps) / columns wide at the proportions of the first slide drawn
 * (a deck has one slide size). Null when even the lowest quality is over
 * 600 KB; rejects when there are no slides, or none that this canvas can
 * draw.
 */
export async function contactSheet(slides, { columns = 2, width = 1600 } = {}) {
  if (slides.length === 0)
    throw new Error("A contact sheet needs at least one slide");
  const { createCanvas } = await import("@napi-rs/canvas");
  const loaded = await Promise.all(
    slides.map(({ svg }) => (svg ? loadSlide(svg).catch(() => null) : null)),
  );
  const first = loaded.find(Boolean);
  if (!first) throw new Error("No slide could be drawn");
  const aspect = first.image.height / first.image.width;
  if (!Number.isFinite(aspect) || aspect <= 0)
    throw new Error("The SVG has no size");
  const cellWidth = Math.floor((width - (columns + 1) * SHEET_GAP) / columns);
  const cellHeight = Math.round(cellWidth * aspect);
  const rowHeight = SHEET_LABEL + cellHeight + SHEET_GAP;
  const rows = Math.ceil(slides.length / columns);
  const canvas = createCanvas(width, SHEET_GAP + rows * rowHeight);
  const context = canvas.getContext("2d");
  context.fillStyle = "#e2e8f0";
  context.fillRect(0, 0, canvas.width, canvas.height);
  context.fillStyle = "#334155";
  context.font = `bold ${SHEET_LABEL - 8}px sans-serif`;
  context.textBaseline = "alphabetic";
  slides.forEach(({ label }, index) => {
    const x = SHEET_GAP + (index % columns) * (cellWidth + SHEET_GAP);
    const y = SHEET_GAP + Math.floor(index / columns) * rowHeight;
    context.fillText(String(label), x, y + SHEET_LABEL - 8);
    const slide = loaded[index];
    if (slide)
      drawSlide(context, slide, x, y + SHEET_LABEL, cellWidth, cellHeight);
    else drawPlaceholder(context, x, y + SHEET_LABEL, cellWidth, cellHeight);
  });
  return encodeWithin(canvas, SHEET_ENCODINGS, MAX_SHEET_BYTES);
}
