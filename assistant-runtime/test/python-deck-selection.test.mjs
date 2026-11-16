import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { constants } from "node:fs";
import {
  chmod,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test, { after, before } from "node:test";
import { promisify } from "node:util";
import { createHash } from "node:crypto";
import {
  assets,
  exists,
  options,
  officecli,
  runner,
  staged,
  zipEntries,
  withoutDensity,
} from "./fixtures/python-engine.mjs";

let directory, node;
before(async () => {
  if (process.platform !== "darwin") return;
  directory = await mkdtemp(path.join(tmpdir(), "ofox-selected-style-tests-"));
  node = path.join(directory, "node");
  await copyFile(staged, node, constants.COPYFILE_FICLONE);
  await chmod(node, 0o700);
  await promisify(execFile)("/usr/bin/codesign", ["--remove-signature", node]);
  await promisify(execFile)("/usr/bin/codesign", [
    "--force",
    "--sign",
    "-",
    node,
  ]);
});
after(async () => {
  if (directory) await rm(directory, { recursive: true, force: true });
});

async function build(
  code,
  selectedStyle,
  mediaManifest,
  sourceEdit,
  overrides = {},
) {
  const work = await mkdtemp(path.join(directory, "case-"));
  const home = path.join(work, "home");
  await mkdir(home);
  const source = path.join(work, "source.py"),
    out = path.join(work, "deck.pptx"),
    lint = path.join(work, "lint.json");
  await writeFile(source, code);
  const manifestPath = path.join(work, "media.json");
  if (mediaManifest !== undefined)
    await writeFile(manifestPath, JSON.stringify(mediaManifest));
  const sourceMetaPath = path.join(work, "source-meta.json");
  if (sourceEdit)
    await writeFile(sourceMetaPath, JSON.stringify(sourceEdit.meta));
  const result = await promisify(execFile)(
    node,
    [
      runner,
      "--assets",
      overrides.assetsPath ?? assets,
      "--code",
      overrides.codeFile ?? source,
      "--out",
      out,
      "--lint",
      lint,
      ...(selectedStyle === undefined
        ? []
        : ["--selected-style", selectedStyle]),
      ...(mediaManifest === undefined
        ? []
        : ["--media-manifest", manifestPath]),
      ...(sourceEdit
        ? ["--source-deck", sourceEdit.path, "--source-meta", sourceMetaPath]
        : []),
      ...(overrides.extraArgs ?? []),
    ],
    {
      env: {
        PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
        HOME: home,
        USERPROFILE: home,
        TMPDIR: work,
        TEMP: work,
        TMP: work,
      },
      cwd: work,
      timeout: 150_000,
      maxBuffer: 3 * 1024 * 1024,
    },
  ).catch((error) => error);
  return {
    out,
    lint,
    status: result instanceof Error ? result.code : 0,
    stderr: result.stderr ?? "",
  };
}

test(
  "trusted selection overrides default and explicit legacy/model styles",
  options,
  async () => {
    const result = await build(
      `
first = new_deck()
legacy = new_deck(style="navy")
prs = new_deck(style="cartesian", language="zh")
assert all(p._ofox_context["style"] == "blue-professional" for p in [first, legacy, prs])
cover_slide(prs, "统一风格", subtitle="可信的手动选择")
s = content_slide(prs, "原生数学仍保留")
equation(s, 1, 2, 10, 1, {"latex": r"\\frac{1}{2}"}, size=24)
`,
      "blue-professional",
    );
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
      [],
    );
    const entries = await zipEntries(result.out);
    assert.match(entries.get("ppt/slides/slide1.xml").toString(), /1E2BFA/);
    const math = entries.get("ppt/slides/slide2.xml").toString();
    assert.match(math, /<m:f>/);
    assert.match(math, /<mc:Fallback>/);
    assert.ok(
      [...entries.keys()].some((name) => name.startsWith("ppt/media/")),
    );
  },
);

