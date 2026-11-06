import test from "node:test";
import assert from "node:assert/strict";
import { createCanvas, ImageData, loadImage } from "@napi-rs/canvas";
import {
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

test("an SVG preview is drawn at the full long edge when the canvas reads SVG", async (t) => {
  // A slide's size only in its viewBox: the canvas gives it a small default.
  const svg = Buffer.from(
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 960 540"><rect width="960" height="540" fill="#fff"/><text x="40" y="100" font-size="60">Title</text></svg>',
  );
  let thumbnail;
  try {
    thumbnail = await thumbnailFromSvg(svg);
  } catch (error) {
    // This build of the canvas has no SVG support: the caller skips the
    // preview, which document-tools tests cover.
    t.skip(`SVG unsupported: ${error.message}`);
    return;
  }
  const image = await decoded(thumbnail);
  assert.equal(image.width, 768);
  // The default size it loads at (267x150) is rounded.
  assert.ok(Math.abs(image.height - 432) <= 1, `height ${image.height}`);
});

test("bytes that are no image are refused, for the caller to skip", async () => {
  await assert.rejects(thumbnailFromSvg(Buffer.from("not an svg")));
  await assert.rejects(thumbnailFromPng(Buffer.from("not a png")));
});
