// Runs the model's python-pptx code in Pyodide and writes the deck.
//
//   node runner.mjs --assets <dir> --code <code.py> --out <out.pptx> --lint <lint.json>
//
// Exit 0: out.pptx and lint.json (a JSON array of strings) are written.
// Exit 1: stderr is "PythonError: <end of the traceback>" for the model's own
// mistakes or "RunnerError: <reason>" for anything else; nothing is written.
//
// Every package comes from <assets> (assets.json, wheels/, fonts/); nothing is
// downloaded. The host runs this in a sandboxed child process: Pyodide is not
// a security boundary.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import { createCanvas, Image } from "@napi-rs/canvas";
import { loadPyodide, version as pyodideVersion } from "pyodide";

const TRACEBACK_CHARS = 1800;

class RunnerError extends Error {}

// Runs with the model's code in `user_code`; returns the lint messages as JSON.
const RUN = `
import json
import deck
from pptx.parts.chart import ChartPart

def positive_axis_ids(prs):
    """python-pptx writes negative c:axId values; the schema wants unsigned."""
    for part in prs.part.package.iter_parts():
        if not isinstance(part, ChartPart):
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
positive_axis_ids(prs)
prs.save("/tmp/out.pptx")
json.dumps(list(deck.LINT))
`;

const svgToPng = (svg, width, height) => {
  const image = new Image();
  image.src = Buffer.from(svg);
  if (!image.width) throw new Error("figure: the SVG did not parse");
  const canvas = createCanvas(width, height);
  canvas.getContext("2d").drawImage(image, 0, 0, width, height);
  return canvas.toBuffer("image/png").toString("base64");
};

const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
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
  const { values } = parseArgs({
    args: argv,
    options: {
      assets: { type: "string" },
      code: { type: "string" },
      out: { type: "string" },
      lint: { type: "string" },
    },
  });
  for (const flag of ["assets", "code", "out", "lint"])
    if (!values[flag])
      throw new RunnerError(
        "usage: runner.mjs --assets <dir> --code <file> --out <file> --lint <file>",
      );
  return values;
}

async function run({ assets, code, out, lint }) {
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
  py.FS.mkdir("/fonts");
  py.FS.writeFile(
    `/fonts/${path.basename(manifest.font)}`,
    readFileSync(path.join(assets, manifest.font)),
  );
  py.registerJsModule("host", { svg_to_png: svgToPng });

  const globals = py.toPy({ user_code: readFileSync(code, "utf8") });
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
  writeFileSync(2, `${text}\n`);
  process.exit(1);
}
process.on("uncaughtException", fail);
process.on("unhandledRejection", fail);

try {
  await run(main(process.argv.slice(2)));
  process.exit(0);
} catch (error) {
  fail(error);
}
