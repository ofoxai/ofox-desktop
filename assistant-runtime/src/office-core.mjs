// Text of DOCX, XLSX and PPTX packages, streamed: ZIP parts are inflated
// chunk by chunk (yauzl) into a namespace-aware SAX parser (saxes), so no
// part or DOM is held whole. Runs in the Office child process, whose heap is
// capped (office.mjs); every limit below stops reading instead of failing
// the whole file, except the entry count, nesting depth and a dead child.
import path from "node:path";
import { crc32 } from "node:zlib";
import yauzl from "yauzl";
import { SaxesParser } from "saxes";
import { OfficeError } from "./office.mjs";

const MAX_TEXT = 16 * 1024 * 1024;
const LIMITS = {
  entries: 10000, // central directory entries
  depth: 256, // XML nesting; saxes resolves namespaces in O(depth) per element
  inflated: 128 * 1024 * 1024, // bytes inflated across all parts of one file
  sharedStrings: 8 * 1024 * 1024, // characters kept from the shared strings
  sharedStringCount: 1024 * 1024,
  styles: 65536, // cell formats and number formats kept
  formatCode: 255, // characters in a number format code, Excel's own limit
  cell: 32767, // characters per cell, Excel's own limit
};

const NS = {
  w: [
    "http://schemas.openxmlformats.org/wordprocessingml/2006/main",
    "http://purl.oclc.org/ooxml/wordprocessingml/main",
  ],
  a: [
    "http://schemas.openxmlformats.org/drawingml/2006/main",
    "http://purl.oclc.org/ooxml/drawingml/main",
  ],
  m: [
    "http://schemas.openxmlformats.org/officeDocument/2006/math",
    "http://purl.oclc.org/ooxml/officeDocument/math",
  ],
  s: [
    "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
    "http://purl.oclc.org/ooxml/spreadsheetml/main",
  ],
  p: [
    "http://schemas.openxmlformats.org/presentationml/2006/main",
    "http://purl.oclc.org/ooxml/presentationml/main",
  ],
  r: [
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
    "http://purl.oclc.org/ooxml/officeDocument/relationships",
  ],
  pr: ["http://schemas.openxmlformats.org/package/2006/relationships"],
  mc: ["http://schemas.openxmlformats.org/markup-compatibility/2006"],
};
const ns = Object.fromEntries(
  Object.entries(NS).map(([key, uris]) => [key, new Set(uris)]),
);
// ".../officeDocument/2006/relationships/slide" -> "slide"
const RELATIONSHIP_BASES = new Set(NS.r.map((uri) => `${uri}/`));
const relationshipKind = (type) => {
  const at = type.lastIndexOf("/");
  return RELATIONSHIP_BASES.has(type.slice(0, at + 1))
    ? type.slice(at + 1)
    : null;
};
const attribute = (tag, local, uris) => {
  for (const item of Object.values(tag.attributes))
    if (item.local === local && (uris ? uris.has(item.uri) : !item.uri))
      return item.value;
  return undefined;
};
const on = (value) => value === undefined || /^(1|true|on)$/i.test(value);

// ------------------------------------------------------------------ package

const CFB = Buffer.from("d0cf11e0a1b11ae1", "hex");
const ENCRYPTED_PACKAGE = Buffer.from("EncryptedPackage", "utf16le");

// Files saved through text-mode transfers or some downloaders carry bytes
// after the end-of-central-directory record; Office and unzip accept them,
// yauzl does not.
function trimZipTail(bytes) {
  const floor = Math.max(0, bytes.length - 22 - 0xffff);
  for (let at = bytes.length - 22; at >= floor; at--) {
    if (bytes.readUInt32LE(at) !== 0x06054b50) continue;
    const end = at + 22 + bytes.readUInt16LE(at + 20);
    return end < bytes.length ? bytes.subarray(0, end) : bytes;
  }
  return bytes;
}

// Parts this reader never opens, named in the warning.
const UNREAD = [
  [/^(?:word|ppt|xl)\/media\/[^/]+$/i, "image"],
  [/^(?:word|ppt|xl)\/charts\/chart[^/]*\.xml$/i, "chart"],
  [/^(?:word|ppt|xl)\/diagrams\/data[^/]*\.xml$/i, "diagram"],
  [/^(?:word|ppt|xl)\/embeddings\/[^/]+$/i, "embedded object"],
];