test(
  "trusted navy selection overrides an explicit public style",
  options,
  async () => {
    const result = await build(
      `
prs = new_deck(style="cartesian")
assert prs._ofox_context["style"] == "navy"
cover_slide(prs, "深蓝默认")
`,
      "navy",
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    assert.match(entries.get("ppt/slides/slide1.xml").toString(), /152B47/);
  },
);

test(
  "automatic selection leaves the existing default and explicit style unchanged",
  options,
  async () => {
    const result = await build(`
legacy = new_deck()
assert legacy._ofox_context["style"] == "navy"
prs = new_deck(style="cartesian")
assert prs._ofox_context["style"] == "cartesian"
cover_slide(prs, "自动保留模型风格")
`);
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    assert.match(entries.get("ppt/slides/slide1.xml").toString(), /EDE8E0/);
  },
);

test(
  "unknown trusted selection is refused before running model code or publishing outputs",
  options,
  async () => {
    const result = await build(
      'raise RuntimeError("MODEL_CODE_RAN")',
      "not-an-enabled-style",
    );
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /unknown or disabled selected style/);
    assert.doesNotMatch(result.stderr, /MODEL_CODE_RAN/);
    assert.equal(await exists(result.out), false);
    assert.equal(await exists(result.lint), false);
  },
);

test(
  "manual selection refuses a deck that bypasses native style context",
  options,
  async () => {
    const result = await build(
      'prs = Presentation("/template.pptx")',
      "cartesian",
    );
    assert.equal(result.status, 1, result.stderr);
    assert.match(result.stderr, /selected style requires a matching new_deck/);
    assert.equal(await exists(result.out), false);
    assert.equal(await exists(result.lint), false);
  },
);

const assetPng = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAQAAAADCAIAAAA7ljmRAAAAEElEQVR4nGOQs4qHIwacHACkYQiVfa22owAAAABJRU5ErkJggg==",
  "base64",
);
const manifest = () => ({
  planId: "plan-1",
  revision: 1,
  pageAssetIds: ["illustration"],
  pageFallbacks: ["stop"],
  assets: [
    {
      assetId: "illustration",
      mimeType: "image/png",
      sha256: createHash("sha256").update(assetPng).digest("hex"),
      dataBase64: assetPng.toString("base64"),
    },
  ],
});
test(
  "media manifest embeds only its controlled asset id and preserves native math",
  options,
  async () => {
    const code = [
      'prs = new_deck(style="navy")',
      's = content_slide(prs, "Controlled image")',
      "before = len(s.shapes)",
      "try:",
      '    image(s, "unknown", 1, 2, 4, 2, alt="unknown")',
      '    raise AssertionError("Unknown asset was accepted")',
      "except ValueError:",
      "    assert len(s.shapes) == before",
      'image(s, "illustration", 1, 2, 4, 2, alt="Authored evidence")',
      'equation(s, 1, 5, 8, 1, {"latex": r"\\frac{1}{2}"}, size=22)',
    ].join("\n");
    const result = await build(code, "cartesian", manifest());
    assert.equal(result.status, 0, result.stderr);
    // A planned deck reports what each page was built with, and nothing else here.
    assert.deepEqual(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
      ["plan-receipt:1=im,eq"],
    );
    const zip = await zipEntries(result.out);
    const xml = zip.get("ppt/slides/slide1.xml").toString();
    assert.match(xml, /Asset illustration/);
    assert.match(xml, /Authored evidence/);
    assert.match(xml, /<m:f>/);
    assert.ok(
      [...zip].some(
        ([name, bytes]) =>
          name.startsWith("ppt/media/") && bytes.equals(assetPng),
      ),
    );
  },
);
test(
  "media manifest rejects corrupt hashes and path fields before model code",
  options,
  async () => {
    for (const bad of [
      {
        ...manifest(),
        assets: [{ ...manifest().assets[0], sha256: "0".repeat(64) }],
      },
      {
        ...manifest(),
        assets: [{ ...manifest().assets[0], path: "/Users/private.png" }],
      },
    ]) {
      const result = await build(
        'raise RuntimeError("MODEL_CODE_RAN")',
        undefined,
        bad,
      );
      assert.equal(result.status, 1, result.stderr);
      assert.match(result.stderr, /media manifest|asset/i);
      assert.doesNotMatch(result.stderr, /MODEL_CODE_RAN|\/Users\/private/);
      assert.equal(await exists(result.out), false);
    }
  },
);
test(
  "private argument and input-file errors are recoverable while missing engine assets remain fatal",
  options,
  async () => {
    for (const overrides of [
      { extraArgs: ["--unknown-private-flag"] },
      { codeFile: path.join(directory, "missing-code.py") },
      {
        extraArgs: [
          "--media-manifest",
          path.join(directory, "missing-media.json"),
        ],
      },
    ]) {
      const result = await build(
        'raise RuntimeError("MODEL_CODE_RAN")',
        undefined,
        undefined,
        undefined,
        overrides,
      );
      assert.equal(result.status, 1, result.stderr);
      assert.doesNotMatch(
        result.stderr,
        /MODEL_CODE_RAN|missing-code\.py|missing-media\.json/,
      );
      assert.equal(await exists(result.out), false);
    }
    const broken = await build(
      'raise RuntimeError("MODEL_CODE_RAN")',
      undefined,
      undefined,
      undefined,
      { assetsPath: path.join(directory, "missing-engine-assets") },
    );
    assert.equal(broken.status, 2, broken.stderr);
    assert.doesNotMatch(broken.stderr, /MODEL_CODE_RAN/);
  },
);

