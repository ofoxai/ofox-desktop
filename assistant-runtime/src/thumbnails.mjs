// Images a model is shown of a file: a rendered PDF page, a thumbnail of a
// generated document's first page or slide, or a contact sheet of a deck's
// slides. The native canvas addon loads on first use, not with the runtime.
import { drawSlideText, slideText } from "./slide-text.mjs";

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
export const SVG_TEXT_FONTS =
  "'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', 'Yu Gothic', 'Noto Sans CJK SC', sans-serif";
// One pass, bounded by the next angle bracket. A tag whose attributes hold an
// unbalanced quote (a ">" inside a value) is left as it is.
const SVG_TEXT_TAG = /<text(\s[^<>]*)?>/g;
const withTextFonts = (source) =>
  source.replace(SVG_TEXT_TAG, (tag, attributes = "") =>
    /\sfont-family\s*=/.test(attributes) ||
    (attributes.match(/"/g) || []).length % 2 ||
    (attributes.match(/'/g) || []).length % 2
      ? tag
      : `<text font-family="${SVG_TEXT_FONTS}"${attributes}>`,
  );

async function loadSlide(bytes) {
  const { loadImage } = await import("@napi-rs/canvas");
  const source = Buffer.from(bytes).toString("utf8");
  const text = slideText(source);
  const drawn = withTextFonts(source);
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
