// Minimal ZIP fixture writer: stored/deflated entries with central-directory
// offsets exposed by the format so corruption tests can alter them, and
// builders for namespaced Word, Excel and PowerPoint packages.
import { createDeflateRaw, crc32, deflateRawSync } from "node:zlib";

/**
 * Entries map names to text or bytes (deflated unless `compress` is false),
 * or to a prepared `{data, crc, size, method}` such as `deflatedFill` builds.
 */
export function officeZip(entries, { compress = true } = {}) {
  const chunks = [],
    directory = [];
  let offset = 0;
  for (const [name, value] of Object.entries(entries)) {
    const filename = Buffer.from(name);
    let prepared = value;
    if (value?.data === undefined) {
      const raw = Buffer.from(value);
      prepared = {
        data: compress ? deflateRawSync(raw) : raw,
        crc: crc32(raw),
        size: raw.length,
        method: compress ? 8 : 0,
      };
    }
    const { data, crc, size, method } = prepared;
    const header = Buffer.alloc(30),
      central = Buffer.alloc(46);
    header.writeUInt32LE(0x04034b50);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(method, 8);
    header.writeUInt32LE(crc, 14);
    header.writeUInt32LE(data.length, 18);
    header.writeUInt32LE(size, 22);
    header.writeUInt16LE(filename.length, 26);
    central.writeUInt32LE(0x02014b50);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(method, 10);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(size, 24);
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

/**
 * A deflated part of `prefix`, `fill` repeated whole for about `fillBytes`
 * bytes, and `suffix`, built in pieces so a zip bomb never exists
 * uncompressed in the test. `size` is the true inflated size; a lying entry
 * may misstate it.
 */
export async function deflatedFill(prefix, fill, fillBytes, suffix) {
  const deflate = createDeflateRaw({ level: 9 });
  const output = [];
  deflate.on("data", (chunk) => output.push(chunk));
  let crc = 0,
    size = 0;
  const write = (bytes) => {
    deflate.write(bytes);
    crc = crc32(bytes, crc);
    size += bytes.length;
  };
  write(Buffer.from(prefix));
  const one = Buffer.byteLength(fill);
  const block = Buffer.from(
    fill.repeat(Math.max(1, Math.floor(2 ** 20 / one))),
  );
  let left = fillBytes;
  for (; left >= block.length; left -= block.length) write(block);
  write(Buffer.from(fill.repeat(Math.floor(left / one))));
  write(Buffer.from(suffix));
  await new Promise((resolve) => deflate.end(resolve));
  return { data: Buffer.concat(output), crc, size, method: 8 };
}

export const officeRelationship =
  "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
export const packageRelationship =
  "http://schemas.openxmlformats.org/package/2006/relationships";
export const W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
export const A = "http://schemas.openxmlformats.org/drawingml/2006/main";
export const P = "http://schemas.openxmlformats.org/presentationml/2006/main";
export const S = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
export const MC = "http://schemas.openxmlformats.org/markup-compatibility/2006";

const contentTypes =
  '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/></Types>';

/** A .rels part from [id, type, target, targetMode?] rows. */
export const relationships = (rows) =>
  `<Relationships xmlns="${packageRelationship}">${rows
    .map(
      ([id, type, target, mode]) =>
        `<Relationship Id="${id}" Type="${officeRelationship}/${type}" Target="${target}"${mode ? ` TargetMode="${mode}"` : ""}/>`,
    )
    .join("")}</Relationships>`;

const rootRelationships = (target) =>
  relationships([["document", "officeDocument", target]]);

/** A Word package; `parts` adds or replaces parts such as footnotes. */
export function wordPackage(body, parts = {}) {
  return {
    "[Content_Types].xml": contentTypes,
    "_rels/.rels": rootRelationships("word/document.xml"),
    "word/document.xml": `<w:document xmlns:w="${W}" xmlns:mc="${MC}" xmlns:r="${officeRelationship}"><w:body>${body}</w:body></w:document>`,
    ...parts,
  };
}
export const paragraph = (text) => `<w:p><w:r><w:t>${text}</w:t></w:r></w:p>`;

/**
 * An Excel package. Each sheet is {name, rows (sheetData XML), state?,
 * kind? ("worksheet" or "chartsheet"), target?}; sharedStrings is a list of
 * texts or the part's XML.
 */
export function workbookPackage({
  sheets,
  sharedStrings,
  styles,
  date1904 = false,
}) {
  const entries = {
    "[Content_Types].xml": contentTypes,
    "_rels/.rels": rootRelationships("xl/workbook.xml"),
    "xl/workbook.xml": `<workbook xmlns="${S}" xmlns:r="${officeRelationship}">${date1904 ? '<workbookPr date1904="1"/>' : ""}<sheets>${sheets
      .map(
        (sheet, index) =>
          `<sheet name="${sheet.name}" sheetId="${index + 1}"${sheet.state ? ` state="${sheet.state}"` : ""} r:id="rId${index + 1}"/>`,
      )
      .join("")}</sheets></workbook>`,
  };
  const rows = sheets.map((sheet, index) => {
    const kind = sheet.kind ?? "worksheet";
    const target =
      sheet.target ??
      (kind === "chartsheet"
        ? `chartsheets/sheet${index + 1}.xml`
        : `worksheets/sheet${index + 1}.xml`);
    entries[`xl/${target}`] ??=
      kind === "chartsheet"
        ? `<chartsheet xmlns="${S}"/>`
        : `<worksheet xmlns="${S}"><sheetData>${sheet.rows ?? ""}</sheetData></worksheet>`;
    return [`rId${index + 1}`, kind, target];
  });
  if (sharedStrings) {
    rows.push(["strings", "sharedStrings", "sharedStrings.xml"]);
    entries["xl/sharedStrings.xml"] = Array.isArray(sharedStrings)
      ? `<sst xmlns="${S}">${sharedStrings.map((text) => `<si><t>${text}</t></si>`).join("")}</sst>`
      : sharedStrings;
  }
  if (styles) {
    rows.push(["styles", "styles", "styles.xml"]);
    entries["xl/styles.xml"] = styles;
  }
  entries["xl/_rels/workbook.xml.rels"] = relationships(rows);
  return entries;
}

/** cellXfs styles from numFmtIds, with custom {id: code} number formats. */
export const cellStyles = (numFmtIds, formats = {}) =>
  `<styleSheet xmlns="${S}"><numFmts>${Object.entries(formats)
    .map(
      ([id, code]) =>
        `<numFmt numFmtId="${id}" formatCode="${code.replace(/"/g, "&quot;")}"/>`,
    )
    .join("")}</numFmts><cellStyleXfs><xf numFmtId="14"/></cellStyleXfs>` +
  `<cellXfs>${numFmtIds.map((id) => `<xf numFmtId="${id}"/>`).join("")}</cellXfs></styleSheet>`;

/** A PowerPoint package; each slide is {shapes (spTree XML), show?, notes?}. */
export function presentationPackage(slides) {
  const entries = {
    "[Content_Types].xml": contentTypes,
    "_rels/.rels": rootRelationships("ppt/presentation.xml"),
    "ppt/presentation.xml": `<p:presentation xmlns:p="${P}" xmlns:r="${officeRelationship}"><p:sldIdLst>${slides
      .map(
        (_, index) => `<p:sldId id="${256 + index}" r:id="rId${index + 1}"/>`,
      )
      .join("")}</p:sldIdLst></p:presentation>`,
    "ppt/_rels/presentation.xml.rels": relationships(
      slides.map((_, index) => [
        `rId${index + 1}`,
        "slide",
        `slides/slide${index + 1}.xml`,
      ]),
    ),
  };
  slides.forEach((slide, index) => {
    const show = slide.show === undefined ? "" : ` show="${slide.show}"`;
    entries[`ppt/slides/slide${index + 1}.xml`] =
      `<p:sld xmlns:p="${P}" xmlns:a="${A}"${show}><p:cSld><p:spTree>${slide.shapes}</p:spTree></p:cSld></p:sld>`;
    if (slide.notes) {
      entries[`ppt/slides/_rels/slide${index + 1}.xml.rels`] = relationships([
        ["notes", "notesSlide", `../notesSlides/notesSlide${index + 1}.xml`],
      ]);
      entries[`ppt/notesSlides/notesSlide${index + 1}.xml`] =
        `<p:notes xmlns:p="${P}" xmlns:a="${A}"><p:cSld><p:spTree>${slide.notes}</p:spTree></p:cSld></p:notes>`;
    }
  });
  return entries;
}
/** A text shape with one DrawingML paragraph per text. */
export const shape = (...texts) =>
  `<p:sp><p:txBody>${texts.map((text) => `<a:p><a:r><a:t>${text}</a:t></a:r></a:p>`).join("")}</p:txBody></p:sp>`;

export function reorderedPresentation() {
  return {
    "[Content_Types].xml": contentTypes,
    "_rels/.rels": rootRelationships("ppt/presentation.xml"),
    "ppt/presentation.xml": `<p:presentation xmlns:p="${P}" xmlns:rel="${officeRelationship}"><p:sldIdLst><p:sldId rel:id="secondPart" id="257"/><p:sldId rel:id="firstPart" id="256"/></p:sldIdLst></p:presentation>`,
    "ppt/_rels/presentation.xml.rels": relationships([
      ["firstPart", "slide", "slides/slide1.xml"],
      ["secondPart", "slide", "slides/slide2.xml"],
    ]),
    "ppt/slides/slide1.xml": `<p:sld xmlns:p="${P}" xmlns:a="${A}"><p:cSld><a:p><a:r><a:t>SECOND-PRESENTED-SLIDE</a:t></a:r></a:p></p:cSld></p:sld>`,
    "ppt/slides/slide2.xml": `<p:sld xmlns:p="${P}" xmlns:a="${A}"><p:cSld><a:p><a:r><a:t>FIRST-PRESENTED-SLIDE</a:t></a:r></a:p></p:cSld></p:sld>`,
    "ppt/slides/_rels/slide2.xml.rels": relationships([
      ["notes", "notesSlide", "../notesSlides/notesSlide1.xml"],
    ]),
    "ppt/notesSlides/notesSlide1.xml": `<p:notes xmlns:p="${P}" xmlns:a="${A}"><p:cSld><a:p><a:r><a:t>FIRST-SLIDE-NOTES</a:t></a:r></a:p></p:cSld></p:notes>`,
  };
}

export function namedWorkbook() {
  return {
    "[Content_Types].xml": contentTypes,
    "_rels/.rels": rootRelationships("xl/workbook.xml"),
    "xl/workbook.xml": `<workbook xmlns="${S}" xmlns:rel="${officeRelationship}"><sheets><sheet name="Revenue" sheetId="7" rel:id="revenue"/><sheet name="Costs" sheetId="3" rel:id="costs"/></sheets></workbook>`,
    "xl/_rels/workbook.xml.rels": relationships([
      ["revenue", "worksheet", "worksheets/sheet7.xml"],
      ["costs", "worksheet", "worksheets/sheet3.xml"],
      ["strings", "sharedStrings", "sharedStrings.xml"],
    ]),
    "xl/worksheets/sheet7.xml": `<worksheet xmlns="${S}"><sheetData><row r="1"><c r="A1"><v>200</v></c></row></sheetData></worksheet>`,
    "xl/worksheets/sheet3.xml": `<worksheet xmlns="${S}"><sheetData><row r="1"><c r="A1"><v>100</v></c></row></sheetData></worksheet>`,
  };
}

export function repeatedSharedWorkbook(cellCounts) {
  return workbookPackage({
    sheets: cellCounts.map((count, index) => ({
      name: `Data ${index + 1}`,
      rows: Array.from(
        { length: count },
        (_, row) =>
          `<row r="${row + 1}"><c r="A${row + 1}" t="s"><v>0</v></c></row>`,
      ).join(""),
    })),
    sharedStrings: ["v".repeat(65536)],
  });
}
