import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  access,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import yauzl from "yauzl";
import {
  PYTHON_PACKAGE_PATHS,
  pruneProduction,
} from "../../scripts/assistant-prune.mjs";
import {
  PYODIDE_BASE_URL,
  pyodideWheelUrl,
  pythonEngineOffered,
} from "../../scripts/python-assets.mjs";

const exists = (file) =>
  access(file).then(
    () => true,
    () => false,
  );

test("development and packaged runtimes resolve Pi 1.5 and patched undici", async () => {
  for (const tree of ["../", "../resources/runtime/"]) {
    const runtime = new URL(tree, import.meta.url);
    for (const name of ["pi-ai", "pi-coding-agent", "pi-mcp", "pi-codemode"]) {
      const installed = JSON.parse(
        await readFile(
          new URL(`node_modules/@earendil-works/${name}/package.json`, runtime),
          "utf8",
        ),
      );
      assert.equal(installed.version, "1.5.0", `${tree}${name}`);
    }
    const lock = JSON.parse(
      await readFile(new URL("package-lock.json", runtime), "utf8"),
    );
    const clients = Object.keys(lock.packages).filter((name) =>
      name.endsWith("node_modules/undici"),
    );
    assert.ok(clients.length > 0);
    for (const client of clients) {
      const { version } = JSON.parse(
        await readFile(new URL(`${client}/package.json`, runtime), "utf8"),
      );
      const [major, minor, patch] = version.split(".").map(Number);
      assert.ok(
        major > 8 ||
          (major === 8 && (minor > 12 || (minor === 12 && patch >= 3))),
        `${tree}${client} ${version} must be at least 8.12.3`,
      );
    }
  }
});

