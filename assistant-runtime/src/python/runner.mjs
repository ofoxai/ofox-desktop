// Runs the model's python-pptx code in Pyodide and writes the deck.
//
//   node runner.mjs --assets <dir> --code <code.py> --out <out.pptx> --lint <lint.json>
// Optional host-only --selected-style <id> locks normal deck helpers to that style.
//
// Exit 0: out.pptx and lint.json (a JSON array of strings) are written.
// Exit 1: recoverable code, argument or private-input failure. Source failures
// include assistant_presentation_source_unavailable on stderr; no host paths
// are revealed. Code errors end with PythonError or RunnerError.
// Exit 2: assets, Pyodide or another engine dependency failed to load
// before the code ran: "RunnerError: <reason>". The
// engine is broken, not the code.
// The report starts on a line of its own, after whatever the code printed.
//
// Every package comes from <assets> (assets.json, wheels/, fonts/); nothing is
// downloaded. The host runs this in a sandboxed child process: Pyodide is not
// a security boundary.
import {
  existsSync,
  lstatSync,
  readFileSync,
  writeFileSync,
  openSync,
  closeSync,
  fstatSync,
  constants,
} from "node:fs";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { createCanvas, Image } from "@napi-rs/canvas";
import { loadPyodide, version as pyodideVersion } from "pyodide";
import { checkedPresentationAssets } from "../presentation-planning.mjs";
import { SVG_TEXT_FONTS } from "../thumbnails.mjs";

const TRACEBACK_CHARS = 1800;

class RunnerError extends Error {}
class PrivateInputError extends Error {}
class SourceInputError extends PrivateInputError {
  constructor(reason) {
    super("assistant_presentation_source_unavailable: " + reason);
  }
}

// Input preflight has its own recoverable class; other startup failures
// belong to the engine until the model's code starts.
let codeStarted = false;

// Runs with the model's code in `user_code`; returns the lint messages as JSON.
const RUN = `
import json
selected_style = json.loads(selected_style_json)
import deck
from pptx.parts.chart import ChartPart

def positive_axis_ids(prs, allowed=None):
    """python-pptx writes negative c:axId values; the schema wants unsigned."""
    for part in prs.part.package.iter_parts():
        if not isinstance(part, ChartPart) or allowed is not None and part not in allowed:
            continue
        ids = part._element.xpath(".//c:axId | .//c:crossAx")
        used = {int(e.get("val")) for e in ids if int(e.get("val")) >= 0}
        mapped = {}
        for e in ids:
            old = int(e.get("val"))
            if old >= 0:
                continue
            if old not in mapped:
                new = -old
                while new in used:
                    new += 1
                used.add(new)
                mapped[old] = new
            e.set("val", str(mapped[old]))

namespace = {"__name__": "__main__"}
exec("from deck import *", namespace)
try:
    exec(compile(user_code, "deck_code.py", "exec"), namespace)
except SystemExit:
    # Pyodide would end the whole process from a timer, outside our catch.
    raise RuntimeError("the code called sys.exit(); remove it")
prs = namespace.get("prs")
if prs is None:
    raise RuntimeError("the code must create prs = new_deck(...)")
if selected_style is not None:
    context = getattr(prs, "_ofox_context", None)
    if not isinstance(context, dict) or context.get("style") != selected_style:
        raise RuntimeError("selected style requires a matching new_deck presentation")
deck._validate_source(prs)
positive_axis_ids(prs, deck._source_charts(prs))
deck._validate_media(prs)
deck._validate_layout(prs)
deck._validate_panel_regions(prs)
deck._validate_density(prs)
deck._validate_title_cover(prs)
deck._plan_receipt(prs)
prs.save("/tmp/out.pptx")
json.dumps(list(deck.LINT))
`;

