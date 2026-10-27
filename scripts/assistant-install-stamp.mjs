import { createHash } from "node:crypto";
import { access, readFile, rm, writeFile } from "node:fs/promises";
import path from "node:path";

// The production dependency tree is installed with `npm ci`, which deletes
// and downloads it again. Tauri's dev and build hooks prepare the runtime on
// every run, so the install is skipped while what it was made from (the
// lockfile, package.json, the scripts that install and prune it, Node version
// and platform) is unchanged. The search server's bundle is skipped the same
// way, with its build script and sources as the `scripts`.

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

/**
 * What an install of `runtime`'s production dependencies is made from;
 * `scripts` are the files that install, prune or build it.
 */
export async function installStamp({
  runtime,
  scripts,
  nodeVersion,
  platform,
}) {
  const scriptDigests = [];
  for (const script of scripts)
    scriptDigests.push(digest(await readFile(script)));
  return {
    lockSha256: digest(await readFile(path.join(runtime, "package-lock.json"))),
    packageSha256: digest(await readFile(path.join(runtime, "package.json"))),
    scriptsSha256: digest(scriptDigests.join("\n")),
    nodeVersion,
    platform,
  };
}

/**
 * Whether `production` holds an install made from `wanted`; `required` is
 * the path inside it that the install produced.
 */
export async function installIsCurrent(
  production,
  stampFile,
  wanted,
  required = "node_modules",
) {
  try {
    await access(path.join(production, required));
    const installed = JSON.parse(await readFile(stampFile, "utf8"));
    return Object.entries(wanted).every(
      ([field, value]) => installed[field] === value,
    );
  } catch {
    return false;
  }
}

/** Record a finished install; an interrupted one has no stamp. */
export async function writeInstallStamp(stampFile, stamp) {
  await writeFile(stampFile, JSON.stringify(stamp, null, 2) + "\n");
}

/** Forget the install before replacing it. */
export async function clearInstallStamp(stampFile) {
  await rm(stampFile, { force: true });
}
