/** Serializes numbered PDF objects (1 is the catalog) with a valid xref. */
export function pdfDocument(objects) {
  let output = "%PDF-1.4\n";
  const offsets = [];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(output));
    output += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(output);
  output += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(output);
}

/** A small real PDF: strings are text pages, null pages contain only an image. */
export function pdfPages(pages) {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pages.map((_, index) => `${4 + index * 2} 0 R`).join(" ")}] /Count ${pages.length} >>`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  for (const [index, text] of pages.entries()) {
    const content =
      text === null
        ? "q 80 0 0 80 20 20 cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID FF0000> EI Q"
        : `BT /F1 12 Tf 20 150 Td (${text.replace(/[\\()]/g, "\\$&")}) Tj ET`;
    objects.push(
      `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + index * 2} 0 R >>`,
      `<< /Length ${Buffer.byteLength(content)} >>\nstream\n${content}\nendstream`,
    );
  }
  return pdfDocument(objects);
}

/** An empty PDF that needs a user password (its /U entry matches none). */
export function passwordPdf() {
  return Buffer.from(
    pdfDocument([
      "<< /Type /Catalog /Pages 2 0 R >>",
      "<< /Type /Pages /Kids [] /Count 0 >>",
      `<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 /O <${"11".repeat(32)}> /U <${"22".repeat(32)}> >>`,
    ])
      .toString("latin1")
      .replace(
        "/Root 1 0 R >>",
        `/Root 1 0 R /Encrypt 3 0 R /ID [<${"33".repeat(16)}> <${"33".repeat(16)}>] >>`,
      ),
    "latin1",
  );
}

/**
 * One page of Chinese text in a non-embedded Adobe-GB1 font. Its UCS-2 string
 * is only readable through the predefined UniGB-UCS2-H CMap, as in many CJK
 * PDFs produced by office suites.
 */
export function chinesePdf(text) {
  const hex = [...text]
    .map((character) =>
      character.codePointAt(0).toString(16).toUpperCase().padStart(4, "0"),
    )
    .join("");
  const content = `BT /F1 12 Tf 20 150 Td <${hex}> Tj ET`;
  return pdfDocument([
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [4 0 R] /Count 1 >>",
    "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [6 0 R] >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 3 0 R >> >> /Contents 5 0 R >>",
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> /FontDescriptor 7 0 R >>",
    "<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 80 >>",
  ]);
}
