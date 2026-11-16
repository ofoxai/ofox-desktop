import test from "node:test";
import assert from "node:assert/strict";
import { documentTools } from "../src/document-tools.mjs";
import { parseDeck } from "../src/deck/schema.mjs";
import { renderDeck } from "../src/deck/render.mjs";
import { THEME_NAMES, themeFor } from "../src/deck/theme.mjs";
import { ADAPTED_STYLES } from "../src/deck/catalog.mjs";

const representatives = [
  "blue-professional",
  "cartesian",
  "course-module",
  "product-launch",
  "architectural-plan",
];
const runtime = {
  wrapTool: (_context, name, description, parameters, execute) => ({
    name,
    description,
    parameters,
    execute: (_id, args) => execute(args),
  }),
  hostTool: () =>
    assert.fail("Style discovery must not invoke an engine or host IO"),
};
const tools = (formats = ["pptx"]) =>
  documentTools(
    runtime,
    { params: { documents: { formats, python: false } } },
    { vision: false },
  );

test("blue comparison labels remain readable and chart series remain distinguishable", () => {
  const { colors } = themeFor("blue-professional");
  assert.notEqual(colors.chart[0], colors.chart[1]);
  const luminance = (hex) => {
    const parts = [0, 2, 4]
      .map((at) => parseInt(hex.slice(at, at + 2), 16) / 255)
      .map((c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return parts[0] * 0.2126 + parts[1] * 0.7152 + parts[2] * 0.0722;
  };
  const pair = [luminance(colors.secondary), luminance(colors.text)].sort(
    (a, b) => b - a,
  );
  assert.ok((pair[0] + 0.05) / (pair[1] + 0.05) >= 4.5);
});

test("PPT style discovery exposes adapted choices and provenance without executing source templates", async () => {
  const tool = tools().find((item) => item.name === "get_presentation_style");
  assert.ok(tool, "PPTX users need a style selection tool");
  const listed = await tool.execute("list", {});
  const catalog = JSON.parse(listed.content[0].text);
  assert.ok(catalog.candidateCount >= 20 && catalog.candidateCount <= 30);
  assert.deepEqual(
    catalog.styles.map((item) => item.id),
    ["navy", ...ADAPTED_STYLES.map((item) => item.id)],
  );
  for (const item of catalog.styles.filter((entry) => entry.id !== "navy")) {
    const source = ADAPTED_STYLES.find((entry) => entry.id === item.id);
    if (!source.pageMapping) continue;
    assert.equal(item.family, source.family);
    assert.equal(
      item.sourcePages,
      source.pageMapping.length,
      "actual source page count",
    );
    assert.equal(
      item.nativePages.adapted +
        item.nativePages.approximate +
        item.nativePages.unsupported,
      source.pageMapping.length,
    );
  }
  const result = await tool.execute("get", { id: "cartesian" });
  const style = JSON.parse(result.content[0].text);
  assert.equal(style.id, "cartesian");
  assert.match(style.source.revision, /^[a-f0-9]{40}$/);
  assert.ok(style.recipe.composition);
  assert.ok(style.workflow.pagePlan.length >= 4);
  assert.equal(style.validation.visualQuality, "pending");
  assert.equal(style.editability.htmlConversion, "unsupported");
  assert.ok(
    style.workflow.references.every((item) =>
      item.url.startsWith("https://github.com/"),
    ),
  );
  await assert.rejects(
    () => tool.execute("bad", { id: "../../LICENSE" }),
    /Unknown presentation style/,
  );
  assert.equal(
    tools(["html"]).some((item) => item.name === tool.name),
    false,
  );
});

test("representative deck objects preserve content while changing composition and type roles", () => {
  const titleBoxes = new Set();
  const compositions = new Set();
  for (const id of representatives) {
    assert.ok(THEME_NAMES.includes(id), `${id} must be selectable`);
    const parsed = parseDeck({
      kind: "deck",
      title: "统一样张",
      language: "zh",
      theme: id,
      slides: [
        { type: "hero", title: "统一样张", subtitle: "合成数据，仅用于验证" },
        {
          type: "pillars",
          title: "两种方案",
          pillars: [
            { heading: "全量", body: "每批重建" },
            { heading: "增量", body: "跟踪变更" },
          ],
          notes: "尚无性能结论",
        },
        { type: "cta", title: "继续验证" },
      ],
    });
    assert.equal(parsed.problems, undefined);
    assert.equal(parsed.deck.theme, id);
    const theme = themeFor(id);
    compositions.add(theme.decor.composition);
    const { commands } = renderDeck(parsed.deck, theme);
    const heading = commands.find(
      (cmd) => cmd.path === "/slide[1]/placeholder[title]",
    );
    titleBoxes.add(
      JSON.stringify([
        heading.props.x,
        heading.props.y,
        heading.props.width,
        heading.props.align,
      ]),
    );
    assert.ok(commands.some((cmd) => cmd.props?.text === "每批重建"));
    assert.ok(commands.some((cmd) => cmd.props?.text === "尚无性能结论"));
  }
  assert.equal(compositions.size, 5);
  assert.ok(
    titleBoxes.size >= 4,
    "Representatives must change composition, not only colors",
  );
});
