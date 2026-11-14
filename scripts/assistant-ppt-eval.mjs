#!/usr/bin/env node
// Offline experiment preparation and fixture smoke, never an implicit paid run.
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  writeFile,
} from "node:fs/promises";
import { arch, platform, release } from "node:os";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const fixture = new URL("./fixtures/ppt-benchmark.json", import.meta.url);
const catalogPath = path.join(
  root,
  "assistant-runtime/skills/ofox-office-pptx/resources/catalog.json",
);
const representatives = [
  "blue-professional",
  "cartesian",
  "course-module",
  "product-launch",
  "architectural-plan",
];
const conditions = ["A", "B", "C", "D"];
const preflight = {
  status: "blocked",
  reason:
    "Condition-specific runtime isolation is not implemented. Current shared guidelines/skill already include public-style selection and per-slide planning, so ordinary chats cannot isolate A/B/C/D.",
  requiredEvidence: [
    "A: pre-change complete Desktop runtime/skill/tool-guidelines snapshot with existing previews/checks",
    "B: A plus pinned public resources/adapters only, without new slide planning/review",
    "C: B plus independent slide plan only",
    "D: C plus final visual review; record reviewer/model/fixes separately",
  ],
};
const hash = (value) => createHash("sha256").update(value).digest("hex");
const canonical = (value) =>
  JSON.stringify(value, (_, item) =>
    item && !Array.isArray(item) && typeof item === "object"
      ? Object.fromEntries(
          Object.entries(item).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0)),
        )
      : item,
  );
export const benchmarkHash = (benchmark) => hash(canonical(benchmark));
export async function loadBenchmark(file = fixture) {
  const benchmark = JSON.parse(await readFile(file, "utf8"));
  assert.equal(benchmark.provenance.kind, "self-authored-synthetic");
  assert.equal(benchmark.slides.length, 6);
  return benchmark;
}

export function buildExperiment(benchmark, { styles, repeats = 3, context }) {
  assert.ok(
    Number.isInteger(repeats) && repeats > 0 && repeats <= 30,
    "repeats must be 1–30",
  );
  assert.ok(styles.length > 0, "styles are required");
  const contentHash = benchmarkHash(benchmark);
  const runs = [];
  for (const style of styles) {
    assert.match(style.id, /^[a-z0-9][a-z0-9-]*$/, "invalid style id");
    for (let repeat = 1; repeat <= repeats; repeat++)
      for (const condition of conditions) {
        runs.push({
          id: `${style.id}-r${repeat}-${condition}`,
          condition,
          repeat,
          pairedStyle: style.id,
          frozen: {
            contentHash,
            ...context,
            style:
              condition === "A"
                ? { id: "desktop-current", version: context.runtimeVersion }
                : { id: style.id, version: style.version },
            conditionPolicy: {
              condition,
              publicStyleResources: condition !== "A",
              newSlidePlan: ["C", "D"].includes(condition),
              additionalVisualFixLimit: condition === "D" ? 2 : 0,
            },
          },
          policy: {
            preserveExistingPreviewAndChecks: true,
            publicStyleResources: condition !== "A",
            newSlidePlan: ["C", "D"].includes(condition),
            additionalVisualFixLimit: condition === "D" ? 2 : 0,
          },
          status: "pending",
          comparability: "blocked",
          preflight: structuredClone(preflight),
          artifacts: [],
          metrics: {
            tokens: null,
            seconds: null,
            calls: null,
            retries: null,
            cache: "unrecorded",
          },
          cost: { actual: null, estimate: null },
          manualReview: {
            status: "pending",
            scores: null,
            powerpointEditability: "pending",
          },
        });
      }
  }
  return {
    schemaVersion: 1,
    benchmarkId: benchmark.id,
    benchmarkVersion: benchmark.version,
    contentHash,
    createdAt: new Date().toISOString(),
    realModelExecuted: false,
    styles,
    runs,
    fixtureSmoke: [],
  };
}

function validateCost(cost = {}) {
  for (const [kind, value] of Object.entries(cost)) {
    assert.ok(["actual", "estimate"].includes(kind), "unknown cost kind");
    if (value === null) continue;
    assert.ok(
      Number.isFinite(value.amount) &&
        value.amount >= 0 &&
        typeof value.currency === "string",
      "invalid cost amount/currency",
    );
    if (kind === "actual")
      assert.ok(
        value.billingEvidence && value.billedAt,
        "actual cost requires billing evidence and billedAt",
      );
    else
      assert.ok(
        value.priceVersion && value.pricedAt,
        "estimate requires price version and date",
      );
  }
}

