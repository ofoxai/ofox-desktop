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
import {
  assets,
  options,
  root,
  runner,
  staged,
  zipEntries,
} from "./fixtures/python-engine.mjs";

// The staged production Node is hardened/Developer-ID signed. Only this
// private copy becomes ad-hoc so it can load the checkout's development addons.
let directory, node;
before(async () => {
  if (process.platform !== "darwin") return;
  directory = await mkdtemp(path.join(tmpdir(), "ofox-math-tests-"));
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

async function build(code) {
  const work = await mkdtemp(path.join(directory, "case-"));
  const home = path.join(work, "home");
  await mkdir(home);
  const source = path.join(work, "source.py"),
    out = path.join(work, "deck.pptx"),
    lint = path.join(work, "lint.json");
  await writeFile(source, code);
  const result = await promisify(execFile)(
    node,
    [
      runner,
      "--assets",
      assets,
      "--code",
      source,
      "--out",
      out,
      "--lint",
      lint,
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
    ...result,
    out,
    lint,
    home,
    work,
    status:
      result instanceof Error ? (result.code ?? result.signal ?? "error") : 0,
  };
}

const ACADEMIC = String.raw`
prs = new_deck(style="cartesian", language="zh")
s = content_slide(prs, "三条学术公式")
equation(s, .9, 1.9, 11.5, 1.2, {"latex": r"\hat{A}=D^{-\frac{1}{2}}(A+I)D^{-\frac{1}{2}}"}, size=24)
equation(s, .9, 3.4, 11.5, 1.2, {"latex": r"h^{(l+1)}=\sigma(\hat{A}h^{(l)}W^{(l)})"}, size=24)
equation(s, .9, 4.9, 11.5, 1.2, {"latex": r"\hat{y}=\sum_t{\alpha_t h_t}"}, size=24)
from PIL import Image
import io
# MS-ODRAWXML 2.2.5 + official Math example: the compatibility image is a
# p:sp picture fill, not a p:pic. Schema validation may select only Choice.
# https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/38b13e1f-1102-4bb7-819a-dd5d9abdb176
for fallback in s._element.iter("{http://schemas.openxmlformats.org/markup-compatibility/2006}Fallback"):
    assert fallback[0].tag == "{http://schemas.openxmlformats.org/presentationml/2006/main}sp", "math image fallback must be a p:sp"
    assert fallback[0].find(".//{http://schemas.openxmlformats.org/drawingml/2006/main}blipFill") is not None
for part in prs.part.package.iter_parts():
    if part.content_type == "image/png":
        picture = Image.open(io.BytesIO(part.blob)).convert("RGBA")
        bounds = picture.getchannel("A").getbbox()
        assert bounds is not None
        assert bounds[0] >= 4 and bounds[1] >= 4 and bounds[2] <= picture.width - 4 and bounds[3] <= picture.height - 4, "typeset formula needs transparent anti-clipping margins"
`;

const COMMON_COMMANDS = String.raw`
prs = new_deck(style="cartesian", language="zh")
s = content_slide(prs, "常用数学命令")
sources = [r"QK^\top", r"x_1,\dots,x_n;y_1,\ldots,y_n;z_1\cdots z_n", r"\operatorname{softmax}(x)+\mathrm{Attention}(Q,K,V)", r"\sqrt{d_k}+\sqrt{\frac{x^{2}}{y}}+\sqrt{\sqrt{x}}", r"\operatorname{Attention}(Q,K,V)=\operatorname{softmax}(\frac{QK^\top}{\sqrt{d_k}})V"]
unsupported = []
for index, source in enumerate(sources):
    try:
        equation(s, .85, 1.8 + index * .95, 11.6, .8, {"latex": source}, size=22)
    except ValueError as error:
        unsupported.append(str(error))
assert not unsupported, "common academic commands failed: " + repr(unsupported)
`;

test(
  "common academic LaTeX commands keep transpose, ellipses, upright names and nested roots as native math",
  options,
  async () => {
    const result = await build(COMMON_COMMANDS);
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    assert.equal((xml.match(/<m:oMath>/g) ?? []).length, 5);
    assert.equal((xml.match(/<m:rad>/g) ?? []).length, 5);
    assert.equal((xml.match(/<m:degHide m:val="1"/g) ?? []).length, 5);
    assert.equal((xml.match(/<m:deg\s*\/>/g) ?? []).length, 5);
    assert.equal((xml.match(/<m:sSup>/g) ?? []).length, 3);
    assert.match(xml, /<m:t[^>]*>⊤<\/m:t>/);
    assert.equal((xml.match(/<m:t[^>]*>…<\/m:t>/g) ?? []).length, 2);
    assert.match(xml, /<m:t[^>]*>⋯<\/m:t>/);
    assert.equal((xml.match(/<m:sty m:val="p"/g) ?? []).length, 4);
    assert.match(xml, /<m:t[^>]*>softmax<\/m:t>/);
    assert.match(xml, /<m:t[^>]*>Attention<\/m:t>/);
    assert.doesNotMatch(
      xml,
      /<m:t[^>]*>[^<]*\\(?:sqrt|mathrm|operatorname|top|dots|ldots|cdots)/,
    );
    assert.equal((xml.match(/<mc:Choice Requires="a14">/g) ?? []).length, 5);
    assert.equal(
      (
        xml.match(
          /name="Equation fallback \(rendered image, not editable math\)"/g,
        ) ?? []
      ).length,
      5,
    );
    assert.equal(
      [...entries.keys()].filter((name) => /^ppt\/media\/.*\.png$/.test(name))
        .length,
      5,
    );
    assert.deepEqual(JSON.parse(await readFile(result.lint, "utf8")), []);
    const verdict = await promisify(execFile)(
      path.join(root, "resources/engines/officecli/officecli"),
      ["validate", result.out, "--json"],
      {
        env: {
          PATH: "/usr/bin:/bin",
          HOME: result.home,
          OFFICECLI_NO_AUTO_RESIDENT: "1",
        },
        timeout: 60_000,
      },
    );
    assert.deepEqual(JSON.parse(verdict.stdout).data.errors, []);
  },
);

test(
  "bounded LaTeX emits the actual academic structures with rendered fallback",
  options,
  async () => {
    const result = await build(ACADEMIC);
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    const xml = entries.get("ppt/slides/slide1.xml").toString();
    assert.equal(
      (xml.match(/<m:acc>/g) ?? []).length,
      3,
      "A hat twice and y hat",
    );
    assert.equal(
      (xml.match(/<m:f>/g) ?? []).length,
      2,
      "two negative half powers",
    );
    assert.match(xml, /<m:nary>/);
    assert.match(xml, /<m:chr m:val="∑"/);
    assert.match(xml, /<m:t[^>]*>σ<\/m:t>/);
    assert.match(xml, /<m:t[^>]*>α<\/m:t>/);
    assert.ok((xml.match(/<m:sSup>/g) ?? []).length >= 5);
    assert.ok((xml.match(/<m:sSub>/g) ?? []).length >= 2);
    assert.doesNotMatch(xml, /<m:t[^>]*>[^<]*(?:_hat|\^|sum_t|sigma)/);
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
      path.join(root, "resources/engines/officecli/officecli"),
      ["validate", result.out, "--json"],
      {
        cwd: result.work,
        env: {
          PATH: "/usr/bin:/bin",
          HOME: result.home,
          USERPROFILE: result.home,
          TMPDIR: result.work,
          OFFICECLI_NO_AUTO_RESIDENT: "1",
          OFFICECLI_SKIP_UPDATE: "1",
        },
        timeout: 60_000,
      },
    );
    assert.deepEqual(JSON.parse(verdict.stdout).data.errors, []);
  },
);

test(
  "final JPEG renders all three normative math image-fill fallbacks",
  options,
  async () => {
    const result = await build(ACADEMIC);
    assert.equal(result.status, 0, result.stderr);
    const original = await readFile(result.out);
    const copy = path.join(result.work, "fallback-only.pptx");
    await promisify(execFile)("/usr/bin/python3", [
      "-c",
      String.raw`
import sys,zipfile,xml.etree.ElementTree as E
ns={'mc':'http://schemas.openxmlformats.org/markup-compatibility/2006','m':'http://schemas.openxmlformats.org/officeDocument/2006/math','p':'http://schemas.openxmlformats.org/presentationml/2006/main','a':'http://schemas.openxmlformats.org/drawingml/2006/main','r':'http://schemas.openxmlformats.org/officeDocument/2006/relationships'}
for prefix,uri in ns.items(): E.register_namespace(prefix,uri)
with zipfile.ZipFile(sys.argv[1]) as source,zipfile.ZipFile(sys.argv[2],'w') as dest:
    for info in source.infolist():
        data=source.read(info.filename)
        if info.filename.startswith('ppt/slides/slide') and info.filename.endswith('.xml'):
            root=E.fromstring(data)
            for parent in root.iter():
                for child in list(parent):
                    if child.tag=='{'+ns['mc']+'}AlternateContent' and child.find('.//m:oMath',ns) is not None:
                        fallback=child.find('mc:Fallback',ns)
                        assert fallback is not None and fallback[0].tag=='{'+ns['p']+'}sp'
                        at=list(parent).index(child);parent.remove(child)
                        for offset,node in enumerate(list(fallback)): parent.insert(at+offset,node)
            data=E.tostring(root,encoding='utf-8',xml_declaration=True)
        dest.writestr(info,data)
`,
      result.out,
      copy,
    ]);
    assert.ok(
      original.equals(await readFile(result.out)),
      "native delivery must remain byte-identical",
    );
    const env = {
      PATH: "/usr/bin:/bin",
      HOME: result.home,
      USERPROFILE: result.home,
      TMPDIR: result.work,
      OFFICECLI_NO_AUTO_RESIDENT: "1",
      OFFICECLI_SKIP_UPDATE: "1",
    };
    const office = path.join(root, "resources/engines/officecli/officecli");
    const verdict = await promisify(execFile)(
      office,
      ["validate", copy, "--json"],
      { env, timeout: 60_000 },
    );
    assert.deepEqual(JSON.parse(verdict.stdout).data.errors, []);
    const rendered = await promisify(execFile)(
      office,
      ["view", copy, "svg", "--start", "1", "--end", "1", "--json"],
      { env, timeout: 60_000, maxBuffer: 8 * 1024 * 1024 },
    );
    const svg = JSON.parse(rendered.stdout).data;
    assert.ok(
      (svg.match(/<image\b/g) ?? []).length >= 3,
      "all three picture-fill formulas must be rendered",
    );
    assert.match(svg, /data:image\/png/);
    const rectangles = [
      ...svg.matchAll(
        /<g transform="translate\(([\d.]+),([\d.]+)\)"><rect width="([\d.]+)" height="([\d.]+)" fill="url\(#[^)]+\)"\/><\/g>/g,
      ),
    ].map((match) => match.slice(1).map(Number));
    assert.equal(
      rectangles.length,
      3,
      "the actual OfficeCLI math pattern geometry",
    );
    const input = JSON.stringify([{ page: 1, svg }]);
    await writeFile(
      path.join(result.work, "document-preview-input.json"),
      input,
    );
    const raster = await promisify(execFile)(
      node,
      [path.join(root, "src/document-preview.mjs")],
      { cwd: result.work, env, timeout: 60_000 },
    );
    assert.deepEqual(JSON.parse(raster.stdout).pages, [1]);
    const pixels = path.join(result.work, "inspect-jpeg.mjs");
    const svgWidth = Number(svg.match(/<svg\b[^>]*\bwidth="([\d.]+)"/)[1]);
    await writeFile(
      pixels,
      `
import {createRequire} from 'node:module'; import {readFile} from 'node:fs/promises';
const require=createRequire(${JSON.stringify(path.join(root, "package.json"))}); const {createCanvas,loadImage}=require('@napi-rs/canvas');
const image=await loadImage(await readFile(${JSON.stringify(path.join(result.work, "document-preview-1.jpg"))}));const canvas=createCanvas(image.width,image.height),context=canvas.getContext('2d');context.drawImage(image,0,0);const data=context.getImageData(0,0,image.width,image.height).data,scale=image.width/${svgWidth};
const counts=${JSON.stringify(rectangles)}.map(([x,y,w,h])=>{let count=0;for(let row=Math.floor(y*scale);row<Math.ceil((y+h)*scale);row++)for(let col=Math.floor(x*scale);col<Math.ceil((x+w)*scale);col++){const p=(row*image.width+col)*4;if(data[p]<100&&data[p+1]<100&&data[p+2]<100)count++}return count});console.log(JSON.stringify(counts));
`,
    );
    const inspected = await promisify(execFile)(node, [pixels], {
      cwd: result.work,
      env,
      timeout: 60_000,
    });
    const counts = JSON.parse(inspected.stdout);
    assert.ok(
      counts.every((count) => count >= 80),
      `final JPEG formulas must contain real ink, got ${JSON.stringify(counts)}`,
    );
    assert.ok(
      original.equals(await readFile(result.out)),
      "the complete preview pipeline must not mutate native delivery",
    );
    assert.equal(
      await readFile(
        path.join(result.work, "document-preview-input.json"),
        "utf8",
      ),
      input,
      "preview normalization must not rewrite the OfficeCLI source SVG",
    );
  },
);

test(
  "legacy ASCII notation is reported instead of receiving a clean math verdict",
  options,
  async () => {
    const result = await build(
      `prs = new_deck()\ns=content_slide(prs,"Legacy")\nformula(s,1,2,11,2,["A_hat = D^-1/2 (A + I) D^-1/2", "y_hat = sum_t alpha_t * h_t"])`,
    );
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.ok(
      lint.some(
        (line) =>
          line.includes("legacy formula") && line.includes("not typeset"),
      ),
      JSON.stringify(lint),
    );
  },
);

test(
  "bounded LaTeX refuses unsupported commands and ambiguous syntax",
  options,
  async () => {
    const result = await build(String.raw`
prs = new_deck()
s=content_slide(prs,"Invalid")
before = len(s.shapes)
for source in [r"\input{secret}", r"\unknown{x}", "x^{", "x^^2", r"\frac{1}", r"\hat" * 30 + "x", "D^-1/2", "x^12", r"\sqrt[3]{x}", r"\sqrt x", r"\sqrt{}", r"\sqrt{x", r"\mathrm{x_i}", r"\operatorname{\input{x}}", r"\operatorname{softmax", r"\mathrm{}", r"\operatorname*{argmax}", r"\mathrm{" + "a" * 65 + "}", r"\sqrt{" * 25 + "x" + "}" * 25]:
    try:
        equation(s,1,2,10,1,{"latex":source})
    except ValueError:
        continue
    raise AssertionError("accepted unsupported math: " + source)
assert len(s.shapes) == before, "rejected equations must not leave partial shapes"
`);
    assert.equal(result.status, 0, result.stderr);
  },
);

test(
  "structured sequences preserve legacy fallback and literal TeX cannot pass as clean math",
  options,
  async () => {
    const result = await build(String.raw`
prs=new_deck()
s=content_slide(prs,"Structures")
equation(s,1,2,10,1,{"sequence":["x","=",{"hat":"y"}]})
equation(s,1,4,10,1,r"\hat{A}")
`);
    assert.equal(result.status, 0, result.stderr);
    const lint = JSON.parse(await readFile(result.lint, "utf8"));
    assert.ok(
      lint.some((line) => line.includes("unparsed math")),
      JSON.stringify(lint),
    );
  },
);

test(
  "sums require an unambiguous scope and phi matches the fallback glyph",
  options,
  async () => {
    const result = await build(String.raw`
prs=new_deck()
s=content_slide(prs,"Scope")
for source in [r"\sum_t a_t", r"(\sum_t a_t)+b", r"\sum_t a_t=b", r"\sum_t a_t\pm b", r"\sum_t a_t≤b", r"\sum_t a_t,b"]:
    try:
        equation(s,1,2,10,1,{"latex":source})
    except ValueError:
        continue
    raise AssertionError("ambiguous sum scope accepted")
equation(s,1,2,10,1,{"latex":r"(\sum_t{a_t})+b=\phi"})
math = next(s._element.iter("{http://schemas.openxmlformats.org/officeDocument/2006/math}nary"))
content = "".join(e.text or "" for e in math.iter("{http://schemas.openxmlformats.org/officeDocument/2006/math}t"))
assert "b" not in content and "ϕ" not in content
`);
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    assert.match(
      entries.get("ppt/slides/slide1.xml").toString(),
      /<m:t[^>]*>ϕ<\/m:t>/,
    );
  },
);
