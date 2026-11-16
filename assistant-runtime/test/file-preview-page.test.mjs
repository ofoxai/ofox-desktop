import { test } from "node:test";
import assert from "node:assert/strict";
import { renderFilePage } from "../src/file-preview-page.mjs";

const svg =
  '<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="720"><rect width="1280" height="720" fill="#fff"/></svg>';
const jpeg = {
  mimeType: "image/jpeg",
  data: Buffer.from([255, 216, 255, 224, 255, 217]).toString("base64"),
};

test("page 70 is independently rendered at requested quality", async () => {
  let calls = 0;
  const found = await renderFilePage(
    { page: 70, scale: 2, svg },
    async (_source, options) => {
      calls++;
      assert.equal(options.maxEdge, 3072);
      assert.equal(options.maxBytes, 2 * 1024 * 1024);
      return jpeg;
    },
  );
  assert.equal(calls, 1);
  assert.deepEqual([found.width, found.height], [3072, 1728]);
});

test("active SVG and unsupported resource requests never reach the raster addon", async () => {
  let calls = 0;
  const render = () => {
    calls++;
    return jpeg;
  };
  for (const input of [
    { page: 1001, scale: 1, svg },
    { page: 1, scale: 3, svg },
    {
      page: 1,
      scale: 1,
      svg: svg.replace(
        "<rect",
        '<script>fetch("file:///etc/passwd")</script><rect',
      ),
    },
    {
      page: 1,
      scale: 1,
      svg: svg.replace(
        "<rect",
        '<image href="https://example.test/exfil"/><rect',
      ),
    },
  ])
    await assert.rejects(renderFilePage(input, render));
  assert.equal(calls, 0);
});

test("a small embedded image header cannot request unbounded raster memory", async () => {
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5wAAAABJRU5ErkJggg==",
    "base64",
  );
  png.writeUInt32BE(0x7fffffff, 16);
  const source = svg.replace(
    "<rect",
    `<image href="data:image/png;base64,${png.toString("base64")}"/><rect`,
  );
  let calls = 0;
  await assert.rejects(
    renderFilePage({ page: 70, scale: 1, svg: source }, async () => {
      calls++;
      return jpeg;
    }),
  );
  assert.equal(calls, 0);
});