export function mergeReceipts(experiment, receipts) {
  assert.ok(Array.isArray(receipts), "receipts must be a JSON array");
  const result = structuredClone(experiment);
  const seen = new Set();
  for (const receipt of receipts) {
    assert.ok(!seen.has(receipt.runId), "duplicate receipt");
    seen.add(receipt.runId);
    const run = result.runs.find((item) => item.id === receipt.runId);
    assert.ok(run, "unknown run id");
    assert.equal(
      canonical(receipt.frozen),
      canonical(run.frozen),
      "frozen inputs differ; do not pool confounded runs",
    );
    assert.ok(
      run.frozen.model !== "unconfigured" &&
        run.frozen.modelVersion !== "unconfigured",
      "configure the model and its version before importing real results",
    );
    assert.ok(
      ["completed", "failed"].includes(receipt.status),
      "receipt must be completed or failed",
    );
    assert.ok(
      receipt.status !== "completed" ||
        receipt.artifacts?.some(
          (item) => typeof item === "string" && item.endsWith(".pptx"),
        ),
      "completed PPTX run requires an artifact reference",
    );
    if (receipt.metrics?.tokens)
      for (const key of ["input", "output"])
        assert.ok(
          Number.isFinite(receipt.metrics.tokens[key]) &&
            receipt.metrics.tokens[key] >= 0,
          "invalid token count",
        );
    validateCost(receipt.cost);
    const review = receipt.manualReview;
    if (review?.status === "pending")
      assert.ok(
        review.scores == null,
        "pending manual review cannot contain scores",
      );
    if (review && review.status !== "pending") {
      assert.ok(
        review.status === "reviewed" &&
          review.reviewer &&
          review.evidence?.length,
        "manual review requires reviewer and evidence",
      );
      if (review.scores)
        for (const score of Object.values(review.scores))
          assert.ok(
            Number.isFinite(score) && score >= 1 && score <= 5,
            "manual scores must be 1–5",
          );
    }
    Object.assign(run, {
      status: receipt.status,
      artifacts: receipt.artifacts ?? [],
      metrics: { ...run.metrics, ...receipt.metrics },
      cost: { ...run.cost, ...receipt.cost },
      manualReview: review ?? run.manualReview,
      failure: receipt.failure ?? null,
    });
  }
  result.realModelExecuted = result.runs.some(
    (run) => run.status !== "pending",
  );
  return result;
}

const xmlText = (xml) =>
  [...xml.matchAll(/<(?:a|m):t(?:\s[^>]*)?>([^<]*)<\/(?:a|m):t>/g)]
    .map((item) =>
      item[1]
        .replaceAll("&amp;", "&")
        .replaceAll("&lt;", "<")
        .replaceAll("&gt;", ">"),
    )
    .join(" ");
export function inspectPptx(entries, benchmark) {
  const slides = [...entries.keys()]
    .filter((name) => /^ppt\/slides\/slide\d+\.xml$/.test(name))
    .sort(
      (a, b) =>
        Number(a.match(/(\d+)\.xml/)[1]) - Number(b.match(/(\d+)\.xml/)[1]),
    );
  const xml = slides.map((name) => entries.get(name).toString());
  const missingFacts = benchmark.slides.flatMap((slide, index) =>
    slide.requiredText
      .filter(
        (fact) =>
          !xmlText(xml[index] ?? "")
            .replaceAll(/\s/g, "")
            .includes(fact.replaceAll(/\s/g, "")),
      )
      .map((fact) => ({ slide: index + 1, fact })),
  );
  const size = entries
    .get("ppt/presentation.xml")
    ?.toString()
    .match(/<p:sldSz[^>]*cx="(\d+)"[^>]*cy="(\d+)"/);
  return {
    slideCount: slides.length,
    aspectRatio16x9: Boolean(
      size && Math.abs(Number(size[1]) / Number(size[2]) - 16 / 9) < 0.001,
    ),
    missingFacts,
    nativeChart:
      xml.some((item) => /<c:chart\b/.test(item)) &&
      [...entries.keys()].some((name) =>
        /^ppt\/embeddings\/.*\.xlsx$/.test(name),
      ),
    nativeEquation: xml.some((item) => /<m:oMath[\s>]/.test(item)),
    powerpointEditability: "pending",
    visualReview: "pending",
    scope:
      "OOXML structural presence only; edit-save-reopen and visual inspection are pending",
  };
}

