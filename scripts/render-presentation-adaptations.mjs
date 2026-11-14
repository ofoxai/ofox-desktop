// Builds, for every adapter, a native PPTX that reproduces each mapped page
// kind of its source deck with Ofox's own sample content, then publishes one
// JPEG per page. Offline: the staged Node, Python engine and OfficeCLI only.
// These pages are evidence of the native adaptation, never the upstream deck.
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, readdir, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { loadAdapters, RESOURCES } from "./presentation-adapters.mjs";

const root = path.resolve(fileURLToPath(new URL("../", import.meta.url)));
const runtime = path.join(root, "assistant-runtime");
const base = path.join(root, RESOURCES);
const node = path.join(runtime, "resources/node/node");
const office = path.join(runtime, "resources/engines/officecli/officecli");
const assets = path.join(runtime, "resources/engines/python");
const work = path.join(
  root,
  ".real-eval/ppt-library-completion-2026-11-11/adaptations",
);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

// Self-written sample content: a fictional community reading club. Every
// figure is a made-up sample value and is labelled as such on the page.
const PRELUDE = String.raw`
def R(s):
    r = slide_regions(s)["main"]
    return r["x"], r["y"], r["w"], r["h"]

def a_cover(prs):
    cover_slide(prs, "社区读书会年度计划", subtitle="自行编写的示例内容，仅用于展示版式", kicker="示例样张", meta="示例组织 · 2026")

def a_chapter(prs):
    section_slide(prs, "为什么要办读书会", number="01", subtitle="从三个观察说起")

def a_agenda(prs):
    s = content_slide(prs, "今天的四个议题"); x, y, w, h = R(s)
    steps(s, [{"title": "现状", "body": "成员怎么读"}, {"title": "目标", "body": "想改变什么"},
              {"title": "做法", "body": "每月怎么办"}, {"title": "安排", "body": "谁来负责"}], x, y + 0.5, w, 1.9, size=16)
    callout(s, y + 3.3, "每个议题留出十分钟讨论。", x=x, w=w)

def a_statement(prs):
    statement_slide(prs, "读得慢一点，才能想得深一点。", kicker="核心主张")

def a_quote(prs):
    statement_slide(prs, "“每月一本书，比一年十二本更容易坚持。”", attribution="示例成员 · 自行编写的引语")

def a_list(prs):
    s = content_slide(prs, "五件要做的事", subtitle="按先后顺序排列"); x, y, w, h = R(s)
    bullets(s, ["每月选定一本共读书目", "提前一周发出导读问题", "线下讨论控制在九十分钟", "会后整理一页讨论纪要", "每季度回顾一次参与情况"],
            x, y + 0.2, w * 0.7, 3.6, size=18, gap=10)

def a_split_image(prs):
    s = content_slide(prs, "讨论现场是什么样子"); x, y, w, h = R(s)
    bullets(s, ["围坐成一圈，不设讲台", "每人先说一句读后感", "主持人只负责追问"], x, y + 0.3, w * 0.46, 2.6, size=18, gap=10)
    slot = shape(s, MSO_SHAPE.RECTANGLE, x + w * 0.52, y + 0.1, w * 0.48, 4.2, fill=T.tint, line=T.line)
    slot.name = "Image region"
    text(s, "图片区域：按需生成或上传", x + w * 0.52, y + 0.1, w * 0.48, 4.2, size=14, color=T.muted, align="center", valign="middle")

def a_cards(prs):
    s = content_slide(prs, "三条共读约定"); x, y, w, h = R(s); cw = (w - 0.6) / 3
    for i, (title, body) in enumerate((("读完再来", "没读完也欢迎，但请先听别人说。"), ("只谈文本", "观点要能回到书里的某一页。"), ("轮流主持", "每位成员每年主持一次讨论。"))):
        card(s, x + i * (cw + 0.3), y + 0.5, cw, 2.2, title, body, highlight=i == 1, size=16)
    callout(s, y + 3.3, "约定写在每次导读的第一页。", x=x, w=w)

def a_stats(prs):
    s = content_slide(prs, "一年下来的三个数字", subtitle="均为示例数值"); x, y, w, h = R(s); cw = (w - 0.6) / 3
    for i, (value, label, note) in enumerate((("12", "共读书目", "每月一本"), ("72", "参与人次", "四季度示例值"), ("90", "分钟", "单次讨论时长"))):
        kpi(s, x + i * (cw + 0.3), y + 0.5, cw, 2.5, value, label, note=note, highlight=i == 0)
    callout(s, y + 3.5, "三个数字都来自示例数据。", x=x, w=w)

def a_comparison(prs):
    s = content_slide(prs, "调整前后对比"); x, y, w, h = R(s); cw = (w - 0.4) / 2
    card(s, x, y + 0.3, cw, 2.9, "调整前", points=["书目临时决定", "讨论没有主持人", "会后没有记录"], size=15)
    card(s, x + cw + 0.4, y + 0.3, cw, 2.9, "调整后", points=["提前一季度公布书目", "成员轮流主持", "一页纪要当天发出"], highlight=True, size=15)

def a_table(prs):
    s = content_slide(prs, "三种活动形式比较"); x, y, w, h = R(s)
    table(s, ["维度", "线下讨论", "线上共读", "作者交流"],
          [["准备时间", "一周", "三天", "一个月"], ["参与门槛", "中", "低", "中"], ["讨论深度", "高", "中", "高"], ["适合人数", "8–12 人", "不限", "20 人以内"]],
          x, y + 0.3, w, 3.0, size=14)

def a_chart_bar(prs):
    s = content_slide(prs, "各季度参与人次", subtitle="示例数据"); x, y, w, h = R(s)
    native_chart(s, "bar", ["一季度", "二季度", "三季度", "四季度"], [{"name": "参与人次（示例）", "values": [34, 48, 61, 72]}], x, y + 0.2, w, 4.3)

def a_chart_line(prs):
    s = content_slide(prs, "读完率的变化", subtitle="示例数据"); x, y, w, h = R(s)
    native_chart(s, "line", ["1月", "2月", "3月", "4月", "5月", "6月"],
                 [{"name": "读完率 %（示例）", "values": [42, 48, 55, 58, 64, 70]}, {"name": "到场率 %（示例）", "values": [60, 62, 66, 65, 71, 74]}], x, y + 0.2, w, 4.3)

def a_chart_pie(prs):
    s = content_slide(prs, "书目类型分布", subtitle="示例数据"); x, y, w, h = R(s)
    native_chart(s, "pie", ["小说", "历史", "科学", "随笔"], [{"name": "书目数（示例）", "values": [5, 3, 2, 2]}], x, y + 0.2, w * 0.62, 4.3)

def a_process(prs):
    s = content_slide(prs, "一次共读的四个步骤"); x, y, w, h = R(s)
    steps(s, [{"title": "选书", "body": "成员提名并投票"}, {"title": "导读", "body": "发出三个问题"}, {"title": "讨论", "body": "线下九十分钟"}, {"title": "纪要", "body": "整理成一页"}],
          x, y + 0.5, w, 1.9, highlight=2, size=16)
    callout(s, y + 3.3, "四步走完大约需要一个月。", x=x, w=w)

def a_timeline(prs):
    s = content_slide(prs, "全年安排"); x, y, w, h = R(s)
    timeline(s, [{"label": "一季度", "title": "试运行", "body": "三本短篇"}, {"label": "二季度", "title": "定规则", "body": "确定主持轮值"},
                 {"label": "三季度", "title": "扩成员", "body": "开放旁听"}, {"label": "四季度", "title": "做回顾", "body": "整理年度书单"}], x, y + 0.6, w, 2.9, size=15)

def a_cycle(prs):
    s = content_slide(prs, "每月循环一次"); x, y, w, h = R(s)
    relation(s, ["选书", "阅读", "讨论", "回顾"], x, y + 0.6, w, 1.5, kind="process", size=18)
    callout(s, y + 2.9, "回顾的结论进入下一轮选书。", x=x, w=w)

def a_hierarchy(prs):
    s = content_slide(prs, "从记录到共识"); x, y, w, h = R(s)
    pyramid(s, [{"title": "共识", "body": "写进年度书单"}, {"title": "观点", "body": "讨论中形成"}, {"title": "纪要", "body": "每次一页"},
                {"title": "笔记", "body": "每人自己的摘录"}], x + w * 0.1, y + 0.3, w * 0.8, 3.6)

def a_diagram(prs):
    s = content_slide(prs, "一本书怎么进入书单"); x, y, w, h = R(s)
    fork_join(s, ["成员提名", "主持人推荐", "上季度遗留"], x, y + 0.3, w, 3.3, source="书目来源", merge="投票后的书单")

def a_code(prs):
    s = content_slide(prs, "纪要模板"); x, y, w, h = R(s)
    code(s, "# 讨论纪要（示例模板）\nbook: 书名\nhost: 本次主持人\nquestions:\n  - 导读问题一\n  - 导读问题二\nnotes: 一页以内", x, y + 0.3, w, 2.9, size=14)

def a_team(prs):
    s = content_slide(prs, "谁来负责"); x, y, w, h = R(s); cw = (w - 0.9) / 4
    for i, (name, role) in enumerate((("示例成员甲", "选书与投票"), ("示例成员乙", "导读问题"), ("示例成员丙", "场地与签到"), ("示例成员丁", "讨论纪要"))):
        card(s, x + i * (cw + 0.3), y + 0.5, cw, 1.9, name, role, size=15)
    callout(s, y + 3.2, "每季度轮换一次分工。", x=x, w=w)

def a_lesson(prs):
    s = lesson_slide(prs, "如何提出一个好问题", "读完本页，你能写出一个开放式导读问题。"); m = slide_regions(s)["main"]
    bullets(s, ["问题要能回到文本", "避免只能回答是或否", "一次只问一件事"], m["x"], m["y"] + 0.2, m["w"], 2.6, size=18, gap=10)

def a_pricing(prs):
    s = content_slide(prs, "三种参与方式", subtitle="示例方案，非真实价格"); x, y, w, h = R(s); cw = (w - 0.6) / 3
    for i, (title, points) in enumerate((("旁听", ["不需要报名", "可以随时离场"]), ("成员", ["参与选书投票", "轮流主持"]), ("发起人", ["确定年度主题", "协调场地"]))):
        card(s, x + i * (cw + 0.3), y + 0.3, cw, 2.6, title, points=points, highlight=i == 1, size=14)

def a_closing(prs):
    closing_slide(prs, "谢谢", subtitle="欢迎提问与建议", meta="示例组织 · 联系方式占位")

ARCHETYPES = {"cover": a_cover, "chapter": a_chapter, "agenda": a_agenda, "statement": a_statement, "quote": a_quote, "list": a_list,
              "split-image": a_split_image, "cards": a_cards, "stats": a_stats, "comparison": a_comparison, "table": a_table,
              "chart-bar": a_chart_bar, "chart-line": a_chart_line, "chart-pie": a_chart_pie, "process": a_process, "timeline": a_timeline,
              "cycle": a_cycle, "hierarchy": a_hierarchy, "diagram": a_diagram, "code": a_code, "team": a_team, "lesson": a_lesson,
              "pricing": a_pricing, "closing": a_closing}
`;

