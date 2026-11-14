// Host-only on-demand workspace renderer, independent of the model thumbnail
// limit. Filesystem names and quality are fixed; no active SVG reaches the app.
import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import path from "node:path";
import {
  normalizeOfficeArrowheads,
  normalizeOfficePatternImages,
  safeSvg,
} from "./document-preview.mjs";
import { thumbnailFromSvg } from "./thumbnails.mjs";

export async function renderFilePage(input, render = thumbnailFromSvg) {
  assert.ok(
    Number.isInteger(input?.page) && input.page >= 1 && input.page <= 1000,
  );
  assert.ok(input.scale === 1 || input.scale === 2);
  assert.ok(safeSvg(input.svg));
  const svg = normalizeOfficeArrowheads(
    normalizeOfficePatternImages(input.svg),
  );
  assert.ok(safeSvg(svg));
  const maxEdge = 1536 * input.scale;
  const result = await render(Buffer.from(svg), {
    maxBytes: 2 * 1024 * 1024,
    maxEdge,
  });
  assert.equal(result?.mimeType, "image/jpeg");
  const bytes = Buffer.from(result.data, "base64");
  assert.ok(bytes.length > 4 && bytes.length <= 2 * 1024 * 1024);
  assert.ok(bytes.subarray(0, 2).equals(Buffer.from([255, 216])));
  assert.ok(bytes.subarray(-2).equals(Buffer.from([255, 217])));
  // Source dimensions, scaled exactly as the raster helper does.
  const { SaxesParser } = await import("saxes");
  let width, height;
  const parser = new SaxesParser();
  parser.on("opentag", (tag) => {
    if (width !== undefined) return;
    const box = tag.attributes.viewBox
      ?.trim()
      .split(/[\s,]+/)
      .map(Number);
    const axis = (name, index) =>
      tag.attributes[name] === undefined
        ? box?.[index]
        : Number.parseFloat(tag.attributes[name]) *
          (tag.attributes[name].endsWith("pt") ? 4 / 3 : 1);
    width = axis("width", 2);
    height = axis("height", 3);
  });
  parser.write(svg).close();
  const factor = maxEdge / Math.max(width, height);
  return {
    bytes,
    width: Math.round(width * factor),
    height: Math.round(height * factor),
  };
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
) {
  const bytes = await readFile("file-preview-input.json");
  assert.ok(bytes.length <= 6 * 1024 * 1024);
  const result = await renderFilePage(JSON.parse(bytes.toString("utf8")));
  await writeFile("file-preview.jpg", result.bytes, {
    flag: "wx",
    mode: 0o600,
  });
  process.stdout.write(
    JSON.stringify({ width: result.width, height: result.height }),
  );
}
