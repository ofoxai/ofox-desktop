import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { OfficeError, readOffice } from "../src/office.mjs";
import {
  AttachmentStore,
  prepareAttachments,
  readAttachmentTool,
} from "../src/attachments.mjs";
import {
  S,
  W,
  cellStyles,
  deflatedFill,
  namedWorkbook,
  officeZip,
  paragraph,
  presentationPackage,
  relationships,
  reorderedPresentation,
  repeatedSharedWorkbook,
  shape,
  wordPackage,
  workbookPackage,
} from "./fixtures/office.mjs";

const MiB = 1024 * 1024;
const word = (body) => wordPackage(body);
async function store(t, extension, bytes, options) {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-office-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(path.join(root, "file"), bytes);
  return new AttachmentStore(
    root,
    [{ id: "file", path: "file", name: `中文.${extension}` }],
    options,
  );
}
const bytesOf = (entries) =>
  Buffer.isBuffer(entries) ? entries : officeZip(entries);
async function result(t, extension, entries) {
  return prepareAttachments(await store(t, extension, bytesOf(entries)), [
    "file",
  ]);
}
// The whole extracted text, not only what is inlined into the turn.
async function extracted(t, extension, entries) {
  return (await store(t, extension, bytesOf(entries))).extract("file");
}
const table = (rows) =>
  `<w:tbl>${rows.map((row) => `<w:tr>${row.map((cell) => `<w:tc>${cell}</w:tc>`).join("")}</w:tr>`).join("")}</w:tbl>`;
const cells = (row, values) =>
  `<row r="${row}">${values.map(([ref, inner, attributes = ""]) => `<c r="${ref}"${attributes}>${inner}</c>`).join("")}</row>`;
// A Word body of `count` bytes of empty paragraphs after `before`: slow to
// parse (one element per few bytes) but tiny once deflated.
const slowWord = async (before = "") =>
  officeZip({
    ...word(""),
    "word/document.xml": await deflatedFill(
      `<w:document xmlns:w="${W}"><w:body>${before}`,
      '<w:p w:rsidR="00A1B2C3" w:rsidRDefault="00A1B2C3"/>',
      120 * MiB,
      "</w:body></w:document>",
    ),
  });

test("Office Word keeps Chinese paragraphs, hyperlinks and table cells in document order", async (t) => {
  const entries = word(
    "<w:p><w:r><w:t>正文</w:t></w:r><w:hyperlink><w:r><w:t>链接</w:t></w:r></w:hyperlink><w:r><w:t>末尾 &amp; &#20013;</w:t></w:r></w:p>" +
      table([[paragraph("上海"), paragraph("42")]]) +
      paragraph("结束"),
  );
  entries["word/media/image1.png"] = "not an image";
  assert.equal(
    (await extracted(t, "docx", entries)).text,
    "正文链接末尾 & 中\n上海\t42\n结束",
  );
  const read = await result(t, "docx", entries);
  assert.equal(read.blocked, null);
  assert.equal(read.images.length, 0);
  assert.equal(read.outcomes[0].status, "partial");
  assert.equal(read.outcomes[0].details.textOnly, true);
  assert.equal(read.outcomes[0].details.structureCoverage, "complete");
  assert.match(read.text, /Not read: 1 image\./);
  assert.match(read.text, /Embedded images.*not.*read/i);
});

test("Word tables put cells on one tab-separated line per row, empty and merged cells included", async (t) => {
  const merged = `<w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr>${paragraph("合计")}</w:tc><w:tc>${paragraph("9")}</w:tc></w:tr>`;
  const { text } = await extracted(
    t,
    "docx",
    word(
      paragraph("前") +
        table([
          [paragraph("城市"), paragraph("数量"), paragraph("备注")],
          [paragraph("上海"), paragraph("1") + paragraph("2"), "<w:p/>"],
          ["<w:p/>", paragraph("x"), paragraph("y")],
          [paragraph("z"), "<w:p/>", "<w:p/>"],
        ]).replace("</w:tbl>", `${merged}</w:tbl>`) +
        paragraph("后"),
    ),
  );
  assert.equal(
    text,
    "前\n城市\t数量\t备注\n上海\t1 2\n\tx\ty\nz\n合计\t\t9\n后",
  );
});

