import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import test from "node:test";
import { promisify } from "node:util";
import {
  assets,
  options,
  run,
  runner,
  scratch,
  staged,
  zipEntries,
} from "./fixtures/python-engine.mjs";

const chart = `native_chart(s, "bar", ["甲", "乙"], [{"name":"示意", "values":[30,45]}], 0.72, 1.70, 9.68, 3.60, title="自编示意")`;
const lateText = `text(s, "说明正文", 0.72, 5.05, 9.68, 0.60)`;

test(
  "finished chart frames diagnose overlapping late text for both native_chart and direct add_chart",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "给定经营图表示意")
${chart}
${lateText}
shape(s, MSO_SHAPE.RECTANGLE, 0.72, 1.7, 9.68, 3.6, fill=T.tint)
text(s, "", 1, 2, 2, 1)
assert s.shapes[-3].top == Inches(5.05)
s = content_slide(prs, "直接原生图表")
cd = CategoryChartData(); cd.categories = ["甲", "乙"]; cd.add_series("示意", [30,45])
s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(0.72), Inches(1.70), Inches(9.68), Inches(3.60), cd)
${lateText}
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.equal(lint.length, 2, JSON.stringify(lint));
    for (let index = 0; index < 2; index++)
      assert.match(
        lint[index],
        new RegExp(
          `slide ${index + 1}: native chart/text overlap.*shape \\d+.*shape \\d+.*0\\.25 in`,
        ),
      );
    const entries = await zipEntries(result.out);
    for (let page = 1; page <= 2; page++) {
      const xml = entries.get(`ppt/slides/slide${page}.xml`).toString();
      assert.match(xml, /<c:chart /);
      assert.match(xml, /y="4617720"/); // Original late text top: 5.05 in.
    }
  },
);

test(
  "separated text and ordinary card containment do not trigger chart-frame warnings",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "分离的图表和说明")
${chart}
text(s, "分离正文", 0.72, 5.45, 9.68, 0.6)
card(s, 10.65, 1.7, 2, 2, "卡片", body="卡片内部正文")
s = content_slide(prs, "只有卡片")
card(s, 0.8, 2, 5, 2, "标题", body="卡片正文不与原生图表相交")
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.ok(
      lint.every((line) => !/chart\/text|layout check/.test(line)),
      JSON.stringify(lint),
    );
  },
);

test(
  "source editing checks actual target pages only and preserves an untargeted preexisting overlap",
  options,
  async (t) => {
    const source = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "保留页")
${chart}
${lateText}
s = content_slide(prs, "目标页")
${chart}
`,
    );
    assert.equal(source.status, 0, source.stderr);
    const work = await scratch(t);
    const code = path.join(work, "edit.py"),
      meta = path.join(work, "source-meta.json"),
      out = path.join(work, "edited.pptx"),
      lintFile = path.join(work, "lint.json");
    const original = await readFile(source.out);
    await writeFile(
      code,
      `prs=source_deck(); s=target_slides(prs)[0]\n${lateText}\n`,
    );
    await writeFile(
      meta,
      JSON.stringify({
        version: createHash("sha256").update(original).digest("hex"),
        totalPages: 2,
        pageNumbers: [2],
      }),
    );
    await promisify(execFile)(
      staged,
      [
        runner,
        "--assets",
        assets,
        "--code",
        code,
        "--out",
        out,
        "--lint",
        lintFile,
        "--source-deck",
        source.out,
        "--source-meta",
        meta,
      ],
      { env: { PATH: process.env.PATH }, timeout: 150_000 },
    );
    const lint = JSON.parse(await readFile(lintFile, "utf8"));
    assert.equal(lint.length, 1, JSON.stringify(lint));
    assert.match(lint[0], /slide 2: native chart\/text overlap/);
    assert.deepEqual(await readFile(source.out), original);
    const [before, after] = await Promise.all([
      zipEntries(source.out),
      zipEntries(out),
    ]);
    assert.deepEqual(
      before.get("ppt/slides/slide1.xml"),
      after.get("ppt/slides/slide1.xml"),
    );
  },
);

test(
  "dense overlapping chart annotations yield bounded diagnostics without moving native objects",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "诊断上限")
${chart}
for i in range(60):
    text(s, "说明" + str(i), 0.72, 5.05, 9.68, 0.6)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.equal(lint.length, 41);
    assert.ok(
      lint
        .slice(0, 40)
        .every((line) => /native chart\/text overlap/.test(line)),
    );
    assert.match(lint[40], /layout check.*limit/);
  },
);
