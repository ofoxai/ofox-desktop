// Text-only OOXML extraction. Runs in the bounded Office worker.
import path from "node:path";
import { crc32 } from "node:zlib";
import { fromBufferPromise } from "yauzl";
import { XMLParser, XMLValidator } from "fast-xml-parser";
import { OfficeError } from "./office.mjs";

const MAX_TEXT = 16 * 1024 * 1024;
const MAX_XML = 15 * 1024 * 1024;
const array = (value) =>
  value === undefined ? [] : Array.isArray(value) ? value : [value];

function xml(bytes, options = {}) {
  const text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  if (/<!DOCTYPE|<!ENTITY/i.test(text))
    throw new Error("Office XML entities and DOCTYPE are not supported");
  if (XMLValidator.validate(text) !== true)
    throw new Error("Office XML is malformed");
  if ((text.match(/</g)?.length ?? 0) > 200000)
    throw new OfficeError("office_expansion");
  return new XMLParser({
    ignoreAttributes: false,
    removeNSPrefix: true,
    parseTagValue: false,
    processEntities: false,
    htmlEntities: true,
    trimValues: false,
    maxNestedTags: 100,
    ...options,
  }).parse(text);
}

class OfficeTextBudget {
  length = 0;
  hasText = false;
  chunks = [];
  get full() {
    return this.length >= MAX_TEXT + 1;
  }
  append(value) {
    const kept = String(value).slice(0, MAX_TEXT + 1 - this.length);
    if (kept) {
      this.chunks.push(kept);
      this.length += kept.length;
    }
  }
  json(pieces) {
    this.append('"');
    for (const piece of pieces) {
      const value = String(piece);
      for (let offset = 0; offset < value.length && !this.full; offset += 4096)
        this.append(
          JSON.stringify(value.slice(offset, offset + 4096)).slice(1, -1),
        );
      if (this.full) return;
    }
    this.append('"');
  }
  finish() {
    return this.chunks.join("");
  }
}

// Object-shaped XML groups equal tag names, which reorders interleaved Word
// paragraphs/tables and text/hyperlink runs. Walk an ordered tree instead.
function documentText(bytes, output) {
  const visit = (nodes) => {
    for (const node of array(nodes))
      for (const [name, children] of Object.entries(node)) {
        if (output.full) return;
        if (name === "AlternateContent") {
          const variants = array(children);
          // Compatibility branches represent the same shape. Reading both
          // duplicates its content; prefer the baseline representation.
          const branch =
            variants.find((item) => item.Fallback)?.Fallback ??
            variants.find((item) => item.Choice)?.Choice;
          if (branch) visit(branch);
        } else if (name === "t") {
          for (const text of array(children))
            if (typeof text["#text"] === "string") {
              if (text["#text"].trim()) output.hasText = true;
              output.append(text["#text"]);
            }
        } else if (name === "tab") output.append("\t");
        else if (name === "br" || name === "cr") output.append("\n");
        else if (
          !name.startsWith("@") &&
          !name.startsWith("#") &&
          name !== "instrText" &&
          name !== "delText"
        ) {
          visit(children);
          if (name === "p" || name === "tr") output.append("\n");
          else if (name === "tc") output.append("\t");
        }
      }
  };
  // DOCTYPE/entity declarations are rejected above; only literal XML escaping
  // is decoded. No DTD, external resource or macro is loaded or executed.
  visit(xml(bytes, { preserveOrder: true, processEntities: true }));
}

function texts(value, key = "t") {
  if (!value || typeof value !== "object") return [];
  return Object.entries(value).flatMap(([name, child]) =>
    name === "rPh"
      ? []
      : name === key
        ? array(child).map((item) =>
            typeof item === "object" ? (item["#text"] ?? "") : String(item),
          )
        : array(child).flatMap((item) => texts(item, key)),
  );
}

function columnName(index) {
  let name = "";
  for (; index > 0; index = Math.floor((index - 1) / 26))
    name = String.fromCharCode(65 + ((index - 1) % 26)) + name;
  return name;
}