function promptFor(benchmark, run) {
  const lines = [
    "# 冻结 PPT 样张任务",
    "BLOCKED PREFLIGHT：当前共享系统/工具指导已经包含公开风格选择和逐页计划。以下只是待接入隔离执行器的任务稿；直接在普通会话执行A/B不能建立有效因果对照，结果只可记为comparability=blocked。",
    `条件 ${run.condition}；runId: ${run.id}`,
    `内容 SHA-256: ${run.frozen.contentHash}`,
    "固定中文、16:9、6 页、平衡阅读密度；正文至少 12pt，数据标签至少 10pt。保持现有生成路径、预览与检查。不得删事实、改数字、增减页数；不得把整页截图当原生可编辑 PPTX。",
    "所有数据都是合成演示，未比较方案 A/B 性能。",
    "",
    JSON.stringify(benchmark, null, 2),
    "",
    "请生成可编辑 PPTX。第4页保留全部数据并提供原生 Chart/嵌入工作簿；第5页包含原生数学对象。若当前工具不支持，报告阻断项，不用截图冒充通过。",
  ];
  if (run.condition !== "A")
    lines.push(
      `仅使用冻结公开风格 ${run.frozen.style.id}@${run.frozen.style.version} 及必要适配，保留该风格的构图、字阶与视觉特征。`,
    );
  if (run.policy.newSlidePlan)
    lines.push(
      "创作前逐页制定独立设计计划：本页论点、表达关系、布局、组件、视觉主次、必需事实/数据、原生可编辑对象、文字密度与字体预算。计划依据本任务和公开资源，自行编写。将计划作为实验附属证据保存。",
    );
  if (run.condition === "D")
    lines.push(
      "在现有预览检查之上，对当前最终导出文件逐页做成品视觉复核：文字完整、论证、布局特征、CJK/对比/溢出、图表单位与公式。额外修复最多2轮。保存成品截图、缺陷/修复记录、审稿模型版本、调用/token、时间与费用；视觉复核不能替代PowerPoint编辑保存重开。",
    );
  lines.push(
    "保存冻结输入、完整失败与重试、初稿/最终耗时、生成/渲染/审稿调用、输入输出及缓存token。账单实际费用与按指定价格版本的估算分开；未知填null。人工盲审评分和目标PowerPoint编辑测试未做前保持pending。A组不得取消已有预览以人为弱化基线。",
  );
  return lines.join("\n\n") + "\n";
}

// This code is authored from our synthetic benchmark, never a model result.
export function fixtureCode(benchmark, style = "desktop-current") {
  const data = JSON.stringify(JSON.stringify(benchmark));
  const styled =
    style === "desktop-current"
      ? ""
      : `, style=${JSON.stringify(style)}, language="zh"`;
  return `import json
from pptx.chart.data import CategoryChartData
from pptx.enum.chart import XL_CHART_TYPE
from pptx.util import Inches, Pt
b = json.loads(${data})
style = ${JSON.stringify(style)}
prs = new_deck(title=b["slides"][0]["title"], footer="合成演示 · 非性能测量"${styled})
for i, page in enumerate(b["slides"]):
    if i == 0:
        cover_slide(prs, page["title"], subtitle="\\n".join(page["body"]), kicker="统一内容样张", meta="自编合成内容 · 6页 · zh-CN")
        continue
    body = page["body"]
    if style == "course-module":
        s = lesson_slide(prs, page["title"], objective=body[0])
        region = slide_regions(s)["main"]
        body = body[1:]
    elif style == "architectural-plan":
        s = panel_slide(prs, page["title"])
        regions = slide_regions(s); region = regions["main"]; panel = regions["panel"]
        text(s, body[0], panel["x"], panel["y"], panel["w"], panel["h"], size=18)
        body = body[1:]
    else:
        s = content_slide(prs, page["title"])
        region = {"x": .8, "y": 1.9, "w": 11.7, "h": 4.8}
    x, y, width, height = [region[key] for key in ["x", "y", "w", "h"]]
    if i == 3:
        rows = [["批次", "输入条目", "有效条目", "索引耗时（秒）"]] + [[r["batch"], str(r["input"]), str(r["valid"]), str(r["seconds"])] for r in b["data"]]
        two_regions = style in ["course-module", "architectural-plan"]
        table_width = width if two_regions else 6
        table = s.shapes.add_table(4, 4, Inches(x), Inches(y), Inches(table_width), Inches(1.8 if two_regions else 2.5)).table
        for ri, row in enumerate(rows):
            for ci, value in enumerate(row):
                table.cell(ri, ci).text = value
                for p in table.cell(ri, ci).text_frame.paragraphs:
                    p.font.size = Pt(13)
        cd = CategoryChartData(); cd.categories = [r["batch"] for r in b["data"]]
        cd.add_series("索引耗时（秒）", [r["seconds"] for r in b["data"]])
        chart_x, chart_y = (x, y + 2.05) if two_regions else (7, y)
        chart_w, chart_h = (width, 2.55) if two_regions else (5.5, 3.5)
        s.shapes.add_chart(XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(chart_x), Inches(chart_y), Inches(chart_w), Inches(chart_h), cd)
        if not two_regions: text(s, page["body"][0], x, 5.6, width, .7, size=14)
    elif i == 4:
        text(s, "\\n\\n".join(body), x, y, width, 1.9, size=22)
        equation(s, x, y + 2.35, width, .85, {"fraction": ["180", "200"]}, size=24)
    elif i == 1 and style not in ["course-module", "architectural-plan"]:
        relation(s, ["采集", "去重", "索引", "检索"], x, y, width, 1.1)
        text(s, page["body"][1], x, y + 1.8, width, 1, size=20)
    else:
        text(s, "\\n\\n".join(body), x, y, width, height, size=20)
`;
}