// The central directory only: no part is inflated here.
function openPackage(bytes) {
  return new Promise((resolve, reject) => {
    yauzl.fromBuffer(
      trimZipTail(bytes),
      { lazyEntries: true, validateEntrySizes: true, strictFileNames: false },
      (error, zip) => {
        if (error) return reject(new OfficeError("invalid_office"));
        if (zip.entryCount > LIMITS.entries) {
          zip.close();
          return reject(new OfficeError("office_expansion"));
        }
        const entries = new Map();
        const lower = new Map();
        const unread = new Map();
        let failed = false;
        const fail = (reason) => {
          if (failed) return;
          failed = true;
          zip.close();
          reject(new OfficeError(reason));
        };
        zip.on("error", () => fail("invalid_office"));
        zip.on("end", () => {
          if (!failed) resolve({ zip, entries, lower, unread });
        });
        zip.on("entry", (entry) => {
          if (entry.isEncrypted()) return fail("encrypted_office");
          const name = entry.fileName;
          if (!name.endsWith("/")) {
            // Office and this reader could pick different duplicates.
            if (entries.has(name)) return fail("invalid_office");
            entries.set(name, entry);
            // Part names are case-insensitive.
            if (!lower.has(name.toLowerCase()))
              lower.set(name.toLowerCase(), entry);
            const kind = UNREAD.find(([pattern]) => pattern.test(name))?.[1];
            if (kind) unread.set(kind, (unread.get(kind) ?? 0) + 1);
          }
          zip.readEntry();
        });
        zip.readEntry();
      },
    );
  });
}

const partEntry = (pkg, name) =>
  pkg.entries.get(name) ?? pkg.lower.get(name.toLowerCase());

/** Inflated bytes, counted while inflating, and the child's deadline. */
class Budget {
  used = 0;
  exhausted = false;
  constructor(deadline) {
    this.ends = performance.now() + deadline;
  }
  take(bytes) {
    this.used += bytes;
    if (this.used > LIMITS.inflated || performance.now() > this.ends)
      this.exhausted = true;
    return !this.exhausted;
  }
}

function decoderFor(head) {
  if (head[0] === 0xff && head[1] === 0xfe)
    return new TextDecoder("utf-16le", { fatal: true });
  if (head[0] === 0xfe && head[1] === 0xff)
    return new TextDecoder("utf-16be", { fatal: true });
  return new TextDecoder("utf-8", { fatal: true });
}

/**
 * Streams one part through SAX. handlers: open(tag, capture), close(tag,
 * capture), text(string), stop() -> true to end early. Text is collected
 * only between capture(true) and capture(false), so whitespace or markup
 * between text elements never accumulates. Returns false when it ended
 * early: on the budget, the deadline or stop().
 */
async function parsePart(pkg, name, budget, handlers) {
  const entry = partEntry(pkg, name);
  if (!entry) throw new OfficeError("invalid_office");
  const parser = new SaxesParser({ xmlns: true, position: false });
  // Parser errors can quote the document; only the outcome leaves.
  const fail = (reason) => {
    throw new OfficeError(reason);
  };
  parser.on("error", () => fail("invalid_office"));
  // No DTD: no entity can expand or reach a file or URL.
  parser.on("doctype", () => fail("invalid_office"));
  const capture = (enabled) => {
    if (enabled) {
      parser.on("text", handlers.text);
      parser.on("cdata", handlers.text);
    } else {
      parser.off("text");
      parser.off("cdata");
    }
  };
  let depth = 0;
  parser.on("opentag", (tag) => {
    if (++depth > LIMITS.depth) fail("office_expansion");
    handlers.open(tag, capture);
  });
  parser.on("closetag", (tag) => {
    depth--;
    handlers.close(tag, capture);
  });
  let stream;
  try {
    stream = await new Promise((resolve, reject) =>
      pkg.zip.openReadStream(entry, (error, opened) =>
        error ? reject(error) : resolve(opened),
      ),
    );
    let decoder;
    let head = Buffer.alloc(0); // bytes before the byte order mark is known
    let checksum = 0;
    for await (const chunk of stream) {
      if (!budget.take(chunk.length) || handlers.stop?.()) return false;
      checksum = crc32(chunk, checksum);
      if (decoder) parser.write(decoder.decode(chunk, { stream: true }));
      else {
        head = Buffer.concat([head, chunk]);
        if (head.length < 2) continue;
        decoder = decoderFor(head);
        parser.write(decoder.decode(head, { stream: true }));
      }
    }
    if (handlers.stop?.()) return false;
    if (checksum !== entry.crc32) fail("invalid_office");
    parser.write(decoder ? decoder.decode() : decoderFor(head).decode(head));
    parser.close();
    return true;
  } catch (error) {
    if (error instanceof OfficeError) throw error;
    // Inflate errors, sizes that differ from the directory, invalid UTF-8.
    throw new OfficeError("invalid_office");
  } finally {
    stream?.destroy();
  }
}

