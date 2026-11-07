import test from "node:test";
import assert from "node:assert/strict";
import {
  FIT,
  fitSize,
  heightCm,
  lineCapacity,
  lineCount,
} from "../src/deck/text.mjs";

const zh = (count) => "字".repeat(count);

test("a full-width line at 20pt holds about 40 CJK characters", () => {
  const capacity = lineCapacity(29.21, 20);
  assert.ok(capacity >= 40.5 && capacity <= 41, `capacity was ${capacity}`);
});

test("six one-line bullets at 20pt need about 6.9 cm", () => {
  const height = heightCm(6, 20, { paragraphs: 6, spaceAfterPt: 6 });
  assert.ok(height >= 6.8 && height <= 7.0, `height was ${height}`);
});

test("mixed text counts Latin at 0.55 of a CJK character", () => {
  // Width 12 at 36pt holds (12 − 0.508) / (36 × 0.0353) = 9.04 units;
  // "Transformer 入门" is 11 × 0.55 + 0.55 (the space) + 2 = 8.6: one line.
  assert.equal(lineCount("Transformer 入门", 12, 36), 1);
  assert.equal(lineCount(zh(41), 29.21, 20), 2);
});

test("Latin text is measured at the checker's 0.55, not the limits' half", () => {
  // A full-width line at 20pt holds (29.21 − 0.508) / (20 × 0.0353) = 40.65
  // units. 73 Latin characters are 73 × 0.55 = 40.15: one line. 74 are
  // 74 × 0.55 = 40.7: two lines, where counting half (37) would say one.
  assert.equal(lineCount("x".repeat(73), 29.21, 20), 1);
  assert.equal(lineCount("x".repeat(74), 29.21, 20), 2);
});

test("an empty paragraph still counts one line", () => {
  assert.equal(lineCount("第一段\n\n第三段", 29.21, 20), 3);
});

// Hand-derived for width 14.22 cm (box 13.712 cm wide after insets):
// 24pt: 16.18 chars/line, 1.1184 cm/line; 22pt: 17.65, 1.0252; 20pt: 19.42,
// 0.9320; 18pt: 21.58, 0.8388. Each height adds the 0.254 cm insets.
test("fitSize returns the first size when short text fits it", () => {
  // 10 chars: 1 line at 24pt; 1 × 24 × 0.0466 + 0.254 = 1.3724 cm.
  const result = fitSize(zh(10), 14.22, 11.7, [24, 22, 20, 18]);
  assert.equal(result.size, 24);
  assert.equal(result.lines, 1);
  assert.equal(result.fits, true);
  assert.ok(
    Math.abs(result.height - 1.3724) < 1e-9,
    `height was ${result.height}`,
  );
});

test("fitSize skips sizes too tall and returns the first that fits", () => {
  // 240 chars, max 14.7 cm. 24pt: 15 lines = 17.03 cm (no). 22pt: 14 lines
  // = 14 × 22 × 0.0466 + 0.254 = 14.6068 cm (fits).
  const result = fitSize(zh(240), 14.22, 14.7, [24, 22, 20, 18]);
  assert.equal(result.size, 22);
  assert.equal(result.lines, 14);
  assert.equal(result.fits, true);
  assert.ok(
    Math.abs(result.height - 14.6068) < 1e-9,
    `height was ${result.height}`,
  );
});

test("fitSize returns a middle size when the larger ones are too tall", () => {
  // 240 chars, max 13 cm. 24pt: 17.03 cm, 22pt: 14.6068 cm (both no).
  // 20pt: 13 lines = 13 × 20 × 0.0466 + 0.254 = 12.37 cm (fits).
  const result = fitSize(zh(240), 14.22, 13, [24, 22, 20, 18]);
  assert.equal(result.size, 20);
  assert.equal(result.lines, 13);
  assert.equal(result.fits, true);
  assert.ok(
    Math.abs(result.height - 12.37) < 1e-9,
    `height was ${result.height}`,
  );
});

test("fitSize reports when nothing fits, at the smallest size", () => {
  // 240 chars at 18pt: 12 lines = 12 × 18 × 0.0466 + 0.254 = 10.3196 cm > 3.
  const result = fitSize(zh(240), 14.22, 3, [24, 22, 20, 18]);
  assert.equal(result.size, 18);
  assert.equal(result.lines, 12);
  assert.equal(result.fits, false);
  assert.ok(
    Math.abs(result.height - 10.3196) < 1e-9,
    `height was ${result.height}`,
  );
});

test("a box with no room for text never fits", () => {
  // Width 0 and width exactly 2 × 0.254 cm leave zero capacity.
  assert.equal(lineCount(zh(500), 0, 20), Infinity);
  assert.equal(lineCount("", 0.508, 20), Infinity);
  const result = fitSize(zh(500), -3, 5, [20, 18]);
  assert.equal(result.size, 18);
  assert.equal(result.lines, Infinity);
  assert.equal(result.fits, false);
});

test("FIT holds the officecli checker constants", () => {
  assert.deepEqual(FIT, {
    cjkWidthCm: 0.0353,
    latinRatio: 0.55,
    lineHeightCm: 0.0466,
    insetXCm: 0.254,
    insetYCm: 0.127,
  });
});