function sheetText(data, strings, output, issues) {
  let previousRow = 0;
  for (const row of array(
    xml(data, { processEntities: true })?.worksheet?.sheetData?.row,
  )) {
    if (output.full) return;
    const cells = array(row.c);
    const firstRow = String(cells[0]?.["@_r"] ?? "").match(
      /^[A-Z]+([0-9]+)$/,
    )?.[1];
    const rowNumber = Number(row["@_r"] ?? firstRow ?? previousRow + 1);
    if (
      !Number.isSafeInteger(rowNumber) ||
      rowNumber < 1 ||
      rowNumber > 1048576
    )
      throw new Error("Invalid worksheet row coordinate");
    previousRow = rowNumber;
    let previousColumn = 0;
    let first = true;
    for (const cell of cells) {
      if (output.full) return;
      const reference =
        cell["@_r"] ?? `${columnName(previousColumn + 1)}${rowNumber}`;
      const match = String(reference).match(/^([A-Z]{1,3})([1-9][0-9]*)$/);
      const column = match?.[1]
        .split("")
        .reduce((value, ch) => value * 26 + ch.charCodeAt(0) - 64, 0);
      if (!column || column > 16384 || Number(match[2]) !== rowNumber)
        throw new Error("Invalid worksheet cell coordinate");
      previousColumn = column;
      const indexText = typeof cell.v === "string" ? cell.v.trim() : "";
      const sharedIndex = /^\d+$/.test(indexText) ? Number(indexText) : NaN;
      const validSharedIndex =
        Number.isSafeInteger(sharedIndex) &&
        sharedIndex >= 0 &&
        strings[sharedIndex] !== undefined;
      let value =
        cell["@_t"] === "s"
          ? validSharedIndex
            ? strings[sharedIndex]
            : "[Unread shared string]"
          : cell["@_t"] === "inlineStr"
            ? texts(cell.is).join("")
            : (cell.v ?? "");
      if (cell["@_t"] === "s" && !validSharedIndex) {
        issues.add(
          "Some worksheet shared strings are missing or invalid; unread cells are marked explicitly.",
        );
      }
      if (String(value).trim() || cell.f) output.hasText = true;
      if (!first) output.append("\t");
      first = false;
      output.append(`${reference}: `);
      output.json(
        cell.f
          ? [
              "=",
              typeof cell.f === "object" ? (cell.f["#text"] ?? "") : cell.f,
              " [cached: ",
              value,
              "]",
            ]
          : [value],
      );
    }
    output.append("\n");
  }
}

const RELATIONSHIP_NAMESPACES = [
  "http://schemas.openxmlformats.org/officeDocument/2006/relationships",
  "http://purl.oclc.org/ooxml/officeDocument/relationships",
];
const PACKAGE_RELATIONSHIP_NAMESPACES = [
  "http://schemas.openxmlformats.org/package/2006/relationships",
];
const PRESENTATION_NAMESPACES = [
  "http://schemas.openxmlformats.org/presentationml/2006/main",
  "http://purl.oclc.org/ooxml/presentationml/main",
];
const WORKBOOK_NAMESPACES = [
  "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
  "http://purl.oclc.org/ooxml/spreadsheetml/main",
];
const SLIDE_PART = /^ppt\/slides\/[^/\\]+\.xml$/;
const NOTES_PART = /^ppt\/notesSlides\/[^/\\]+\.xml$/;
const SHEET_PART = /^xl\/worksheets\/[^/\\]+\.xml$/;

function namespaceBindings(...nodes) {
  const namespaces = new Map();
  for (const node of nodes)
    for (const [key, value] of Object.entries(node ?? {})) {
      if (key === "@_xmlns") namespaces.set("", value);
      else if (key.startsWith("@_xmlns:")) namespaces.set(key.slice(8), value);
    }
  return namespaces;
}

