import path from "node:path";

// Entitlements of each executable the assistant ships, by file name. The value
// is a plist in assistant-runtime/, or null for an executable that is signed
// with the hardened runtime and no entitlements.
//
// Node and officecli get `allow-jit` only: Node's V8 and officecli's .NET
// runtime compile code at run time, which the hardened runtime forbids
// otherwise. officecli's upstream Developer ID build carries exactly that key,
// and its single-file bundle loads no extracted native library. Add
// `allow-unsigned-executable-memory`, then `disable-library-validation`, then
// `allow-dyld-environment-variables` only for a failure observed with the
// signed binary (docs/assistant-engines.md). typst is Rust and needs none.
export const ENTITLEMENTS = {
  node: "node-entitlements.plist",
  officecli: "officecli-entitlements.plist",
  typst: null,
};

/**
 * The plist `filename` is signed with, or null for none. Every executable
 * under `enginesDirectory` must be listed: one that is not (a new engine, or
 * an upstream rename) would be signed without the entitlements it needs and
 * only crash when first run, so it fails the build instead.
 */
export function entitlementsFor(filename, enginesDirectory) {
  const name = path.basename(filename);
  if (Object.hasOwn(ENTITLEMENTS, name)) return ENTITLEMENTS[name];
  const relative = path.relative(enginesDirectory, filename);
  if (relative.split(path.sep)[0] !== ".." && !path.isAbsolute(relative))
    throw new Error(
      `${filename} is an executable under the engines directory with no entry in ENTITLEMENTS (scripts/assistant-entitlements.mjs). Add "${name}" with its entitlements plist, or null for none.`,
    );
  return null;
}
