import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import {
  normalizeOfficeArrowheads,
  rasterPages,
} from "../src/document-preview.mjs";
import { thumbnailFromSvg } from "../src/thumbnails.mjs";
import { renderFilePage } from "../src/file-preview-page.mjs";
import { documentTools } from "../src/document-tools.mjs";

// Marker/line geometry only, from the self-authored native four-node flow.
// No user text, identity, generated artwork or private source path is retained.
const sourceUrl = new URL(
  "./fixtures/officecli-arrowheads.svg",
  import.meta.url,
);
const source = await readFile(sourceUrl, "utf8");
const hash = (value) => createHash("sha256").update(value).digest("hex");

test("actual OfficeCLI triangle marker geometry survives the final preview JPEG", async (t) => {
  const before = hash(source);
  const written = [];
  const result = await rasterPages([{ page: 4, svg: source }], {
    render: (svg) =>
      thumbnailFromSvg(svg, { maxEdge: 1920, maxBytes: 320 * 1024 }),
    write: async (name, bytes) => written.push({ name, bytes }),
  });
  assert.deepEqual(result.pages, [4]);
  const image = await loadImage(written[0].bytes);
  assert.equal(image.width, 1920);
  assert.equal(image.height, 1080);
  const context = createCanvas(1920, 1080).getContext("2d");
  context.drawImage(image, 0, 0);
  const counts = [483.84, 843.84, 1203.84].map((x) => {
    const region = context.getImageData(Math.ceil(x) - 25, 477, 25, 25).data;
    let ink = 0;
    for (let pixel = 0; pixel < region.length / 4; pixel++) {
      // Exclude the shaft: only pixels above/below the line establish a head.
      const y = 477 + Math.floor(pixel / 25);
      if (Math.abs(y - 489.6) <= 3) continue;
      const i = pixel * 4;
      if (region[i] < 150 && region[i + 1] < 150 && region[i + 2] < 150) ink++;
    }
    return ink;
  });
  t.diagnostic(`final JPEG triangle-head pixels: ${JSON.stringify(counts)}`);
  assert.ok(
    counts.every((count) => count >= 60),
    String(counts),
  );
  assert.equal(hash(await readFile(sourceUrl)), before);
});

async function scaledHeadPixels(bytes, { width, left = 0, top = 0 }) {
  const image = await loadImage(bytes);
  const context = createCanvas(image.width, image.height).getContext("2d");
  context.drawImage(image, 0, 0);
  const scale = width / 1920;
  return [483.84, 843.84, 1203.84].map((x) => {
    const x0 = Math.ceil(left + (x - 24) * scale);
    const y0 = Math.ceil(top + 477 * scale);
    const w = Math.max(1, Math.floor(22 * scale));
    const h = Math.max(1, Math.floor(25 * scale));
    const region = context.getImageData(x0, y0, w, h).data;
    let ink = 0;
    for (let pixel = 0; pixel < region.length / 4; pixel++) {
      const y = (y0 + Math.floor(pixel / w) - top) / scale;
      if (Math.abs(y - 489.6) <= 3) continue;
      const i = pixel * 4;
      if (region[i] < 150 && region[i + 1] < 150 && region[i + 2] < 150) ink++;
    }
    return ink;
  });
}

test("the actual workspace page adapter renders arrow tips before the target endpoint", async (t) => {
  const result = await renderFilePage({ page: 4, scale: 1, svg: source });
  const counts = await scaledHeadPixels(result.bytes, { width: 1536 });
  t.diagnostic(`workspace JPEG head pixels: ${JSON.stringify(counts)}`);
  assert.ok(
    counts.every((count) => count >= 40),
    String(counts),
  );
  assert.equal(hash(await readFile(sourceUrl)), hash(source));
});

