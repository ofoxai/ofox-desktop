import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { options, run, zipEntries } from "./fixtures/python-engine.mjs";

test(
  "read-only documented palette aliases track all six styles without changing geometry",
  options,
  async (t) => {
    const result = await run(
      t,
      `
for style in ["navy", "blue-professional", "cartesian", "course-module", "product-launch", "architectural-plan"]:
    prs = new_deck(style=style)
    s = content_slide(prs, "配色兼容")
    before = s._element.xml
    theme = prs._ofox_context["theme"]
    expected_card = theme["colors"]["card"].lstrip("#") if theme else "EEF3F8"
    expected_muted = theme["colors"]["textMuted"].lstrip("#") if theme else "98A2AE"
    assert T.card == T.tint == expected_card
    assert T.textMuted == T.faint == expected_muted
    for alias in ["card", "textMuted"]:
        try:
            setattr(T, alias, "000000")
            raise AssertionError("alias accepted a write")
        except AttributeError:
            pass
    assert s._element.xml == before, "palette access changed native geometry or text"
`,
    );
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "bring_title_to_front retains native ID, rich text, font, geometry and other sibling order",
  options,
  async (t) => {
    const result = await run(
      t,
      `
from lxml import etree
from pptx.oxml.ns import qn
prs = new_deck(style="product-launch")
s = content_slide(prs, "保留原生标题", notes="原备注保留")
other = content_slide(prs, "其他页面不变")
title = s.shapes.title
p = title.text_frame.paragraphs[0]
p.clear()
r = p.add_run(); r.text = "原生标题"; r.font.bold = True; r.font.color.rgb = rgb("FFFFFF"); r.font.size = Pt(26)
r = p.add_run(); r.text = "（概念）"; r.font.italic = True; r.font.color.rgb = rgb("F75B35"); r.font.size = Pt(22)
late = shape(s, MSO_SHAPE.RECTANGLE, 0, 0, 13.333, 1.5, fill=T.primaryDark)
tree = s.shapes._spTree
extension = etree.Element(qn("p:extLst")); tree.append(extension)
original_id, original_element = title.shape_id, title._element
original_xml = etree.tostring(original_element)
others = [item for item in tree if item is not original_element]
other_xml = other._element.xml
bounds = (title.left, title.top, title.width, title.height)
raised = bring_title_to_front(s)
assert raised.shape_id == original_id and raised._element is original_element and raised.is_placeholder
assert s.shapes.title._element is original_element
assert etree.tostring(original_element) == original_xml, "title rich text or style was rewritten"
assert (raised.left, raised.top, raised.width, raised.height) == bounds
assert [item for item in tree if item is not original_element] == others
assert list(tree).index(original_element) > list(tree).index(late._element)
assert list(tree)[-1] is extension and list(tree)[-2] is original_element
assert other._element.xml == other_xml
once = s._element.xml; bring_title_to_front(s); assert s._element.xml == once
# Missing titles fail without silently creating or changing any other object.
missing_title = other.shapes.title; other_tree = other.shapes._spTree
index = list(other_tree).index(missing_title._element); other_tree.remove(missing_title._element)
before_missing = other._element.xml
try:
    bring_title_to_front(other)
    raise AssertionError("missing native title was accepted")
except ValueError as error:
    assert "native title" in str(error)
assert other._element.xml == before_missing
other_tree.insert(index, missing_title._element)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    const title = [...xml.matchAll(/<p:sp>[\s\S]*?<\/p:sp>/g)]
      .map(([value]) => value)
      .find((value) => /<p:ph type="title"/.test(value));
    assert.match(title, /<p:cNvPr id="2" /);
    assert.match(title, /FFFFFF/);
    assert.match(title, /F75B35/);
    assert.match(title, /原生标题/);
    assert.match(title, /（概念）/);
    assert.match(xml, /<p:extLst\/>/);
  },
);

test("PPT guide documents the safe native-title helper and the two read-only palette aliases", async () => {
  const skill = await readFile(
    new URL("../skills/ofox-office-pptx/SKILL.md", import.meta.url),
    "utf8",
  );
  assert.match(skill, /bring_title_to_front\(slide\)/);
  assert.match(skill, /readonly[^\n]*T\.card[^\n]*T\.textMuted/i);
});