const env = (home) => ({
  PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
  HOME: home,
  TMPDIR: home,
  OFFICECLI_NO_AUTO_RESIDENT: "1",
  OFFICECLI_SKIP_UPDATE: "1",
});
function run(label, executable, args, home) {
  const result = spawnSync(executable, args, {
    cwd: home,
    env: env(home),
    encoding: "utf8",
    timeout: 240_000,
    maxBuffer: 64 * 1024 * 1024,
  });
  assert.equal(
    result.status,
    0,
    `${label}: ${result.stderr || result.error?.message || result.stdout}`,
  );
  return result.stdout;
}

export async function renderAdaptations(ids, { publish = true } = {}) {
  const { thumbnailFromSvg } = await import(
    pathToFileURL(path.join(runtime, "src/thumbnails.mjs"))
  );
  const { normalizeOfficePatternImages, normalizeOfficeArrowheads, safeSvg } =
    await import(pathToFileURL(path.join(runtime, "src/document-preview.mjs")));
  const yauzl = (
    await import(
      pathToFileURL(path.join(runtime, "node_modules/yauzl/index.js"))
    )
  ).default;
  const adapters = (await loadAdapters(root)).filter(
    (adapter) => !ids || ids.includes(adapter.id),
  );
  const manifestFile = path.join(base, "adaptation-previews.json");
  let manifest = { schemaVersion: 1, styles: [] };
  try {
    manifest = JSON.parse(await readFile(manifestFile, "utf8"));
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  for (const adapter of adapters) {
    const folder = path.join(work, adapter.id);
    await rm(folder, { recursive: true, force: true });
    await mkdir(folder, { recursive: true, mode: 0o700 });
    const mapped = adapter.pageMapping.filter(
      (entry) => entry.status !== "unsupported",
    );
    const code =
      PRELUDE +
      `\nprs = new_deck(title="社区读书会年度计划（示例）", style=${JSON.stringify(adapter.id)}, language="zh")\n` +
      mapped
        .map((entry) => `ARCHETYPES[${JSON.stringify(entry.archetype)}](prs)`)
        .join("\n") +
      "\n";
    const files = {
      code: path.join(folder, "source.py"),
      out: path.join(folder, "adaptation.pptx"),
      lint: path.join(folder, "lint.json"),
    };
    await writeFile(files.code, code);
    run(
      "native deck " + adapter.id,
      node,
      [
        path.join(runtime, "src/python/runner.mjs"),
        "--assets",
        assets,
        "--code",
        files.code,
        "--out",
        files.out,
        "--lint",
        files.lint,
      ],
      folder,
    );
    const lint = JSON.parse(await readFile(files.lint, "utf8"));
    const validation = JSON.parse(
      run(
        "schema " + adapter.id,
        office,
        ["validate", files.out, "--json"],
        folder,
      ),
    );
    assert.equal(validation.success, true, adapter.id + " schema");
    // Native objects only: an adaptation must never embed a page picture.
    const names = await new Promise((resolve, reject) =>
      yauzl.open(files.out, { lazyEntries: true }, (error, zip) => {
        if (error) return reject(error);
        const found = [];
        zip.on("entry", (entry) => {
          found.push(entry.fileName);
          zip.readEntry();
        });
        zip.on("end", () => resolve(found));
        zip.on("error", reject);
        zip.readEntry();
      }),
    );
    const media = names.filter((name) => name.startsWith("ppt/media/"));
    assert.deepEqual(media, [], adapter.id + " embeds pictures");
    const pages = [];
    const outputs = new Map();
    for (const [index, entry] of mapped.entries()) {
      const svg = JSON.parse(
        run(
          `SVG ${adapter.id} ${index + 1}`,
          office,
          [
            "view",
            files.out,
            "svg",
            "--start",
            String(index + 1),
            "--end",
            String(index + 1),
            "--json",
          ],
          folder,
        ),
      ).data;
      assert.ok(typeof svg === "string" && safeSvg(svg), "unsafe SVG");
      const normalized = normalizeOfficeArrowheads(
        normalizeOfficePatternImages(svg),
      );
      const image = await thumbnailFromSvg(Buffer.from(normalized), {
        maxEdge: 1280,
        maxBytes: 200 * 1024,
      });
      assert.equal(image?.mimeType, "image/jpeg");
      const bytes = Buffer.from(image.data, "base64");
      const sha256 = digest(bytes);
      const relative = `adaptation-previews/${adapter.id}/page-${String(entry.page).padStart(2, "0")}-${sha256}.jpg`;
      outputs.set(relative, bytes);
      await writeFile(
        path.join(folder, `page-${String(entry.page).padStart(2, "0")}.jpg`),
        bytes,
      );
      pages.push({
        sourcePage: entry.page,
        nativePage: index + 1,
        archetype: entry.archetype,
        status: entry.status,
        path: relative,
        sha256,
      });
    }
    const record = {
      id: adapter.id,
      kind: "nativeAdaptationSample",
      sourceEquivalent: false,
      language: "zh",
      content:
        "Self-written sample content about a fictional reading club; every figure is a labelled sample value.",
      pptxSha256: digest(await readFile(files.out)),
      codeSha256: digest(Buffer.from(code)),
      adapterSha256: digest(Buffer.from(JSON.stringify(adapter))),
      lint,
      schemaErrors: validation.data.errors,
      embeddedMedia: media.length,
      sourcePages: adapter.pageMapping.length,
      unsupportedPages: adapter.pageMapping
        .filter((entry) => entry.status === "unsupported")
        .map((entry) => entry.page),
      pages,
      visualReview: "pending",
      targetApplicationEditing: "not-inferred",
    };
    if (publish) {
      const directory = path.join(base, "adaptation-previews", adapter.id);
      await rm(directory, { recursive: true, force: true });
      await mkdir(directory, { recursive: true });
      for (const [relative, bytes] of outputs)
        await writeFile(path.join(base, relative), bytes);
      manifest.styles = [
        ...manifest.styles.filter((style) => style.id !== adapter.id),
        record,
      ].sort((a, b) => (a.id < b.id ? -1 : 1));
      await writeFile(manifestFile, JSON.stringify(manifest, null, 2) + "\n");
    }
    console.log(
      JSON.stringify({
        style: adapter.id,
        pages: pages.length,
        unsupported: record.unsupportedPages.length,
        lint: lint.length,
        schemaErrors: validation.data.errors.length,
      }),
    );
    if (lint.length) console.log(lint.join("\n"));
  }
  // Keep only directories that still have a manifest record.
  if (publish) {
    const known = new Set(manifest.styles.map((style) => style.id));
    for (const name of await readdir(path.join(base, "adaptation-previews")))
      if (!known.has(name))
        await rm(path.join(base, "adaptation-previews", name), {
          recursive: true,
          force: true,
        });
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  const requested = process.argv
    .find((arg) => arg.startsWith("--styles="))
    ?.slice(9)
    .split(",");
  await renderAdaptations(requested, {
    publish: !process.argv.includes("--no-publish"),
  });
}