async function renderSmoke(experiment, benchmark, out) {
  assert.equal(
    platform(),
    "darwin",
    "fixture smoke currently requires the staged macOS Python engine",
  );
  // Reuse the exact staged runner and ZIP reader used by Python engine tests.
  const { run, zipEntries, officecli } = await import(
    "../assistant-runtime/test/fixtures/python-engine.mjs"
  );
  const results = [];
  for (const style of [
    "desktop-current",
    ...experiment.styles.map((item) => item.id),
  ]) {
    const cleanups = [];
    const directory = path.join(out, "fixture-smoke", style);
    await mkdir(directory, { recursive: true });
    const started = performance.now();
    const record = {
      style,
      kind: "fixed-code-fixture-smoke",
      modelExecuted: false,
      status: "pending",
      cost: { actual: null, estimate: null },
      tokens: null,
      localModelCalls: 0,
      manualReview: "pending",
      powerpointEditability: "pending",
    };
    record.codeHashes = await smokeHashes();
    try {
      const result = await run(
        { after: (cleanup) => cleanups.push(cleanup) },
        fixtureCode(benchmark, style),
      );
      record.generationSeconds = (performance.now() - started) / 1000;
      assert.equal(
        result.status,
        0,
        `Python fixture failed: ${result.stderr.slice(-2000)}`,
      );
      await Promise.all([
        copyFile(result.code, path.join(directory, "source.py")),
        copyFile(result.out, path.join(directory, "deck.pptx")),
        copyFile(result.lint, path.join(directory, "lint.json")),
      ]);
      record.structure = inspectPptx(await zipEntries(result.out), benchmark);
      record.lint = JSON.parse(await readFile(result.lint, "utf8"));
      record.artifact = path.relative(out, path.join(directory, "deck.pptx"));
      record.artifactSha256 = hash(await readFile(result.out));
      record.fixtureSourceSha256 = hash(await readFile(result.code));
      // Like python-deck.test.mjs: private HOME, no resident/cache or updater.
      const home = path.join(directory, "home");
      await mkdir(home, { recursive: true });
      const env = {
        PATH: process.env.PATH,
        HOME: home,
        OFFICECLI_NO_AUTO_RESIDENT: "1",
        OFFICECLI_SKIP_UPDATE: "1",
      };
      try {
        record.officecliValidation = JSON.parse(
          execFileSync(officecli, ["validate", result.out, "--json"], {
            env,
            encoding: "utf8",
            timeout: 60000,
            stdio: ["ignore", "pipe", "pipe"],
          }),
        );
      } catch (error) {
        try {
          record.officecliValidation = JSON.parse(error.stdout);
        } catch {
          record.officecliValidation = {
            status: "failed",
            note: "OfficeCLI validation could not be read",
          };
        }
      }
      try {
        record.officecli = JSON.parse(
          execFileSync(officecli, ["view", result.out, "outline", "--json"], {
            env,
            encoding: "utf8",
            timeout: 60000,
            stdio: ["ignore", "pipe", "pipe"],
          }),
        ).data;
      } catch {
        record.officecli = {
          status: "failed",
          note: "OfficeCLI outline read failed; does not establish editability",
        };
      }
      const renderStarted = performance.now();
      record.previews = [];
      try {
        const { thumbnailFromSvg } = await import(
          "../assistant-runtime/src/thumbnails.mjs"
        );
        const require = createRequire(
          new URL("../assistant-runtime/package.json", import.meta.url),
        );
        const { createCanvas, loadImage } = require("@napi-rs/canvas");
        const sheet = createCanvas(1200, 450);
        const ctx = sheet.getContext("2d");
        ctx.fillStyle = "#d8d8d8";
        ctx.fillRect(0, 0, 1200, 450);
        for (let page = 1; page <= 6; page++) {
          const svg = JSON.parse(
            execFileSync(
              officecli,
              [
                "view",
                result.out,
                "svg",
                "--start",
                String(page),
                "--end",
                String(page),
                "--json",
              ],
              {
                env,
                encoding: "utf8",
                timeout: 60000,
                stdio: ["ignore", "pipe", "pipe"],
              },
            ),
          ).data;
          assert.equal(typeof svg, "string", "OfficeCLI did not return SVG");
          await writeFile(path.join(directory, `slide-${page}.svg`), svg);
          const thumbnail = await thumbnailFromSvg(Buffer.from(svg));
          const png = Buffer.from(thumbnail.data, "base64");
          await writeFile(path.join(directory, `slide-${page}.png`), png);
          ctx.drawImage(
            await loadImage(png),
            ((page - 1) % 3) * 400,
            Math.floor((page - 1) / 3) * 225,
            400,
            225,
          );
          record.previews.push(
            path.relative(out, path.join(directory, `slide-${page}.png`)),
          );
        }
        await writeFile(
          path.join(directory, "contact-sheet.png"),
          sheet.toBuffer("image/png"),
        );
      } catch (error) {
        record.previewFailure = error.message;
      }
      record.renderSeconds = (performance.now() - renderStarted) / 1000;
      record.status =
        record.structure.slideCount === 6 &&
        record.structure.aspectRatio16x9 &&
        record.structure.missingFacts.length === 0 &&
        record.previews.length === 6
          ? "generated"
          : "failed";
      assert.equal(
        canonical(record.codeHashes),
        canonical(await smokeHashes()),
        "engine/catalog changed during fixture execution; rerun against a stable snapshot",
      );
    } catch (error) {
      record.status = "failed";
      record.failure = error.message;
    } finally {
      record.seconds = (performance.now() - started) / 1000;
      for (const cleanup of cleanups) await cleanup();
    }
    results.push(record);
    console.log(`fixture ${style}: ${record.status}; model not run`);
  }
  return results;
}

