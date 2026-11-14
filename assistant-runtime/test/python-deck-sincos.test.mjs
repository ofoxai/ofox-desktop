import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { promisify } from "node:util";
import {
  officecli,
  options,
  run,
  zipEntries,
} from "./fixtures/python-engine.mjs";

const POSITION_ENCODING = String.raw`
prs = new_deck(style="cartesian", language="zh")
s = content_slide(prs, "正弦与余弦位置编码")
sources = [
    r"\mathrm{PE}_{p,2i}=\sin\left(\frac{p}{10000^{\frac{2i}{d_{\mathrm{model}}}}}\right)",
    r"\mathrm{PE}_{p,2i+1}=\cos\left(\frac{p}{10000^{\frac{2i}{d_{\mathrm{model}}}}}\right)",
    r"\sin^{2}(x)+\cos^{2}(x)=1",
]
for index, source in enumerate(sources):
    equation(s, .85, 1.8 + index * 1.4, 11.6, 1.05, {"latex": source}, size=22)
from PIL import Image
import io
pngs = [part.blob for part in prs.part.package.iter_parts() if part.content_type == "image/png"]
assert len(pngs) == 3
for blob in pngs:
    image = Image.open(io.BytesIO(blob)).convert("RGBA")
    assert image.width > 80 and image.height > 20
    assert image.getchannel("A").getbbox() is not None, "math fallback must contain real rendered ink"
`;

test(
  "sin and cos position encoding preserve upright names, scripts and fractions in saved native math",
  options,
  async (t) => {
    const result = await run(t, POSITION_ENCODING);
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    assert.equal((xml.match(/<m:oMath>/g) ?? []).length, 3);
    const runs = [...xml.matchAll(/<m:r>[\s\S]*?<\/m:r>/g)].map(
      (match) => match[0],
    );
    const functions = runs.filter((run) =>
      /<m:t\b[^>]*>(?:sin|cos)<\/m:t>/.test(run),
    );
    assert.equal(functions.length, 4);
    for (const run of functions)
      assert.match(run, /<m:rPr><m:sty m:val="p"\s*\/><\/m:rPr>/);
    assert.equal((xml.match(/<m:sSub>/g) ?? []).length, 4);
    assert.equal((xml.match(/<m:sSup>/g) ?? []).length, 4);
    assert.equal((xml.match(/<m:f>/g) ?? []).length, 4);
    assert.doesNotMatch(xml, /<m:t\b[^>]*>[^<]*\\(?:sin|cos)/);
    assert.equal((xml.match(/<mc:Choice Requires="a14">/g) ?? []).length, 3);
    assert.equal(
      (
        xml.match(
          /name="Equation fallback \(rendered image, not editable math\)"/g,
        ) ?? []
      ).length,
      3,
    );
    assert.equal(
      [...entries.keys()].filter((name) => /^ppt\/media\/.*\.png$/.test(name))
        .length,
      3,
    );
    assert.deepEqual(JSON.parse(await readFile(result.lint, "utf8")), []);
    const verdict = await promisify(execFile)(
      officecli,
      ["validate", result.out, "--json"],
      {
        env: {
          PATH: "/usr/bin:/bin:/usr/sbin:/sbin",
          OFFICECLI_NO_AUTO_RESIDENT: "1",
          OFFICECLI_SKIP_UPDATE: "1",
        },
        timeout: 60_000,
      },
    );
    const report = JSON.parse(verdict.stdout);
    assert.equal(report.success, true);
    assert.deepEqual(report.data.errors, []);
  },
);

test(
  "quad keeps a fixed native em-space and rendered fallback",
  options,
  async (t) => {
    const result = await run(
      t,
      String.raw`prs=new_deck(); s=content_slide(prs,"Spacing"); equation(s,1,2,10,1,{"latex":r"x\quad y"})`,
    );
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    assert.match(xml, /<m:t\b[^>]*xml:space="preserve"[^>]*>\u2003<\/m:t>/);
    assert.equal((xml.match(/<m:oMath>/g) ?? []).length, 1);
    assert.equal(
      [...entries.keys()].filter((name) => /^ppt\/media\/.*\.png$/.test(name))
        .length,
      1,
    );
  },
);

test(
  "sin cos and quad do not admit other macros or leave partial slide mutations",
  options,
  async (t) => {
    const result = await run(
      t,
      String.raw`
prs=new_deck(); s=content_slide(prs,"Bounded names")
before=etree.tostring(s._element)
for source in [r"\sinh(x)",r"\cosh(x)",r"\sinx",r"\cosine(x)",r"\qquad x",r"\quadx",r"\sin(\input{secret})",r"\cos(\includegraphics{secret})",r"\csname sin\endcsname(x)",r"\def\foo{x}",r"\write18{anything}",r"\sin{"*25+"x"+"}"*25]:
    try:
        equation(s,1,2,10,1,{"latex":source})
    except ValueError:
        assert etree.tostring(s._element)==before, "rejected math must leave the slide XML unchanged"
        continue
    raise AssertionError("accepted unsupported math: "+source)
`,
    );
    assert.equal(result.status, 0, result.stderr);
  },
);