test("the actual model overview adapter includes rendered arrow tips", async (t) => {
  const directory = await mkdtemp(
    path.join(os.tmpdir(), "ofox-marker-preview-"),
  );
  t.after(() => rm(directory, { recursive: true, force: true }));
  const svgPath = path.join(directory, "4.svg");
  await writeFile(svgPath, source);
  const runtime = {
    wrapTool: (_context, name, _description, parameters, execute) => ({
      name,
      parameters,
      execute: (args) => execute(args, undefined, "preview-call"),
    }),
    hostTool: async () => ({
      status: "completed",
      preview: true,
      previewId: "preview-marker",
      slides: 1,
      issues: [],
      pagePaths: [svgPath],
    }),
  };
  const tools = documentTools(
    runtime,
    { params: { documents: { formats: ["pptx"], python: true } } },
    { vision: true },
  );
  const result = await tools
    .find((tool) => tool.name === "create_document")
    .execute({
      name: "marker.pptx",
      format: "pptx",
      content: { kind: "python", code: "prs = new_deck()" },
      preview: true,
    });
  const image = result.content.find((block) => block.type === "image");
  assert.ok(image);
  const counts = await scaledHeadPixels(Buffer.from(image.data, "base64"), {
    width: 764,
    left: 24,
    top: 56,
  });
  t.diagnostic(`model overview JPEG head pixels: ${JSON.stringify(counts)}`);
  assert.ok(
    counts.every((count) => count >= 6),
    String(counts),
  );
  assert.equal(hash(await readFile(svgPath)), hash(source));
});

