// Builds the 16:9 deck template python-pptx opens in the assistant's Python
// engine: assistant-runtime/src/python/template.pptx (committed).
//
//   node scripts/assistant-deck-template.mjs [out.pptx]
//
// pptxgenjs draws the package, the masters and the layouts; the rest of this
// file fixes what it gets wrong or cannot express (an extra empty layout, a
// presentation.xml out of schema order, the Office theme, footer and slide
// number placeholders). pptxgenjs is a devDependency of this script only. The
// result must pass `officecli validate` with zero errors.
//
// The colours, fonts and geometry mirror `T` and `G` in
// assistant-runtime/src/python/deck.py; change both together.
import { mkdirSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { fileURLToPath } from "node:url";
import PptxGenJS from "pptxgenjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultOut = path.join(
  root,
  "assistant-runtime/src/python/template.pptx",
);

// jszip is pptxgenjs's own dependency; pnpm keeps it out of the root.
const JSZip = createRequire(import.meta.resolve("pptxgenjs"))("jszip");

const COLOR = {
  primary: "1E3A5F",
  primaryDark: "152B47",
  accent: "D9822B",
  text: "1F2A37",
  tint: "EEF3F8",
  faint: "98A2AE",
  line: "D5DDE6",
  white: "FFFFFF",
};
const FONT = "Microsoft YaHei";
// Standard PowerPoint widescreen, the 13.333 x 7.5 in of G.W and G.H in deck.py.
const SLIDE = { cx: 12192000, cy: 6858000 };
const EMU = 914400;
const emu = (inches) => Math.round(inches * EMU);

// The layout names deck.py looks up.
const LAYOUTS = ["Cover", "Section", "Content"];

// The theme's six accents are what a native chart uses for its series when
// nothing sets a colour.
const THEME_COLORS = {
  dk2: COLOR.primary,
  lt2: COLOR.tint,
  accent1: COLOR.primary,
  accent2: COLOR.accent,
  accent3: "5B7FA6",
  accent4: COLOR.faint,
  accent5: "2F7D6D",
  accent6: "7A5C8E",
  hlink: "2F6DB5",
  folHlink: "7A5C8E",
};

function defineLayouts(pptx) {
  const title = (name, box, style) => ({
    placeholder: {
      options: {
        name,
        type: "title",
        ...box,
        fontFace: FONT,
        align: "left",
        margin: 0,
        ...style,
      },
      text: "Click to edit the title",
    },
  });
  pptx.defineSlideMaster({
    title: "Cover",
    background: { color: COLOR.primaryDark },
    objects: [
      title(
        "Title",
        { x: 0.72, y: 1.9, w: 10.5, h: 1.6 },
        { fontSize: 44, bold: true, color: COLOR.white, valign: "top" },
      ),
    ],
  });
  pptx.defineSlideMaster({
    title: "Section",
    background: { color: COLOR.primaryDark },
    objects: [
      title(
        "Title",
        { x: 0.72, y: 3.1, w: 11, h: 1.0 },
        { fontSize: 36, bold: true, color: COLOR.white, valign: "top" },
      ),
    ],
  });
  pptx.defineSlideMaster({
    title: "Content",
    background: { color: COLOR.white },
    objects: [
      {
        rect: {
          x: 0.72,
          y: 0.5,
          w: 0.07,
          h: 0.5,
          fill: { color: COLOR.accent },
        },
      },
      // A hairline is a thin rectangle: officecli's preview does not draw a
      // zero-height shape.
      {
        rect: {
          x: 0.72,
          y: 1.42,
          w: 11.893,
          h: 0.75 / 72,
          fill: { color: COLOR.line },
        },
      },
      title(
        "Title",
        { x: 0.92, y: 0.4, w: 11.693, h: 0.62 },
        { fontSize: 26, bold: true, color: COLOR.primary, valign: "middle" },
      ),
    ],
  });
}

