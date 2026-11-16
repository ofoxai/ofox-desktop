import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import {
  normalizeOfficePatternImages,
  rasterPages,
} from "../src/document-preview.mjs";
import { thumbnailFromSvg } from "../src/thumbnails.mjs";

const source = await readFile(
  new URL("./fixtures/officecli-rounded-math.svg", import.meta.url),
  "utf8",
);
const hash = (value) => createHash("sha256").update(value).digest("hex");

test("the actual OfficeCLI rounded equation reaches final JPEG pixels without rewriting its SVG", async (t) => {
  const before = hash(source);
  const written = [];
  const result = await rasterPages([{ page: 5, svg: source }], {
    render: (svg) =>
      thumbnailFromSvg(svg, { maxEdge: 1920, maxBytes: 320 * 1024 }),
    write: async (name, bytes) => written.push({ name, bytes }),
  });
  assert.deepEqual(result.pages, [5]);
  assert.equal(written[0].name, "document-preview-5.jpg");
  const image = await loadImage(written[0].bytes);
  assert.equal(image.width, 1920);
  assert.equal(image.height, 1080);
  const canvas = createCanvas(1920, 1080);
  const context = canvas.getContext("2d");
  context.drawImage(image, 0, 0);
  const rgba = context.getImageData(1144, 907, 636, 44).data;
  let ink = 0;
  for (let index = 0; index < rgba.length; index += 4)
    if (
      rgba[index] < 150 &&
      rgba[index + 1] < 150 &&
      rgba[index + 2] < 150 &&
      rgba[index + 3] === 255
    )
      ink++;
  t.diagnostic(`actual MultiHead formula ROI: ${ink} opaque dark JPEG pixels`);
  assert.ok(
    ink >= 800,
    `actual MultiHead formula ROI has only ${ink} dark pixels`,
  );
  assert.equal(hash(source), before);
});

test("OfficeCLI rounding recovery retains crop, precision, context and external-reference guards", async () => {
  for (const invalid of [
    source.replace('height="41.45"', 'height="41.46"'),
    source.replace('width="633.6"', 'width="640"'),
    source.replace('height="41.45"', 'height="40"'),
    source.replace('height="41.45"', 'height="41.451"'),
    source.replace(
      'fill="url(#pat7)"',
      'fill="url(#pat7)" clip-path="url(#crop)"',
    ),
    source.replace("<g transform=", '<g fill-opacity="0.5" transform='),
    source.replace("<g transform=", '<g clip-path="url(#crop)" transform='),
    source.replace("<image ", '<image transform="scale(2)" '),
    source.replace("</defs>", '<pattern id="pat7"/></defs>'),
    source.replace(
      /data:image\/png;base64,[^"]+/,
      "https://example.test/image.png",
    ),
  ])
    assert.equal(normalizeOfficePatternImages(invalid), invalid);
  const exact = source.replace('height="41.45"', 'height="41.450467289719626"');
  assert.notEqual(normalizeOfficePatternImages(exact), exact);
  let renders = 0;
  const external = source.replace(
    /data:image\/png;base64,[^"]+/,
    "https://example.test/image.png",
  );
  const result = await rasterPages([{ page: 5, svg: external }], {
    render: async () => {
      renders++;
      throw new Error("external SVG must not reach the renderer");
    },
  });
  assert.deepEqual(result.pages, []);
  assert.equal(renders, 0);
});