test(
  "a planned image cannot cover the page's title or text unless it is placed behind them on purpose",
  options,
  async () => {
    const code = [
      'prs = new_deck(style="cartesian", language="en")',
      's = cover_slide(prs, "Clear title", subtitle="A self-written subtitle")',
      "before = len(s.shapes)",
      "try:",
      '    image(s, "illustration", 0.5, 1.8, 6, 3)',
      '    raise AssertionError("An image covering the title was accepted")',
      "except ValueError as error:",
      '    assert "covers" in str(error) and "Clear title" in str(error), str(error)',
      "    assert len(s.shapes) == before",
      // The title box is wide, but its words end well before the right edge.
      'image(s, "illustration", 9.4, 1.8, 3.2, 3.2)',
      "s._ofox_done = True",
    ].join("\n");
    const beside = await build(code, "cartesian", manifest());
    assert.equal(beside.status, 0, beside.stderr);
    const behind = await build(
      [
        'prs = new_deck(style="cartesian", language="en")',
        's = cover_slide(prs, "Clear title")',
        'image(s, "illustration", 0, 0, 13.333, 7.5, fit="cover", behind=True)',
      ].join("\n"),
      "cartesian",
      manifest(),
    );
    assert.equal(behind.status, 0, behind.stderr);
    const xml = (await zipEntries(behind.out))
      .get("ppt/slides/slide1.xml")
      .toString();
    const at = (needle) => xml.indexOf(needle);
    assert.ok(at('name="Background"') < at("Asset illustration"));
    assert.ok(at("Asset illustration") < at('type="title"'));
    // The page's own colour lies over the picture, half see-through, between
    // it and the text, so the title stays readable whatever was drawn.
    assert.ok(at("Asset illustration") < at('name="Image veil"'));
    assert.ok(at('name="Image veil"') < at('type="title"'));
    const veil = xml.slice(at('name="Image veil"'));
    assert.match(
      veil.slice(0, veil.indexOf("</p:sp>")),
      /<a:srgbClr val="[0-9A-F]{6}"><a:alpha val="60000"\/><\/a:srgbClr>/,
    );
    const bare = await build(
      [
        'prs = new_deck(style="cartesian", language="en")',
        's = cover_slide(prs, "Clear title")',
        'image(s, "illustration", 0, 0, 13.333, 7.5, fit="cover", behind=True, veil=0)',
      ].join("\n"),
      "cartesian",
      manifest(),
    );
    assert.equal(bare.status, 0, bare.stderr);
    assert.doesNotMatch(
      (await zipEntries(bare.out)).get("ppt/slides/slide1.xml").toString(),
      /Image veil/,
    );
    assert.match(
      withoutDensity(JSON.parse(await readFile(bare.lint, "utf8"))).join("\n"),
      /background image.*contrast/i,
    );
  },
);