// A relationship target, resolved against the part that owns the .rels file.
function resolveTarget(owner, target) {
  if (
    !target ||
    /^[a-z][a-z0-9+.-]*:/i.test(target) ||
    /[\\?#]/.test(target) ||
    target.startsWith("//")
  )
    return null;
  let decoded;
  try {
    decoded = decodeURIComponent(target);
  } catch {
    return null;
  }
  const joined = decoded.startsWith("/")
    ? decoded.slice(1)
    : path.posix.join(path.posix.dirname(owner), decoded);
  const normalized = path.posix.normalize(joined);
  return normalized === ".." || normalized.startsWith("../")
    ? null
    : normalized;
}

/**
 * Internal relationships of a part ("" is the package): byId and byKind.
 * External targets are never followed. Read once per part.
 */
async function relationshipsOf(pkg, owner, budget) {
  const cached = pkg.relationships.get(owner);
  if (cached) return cached;
  const part =
    owner === ""
      ? "_rels/.rels"
      : `${path.posix.dirname(owner)}/_rels/${path.posix.basename(owner)}.rels`;
  const found = { byId: new Map(), byKind: new Map(), present: false };
  if (partEntry(pkg, part)) {
    found.present = true;
    await parsePart(pkg, part, budget, {
      open(tag) {
        if (!ns.pr.has(tag.uri) || tag.local !== "Relationship") return;
        const id = attribute(tag, "Id");
        const kind = relationshipKind(attribute(tag, "Type") ?? "");
        if (!id || !kind || attribute(tag, "TargetMode") === "External") return;
        const target = resolveTarget(
          owner === "" ? "package" : owner,
          attribute(tag, "Target"),
        );
        if (!target) return;
        found.byId.set(id, { kind, target });
        if (!found.byKind.has(kind)) found.byKind.set(kind, []);
        found.byKind.get(kind).push(target);
      },
      close() {},
    });
  }
  pkg.relationships.set(owner, found);
  return found;
}

// ------------------------------------------------------------------ output

const RANK = { "": 0, " ": 1, "\n": 2, "\n\n": 3 };

/**
 * The extracted text. Separators are owed and written only before the next
 * text, so none trails or leads; inside a table row, cell boundaries are
 * counted as tabs so empty cells keep the columns aligned.
 */
class Output {
  blocks = [];
  chunks = [];
  length = 0;
  content = 0; // pieces of document text, not labels
  #break = "";
  #tabs = 0;
  #rowFresh = false;
  get full() {
    return this.length > MAX_TEXT;
  }
  #push(text) {
    this.chunks.push(text);
    this.length += text.length;
    if (this.chunks.length === 4096) {
      this.blocks.push(this.chunks.join(""));
      this.chunks = [];
    }
  }
  /** A label or separator character: written, but not document text. */
  write(text) {
    if (!text || this.full) return;
    if (this.length && this.#break) this.#push(this.#break);
    if (this.#tabs) this.#push("\t".repeat(this.#tabs));
    this.#break = "";
    this.#tabs = 0;
    this.#rowFresh = false;
    // One character past the limit marks the text as cut.
    this.#push(text.slice(0, Math.max(0, MAX_TEXT + 1 - this.length)));
  }
  text(text) {
    if (/\S/.test(text)) this.content++;
    this.write(text);
  }
  #owe(kind) {
    if (RANK[kind] > RANK[this.#break]) this.#break = kind;
  }
  space() {
    if (!this.#tabs) this.#owe(" ");
  }
  newline() {
    this.#tabs = 0;
    this.#owe("\n");
  }
  // An empty paragraph after a paragraph leaves one blank line.
  paragraph() {
    this.#tabs = 0;
    this.#owe(this.#break === "\n" ? "\n\n" : "\n");
  }
  line(label) {
    this.#tabs = 0;
    this.#owe("\n\n");
    this.write(label);
    this.newline();
  }
  row() {
    this.newline();
    this.#rowFresh = true;
  }
  /** The boundary before a row's next cell; `span` grid columns before it. */
  cell(span = 1) {
    // A space owed inside the previous cell is dropped; the line break
    // owed before a row whose cells are all empty so far is kept.
    if (!this.#rowFresh) this.#break = "";
    this.#tabs += span;
  }
  finish() {
    return [...this.blocks, this.chunks.join("")];
  }
}

// ------------------------------------------------------------ Word and slides

/**
 * WordprocessingML and DrawingML text: the Word body, footnotes and
 * endnotes, slides and slide notes. Skips tracked deletions, moved-from
 * text, field codes, hidden (vanish) runs and a compatibility fallback whose
 * choice had text. Outermost tables are laid out as rows of tab-separated
 * cells; nested tables are flattened into their cell.
 */
function flowText(pkg, name, budget, out, { notes, onRoot } = {}) {
  const stack = [];
  const alternatives = [];
  const rows = [];
  const runs = []; // per open run: hidden by vanish; a run can hold a text box
  const hidden = () => runs.includes(true);
  let skip = 0;
  let inText = 0;
  let tables = 0;
  let root = true;
  return parsePart(pkg, name, budget, {
    open(tag, capture) {
      const parent = stack.at(-1);
      const grandparent = stack.at(-2);
      stack.push(tag.local);
      if (root) {
        root = false;
        onRoot?.(tag);
      }
      if (skip) {
        skip++;
        return;
      }
      const w = ns.w.has(tag.uri);
      const a = ns.a.has(tag.uri);
      const local = tag.local;
      if (ns.mc.has(tag.uri)) {
        if (local === "AlternateContent")
          alternatives.push({ read: false, mark: 0 });
        else if (local === "Choice" || local === "Fallback") {
          const alternative = alternatives.at(-1);
          if (!alternative || alternative.read) skip = 1;
          else alternative.mark = out.content;
        }
      } else if (
        w &&
        (local === "del" ||
          local === "moveFrom" ||
          local === "instrText" ||
          local === "delInstrText")
      )
        skip = 1;
      else if (
        w &&
        notes &&
        (local === "footnote" || local === "endnote") &&
        (attribute(tag, "type", ns.w) ?? "normal") !== "normal"
      )
        skip = 1; // separators and continuation notices
      else if (
        a &&
        local === "fld" &&
        /^slidenum/i.test(attribute(tag, "type") ?? "")
      )
        skip = 1;
      else if (w && local === "r") runs.push(false);
      else if (local === "t" && (w || a || ns.m.has(tag.uri)) && !hidden()) {
        if (inText++ === 0) capture(true);
      } else if (
        w &&
        local === "vanish" &&
        parent === "rPr" &&
        grandparent === "r"
      )
        runs[runs.length - 1] = on(attribute(tag, "val", ns.w));
      else if (
        ((w && (local === "tab" || local === "ptab") && parent === "r") ||
          (w && (local === "br" || local === "cr") && parent === "r") ||
          (a && local === "br")) &&
        !hidden()
      ) {
        // Within a table a tab or line break would split the row.
        if (tables) out.space();
        else if (local.endsWith("tab")) out.write("\t");
        else out.newline();
      } else if (w && local === "noBreakHyphen" && !hidden()) out.text("-");
      else if (w && notes && (local === "footnote" || local === "endnote")) {
        if (!notes.written) {
          out.line(notes.heading);
          notes.written = true;
        }
        out.newline();
        out.write(`[^${notes.prefix}${attribute(tag, "id", ns.w)}]:`);
        out.space();
      } else if (w && local === "footnoteReference")
        out.write(`[^${attribute(tag, "id", ns.w)}]`);
      else if (w && local === "endnoteReference")
        out.write(`[^e${attribute(tag, "id", ns.w)}]`);
      else if ((w || a) && local === "tbl") tables++;
      else if ((w || a) && local === "tr") {
        if (tables === 1) {
          out.row();
          rows.push({ cells: 0, span: 1 });
        } else out.space();
      } else if ((w || a) && local === "tc") {
        const row = rows.at(-1);
        if (tables !== 1 || !row) out.space();
        else {
          if (row.cells++) out.cell(row.span);
          row.span = 1;
        }
      } else if (
        w &&
        local === "gridSpan" &&
        tables === 1 &&
        parent === "tcPr"
      ) {
        const span = Number(attribute(tag, "val", ns.w));
        const row = rows.at(-1);
        if (row && Number.isInteger(span) && span > 1 && span <= 64)
          row.span = span;
      }
    },
    close(tag, capture) {
      stack.pop();
      if (skip) {
        skip--;
        return;
      }
      const w = ns.w.has(tag.uri);
      const a = ns.a.has(tag.uri);
      const local = tag.local;
      if (ns.mc.has(tag.uri)) {
        const alternative = alternatives.at(-1);
        if (local === "AlternateContent") alternatives.pop();
        else if (alternative && out.content > alternative.mark)
          alternative.read = true;
      } else if (local === "t" && inText && (w || a || ns.m.has(tag.uri))) {
        if (--inText === 0) capture(false);
      } else if (w && local === "r") runs.pop();
      else if ((w || a) && local === "p") {
        // A paragraph of a hidden run (in its text box) leaves no break.
        if (hidden()) return;
        if (tables) out.space();
        else out.paragraph();
      } else if ((w || a) && local === "tr" && tables === 1) {
        rows.pop();
        out.newline();
      } else if ((w || a) && local === "tbl") {
        tables--;
        if (!tables) out.newline();
      }
    },
    text(text) {
      if (skip || hidden()) return;
      // Within a table a tab or line break would split the row.
      out.text(tables ? text.replace(/[\t\r\n]+/g, " ") : text);
    },
    stop: () => out.full,
  });
}

async function mainPart(pkg, budget, fallback) {
  const root = await relationshipsOf(pkg, "", budget);
  return root.byKind.get("officeDocument")?.[0] ?? fallback;
}

async function docxText(pkg, budget, out, main, issues) {
  let complete = await flowText(pkg, main, budget, out);
  const related = await relationshipsOf(pkg, main, budget);
  for (const [kind, heading, prefix] of [
    ["footnotes", "[Footnotes]", ""],
    ["endnotes", "[Endnotes]", "e"],
  ]) {
    const target = related.byKind.get(kind)?.[0];
    if (!target || !complete) continue;
    if (!partEntry(pkg, target)) {
      issues.add(
        `Word ${kind} are missing from the package and were not read.`,
      );
      continue;
    }
    complete = await flowText(pkg, target, budget, out, {
      notes: { heading, prefix, written: false },
    });
  }
  return complete;
}

async function pptxText(pkg, budget, out, main, issues) {
  const related = await relationshipsOf(pkg, main, budget);
  const slides = [];
  await parsePart(pkg, main, budget, {
    open(tag) {
      if (ns.p.has(tag.uri) && tag.local === "sldId")
        slides.push(attribute(tag, "id", ns.r));
    },
    close() {},
  });
  const seen = new Map(); // slide part's zip entry -> its first slide number
  let complete = true;
  for (const [index, id] of slides.entries()) {
    if (out.full || !complete) return false;
    const number = index + 1;
    const slide = related.byId.get(id);
    if (!slide || slide.kind !== "slide" || !partEntry(pkg, slide.target)) {
      out.line(`[Slide ${number}]`);
      out.write("(not readable)");
      issues.add(
        "Some slides are missing, external or unsafe in the package and were not read.",
      );
      continue;
    }
    // Part names ignore case, so the entry, not the name, identifies it.
    const entry = partEntry(pkg, slide.target);
    if (seen.has(entry)) {
      out.line(`[Slide ${number}]`);
      out.write(`(same part as Slide ${seen.get(entry)})`);
      continue;
    }
    seen.set(entry, number);
    complete = await flowText(pkg, slide.target, budget, out, {
      onRoot: (tag) =>
        out.line(
          `[Slide ${number}${on(attribute(tag, "show")) ? "" : " (hidden)"}]`,
        ),
    });
    const notes = (await relationshipsOf(pkg, slide.target, budget)).byKind.get(
      "notesSlide",
    )?.[0];
    if (!notes || !complete) continue;
    if (!partEntry(pkg, notes)) {
      issues.add(
        "Some slide notes are missing from the package and were not read.",
      );
      continue;
    }
    out.line(`[Notes for slide ${number}]`);
    complete = await flowText(pkg, notes, budget, out);
  }
  return complete;
}

// ------------------------------------------------------------------ Excel

// Built-in number formats that show a date, a time or both. 27-36 and 50-58
// depend on the locale (ja, zh, ko): 32-33 are times everywhere, 34-35 and
// 55-56 are dates in some locales and times in others, so they show both.
const BUILTIN_DATES = new Map([
  ...[14, 15, 16, 17, 27, 28, 29, 30, 31, 36, 50, 51, 52, 53, 54, 57, 58].map(
    (id) => [id, "date"],
  ),
  ...[18, 19, 20, 21, 32, 33, 45, 47].map((id) => [id, "time"]),
  ...[22, 34, 35, 55, 56].map((id) => [id, "datetime"]),
  [46, "elapsed"],
]);

/** What a number format code shows: "date", "time", "datetime", "elapsed" or null. */
function dateKind(code) {
  // Longer than Excel allows: not a format Excel wrote, and the patterns
  // below would cost time with its length.
  if (code.length > LIMITS.formatCode) return null;
  if (/\[(h+|m+|s+)\]/i.test(code)) return "elapsed";
  // Literal text, escapes, colours/locales, padding and fill characters
  // are not date parts.
  const parts = code.replace(/"[^"]*"|\\.|\[[^\]]*\]|_.|\*.|General/gi, "");
  const date =
    /[dy]/i.test(parts) || (/m/i.test(parts) && !/[hs]/i.test(parts));
  const time = /[hs]/i.test(parts);
  return date && time ? "datetime" : date ? "date" : time ? "time" : null;
}

const pad = (value) => String(value).padStart(2, "0");
const DAY = 86400;

/** A serial date as Excel shows it, ISO-ordered; null when out of range. */
function serialText(value, kind, date1904) {
  const serial = Number(value);
  if (!/^-?[\d.]+(?:E[+-]?\d+)?$/i.test(value) || !Number.isFinite(serial))
    return null;
  if (serial < 0 || serial >= 2958466) return null;
  const total = Math.round(serial * DAY);
  const seconds = total % DAY;
  const clock = `${pad(Math.floor(seconds / 3600))}:${pad(Math.floor((seconds % 3600) / 60))}:${pad(seconds % 60)}`;
  if (kind === "elapsed")
    return `${Math.floor(total / 3600)}:${pad(Math.floor((total % 3600) / 60))}:${pad(total % 60)}`;
  if (kind === "time") return clock;
  const days = Math.floor(total / DAY);
  let day;
  if (date1904) day = new Date(Date.UTC(1904, 0, 1 + days));
  // Excel's 1900 system counts a 29 February 1900 that never was.
  else if (days === 60) day = null;
  else day = new Date(Date.UTC(1899, 11, days < 60 ? 31 + days : 30 + days));
  const date = day ? day.toISOString().slice(0, 10) : "1900-02-29";
  return kind === "date" ? date : `${date} ${clock}`;
}

const columnIndex = (reference) => {
  let column = 0;
  for (const letter of /^[A-Z]{1,3}/.exec(reference)?.[0] ?? "")
    column = column * 26 + letter.charCodeAt(0) - 64;
  return column;
};
const columnName = (index) => {
  let name = "";
  for (; index > 0; index = Math.floor((index - 1) / 26))
    name = String.fromCharCode(65 + ((index - 1) % 26)) + name;
  return name;
};

async function sharedStrings(pkg, target, budget) {
  const strings = [];
  let characters = 0;
  let current = null;
  let phonetic = 0;
  let inText = 0;
  const complete = await parsePart(pkg, target, budget, {
    open(tag, capture) {
      if (!ns.s.has(tag.uri)) return;
      if (phonetic) phonetic++;
      // Phonetic runs (furigana) repeat the reading, not the text.
      else if (tag.local === "rPh") phonetic = 1;
      else if (tag.local === "si") current = "";
      else if (tag.local === "t" && current !== null && inText++ === 0)
        capture(true);
    },
    close(tag, capture) {
      if (!ns.s.has(tag.uri)) return;
      if (phonetic) phonetic--;
      else if (tag.local === "t" && inText) {
        if (--inText === 0) capture(false);
      } else if (tag.local === "si" && current !== null) {
        strings.push(current.slice(0, LIMITS.cell));
        characters += current.length;
        current = null;
      }
    },
    text(text) {
      if (current !== null) current += text;
    },
    stop: () =>
      characters > LIMITS.sharedStrings ||
      strings.length >= LIMITS.sharedStringCount,
  });
  return { strings, complete };
}

/** The date kind of each cell format (cellXfs index), from styles.xml. */
async function dateStyles(pkg, target, budget) {
  const formats = new Map(); // numFmtId -> its date kind, read once
  const kinds = [];
  let inCellFormats = false;
  await parsePart(pkg, target, budget, {
    open(tag) {
      if (!ns.s.has(tag.uri)) return;
      if (tag.local === "numFmt" && formats.size < LIMITS.styles)
        formats.set(
          Number(attribute(tag, "numFmtId")),
          dateKind(attribute(tag, "formatCode") ?? ""),
        );
      else if (tag.local === "cellXfs") inCellFormats = true;
      else if (
        tag.local === "xf" &&
        inCellFormats &&
        kinds.length < LIMITS.styles
      ) {
        const id = Number(attribute(tag, "numFmtId") ?? 0);
        kinds.push(
          formats.has(id) ? formats.get(id) : (BUILTIN_DATES.get(id) ?? null),
        );
      }
    },
    close(tag) {
      if (ns.s.has(tag.uri) && tag.local === "cellXfs") inCellFormats = false;
    },
  });
  return kinds;
}

const cellText = (value) =>
  value.slice(0, LIMITS.cell).replace(/[\t\r\n]+/g, " ");

function sheetText(pkg, target, budget, out, book, issues) {
  let cell = null;
  let field = null; // "v", "f" or "t" while their text is captured
  let phonetic = 0;
  let rowNumber = 0;
  let lastColumn = 0;
  let written = 0;
  let rowWritten = false;
  const value = () => {
    const raw = cell.v;
    switch (cell.t) {
      case "s": {
        const index = /^\s*\d+\s*$/.test(raw) ? Number(raw) : NaN;
        const text = book.strings[index];
        if (text !== undefined) return text;
        if (book.stringsComplete)
          issues.add("Some cells refer to shared strings that do not exist.");
        return "[unread shared string]";
      }
      case "inlineStr":
        return cell.is;
      case "b":
        return raw.trim() === "1" ? "TRUE" : raw.trim() === "0" ? "FALSE" : raw;
      case "n": {
        const kind = book.dateKinds[cell.s];
        return (
          (kind && raw !== "" && serialText(raw.trim(), kind, book.date1904)) ||
          raw
        );
      }
      default: // str (formula text), e (error), d (ISO date)
        return raw;
    }
  };
  return parsePart(pkg, target, budget, {
    open(tag, capture) {
      if (!ns.s.has(tag.uri)) return;
      if (phonetic) {
        phonetic++;
        return;
      }
      switch (tag.local) {
        case "row": {
          const number = Number(attribute(tag, "r"));
          rowNumber =
            Number.isSafeInteger(number) && number > 0 ? number : rowNumber + 1;
          lastColumn = 0;
          rowWritten = false;
          break;
        }
        case "c":
          cell = {
            ref: attribute(tag, "r") ?? "",
            t: attribute(tag, "t") ?? "n",
            s: Number(attribute(tag, "s") ?? 0),
            shared: false,
            v: "",
            f: "",
            is: "",
          };
          break;
        case "f":
          if (cell) cell.shared = attribute(tag, "t") === "shared";
        // falls through
        case "v":
        case "t":
          if (cell && !field) {
            field = tag.local;
            capture(true);
          }
          break;
        case "rPh":
          phonetic = 1;
          break;
      }
    },
    close(tag, capture) {
      if (!ns.s.has(tag.uri)) return;
      if (phonetic) {
        phonetic--;
        return;
      }
      if (tag.local === field) {
        field = null;
        capture(false);
        return;
      }
      if (tag.local !== "c" || !cell) return;
      let text = cellText(value());
      const formula = cellText(cell.f).trim();
      if (formula) text = text === "" ? `=${formula}` : `${text} [=${formula}]`;
      else if (cell.shared)
        text = text === "" ? "[shared formula]" : `${text} [shared formula]`;
      if (text !== "") {
        const parsed = columnIndex(cell.ref);
        const column = parsed > 0 && parsed <= 16384 ? parsed : lastColumn + 1;
        // A row's first cell and any cell after a gap carry their coordinate.
        const label =
          !rowWritten || column !== lastColumn + 1
            ? `${columnName(column)}${rowNumber}: `
            : "";
        if (rowWritten) out.cell();
        else out.row();
        out.text(label + text);
        rowWritten = true;
        lastColumn = column;
        written++;
      }
      cell = null;
    },
    text(text) {
      if (!cell || phonetic) return;
      if (field === "v") cell.v += text;
      else if (field === "f") cell.f += text;
      else if (field === "t") cell.is += text;
    },
    stop: () => out.full,
  }).then((complete) => {
    if (!written && complete) out.write("(empty)");
    return complete;
  });
}

async function xlsxText(pkg, budget, out, main, issues) {
  const related = await relationshipsOf(pkg, main, budget);
  const sheets = [];
  let date1904 = false;
  await parsePart(pkg, main, budget, {
    open(tag) {
      if (!ns.s.has(tag.uri)) return;
      if (tag.local === "workbookPr")
        date1904 = /^(1|true|on)$/i.test(attribute(tag, "date1904") ?? "");
      else if (tag.local === "sheet")
        sheets.push({
          name: attribute(tag, "name") ?? "",
          state: attribute(tag, "state") ?? "visible",
          id: attribute(tag, "id", ns.r),
        });
    },
    close() {},
  });
  const book = {
    strings: [],
    stringsComplete: true,
    dateKinds: [],
    date1904,
  };
  const stringsPart = related.byKind.get("sharedStrings")?.[0];
  if (stringsPart && partEntry(pkg, stringsPart)) {
    const read = await sharedStrings(pkg, stringsPart, budget);
    book.strings = read.strings;
    book.stringsComplete = read.complete;
  }
  const stylesPart = related.byKind.get("styles")?.[0];
  if (stylesPart && partEntry(pkg, stylesPart) && !budget.exhausted)
    book.dateKinds = await dateStyles(pkg, stylesPart, budget);
  let complete = book.stringsComplete && !budget.exhausted;
  const seen = new Map(); // worksheet's zip entry -> its first sheet number
  for (const [index, sheet] of sheets.entries()) {
    if (out.full || budget.exhausted) return false;
    const number = index + 1;
    const state = sheet.state === "visible" ? "" : ` (${sheet.state})`;
    out.line(`[Sheet ${number}: ${JSON.stringify(sheet.name)}${state}]`);
    const relation = related.byId.get(sheet.id);
    if (relation?.kind === "chartsheet") {
      out.write("(chart sheet: not read)");
      continue;
    }
    if (relation?.kind === "dialogsheet") {
      out.write("(dialog sheet: not read)");
      continue;
    }
    if (relation?.kind !== "worksheet" || !partEntry(pkg, relation.target)) {
      out.write("(not readable)");
      issues.add(
        "Some worksheets are missing, external or unsafe in the package and were not read.",
      );
      continue;
    }
    // Part names ignore case, so the entry, not the name, identifies it.
    const entry = partEntry(pkg, relation.target);
    if (seen.has(entry)) {
      out.write(`(same part as Sheet ${seen.get(entry)})`);
      continue;
    }
    seen.set(entry, number);
    if (!(await sheetText(pkg, relation.target, budget, out, book, issues)))
      complete = false;
  }
  return complete;
}

// ------------------------------------------------------------------ entry

const FAMILIES = [
  [/^word\//i, ".docx", "word/document.xml"],
  [/^xl\//i, ".xlsx", "xl/workbook.xml"],
  [/^ppt\//i, ".pptx", "ppt/presentation.xml"],
];

/**
 * Text of an Office package. `extension` is the family the file name or MIME
 * type promises; the package's own main part decides when it names another.
 * Resolves to {text: [blocks], format, structureCoverage, truncated,
 * warning?}; rejects with an OfficeError.
 */
export async function officeText(bytes, extension, { deadline = 7500 } = {}) {
  if (bytes.subarray(0, 8).equals(CFB))
    throw new OfficeError(
      bytes.includes(ENCRYPTED_PACKAGE) ? "encrypted_office" : "invalid_office",
    );
  if (bytes.subarray(0, 2).toString("latin1") !== "PK")
    throw new OfficeError("invalid_office");
  const budget = new Budget(deadline);
  const pkg = await openPackage(bytes);
  pkg.relationships = new Map();
  try {
    const fallback = FAMILIES.find(([, family]) => family === extension)[2];
    const main = await mainPart(pkg, budget, fallback);
    const family =
      FAMILIES.find(([prefix]) => prefix.test(main))?.[1] ?? extension;
    if (!partEntry(pkg, main)) throw new OfficeError("invalid_office");
    const out = new Output();
    const issues = new Set();
    const read =
      family === ".xlsx" ? xlsxText : family === ".pptx" ? pptxText : docxText;
    const complete = await read(pkg, budget, out, main, issues);
    const truncated = !complete || out.full || budget.exhausted;
    if (!out.content)
      throw new OfficeError(
        truncated
          ? "office_expansion"
          : issues.size
            ? "invalid_office"
            : "empty_office",
      );
    const unread = [...pkg.unread].map(
      ([kind, count]) => `${count} ${kind}${count === 1 ? "" : "s"}`,
    );
    const warning = [
      ...issues,
      unread.length ? `Not read: ${unread.join(", ")}.` : "",
    ]
      .filter(Boolean)
      .join(" ");
    return {
      text: out.finish(),
      format: family.slice(1),
      structureCoverage: issues.size ? "partial" : "complete",
      truncated,
      ...(warning ? { warning } : {}),
    };
  } finally {
    pkg.zip.close();
  }
}