function childElement(node, localName, expectedNamespaces, ...ancestors) {
  if (!node || typeof node !== "object" || Array.isArray(node))
    return undefined;
  const entries = Object.entries(node).filter(
    ([name]) => !name.startsWith("@_") && name.split(":").at(-1) === localName,
  );
  if (entries.length !== 1) return undefined;
  const [name, children] = entries[0];
  const parts = name.split(":");
  if (parts.length > 2) return undefined;
  const prefix = parts.length === 2 ? parts[0] : "";
  for (const child of array(children)) {
    const namespaces = namespaceBindings(...ancestors, node, child);
    if (!expectedNamespaces.includes(namespaces.get(prefix))) return undefined;
  }
  return children;
}

function relationshipId(node, ...ancestors) {
  const namespaces = namespaceBindings(...ancestors, node);
  const ids = Object.entries(node).filter(([key]) => {
    const match = /^@_([^:]+):id$/.exec(key);
    return match && RELATIONSHIP_NAMESPACES.includes(namespaces.get(match[1]));
  });
  return ids.length === 1 && typeof ids[0][1] === "string" ? ids[0][1] : null;
}

function hasRelationshipType(value, kind) {
  return RELATIONSHIP_NAMESPACES.some(
    (namespace) => value === `${namespace}/${kind}`,
  );
}

function relationshipMap(bytes) {
  if (!bytes) throw new Error("Office relationships are missing");
  const root = childElement(
    xml(bytes, { removeNSPrefix: false, processEntities: true }),
    "Relationships",
    PACKAGE_RELATIONSHIP_NAMESPACES,
  );
  if (!root) throw new Error("Office relationships are malformed");
  const relationships = childElement(
    root,
    "Relationship",
    PACKAGE_RELATIONSHIP_NAMESPACES,
  );
  if (
    relationships === undefined &&
    Object.keys(root).some(
      (key) =>
        !key.startsWith("@_") && key.split(":").at(-1) === "Relationship",
    )
  )
    throw new Error("Office relationship namespaces are invalid");
  const result = new Map();
  for (const relationship of array(relationships)) {
    const id = relationship?.["@_Id"];
    if (typeof id !== "string" || !id || result.has(id))
      throw new Error("Office relationship IDs are ambiguous");
    result.set(id, relationship);
  }
  return result;
}