// The footer and the page number are real placeholders (PowerPoint's Header &
// Footer dialog edits them); pptxgenjs cannot write those types. python-pptx
// does not copy them to a slide, so deck.py adds them to each content slide.
function footerPlaceholders() {
  const placeholder = (id, name, type, idx, box, align, body) =>
    `<p:sp><p:nvSpPr><p:cNvPr id="${id}" name="${name}"/><p:cNvSpPr><a:spLocks noGrp="1"/></p:cNvSpPr>` +
    `<p:nvPr><p:ph type="${type}" sz="quarter" idx="${idx}"/></p:nvPr></p:nvSpPr>` +
    `<p:spPr><a:xfrm><a:off x="${emu(box.x)}" y="${emu(box.y)}"/><a:ext cx="${emu(box.w)}" cy="${emu(box.h)}"/></a:xfrm>` +
    `<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>` +
    `<p:txBody><a:bodyPr wrap="square" lIns="0" tIns="0" rIns="0" bIns="0" anchor="t"/>` +
    `<a:lstStyle><a:lvl1pPr algn="${align}"><a:defRPr sz="900"><a:solidFill><a:srgbClr val="${COLOR.faint}"/></a:solidFill>` +
    `<a:latin typeface="${FONT}"/><a:ea typeface="${FONT}"/></a:defRPr></a:lvl1pPr></a:lstStyle>` +
    `<a:p>${body}<a:endParaRPr lang="en-US"/></a:p></p:txBody></p:sp>`;
  return (
    placeholder(
      20,
      "Footer Placeholder 19",
      "ftr",
      11,
      { x: 0.72, y: 7.0, w: 8, h: 0.3 },
      "l",
      "",
    ) +
    placeholder(
      21,
      "Slide Number Placeholder 20",
      "sldNum",
      12,
      { x: 12.0, y: 7.0, w: 0.6, h: 0.3 },
      "r",
      `<a:fld id="{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}" type="slidenum"><a:rPr lang="en-US"/><a:t>‹#›</a:t></a:fld>`,
    )
  );
}

const read = (zip, name) => zip.file(name).async("string");

function replaceOnce(xml, pattern, replacement, what) {
  if (!pattern.test(xml)) throw new Error(`template: ${what} not found`);
  return xml.replace(pattern, replacement);
}

// pptxgenjs always adds an empty "DEFAULT" layout first; drop it and number
// the three real layouts 1-3.
async function keepOnlyOurLayouts(zip) {
  const dropped = "ppt/slideLayouts/slideLayout1.xml";
  const first = await read(zip, dropped);
  if (!first.includes('name="DEFAULT"'))
    throw new Error("template: the first layout is not pptxgenjs's DEFAULT");
  for (let n = 1; n <= LAYOUTS.length; n++) {
    const from = `ppt/slideLayouts/slideLayout${n + 1}`;
    zip.file(
      `ppt/slideLayouts/slideLayout${n}.xml`,
      await read(zip, `${from}.xml`),
    );
    zip.file(
      `ppt/slideLayouts/_rels/slideLayout${n}.xml.rels`,
      await read(zip, `ppt/slideLayouts/_rels/slideLayout${n + 1}.xml.rels`),
    );
  }
  const last = LAYOUTS.length + 1;
  zip.remove(`ppt/slideLayouts/slideLayout${last}.xml`);
  zip.remove(`ppt/slideLayouts/_rels/slideLayout${last}.xml.rels`);

  let types = await read(zip, "[Content_Types].xml");
  types = replaceOnce(
    types,
    new RegExp(
      `<Override PartName="/ppt/slideLayouts/slideLayout${last}\\.xml"[^>]*/>`,
    ),
    "",
    "the fourth layout's content type",
  );
  // pptxgenjs declares the master's content type once per slide; there are none.
  types = replaceOnce(
    types,
    /<Override PartName="\/ppt\/presProps\.xml"/,
    '<Override PartName="/ppt/slideMasters/slideMaster1.xml" ContentType="application/vnd.openxmlformats-officedocument.presentationml.slideMaster+xml"/>$&',
    "the presProps content type",
  );
  zip.file("[Content_Types].xml", types);

  const relationship = (type, id, target) =>
    `<Relationship Id="rId${id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/${type}" Target="${target}"/>`;
  const layoutRels = LAYOUTS.map((_, i) =>
    relationship(
      "slideLayout",
      i + 1,
      `../slideLayouts/slideLayout${i + 1}.xml`,
    ),
  ).join("");
  zip.file(
    "ppt/slideMasters/_rels/slideMaster1.xml.rels",
    `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${layoutRels}${relationship("theme", LAYOUTS.length + 1, "../theme/theme1.xml")}</Relationships>`,
  );
  const ids = LAYOUTS.map(
    (_, i) => `<p:sldLayoutId id="${2147483649 + i}" r:id="rId${i + 1}"/>`,
  ).join("");
  let master = await read(zip, "ppt/slideMasters/slideMaster1.xml");
  master = replaceOnce(
    master,
    /<p:sldLayoutIdLst>[\s\S]*?<\/p:sldLayoutIdLst>/,
    `<p:sldLayoutIdLst>${ids}</p:sldLayoutIdLst>`,
    "the master's layout list",
  );
  // New slides in PowerPoint get a footer and a page number, not a date.
  master = replaceOnce(
    master,
    /<p:hf [^>]*\/>/,
    '<p:hf hdr="0" dt="0"/>',
    "the master's header and footer flags",
  );
  zip.file("ppt/slideMasters/slideMaster1.xml", master);
}

