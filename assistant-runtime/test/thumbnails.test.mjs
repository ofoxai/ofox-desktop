import test from "node:test";
import assert from "node:assert/strict";
import { createCanvas, ImageData, loadImage } from "@napi-rs/canvas";
import {
  contactSheet,
  encodeThumbnail,
  thumbnailFromPng,
  thumbnailFromSvg,
} from "../src/thumbnails.mjs";

/** A page-like canvas: a gradient with text and shapes over it. */
function drawing(width, height) {
  const canvas = createCanvas(width, height);
  const context = canvas.getContext("2d");
  const gradient = context.createLinearGradient(0, 0, width, height);
  gradient.addColorStop(0, "#d33");
  gradient.addColorStop(1, "#33d");
  context.fillStyle = gradient;
  context.fillRect(0, 0, width, height);
  context.fillStyle = "#fff";
  context.font = "120px sans-serif";
  for (let y = 150; y < height; y += 200)
    context.fillText("Quarterly results", 40, y);
  return canvas;
}

/** Random pixels: nothing JPEG can leave out. */
function noise(width, height) {
  const canvas = createCanvas(width, height);
  const pixels = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < pixels.length; i++)
    pixels[i] = i % 4 === 3 ? 255 : Math.floor(Math.random() * 256);
  canvas
    .getContext("2d")
    .putImageData(new ImageData(pixels, width, height), 0, 0);
  return canvas;
}

const decoded = (thumbnail) => loadImage(Buffer.from(thumbnail.data, "base64"));

test("a 2000 px canvas becomes a JPEG within 200 KB at a 768 px long edge", async () => {
  const thumbnail = await encodeThumbnail(drawing(2000, 1000));
  assert.equal(thumbnail.mimeType, "image/jpeg");
  assert.ok(Buffer.from(thumbnail.data, "base64").length <= 200 * 1024);
  const image = await decoded(thumbnail);
  assert.equal(image.width, 768);
  assert.equal(image.height, 384);
});

test("a 2000x2000 canvas stays within the default budget", async () => {
  const thumbnail = await encodeThumbnail(drawing(2000, 2000));
  assert.ok(Buffer.from(thumbnail.data, "base64").length <= 200 * 1024);
});

test("a picture no quality step brings within budget gives no thumbnail", async () => {
  assert.equal(
    await encodeThumbnail(noise(768, 768), { maxBytes: 100 * 1024 }),
    null,
  );
});

test("a small canvas keeps its size", async () => {
  const image = await decoded(await encodeThumbnail(drawing(64, 48)));
  assert.equal(image.width, 64);
  assert.equal(image.height, 48);
});

test("transparent areas become white, not black", async () => {
  const canvas = createCanvas(32, 32);
  const image = await decoded(await encodeThumbnail(canvas));
  const check = createCanvas(32, 32);
  const context = check.getContext("2d");
  context.drawImage(image, 0, 0);
  const [r, g, b] = context.getImageData(16, 16, 1, 1).data;
  assert.ok(r > 240 && g > 240 && b > 240, `${r},${g},${b}`);
});

test("a PNG preview becomes a JPEG thumbnail", async () => {
  const png = drawing(1600, 900).toBuffer("image/png");
  const thumbnail = await thumbnailFromPng(png);
  assert.equal(thumbnail.mimeType, "image/jpeg");
  const image = await decoded(thumbnail);
  assert.equal(image.width, 768);
  assert.equal(image.height, 432);
});

// Whether this build of the canvas reads SVG at all, so only that skips a
// test and a failure in the thumbnail code fails it.
async function readsSvg() {
  try {
    await loadImage(
      Buffer.from(
        '<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"/>',
      ),
    );
    return true;
  } catch {
    return false;
  }
}

test("an SVG preview is drawn at the full long edge when the canvas reads SVG", async (t) => {
  // A slide's size only in its viewBox: the canvas gives it a small default.
  const svg = Buffer.from(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 540"><rect width="960" height="540" fill="#fff"/><text x="40" y="100" font-size="60">Title</text></svg>',
  );
  if (!(await readsSvg())) {
    // This build of the canvas has no SVG support: the caller skips the
    // preview, which document-tools tests cover.
    t.skip("SVG unsupported");
    return;
  }
  const image = await decoded(await thumbnailFromSvg(svg));
  assert.equal(image.width, 768);
  // The default size it loads at (267x150) is rounded.
  assert.ok(Math.abs(image.height - 432) <= 1, `height ${image.height}`);
});

