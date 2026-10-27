import assert from "node:assert/strict";
import {
  mkdtemp,
  mkdir,
  writeFile,
  symlink,
  rm,
  realpath,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { zipSync, strToU8 } from "fflate";
import { AttachmentStore, officeText } from "../src/attachments.mjs";
import { pdfPages } from "./fixtures/pdf.mjs";
import {
  namedWorkbook,
  officeZip,
  repeatedSharedWorkbook,
  reorderedPresentation,
} from "./fixtures/office.mjs";

test("attachments are bounded, paged and confined to their imported root", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-attachments-"));
  try {
    await mkdir(path.join(root, "files"));
    await writeFile(
      path.join(root, "files", "notes.txt"),
      "line\n".repeat(3000),
    );
    const store = new AttachmentStore(root, [
      {
        id: "notes",
        path: "files/notes.txt",
        name: "notes.txt",
        mimeType: "text/plain",
      },
    ]);
    const first = await store.read("notes", 0, 50);
    assert.equal(first.text.length, 50);
    assert.equal(first.hasMore, true);
    await assert.rejects(() => store.read("../../elsewhere"), /attachment/i);
    const traversal = new AttachmentStore(root, [
      { id: "bad", path: "../escape", name: "x.txt" },
    ]);
    await assert.rejects(() => traversal.read("bad"), /root|path/i);
    if (process.platform !== "win32") {
      await symlink(path.dirname(root), path.join(root, "outside"));
      const link = new AttachmentStore(root, [
        { id: "link", path: "outside/missing", name: "x.txt" },
      ]);
      await assert.rejects(() => link.read("link"));
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test(
  "a host attachment root alias accepts its own absolute paths without allowing nested symlinks",
  { skip: process.platform === "win32" },
  async (t) => {
    const temporary = await mkdtemp(path.join(tmpdir(), "ofox-root-alias-"));
    t.after(() => rm(temporary, { recursive: true, force: true }));
    const canonical = await realpath(temporary);
    const root = path.join(canonical, "imported");
    const alias = path.join(canonical, "alias");
    await mkdir(root);
    await symlink(root, alias, "dir");
    await writeFile(path.join(root, "note.txt"), "confirmation: MANDARIN");
    for (const requested of ["note.txt", path.join(alias, "note.txt")]) {
      const store = new AttachmentStore(alias, [
        { id: "note", path: requested, name: "note.txt" },
      ]);
      assert.match((await store.read("note")).text, /MANDARIN/);
    }
    await writeFile(path.join(canonical, "outside.txt"), "private");
    await symlink(
      path.join(canonical, "outside.txt"),
      path.join(root, "linked.txt"),
    );
    for (const requested of [
      path.join(canonical, "outside.txt"),
      "../outside.txt",
      path.join(alias, "linked.txt"),
    ]) {
      const store = new AttachmentStore(alias, [
        { id: "outside", path: requested, name: "outside.txt" },
      ]);
      await assert.rejects(() => store.read("outside"), /root|symlink/i);
    }
  },
);

test("Office text is extracted without running macros or resolving XML entities", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-office-"));
  try {
    await writeFile(
      path.join(root, "safe.docx"),
      zipSync({
        "word/document.xml": strToU8(
          "<w:document><w:p><w:r><w:t>Quarterly revenue</w:t></w:r></w:p></w:document>",
        ),
      }),
    );
    const store = new AttachmentStore(root, [
      { id: "doc", path: "safe.docx", name: "safe.docx" },
    ]);
    assert.match((await store.read("doc")).text, /Quarterly revenue/);
    await writeFile(
      path.join(root, "evil.docx"),
      zipSync({
        "word/document.xml": strToU8(
          '<!DOCTYPE a [<!ENTITY x SYSTEM "file:///secret">]><w:t>&x;</w:t>',
        ),
      }),
    );
    const evil = new AttachmentStore(root, [
      { id: "doc", path: "evil.docx", name: "evil.docx" },
    ]);
    await assert.rejects(() => evil.read("doc"), /XML|entity|DOCTYPE/i);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

function blankPdf() {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R >>",
    "<< /Length 0 >>\nstream\n\nendstream",
  ];
  let output = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, index) => {
    offsets.push(Buffer.byteLength(output));
    output += `${index + 1} 0 obj\n${object}\nendobj\n`;
  });
  const xref = Buffer.byteLength(output);
  output += `xref\n0 5\n0000000000 65535 f \n${offsets
    .slice(1)
    .map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`)
    .join("")}trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(output);
}

test("scanned PDFs render bounded images without pretending to extract text", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-pdf-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "scan.pdf"), blankPdf());
  const store = new AttachmentStore(root, [
    { id: "scan", path: "scan.pdf", name: "scan.pdf" },
  ]);
  const text = await store.read("scan");
  assert.equal(text.scanned, true);
  assert.equal(text.pageCount, 1);
  assert.deepEqual(text.pagesWithoutText, [1]);
  assert.equal(text.textCoverage, "none");
  const image = await store.image("scan", 1);
  assert.equal(image.mimeType, "image/png");
  assert.equal(
    Buffer.from(image.data, "base64").subarray(1, 4).toString(),
    "PNG",
  );
  await assert.rejects(() => store.image("scan", 2), /page/i);
});

