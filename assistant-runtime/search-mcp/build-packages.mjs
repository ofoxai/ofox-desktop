// The third-party packages esbuild compiled into the bundle, from its
// metafile inputs. A file belongs to its innermost node_modules/<name>, so a
// nested copy counts as its own package with its own version.
const PACKAGE_DIRECTORY = /^((?:.*\/)?node_modules\/(?:@[^/]+\/)?[^/]+)\//;

/** `readManifest(path)` returns the parsed package.json at `path`. */
export async function bundledPackages(inputs, readManifest) {
  const directories = new Set(
    inputs.map((input) => PACKAGE_DIRECTORY.exec(input)?.[1]).filter(Boolean),
  );
  const packages = new Map();
  for (const directory of directories) {
    const name = /node_modules\/((?:@[^/]+\/)?[^/]+)$/.exec(directory)[1];
    const manifest = await readManifest(`${directory}/package.json`);
    const license =
      typeof manifest.license === "string"
        ? manifest.license
        : (manifest.license?.type ?? "UNKNOWN");
    packages.set(`${name}@${manifest.version}`, {
      name,
      version: manifest.version,
      license,
    });
  }
  return [...packages.values()].sort(
    (a, b) =>
      a.name.localeCompare(b.name) ||
      a.version.localeCompare(b.version, undefined, { numeric: true }),
  );
}