function relationshipTarget(relationship, source, kind, partPattern) {
  if (
    !relationship ||
    !hasRelationshipType(relationship["@_Type"], kind) ||
    (relationship["@_TargetMode"] !== undefined &&
      relationship["@_TargetMode"] !== "Internal") ||
    typeof relationship["@_Target"] !== "string"
  )
    return null;
  let target;
  try {
    target = decodeURIComponent(relationship["@_Target"]);
  } catch {
    return null;
  }
  if (
    !target ||
    /[\\%?#\u0000-\u001f]/.test(target) ||
    /^[a-z][a-z0-9+.-]*:/i.test(target) ||
    target.startsWith("//")
  )
    return null;
  const parts = [];
  const joined = target.startsWith("/")
    ? target.slice(1)
    : `${path.posix.dirname(source)}/${target}`;
  for (const part of joined.split("/")) {
    if (part === "..") {
      if (!parts.length) return null;
      parts.pop();
    } else if (part && part !== ".") parts.push(part);
  }
  const resolved = parts.join("/");
  return partPattern.test(resolved) ? resolved : null;
}

function relatedParts(files, extension, issues) {
  const presentation = extension === ".pptx";
  const source = presentation ? "ppt/presentation.xml" : "xl/workbook.xml";
  const relations = presentation
    ? "ppt/_rels/presentation.xml.rels"
    : "xl/_rels/workbook.xml.rels";
  const mainNamespaces = presentation
    ? PRESENTATION_NAMESPACES
    : WORKBOOK_NAMESPACES;
  try {
    if (!files[source]) throw new Error("Office metadata is missing");
    const root = childElement(
      xml(files[source], { removeNSPrefix: false, processEntities: true }),
      presentation ? "presentation" : "workbook",
      mainNamespaces,
    );
    const list = childElement(
      root,
      presentation ? "sldIdLst" : "sheets",
      mainNamespaces,
    );
    const references = array(
      childElement(
        list,
        presentation ? "sldId" : "sheet",
        mainNamespaces,
        root,
      ),
    );
    if (!root || !list || !references.length)
      throw new Error("Office metadata has no declared document structure");
    const relationships = relationshipMap(files[relations]);
    return references.map((reference, index) => {
      const id = relationshipId(reference, root, list);
      const name = presentation ? null : reference?.["@_name"];
      const target = relationshipTarget(
        relationships.get(id),
        source,
        presentation ? "slide" : "worksheet",
        presentation ? SLIDE_PART : SHEET_PART,
      );
      if (
        !target ||
        !files[target] ||
        (!presentation && (typeof name !== "string" || !name.trim()))
      ) {
        issues.add(
          "Some Office relationships or worksheet names are missing, invalid, external or unsafe; unavailable content has not been read.",
        );
        return { index: index + 1, target: null, name };
      }
      return { index: index + 1, target, name };
    });
  } catch {
    issues.add(
      "Office structure metadata or relationships are missing or invalid. Logical slide order or worksheet names are unverified; Unmapped labels are internal package parts.",
    );
    return null;
  }
}

export async function officeText(bytes, extension) {
  const files = await readParts(bytes, extension);
  const output = new OfficeTextBudget();
  if (extension === ".docx") {
    if (!files["word/document.xml"])
      throw new Error("DOCX document content is missing");
    documentText(files["word/document.xml"], output);
    if (!output.hasText) throw new OfficeError("empty_office");
    return { text: output.finish(), structureCoverage: "complete" };
  }
  const issues = new Set();
  const ordered = relatedParts(files, extension, issues);
  if (
    !Object.keys(files).some((name) =>
      (extension === ".xlsx" ? SHEET_PART : SLIDE_PART).test(name),
    )
  )
    throw new OfficeError("invalid_office");
  const parts = Object.keys(files).sort((a, b) =>
    a.localeCompare(b, undefined, { numeric: true }),
  );
  if (extension === ".xlsx") {
    const strings = files["xl/sharedStrings.xml"]
      ? array(
          xml(files["xl/sharedStrings.xml"], { processEntities: true })?.sst
            ?.si,
        ).map((item) => texts(item).join(""))
      : [];
    for (const item of ordered ??
      parts
        .filter((name) => SHEET_PART.test(name))
        .map((target) => ({ target }))) {
      if (output.full) break;
      if (!item.target) {
        output.append(
          `[Worksheet ${item.index}: unavailable relationship; content not read]\n\n`,
        );
        continue;
      }
      if (ordered) {
        output.append(`[Worksheet ${item.index}: `);
        output.json([item.name]);
        output.append("]\n");
      } else output.append(`[Unmapped worksheet part: ${item.target}]\n`);
      sheetText(files[item.target], strings, output, issues);
      output.append("\n");
    }
  } else {
    const readNotes = (target, index) => {
      if (output.full) return;
      const relPart = `${path.posix.dirname(target)}/_rels/${path.posix.basename(target)}.rels`;
      if (!files[relPart]) return;
      try {
        const notes = [...relationshipMap(files[relPart]).values()].filter(
          (rel) => hasRelationshipType(rel["@_Type"], "notesSlide"),
        );
        if (!notes.length) return;
        if (notes.length !== 1) throw new Error("Ambiguous slide notes");
        const note = relationshipTarget(
          notes[0],
          target,
          "notesSlide",
          NOTES_PART,
        );
        if (!note || !files[note])
          throw new Error("Missing or unsafe notes relationship");
        output.append(`\n\n[Notes for slide ${index}]\n`);
        documentText(files[note], output);
      } catch {
        issues.add(
          "Some slide notes relationships are invalid, external or unsafe; those notes have not been read.",
        );
      }
    };
    if (ordered) {
      for (const item of ordered) {
        if (output.full) break;
        if (!item.target) {
          output.append(
            `[Slide ${item.index}: unavailable relationship; content not read]\n\n`,
          );
          continue;
        }
        output.append(`[Slide ${item.index}]\n`);
        documentText(files[item.target], output);
        readNotes(item.target, item.index);
        output.append("\n\n");
      }
    } else {
      for (const name of parts.filter(
        (name) => SLIDE_PART.test(name) || NOTES_PART.test(name),
      )) {
        if (output.full) break;
        output.append(
          `[Unmapped ${SLIDE_PART.test(name) ? "slide" : "notes"} part: ${name}]\n`,
        );
        documentText(files[name], output);
        output.append("\n\n");
      }
    }
  }
  if (!output.hasText) throw new OfficeError("empty_office");
  if (output.length > MAX_TEXT)
    issues.add(
      "Office text expansion reached its safety limit across document parts; unread content remains.",
    );
  return {
    text: output.finish(),
    structureCoverage: issues.size ? "partial" : "complete",
    ...(issues.size ? { warning: [...issues].join(" ") } : {}),
  };
}

// Only these XML parts are inflated; embedded objects, images and macros are
// never opened. Validate actual byte counts and CRC as well as ZIP metadata.
function selectedPart(name, extension) {
  if (extension === ".docx") return name === "word/document.xml";
  if (extension === ".xlsx")
    return (
      [
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/sharedStrings.xml",
      ].includes(name) || SHEET_PART.test(name)
    );
  return (
    ["ppt/presentation.xml", "ppt/_rels/presentation.xml.rels"].includes(
      name,
    ) ||
    SLIDE_PART.test(name) ||
    NOTES_PART.test(name) ||
    /^ppt\/slides\/_rels\/[^/\\]+\.xml\.rels$/.test(name)
  );
}
async function readParts(bytes, extension) {
  const buffer = Buffer.from(bytes);
  if (buffer.subarray(0, 8).equals(Buffer.from("d0cf11e0a1b11ae1", "hex")))
    throw new OfficeError(
      buffer.includes(Buffer.from("EncryptedPackage", "utf16le"))
        ? "encrypted_office"
        : "invalid_office",
    );
  const zip = await fromBufferPromise(buffer, {
    lazyEntries: true,
    validateEntrySizes: true,
    strictFileNames: true,
  });
  const files = Object.create(null),
    names = new Set();
  let count = 0,
    total = 0;
  try {
    if (zip.entryCount > 4096) throw new OfficeError("office_expansion");
    for await (const entry of zip.eachEntry()) {
      if (++count > 4096) throw new OfficeError("office_expansion");
      const name = entry.fileName;
      if (entry.isEncrypted()) throw new OfficeError("encrypted_office");
      if (
        names.has(name) ||
        name !== path.posix.normalize(name) ||
        /[\\\u0000-\u001f]/.test(name)
      )
        throw new OfficeError("invalid_office");
      names.add(name);
      if (!selectedPart(name, extension)) continue;
      const maximum = Math.min(
        MAX_XML - total,
        Math.max(1024 * 1024, entry.compressedSize * 500),
      );
      if (entry.uncompressedSize > maximum)
        throw new OfficeError("office_expansion");
      const stream = await zip.openReadStreamPromise(entry);
      const chunks = [];
      let size = 0,
        checksum = 0;
      try {
        for await (const chunk of stream) {
          size += chunk.length;
          if (size > maximum) throw new OfficeError("office_expansion");
          checksum = crc32(chunk, checksum);
          chunks.push(chunk);
        }
      } finally {
        stream.destroy();
      }
      if (checksum !== entry.crc32 || size !== entry.uncompressedSize)
        throw new OfficeError("invalid_office");
      total += size;
      files[name] = Buffer.concat(chunks, size);
    }
    return files;
  } finally {
    zip.close();
  }
}