async function hashesFor(paths) {
  const result = {};
  for (const name of paths) {
    try {
      result[name] = hash(await readFile(path.join(root, name)));
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      result[name] = null;
    }
  }
  return result;
}

async function smokeHashes() {
  return hashesFor([
    "assistant-runtime/src/python/deck.py",
    "assistant-runtime/src/python/runner.mjs",
    "assistant-runtime/src/python/template.pptx",
    "assistant-runtime/resources/engines/python/assets.json",
    "assistant-runtime/skills/ofox-office-pptx/resources/catalog.json",
    "assistant-runtime/src/thumbnails.mjs",
    "assistant-runtime/resources/engines/officecli/officecli",
  ]);
}

async function generationSnapshot() {
  const files = [];
  async function walk(directory) {
    let entries;
    try {
      entries = await readdir(path.join(root, directory), {
        withFileTypes: true,
      });
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      return;
    }
    for (const entry of entries) {
      const name = path.join(directory, entry.name);
      if (entry.isDirectory()) await walk(name);
      else if (entry.isFile()) files.push(name);
    }
  }
  for (const directory of [
    "assistant-runtime/src",
    "assistant-runtime/skills/ofox-office-pptx",
    "src-tauri/src/services/assistant",
  ])
    await walk(directory);
  files.push(
    "assistant-runtime/package.json",
    "assistant-runtime/resources/engines/python/assets.json",
  );
  return hashesFor(files.sort());
}

