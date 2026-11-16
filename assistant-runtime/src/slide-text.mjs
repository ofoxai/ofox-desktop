// officecli draws a slide's text as XHTML inside <foreignObject>, which the
// canvas's SVG reader skips, so a thumbnail of the SVG alone shows the
// shapes with no words on them. slideText lifts the text boxes out of that
// SVG; layoutBox wraps a box's paragraphs to its width the way the slide's
// CSS does (an empty paragraph keeps one line, as PowerPoint draws it), and
// drawSlideText draws them with the canvas's own text, which falls back to
// a system font for CJK. Text is drawn over every shape, not in the SVG's
// paint order: a thumbnail is for reading the slide.
import { SaxesParser } from "saxes";

// officecli draws 1 pt as 2 px (56.69 px to the cm) whatever the slide's
// size; lengths it gives in px are already in the SVG's units.
const PX_PER_PT = 2;
const DEFAULT_SIZE_PX = 18 * PX_PER_PT;
const DEFAULT_COLOR = "#000000";
const VALIGN = { "flex-start": "top", center: "middle", "flex-end": "bottom" };
const TRANSLATE = /^\s*translate\(\s*(-?[\d.]+)(?:[\s,]+(-?[\d.]+))?\s*\)\s*$/;
// A line breaks between tokens: a CJK ideograph, kana, Hangul syllable,
// CJK punctuation mark or full-width form on its own, or a run of other
// non-space text. Closing punctuation stays with the token before it and
// opening punctuation with the one after, Latin words and numbers included,
// so neither starts or ends a line on its own.
const CJK_RANGE =
  "\\u2e80-\\u9fff\\uac00-\\ud7af\\uf900-\\ufaff\\ufe30-\\ufe4f\\uff00-\\uffef";
const CLOSING = "、。，．：；？！）」』】〉》〕”’…%,.:;?!)\\]";
const OPENING = "（「『【〈《〔“‘";
const TOKENS = new RegExp(
  `\\n|[^\\S\\n]+|[${OPENING}]*(?:[${CJK_RANGE}]|[^\\s${CJK_RANGE}]+)[${CLOSING}]*`,
  "gu",
);

/** A CSS declaration list as a Map of lower-case names to values. */
function declarations(style = "") {
  const map = new Map();
  for (const part of style.split(";")) {
    const colon = part.indexOf(":");
    if (colon > 0)
      map.set(
        part.slice(0, colon).trim().toLowerCase(),
        part.slice(colon + 1).trim(),
      );
  }
  return map;
}

/** A CSS length in SVG units: pt at PX_PER_PT, px as is, em of `em`. */
function length(value, em) {
  const match = /^(-?[\d.]+)(pt|px|em)?$/.exec(value?.trim() ?? "");
  if (!match) return undefined;
  const number = Number(match[1]);
  if (!Number.isFinite(number)) return undefined;
  if (match[2] === "pt") return number * PX_PER_PT;
  if (match[2] === "em") return number * em;
  return number;
}

const number = (value) => {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : undefined;
};

// The text style an element hands down: its own declarations over its
// parent's.
function inherit(parent, css) {
  const size = length(css.get("font-size"), parent.size) ?? parent.size;
  const weight = css.get("font-weight");
  const lineHeight = css.get("line-height");
  return {
    size,
    family: css.get("font-family") ?? parent.family,
    bold:
      weight === undefined
        ? parent.bold
        : weight === "bold" || weight === "bolder" || Number(weight) >= 600,
    italic: css.has("font-style")
      ? css.get("font-style") === "italic"
      : parent.italic,
    color: css.get("color") ?? parent.color,
    align: css.get("text-align") ?? parent.align,
    lineHeight:
      lineHeight === undefined
        ? parent.lineHeight
        : /^[\d.]+$/.test(lineHeight)
          ? Number(lineHeight)
          : (length(lineHeight, size) ?? size) / size,
  };
}

const ROOT_STYLE = {
  size: DEFAULT_SIZE_PX,
  family: undefined,
  bold: false,
  italic: false,
  color: DEFAULT_COLOR,
  align: "left",
  lineHeight: 1.2,
};

/**
 * The text boxes in an officecli slide SVG, in SVG units, with the SVG's
 * own size (its viewBox, else its width and height):
 * { width, height, boxes: [{ x, y, width, height, valign, paragraphs }] },
 * each paragraph { align, lineHeight, size, marginTop, marginBottom,
 * marginLeft, textIndent, runs: [{ text, size, family, bold, italic,
 * color }] }. A box under a transform other than translate, or inside a
 * nested <svg> (a chart), is left out. Throws on SVG that is not XML.
 */