// The title's vertical anchor in each layout (see defineLayouts).
const TITLE_ANCHOR = { Cover: "t", Section: "t", Content: "ctr" };

async function fixLayoutPlaceholders(zip) {
  for (const [i, name] of LAYOUTS.entries()) {
    const file = `ppt/slideLayouts/slideLayout${i + 1}.xml`;
    let xml = await read(zip, file);
    // pptxgenjs numbers its placeholders from 100; a title has no index.
    xml = replaceOnce(
      xml,
      /<p:ph\s+idx="\d+"\s+type="title"\s+hasCustomPrompt="1"\s*\/>/,
      '<p:ph type="title"/>',
      `the title placeholder of ${name}`,
    );
    // The title sits flush in its box, as the footer and page number do.
    xml = replaceOnce(
      xml,
      /<a:bodyPr wrap="square" rtlCol="0"(?:\/>|><\/a:bodyPr>)/,
      `<a:bodyPr wrap="square" lIns="0" tIns="0" rIns="0" bIns="0" rtlCol="0" anchor="${TITLE_ANCHOR[name]}"/>`,
      `the title's text box of ${name}`,
    );
    // An empty <a:ln/> is no line in PowerPoint but a black one in officecli.
    xml = xml.replace(/<a:ln(?:\/>|><\/a:ln>)/g, "<a:ln><a:noFill/></a:ln>");
    if (name === "Content")
      xml = replaceOnce(
        xml,
        /<\/p:spTree>/,
        `${footerPlaceholders()}</p:spTree>`,
        "the Content layout's shape tree",
      );
    zip.file(file, xml);
  }
}

async function fixPresentation(zip) {
  let xml = await read(zip, "ppt/presentation.xml");
  // The schema wants notesMasterIdLst before sldIdLst; pptxgenjs writes it
  // after an empty sldIdLst.
  const notes = xml.match(/<p:notesMasterIdLst>[\s\S]*?<\/p:notesMasterIdLst>/);
  if (!notes) throw new Error("template: notesMasterIdLst not found");
  xml = xml.replace(notes[0], "");
  xml = replaceOnce(
    xml,
    /<p:sldIdLst><\/p:sldIdLst>/,
    notes[0],
    "the empty slide list",
  );
  xml = replaceOnce(
    xml,
    /<p:sldSz [^>]*\/>/,
    `<p:sldSz cx="${SLIDE.cx}" cy="${SLIDE.cy}"/>`,
    "the slide size",
  );
  xml = replaceOnce(
    xml,
    /<p:notesSz [^>]*\/>/,
    '<p:notesSz cx="6858000" cy="9144000"/>',
    "the notes page size",
  );
  zip.file("ppt/presentation.xml", xml);
}