function parseArgs(argv) {
  const options = {
    styles: representatives,
    repeats: 3,
    model: "unconfigured",
    modelVersion: "unconfigured",
    engine: "python",
    render: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const flag = argv[i];
    if (flag === "--help") return { help: true };
    if (flag === "--render") {
      options.render = true;
      continue;
    }
    assert.ok(
      [
        "--out",
        "--styles",
        "--repeats",
        "--model",
        "--model-version",
        "--engine",
        "--receipts",
        "--parameters",
      ].includes(flag),
      `unknown argument: ${flag}`,
    );
    assert.ok(
      argv[i + 1] && !argv[i + 1].startsWith("--"),
      `missing value for ${flag}`,
    );
    const value = argv[++i];
    if (flag === "--styles")
      options.styles = value
        .split(",")
        .map((item) => item.trim())
        .filter(Boolean);
    else if (flag === "--repeats") options.repeats = Number(value);
    else
      options[
        flag.slice(2).replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())
      ] = value;
  }
  assert.ok(
    ["python", "officecli", "node"].includes(options.engine),
    "invalid engine",
  );
  assert.ok(
    !options.render || options.engine === "python",
    "--render smoke uses python only; other routes need separate experiments",
  );
  assert.equal(
    new Set(options.styles).size,
    options.styles.length,
    "duplicate styles",
  );
  return options;
}

