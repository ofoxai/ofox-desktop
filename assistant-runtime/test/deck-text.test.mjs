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

test("six one-line bullets at 20pt need about 6.5 cm", () => {
  const height = heightCm(6, 20, { paragraphs: 6, spaceAfterPt: 6 });
  assert.ok(height >= 6.8 && height <= 7.0, `height was ${height}`);
});

test("mixed text counts Latin as half", () => {
  assert.equal(lineCount("Transformer 入门", 12, 36), 1);
  assert.equal(lineCount(zh(41), 29.21, 20), 2);
});

test("an empty paragraph still counts one line", () => {
  assert.equal(lineCount("第一段\n\n第三段", 29.21, 20), 3);
});

test("fitSize steps down to the first size that fits", () => {
  const result = fitSize(zh(240), 14.22, 11.7, [24, 22, 20, 18]);
  assert.equal(result.size, 18);
  assert.equal(result.lines, 12);
  assert.equal(result.fits, true);
  assert.ok(Math.abs(result.height - 10.32) < 0.01, `height was ${result.height}`);
});

test("fitSize reports when nothing fits, at the smallest size", () => {
  const result = fitSize(zh(240), 14.22, 3, [24, 22, 20, 18]);
  assert.equal(result.size, 18);
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
