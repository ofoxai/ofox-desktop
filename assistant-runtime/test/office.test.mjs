import assert from "node:assert/strict";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { readOffice } from "../src/office.mjs";
import { AttachmentStore, prepareAttachments } from "../src/attachments.mjs";
import {
  officeZip,
  namedWorkbook,
  reorderedPresentation,
  repeatedSharedWorkbook,
} from "./fixtures/office.mjs";

const word = (body) => ({
  "word/document.xml": `<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>${body}</w:body></w:document>`,
});
const paragraph = (text) => `<w:p><w:r><w:t>${text}</w:t></w:r></w:p>`;
async function store(t, extension, bytes) {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-office-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "file"), bytes);
  return new AttachmentStore(root, [
    { id: "file", path: "file", name: `中文.${extension}` },
  ]);
}
async function result(t, extension, entries) {
  const reader = await store(
    t,
    extension,
    Buffer.isBuffer(entries) ? entries : officeZip(entries),
  );
  return prepareAttachments(reader, ["file"]);
}

test("Office Word keeps Chinese paragraphs, hyperlinks and table cells in document order", async (t) => {
  const entries = word(
    "<w:p><w:r><w:t>正文</w:t></w:r><w:hyperlink><w:r><w:t>链接</w:t></w:r></w:hyperlink><w:r><w:t>末尾 &amp; &#20013;</w:t></w:r></w:p><w:tbl><w:tr><w:tc>" +
      paragraph("上海") +
      "</w:tc><w:tc>" +
      paragraph("42") +
      "</w:tc></w:tr></w:tbl>" +
      paragraph("结束"),
  );
  entries["word/media/ignored.png"] = "not an image";
  const read = await result(t, "docx", entries);
  assert.equal(read.blocked, null);
  assert.match(read.text, /正文链接末尾 & 中[\s\S]*上海[\s\S]*42[\s\S]*结束/);
  assert.equal(read.images.length, 0);
  assert.equal(read.outcomes[0].status, "partial");
  assert.equal(read.outcomes[0].details.textOnly, true);
  assert.match(read.text, /images.*not.*read/i);
});

test("Office workbook follows sheet names/order, sparse coordinates, shared strings and cached formulas", async (t) => {
  const entries = namedWorkbook();
  entries["xl/workbook.xml"] = entries["xl/workbook.xml"].replace(
    'name="Revenue"',
    'name="中文收入"',
  );
  entries["xl/sharedStrings.xml"] = "<sst><si><t>上海</t></si></sst>";
  entries["xl/worksheets/sheet7.xml"] =
    '<worksheet><sheetData><row r="3"><c r="B3" t="s"><v>0</v></c><c r="D3"><f>SUM(A1:A2)</f><v>42</v></c><c r="F3" t="inlineStr"><is><t>日本語</t></is></c></row></sheetData></worksheet>';
  const read = await result(t, "xlsx", entries);
  assert.equal(read.blocked, null);
  assert.match(
    read.text,
    /Worksheet 1: "中文收入"[\s\S]*B3: "上海"\tD3: "=SUM\(A1:A2\) \[cached: 42\]"\tF3: "日本語"[\s\S]*Worksheet 2: "Costs"/,
  );
  assert.equal(read.outcomes[0].details.structureCoverage, "complete");
});

test("Office presentations follow actual slide order and attach notes to their slide", async (t) => {
  const entries = reorderedPresentation();
  entries["ppt/slides/slide2.xml"] = entries["ppt/slides/slide2.xml"].replace(
    "FIRST-PRESENTED-SLIDE",
    "中文第一页",
  );
  const read = await result(t, "pptx", entries);
  assert.equal(read.blocked, null);
  assert.match(
    read.text,
    /\[Slide 1\][\s\S]*中文第一页[\s\S]*\[Notes for slide 1\][\s\S]*FIRST-SLIDE-NOTES[\s\S]*\[Slide 2\][\s\S]*SECOND-PRESENTED-SLIDE/,
  );
});

test("Office missing relationships fall back with an explicit incomplete-structure warning", async (t) => {
  const entries = namedWorkbook();
  delete entries["xl/_rels/workbook.xml.rels"];
  const read = await result(t, "xlsx", entries);
  assert.equal(read.blocked, null);
  assert.match(read.text, /Unmapped worksheet part/);
  assert.equal(read.outcomes[0].details.structureCoverage, "partial");
});