export function slideText(svg) {
  const parser = new SaxesParser();
  const boxes = [];
  const stack = [];
  let size = { width: undefined, height: undefined };
  let box;
  let paragraph;

  const top = () => stack.at(-1);

  parser.on("opentag", (node) => {
    const parent = top();
    const name = node.local ?? node.name;
    const attrs = node.attributes;
    const frame = {
      name,
      dx: parent?.dx ?? 0,
      dy: parent?.dy ?? 0,
      skip: parent?.skip ?? false,
      style: parent?.style ?? ROOT_STYLE,
      depth: parent?.depth,
    };
    if (name === "svg") {
      if (parent === undefined) {
        const viewBox = String(attrs.viewBox ?? "")
          .trim()
          .split(/[\s,]+/)
          .map(Number);
        size =
          viewBox.length === 4 && viewBox.every(Number.isFinite)
            ? { width: viewBox[2], height: viewBox[3] }
            : { width: number(attrs.width), height: number(attrs.height) };
      } else frame.skip = true;
    }
    if (attrs.transform !== undefined && frame.depth === undefined) {
      const move = TRANSLATE.exec(attrs.transform);
      if (move) {
        frame.dx += Number(move[1]);
        frame.dy += Number(move[2] ?? 0);
      } else frame.skip = true;
    }
    if (name === "foreignObject" && !frame.skip) {
      box = {
        x: frame.dx + (number(attrs.x) ?? 0),
        y: frame.dy + (number(attrs.y) ?? 0),
        width: number(attrs.width) ?? 0,
        height: number(attrs.height) ?? 0,
        valign: "top",
        paragraphs: [],
      };
      boxes.push(box);
      frame.depth = 0;
      frame.style = ROOT_STYLE;
    } else if (frame.depth !== undefined && box !== undefined && !frame.skip) {
      // Inside a text box: the first element holds the paragraphs.
      frame.depth += 1;
      const css = declarations(attrs.style);
      frame.style = inherit(frame.style, css);
      if (frame.depth === 1)
        box.valign = VALIGN[css.get("justify-content")] ?? box.valign;
      if (frame.depth === 2 || (name === "br" && paragraph === undefined))
        paragraph = startParagraph(box, frame.style, css);
      if (name === "br")
        paragraph.runs.push({ ...run(frame.style), text: "\n" });
    }
    stack.push(frame);
  });

  parser.on("text", (text) => {
    const frame = top();
    if (frame?.depth === undefined || frame.depth < 1 || box === undefined)
      return;
    if (frame.skip || text.length === 0) return;
    // Whitespace between paragraph divs is no text; flexbox drops it.
    if (frame.depth === 1 && text.trim() === "") return;
    if (paragraph === undefined)
      paragraph = startParagraph(box, frame.style, new Map());
    paragraph.runs.push({ ...run(frame.style), text });
  });

  parser.on("closetag", () => {
    const frame = stack.pop();
    if (frame.depth === 2 || frame.name === "foreignObject")
      paragraph = undefined;
    if (frame.name === "foreignObject") box = undefined;
  });

  parser.write(svg).close();
  return { ...size, boxes };
}

function startParagraph(box, style, css) {
  const paragraph = {
    align: style.align,
    lineHeight: style.lineHeight,
    size: style.size,
    marginTop: length(css.get("margin-top"), style.size) ?? 0,
    marginBottom: length(css.get("margin-bottom"), style.size) ?? 0,
    marginLeft: length(css.get("margin-left"), style.size) ?? 0,
    textIndent: length(css.get("text-indent"), style.size) ?? 0,
    runs: [],
  };
  box.paragraphs.push(paragraph);
  return paragraph;
}

const run = ({ size, family, bold, italic, color }) => ({
  size,
  family,
  bold,
  italic,
  color,
});

/** A run's text as tokens a line breaks between. */
const tokens = (text) => text.match(TOKENS) ?? [];

/**
 * A box's lines, wrapped to its width with `measure(text, run)` →
 * { width, ascent, descent }, placed in it by its paragraphs' alignment and
 * margins and the box's vertical alignment: [{ y (baseline), items:
 * [{ x, text, run }] }]. Lines that end outside the box are kept; the
 * caller clips to the box.
 */