test("PowerPoint tables use the same row and cell layout", async (t) => {
  const cell = (...texts) =>
    `<a:tc><a:txBody>${texts.map((text) => `<a:p><a:r><a:t>${text}</a:t></a:r></a:p>`).join("")}</a:txBody></a:tc>`;
  const frame = `<p:graphicFrame><a:graphic><a:graphicData><a:tbl><a:tr>${cell("城市")}${cell("数量")}</a:tr><a:tr>${cell("深圳")}${cell("88", "99")}</a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>`;
  const { text } = await extracted(
    t,
    "pptx",
    presentationPackage([{ shapes: shape("标题") + frame }]),
  );
  assert.equal(text, "[Slide 1]\n标题\n城市\t数量\n深圳\t88 99");
});

test("Office workbook follows sheet names/order, sparse coordinates, shared strings and cached formulas", async (t) => {
  const entries = namedWorkbook();
  entries["xl/workbook.xml"] = entries["xl/workbook.xml"].replace(
    'name="Revenue"',
    'name="中文收入"',
  );
  entries["xl/sharedStrings.xml"] =
    `<sst xmlns="${S}"><si><t>上海</t></si></sst>`;
  entries["xl/worksheets/sheet7.xml"] =
    `<worksheet xmlns="${S}"><sheetData><row r="3"><c r="B3" t="s"><v>0</v></c><c r="D3"><f>SUM(A1:A2)</f><v>42</v></c><c r="E3"><v>7</v></c><c r="G3" t="inlineStr"><is><t>日本語</t></is></c></row><row r="4"><c r="A4" t="b"><v>1</v></c><c r="B4" t="e"><v>#DIV/0!</v></c><c r="C4"><f>NOW()</f></c></row></sheetData></worksheet>`;
  const { text } = await extracted(t, "xlsx", entries);
  assert.equal(
    text,
    '[Sheet 1: "中文收入"]\nB3: 上海\tD3: 42 [=SUM(A1:A2)]\t7\tG3: 日本語\nA4: TRUE\t#DIV/0!\t=NOW()\n\n[Sheet 2: "Costs"]\nA1: 100',
  );
  const read = await result(t, "xlsx", entries);
  assert.equal(read.outcomes[0].details.structureCoverage, "complete");
  assert.match(read.text, /ISO dates/);
});

test("Excel dates become ISO dates by their cell style, in the 1900 and 1904 systems", async (t) => {
  const styles = cellStyles([0, 14, 22, 21, 164, 165, 4, 46, 166], {
    164: "yyyy/mm/dd",
    165: "[$-409]h:mm AM/PM;@",
    166: '0.00 "days"',
  });
  const row = cells(1, [
    ["A1", "<v>46323</v>", ' s="1"'],
    ["B1", "<v>46323.5</v>", ' s="2"'],
    ["C1", "<v>0.75</v>", ' s="3"'],
    ["D1", "<v>46323</v>", ' s="4"'],
    ["E1", "<v>0.25</v>", ' s="5"'],
    ["F1", "<v>12345.678</v>", ' s="6"'],
    ["G1", "<v>1.5</v>", ' s="7"'],
    ["H1", "<v>46323</v>", ' s="8"'],
    ["I1", "<v>46323</v>"],
  ]);
  const { text } = await extracted(
    t,
    "xlsx",
    workbookPackage({ sheets: [{ name: "日期", rows: row }], styles }),
  );
  assert.equal(
    text,
    '[Sheet 1: "日期"]\nA1: 2026-10-28\t2026-10-28 12:00:00\t18:00:00\t2026-10-28\t06:00:00\t12345.678\t36:00:00\t46323\t46323',
  );
  const dated = (serials, date1904) =>
    extracted(
      t,
      "xlsx",
      workbookPackage({
        sheets: [
          {
            name: "D",
            rows: cells(
              1,
              serials.map((serial, index) => [
                `${"AB"[index]}1`,
                `<v>${serial}</v>`,
                ' s="1"',
              ]),
            ),
          },
        ],
        styles: cellStyles([0, 14]),
        date1904,
      }),
    );
  assert.match((await dated([1, 61])).text, /A1: 1900-01-01\t1900-03-01/);
  assert.match(
    (await dated([0, 44861], true)).text,
    /A1: 1904-01-01\t2026-10-28/,
  );
});