test("mixed PDFs disclose each page whose visual content was not extracted", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-mixed-pdf-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(
    path.join(root, "mixed.pdf"),
    pdfPages(["Cover only", null, null]),
  );
  const store = new AttachmentStore(root, [
    { id: "mixed", path: "mixed.pdf", name: "mixed.pdf" },
  ]);
  const result = await store.read("mixed");
  assert.match(result.text, /Cover only/);
  assert.equal(result.scanned, false);
  assert.deepEqual(result.pagesWithoutText, [2, 3]);
  assert.equal(result.textCoverage, "partial");
  assert.match(result.warning, /2, 3.*no extractable text/i);
  assert.match(result.warning, /not.*read|unread/i);
  const end = await store.read("mixed", result.nextOffset);
  assert.equal(end.hasMore, false);
  assert.deepEqual(end.pagesWithoutText, [2, 3]);
  assert.match(end.warning, /complete document/i);
});

test("fully textual PDFs preserve complete text coverage", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-text-pdf-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "text.pdf"), pdfPages(["First", "Second"]));
  const store = new AttachmentStore(root, [
    { id: "text", path: "text.pdf", name: "text.pdf" },
  ]);
  const result = await store.read("text");
  assert.deepEqual(result.pagesWithoutText, []);
  assert.equal(result.textCoverage, "complete");
  assert.equal(result.warning, undefined);
  assert.match(result.text, /First[\s\S]*Second/);
});

test(
  "a PDF final page exceeding the extraction limit has partial text coverage",
  { timeout: 45000 },
  async (t) => {
    const root = await mkdtemp(path.join(tmpdir(), "ofox-pdf-text-limit-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    // Keep the long text inside the page; PDF.js omits off-page text chunks.
    const scale = "0.000001 0 0 1 0 0 cm\n";
    let source = pdfPages(["A".repeat(16 * 1024 * 1024 + 64)]).toString();
    source = source
      .replace(
        /\/Length (\d+)/,
        (_, length) => `/Length ${Number(length) + scale.length}`,
      )
      .replace("stream\n", `stream\n${scale}`);
    // The single page's stream is the final object, so only startxref moves.
    source = source.replace(
      /startxref\n\d+/,
      `startxref\n${source.indexOf("\nxref\n") + 1}`,
    );
    const pdf = Buffer.from(source);
    assert.ok(pdf.length <= 25 * 1024 * 1024, "fixture must be importable");
    await writeFile(path.join(root, "large.pdf"), pdf);
    const store = new AttachmentStore(root, [
      { id: "large", path: "large.pdf", name: "large.pdf" },
    ]);
    const result = await store.read("large");
    assert.equal(result.pageCount, 1);
    assert.deepEqual(result.pagesWithoutText, []);
    assert.equal(result.truncated, true);
    assert.equal(result.textCoverage, "partial");
    assert.match(result.warning, /unread|limit/i);
  },
);

test("Office decompression limits reject a compressed XML bomb", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-zip-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(
    path.join(root, "bomb.docx"),
    zipSync({ "word/document.xml": strToU8("x".repeat(2 * 1024 * 1024)) }),
  );
  const store = new AttachmentStore(root, [
    { id: "bomb", path: "bomb.docx", name: "bomb.docx" },
  ]);
  await assert.rejects(() => store.read("bomb"), /decompression/i);
});

test("long text and Office documents can page beyond the initial extraction prefix", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-long-attachment-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const text = "a".repeat(250000) + "TAIL-MUST-BE-READABLE";
  const documents = {
    "long.txt": Buffer.from(text),
    "long.docx": zipSync({
      "word/document.xml": strToU8(
        `<w:document><w:t>${text}</w:t></w:document>`,
      ),
    }),
    "long.pptx": zipSync({
      "ppt/slides/slide1.xml": strToU8(`<p:sld><a:t>${text}</a:t></p:sld>`),
    }),
  };
  for (const [name, bytes] of Object.entries(documents)) {
    await writeFile(path.join(root, name), bytes);
    const store = new AttachmentStore(root, [{ id: "long", path: name, name }]);
    const extracted = await store.extract("long");
    const tailOffset = extracted.text.indexOf("TAIL-MUST-BE-READABLE");
    assert.ok(tailOffset >= 250000, `${name} must retain the tail for paging`);
    const tail = await store.read("long", tailOffset, 100);
    assert.match(tail.text, /TAIL-MUST-BE-READABLE/);
    assert.equal(tail.hasMore, false);
  }
});

