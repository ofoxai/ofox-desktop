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

export async function pruneProduction(directory) {
  for (const unused of UNUSED_PACKAGE_PATHS)
    await rm(path.join(directory, "node_modules", ...unused.split("/")), {
      recursive: true,
      force: true,
    });
  await removeSourceMaps(directory);
}

async function removeSourceMaps(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) await removeSourceMaps(filename);
    else if (entry.isFile() && entry.name.endsWith(".map")) await rm(filename);
  }
}