test("Office external relationships never load a URL or mislabel another part", async (t) => {
  const entries = namedWorkbook();
  entries["xl/_rels/workbook.xml.rels"] = entries[
    "xl/_rels/workbook.xml.rels"
  ].replace(
    'Target="worksheets/sheet7.xml"',
    'Target="https://example.invalid/secret" TargetMode="External"',
  );
  const read = await result(t, "xlsx", entries);
  assert.equal(read.blocked, null);
  assert.match(read.text, /Worksheet 1: unavailable relationship/);
  assert.doesNotMatch(read.text, /200|example.invalid/);
  assert.equal(read.outcomes[0].details.structureCoverage, "partial");
});

for (const [name, entries, reason] of [
  ["damaged ZIP", Buffer.from("PKbroken"), "invalid_office"],
  [
    "password-protected Office container",
    Buffer.concat([
      Buffer.from("d0cf11e0a1b11ae1", "hex"),
      Buffer.from("EncryptedPackage", "utf16le"),
    ]),
    "encrypted_office",
  ],
  ["empty Word document", word(""), "empty_office"],
  [
    "malformed XML",
    { "word/document.xml": "<document><p></document>" },
    "invalid_office",
  ],
  [
    "entity declaration",
    {
      "word/document.xml":
        '<!DOCTYPE document [<!ENTITY x SYSTEM "file:///etc/passwd">]><document><t>&x;</t></document>',
    },
    "invalid_office",
  ],
  ["missing document part", { "word/media/a.png": "x" }, "invalid_office"],
])
  test(`Office rejects ${name} before model dispatch`, async (t) => {
    const read = await result(t, "docx", entries);
    assert.equal(read.blocked?.code, "assistant_attachment_unreadable");
    assert.equal(read.outcomes[0].details.reason, reason);
    assert.equal(read.text, "");
  });

for (const [name, entries] of [
  ["high compression ratio", word(paragraph("x".repeat(2 * 1024 * 1024)))],
  [
    "too many entries",
    Object.fromEntries(
      Array.from({ length: 4097 }, (_, i) => [`part${i}`, ""]),
    ),
  ],
  [
    "deep XML",
    { "word/document.xml": "<x>".repeat(130) + "text" + "</x>".repeat(130) },
  ],
])
  test(`Office bounds ${name}`, async (t) => {
    const read = await result(t, "docx", entries);
    assert.equal(read.blocked?.code, "assistant_attachment_too_large");
    assert.equal(read.outcomes[0].details.limit, "office_expansion");
  });

test("Office validates actual decoded size and CRC instead of trusting ZIP metadata", async (t) => {
  for (const offset of [16, 24]) {
    const bytes = officeZip(word(paragraph("中文完整正文")));
    const central = bytes.indexOf(Buffer.from("504b0102", "hex"));
    bytes.writeUInt32LE(1, central + offset);
    const read = await result(t, "docx", bytes);
    assert.equal(read.outcomes[0].status, "unreadable");
    assert.equal(read.outcomes[0].details.reason, "invalid_office");
  }
});

test("Office rejects encrypted ZIP entries", async (t) => {
  const bytes = officeZip(word(paragraph("中文")));
  const central = bytes.indexOf(Buffer.from("504b0102", "hex"));
  bytes.writeUInt16LE(1, central + 8);
  const read = await result(t, "docx", bytes);
  assert.equal(read.outcomes[0].details.reason, "encrypted_office");
});

test("Office shared-string expansion is bounded and reported as truncated", async (t) => {
  const read = await result(t, "xlsx", repeatedSharedWorkbook([140, 140]));
  assert.equal(read.blocked, null);
  assert.equal(read.outcomes[0].status, "partial");
  assert.equal(read.outcomes[0].details.truncated, true);
  assert.equal(read.outcomes[0].details.characters, 16 * 1024 * 1024);
});

test("Office reads are cancellable without becoming an attachment failure", async (t) => {
  const reader = await store(t, "docx", officeZip(word(paragraph("取消测试"))));
  const controller = new AbortController();
  const pending = prepareAttachments(reader, ["file"], {
    signal: controller.signal,
  });
  setImmediate(() => controller.abort(new Error("user cancelled")));
  await assert.rejects(pending, /user cancelled/);
});

test("old .doc stays unsupported with a conversion hint", async (t) => {
  const read = await result(t, "doc", Buffer.from("old doc"));
  assert.equal(read.outcomes[0].status, "unsupported");
  assert.match(read.blocked.message, /docx or PDF/i);
});

