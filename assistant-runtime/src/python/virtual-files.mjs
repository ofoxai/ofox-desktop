import path from "node:path";

// Pyodide's module directory belongs to its virtual filesystem. The optional
// platform path API lets the same real-MEMFS regression exercise Windows on
// every host. OS file paths remain native; MEMFS paths always stay POSIX.
export function writeDeckModule(fs, site, source, platformPath = path) {
  fs.writeFile(platformPath.posix.join(site, "deck.py"), source);
}