test(
  "media manifest enforces page ownership and required-image fallback",
  options,
  async () => {
    const scoped = {
      ...manifest(),
      pageAssetIds: [null, "illustration"],
      pageFallbacks: ["omit", "stop"],
    };
    const code = [
      "prs = new_deck()",
      's = content_slide(prs, "First")',
      "before = len(s.shapes)",
      "try:",
      '    image(s, "illustration", 1, 2, 4, 2)',
      '    raise AssertionError("Cross-page asset was accepted")',
      "except ValueError:",
      "    assert len(s.shapes) == before",
      's = content_slide(prs, "Second")',
      'image(s, "illustration", 1, 2, 4, 2)',
    ].join("\n");
    const success = await build(code, undefined, scoped);
    assert.equal(success.status, 0, success.stderr);
    const missing = await build(
      'prs=new_deck(); content_slide(prs,"Missing")',
      undefined,
      manifest(),
    );
    assert.equal(missing.status, 1, missing.stderr);
    assert.match(missing.stderr, /required presentation image/);
    const fallback = await build(
      'prs=new_deck(); content_slide(prs,"Native summary")',
      undefined,
      { ...manifest(), pageFallbacks: ["native-summary"] },
    );
    assert.equal(fallback.status, 0, fallback.stderr);
    assert.match(
      JSON.stringify(
        withoutDensity(JSON.parse(await readFile(fallback.lint, "utf8"))),
      ),
      // The picture is there: say so, and how to draw it, rather than
      // letting the model conclude it could not be used.
      /slide 1: the plan's image \\"illustration\\" is ready but was not drawn: add image\(slide, \\"illustration\\", .*native-summary fallback used/,
    );
    const absent = await build(
      'prs=new_deck(); content_slide(prs,"No picture came")',
      undefined,
      { ...manifest(), pageFallbacks: ["omit"], assets: [] },
    );
    assert.equal(absent.status, 0, absent.stderr);
    assert.match(
      JSON.stringify(
        withoutDensity(JSON.parse(await readFile(absent.lint, "utf8"))),
      ),
      /planned image unavailable; omit fallback used/,
    );
  },
);
test(
  "source editing preserves all untargeted parts and original bytes",
  options,
  async () => {
    const original = await build(
      'prs=new_deck(title="Source fixture"); content_slide(prs,"First",notes="Keep first"); content_slide(prs,"Second",notes="Target"); content_slide(prs,"Third",notes="Keep third")',
    );
    assert.equal(original.status, 0, original.stderr);
    const bytes = await readFile(original.out);
    const sourceEdit = {
      path: original.out,
      meta: {
        version: createHash("sha256").update(bytes).digest("hex"),
        totalPages: 3,
        pageNumbers: [2],
      },
    };
    const edited = await build(
      'prs=source_deck(); s=target_slides(prs)[0]; s.shapes.title.text="Edited target"',
      "navy",
      undefined,
      sourceEdit,
    );
    assert.equal(edited.status, 0, edited.stderr);
    const before = await zipEntries(original.out),
      after = await zipEntries(edited.out);
    for (const [name, data] of before)
      if (name !== "ppt/slides/slide2.xml")
        assert.ok(
          after.get(name)?.equals(data),
          "Untargeted part changed: " + name,
        );
    assert.match(
      after.get("ppt/slides/slide2.xml").toString(),
      /Edited target/,
    );
    assert.ok((await readFile(original.out)).equals(bytes));
    const rebuilt = await build(
      'prs=new_deck(); content_slide(prs,"Replacement")',
      "navy",
      undefined,
      sourceEdit,
    );
    assert.equal(rebuilt.status, 1, rebuilt.stderr);
    assert.match(rebuilt.stderr, /source_deck|source edit/i);
    const corrupt = await build(
      'raise RuntimeError("MODEL_CODE_RAN")',
      undefined,
      undefined,
      { ...sourceEdit, meta: { ...sourceEdit.meta, version: "0".repeat(64) } },
    );
    assert.equal(corrupt.status, 1, corrupt.stderr);
    assert.match(corrupt.stderr, /assistant_presentation_source_unavailable/);
    assert.doesNotMatch(corrupt.stderr, /MODEL_CODE_RAN/);
    const scopedMedia = {
      ...manifest(),
      planPageNumbers: [2],
    };
    const withImage = await build(
      [
        "prs=source_deck()",
        "before=len(prs.slides[0].shapes)",
        "try:",
        '    image(prs.slides[0], "illustration", 1, 2, 4, 2)',
        '    raise AssertionError("Non-target page accepted an asset")',
        "except ValueError:",
        "    assert len(prs.slides[0].shapes)==before",
        'image(target_slides(prs)[0], "illustration", 1, 2, 4, 2)',
      ].join("\n"),
      undefined,
      scopedMedia,
      sourceEdit,
    );
    assert.equal(withImage.status, 0, withImage.stderr);
    const imageZip = await zipEntries(withImage.out);
    assert.match(
      imageZip.get("ppt/slides/slide2.xml").toString(),
      /Asset illustration/,
    );
    for (const page of [1, 3])
      assert.ok(
        imageZip
          .get(`ppt/slides/slide${page}.xml`)
          .equals(before.get(`ppt/slides/slide${page}.xml`)),
      );
    const wrongPage = await build(
      'raise RuntimeError("MODEL_CODE_RAN")',
      undefined,
      { ...scopedMedia, planPageNumbers: [1] },
      sourceEdit,
    );
    assert.equal(wrongPage.status, 1, wrongPage.stderr);
    assert.match(wrongPage.stderr, /assistant_presentation_source_unavailable/);
    assert.doesNotMatch(wrongPage.stderr, /MODEL_CODE_RAN/);
    assert.ok((await readFile(original.out)).equals(bytes));
  },
);
test(
  "an unsupported source is recoverable and does not break a later new deck",
  options,
  async () => {
    const bytes = Buffer.from("Not a supported PPTX archive");
    const source = path.join(directory, "unsupported-source.pptx");
    await writeFile(source, bytes);
    const invalid = await build(
      'raise RuntimeError("MODEL_CODE_RAN")',
      undefined,
      undefined,
      {
        path: source,
        meta: {
          version: createHash("sha256").update(bytes).digest("hex"),
          totalPages: 1,
          pageNumbers: [1],
        },
      },
    );
    assert.equal(invalid.status, 1, invalid.stderr);
    assert.match(invalid.stderr, /assistant_presentation_source_unavailable/);
    assert.doesNotMatch(
      invalid.stderr,
      /MODEL_CODE_RAN|unsupported-source\.pptx/,
    );
    assert.equal(await exists(invalid.out), false);
    const valid = await build(
      'prs=new_deck(); content_slide(prs,"Still available")',
    );
    assert.equal(valid.status, 0, valid.stderr);
    assert.ok(await exists(valid.out));
  },
);

