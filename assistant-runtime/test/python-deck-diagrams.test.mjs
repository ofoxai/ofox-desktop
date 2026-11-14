import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { promisify } from "node:util";
import {
  officecli,
  options,
  run,
  scratch,
  zipEntries,
} from "./fixtures/python-engine.mjs";

test(
  "native fork/join connects every parallel branch to its source and merge, never to another branch",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(style="course-module", language="zh")
s = lesson_slide(prs, "并行处理", objective="各分支独立处理，再汇总结果。")
other = new_deck(style="cartesian")
b = slide_regions(s)["main"]
fork_join(s, ["分支甲", "分支乙", "分支丙", "分支丁"], **b, source="共同输入", merge="汇总结果")
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    const nodes = new Map(
      [...xml.matchAll(/<p:sp>.*?<\/p:sp>/gs)].flatMap(([part]) => {
        const match = part.match(
          /<p:cNvPr id="(\d+)" name="(Fork join [^"]+)"/,
        );
        return match ? [[match[2], match[1]]] : [];
      }),
    );
    assert.equal(nodes.size, 6);
    const edges = [...xml.matchAll(/<p:cxnSp>.*?<\/p:cxnSp>/gs)].map(
      ([part]) => {
        assert.match(part, /<a:tailEnd type="triangle"/);
        assert.doesNotMatch(part, /<a:headEnd/);
        assert.match(part, /val="2D7D6E"/);
        const start = part.match(/<a:stCxn id="(\d+)" idx="3"/);
        const end = part.match(/<a:endCxn id="(\d+)" idx="1"/);
        assert.ok(start && end, "native connections attach to rectangle sides");
        return `${start[1]}->${end[1]}`;
      },
    );
    const expected = [1, 2, 3, 4].flatMap((number) => {
      const branch = nodes.get(`Fork join branch ${number}`);
      assert.ok(branch);
      return [
        `${nodes.get("Fork join source")}->${branch}`,
        `${branch}->${nodes.get("Fork join merge")}`,
      ];
    });
    assert.deepEqual(edges.sort(), expected.sort());
    assert.match(xml, /<a:ea typeface="SimSun"/);
    assert.doesNotMatch(xml, /val="E2DBD1"/);
    assert.ok(
      ![...entries.keys()].some((name) => name.startsWith("ppt/media/")),
    );
    assert.deepEqual(JSON.parse(await readFile(result.lint, "utf8")), []);
    const home = await scratch(t);
    const validation = await promisify(execFile)(
      officecli,
      ["validate", result.out, "--json"],
      {
        env: {
          PATH: process.env.PATH,
          HOME: home,
          OFFICECLI_NO_AUTO_RESIDENT: "1",
        },
        timeout: 60_000,
      },
    );
    const verdict = JSON.parse(validation.stdout);
    assert.equal(verdict.success, true, JSON.stringify(verdict));
    assert.deepEqual(verdict.data.errors, []);
  },
);

test(
  "a lesson body relation crossing the objective rail reports the actual overlap without silently moving it",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck(style="course-module", language="zh")
s = lesson_slide(prs, "嵌入与位置", objective="理解向量相加。")
boxes = relation(s, ["token → embedding", "+ position encoding", "处理结果"], x=0.72, y=1.9, w=11.5, h=3.6, kind="process", size=14)
assert boxes[0].left == Inches(0.72)
parallel = lesson_slide(prs, "并行区越界", objective="保留侧栏。")
fork_join(parallel, ["分支甲", "分支乙", "分支丙"], 0.72, 1.9, 11.5, 3.6, source="共同输入", merge="汇总")
safe = lesson_slide(prs, "区域内处理", objective="保留侧栏。")
relation(safe, ["输入", "输出"], **slide_regions(safe)["main"])
cover = cover_slide(prs, "自编示例")
relation(cover, ["甲", "乙"], 0.72, 5.5, 11.5, 0.8)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.equal(lint.length, 2, JSON.stringify(lint));
    assert.match(lint[0], /slide 1: relation.*main.*objective/i);
    assert.match(lint[0], /slide_regions/);
    assert.match(lint[1], /slide 2: fork_join.*main.*objective/i);
  },
);

test(
  "diagram bounds and branch counts fail before adding native objects, while optional endpoints and valid legacy positions remain supported",
  options,
  async (t) => {
    const result = await run(
      t,
      `
prs = new_deck()
s = content_slide(prs, "边界校验")
for box in [(-0.1, 2, 4, 1), (1, 2, 0, 1), (1, 2, 4, -1), (1, 2, float("nan"), 1), (12, 2, 3, 1), (1, 7, 4, 1)]:
    for draw in [relation, fork_join]:
        before = len(s.shapes)
        try:
            draw(s, ["甲", "乙"], *box)
            raise AssertionError("invalid diagram bounds accepted")
        except ValueError as error:
            assert "bounds" in str(error)
        assert len(s.shapes) == before
for branches in [["一个"], ["分支"] * 7, ["甲", ""], "甲乙"]:
    before = len(s.shapes)
    try:
        fork_join(s, branches, 0.8, 2, 11.5, 3)
        raise AssertionError("invalid parallel branches accepted")
    except ValueError:
        pass
    assert len(s.shapes) == before
graph = fork_join(s, ["甲", "乙"], 0.8, 2, 11.5, 3)
assert graph["source"] is None and graph["merge"] is None and not graph["connectors"]
relation(s, ["输入", "输出"], 0.8, 5.6, 11.5, 1.0)
for endpoint in [{"source": "共同输入"}, {"merge": "汇总"}]:
    page = content_slide(prs, "六分支边界")
    graph = fork_join(page, [str(i) for i in range(6)], 0.8, 1.9, 11.5, 4.5, **endpoint)
    assert len(graph["branches"]) == len(graph["connectors"]) == 6
`,
    );
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(await readFile(result.lint, "utf8")), []);
  },
);

test(
  "diagram bounds follow the actual presentation dimensions rather than the default 16:9 canvas",
  options,
  async (t) => {
    const result = await run(
      t,
      `
small = new_deck()
small.slide_width = Inches(10)
s = content_slide(small, "四比三原稿")
for draw in [relation, fork_join]:
    before = len(s.shapes)
    try:
        draw(s, ["甲", "乙"], 9, 2, 2, 2)
        raise AssertionError("diagram accepted outside actual 4:3 slide")
    except ValueError as error:
        assert "bounds" in str(error)
    assert len(s.shapes) == before
prs = new_deck()
prs.slide_width = Inches(20)
prs.slide_height = Inches(9)
wide = content_slide(prs, "宽幅原稿")
relation(wide, ["甲", "乙"], 15, 7.6, 4, 0.6)
fork_join(wide, ["甲", "乙"], 15, 5, 4, 2)
`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    assert.match(
      entries.get("ppt/presentation.xml").toString(),
      /<p:sldSz cx="18288000" cy="8229600"/,
    );
    assert.deepEqual(JSON.parse(await readFile(result.lint, "utf8")), []);
  },
);