export function layoutBox(box, measure) {
  const laid = [];
  let height = 0;
  for (const paragraph of box.paragraphs) {
    height += paragraph.marginTop;
    for (const line of wrap(paragraph, box.width, measure)) {
      laid.push({ ...line, top: height });
      height += line.height;
    }
    height += paragraph.marginBottom;
  }
  const offset =
    box.valign === "middle"
      ? (box.height - height) / 2
      : box.valign === "bottom"
        ? box.height - height
        : 0;
  return laid.map(({ items, top, height: lineHeight, ascent, descent }) => {
    // The glyphs' em box is centered in the line box (CSS half-leading).
    const y =
      box.y + offset + top + (lineHeight - ascent - descent) / 2 + ascent;
    return { y, items: items.map((item) => ({ ...item, x: box.x + item.x })) };
  });
}

// One paragraph's lines: greedy, breaking between tokens, and inside a
// token wider than a whole line (CSS word-wrap: break-word). A line break
// that ends the paragraph adds no line, as in CSS: officecli writes a soft
// break after all of its paragraph's runs.
function wrap(paragraph, width, measure) {
  const lines = [];
  let line;
  const open = () => {
    const first = lines.length === 0;
    const start = paragraph.marginLeft + (first ? paragraph.textIndent : 0);
    line = { start, room: width - start, used: 0, pieces: [] };
    lines.push(line);
  };
  const place = (text, style) => {
    const metrics = measure(text, style);
    line.pieces.push({ text, run: style, x: line.used, metrics });
    line.used += metrics.width;
  };
  const all = paragraph.runs.flatMap((style) =>
    tokens(style.text).map((token) => ({ token, style })),
  );
  if (all.at(-1)?.token === "\n") all.pop();
  open();
  for (const { token, style } of all) {
    if (token === "\n") {
      open();
      continue;
    }
    const blank = /^\s+$/.test(token);
    const { width: tokenWidth } = measure(token, style);
    if (line.used + tokenWidth <= line.room || line.pieces.length === 0) {
      if (tokenWidth <= line.room || blank) place(token, style);
      else for (const char of token) fit(char, style);
      continue;
    }
    if (blank) continue;
    open();
    if (tokenWidth <= line.room) place(token, style);
    else for (const char of token) fit(char, style);
  }
  return lines.map((each) => finish(each, paragraph, measure));

  function fit(char, style) {
    const { width: charWidth } = measure(char, style);
    if (line.used + charWidth > line.room && line.pieces.length > 0) open();
    place(char, style);
  }
}

// A wrapped line's height, metrics and items, aligned in its room without
// its trailing spaces.
function finish(line, paragraph, measure) {
  const pieces = [...line.pieces];
  while (pieces.length > 0 && /^\s+$/.test(pieces.at(-1).text)) pieces.pop();
  const used = pieces.reduce((sum, piece) => sum + piece.metrics.width, 0);
  const styles = pieces.length > 0 ? pieces.map((piece) => piece.run) : [];
  const largest = Math.max(
    paragraph.size,
    ...styles.map((style) => style.size),
  );
  const metrics =
    pieces.length > 0
      ? pieces.reduce((best, piece) =>
          piece.run.size > best.run.size ? piece : best,
        ).metrics
      : measure("M", { ...paragraph, size: largest });
  const shift =
    paragraph.align === "center"
      ? (line.room - used) / 2
      : paragraph.align === "right"
        ? line.room - used
        : 0;
  return {
    height: largest * paragraph.lineHeight,
    ascent: metrics.ascent,
    descent: metrics.descent,
    items: pieces.map((piece) => ({
      x: line.start + shift + piece.x,
      text: piece.text,
      run: piece.run,
    })),
  };
}

/** The canvas font for a run. */
export function fontOf({ size, family, bold, italic }) {
  const families =
    family === undefined ? "sans-serif" : `${family}, sans-serif`;
  return `${italic ? "italic " : ""}${bold ? "bold " : ""}${size}px ${families}`;
}

/** Draws `boxes` (slideText's) on a 2D context set to SVG units. */
export function drawSlideText(context, boxes) {
  const measure = (text, style) => {
    context.font = fontOf(style);
    const metrics = context.measureText(text);
    return {
      width: metrics.width,
      ascent: metrics.fontBoundingBoxAscent ?? style.size * 0.8,
      descent: metrics.fontBoundingBoxDescent ?? style.size * 0.2,
    };
  };
  context.textBaseline = "alphabetic";
  context.textAlign = "left";
  for (const box of boxes) {
    if (box.paragraphs.length === 0) continue;
    context.save();
    context.beginPath();
    context.rect(box.x, box.y, box.width, box.height);
    context.clip();
    for (const line of layoutBox(box, measure))
      for (const item of line.items) {
        context.font = fontOf(item.run);
        context.fillStyle = item.run.color;
        context.fillText(item.text, item.x, line.y);
      }
    context.restore();
  }
}