test(
  "native_chart preserves provided values in a native chart and workbook",
  options,
  async () => {
    const code = [
      'prs = new_deck(style="cartesian")',
      's = content_slide(prs, "Supplied data")',
      'native_chart(s, "bar", ["A","B","C"], [{"name":"Actual","values":[30,45,25]}], 1, 2, 10, 4)',
      's = content_slide(prs, "Native line")',
      'native_chart(s, "line", ["A","B","C"], [{"name":"Trend","values":[-1,0,3]}], 1, 2, 10, 4)',
      's = content_slide(prs, "Native pie")',
      'native_chart(s, "pie", ["A","B","C"], [{"name":"Parts","values":[30,45,25]}], 1, 2, 10, 4)',
      'invalid = [("bar", ["A"], [{"name":"Bad","values":[float("nan")]}]), ("line", ["A","B"], [{"name":"Bad","values":[1]}]), ("pie", ["A","B"], [{"name":"Bad","values":[-1,2]}]), ("bar", ["A"]*101, [{"name":"Bad","values":[1]*101}])]',
      "for kind, categories, series in invalid:",
      "    before = len(s.shapes)",
      "    try:",
      "        native_chart(s, kind, categories, series, 1, 2, 10, 4)",
      '        raise AssertionError("Invalid native chart was accepted")',
      "    except ValueError:",
      "        assert len(s.shapes) == before",
    ].join("\n");
    const result = await build(code);
    assert.equal(result.status, 0, result.stderr);
    const zip = await zipEntries(result.out);
    assert.match(zip.get("ppt/slides/slide1.xml").toString(), /<c:chart /);
    const chart = [...zip].find(([name]) =>
      /^ppt\/charts\/chart\d+\.xml$/.test(name),
    );
    assert.ok(chart);
    assert.match(
      chart[1].toString(),
      /<c:v>30<\/c:v>.*<c:v>45<\/c:v>.*<c:v>25<\/c:v>/,
    );
    assert.equal(
      [...zip.keys()].filter(
        (name) => name.startsWith("ppt/embeddings/") && name.endsWith(".xlsx"),
      ).length,
      3,
    );
    assert.match(zip.get("ppt/charts/chart2.xml").toString(), /<c:lineChart>/);
    assert.match(zip.get("ppt/charts/chart3.xml").toString(), /<c:pieChart>/);
    assert.equal(
      [...zip.keys()].some((name) => name.startsWith("ppt/media/")),
      false,
    );
    assert.deepEqual(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8"))),
      [],
    );
    const validation = await promisify(execFile)(
      officecli,
      ["validate", result.out, "--json"],
      {
        env: {
          PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
          HOME: path.join(path.dirname(result.out), "home"),
          OFFICECLI_NO_AUTO_RESIDENT: "1",
        },
        timeout: 30_000,
      },
    );
    assert.deepEqual(JSON.parse(validation.stdout).data.errors, []);
  },
);

