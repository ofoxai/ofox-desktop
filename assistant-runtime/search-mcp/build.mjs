// Bundle src/server.mjs (stdio transport only) into one self-contained file.
// The MCP SDK also ships Express/Hono HTTP transports; the server never
// imports them, and the build fails if any of them slips into the bundle.
// dist/bundled-packages.json lists the third-party packages compiled in, whose
// notices ship in ../notices/ofox-search-mcp.txt (test/notices.test.mjs).
import { build } from "esbuild";
import { mkdir, readFile, rename, writeFile } from "node:fs/promises";
import path from "node:path";
import { bundledPackages } from "./build-packages.mjs";

const OUTFILE = "dist/ofox-search-mcp.mjs";
const PACKAGES = "dist/bundled-packages.json";
const FORBIDDEN =
  /(?:^|\/)node_modules\/(?:express|hono|@hono|cors|body-parser|raw-body|express-rate-limit)\//;

const result = await build({
  entryPoints: ["src/server.mjs"],
  outfile: OUTFILE,
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node22",
  minify: true,
  legalComments: "eof",
  metafile: true,
  write: false,
  logLevel: "warning",
  // Bundled CommonJS dependencies (ajv) may require Node built-ins.
  banner: {
    js: 'import { createRequire as __ofoxCreateRequire } from "node:module"; const require = __ofoxCreateRequire(import.meta.url);',
  },
});

const forbidden = Object.keys(result.metafile.inputs).filter((input) =>
  FORBIDDEN.test(input),
);
if (forbidden.length > 0) {
  console.error(`HTTP server code in the bundle:\n${forbidden.join("\n")}`);
  process.exit(1);
}
const bundled = await bundledPackages(
  Object.keys(result.metafile.inputs),
  async (file) => JSON.parse(await readFile(file, "utf8")),
);

// Test runs may build at the same time: each file appears whole, by rename.
async function writeWhole(file, contents) {
  await mkdir(path.dirname(file), { recursive: true });
  const temporary = `${file}.${process.pid}.tmp`;
  await writeFile(temporary, contents);
  await rename(temporary, file);
}
const [bundle] = result.outputFiles;
await writeWhole(bundle.path, bundle.contents);
await writeWhole(PACKAGES, `${JSON.stringify(bundled, null, 2)}\n`);
console.log(`${OUTFILE} ${(bundle.contents.byteLength / 1024).toFixed(1)} KiB`);