// The canvas's SVG reader draws all of a text with the first installed family
// of its list, with no per-glyph fallback: Chinese in a figure became boxes,
// whether no family was named or a Latin one was ("Georgia, SimSun, serif").
// A root <svg> that names no font gets the CJK-capable list; when the figure
// holds CJK text, every family list it names gets a CJK font of the same kind
// put first, so its Latin text keeps a serif or sans look. A figure without
// CJK text keeps the fonts it asked for.
const CJK_TEXT = /[\u3040-\u30ff\u3400-\u9fff\uac00-\ud7af\uff00-\uffef]/;
const CJK_SERIF = "'Songti SC', 'STSong', 'Noto Serif CJK SC', 'SimSun'";
const CJK_SANS = SVG_TEXT_FONTS.replace(/,\s*sans-serif$/, "");
const SERIF_FAMILY =
  /(^|[\s,'"])(serif|georgia|times|garamond|playfair|merriweather|lora|baskerville|didot|bodoni|song|simsun|mincho|ming|serif cjk)/i;
const withCjkFirst = (list) => {
  const family = list.trim();
  if (/^(['"]?)(PingFang|Hiragino|Songti|STSong|Noto (Sans|Serif) CJK)/i.test(family))
    return family;
  const serif = SERIF_FAMILY.test(family) && !/sans-serif/i.test(family);
  return `${serif ? CJK_SERIF : CJK_SANS}, ${family}`;
};
const withRootFont = (svg) => {
  const rooted = svg.replace(/<svg\b[^>]*>/, (tag) =>
    /\sfont-family\s*=/.test(tag)
      ? tag
      : tag.replace(/^<svg\b/, `<svg font-family="${SVG_TEXT_FONTS}"`),
  );
  if (!CJK_TEXT.test(svg)) return rooted;
  return rooted
    .replace(
      /(\sfont-family\s*=\s*)(["'])([^"']*)\2/g,
      (_, head, quote, list) =>
        `${head}${quote}${withCjkFirst(list).replace(new RegExp(quote, "g"), quote === '"' ? "'" : '"')}${quote}`,
    )
    .replace(
      /(font-family\s*:\s*)([^;}"<]+)/g,
      (_, head, list) => `${head}${withCjkFirst(list)}`,
    );
};

const svgToPng = (svg, width, height) => {
  const image = new Image();
  image.src = Buffer.from(withRootFont(String(svg)));
  if (!image.width) throw new Error("figure: the SVG did not parse");
  const canvas = createCanvas(width, height);
  canvas.getContext("2d").drawImage(image, 0, 0, width, height);
  return canvas.toBuffer("image/png").toString("base64");
};

const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
function privateBytes(file, limit) {
  if (!path.isAbsolute(file) || lstatSync(file).isSymbolicLink())
    throw new Error("invalid private input");
  const fd = openSync(file, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try {
    const metadata = fstatSync(fd);
    if (!metadata.isFile() || metadata.size > limit)
      throw new Error("private input bounds");
    const bytes = readFileSync(fd);
    if (bytes.length > limit) throw new Error("private input bounds");
    return bytes;
  } finally {
    closeSync(fd);
  }
}
// Pyodide refuses a Buffer where it wants a plain Uint8Array.
const readBytes = (file) => {
  const bytes = readFileSync(file);
  return new Uint8Array(bytes.buffer, bytes.byteOffset, bytes.byteLength);
};

// The wheels Pyodide loads for `names` and their dependencies, by file name.
function lockedWheels(lock, names) {
  const wheels = new Set();
  const seen = new Set();
  const visit = (name) => {
    if (seen.has(name)) return;
    seen.add(name);
    const entry = lock.packages[name];
    if (!entry) throw new RunnerError(`${name} is not in the Pyodide lock`);
    wheels.add(entry.file_name);
    entry.depends.forEach(visit);
  };
  names.forEach(visit);
  return [...wheels];
}

function main(argv) {
  let values;
  try {
    ({ values } = parseArgs({
      args: argv,
      options: {
        assets: { type: "string" },
        code: { type: "string" },
        out: { type: "string" },
        lint: { type: "string" },
        "selected-style": { type: "string" },
        "media-manifest": { type: "string" },
        "source-deck": { type: "string" },
        "source-meta": { type: "string" },
      },
    }));
  } catch {
    throw new PrivateInputError("invalid runner arguments");
  }
  for (const flag of ["assets", "code", "out", "lint"])
    if (!values[flag])
      throw new PrivateInputError(
        "usage: runner.mjs --assets <dir> --code <file> --out <file> --lint <file>",
      );
  return values;
}

async function run({
  assets,
  code,
  out,
  lint,
  "selected-style": selectedStyle,
  "media-manifest": mediaManifest,
  "source-deck": sourceDeck,
  "source-meta": sourceMetaPath,
}) {
  let userCode;
  try {
    userCode = readFileSync(code, "utf8");
  } catch {
    throw new PrivateInputError("invalid Python code input");
  }
  let sourceMeta = null,
    sourceBytes = null;
  if (sourceDeck !== undefined || sourceMetaPath !== undefined) {
    try {
      if (sourceDeck === undefined || sourceMetaPath === undefined)
        throw new Error("pair");
      sourceBytes = privateBytes(sourceDeck, 25 * 1024 * 1024);
      sourceMeta = JSON.parse(
        privateBytes(sourceMetaPath, 64 * 1024).toString("utf8"),
      );
      if (
        Object.keys(sourceMeta).some(
          (key) => !["version", "totalPages", "pageNumbers"].includes(key),
        ) ||
        !/^[a-f0-9]{64}$/.test(sourceMeta.version ?? "") ||
        sourceMeta.version !==
          createHash("sha256").update(sourceBytes).digest("hex") ||
        !Number.isInteger(sourceMeta.totalPages) ||
        sourceMeta.totalPages < 1 ||
        sourceMeta.totalPages > 1000 ||
        !Array.isArray(sourceMeta.pageNumbers) ||
        sourceMeta.pageNumbers.length < 1 ||
        sourceMeta.pageNumbers.length > 30 ||
        new Set(sourceMeta.pageNumbers).size !==
          sourceMeta.pageNumbers.length ||
        sourceMeta.pageNumbers.some(
          (page) =>
            !Number.isInteger(page) || page < 1 || page > sourceMeta.totalPages,
        )
      )
        throw new Error("metadata");
    } catch {
      throw new SourceInputError("invalid source deck or source metadata");
    }
  }
  let media = { assets: [] };
  if (mediaManifest !== undefined) {
    try {
      const metadata = lstatSync(mediaManifest);
      if (
        !metadata.isFile() ||
        metadata.isSymbolicLink() ||
        metadata.size > 28 * 1024 * 1024
      )
        throw new Error("bounds");
      media = readJson(mediaManifest);
      if (
        Object.keys(media).some(
          (key) =>
            ![
              "planId",
              "revision",
              "assets",
              "pageAssetIds",
              "pageFallbacks",
              "planPageNumbers",
            ].includes(key),
        ) ||
        !/^[A-Za-z0-9_-]{1,80}$/.test(media.planId ?? "") ||
        !Number.isSafeInteger(media.revision) ||
        media.revision < 1 ||
        !Array.isArray(media.assets) ||
        !Array.isArray(media.pageAssetIds) ||
        media.pageAssetIds.length < 1 ||
        media.pageAssetIds.length > 30 ||
        media.pageAssetIds.some(
          (assetId) =>
            assetId !== null &&
            (typeof assetId !== "string" ||
              !/^[A-Za-z0-9_-]{1,80}$/.test(assetId)),
        ) ||
        media.assets.some(
          (asset) => !media.pageAssetIds.includes(asset.assetId),
        ) ||
        !Array.isArray(media.pageFallbacks) ||
        media.pageFallbacks.length !== media.pageAssetIds.length ||
        media.pageFallbacks.some(
          (fallback) => !["native-summary", "omit", "stop"].includes(fallback),
        )
      )
        throw new Error("scope");
      checkedPresentationAssets(
        media,
        media.assets.map((asset) => asset.assetId),
      );
      const expectedPages =
        sourceMeta?.pageNumbers ??
        media.pageAssetIds.map((_, index) => index + 1);
      if (
        (sourceMeta && !Array.isArray(media.planPageNumbers)) ||
        (media.planPageNumbers !== undefined &&
          JSON.stringify(media.planPageNumbers) !==
            JSON.stringify(expectedPages)) ||
        expectedPages.length !== media.pageAssetIds.length
      )
        throw new Error("page mapping");
      media.planPageNumbers = expectedPages;
    } catch {
      if (sourceMeta)
        throw new SourceInputError("source page media manifest is invalid");
      throw new PrivateInputError("invalid media manifest or asset");
    }
  }
  const catalogBytes = readFileSync(
    new URL(
      "../../skills/ofox-office-pptx/resources/catalog.json",
      import.meta.url,
    ),
  );
  if (selectedStyle !== undefined) {
    const enabled = JSON.parse(catalogBytes).styles.filter(
      (item) =>
        item.status === "enabled" &&
        item.enabled === true &&
        item.adapted === true &&
        item.licenseReviewed === true &&
        item.theme &&
        typeof item.theme === "object" &&
        !Array.isArray(item.theme),
    );
    if (
      selectedStyle !== "navy" &&
      !enabled.some((item) => item.id === selectedStyle)
    )
      throw new PrivateInputError("unknown or disabled selected style");
  }
  const manifest = readJson(path.join(assets, "assets.json"));
  if (manifest.pyodide !== pyodideVersion)
    throw new RunnerError(
      `the assets are for Pyodide ${manifest.pyodide}, this is ${pyodideVersion}`,
    );
  const lockFile = path.join(
    path.dirname(fileURLToPath(import.meta.resolve("pyodide"))),
    "pyodide-lock.json",
  );
  const lockText = readFileSync(lockFile, "utf8");
  const wheels = [
    ...lockedWheels(JSON.parse(lockText), manifest.binary),
    ...manifest.pure,
  ].map((file) => `wheels/${file}`);
  const missing = [...wheels, manifest.font].filter(
    (file) => !existsSync(path.join(assets, file)),
  );
  if (missing.length) throw new RunnerError(`missing ${missing.join(", ")}`);

  // The wheels folder is the only package source: loadPackage reads from it
  // and has no other place to look.
  const py = await loadPyodide({
    packageBaseUrl: path.join(assets, "wheels"),
    lockFileContents: lockText,
  });
  await py.loadPackage(manifest.binary, { messageCallback: () => {} });
  for (const file of manifest.pure)
    py.unpackArchive(readBytes(path.join(assets, "wheels", file)), "wheel");

  const site = py.runPython("import sysconfig; sysconfig.get_path('purelib')");
  py.FS.writeFile(
    path.join(site, "deck.py"),
    readFileSync(new URL("./deck.py", import.meta.url)),
  );
  py.FS.writeFile(
    "/template.pptx",
    readFileSync(new URL("./template.pptx", import.meta.url)),
  );
  // Public, packaged data only. User Python sees an in-memory JSON file,
  // never a host directory or an arbitrary path supplied by model code.
  py.FS.writeFile("/styles.json", catalogBytes);
  py.FS.writeFile(
    "/selection.json",
    JSON.stringify({ selectedStyle: selectedStyle ?? null }),
  );
  // Only validated, authorized image bytes enter the virtual filesystem.
  // Neither this JSON nor any Python helper reveals the host manifest path.
  py.FS.writeFile("/media.json", JSON.stringify(media));
  if (sourceMeta) {
    py.FS.writeFile("/source.pptx", new Uint8Array(sourceBytes));
    py.FS.writeFile("/source-meta.json", JSON.stringify(sourceMeta));
  }
  py.FS.mkdir("/fonts");
  py.FS.writeFile(
    `/fonts/${path.basename(manifest.font)}`,
    readFileSync(path.join(assets, manifest.font)),
  );
  py.registerJsModule("host", { svg_to_png: svgToPng });
  // What the code's prelude imports loads now, so that a package that does
  // not load is the engine's failure, not the code's.
  py.runPython("import deck, pptx.parts.chart");
  if (sourceMeta) {
    try {
      py.runPython("deck._prepare_source()");
    } catch {
      throw new SourceInputError(
        "unsupported source deck structure or metadata",
      );
    }
  }

  const globals = py.toPy({
    user_code: userCode,
    selected_style_json: JSON.stringify(selectedStyle ?? null),
  });
  codeStarted = true;
  const messages = await py.runPythonAsync(RUN, { globals });
  writeFileSync(out, py.FS.readFile("/tmp/out.pptx"));
  writeFileSync(lint, messages);
}

// Every failure ends here as one bounded block on stderr. Pyodide raises some
// of them (os._exit, a fatal error) from a timer, outside the try below, so
// the process-level handlers report them the same way.
function fail(error) {
  const message = String(error?.message ?? error).trim();
  const text =
    error?.name === "PythonError"
      ? `PythonError: ${message.slice(-TRACEBACK_CHARS)}`
      : `RunnerError: ${message.slice(0, TRACEBACK_CHARS)}`;
  writeFileSync(2, `\n${text}\n`);
  process.exit(codeStarted || error instanceof PrivateInputError ? 1 : 2);
}
process.on("uncaughtException", fail);
process.on("unhandledRejection", fail);

try {
  await run(main(process.argv.slice(2)));
  process.exit(0);
} catch (error) {
  fail(error);
}
