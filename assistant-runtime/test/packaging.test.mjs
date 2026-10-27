import assert from "node:assert/strict";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { pruneProduction } from "../../scripts/assistant-prune.mjs";

const exists = (file) =>
  access(file).then(
    () => true,
    () => false,
  );

test("production pruning drops unused PDF.js builds and fonts but keeps the legacy runtime", async (t) => {
  const production = await mkdtemp(path.join(tmpdir(), "ofox-prune-"));
  t.after(() => rm(production, { recursive: true, force: true }));
  const modules = path.join(production, "node_modules");
  const files = [
    "pdfjs-dist/build/pdf.mjs",
    "pdfjs-dist/web/pdf_viewer.mjs",
    "pdfjs-dist/standard_fonts/LiberationSans-Regular.ttf",
    "pdfjs-dist/standard_fonts/LICENSE_LIBERATION",
    "pdfjs-dist/legacy/build/pdf.mjs",
    "pdfjs-dist/legacy/build/pdf.worker.mjs",
    "pdfjs-dist/legacy/build/pdf.mjs.map",
    "pdfjs-dist/cmaps/UniGB-UCS2-H.bcmap",
    "pdfjs-dist/iccs/LICENSE",
    "pdfjs-dist/wasm/openjpeg.wasm",
    "pdfjs-dist/LICENSE",
    "@earendil-works/pi-coding-agent/examples/doom/doom.wasm",
    "@earendil-works/pi-coding-agent/dist/index.js",
    "@napi-rs/canvas/index.js",
  ];
  for (const file of files) {
    const target = path.join(modules, file);
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, "fixture");
  }
  await pruneProduction(production);
  for (const removed of [
    "pdfjs-dist/build",
    "pdfjs-dist/web",
    "pdfjs-dist/standard_fonts",
    "pdfjs-dist/legacy/build/pdf.mjs.map",
    "@earendil-works/pi-coding-agent/examples",
  ])
    assert.equal(await exists(path.join(modules, removed)), false, removed);
  for (const kept of [
    "pdfjs-dist/legacy/build/pdf.mjs",
    "pdfjs-dist/legacy/build/pdf.worker.mjs",
    "pdfjs-dist/cmaps/UniGB-UCS2-H.bcmap",
    "pdfjs-dist/iccs/LICENSE",
    "pdfjs-dist/wasm/openjpeg.wasm",
    "pdfjs-dist/LICENSE",
    "@earendil-works/pi-coding-agent/dist/index.js",
    "@napi-rs/canvas/index.js",
  ])
    assert.equal(await exists(path.join(modules, kept)), true, kept);
});

test("the runtime pins PDF.js and the native canvas and ships no Office parsers", async () => {
  const runtime = new URL("../", import.meta.url);
  const manifest = JSON.parse(
    await readFile(new URL("package.json", runtime), "utf8"),
  );
  assert.equal(manifest.dependencies["pdfjs-dist"], "6.4.299");
  assert.equal(manifest.dependencies["@napi-rs/canvas"], "1.0.13");
  const lock = JSON.parse(
    await readFile(new URL("package-lock.json", runtime), "utf8"),
  );
  assert.equal(lock.packages["node_modules/pdfjs-dist"].version, "6.4.299");
  assert.equal(lock.packages["node_modules/@napi-rs/canvas"].version, "1.0.13");
  for (const name of Object.keys(lock.packages))
    assert.doesNotMatch(name, /fflate|fast-xml-parser|mammoth|xlsx/);
});

test("third-party notices cover the native canvas and PDF.js", async () => {
  const notices = new URL("../notices/", import.meta.url);
  const canvas = await readFile(new URL("napi-rs-canvas.txt", notices), "utf8");
  for (const component of [
    "@napi-rs/canvas",
    "Mozilla Public License",
    "cssparser",
    "cssparser-macros",
    "dtoa-short",
    "selectors",
    "FreeType Project",
    "ICU",
    "Independent JPEG Group",
    "PNG Reference Library",
    "UNICODE LICENSE V3",
    "Jean-loup Gailly and Mark Adler",
  ])
    assert.ok(canvas.includes(component), component);
  const pdfjs = await readFile(new URL("pdfjs-dist.txt", notices), "utf8");
  assert.match(pdfjs, /pdfjs-dist 6\.4\.299/);
  assert.match(pdfjs, /Apache License\s+Version 2\.0/);
});
