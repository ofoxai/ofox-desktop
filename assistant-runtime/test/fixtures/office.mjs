import { zipSync, strToU8 } from "fflate";

export const officeRelationship =
  "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
export const packageRelationship =
  "http://schemas.openxmlformats.org/package/2006/relationships";

export function officeZip(entries) {
  return zipSync(
    Object.fromEntries(
      Object.entries(entries).map(([name, value]) => [name, strToU8(value)]),
    ),
  );
}

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
