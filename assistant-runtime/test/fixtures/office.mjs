// Minimal ZIP fixture writer: stored/deflated entries with central-directory
// offsets exposed by the format so corruption tests can alter them.
import { deflateRawSync } from "node:zlib";
export function crc32(bytes) {
  let crc = 0xffffffff;
  for (const byte of bytes) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit++)
      crc = (crc >>> 1) ^ (crc & 1 ? 0xedb88320 : 0);
  }
  return (crc ^ 0xffffffff) >>> 0;
}
export function officeZip(entries, { compress = true } = {}) {
  const chunks = [],
    directory = [];
  let offset = 0;
  for (const [name, value] of Object.entries(entries)) {
    const filename = Buffer.from(name),
      raw = Buffer.from(value);
    const data = compress ? deflateRawSync(raw) : raw;
    const header = Buffer.alloc(30),
      central = Buffer.alloc(46);
    header.writeUInt32LE(0x04034b50);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(compress ? 8 : 0, 8);
    header.writeUInt32LE(crc32(raw), 14);
    header.writeUInt32LE(data.length, 18);
    header.writeUInt32LE(raw.length, 22);
    header.writeUInt16LE(filename.length, 26);
    central.writeUInt32LE(0x02014b50);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(compress ? 8 : 0, 10);
    central.writeUInt32LE(crc32(raw), 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(raw.length, 24);
    central.writeUInt16LE(filename.length, 28);
    central.writeUInt32LE(offset, 42);
    chunks.push(header, filename, data);
    directory.push(central, filename);
    offset += header.length + filename.length + data.length;
  }
  const cd = Buffer.concat(directory),
    end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50);
  end.writeUInt16LE(directory.length / 2, 8);
  end.writeUInt16LE(directory.length / 2, 10);
  end.writeUInt32LE(cd.length, 12);
  end.writeUInt32LE(offset, 16);
  return Buffer.concat([...chunks, cd, end]);
}

export const officeRelationship =
  "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
export const packageRelationship =
  "http://schemas.openxmlformats.org/package/2006/relationships";

export function reorderedPresentation() {
  return {
    "[Content_Types].xml":
      '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>',
    "_rels/.rels": `<Relationships xmlns="${packageRelationship}"><Relationship Id="document" Type="${officeRelationship}/officeDocument" Target="ppt/presentation.xml"/></Relationships>`,
    "ppt/presentation.xml": `<p:presentation xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:rel="${officeRelationship}"><p:sldIdLst><p:sldId rel:id="secondPart" id="257"/><p:sldId rel:id="firstPart" id="256"/></p:sldIdLst></p:presentation>`,
    "ppt/_rels/presentation.xml.rels": `<Relationships xmlns="${packageRelationship}"><Relationship Id="firstPart" Type="${officeRelationship}/slide" Target="slides/slide1.xml"/><Relationship Id="secondPart" Type="${officeRelationship}/slide" Target="slides/slide2.xml"/></Relationships>`,
    "ppt/slides/slide1.xml":
      '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><a:p><a:r><a:t>SECOND-PRESENTED-SLIDE</a:t></a:r></a:p></p:cSld></p:sld>',
    "ppt/slides/slide2.xml":
      '<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><a:p><a:r><a:t>FIRST-PRESENTED-SLIDE</a:t></a:r></a:p></p:cSld></p:sld>',
    "ppt/slides/_rels/slide2.xml.rels": `<Relationships xmlns="${packageRelationship}"><Relationship Id="notes" Type="${officeRelationship}/notesSlide" Target="../notesSlides/notesSlide1.xml"/></Relationships>`,
    "ppt/notesSlides/notesSlide1.xml":
      '<p:notes xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"><p:cSld><a:p><a:r><a:t>FIRST-SLIDE-NOTES</a:t></a:r></a:p></p:cSld></p:notes>',
  };
}

export function namedWorkbook() {
  return {
    "[Content_Types].xml":
      '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>',
    "_rels/.rels": `<Relationships xmlns="${packageRelationship}"><Relationship Id="document" Type="${officeRelationship}/officeDocument" Target="xl/workbook.xml"/></Relationships>`,
    "xl/workbook.xml": `<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:rel="${officeRelationship}"><sheets><sheet name="Revenue" sheetId="7" rel:id="revenue"/><sheet name="Costs" sheetId="3" rel:id="costs"/></sheets></workbook>`,
    "xl/_rels/workbook.xml.rels": `<Relationships xmlns="${packageRelationship}"><Relationship Id="revenue" Type="${officeRelationship}/worksheet" Target="worksheets/sheet7.xml"/><Relationship Id="costs" Type="${officeRelationship}/worksheet" Target="worksheets/sheet3.xml"/></Relationships>`,
    "xl/worksheets/sheet7.xml":
      '<worksheet><sheetData><row r="1"><c r="A1"><v>200</v></c></row></sheetData></worksheet>',
    "xl/worksheets/sheet3.xml":
      '<worksheet><sheetData><row r="1"><c r="A1"><v>100</v></c></row></sheetData></worksheet>',
  };
}

export function repeatedSharedWorkbook(cellCounts) {
  const entries = namedWorkbook();
  delete entries["xl/worksheets/sheet7.xml"];
  delete entries["xl/worksheets/sheet3.xml"];
  entries["xl/workbook.xml"] =
    `<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="${officeRelationship}"><sheets>${cellCounts.map((_, index) => `<sheet name="Data ${index + 1}" sheetId="${index + 1}" r:id="sheet${index + 1}"/>`).join("")}</sheets></workbook>`;
  entries["xl/_rels/workbook.xml.rels"] =
    `<Relationships xmlns="${packageRelationship}">${cellCounts.map((_, index) => `<Relationship Id="sheet${index + 1}" Type="${officeRelationship}/worksheet" Target="worksheets/sheet${index + 1}.xml"/>`).join("")}</Relationships>`;
  entries["xl/sharedStrings.xml"] =
    `<sst><si><t>${"v".repeat(65536)}</t></si></sst>`;
  for (const [index, count] of cellCounts.entries())
    entries[`xl/worksheets/sheet${index + 1}.xml`] =
      `<worksheet><sheetData>${Array.from({ length: count }, (_, row) => `<row r="${row + 1}"><c r="A${row + 1}" t="s"><v>0</v></c></row>`).join("")}</sheetData></worksheet>`;
  return entries;
}
