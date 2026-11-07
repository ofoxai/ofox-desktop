// Estimates whether a text fits its box, using the same model as officecli's
// `view issues` checker (officecli 1.0.160), so a deck that passes here does
// not overflow when the host checks it. Sizes are in cm; font sizes in pt.
import { textLength } from "./schema.mjs";

// 1 pt = 0.0353 cm. A CJK glyph is one font size wide, a Latin character
// 0.55 of one, a line is 1.32 font sizes tall. Insets are 0.1 in left and
// right, 0.05 in top and bottom (0.254 cm and 0.127 cm).
export const FIT = {
  cjkWidthCm: 0.0353,
  latinRatio: 0.55,
  lineHeightCm: 0.0466,
  insetXCm: 0.254,
  insetYCm: 0.127,
};

const PT_CM = 0.0353;

// How many CJK characters fit on one line of a box `widthCm` wide.
export function lineCapacity(widthCm, sizePt) {
  return (widthCm - 2 * FIT.insetXCm) / (sizePt * FIT.cjkWidthCm);
}

// Lines a text takes: each paragraph (split on \n) wraps on its own, and an
// empty paragraph still takes one line.
export function lineCount(text, widthCm, sizePt) {
  const capacity = lineCapacity(widthCm, sizePt);
  return text
    .split(/\r?\n/)
    .reduce(
      (total, paragraph) =>
        total + Math.max(1, Math.ceil(textLength(paragraph) / capacity)),
      0,
    );
}

// Height of `lines` lines at `sizePt`, with insets, and the space between
// paragraphs (spaceAfterPt is in pt).
export function heightCm(
  lines,
  sizePt,
  { paragraphs = 1, spaceAfterPt = 0 } = {},
) {
  return (
    lines * sizePt * FIT.lineHeightCm +
    2 * FIT.insetYCm +
    (paragraphs - 1) * spaceAfterPt * PT_CM
  );
}

// The largest size in `sizes` (descending) whose text fits `maxHeightCm`.
// When none fits, the smallest size is returned with `fits: false`.
export function fitSize(text, widthCm, maxHeightCm, sizes, options = {}) {
  const {
    paragraphs = text.split(/\r?\n/).length,
    spaceAfterPt,
  } = options;
  let result;
  for (const size of sizes) {
    const lines = lineCount(text, widthCm, size);
    const height = heightCm(lines, size, { paragraphs, spaceAfterPt });
    result = { size, lines, height, fits: height <= maxHeightCm };
    if (result.fits) return result;
  }
  return result;
}