test("pathological number format codes are not dates and do not stall the reader", async (t) => {
  const workbook = (styles) => {
    const entries = workbookPackage({
      sheets: [
        { name: "D", rows: cells(1, [["A1", "<v>46323</v>", ' s="0"']]) },
      ],
      styles: "placeholder",
    });
    entries["xl/styles.xml"] = styles;
    return entries;
  };
  // One 30 MiB code of unclosed brackets.
  const long = await deflatedFill(
    `<styleSheet xmlns="${S}"><numFmts><numFmt numFmtId="164" formatCode="`,
    "[h",
    30 * MiB,
    '"/></numFmts><cellXfs><xf numFmtId="164"/></cellXfs></styleSheet>',
  );
  // A 200,000-character code used by 65,536 cell formats.
  const shared = await deflatedFill(
    `<styleSheet xmlns="${S}"><numFmts><numFmt numFmtId="164" formatCode="${"[h".repeat(100000)}"/></numFmts><cellXfs>`,
    '<xf numFmtId="164"/>',
    65536 * 19,
    "</cellXfs></styleSheet>",
  );
  for (const styles of [long, shared]) {
    const started = Date.now();
    const office = await readOffice(officeZip(workbook(styles)), ".xlsx");
    assert.equal(office.text, '[Sheet 1: "D"]\nA1: 46323');
    assert.equal(office.truncated, false);
    assert.ok(Date.now() - started < 3000, `${Date.now() - started} ms`);
  }
});

test("hidden sheets and chart sheets are labelled; chart sheets are not a missing part", async (t) => {
  const entries = workbookPackage({
    sheets: [
      { name: "Visible", rows: cells(1, [["A1", "<v>1</v>"]]) },
      { name: "Secret", state: "hidden", rows: cells(1, [["A1", "<v>2</v>"]]) },
      {
        name: "Deep",
        state: "veryHidden",
        rows: cells(1, [["A1", "<v>3</v>"]]),
      },
      { name: "Chart1", kind: "chartsheet" },
      { name: "Blank" },
    ],
  });
  const { text, structureCoverage } = await extracted(t, "xlsx", entries);
  assert.equal(
    text,
    '[Sheet 1: "Visible"]\nA1: 1\n\n[Sheet 2: "Secret" (hidden)]\nA1: 2\n\n[Sheet 3: "Deep" (veryHidden)]\nA1: 3\n\n[Sheet 4: "Chart1"]\n(chart sheet: not read)\n\n[Sheet 5: "Blank"]\n(empty)',
  );
  assert.equal(structureCoverage, "complete");
});

test("shared formulas show their cached value marked as shared", async (t) => {
  const rows =
    cells(1, [
      ["A1", "<v>5</v>"],
      ["B1", '<f t="shared" ref="B1:B2" si="0">A1+1</f><v>6</v>'],
    ]) +
    cells(2, [
      ["A2", "<v>6</v>"],
      ["B2", '<f t="shared" si="0"/><v>7</v>'],
    ]);
  const { text } = await extracted(
    t,
    "xlsx",
    workbookPackage({ sheets: [{ name: "F", rows }] }),
  );
  assert.equal(
    text,
    '[Sheet 1: "F"]\nA1: 5\t6 [=A1+1]\nA2: 6\t7 [shared formula]',
  );
});

test("Office presentations follow actual slide order and attach notes to their slide", async (t) => {
  const entries = reorderedPresentation();
  entries["ppt/slides/slide2.xml"] = entries["ppt/slides/slide2.xml"].replace(
    "FIRST-PRESENTED-SLIDE",
    "中文第一页",
  );
  const { text } = await extracted(t, "pptx", entries);
  assert.equal(
    text,
    "[Slide 1]\n中文第一页\n\n[Notes for slide 1]\nFIRST-SLIDE-NOTES\n\n[Slide 2]\nSECOND-PRESENTED-SLIDE",
  );
});

test("hidden slides are labelled and still read", async (t) => {
  const { text } = await extracted(
    t,
    "pptx",
    presentationPackage([
      { shapes: shape("一") },
      { shapes: shape("二"), show: "0", notes: shape("备注") },
    ]),
  );
  assert.equal(
    text,
    "[Slide 1]\n一\n\n[Slide 2 (hidden)]\n二\n\n[Notes for slide 2]\n备注",
  );
});

test("hidden Word text is not read; text whose vanish is switched off is", async (t) => {
  const { text } = await extracted(
    t,
    "docx",
    word(
      '<w:p><w:r><w:t>可见</w:t></w:r><w:r><w:rPr><w:vanish/></w:rPr><w:t>隐藏</w:t></w:r><w:r><w:rPr><w:vanish w:val="0"/></w:rPr><w:t>显示</w:t></w:r></w:p>' +
        "<w:p><w:pPr><w:rPr><w:vanish/></w:rPr></w:pPr><w:r><w:t>段落</w:t></w:r></w:p>" +
        // A hidden run stays hidden after a text box inside it ends.
        `<w:p><w:r><w:rPr><w:vanish/></w:rPr><w:pict><w:txbxContent>${paragraph("框内")}</w:txbxContent></w:pict><w:t>框后</w:t></w:r><w:r><w:t>末</w:t></w:r></w:p>`,
    ),
  );
  assert.equal(text, "可见显示\n段落\n末");
});