test("the packaged sidecar and its manifest report the installed Pi 1.5 SDK", async () => {
  const resources = new URL("../resources/", import.meta.url);
  const manifest = JSON.parse(
    await readFile(new URL("manifest.json", resources), "utf8"),
  );
  const result = spawnSync(
    process.execPath,
    [fileURLToPath(new URL(manifest.entry, resources))],
    {
      input: '{"id":1,"method":"health"}\n',
      encoding: "utf8",
      env: process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {},
      timeout: 10000,
    },
  );
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stderr, "");
  const health = JSON.parse(result.stdout.trim()).result;
  assert.equal(manifest.sdkVersion, "1.5.0");
  assert.deepEqual(health, { protocolVersion: 1, sdkVersion: "1.5.0" });
});

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
  assert.equal(manifest.dependencies["@earendil-works/pi-mcp"], "1.5.0");
  const lock = JSON.parse(
    await readFile(new URL("package-lock.json", runtime), "utf8"),
  );
  assert.equal(lock.packages["node_modules/pdfjs-dist"].version, "6.4.299");
  assert.equal(lock.packages["node_modules/@napi-rs/canvas"].version, "1.0.13");
  assert.equal(
    lock.packages["node_modules/@earendil-works/pi-mcp"].version,
    "1.5.0",
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

const engineNames = ["officecli", "typst"];
const platform = `${process.platform}-${process.arch}`;
const readJson = async (url) => JSON.parse(await readFile(url, "utf8"));

test("manifest lists both engines with the pinned versions", async () => {
  const resources = new URL("../resources/", import.meta.url);
  const manifest = await readJson(new URL("manifest.json", resources));
  const downloads = await readJson(
    new URL("../engine-downloads.json", import.meta.url),
  );
  for (const name of engineNames) {
    // An engine with no build for this platform is left out of the manifest.
    if (!downloads[name][platform]) {
      assert.equal(manifest.engines?.[name], undefined, name);
      continue;
    }
    const engine = manifest.engines[name];
    assert.equal(engine.version, downloads[name].version, name);
    const file = new URL(engine.path, resources);
    assert.equal(
      path.basename(engine.path),
      downloads[name][platform].executable.split("/").at(-1),
    );
    assert.ok(await exists(file), `${engine.path} must ship`);
    if (process.platform !== "win32")
      assert.ok((await stat(file)).mode & 0o111, `${engine.path} executable`);
    assert.ok(
      await exists(new URL(`${path.dirname(engine.path)}/LICENSE`, resources)),
      `${name} LICENSE must ship beside its executable`,
    );
  }
});

test("officecli and typst notices name the version and the Apache license", async () => {
  const notices = new URL("../notices/", import.meta.url);
  for (const [name, version] of [
    ["officecli", /officecli 1\.0\.160/],
    ["typst", /typst 0\.15\.1/],
  ]) {
    const notice = await readFile(new URL(`${name}.txt`, notices), "utf8");
    assert.match(notice, version, name);
    assert.match(notice, /Apache License\s+Version 2\.0/, name);
  }
});

test("engine stamp matches the pinned digests", async () => {
  const downloads = await readJson(
    new URL("../engine-downloads.json", import.meta.url),
  );
  const stamp = await readJson(
    new URL("../resources/engines/source.json", import.meta.url),
  );
  for (const name of engineNames) {
    const pinned = downloads[name][platform];
    if (!pinned) {
      assert.equal(stamp[name], undefined, name);
      continue;
    }
    assert.match(pinned.sha256, /^[0-9a-f]{64}$/, name);
    assert.deepEqual(stamp[name], {
      version: downloads[name].version,
      platform,
      sha256: pinned.sha256,
    });
  }
});

// The Python engine: Pyodide's own wheels and pure-Python wheels pinned by
// scripts/python-downloads.json, plus a CJK font. Staged on macOS only.
const sha256 = (bytes) => createHash("sha256").update(bytes).digest("hex");
const pythonExpected = {
  binary: {
    numpy: "2.4.6",
    matplotlib: "3.10.8",
    contourpy: "1.3.3",
    cycler: "0.12.1",
    fonttools: "4.62.1",
    kiwisolver: "1.5.0",
    packaging: "26.1",
    pyparsing: "3.3.2",
    "python-dateutil": "2.9.0.post0",
    pytz: "2026.1.post1",
    six: "1.17.0",
    lxml: "6.1.3",
    pillow: "12.2.0",
    "typing-extensions": "4.15.0",
  },
  pure: {
    "python-pptx": "1.0.2",
    "python-docx": "1.2.0",
    openpyxl: "3.1.5",
    xlsxwriter: "3.2.9",
    "et-xmlfile": "2.0.0",
  },
};

test("python assets are pinned and staged", async () => {
  const downloadsUrl = new URL(
    "../../scripts/python-downloads.json",
    import.meta.url,
  );
  const downloads = await readJson(downloadsUrl);
  const lock = await readJson(
    new URL("../node_modules/pyodide/pyodide-lock.json", import.meta.url),
  );

  // Pyodide itself is a dependency of the runtime at the same version.
  assert.equal(downloads.pyodide, "314.0.7");
  const runtimePackage = await readJson(
    new URL("../package.json", import.meta.url),
  );
  assert.equal(runtimePackage.dependencies.pyodide, downloads.pyodide);
  const runtimeLock = await readJson(
    new URL("../package-lock.json", import.meta.url),
  );
  assert.equal(
    runtimeLock.packages["node_modules/pyodide"].version,
    downloads.pyodide,
  );

  // Every pin names itself, downloads over https and carries a SHA-256.
  const wheelsAndFont = [
    ...downloads.binary,
    ...downloads.pure,
    downloads.font,
  ];
  for (const pin of [...wheelsAndFont, downloads.font.license]) {
    assert.ok(pin.name, JSON.stringify(pin));
    assert.match(pin.url, /^https:\/\//, pin.name);
    assert.match(pin.sha256, /^[0-9a-f]{64}$/, pin.name);
  }
  // The staged file is named as the download is.
  for (const pin of wheelsAndFont)
    assert.equal(path.posix.basename(new URL(pin.url).pathname), pin.file);

  // Each required package is pinned exactly once at the required version.
  for (const [group, expected] of Object.entries(pythonExpected)) {
    const names = downloads[group].map((pin) => pin.name);
    assert.deepEqual(names.toSorted(), Object.keys(expected).toSorted(), group);
    for (const pin of downloads[group])
      assert.equal(pin.version, expected[pin.name], pin.name);
  }

  // A binary wheel is exactly what Pyodide's lock file names, at its own CDN.
  for (const pin of downloads.binary) {
    const locked = lock.packages[pin.name];
    assert.ok(locked, `${pin.name} is not in pyodide-lock.json`);
    assert.equal(locked.version, pin.version, pin.name);
    assert.equal(locked.file_name, pin.file, pin.name);
    assert.equal(locked.sha256, pin.sha256, pin.name);
    assert.equal(pin.url, `${PYODIDE_BASE_URL}${locked.file_name}`, pin.name);
  }
  for (const pin of downloads.pure)
    assert.match(
      pin.url,
      /^https:\/\/files\.pythonhosted\.org\/packages\//,
      pin.name,
    );

  // The font and its license come from a release tag, never a moving branch.
  assert.match(
    downloads.font.url,
    /^https:\/\/github\.com\/notofonts\/noto-cjk\/raw\/Sans\d+\.\d+\/Sans\/SubsetOTF\/SC\/NotoSansSC-Regular\.otf$/,
  );
  assert.match(
    downloads.font.license.url,
    /^https:\/\/raw\.githubusercontent\.com\/notofonts\/noto-cjk\/Sans\d+\.\d+\/LICENSE$/,
  );

  // Staging: where the engine is offered every listed file ships; elsewhere
  // nothing of it is in the manifest or the stamp.
  const resources = new URL("../resources/", import.meta.url);
  const manifest = await readJson(new URL("manifest.json", resources));
  const stamps = await readJson(new URL("engines/source.json", resources));
  if (!downloads.platforms.includes(platform)) {
    assert.equal(manifest.engines?.python, undefined);
    assert.equal(stamps.python, undefined);
    return;
  }
  const staged = new URL("engines/python/", resources);
  const assets = await readJson(new URL("assets.json", staged));
  assert.deepEqual(assets, {
    pyodide: downloads.pyodide,
    binary: downloads.binary.map((pin) => pin.name),
    pure: downloads.pure.map((pin) => pin.file),
    font: "fonts/NotoSansSC-Regular.otf",
  });
  const shipped = [
    ...[...downloads.binary, ...downloads.pure].map((pin) => [
      `wheels/${pin.file}`,
      pin.sha256,
    ]),
    [assets.font, downloads.font.sha256],
    [`fonts/${downloads.font.license.file}`, downloads.font.license.sha256],
  ];
  for (const [file, sha] of shipped) {
    assert.ok(await exists(new URL(file, staged)), `${file} must ship`);
    assert.equal(sha256(await readFile(new URL(file, staged))), sha, file);
  }
  assert.deepEqual(manifest.engines.python, {
    path: "engines/python/assets.json",
    runner: "runtime/src/python/runner.mjs",
    version: downloads.pyodide,
  });
  assert.deepEqual(stamps.python, {
    version: downloads.pyodide,
    sha256: sha256(await readFile(downloadsUrl)),
  });
});

test("OFOX_PYODIDE_BASE_URL replaces only the Pyodide CDN base of a wheel", () => {
  const wheel = `${PYODIDE_BASE_URL}numpy-2.4.6-cp314-cp314-pyemscripten_2026_0_wasm32.whl`;
  const pin = { file: path.posix.basename(wheel), url: wheel };
  assert.equal(pyodideWheelUrl(pin, {}), wheel);
  assert.equal(pyodideWheelUrl(pin, { OFOX_PYODIDE_BASE_URL: "" }), wheel);
  const mirror = "https://gcore.jsdelivr.net/pyodide/v314.0.7/full/";
  const mirrored = `${mirror}${pin.file}`;
  assert.equal(
    pyodideWheelUrl(pin, { OFOX_PYODIDE_BASE_URL: mirror }),
    mirrored,
  );
  // A missing trailing slash is added, not glued onto the file name.
  assert.equal(
    pyodideWheelUrl(pin, { OFOX_PYODIDE_BASE_URL: mirror.slice(0, -1) }),
    mirrored,
  );
});

test("the Python engine is offered on macOS only, and its npm package is pruned elsewhere", async (t) => {
  const downloads = await readJson(
    new URL("../../scripts/python-downloads.json", import.meta.url),
  );
  for (const [target, offered] of Object.entries({
    "darwin-arm64": true,
    "darwin-x64": true,
    "win32-x64": false,
    "linux-x64": false,
  }))
    assert.equal(pythonEngineOffered(target, downloads), offered, target);
  assert.deepEqual(PYTHON_PACKAGE_PATHS, ["pyodide"]);

  const production = await mkdtemp(path.join(tmpdir(), "ofox-python-prune-"));
  t.after(() => rm(production, { recursive: true, force: true }));
  const modules = path.join(production, "node_modules");
  const files = [
    "pyodide/pyodide.asm.wasm",
    "pyodide/pyodide-lock.json",
    "pdfjs-dist/legacy/build/pdf.mjs",
    "ws/index.js",
  ];
  const stage = async () => {
    await rm(modules, { recursive: true, force: true });
    for (const file of files) {
      await mkdir(path.dirname(path.join(modules, file)), { recursive: true });
      await writeFile(path.join(modules, file), "fixture");
    }
  };
  // Offered: the package stays. Not offered: only the package goes.
  for (const options of [undefined, { pythonEngine: true }]) {
    await stage();
    await pruneProduction(production, options);
    for (const file of files)
      assert.equal(await exists(path.join(modules, file)), true, file);
  }
  await stage();
  await pruneProduction(production, { pythonEngine: false });
  assert.equal(await exists(path.join(modules, "pyodide")), false);
  for (const file of files.filter((file) => !file.startsWith("pyodide/")))
    assert.equal(await exists(path.join(modules, file)), true, file);

  // The tree this machine prepared follows the same decision.
  assert.equal(
    await exists(
      new URL("../resources/runtime/node_modules/pyodide/", import.meta.url),
    ),
    pythonEngineOffered(platform, downloads),
  );
});

// The license files a wheel carries, as the notice must reproduce them.
const wheelLicensePattern =
  /\.dist-info\/(licen[sc]e[^/]*|copying[^/]*|licenses\/.+)$|mpl-data\/fonts\/ttf\/LICENSE_[A-Z]+$/i;
const noticeText = (text) =>
  text
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .join("\n")
    .trim();

function licensesInWheel(file) {
  return new Promise((resolve, reject) => {
    yauzl.open(file, { lazyEntries: true }, (error, zip) => {
      if (error) return reject(error);
      const found = [];
      zip.on("error", reject);
      zip.on("end", () => resolve(found));
      zip.on("entry", (entry) => {
        if (!wheelLicensePattern.test(entry.fileName)) return zip.readEntry();
        zip.openReadStream(entry, (error, stream) => {
          if (error) return reject(error);
          const chunks = [];
          stream.on("error", reject);
          stream.on("data", (chunk) => chunks.push(chunk));
          stream.on("end", () => {
            found.push([
              entry.fileName,
              Buffer.concat(chunks).toString("utf8"),
            ]);
            zip.readEntry();
          });
        });
      });
      zip.readEntry();
    });
  });
}

test("the Python notice names every pinned package, Pyodide and the font, and reproduces their license texts", async () => {
  const downloads = await readJson(
    new URL("../../scripts/python-downloads.json", import.meta.url),
  );
  const lock = await readJson(
    new URL("../node_modules/pyodide/pyodide-lock.json", import.meta.url),
  );
  const notice = await readFile(
    new URL("../notices/python.txt", import.meta.url),
    "utf8",
  );
  assert.equal(notice.includes("\r"), false);

  // "<name> <version> (<license>)" over a rule, as office.txt does it; names
  // compare the way package names do.
  const normalize = (name) => name.toLowerCase().replace(/[-_. ]+/g, "-");
  const noticed = new Map();
  for (const [, name, version] of notice.matchAll(/^(.+) (\S+) \(.+\)\n-+\n/gm))
    noticed.set(normalize(name), [
      ...(noticed.get(normalize(name)) ?? []),
      version,
    ]);
  const wanted = [
    ...[...downloads.binary, ...downloads.pure, downloads.font].map((pin) => [
      pin.name,
      pin.version,
    ]),
    ["pyodide", downloads.pyodide],
    ["cpython", lock.info.python],
    [
      "emscripten",
      lock.info.platform
        .match(/^emscripten_(\d+)_(\d+)_(\d+)$/)
        .slice(1)
        .join("."),
    ],
  ];
  for (const [name, version] of wanted)
    assert.ok(
      noticed.get(normalize(name))?.includes(version),
      `python.txt must have a "${name} ${version} (license)" section`,
    );

  // The texts of what the runtime itself and the C libraries need.
  for (const marker of [
    "Mozilla Public License Version 2.0", // Pyodide
    "PYTHON SOFTWARE FOUNDATION LICENSE VERSION 2", // CPython
    "University of Illinois/NCSA Open Source License", // Emscripten
    "SIL OPEN FONT LICENSE Version 1.1", // Noto Sans SC
    "\nFreeType (FTL)\n",
    "\nlibjpeg-turbo (",
    "\nzlib (Zlib)\n",
    "\nlibtiff 4.4.0 (",
    "\nlibwebp (",
    "\nQhull (",
    "\nlibxml2 (MIT)\n",
    "\nlibxslt (MIT)\n",
  ])
    assert.ok(notice.includes(marker), marker);

  // Where the wheels and the font are staged, every license file they carry
  // is in the notice word for word.
  if (!pythonEngineOffered(platform, downloads)) return;
  const staged = new URL("../resources/engines/python/", import.meta.url);
  for (const pin of [...downloads.binary, ...downloads.pure]) {
    const licenses = await licensesInWheel(
      fileURLToPath(new URL(`wheels/${pin.file}`, staged)),
    );
    assert.ok(licenses.length > 0, `${pin.file} carries no license file`);
    for (const [file, text] of licenses)
      assert.ok(
        notice.includes(noticeText(text)),
        `${pin.name}: ${file} is missing from python.txt`,
      );
  }
  const ofl = await readFile(
    new URL(`fonts/${downloads.font.license.file}`, staged),
    "utf8",
  );
  assert.ok(notice.includes(noticeText(ofl)), "the font's OFL text");
});
