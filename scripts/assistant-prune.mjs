import { readdir, rm } from "node:fs/promises";
import path from "node:path";

// Package paths the sidecar never loads, kept out of the application bundle.
// Pi ships demo extensions (one bundles a GPL-licensed doom.wasm). PDF.js
// ships a modern build that fails on Node 22, viewers and standalone image
// decoders that the legacy build does not import, and standard fonts that are
// read only when standardFontDataUrl is set, which the runtime never does.
// The legacy build and the cmaps, iccs and wasm it reads stay.
// The application loads Pi's unbundled SDK, not its standalone CLI/RPC entry.
// The standalone bundle embeds its own dependencies, so npm overrides cannot
// patch it. Keep it and its launchers out of the production application.
export const UNUSED_PACKAGE_PATHS = [
  "@earendil-works/pi-coding-agent/examples",
  "@earendil-works/pi-coding-agent/dist/bundle",
  ".bin/pi",
  ".bin/pi.cmd",
  ".bin/pi.ps1",
  "pdfjs-dist/build",
  "pdfjs-dist/web",
  "pdfjs-dist/image_decoders",
  "pdfjs-dist/standard_fonts",
  "pdfjs-dist/legacy/web",
  "pdfjs-dist/legacy/image_decoders",
];

// The pyodide npm package (about 13 MB) only runs the Python engine. Where
// that engine is not offered (python-assets.mjs) it can never run.
export const PYTHON_PACKAGE_PATHS = ["pyodide"];

export async function pruneProduction(
  directory,
  { pythonEngine = true, platform = process.platform } = {},
) {
  const unusedPaths = pythonEngine
    ? UNUSED_PACKAGE_PATHS
    : [...UNUSED_PACKAGE_PATHS, ...PYTHON_PACKAGE_PATHS];
  for (const unused of unusedPaths)
    await rm(path.join(directory, "node_modules", ...unused.split("/")), {
      recursive: true,
      force: true,
    });
  // Windows reopens every packaged file to verify the document runtime.
  // Node executes the JS/TS modules; the .d.* siblings are editor metadata.
  const metadata = platform === "win32" ? /(?:\.map|\.d\.[cm]?ts)$/ : /\.map$/;
  // These two pinned SDKs run the .js/.mjs at the package root. Their src
  // mirrors also contain the only copies of several third-party licenses.
  const sourceMirrors = new Map(
    platform === "win32"
      ? ["openai", "@anthropic-ai/sdk"].map((name) => [
          path.join(directory, "node_modules", name, "src"),
          /(?:\.map|\.[cm]?ts)$/,
        ])
      : [],
  );
  // These pinned packages resolve every public Node import and require to
  // dist-cjs. dist-es is a second build for bundlers. Keep embedded notices.
  if (platform === "win32")
    for (const name of [
      "@smithy/core",
      "@aws-sdk/core",
      "@aws-sdk/nested-clients",
    ])
      sourceMirrors.set(
        path.join(directory, "node_modules", name, "dist-es"),
        /(?:\.map|\.js|\.d\.[cm]?ts)$/,
      );
  await removeBuildMetadata(directory, metadata, sourceMirrors);
}

async function removeBuildMetadata(directory, metadata, sourceMirrors) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory())
      await removeBuildMetadata(
        filename,
        sourceMirrors.get(filename) ?? metadata,
        sourceMirrors,
      );
    else if (entry.isFile() && metadata.test(entry.name)) await rm(filename);
  }
}
