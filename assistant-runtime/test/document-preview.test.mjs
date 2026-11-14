import assert from "node:assert/strict";
import test from "node:test";
import {
  normalizeOfficePatternImages,
  rasterPages,
} from "../src/document-preview.mjs";

const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><rect width="160" height="90" fill="white"/></svg>';
test("document previews rasterize bounded SVG pages to fixed JPEG names", async () => {
  const written = [];
  const result = await rasterPages([{ page: 1, svg }], {
    write: async (name, bytes) => written.push([name, bytes]),
  });
  assert.deepEqual(result.pages, [1]);
  assert.equal(written[0][0], "document-preview-1.jpg");
  assert.ok(written[0][1].length <= 200 * 1024);
  assert.equal(written[0][1].subarray(0, 2).toString("hex"), "ffd8");
});
test("target source page IDs retain their actual numbers without raising the page-count cap", async () => {
  const written = [];
  const result = await rasterPages(
    [
      { page: 70, svg },
      { page: 1000, svg },
    ],
    { write: async (name, bytes) => written.push([name, bytes]) },
  );
  assert.deepEqual(result.pages, [70, 1000]);
  assert.deepEqual(
    written.map(([name]) => name),
    ["document-preview-70.jpg", "document-preview-1000.jpg"],
  );
  assert.ok(
    written.every(
      ([, bytes]) => bytes.subarray(0, 2).toString("hex") === "ffd8",
    ),
  );
  let renders = 0;
  const options = {
    render: async () => {
      renders++;
      throw new Error("unexpected render");
    },
    write: async () => assert.fail("must not write"),
  };
  await assert.rejects(rasterPages([{ page: 1001, svg }], options));
  await assert.rejects(
    rasterPages(
      Array.from({ length: 61 }, (_, i) => ({ page: i + 1, svg })),
      options,
    ),
  );
  assert.equal(renders, 0);
});

test("document previews reject active/external SVG references and caller paths before rendering", async () => {
  let renders = 0;
  const options = {
    render: async () => {
      renders++;
      throw new Error("unexpected render");
    },
    write: async () => assert.fail("must not write"),
  };
  for (const bad of [
    svg.replace("<rect", "<script>alert(1)</script><rect"),
    svg.replace("<rect", '<image href="file:///private/test"/><rect'),
    svg.replace("<rect", '<image href="https://example.test/a.png"/><rect'),
    svg.replace(
      "<rect",
      '<style>@import "https://example.test/a.css";</style><rect',
    ),
    '<!DOCTYPE svg [<!ENTITY x SYSTEM "file:///private/test">]>' + svg,
  ]) {
    assert.deepEqual(
      (await rasterPages([{ page: 1, svg: bad }], options)).pages,
      [],
    );
  }
  await assert.rejects(rasterPages([{ page: "../1", svg }], options));
  await assert.rejects(
    rasterPages(
      Array.from({ length: 61 }, (_, i) => ({ page: i + 1, svg })),
      options,
    ),
  );
  assert.equal(renders, 0);
});

const png =
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5wAAAABJRU5ErkJggg==";
const patternSvg = ({
  pattern = "",
  image = "",
  rect = "",
  width = "2",
  height = "2",
  extra = "",
  href = `data:image/png;base64,${png}`,
} = {}) =>
  `<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><text>🧪中文</text><defs><pattern id="pat0" patternUnits="objectBoundingBox" width="1" height="1" ${pattern}><image href="${href}" width="100%" height="100%" preserveAspectRatio="xMidYMid slice" ${image}/>${extra}</pattern></defs><g transform="translate(3 4) rotate(10)"><rect width="${width}" height="${height}" fill="url(#pat0)" ${rect}/></g></svg>`;

test("only aspect-matched plain OfficeCLI PNG rectangles become direct images", () => {
  const source = patternSvg();
  const projected = normalizeOfficePatternImages(source);
  assert.notEqual(projected, source);
  assert.ok(
    projected.includes(
      '<g transform="translate(3 4) rotate(10)"><image width="2" height="2"',
    ),
  );
  assert.ok(projected.includes("<text>🧪中文</text>"));
  assert.equal((projected.match(/<image\b/g) ?? []).length, 2);
  assert.equal(
    (projected.match(/preserveAspectRatio="xMidYMid slice"/g) ?? []).length,
    2,
  );
  assert.ok(projected.includes('<pattern id="pat0"'));
});

test("pattern projection leaves geometry, clipping, transforms and ambiguous resources unchanged", () => {
  const cases = [
    ...[
      'rx="1"',
      'ry="1"',
      'stroke="black"',
      'clip-path="url(#clip)"',
      'transform="scale(2)"',
      'x="1"',
      'style="opacity:.5"',
      'id="rect"',
    ].map((rect) => patternSvg({ rect })),
    ...[
      'patternTransform="scale(2)"',
      'patternContentUnits="userSpaceOnUse"',
      'viewBox="0 0 1 1"',
      'x="1"',
    ].map((pattern) => patternSvg({ pattern })),
    ...[
      'transform="rotate(3)"',
      'opacity=".5"',
      'filter="url(#filter)"',
      'x="1"',
    ].map((image) => patternSvg({ image })),
    patternSvg({ height: "3" }),
    patternSvg({ extra: '<rect width="1" height="1"/>' }),
    patternSvg({ href: "https://example.test/a.png" }),
    patternSvg().replace(
      "</defs>",
      '<rect id="pat0" width="1" height="1"/></defs>',
    ),
    patternSvg().replace("http://www.w3.org/2000/svg", "urn:fake-svg"),
  ];
  for (const source of cases)
    assert.equal(normalizeOfficePatternImages(source), source);
});

test("pattern projection never expands a raster SVG beyond the input/output byte limit", () => {
  const source = patternSvg({
    href: `data:image/png;base64,${png}${"A".repeat(2.5 * 1024 * 1024)}`,
  });
  assert.equal(normalizeOfficePatternImages(source), source);
});

test("pattern projection only rewrites ordinary svg/g drawing contexts without inherited paint semantics", () => {
  const source = patternSvg();
  const opening = '<g transform="translate(3 4) rotate(10)">';
  const cases = [
    ...["defs", "clipPath", "mask", "pattern", "symbol"].map((name) =>
      source.replace(opening, `<${name}>`).replace("</g>", `</${name}>`),
    ),
    ...[
      'fill-opacity=".5"',
      'stroke="black"',
      'stroke-width="2"',
      'style="fill-opacity:.5"',
      'class="paint"',
    ].map((attrs) => source.replace(opening, `<g ${attrs}>`)),
    ...['fill-opacity=".5"', 'stroke="black"', 'style="stroke:black"'].map(
      (attrs) => source.replace('height="100">', `height="100" ${attrs}>`),
    ),
  ];
  for (const svg of cases) assert.equal(normalizeOfficePatternImages(svg), svg);
  const opacity = source.replace(
    opening,
    '<g transform="translate(3 4) rotate(10)" opacity=".5">',
  );
  assert.notEqual(normalizeOfficePatternImages(opacity), opacity);
  assert.ok(
    normalizeOfficePatternImages(opacity).includes('opacity=".5"><image'),
  );
});
