import assert from "node:assert/strict";
import test from "node:test";
import { options, run, zipEntries } from "./fixtures/python-engine.mjs";

const bounds = (xml) => {
  const offset = xml.match(/<a:off x="(-?\d+)" y="(-?\d+)"/);
  const extent = xml.match(/<a:ext cx="(\d+)" cy="(\d+)"/);
  assert.ok(offset && extent, "native title/rectangle has an explicit box");
  return [
    Number(offset[1]),
    Number(offset[2]),
    Number(extent[1]),
    Number(extent[2]),
  ];
};

const intersects = ([ax, ay, aw, ah], [bx, by, bw, bh]) =>
  Math.min(ax + aw, bx + bw) > Math.max(ax, bx) &&
  Math.min(ay + ah, by + bh) > Math.max(ay, by);

test(
  "native titles paint after overlapping opaque decorations while preserving title identity and other object order",
  options,
  async (t) => {
    for (const style of [
      "product-launch",
      "architectural-plan",
      "course-module",
      "blue-professional",
      "cartesian",
      "navy",
    ]) {
      const result = await run(
        t,
        `
import deck
original_title = deck._native_title
def identity_checked_title(slide, *args, **kwargs):
    title = slide.shapes.title
    original_element, original_id = title._element, title.shape_id
    others = [child for child in slide.shapes._spTree if child is not original_element]
    value = original_title(slide, *args, **kwargs)
    assert value._element is original_element and value.shape_id == original_id
    assert [child for child in slide.shapes._spTree if child is not original_element] == others
    assert value.is_placeholder and slide.shapes.title._element is original_element
    return value
deck._native_title = identity_checked_title
prs = new_deck(style="${style}", language="zh")
cover_slide(prs, "产品概览", subtitle="保留原生标题", kicker="自编样例", notes="封面来源保留")
section_slide(prs, "核心问题", number="01", notes="章节来源保留")
content_slide(prs, "清晰的产品说明", subtitle="标题应显示在不透明页头之上", notes="内容来源保留")
`,
      );
      assert.equal(result.status, 0, `${style}: ${result.stderr}`);
      const entries = await zipEntries(result.out);
      for (let page = 1; page <= 3; page++) {
        const xml = entries.get(`ppt/slides/slide${page}.xml`).toString();
        const shapes = [...xml.matchAll(/<p:sp>[\s\S]*?<\/p:sp>/g)].map(
          ([shape]) => shape,
        );
        const titles = shapes.filter((shape) =>
          /<p:ph type="(?:title|ctrTitle)"/.test(shape),
        );
        assert.equal(
          titles.length,
          1,
          `${style} page ${page}: one native title`,
        );
        const title = titles[0],
          titleIndex = shapes.indexOf(title);
        assert.match(
          title,
          /<p:cNvPr id="2" /,
          "original cloned title ID retained",
        );
        const ids = shapes.map(
          (shape) => shape.match(/<p:cNvPr id="(\d+)"/)?.[1],
        );
        assert.equal(
          new Set(ids).size,
          ids.length,
          "native shape IDs stay unique",
        );
        assert.ok(entries.has(`ppt/notesSlides/notesSlide${page}.xml`));
        // The unchanged legacy section/content preset inherits its title box.
        // _native_title is used by public-style helpers, not this legacy path.
        if (style === "navy" && !/<a:xfrm>/.test(title)) continue;
        const titleBounds = bounds(title);
        let overlappingOpaque = 0;
        for (let index = 0; index < shapes.length; index++) {
          const properties = shapes[index].match(
            /<p:spPr>[\s\S]*?<\/p:spPr>/,
          )?.[0];
          if (
            !properties ||
            !/<a:prstGeom prst="rect"/.test(properties) ||
            !/<a:solidFill>/.test(properties.split("<a:ln")[0])
          )
            continue;
          if (!intersects(titleBounds, bounds(properties))) continue;
          overlappingOpaque++;
          assert.ok(
            titleIndex > index,
            `${style} page ${page}: title at z=${titleIndex} is covered by opaque rectangle at z=${index}`,
          );
        }
        assert.ok(
          overlappingOpaque >= 1,
          "background is explicitly behind the title",
        );
        if (style === "product-launch" && page === 3)
          assert.equal(
            overlappingOpaque,
            2,
            "the real opaque launch header intersects the native title, in addition to background",
          );
      }
    }
  },
);
