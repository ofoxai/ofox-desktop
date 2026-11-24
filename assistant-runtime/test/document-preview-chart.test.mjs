import assert from "node:assert/strict";
import test from "node:test";
import { createCanvas, loadImage } from "@napi-rs/canvas";
import { rasterPages } from "../src/document-preview.mjs";
import { renderFilePage } from "../src/file-preview-page.mjs";
import { contactSheet } from "../src/thumbnails.mjs";

// The nested viewport and axis geometry are those emitted by OfficeCLI for
// the three-category column chart in the Win11 acceptance deck. Separate ink
// colors let the test measure the tick and title rows in the rendered JPEG.
const chart = `<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480">
  <g transform="translate(32,24)">
    <rect width="704" height="412" fill="white" fill-opacity="0"/>
    <text x="352" y="12" text-anchor="middle" font-size="14" fill="#111">合成示例</text>
    <svg x="0" y="16" width="704" height="396" viewBox="0 0 704 396" preserveAspectRatio="xMidYMid meet">
      <line x1="40" y1="371" x2="689" y2="371" stroke="#111" stroke-width="1"/>
      <rect x="91.2" y="280.8" width="113.9" height="90.3" fill="#444"/>
      <rect x="307.6" y="208.6" width="113.9" height="162.5" fill="#444"/>
      <rect x="523.9" y="64.2" width="113.9" height="306.9" fill="#444"/>
      <line x1="148.2" y1="371" x2="148.2" y2="375" stroke="#111" stroke-width="1"/>
      <text x="148.2" y="387" fill="#1234dd" font-size="13" text-anchor="middle">128</text>
      <line x1="364.5" y1="371" x2="364.5" y2="375" stroke="#111" stroke-width="1"/>
      <text x="364.5" y="387" fill="#1234dd" font-size="13" text-anchor="middle">256</text>
      <line x1="580.8" y1="371" x2="580.8" y2="375" stroke="#111" stroke-width="1"/>
      <text x="580.8" y="387" fill="#1234dd" font-size="13" text-anchor="middle">512</text>
      <text x="352" y="394" fill="#d91d10" font-size="13" text-anchor="middle">序列长度 (token)</text>
    </svg>
  </g>
</svg>`;

async function assertLabelSeparation(bytes, t) {
  const image = await loadImage(bytes);
  const context = createCanvas(image.width, image.height).getContext("2d");
  context.drawImage(image, 0, 0);
  const pixels = context.getImageData(0, 0, image.width, image.height).data;
  let titleTop = image.height;
  let tickBottom = -1;
  for (let i = 0; i < pixels.length; i += 4) {
    const y = Math.floor(i / (4 * image.width));
    const red = pixels[i];
    const green = pixels[i + 1];
    const blue = pixels[i + 2];
    if (red > green + 70 && red > blue + 70) titleTop = Math.min(titleTop, y);
    if (blue > red + 70 && blue > green + 70)
      tickBottom = Math.max(tickBottom, y);
  }
  assert.ok(
    titleTop < image.height && tickBottom >= 0,
    "both rows are visible",
  );
  t.diagnostic(
    `tick ink ends at ${tickBottom}; title ink begins at ${titleTop}`,
  );
  assert.ok(
    titleTop - tickBottom >= 2,
    "axis title and tick ink must be separated",
  );
}

test("the file workspace preview separates OfficeCLI chart ticks and their title", async (t) => {
  const result = await renderFilePage({ page: 7, scale: 1, svg: chart });
  await assertLabelSeparation(result.bytes, t);
});

test("the persisted document preview separates OfficeCLI chart ticks and their title", async (t) => {
  const files = [];
  const result = await rasterPages([{ page: 7, svg: chart }], {
    write: async (_name, bytes) => files.push(bytes),
  });
  assert.deepEqual(result.pages, [7]);
  await assertLabelSeparation(files[0], t);
});

test("the model's overview separates OfficeCLI chart ticks and their title", async (t) => {
  const result = await contactSheet([{ label: 7, svg: Buffer.from(chart) }], {
    columns: 1,
    width: 900,
  });
  await assertLabelSeparation(Buffer.from(result.data, "base64"), t);
});

test("nested SVGs with incomplete axis geometry or a styled parent keep their placement", async () => {
  const unrelated = [
    chart.replaceAll(/<line [^>]+\/>/g, ""),
    chart.replace(
      'transform="translate(32,24)"',
      'transform="translate(32,24)" opacity="0.75"',
    ),
  ];
  for (const svg of unrelated) {
    const equivalent = svg.replace("xMidYMid meet", "xMidYMid");
    const actual = await renderFilePage({ page: 7, scale: 1, svg });
    const expected = await renderFilePage({
      page: 7,
      scale: 1,
      svg: equivalent,
    });
    assert.deepEqual(actual.bytes, expected.bytes);
  }
});

test("hidden axis titles or tick labels do not change the plot geometry", async () => {
  const hiddenLabels = [
    chart.replace('y="394"', 'y="394" display="none"'),
    chart.replace('y="394"', 'y="394" visibility="hidden"'),
    chart.replaceAll('y="387"', 'y="387" opacity="0"'),
    chart.replaceAll('fill="#1234dd"', 'fill="none"'),
  ];
  for (const svg of hiddenLabels) {
    const equivalent = svg.replace("xMidYMid meet", "xMidYMid");
    const actual = await renderFilePage({ page: 7, scale: 1, svg });
    const expected = await renderFilePage({
      page: 7,
      scale: 1,
      svg: equivalent,
    });
    assert.deepEqual(actual.bytes, expected.bytes);
  }
});