test(
  "a planned deck's receipt records diagrams and curves drawn in code",
  options,
  async () => {
    const plan = {
      ...manifest(),
      pageAssetIds: [null, null],
      pageFallbacks: ["omit", "omit"],
      assets: [],
    };
    const result = await build(
      [
        'prs = new_deck(style="studio", language="en")',
        's = content_slide(prs, "Architecture")',
        'figure(s, \'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1152 576"><rect x="10" y="10" width="300" height="120" fill="#FFFFFF" stroke="#000000"/></svg>\', 0.72, 1.8, 6, 3, alt="stack")',
        's = content_slide(prs, "Curve")',
        "fig, ax = plt.subplots()",
        "ax.plot([1, 2, 3]); ax.tick_params(labelsize=10)",
        'plot(s, fig, 0.72, 1.8, 6, 4, alt="curve")',
      ].join("\n"),
      "studio",
      plan,
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8")))[0],
      "plan-receipt:1=fg;2=pl",
    );
  },
);

test(
  "a plot sits on its page's colour and keeps its text readable on a dark page",
  options,
  async () => {
    const result = await build(
      [
        'prs = new_deck(style="knowledge-arch-blueprint", language="zh")',
        "from PIL import Image as Picture",
        "import io",
        "def corner(pic):",
        '    return "%02X%02X%02X" % Picture.open(io.BytesIO(pic.image.blob)).convert("RGB").getpixel((3, 3))',
        "def page(slide):",
        '    return slide._element.xpath("./p:cSld/p:bg//a:srgbClr/@val")[0]',
        's = content_slide(prs, "Curve")',
        "fig, ax = plt.subplots()",
        'ax.plot([1, 2, 3]); ax.set_title("Curve", fontsize=12); ax.tick_params(labelsize=10)',
        'pic = plot(s, fig, 0.72, 1.8, 6, 4, alt="curve")',
        'assert page(s) != "FFFFFF" and corner(pic) == page(s), (corner(pic), page(s))',
        's = content_slide(prs, "Dark")',
        'background(s, "1A1A1A")',
        "fig, ax = plt.subplots()",
        'ax.plot([1, 2, 3], color="#FFB000", label="series"); ax.set_title("Dark", fontsize=12); ax.tick_params(labelsize=10)',
        "legend = ax.legend(fontsize=10)",
        "# A number written into a light heatmap cell stays dark.",
        'cell = ax.text(1, 2, "0.71", fontsize=10)',
        'pic = plot(s, fig, 0.72, 1.8, 6, 4, alt="dark")',
        "from matplotlib.colors import to_hex",
        'assert corner(pic) == "1A1A1A", corner(pic)',
        'assert to_hex(ax.title.get_color()) != "#000000", ax.title.get_color()',
        'assert to_hex(ax.lines[0].get_color()) == "#ffb000", "a chosen colour is kept"',
        'assert to_hex(legend.get_texts()[0].get_color()) != "#000000"',
        'assert to_hex(legend.get_frame().get_facecolor(), keep_alpha=False) != "#ffffff", "light legend text on a white frame"',
        'assert to_hex(cell.get_color()) == "#000000", "text inside the data area keeps its colour"',
      ].join("\n"),
      "knowledge-arch-blueprint",
    );
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "a plot on a coloured page of the default style needs no theme colours",
  options,
  async () => {
    const result = await build(
      [
        'prs = new_deck(title="t")',
        's = content_slide(prs, "Curve")',
        'background(s, "F2F2F2")',
        "fig, ax = plt.subplots()",
        'ax.plot([1, 2, 3]); ax.set_title("Curve", fontsize=12); ax.tick_params(labelsize=10)',
        'plot(s, fig, 0.72, 1.8, 6, 4, alt="curve")',
        's = cover_slide(prs, "Cover")',
        "fig, ax = plt.subplots()",
        'ax.plot([1, 2, 3]); ax.set_title("On dark", fontsize=12); ax.tick_params(labelsize=10)',
        'plot(s, fig, 7, 1.8, 5, 4, alt="cover curve")',
        "from matplotlib.colors import to_hex",
        'assert to_hex(ax.title.get_color()) == "#ffffff", ax.title.get_color()',
      ].join("\n"),
    );
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "a figure whose SVG names no font still draws Chinese with a CJK font",
  options,
  async () => {
    const body =
      '<rect width="600" height="150" fill="#FFFFFF"/><text x="20" y="100" font-size="72" fill="#000000">编码器</text></svg>';
    const result = await build(
      [
        'prs = new_deck(style="navy", language="zh")',
        's = content_slide(prs, "Figure")',
        `bare = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150">${body}', 0.72, 1.8, 4, 1, alt="bare")`,
        `named = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150" font-family="PingFang SC">${body}', 0.72, 3.2, 4, 1, alt="named")`,
        'assert bare.image.blob == named.image.blob, "the bare figure fell back to a font without Chinese"',
        "# A font named on one other text element leaves the root without one.",
        `latin = '<text x="560" y="140" font-size="8" font-family="Georgia" fill="#FFFFFF">x</text>'`,
        `partial = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150">' + latin + '${body}', 0.72, 4.6, 4, 1, alt="partial")`,
        `rooted = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150" font-family="PingFang SC">' + latin + '${body}', 0.72, 5.8, 4, 1, alt="rooted")`,
        'assert partial.image.blob == rooted.image.blob, "a font on another element left Chinese without a CJK font"',
        "# A Latin font named for Chinese text: the renderer uses the first installed family",
        "# for every glyph, so a CJK font of the same kind goes first (found in a vellum deck).",
        `serif = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150" font-family="Georgia, SimSun, serif">${body}', 0.72, 1.8, 4, 1, alt="serif")`,
        `song = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150" font-family="Songti SC">${body}', 0.72, 3.2, 4, 1, alt="song")`,
        'assert serif.image.blob == song.image.blob, "Chinese in a serif figure drew without a CJK serif font"',
        `css = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150"><style>text { font-family: Arial, sans-serif; }</style>${body}', 0.72, 4.6, 4, 1, alt="css")`,
        'assert css.image.blob == named.image.blob, "a font named in CSS left Chinese without a CJK font"',
        "# Latin text keeps the font it asked for.",
        `english = '<rect width="600" height="150" fill="#FFFFFF"/><text x="20" y="100" font-size="72" fill="#000000">Encoder</text></svg>'`,
        `georgia = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150" font-family="Georgia">' + english, 0.72, 5.8, 4, 1, alt="georgia")`,
        `pingfang = figure(s, '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 600 150" font-family="PingFang SC">' + english, 0.72, 5.8, 4, 1, alt="pingfang")`,
        'assert georgia.image.blob != pingfang.image.blob, "a Latin-only figure lost its own font"',
      ].join("\n"),
      "navy",
    );
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "a planned deck's receipt names what each page was built with",
  options,
  async () => {
    const plan = {
      ...manifest(),
      pageAssetIds: ["illustration", null, null, null],
      pageFallbacks: ["omit", "omit", "omit", "omit"],
    };
    const result = await build(
      [
        'prs = new_deck(style="studio", language="en")',
        'cover_slide(prs, "Title")',
        's = content_slide(prs, "Numbers")',
        'kpi(s, 0.72, 2.0, 3.6, 2.2, "12", "Years")',
        'kpi(s, 4.6, 2.0, 3.6, 2.2, "3", "Teams")',
        's = content_slide(prs, "Chart")',
        'native_chart(s, "bar", ["a", "b"], [{"name": "n", "values": [1, 2]}], 0.72, 1.8, 11, 4.5)',
        'closing_slide(prs, "Thanks")',
      ].join("\n"),
      "studio",
      plan,
    );
    assert.equal(result.status, 0, result.stderr);
    assert.equal(
      withoutDensity(JSON.parse(await readFile(result.lint, "utf8")))[0],
      "plan-receipt:1=C;2=n2;3=cb;4=C",
    );
  },
);