test("extraction limits disclose unread content instead of claiming a complete document", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-text-limit-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "large.txt"), "a".repeat(17 * 1024 * 1024));
  const store = new AttachmentStore(root, [
    { id: "large", path: "large.txt", name: "large.txt" },
  ]);
  const first = await store.read("large");
  assert.equal(first.truncated, true);
  assert.match(first.warning, /limit|truncat/i);
  assert.equal(first.extractionLimit, 16 * 1024 * 1024);
});

test("sparse worksheets preserve cell coordinates and cached formula values", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-sparse-sheet-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(
    path.join(root, "sparse.xlsx"),
    zipSync({
      "xl/worksheets/sheet1.xml": strToU8(
        '<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Name</t></is></c><c r="C1" t="inlineStr"><is><t>Revenue</t></is></c></row><row r="3"><c r="A3" t="inlineStr"><is><t>Alice</t></is></c><c r="B3"><v>2</v></c><c r="C3"><f>B3*5</f><v>10</v></c></row></sheetData></worksheet>',
      ),
    }),
  );
  const store = new AttachmentStore(root, [
    { id: "sheet", path: "sparse.xlsx", name: "sparse.xlsx" },
  ]);
  const result = await store.read("sheet");
  assert.match(result.text, /A1:.*Name/);
  assert.match(result.text, /C1:.*Revenue/);
  assert.match(result.text, /A3:.*Alice/);
  assert.match(result.text, /B3:.*2/);
  assert.match(result.text, /C3:.*=B3\*5 \[cached: 10\]/);
  assert.doesNotMatch(result.text, /B1:.*Revenue/);
});

async function officeResult(t, name, entries) {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-office-order-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, name), officeZip(entries));
  return new AttachmentStore(root, [
    { id: "document", path: name, name },
  ]).extract("document");
}

test("OOXML presentation order and notes follow manifest relationships rather than part names", async (t) => {
  const result = await officeResult(t, "deck.pptx", reorderedPresentation());
  assert.match(
    result.text,
    /\[Slide 1\][\s\S]*FIRST-PRESENTED-SLIDE[\s\S]*\[Notes for slide 1\][\s\S]*FIRST-SLIDE-NOTES[\s\S]*\[Slide 2\][\s\S]*SECOND-PRESENTED-SLIDE/,
  );
  assert.equal(result.structureCoverage, "complete");
  assert.equal(result.warning, undefined);
});

test("OOXML workbook retains real worksheet names and declared order", async (t) => {
  const result = await officeResult(t, "book.xlsx", namedWorkbook());
  assert.match(
    result.text,
    /\[Worksheet 1: "Revenue"\][\s\S]*A1: "200"[\s\S]*\[Worksheet 2: "Costs"\][\s\S]*A1: "100"/,
  );
  assert.equal(result.structureCoverage, "complete");
  assert.equal(result.warning, undefined);
});

test("OOXML DOCX preserves interleaved paragraphs, tables and inline hyperlink order", async (t) => {
  const result = await officeResult(t, "flow.docx", {
    "word/document.xml":
      '<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Before </w:t></w:r><w:hyperlink><w:r><w:t>linked</w:t></w:r></w:hyperlink><w:r><w:t> after</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Middle table</w:t></w:r></w:p></w:tc></w:tr></w:tbl><w:p><w:r><w:t>End</w:t></w:r></w:p></w:body></w:document>',
  });
  assert.match(result.text, /Before linked after[\s\S]*Middle table[\s\S]*End/);
});