test("tracked deletions, moved-from text and field codes are skipped; insertions are read", async (t) => {
  const { text } = await extracted(
    t,
    "docx",
    word(
      "<w:p><w:r><w:t>保留</w:t></w:r><w:del><w:r><w:delText>删除</w:delText></w:r></w:del><w:ins><w:r><w:t>插入</w:t></w:r></w:ins></w:p>" +
        "<w:p><w:moveFrom><w:r><w:t>移走</w:t></w:r></w:moveFrom><w:r><w:t>中间</w:t></w:r><w:moveTo><w:r><w:t>移来</w:t></w:r></w:moveTo></w:p>" +
        '<w:p><w:r><w:t>第</w:t></w:r><w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText> PAGE </w:instrText></w:r><w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>3</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r><w:r><w:t>页</w:t></w:r></w:p>',
    ),
  );
  assert.equal(text, "保留插入\n中间移来\n第3页");
});

test("Word compatibility alternatives are read once: the choice, or the fallback when the choice has no text", async (t) => {
  const alternative = (choice, fallback) =>
    `<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps">${choice}</mc:Choice><mc:Fallback>${fallback}</mc:Fallback></mc:AlternateContent></w:r></w:p>`;
  const box = (text) => `<w:txbxContent>${paragraph(text)}</w:txbxContent>`;
  const { text } = await extracted(
    t,
    "docx",
    word(
      alternative(box("文本框正文"), box("文本框正文")) +
        alternative("<w:drawing/>", box("兼容文本")),
    ),
  );
  assert.equal(text, "文本框正文\n\n兼容文本");
});

test("footnotes, endnotes and text boxes are read; headers are not", async (t) => {
  const entries = word(
    '<w:p><w:r><w:t>正文</w:t></w:r><w:r><w:footnoteReference w:id="1"/></w:r><w:r><w:endnoteReference w:id="1"/></w:r></w:p>' +
      `<w:p><w:r><w:pict><v:shape xmlns:v="urn:schemas-microsoft-com:vml"><v:textbox><w:txbxContent>${paragraph("文本框")}</w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>`,
  );
  entries["word/_rels/document.xml.rels"] = relationships([
    ["f", "footnotes", "footnotes.xml"],
    ["e", "endnotes", "endnotes.xml"],
    ["h", "header", "header1.xml"],
  ]);
  entries["word/footnotes.xml"] =
    `<w:footnotes xmlns:w="${W}"><w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote><w:footnote w:id="1">${paragraph("脚注内容")}</w:footnote></w:footnotes>`;
  entries["word/endnotes.xml"] =
    `<w:endnotes xmlns:w="${W}"><w:endnote w:id="1">${paragraph("尾注内容")}</w:endnote></w:endnotes>`;
  entries["word/header1.xml"] =
    `<w:hdr xmlns:w="${W}">${paragraph("页眉")}</w:hdr>`;
  const { text } = await extracted(t, "docx", entries);
  assert.equal(
    text,
    "正文[^1][^e1]\n文本框\n\n[Footnotes]\n[^1]: 脚注内容\n\n[Endnotes]\n[^e1]: 尾注内容",
  );
});

test("UTF-16 parts are decoded by their byte order mark", async (t) => {
  const xml = `<?xml version="1.0" encoding="UTF-16"?><w:document xmlns:w="${W}"><w:body>${paragraph("统一码文本")}</w:body></w:document>`;
  const little = Buffer.concat([
    Buffer.from([0xff, 0xfe]),
    Buffer.from(xml, "utf16le"),
  ]);
  const big = Buffer.from(little).swap16();
  for (const encoded of [little, big]) {
    const entries = word("");
    entries["word/document.xml"] = encoded;
    assert.equal((await extracted(t, "docx", entries)).text, "统一码文本");
  }
});

test("a missing sheet relationship is reported as partial structure; a workbook without any is invalid", async (t) => {
  const entries = namedWorkbook();
  entries["xl/_rels/workbook.xml.rels"] = relationships([
    ["costs", "worksheet", "worksheets/sheet3.xml"],
  ]);
  const read = await result(t, "xlsx", entries);
  assert.equal(read.blocked, null);
  assert.match(read.text, /\[Sheet 1: "Revenue"\]\n\(not readable\)/);
  assert.match(read.text, /\[Sheet 2: "Costs"\]\nA1: 100/);
  assert.equal(read.outcomes[0].details.structureCoverage, "partial");
  delete entries["xl/_rels/workbook.xml.rels"];
  const none = await result(t, "xlsx", entries);
  assert.equal(none.outcomes[0].details.reason, "invalid_office");
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
  assert.match(read.text, /\[Sheet 1: "Revenue"\]\n\(not readable\)/);
  assert.doesNotMatch(read.text, /200|example.invalid/);
  assert.equal(read.outcomes[0].details.structureCoverage, "partial");
});