const single = ({
  marker = "",
  polygon = "",
  line = "",
  group = "",
  defs = "",
} = {}) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><defs ${defs}><marker id="arrow" markerWidth="9" markerHeight="9" refX="0" refY="4.5" orient="auto" ${marker}><polygon points="0,0 9,4.5 0,9" fill="#18293B" ${polygon}/></marker></defs><g ${group}><line x1="100" y1="100" x2="200" y2="100" stroke="#18293B" stroke-width="3" marker-end="url(#arrow)" ${line}/></g></svg>`;

test("plain triangle geometry preserves stroke units, orientation, start markers and surrounding transforms", () => {
  const projected = normalizeOfficeArrowheads(single());
  assert.ok(projected.includes('points="200,86.5 227,100 200,113.5"'));
  assert.ok(!projected.includes('marker-end="url(#arrow)"'));
  const absolute = normalizeOfficeArrowheads(
    single({ marker: 'markerUnits="userSpaceOnUse"' }),
  );
  assert.ok(absolute.includes('points="200,95.5 209,100 200,104.5"'));
  const vertical = normalizeOfficeArrowheads(
    single().replace('x2="200" y2="100"', 'x2="100" y2="200"'),
  );
  assert.ok(vertical.includes('points="113.5,200 100,227 86.5,200"'));
  const start = normalizeOfficeArrowheads(
    single()
      .replace('marker-end="url(#arrow)"', 'marker-start="url(#arrow)"')
      .replace('points="0,0 9,4.5 0,9"', 'points="9,0 0,4.5 9,9"'),
  );
  assert.ok(start.includes('points="127,86.5 100,100 127,113.5"'));
  const transformed = single({
    group: 'transform="translate(4 6) rotate(12)" opacity="0.5"',
  });
  const changed = normalizeOfficeArrowheads(transformed);
  assert.notEqual(changed, transformed);
  assert.ok(
    changed.includes('<g transform="translate(4 6) rotate(12)" opacity="0.5">'),
  );
  assert.equal(
    normalizeOfficeArrowheads(changed),
    changed,
    "projection is idempotent",
  );
});

test("only the proven OfficeCLI arrowN signature anchors its forward tip to the endpoint", () => {
  const office = single()
    .replaceAll('id="arrow"', 'id="arrow0"')
    .replaceAll("url(#arrow)", "url(#arrow0)");
  const changed = normalizeOfficeArrowheads(office);
  assert.ok(changed.includes('points="173,86.5 200,100 173,113.5"'));
  const reverse = office
    .replace('x1="100"', 'x1="200"')
    .replace('x2="200"', 'x2="100"');
  assert.ok(
    normalizeOfficeArrowheads(reverse).includes(
      'points="127,113.5 100,100 127,86.5"',
    ),
  );
  const vertical = office.replace('x2="200" y2="100"', 'x2="100" y2="200"');
  assert.ok(
    normalizeOfficeArrowheads(vertical).includes(
      'points="113.5,173 100,200 86.5,173"',
    ),
  );
  // Other SVG marker semantics are preserved; no guessing based on the word arrow.
  assert.ok(
    normalizeOfficeArrowheads(single()).includes(
      'points="200,86.5 227,100 200,113.5"',
    ),
  );
  const start = office.replace("marker-end=", "marker-start=");
  assert.ok(
    normalizeOfficeArrowheads(start).includes(
      'points="100,86.5 127,100 100,113.5"',
    ),
  );
});

test("ambiguous, cyclic and unsupported markers or drawing contexts remain unchanged", () => {
  const opening = "<g >";
  const cases = [
    ...[
      'viewBox="0 0 9 9"',
      'orient="auto-start-reverse"',
      'markerUnits="unknown"',
      'opacity="0.5"',
      'markerWidth="NaN"',
      'refX="Infinity"',
    ].map((extra) =>
      single().replace(
        'orient="auto"',
        extra.includes("orient=") ? extra : `orient="auto" ${extra}`,
      ),
    ),
    ...[
      'stroke="red"',
      'transform="scale(2)"',
      'style="fill:red"',
      'clip-path="url(#clip)"',
    ].map((polygon) => single({ polygon })),
    ...[
      'transform="translate(Infinity 0)"',
      'transform="matrix(1 0 NaN 1 0 0)"',
      'transform="scale(1e999)"',
      'style="stroke:red"',
      'stroke="red"',
      'fill-opacity="0.5"',
      'clip-path="url(#clip)"',
    ].map((group) => single({ group })),
    ...['stroke="red"', 'style="fill:red"', 'transform="translate(3 4)"'].map(
      (defs) => single({ defs }),
    ),
    ...[
      'transform="rotate(12)"',
      'stroke-dasharray="3 2"',
      'opacity="0.5"',
      'marker-mid="url(#arrow)"',
    ].map((line) => single({ line })),
    single().replace('x2="200"', 'x2="NaN"'),
    single().replace('stroke-width="3"', 'stroke-width="Infinity"'),
    single().replace(
      '<polygon points="0,0 9,4.5 0,9" fill="#18293B" />',
      '<use href="#arrow"/>',
    ),
    single().replace('fill="#18293B"', 'fill="url(#arrow)"'),
    single().replace("</defs>", '<path id="arrow" d="M0 0"/></defs>'),
    single().replace(
      "</defs>",
      '<path xmlns:q="urn:untrusted" q:id="arrow" d="M0 0"/></defs>',
    ),
    single().replace("url(#arrow)", "url(#missing)"),
    single()
      .replace("<line ", '<path d="M100 100 L200 100" ')
      .replace("</g>", "</g>"),
    single().replace("http://www.w3.org/2000/svg", "urn:not-svg"),
    ...["defs", "clipPath", "mask", "pattern", "symbol"].map((tag) =>
      single().replace(opening, `<${tag}>`).replace("</g>", `</${tag}>`),
    ),
  ];
  for (const value of cases)
    assert.equal(normalizeOfficeArrowheads(value), value);
});

test("unsafe references and XML never reach the rasterizer and oversized inputs do not expand", async () => {
  let renders = 0;
  for (const value of [
    single().replace("url(#arrow)", "url(https://example.test/marker)"),
    single().replace(
      "<polygon ",
      '<image href="file:///private/example"/><polygon ',
    ),
    '<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///private/example">]>' + single(),
    single().replace("<line ", "<script>bad()</script><line "),
  ]) {
    assert.equal(normalizeOfficeArrowheads(value), value);
    assert.deepEqual(
      (
        await rasterPages([{ page: 4, svg: value }], {
          render: async () => {
            renders++;
            throw new Error("unsafe SVG reached rasterizer");
          },
        })
      ).pages,
      [],
    );
  }
  assert.equal(renders, 0);
  const huge = single().replace(
    "</svg>",
    " ".repeat(4 * 1024 * 1024) + "</svg>",
  );
  assert.equal(normalizeOfficeArrowheads(huge), huge);
});