// Light pixels in a region of a decoded thumbnail.
async function lightPixels(thumbnail, [x, y, w, h]) {
  const image = await decoded(thumbnail);
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  const { data } = context.getImageData(x, y, w, h);
  let count = 0;
  for (let i = 0; i < data.length; i += 4)
    if (data[i] > 200 && data[i + 1] > 200 && data[i + 2] > 200) count += 1;
  return count;
}

// White 60pt text in a box across the middle of a navy 16:9 slide, as
// officecli draws it: the text in foreignObject, which the canvas skips.
const navySlide = (text) =>
  Buffer.from(
    '<svg xmlns="http://www.w3.org/2000/svg" width="1920" height="1080" viewBox="0 0 1920 1080">' +
      '<rect width="1920" height="1080" fill="#1E3A5F"/>' +
      '<g transform="translate(132,340)"><rect width="1656" height="255" fill="none"/>' +
      '<foreignObject x="14.4" y="7.2" width="1627" height="240">' +
      '<div xmlns="http://www.w3.org/1999/xhtml" style="width:100%;height:100%;overflow:hidden;display:flex;flex-direction:column;justify-content:center;line-height:1">' +
      '<div style="white-space:pre-wrap;margin:0;font-size:60pt;text-align:center;line-height:1.41">' +
      `<span style="font-family:'Arial';font-weight:bold;color:#FFFFFF">${text}</span>` +
      "</div></div></foreignObject></g></svg>",
  );

test("an officecli slide's text, kept in foreignObject, is drawn on the thumbnail", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  // The box spans x 58–720 and y 139–242 of the 768 × 432 thumbnail.
  const region = [58, 139, 662, 103];
  const lit = async (text) =>
    lightPixels(await thumbnailFromSvg(navySlide(text)), region);
  assert.ok((await lit("Transformer")) > 2000, "Latin text is drawn");
  // Arial has no CJK glyphs: the canvas falls back to a system font.
  assert.ok((await lit("入门介绍")) > 2000, "CJK text is drawn");
  assert.equal(await lit(""), 0, "no text, no light pixels");
});

// The pixel at (x, y) of a decoded thumbnail.
async function pixelAt(thumbnail, x, y) {
  const image = await decoded(thumbnail);
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  return [...context.getImageData(x, y, 1, 1).data.slice(0, 3)];
}

// A contact sheet of 1600 px lays 16:9 slides out in two columns: a 24 px
// gap around every cell and a 32 px strip above it for its number, so a
// cell is 764 x 430 and sits at (24 + 788 * column, 56 + 486 * row).
const cellAt = (column, row) => [24 + 788 * column, 56 + 486 * row, 764, 430];
// The strip above a cell, where its number is written.
const labelAt = (column, row) => [24 + 788 * column, 24 + 486 * row, 764, 32];

// Slides numbered from 1, as a contact sheet takes them.
const numbered = (...svgs) =>
  svgs.map((svg, index) => ({ label: String(index + 1), svg }));

// The RGBA bytes of a region of a decoded sheet.
async function region(sheet, [x, y, width, height]) {
  const image = await decoded(sheet);
  const canvas = createCanvas(image.width, image.height);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  return Buffer.from(context.getImageData(x, y, width, height).data);
}

test("a contact sheet draws every slide, with its text, in two columns", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  const sheet = await contactSheet(
    numbered(navySlide("Transformer"), navySlide("入门介绍"), navySlide("")),
  );
  assert.equal(sheet.mimeType, "image/jpeg");
  const image = await decoded(sheet);
  assert.equal(image.width, 1600);
  // Two rows of cells for three slides.
  assert.equal(image.height, 24 + 2 * (32 + 430 + 24));
  // The slide's text box spans x 58–708 and y 138–233 of its cell.
  const text = ([x, y]) => [x + 58, y + 138, 650, 95];
  const filled = [cellAt(0, 0), cellAt(1, 0), cellAt(0, 1)];
  for (const [x, y, w, h] of filled) {
    const [r, g, b] = await pixelAt(sheet, x + 10, y + h - 10);
    assert.ok(r < 80 && b > 60 && b < 140, `navy ${r},${g},${b}`);
  }
  assert.ok((await lightPixels(sheet, text(filled[0]))) > 2000, "Latin text");
  assert.ok((await lightPixels(sheet, text(filled[1]))) > 2000, "CJK text");
  assert.equal(await lightPixels(sheet, text(filled[2])), 0, "no text");
  // The fourth cell has no slide: the sheet's light background.
  const [x, y, w, h] = cellAt(1, 1);
  const [r, g, b] = await pixelAt(sheet, x + w / 2, y + h / 2);
  assert.ok(r > 200 && g > 200 && b > 200, `empty ${r},${g},${b}`);
});