async function stylesFor(ids) {
  let catalog;
  try {
    catalog = JSON.parse(await readFile(catalogPath, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  return ids.map((id) => {
    assert.ok(
      representatives.includes(id),
      `unknown representative style: ${id}`,
    );
    const style = catalog?.styles.find((item) => item.id === id);
    const source = catalog?.sources.find((item) => item.id === style?.sourceId);
    return {
      id,
      version: source
        ? `${source.revision}:${hash(canonical(style))}`
        : "unrecorded",
      sourceRevision: source?.revision ?? null,
      status: style?.status ?? "pending-resource-intake",
      adapted: style?.adapted ?? false,
      verified: style?.verified ?? false,
      enabled: style?.enabled ?? false,
      licenseScope:
        "per-resource catalog; fonts and downstream use require separate review",
    };
  });
}

function renderReport(experiment) {
  const out = [
    "# PPT 同内容实验记录",
    "",
    `内容 SHA-256：${experiment.contentHash}`,
    "",
    `真实模型执行：${experiment.realModelExecuted ? "存在已导入结果（见收据，尚不等于质量通过）" : "未执行；A/B/C/D 均为冻结计划"}。没有质量提升分数。enabled 仅表示可选实验资源，verified=false 不能称为已验收。`,
    "",
    "对照可比性：BLOCKED。尚未实现A/B/C/D实际运行时隔离。当前共享guidelines/skill已包含选风格和逐页计划，普通会话即使使用不同prompt也不能成为有效A/B对照。导入原始收据不会解除此阻断；不得计算或宣称B−A、C−B、D−C收益。",
    "",
    "| 条件 | 完成 | 失败 | 待执行 | 人工复核待做 |",
    "| --- | ---: | ---: | ---: | ---: |",
  ];
  for (const condition of conditions) {
    const runs = experiment.runs.filter((run) => run.condition === condition);
    out.push(
      `| ${condition} | ${runs.filter((r) => r.status === "completed").length} | ${runs.filter((r) => r.status === "failed").length} | ${runs.filter((r) => r.status === "pending").length} | ${runs.filter((r) => r.manualReview.status === "pending").length} |`,
    );
  }
  out.push(
    "",
    "A保留现有预览/检查；B只增加公开资源；C再增加逐页计划；D再增加额外成品视觉复核，最多2轮。配对A会重复生成，不当作独立风格；不得跨引擎、平台或不同冻结输入合并收益。",
    "",
    "固定代码样张仅验证离线工具链与结构，不属于模型四条件对照。结构存在不等于PowerPoint编辑保存重开或视觉通过。",
    "",
    "| 固定样张 | 状态 | 页数 | 缺少事实 | 原生Chart+工作簿 | OMML结构 | 秒 |",
    "| --- | --- | ---: | ---: | --- | --- | ---: |",
  );
  for (const row of experiment.fixtureSmoke)
    out.push(
      `| ${row.style} | ${row.status} | ${row.structure?.slideCount ?? "-"} | ${row.structure?.missingFacts.length ?? "-"} | ${row.structure?.nativeChart ?? "-"} | ${row.structure?.nativeEquation ?? "-"} | ${row.seconds.toFixed(2)} |`,
    );
  out.push(
    "",
    "token/实际费用/估算未取得时均为null。fixture未调用模型，localModelCalls=0；本机生成耗时单列。真实收据的estimate必须带priceVersion/pricedAt；actual必须带billingEvidence/billedAt。人工评分、macOS/Windows目标PowerPoint编辑复核仍需独立证据。",
    "",
  );
  return out.join("\n");
}

export async function main(argv) {
  const options = parseArgs(argv);
  if (options.help) {
    console.log(
      "Usage: node scripts/assistant-ppt-eval.mjs [--out DIR] [--styles id,...] [--repeats 3] [--model ID --model-version VERSION --parameters FILE] [--engine python|officecli|node] [--receipts FILE] [--render]\nDefault prepares frozen prompts and pending runs offline. --render additionally generates fixed-code Python smoke fixtures, never real A/B/C/D results. Receipts are a JSON array with runId,frozen,status,artifacts,metrics,cost and optional evidence-backed manualReview. --parameters freezes the exact runtime generation parameters from a JSON object (never put credentials there).",
    );
    return 0;
  }
  const benchmark = await loadBenchmark();
  const out = path.resolve(
    options.out ??
      path.join(
        root,
        ".real-eval",
        `ppt-${new Date().toISOString().replaceAll(":", "-")}`,
      ),
  );
  await mkdir(path.join(out, "prompts"), { recursive: true });
  let runtimeVersion = "unrecorded";
  try {
    runtimeVersion = execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    }).trim();
  } catch {}
  let workingTreeDirty = null;
  try {
    workingTreeDirty =
      execFileSync("git", ["status", "--porcelain"], {
        cwd: root,
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }).trim().length > 0;
  } catch {}
  const parameters = options.parameters
    ? JSON.parse(await readFile(options.parameters, "utf8"))
    : {
        temperature: null,
        seed: null,
        note: "record exact runtime generation parameters before a real comparison",
      };
  assert.ok(
    parameters && !Array.isArray(parameters) && typeof parameters === "object",
    "parameters must be a JSON object",
  );
  const generationFiles = await generationSnapshot();
  const engineFiles = await smokeHashes();
  let experiment = buildExperiment(benchmark, {
    styles: await stylesFor(options.styles),
    repeats: options.repeats,
    context: {
      model: options.model,
      modelVersion: options.modelVersion,
      engine: options.engine,
      engineVersion:
        options.engine === "python"
          ? `sha256:${benchmarkHash(engineFiles)}`
          : "unrecorded",
      engineFiles,
      generationSnapshotHash: benchmarkHash(generationFiles),
      runtimeVersion,
      workingTreeDirty,
      evaluatorVersion: `sha256:${hash(await readFile(fileURLToPath(import.meta.url)))}`,
      platform: `${platform()}/${arch()}/${release()}`,
      parameters,
    },
  });
  experiment.generationFiles = generationFiles;
  if (options.receipts)
    experiment = mergeReceipts(
      experiment,
      JSON.parse(await readFile(options.receipts, "utf8")),
    );
  await writeFile(
    path.join(out, "benchmark.json"),
    JSON.stringify(benchmark, null, 2) + "\n",
  );
  await Promise.all(
    experiment.runs.map((run) =>
      writeFile(
        path.join(out, "prompts", `${run.id}.md`),
        promptFor(benchmark, run),
      ),
    ),
  );
  if (options.render)
    experiment.fixtureSmoke = await renderSmoke(experiment, benchmark, out);
  await writeFile(
    path.join(out, "experiment.json"),
    JSON.stringify(experiment, null, 2) + "\n",
  );
  await writeFile(path.join(out, "report.md"), renderReport(experiment));
  console.log(
    `PPT experiment: ${out}; real model executed: ${experiment.realModelExecuted}`,
  );
  return experiment.fixtureSmoke.some((row) => row.status === "failed") ? 1 : 0;
}

if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
)
  main(process.argv.slice(2))
    .then((code) => {
      process.exitCode = code;
    })
    .catch((error) => {
      console.error(error.message);
      process.exitCode = 2;
    });
