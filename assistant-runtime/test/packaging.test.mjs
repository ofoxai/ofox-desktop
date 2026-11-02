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

test("production pruning drops unused PDF.js builds, viewers, decoders and fonts but keeps the legacy runtime", async (t) => {
  const production = await mkdtemp(path.join(tmpdir(), "ofox-prune-"));
  t.after(() => rm(production, { recursive: true, force: true }));
  const modules = path.join(production, "node_modules");
  const files = [
    "pdfjs-dist/build/pdf.mjs",
    "pdfjs-dist/web/pdf_viewer.mjs",
    "pdfjs-dist/image_decoders/pdf.image_decoders.mjs",
    "pdfjs-dist/legacy/web/pdf_viewer.mjs",
    "pdfjs-dist/legacy/image_decoders/pdf.image_decoders.mjs",
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
    "pdfjs-dist/image_decoders",
    "pdfjs-dist/legacy/web",
    "pdfjs-dist/legacy/image_decoders",
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

test("production pruning removes standalone SDK CLI bundles and launchers while preserving the SDK", async (t) => {
  const production = await mkdtemp(path.join(tmpdir(), "ofox-sdk-prune-"));
  t.after(() => rm(production, { recursive: true, force: true }));
  const modules = path.join(production, "node_modules");
  const sdk = "@earendil-works/pi-coding-agent";
  const removed = [
    `${sdk}/dist/bundle/cli.js`,
    `${sdk}/dist/bundle/rpc-entry.js`,
    `${sdk}/dist/bundle/chunks/legacy.js`,
    ".bin/pi",
    ".bin/pi.cmd",
    ".bin/pi.ps1",
  ];
  const kept = [
    `${sdk}/dist/index.js`,
    `${sdk}/dist/core/sdk.js`,
    `${sdk}/LICENSE`,
    ".bin/other-tool",
  ];
  for (const filename of [...removed, ...kept]) {
    const target = path.join(modules, filename);
    await mkdir(path.dirname(target), { recursive: true });
    await writeFile(target, "fixture");
  }

  await pruneProduction(production);

  for (const filename of removed)
    assert.equal(await exists(path.join(modules, filename)), false, filename);
  for (const filename of kept)
    assert.equal(await exists(path.join(modules, filename)), true, filename);
});

test("the runtime pins PDF.js, the native canvas and the MCP client and the bounded Office parsers", async () => {
  const runtime = new URL("../", import.meta.url);
  const manifest = JSON.parse(
    await readFile(new URL("package.json", runtime), "utf8"),
  );
  assert.equal(manifest.dependencies["pdfjs-dist"], "6.4.299");
  assert.equal(manifest.dependencies["@napi-rs/canvas"], "1.0.13");
  assert.equal(manifest.dependencies["@earendil-works/pi-mcp"], "1.4.0");
  const lock = JSON.parse(
    await readFile(new URL("package-lock.json", runtime), "utf8"),
  );
  assert.equal(lock.packages["node_modules/pdfjs-dist"].version, "6.4.299");
  assert.equal(lock.packages["node_modules/@napi-rs/canvas"].version, "1.0.13");
  assert.equal(
    lock.packages["node_modules/@earendil-works/pi-mcp"].version,
    "1.4.0",
  );
  for (const name of Object.keys(lock.packages))
    assert.doesNotMatch(
      name,
      /fflate|word-extractor|mammoth|xlsx|jszip|fast-xml-parser/,
    );
  assert.equal(manifest.dependencies.yauzl, "3.4.0");
  assert.equal(manifest.dependencies.saxes, "5.0.1");
  assert.equal(lock.packages["node_modules/yauzl"].version, "3.4.0");
  assert.equal(lock.packages["node_modules/saxes"].version, "5.0.1");
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

test("the search server's notice names the versions its lockfile bundles", async () => {
  const notice = await readFile(
    new URL("../notices/ofox-search-mcp.txt", import.meta.url),
    "utf8",
  );
  const lock = JSON.parse(
    await readFile(
      new URL("../search-mcp/package-lock.json", import.meta.url),
      "utf8",
    ),
  );
  // search-mcp's own test checks every package of the built bundle.
  for (const name of ["@modelcontextprotocol/sdk", "zod", "undici"]) {
    const { version, license } = lock.packages[`node_modules/${name}`];
    assert.ok(
      notice.includes(`\n${name} ${version} (${license})\n`),
      `${name} ${version}`,
    );
  }
  assert.match(notice, /Copyright \(c\) 2024 Anthropic, PBC/);
});

test("the Office notice covers exactly the shipped Office packages, with their license texts", async () => {
  const runtime = new URL("../", import.meta.url);
  const lock = JSON.parse(
    await readFile(new URL("package-lock.json", runtime), "utf8"),
  );
  const notice = await readFile(new URL("notices/office.txt", runtime), "utf8");
  const remaining = ["yauzl", "saxes"],
    shipped = new Set();
  while (remaining.length) {
    const name = remaining.pop();
    if (shipped.has(name)) continue;
    shipped.add(name);
    const pkg = lock.packages[`node_modules/${name}`];
    assert.match(pkg.license, /^(MIT|ISC)$/, name);
    assert.ok(
      notice.includes(`\n${name} ${pkg.version} (${pkg.license})\n`),
      name,
    );
    // A package's own license file is reproduced verbatim.
    const own = new URL(`node_modules/${name}/LICENSE`, runtime);
    if (await exists(own))
      assert.ok(notice.includes((await readFile(own, "utf8")).trim()), name);
    remaining.push(...Object.keys(pkg.dependencies ?? {}));
  }
  assert.deepEqual([...shipped].sort(), ["pend", "saxes", "xmlchars", "yauzl"]);
  // No package that does not ship is named.
  for (const [, name] of notice.matchAll(/^(\S+) \S+ \((?:MIT|ISC)\)$/gm))
    assert.ok(shipped.has(name), name);
  for (const marker of [
    "Isaac Z. Schlueter and Contributors", // sax, in saxes' repository LICENSE
    "2013 Brian J. Brennan", // buffer-crc32, vendored in yauzl
    "2014 Andrew Kelley", // fd-slicer, vendored in yauzl
  ])
    assert.ok(notice.includes(marker), marker);
});