test("a contact sheet of one slide has one row and keeps the width", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  const image = await decoded(await contactSheet(numbered(navySlide("Only"))));
  assert.equal(image.width, 1600);
  assert.equal(image.height, 24 + 32 + 430 + 24);
  // One column of 800 px: cells 752 wide and 423 high.
  const column = await decoded(
    await contactSheet(numbered(navySlide("A"), navySlide("B")), {
      columns: 1,
      width: 800,
    }),
  );
  assert.equal(column.width, 800);
  assert.equal(column.height, 24 + 2 * (32 + 423 + 24));
});

test("a contact sheet stays within its byte budget", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  const sheet = await contactSheet(
    numbered(
      ...Array.from({ length: 24 }, (_, i) => navySlide(`Slide ${i + 1}`)),
    ),
  );
  assert.ok(Buffer.from(sheet.data, "base64").length <= 600 * 1024);
});

test("a contact sheet writes each cell's own label over it", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  // Slide 3 has no page: the third cell is slide 4, and says so.
  const slides = numbered(navySlide("A"), navySlide("B"), navySlide("D"));
  const missing = await contactSheet([
    ...slides.slice(0, 2),
    { label: "4", svg: slides[2].svg },
  ]);
  const positional = await contactSheet(slides);
  // Inside the third cell, away from the JPEG blocks it shares with its
  // label strip.
  const [x, y, w, h] = cellAt(0, 1);
  const inside = [x + 16, y + 16, w - 32, h - 32];
  for (const cell of [labelAt(0, 0), labelAt(1, 0), inside])
    assert.ok(
      (await region(missing, cell)).equals(await region(positional, cell)),
    );
  assert.ok(
    !(await region(missing, labelAt(0, 1))).equals(
      await region(positional, labelAt(0, 1)),
    ),
    "the third label reads 4, not 3",
  );
});

test("a slide that cannot be drawn keeps its cell, as a placeholder", async (t) => {
  if (!(await readsSvg())) {
    t.skip("SVG unsupported");
    return;
  }
  const sheet = await contactSheet([
    { label: "1", svg: navySlide("ok") },
    { label: "2", svg: Buffer.from("no svg") },
    { label: "3", svg: null },
  ]);
  const image = await decoded(sheet);
  assert.equal(image.height, 24 + 2 * (32 + 430 + 24));
  const [r, g, b] = await pixelAt(sheet, cellAt(0, 0)[0] + 10, 56 + 420);
  assert.ok(r < 80 && b > 60 && b < 140, `navy ${r},${g},${b}`);
  // The placeholder is neither a slide nor the sheet's background.
  for (const [x, y] of [cellAt(1, 0), cellAt(0, 1)]) {
    const [r, g, b] = await pixelAt(sheet, x + 10, y + 10);
    assert.ok(
      Math.abs(r - 148) < 12 &&
        Math.abs(g - 163) < 12 &&
        Math.abs(b - 184) < 12,
      `placeholder ${r},${g},${b}`,
    );
  }
});

test("a contact sheet needs a slide it can draw", async () => {
  await assert.rejects(contactSheet([]), /at least one/);
  await assert.rejects(
    contactSheet([
      { label: "1", svg: Buffer.from("no svg") },
      { label: "2", svg: null },
    ]),
    /No slide could be drawn/,
  );
});

test("bytes that are no image are refused, for the caller to skip", async () => {
  await assert.rejects(thumbnailFromSvg(Buffer.from("not an svg")));
  await assert.rejects(thumbnailFromPng(Buffer.from("not a png")));
});