for (const [format, expected] of [
  [
    "docx",
    ["星河项目季度报告", "中文正文：本季度完成验收", "上海", "42", "星河归档"],
  ],
  [
    "xlsx",
    [
      "中文收入",
      "上海",
      "12345.67",
      "D4",
      "SUM(B2:B3)",
      "日文成本",
      "東京支出",
    ],
  ],
  [
    "pptx",
    [
      "中文第一页",
      "中文第二页",
      "提升质量",
      "全部通过",
      "讲者备注",
      "深圳",
      "88",
    ],
  ],
])
  test(`real Chinese ${format} generated by an Office writer is readable`, async (t) => {
    const bytes = await readFile(
      new URL(`./fixtures/office/chinese.${format}`, import.meta.url),
    );
    const read = await result(t, format, bytes);
    assert.equal(read.blocked, null);
    for (const text of expected) assert.ok(read.text.includes(text), text);
    assert.equal(read.outcomes[0].details.structureCoverage, "complete");
  });

test("spreadsheet phonetic guides do not change a cell's visible value", async (t) => {
  const entries = namedWorkbook();
  entries["xl/sharedStrings.xml"] =
    '<sst><si><t>東京都</t><rPh sb="0" eb="3"><t>トウキョウト</t></rPh></si></sst>';
  entries["xl/worksheets/sheet7.xml"] =
    '<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>上海</t><rPh sb="0" eb="2"><t>shanghai</t></rPh></is></c></row></sheetData></worksheet>';
  const read = await result(t, "xlsx", entries);
  assert.match(read.text, /A1: "東京都"\tB1: "上海"/);
  assert.doesNotMatch(read.text, /トウキョウト|shanghai/);
});

test("Word compatibility alternatives contribute their fallback text only once", async (t) => {
  const read = await result(
    t,
    "docx",
    word(
      '<mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="wps">' +
        paragraph("复杂图形替代文字") +
        "</mc:Choice><mc:Fallback>" +
        paragraph("兼容文本框正文") +
        "</mc:Fallback></mc:AlternateContent>",
    ),
  );
  assert.equal(read.blocked, null);
  assert.match(read.text, /兼容文本框正文/);
  assert.doesNotMatch(read.text, /复杂图形替代文字/);
});

test("cancelling an already-created Office worker returns promptly", async () => {
  const controller = new AbortController();
  const started = Date.now();
  const pending = readOffice(
    officeZip(word(paragraph("worker cancellation"))),
    ".docx",
    controller.signal,
  );
  controller.abort(new Error("cancel running worker"));
  await assert.rejects(pending, /cancel running worker/);
  assert.ok(Date.now() - started < 2000);
});

test("Office enforces the combined decompressed XML budget across parts", async (t) => {
  const entries = namedWorkbook();
  for (const key of ["xl/worksheets/sheet7.xml", "xl/worksheets/sheet3.xml"])
    entries[key] =
      `<worksheet><sheetData>${" ".repeat(8 * 1024 * 1024)}</sheetData></worksheet>`;
  const read = await result(t, "xlsx", officeZip(entries, { compress: false }));
  assert.equal(read.blocked.code, "assistant_attachment_too_large");
  assert.equal(read.outcomes[0].details.limit, "office_expansion");
});

test("Office rejects unsafe and duplicate ZIP part names", async (t) => {
  const duplicate = officeZip({
    ...word(paragraph("first")),
    "word/zzzzzzzz.xml": "<document/>",
  });
  let at = 0;
  while (
    (at = duplicate.indexOf(Buffer.from("word/zzzzzzzz.xml"), at)) !== -1
  ) {
    duplicate.write("word/document.xml", at);
    at += 17;
  }
  for (const bytes of [
    duplicate,
    officeZip({ "../word/document.xml": "<document/>" }),
  ]) {
    const read = await result(t, "docx", bytes);
    assert.equal(read.outcomes[0].details.reason, "invalid_office");
  }
});

for (const [extension, entries] of [
  [
    "xlsx",
    { "xl/worksheets/sheet1.xml": "<worksheet><sheetData/></worksheet>" },
  ],
  ["pptx", { "ppt/slides/slide1.xml": "<sld/>" }],
])
  test(`empty ${extension} is not reported as a successfully read document`, async (t) => {
    const read = await result(t, extension, entries);
    assert.equal(read.outcomes[0].details.reason, "empty_office");
  });
