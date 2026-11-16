import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { options, run, zipEntries } from "./fixtures/python-engine.mjs";

test(
  "architectural cover metadata stays clear of the page index on both alternating sides",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(style="architectural-plan")
for page in (1, 2):
    s = cover_slide(prs, "概念评审", meta="产品·设计·工程 内部讨论")
    navigation = next(item for item in s.shapes if item.name == "Panel navigation")
    metadata = next(item for item in s.shapes if item.has_text_frame and item.text == "产品·设计·工程 内部讨论")
    overlap_w = min(metadata.left + metadata.width, navigation.left + navigation.width) - max(metadata.left, navigation.left)
    overlap_h = min(metadata.top + metadata.height, navigation.top + navigation.height) - max(metadata.top, navigation.top)
    assert overlap_w <= 0 or overlap_h <= 0, "cover metadata overlaps its native page index"
    assert metadata.left >= 0 and metadata.top >= 0
    assert metadata.left + metadata.width <= prs.slide_width
    assert metadata.top + metadata.height <= prs.slide_height
    assert navigation.text == f"{page:02d}"
`,
    );
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(await readFile(result.lint, "utf8")), []);
    const entries = await zipEntries(result.out);
    for (let page = 1; page <= 2; page++) {
      const xml = entries.get(`ppt/slides/slide${page}.xml`).toString();
      assert.match(xml, /产品·设计·工程 内部讨论/);
      assert.match(xml, /Panel navigation/);
      assert.match(xml, /type="title"/);
    }
  },
);

test(
  "cover metadata retains its normal placement in sibling styles",
  options,
  async (t) => {
    const result = await run(
      t,
      `
for style in ("navy", "blue-professional", "cartesian", "course-module", "product-launch"):
    prs = new_deck(style=style)
    s = cover_slide(prs, "概念评审", meta="评审团队")
    metadata = next(item for item in s.shapes if item.has_text_frame and item.text == "评审团队")
    title = s.shapes.title
    assert metadata.left == title.left
    assert not any(item.name == "Panel navigation" for item in s.shapes)
`,
    );
    assert.equal(result.status, 0, result.stderr);
  },
);