async function fixTheme(zip) {
  let xml = await read(zip, "ppt/theme/theme1.xml");
  for (const [role, value] of Object.entries(THEME_COLORS))
    xml = replaceOnce(
      xml,
      new RegExp(`<a:${role}><a:srgbClr val="[0-9A-F]{6}"/></a:${role}>`),
      `<a:${role}><a:srgbClr val="${value}"/></a:${role}>`,
      `theme colour ${role}`,
    );
  xml = replaceOnce(
    xml,
    /<a:dk1><a:sysClr [^>]*\/><\/a:dk1>/,
    `<a:dk1><a:srgbClr val="${COLOR.text}"/></a:dk1>`,
    "theme colour dk1",
  );
  // East Asian text takes the same face as Latin text.
  xml = xml.replaceAll('<a:ea typeface=""/>', `<a:ea typeface="${FONT}"/>`);
  xml = replaceOnce(
    xml,
    /<a:theme ([^>]*)name="Office Theme"/,
    '<a:theme $1name="Ofox"',
    "the theme name",
  );
  zip.file("ppt/theme/theme1.xml", xml);
}

// pptxgenjs points the notes master at the slide master's theme part; PowerPoint
// gives each master a theme of its own.
async function giveNotesMasterItsTheme(zip) {
  zip.file("ppt/theme/theme2.xml", await read(zip, "ppt/theme/theme1.xml"));
  zip.file(
    "[Content_Types].xml",
    replaceOnce(
      await read(zip, "[Content_Types].xml"),
      /<Override PartName="\/ppt\/theme\/theme1\.xml"[^>]*\/>/,
      (theme) => `${theme}${theme.replace("theme1", "theme2")}`,
      "the theme content type",
    ),
  );
  const rels = "ppt/notesMasters/_rels/notesMaster1.xml.rels";
  zip.file(
    rels,
    replaceOnce(
      await read(zip, rels),
      /\.\.\/theme\/theme1\.xml/,
      "../theme/theme2.xml",
      "the notes master's theme",
    ),
  );
}

const FIXED_DATE = new Date("2026-11-09T00:00:00Z");

async function fixProperties(zip) {
  zip.file(
    "docProps/app.xml",
    '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n' +
      '<Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties">' +
      "<Application>Ofox</Application><PresentationFormat>Widescreen</PresentationFormat>" +
      "</Properties>",
  );
  let core = await read(zip, "docProps/core.xml");
  core = core.replace(/<dc:subject>[\s\S]*?<\/dc:subject>\s*/, "");
  core = core.replace(
    /(<dcterms:(?:created|modified) [^>]*>)[^<]*/g,
    `$1${FIXED_DATE.toISOString().replace(/\.\d+Z$/, "Z")}`,
  );
  zip.file("docProps/core.xml", core);
}

export async function buildTemplate() {
  const pptx = new PptxGenJS();
  pptx.defineLayout({
    name: "WIDE",
    width: SLIDE.cx / EMU,
    height: SLIDE.cy / EMU,
  });
  pptx.layout = "WIDE";
  pptx.title = "Ofox deck";
  pptx.author = "Ofox";
  pptx.company = "Ofox";
  pptx.subject = "";
  pptx.theme = { headFontFace: FONT, bodyFontFace: FONT };
  defineLayouts(pptx);

  const zip = await JSZip.loadAsync(
    await pptx.write({ outputType: "nodebuffer" }),
  );
  await keepOnlyOurLayouts(zip);
  await fixLayoutPlaceholders(zip);
  await fixPresentation(zip);
  await fixTheme(zip);
  await giveNotesMasterItsTheme(zip);
  await fixProperties(zip);
  // Files only (no folder entries), in name order, with one date, so a
  // rebuild changes nothing.
  const sorted = new JSZip();
  for (const name of Object.keys(zip.files).sort())
    if (!zip.files[name].dir)
      sorted.file(name, await zip.files[name].async("nodebuffer"), {
        date: FIXED_DATE,
        createFolders: false,
      });
  return sorted.generateAsync({
    type: "nodebuffer",
    compression: "DEFLATE",
  });
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const out = path.resolve(process.argv[2] ?? defaultOut);
  mkdirSync(path.dirname(out), { recursive: true });
  writeFileSync(out, await buildTemplate());
  console.log(path.relative(process.cwd(), out));
}