for (const [name, entries, reason] of [
  ["damaged ZIP", Buffer.from("PKbroken"), "invalid_office"],
  ["a file that is not a ZIP", Buffer.from("plain text"), "invalid_office"],
  [
    "password-protected Office container",
    Buffer.concat([
      Buffer.from("d0cf11e0a1b11ae1", "hex"),
      Buffer.from("EncryptedPackage", "utf16le"),
    ]),
    "encrypted_office",
  ],
  [
    "legacy binary file named docx",
    Buffer.concat([
      Buffer.from("d0cf11e0a1b11ae1", "hex"),
      Buffer.from("WordDocument", "utf16le"),
    ]),
    "invalid_office",
  ],
  ["empty Word document", word(""), "empty_office"],
  [
    "malformed XML",
    {
      ...word(""),
      "word/document.xml": `<w:document xmlns:w="${W}"><w:p></w:document>`,
    },
    "invalid_office",
  ],
  [
    "entity declaration",
    {
      ...word(""),
      "word/document.xml": `<!DOCTYPE document [<!ENTITY x SYSTEM "file:///etc/passwd">]><w:document xmlns:w="${W}"><w:p><w:r><w:t>&x;</w:t></w:r></w:p></w:document>`,
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
    if (reason !== "encrypted_office")
      assert.doesNotMatch(read.blocked.message, /unencrypted|password/i);
  });

for (const [name, entries] of [
  [
    "60,000 entries",
    Object.fromEntries(
      Array.from({ length: 60000 }, (_, i) => [`part${i}`, ""]),
    ),
  ],
  [
    "deep XML",
    word("<w:sdt>".repeat(300) + paragraph("text") + "</w:sdt>".repeat(300)),
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
    // The last central record is word/document.xml's.
    const central = bytes.lastIndexOf(Buffer.from("504b0102", "hex"));
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

test("Office reads packages with bytes after the end of the central directory", async (t) => {
  for (const tail of ["\n", "\r\n"]) {
    const bytes = Buffer.concat([
      officeZip(word(paragraph("尾随字节"))),
      Buffer.from(tail),
    ]);
    const read = await result(t, "docx", bytes);
    assert.equal(read.blocked, null, JSON.stringify(tail));
    assert.match(read.text, /尾随字节/);
  }
});

test("a realistic 10,000-row by 6-column workbook is read in full", async (t) => {
  const names = ["编号", "名称", "城市", "金额", "日期", "合计"];
  const cities = ["上海", "北京", "深圳", "广州", "杭州"];
  const rows = [
    cells(
      1,
      names.map((_, index) => [
        `${"ABCDEF"[index]}1`,
        `<v>${index}</v>`,
        ' t="s"',
      ]),
    ),
  ];
  for (let row = 2; row <= 10001; row++)
    rows.push(
      cells(row, [
        [`A${row}`, `<v>${row - 1}</v>`],
        [`B${row}`, `<is><t>产品 ${row - 1}</t></is>`, ' t="inlineStr"'],
        [`C${row}`, `<v>${names.length + (row % cities.length)}</v>`, ' t="s"'],
        [`D${row}`, `<v>${(row * 1.25).toFixed(2)}</v>`],
        [`E${row}`, `<v>${45658 + (row % 365)}</v>`, ' s="1"'],
        [`F${row}`, `<f>D${row}*2</f><v>${row * 2.5}</v>`],
      ]),
    );
  const entries = workbookPackage({
    sheets: [{ name: "销售", rows: rows.join("") }],
    sharedStrings: [...names, ...cities],
    styles: cellStyles([0, 14]),
  });
  const read = await result(t, "xlsx", entries);
  assert.equal(read.blocked, null);
  const { details } = read.outcomes[0];
  assert.equal(details.truncated, false);
  assert.equal(details.structureCoverage, "complete");
  const { text } = await extracted(t, "xlsx", entries);
  assert.ok(
    text.startsWith(
      '[Sheet 1: "销售"]\nA1: 编号\t名称\t城市\t金额\t日期\t合计\nA2: 1\t产品 1\t深圳\t2.50\t2025-01-03\t5 [=D2*2]\n',
    ),
    text.slice(0, 200),
  );
  assert.ok(
    text.endsWith(
      "\nA10001: 10000\t产品 10000\t北京\t12501.25\t2025-05-27\t25002.5 [=D10001*2]",
    ),
    text.slice(-200),
  );
});

test("Office shared-string expansion is bounded and reported as truncated", async (t) => {
  // 600 cells of one 65,536-character string, each cut to Excel's 32,767.
  const read = await result(t, "xlsx", repeatedSharedWorkbook([300, 300]));
  assert.equal(read.blocked, null);
  assert.equal(read.outcomes[0].status, "partial");
  assert.equal(read.outcomes[0].details.truncated, true);
  assert.equal(read.outcomes[0].details.characters, 16 * 1024 * 1024);
});

test("sheets that repeat one part are read once", async (t) => {
  const entries = workbookPackage({
    sheets: [
      {
        name: "Data",
        rows: Array.from({ length: 1000 }, (_, row) =>
          cells(row + 1, [[`A${row + 1}`, `<v>${row}</v>`]]),
        ).join(""),
      },
    ],
  });
  entries["xl/workbook.xml"] = entries["xl/workbook.xml"].replace(
    "</sheets>",
    `${'<sheet name="Copy" sheetId="2" r:id="rId1"/>'.repeat(149999)}</sheets>`,
  );
  const started = Date.now();
  const { text } = await extracted(t, "xlsx", entries);
  assert.ok(Date.now() - started < 5000, `${Date.now() - started} ms`);
  assert.match(text, /\[Sheet 2: "Copy"\]\n\(same part as Sheet 1\)/);
  assert.match(text, /\[Sheet 150000: "Copy"\]\n\(same part as Sheet 1\)$/);
  assert.ok(text.length < 10 * MiB, `${text.length}`);
});

test("sheets naming one part in other letter cases are read once", async (t) => {
  const path = "worksheets/sheet1.xml";
  const letters = [...path].flatMap((char, at) =>
    /[a-z]/.test(char) ? [at] : [],
  );
  const variants = Array.from({ length: 20000 }, (_, n) =>
    [...path]
      .map((char, at) => {
        const bit = letters.indexOf(at);
        return bit >= 0 && (n >> bit) & 1 ? char.toUpperCase() : char;
      })
      .join(""),
  );
  const entries = workbookPackage({
    sheets: variants.map((target, index) => ({
      name: `S${index + 1}`,
      target,
      rows: cells(1, [["A1", "<is><t>唯一内容</t></is>", ' t="inlineStr"']]),
    })),
  });
  for (const variant of variants.slice(1)) delete entries[`xl/${variant}`];
  const started = Date.now();
  const { text } = await extracted(t, "xlsx", entries);
  assert.ok(Date.now() - started < 5000, `${Date.now() - started} ms`);
  assert.equal(text.split("唯一内容").length - 1, 1);
  assert.match(text, /\[Sheet 2: "S2"\]\n\(same part as Sheet 1\)/);
});

const bombBody = (fill, bytes) =>
  deflatedFill(
    `<w:document xmlns:w="${W}"><w:body>${paragraph("开头")}`,
    fill,
    bytes,
    "</w:body></w:document>",
  );

test("an honest zip bomb is read up to the inflate budget and reported as partial", async (t) => {
  const bomb = await bombBody(" ", 200 * MiB);
  const started = Date.now();
  const read = await result(t, "docx", {
    ...word(""),
    "word/document.xml": bomb,
  });
  assert.equal(read.blocked, null);
  assert.equal(read.outcomes[0].status, "partial");
  assert.equal(read.outcomes[0].details.truncated, true);
  assert.equal(read.outcomes[0].details.extractionLimit, 2);
  assert.match(read.text, /开头/);
  assert.ok(Date.now() - started < 8000, `${Date.now() - started} ms`);
});

test("a zip bomb that lies about its size is rejected as damaged as soon as it overflows", async (t) => {
  const bomb = await bombBody(" ", 200 * MiB);
  const started = Date.now();
  const read = await result(t, "docx", {
    ...word(""),
    "word/document.xml": { ...bomb, size: 1024 },
  });
  assert.equal(read.outcomes[0].details.reason, "invalid_office");
  assert.ok(Date.now() - started < 3000, `${Date.now() - started} ms`);
});

test("one text run longer than the text limit is cut at the limit", async (t) => {
  const run = await deflatedFill(
    `<w:document xmlns:w="${W}"><w:body><w:p><w:r><w:t>`,
    "长",
    60 * MiB,
    "</w:t></w:r></w:p></w:body></w:document>",
  );
  const read = await result(t, "docx", {
    ...word(""),
    "word/document.xml": run,
  });
  assert.equal(read.blocked, null);
  assert.equal(read.outcomes[0].details.truncated, true);
  assert.equal(read.outcomes[0].details.characters, 16 * MiB);
  assert.equal(read.outcomes[0].details.extractionLimit, 16 * MiB);
});

test("the inflate budget is shared by all parts: later sheets end partial, not as a failure", async (t) => {
  const filler = (value) =>
    deflatedFill(
      `<worksheet xmlns="${S}"><sheetData><row r="1"><c r="A1"><v>${value}</v></c></row>`,
      " ",
      80 * MiB,
      "</sheetData></worksheet>",
    );
  const entries = workbookPackage({
    sheets: [{ name: "One" }, { name: "Two" }, { name: "Three" }],
  });
  entries["xl/worksheets/sheet1.xml"] = await filler(1);
  entries["xl/worksheets/sheet2.xml"] = await filler(2);
  entries["xl/worksheets/sheet3.xml"] = await filler(3);
  const read = await result(t, "xlsx", entries);
  assert.equal(read.blocked, null);
  assert.equal(read.outcomes[0].details.truncated, true);
  assert.match(
    read.text,
    /\[Sheet 1: "One"\]\nA1: 1\n\n\[Sheet 2: "Two"\]\nA1: 2/,
  );
  assert.doesNotMatch(read.text, /Sheet 3/);
});

test("a document that parses too slowly ends partial at the reader's deadline", async () => {
  // Reading all of it takes longer than 1.5 s; the reader would be killed
  // after 2 s and the read reported as too large.
  const office = await readOffice(
    await slowWord(paragraph("开头")),
    ".docx",
    undefined,
    { timeout: 2000 },
  );
  assert.equal(office.text, "开头");
  assert.equal(office.truncated, true);
});

for (const [name, fixture] of [
  ["does not answer in time", "office-child-hangs.mjs"],
  ["stops without answering", "office-child-dies.mjs"],
])
  test(`a reader that ${name} is reported as too large`, async () => {
    await assert.rejects(
      readOffice(officeZip(word(paragraph("x"))), ".docx", undefined, {
        timeout: 1000,
        childModule: fileURLToPath(
          new URL(`./fixtures/${fixture}`, import.meta.url),
        ),
      }),
      (error) =>
        error instanceof OfficeError && error.reason === "office_expansion",
    );
  });

test("a reader that never reports ready is a start-up failure, not a document outcome", async () => {
  await assert.rejects(
    readOffice(officeZip(word(paragraph("x"))), ".docx", undefined, {
      timeout: 1000,
      childModule: fileURLToPath(
        new URL("./fixtures/office-child-silent.mjs", import.meta.url),
      ),
    }),
    (error) =>
      !(error instanceof OfficeError) && /could not start/.test(error.message),
  );
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

test("stopping a running Office read kills its process promptly", async () => {
  const controller = new AbortController();
  let child;
  const pending = readOffice(await slowWord(), ".docx", controller.signal, {
    onChild: (process) => (child = process),
  });
  await new Promise((resolve) => setTimeout(resolve, 500));
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const started = Date.now();
  controller.abort(new Error("stop the reader"));
  await assert.rejects(pending, /stop the reader/);
  await exited;
  assert.ok(Date.now() - started < 1000, `${Date.now() - started} ms`);
  assert.notEqual(child.exitCode ?? child.signalCode, null);
});

test("the reader's heap stays capped when the sidecar runs with a larger heap", () => {
  const officeModule = new URL("../src/office.mjs", import.meta.url).href;
  const fixtures = new URL("./fixtures/office.mjs", import.meta.url).href;
  const output = execFileSync(
    process.execPath,
    [
      "--max-old-space-size=512",
      "--input-type=module",
      "--eval",
      `import v8 from "node:v8";
       import { readOffice } from ${JSON.stringify(officeModule)};
       import { officeZip, wordPackage, paragraph } from ${JSON.stringify(fixtures)};
       const result = await readOffice(officeZip(wordPackage(paragraph("堆"))), ".docx");
       console.log(JSON.stringify({ parent: v8.getHeapStatistics().heap_size_limit, child: result.heapLimit, text: result.text }));`,
    ],
    { encoding: "utf8", env: {} },
  );
  const { parent, child, text } = JSON.parse(output);
  assert.equal(text, "堆");
  assert.ok(parent > 500 * MiB, `${parent}`);
  assert.ok(child <= 200 * MiB, `${child}`);
});

test("a reader that cannot start is a runtime failure, not a damaged document", async (t) => {
  const childModule = fileURLToPath(
    new URL("./fixtures/office-child-fails.mjs", import.meta.url),
  );
  await assert.rejects(
    readOffice(officeZip(word(paragraph("x"))), ".docx", undefined, {
      childModule,
    }),
    (error) =>
      !(error instanceof OfficeError) && /could not start/.test(error.message),
  );
  const reader = await store(t, "docx", officeZip(word(paragraph("x"))), {
    readOffice: (bytes, extension, signal) =>
      readOffice(bytes, extension, signal, { childModule }),
  });
  await assert.rejects(
    prepareAttachments(reader, ["file"]),
    /Office reader could not start/,
  );
});

test("old .doc stays unsupported with a conversion hint", async (t) => {
  const read = await result(t, "doc", Buffer.from("old doc"));
  assert.equal(read.outcomes[0].status, "unsupported");
  assert.match(read.blocked.message, /docx or PDF/i);
});

for (const [format, expected] of [
  [
    "docx",
    [
      "星河项目季度报告",
      "中文正文：本季度完成验收",
      "城市\t数量\n上海\t42",
      "星河归档",
    ],
  ],
  [
    "xlsx",
    [
      '[Sheet 1: "中文收入"]',
      "A2: 上海\t12345.67",
      "D4: 12345.67 [=SUM(B2:B3)]",
      '[Sheet 2: "日文成本"]',
      "B3: 東京支出",
    ],
  ],
  [
    "pptx",
    [
      "[Slide 1]\n中文第一页",
      "中文第二页",
      "提升质量",
      "全部通过",
      "[Notes for slide 1]\n讲者备注",
      "城市\t数量\n深圳\t88",
    ],
  ],
])
  test(`real Chinese ${format} generated by an Office writer is readable`, async (t) => {
    const bytes = await readFile(
      new URL(`./fixtures/office/chinese.${format}`, import.meta.url),
    );
    const { text, structureCoverage } = await extracted(t, format, bytes);
    for (const marker of expected) assert.ok(text.includes(marker), marker);
    assert.equal(structureCoverage, "complete");
  });

test("spreadsheet phonetic guides do not change a cell's visible value", async (t) => {
  const entries = namedWorkbook();
  entries["xl/sharedStrings.xml"] =
    `<sst xmlns="${S}"><si><t>東京都</t><rPh sb="0" eb="3"><t>トウキョウト</t></rPh></si></sst>`;
  entries["xl/worksheets/sheet7.xml"] =
    `<worksheet xmlns="${S}"><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="inlineStr"><is><t>上海</t><rPh sb="0" eb="2"><t>shanghai</t></rPh></is></c></row></sheetData></worksheet>`;
  const read = await result(t, "xlsx", entries);
  assert.match(read.text, /A1: 東京都\t上海/);
  assert.doesNotMatch(read.text, /トウキョウト|shanghai/);
});

test("embedded pictures, charts and objects are counted as not read", async (t) => {
  const entries = word(paragraph("正文"));
  entries["word/media/image1.png"] = "x";
  entries["word/media/image2.jpeg"] = "x";
  entries["word/charts/chart1.xml"] = "<c/>";
  entries["word/charts/colors1.xml"] = "<c/>";
  entries["word/embeddings/sheet.xlsx"] = "x";
  const read = await result(t, "docx", entries);
  assert.match(read.text, /Not read: 2 images, 1 chart, 1 embedded object\./);
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
    officeZip({ ...word(paragraph("x")), "../word/document.xml": "<a/>" }),
  ]) {
    const read = await result(t, "docx", bytes);
    assert.equal(read.outcomes[0].details.reason, "invalid_office");
  }
});

for (const [extension, entries] of [
  ["xlsx", workbookPackage({ sheets: [{ name: "Empty" }] })],
  ["pptx", presentationPackage([{ shapes: "" }])],
])
  test(`empty ${extension} is not reported as a successfully read document`, async (t) => {
    const read = await result(t, extension, entries);
    assert.equal(read.outcomes[0].details.reason, "empty_office");
    assert.doesNotMatch(read.blocked.message, /unencrypted/);
  });

test("the model is told how Office files are read", () => {
  const tool = readAttachmentTool(
    { wrapTool: (_context, name, description) => ({ name, description }) },
    { attachments: new AttachmentStore("/", []) },
    { vision: false },
  );
  const guidelines = tool.promptGuidelines.join("\n");
  for (const fact of [
    /\[Sheet N: "name"\]/,
    /\[Slide N\]/,
    /hidden/i,
    /ISO dates/,
    /other numbers/i,
    /pictures/i,
    /hidden rows, columns and shapes are read without a label/i,
    /\[shared formula\]/,
  ])
    assert.match(guidelines, fact);
  assert.match(tool.description, /Word, Excel and PowerPoint/);
});
