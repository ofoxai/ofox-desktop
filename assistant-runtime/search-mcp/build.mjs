// Bundle src/server.mjs (stdio transport only) into one self-contained file.
// The MCP SDK also ships Express/Hono HTTP transports; the server never
// imports them, and the build fails if any of them slips into the bundle.
// dist/bundled-packages.json lists the third-party packages compiled in, whose
// notices ship in ../notices/ofox-search-mcp.txt (test/notices.test.mjs).
import { build } from "esbuild";
import { readFile, stat, writeFile } from "node:fs/promises";

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
const packages = new Set(
  Object.keys(result.metafile.inputs)
    .map(
      (input) =>
        /(?:^|\/)node_modules\/((?:@[^/]+\/)?[^/]+)\//.exec(input)?.[1],
    )
    .filter(Boolean),
);
const bundled = [];
for (const name of [...packages].sort()) {
  const manifest = JSON.parse(
    await readFile(`node_modules/${name}/package.json`, "utf8"),
  );
  bundled.push({ name, version: manifest.version, license: manifest.license });
}
await writeFile(PACKAGES, `${JSON.stringify(bundled, null, 2)}\n`);
const { size } = await stat(OUTFILE);
console.log(`${OUTFILE} ${(size / 1024).toFixed(1)} KiB`);