for (const [name, entries] of [
  [
    "minimal.pptx",
    { "ppt/slides/slide2.xml": "<p:sld><a:t>Legacy slide</a:t></p:sld>" },
  ],
  [
    "minimal.xlsx",
    {
      "xl/worksheets/sheet3.xml":
        '<worksheet><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>',
    },
  ],
])
  test(`OOXML missing metadata keeps ${name} readable with explicit partial structure`, async (t) => {
    const result = await officeResult(t, name, entries);
    assert.equal(result.structureCoverage, "partial");
    assert.match(result.warning, /metadata|order|names/i);
    assert.match(result.text, /Unmapped/);
    assert.doesNotMatch(result.text, /\[Slide 1\]|\[Worksheet 1:/);
    assert.match(
      result.text,
      name.endsWith("pptx") ? /Legacy slide/ : /A1: "7"/,
    );
  });

test("OOXML damaged metadata is disclosed instead of claiming logical order", async (t) => {
  for (const [name, entries, key] of [
    ["damaged.pptx", reorderedPresentation(), "ppt/presentation.xml"],
    ["damaged.xlsx", namedWorkbook(), "xl/_rels/workbook.xml.rels"],
  ]) {
    entries[key] = "<not-closed";
    const result = await officeResult(t, name, entries);
    assert.equal(result.structureCoverage, "partial");
    assert.match(result.warning, /metadata|relationship|order/i);
    assert.doesNotMatch(result.text, /\[Slide 1\]|\[Worksheet 1:/);
  }
});

test("OOXML relationships reject external, escaped and traversal targets without reading other parts", async (t) => {
  for (const target of [
    "https://external.invalid/slide.xml",
    "../../word/document.xml",
    "%2e%2e/%2e%2e/word/document.xml",
    "..\\..\\word\\document.xml",
  ]) {
    const entries = reorderedPresentation();
    entries["ppt/_rels/presentation.xml.rels"] = entries[
      "ppt/_rels/presentation.xml.rels"
    ].replace('Target="slides/slide2.xml"', `Target="${target}"`);
    entries["word/document.xml"] =
      "<w:document><w:t>UNRELATED-PART-MUST-NOT-BE-READ</w:t></w:document>";
    const result = await officeResult(t, "unsafe.pptx", entries);
    assert.doesNotMatch(result.text, /UNRELATED-PART-MUST-NOT-BE-READ/);
    assert.equal(result.structureCoverage, "partial");
    assert.match(result.warning, /relationship|mapping/i);
  }
  const entries = namedWorkbook();
  entries["xl/_rels/workbook.xml.rels"] = entries[
    "xl/_rels/workbook.xml.rels"
  ].replace(
    'Target="worksheets/sheet7.xml"',
    'Target="worksheets/sheet7.xml" TargetMode="External"',
  );
  const result = await officeResult(t, "external.xlsx", entries);
  assert.equal(result.structureCoverage, "partial");
  assert.match(result.warning, /relationship|mapping/i);
});

test("OOXML added metadata remains subject to decompression limits", async (t) => {
  const entries = reorderedPresentation();
  entries["ppt/presentation.xml"] = "x".repeat(2 * 1024 * 1024);
  await assert.rejects(
    () => officeResult(t, "bomb.pptx", entries),
    /decompression/i,
  );
});

for (const counts of [[512], [160, 160]])
  test(`OOXML shared-string expansion is bounded before joining ${counts.length} worksheets`, async (t) => {
    const entries = repeatedSharedWorkbook(counts);
    const expanded = officeText(officeZip(entries), ".xlsx");
    assert.ok(
      expanded.text.length <= 16 * 1024 * 1024 + 1,
      `pre-bounded expansion allocated ${expanded.text.length} characters`,
    );
    const result = await officeResult(t, "repeated.xlsx", entries);
    assert.equal(result.truncated, true);
    assert.equal(result.structureCoverage, "partial");
    assert.match(result.warning, /limit|unread/i);
  });

test("OOXML repeated slide references share the same document text budget", () => {
  const entries = reorderedPresentation();
  entries["ppt/presentation.xml"] = entries["ppt/presentation.xml"].replace(
    /<p:sldIdLst>[\s\S]*<\/p:sldIdLst>/,
    `<p:sldIdLst>${Array.from({ length: 512 }, (_, index) => `<p:sldId id="${index + 256}" rel:id="firstPart"/>`).join("")}</p:sldIdLst>`,
  );
  entries["ppt/slides/slide1.xml"] =
    `<p:sld><a:t>${"v".repeat(65536)}</a:t></p:sld>`;
  const result = officeText(officeZip(entries), ".pptx");
  assert.ok(result.text.length <= 16 * 1024 * 1024 + 1);
  assert.equal(result.structureCoverage, "partial");
  assert.match(result.warning, /limit|unread/i);
});

test("OOXML shared-string indices require explicit nonnegative decimal integers", async (t) => {
  for (const [label, index] of [
    ["missing", null],
    ["empty", ""],
    ["whitespace", "   "],
    ["nonnumeric", "not-an-index"],
    ["decimal", "1.0"],
    ["exponent", "1e0"],
    ["hexadecimal", "0x1"],
    ["negative", "-1"],
  ])
    await t.test(label, async (t) => {
      const entries = namedWorkbook();
      entries["xl/sharedStrings.xml"] =
        "<sst><si><t>FIRST-SHARED-STRING</t></si><si><t>SECOND-SHARED-STRING</t></si></sst>";
      entries["xl/worksheets/sheet7.xml"] =
        `<worksheet><sheetData><row r="1"><c r="A1" t="s">${index === null ? "" : `<v>${index}</v>`}</c></row></sheetData></worksheet>`;
      const result = await officeResult(t, "indices.xlsx", entries);
      assert.doesNotMatch(
        result.text,
        /FIRST-SHARED-STRING|SECOND-SHARED-STRING/,
      );
      assert.match(result.text, /Unread shared string/);
      assert.equal(result.structureCoverage, "partial");
      assert.match(result.warning, /shared strings.*missing|invalid/i);
    });
});

test("OOXML wrong main or relationship namespaces cannot certify document structure", async (t) => {
  const cases = [
    [
      "PPTX root",
      "deck.pptx",
      reorderedPresentation,
      "ppt/presentation.xml",
      (value) =>
        value.replace(
          'xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"',
          'xmlns:p="urn:unrelated-namespace"',
        ),
    ],
    [
      "PPTX list",
      "deck.pptx",
      reorderedPresentation,
      "ppt/presentation.xml",
      (value) =>
        value.replace(
          "<p:sldIdLst>",
          '<p:sldIdLst xmlns:p="urn:unrelated-namespace">',
        ),
    ],
    [
      "PPTX reference",
      "deck.pptx",
      reorderedPresentation,
      "ppt/presentation.xml",
      (value) =>
        value.replace(
          '<p:sldId rel:id="secondPart"',
          '<p:sldId xmlns:p="urn:unrelated-namespace" rel:id="secondPart"',
        ),
    ],
    [
      "XLSX root",
      "book.xlsx",
      namedWorkbook,
      "xl/workbook.xml",
      (value) =>
        value.replace(
          'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"',
          'xmlns="urn:unrelated-namespace"',
        ),
    ],
    [
      "XLSX list",
      "book.xlsx",
      namedWorkbook,
      "xl/workbook.xml",
      (value) =>
        value.replace("<sheets>", '<sheets xmlns="urn:unrelated-namespace">'),
    ],
    [
      "XLSX reference",
      "book.xlsx",
      namedWorkbook,
      "xl/workbook.xml",
      (value) =>
        value.replace(
          '<sheet name="Revenue"',
          '<sheet xmlns="urn:unrelated-namespace" name="Revenue"',
        ),
    ],
    [
      "PPTX relationships",
      "deck.pptx",
      reorderedPresentation,
      "ppt/_rels/presentation.xml.rels",
      (value) =>
        value.replace(
          'xmlns="http://schemas.openxmlformats.org/package/2006/relationships"',
          'xmlns="urn:unrelated-namespace"',
        ),
    ],
    [
      "XLSX relationships",
      "book.xlsx",
      namedWorkbook,
      "xl/_rels/workbook.xml.rels",
      (value) =>
        value.replace(
          'xmlns="http://schemas.openxmlformats.org/package/2006/relationships"',
          'xmlns="urn:unrelated-namespace"',
        ),
    ],
  ];
  for (const [label, name, create, key, alter] of cases)
    await t.test(label, async (t) => {
      const entries = create();
      entries[key] = alter(entries[key]);
      const result = await officeResult(t, name, entries);
      assert.equal(result.structureCoverage, "partial");
      assert.match(result.warning, /metadata|relationships/i);
      assert.match(result.text, /Unmapped/);
      assert.doesNotMatch(result.text, /\[Slide 1\]|\[Worksheet 1:/);
    });
});
