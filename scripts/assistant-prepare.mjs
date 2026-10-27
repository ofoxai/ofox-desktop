import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  access,
  chmod,
  copyFile,
  cp,
  mkdir,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  writeFile,
  realpath,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const runtime = path.join(root, "assistant-runtime");
const manifest = JSON.parse(
  await readFile(path.join(runtime, "node-downloads.json"), "utf8"),
);
const target = `${process.platform}-${process.arch}`;
const download = manifest[target];
if (!download)
  throw new Error(`Assistant runtime is not packaged for ${target}`);
const resources = path.join(runtime, "resources");
const nodeDirectory = path.join(resources, "node");
const node = path.join(
  nodeDirectory,
  process.platform === "win32" ? "node.exe" : "node",
);
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const cache = path.join(runtime, ".cache");
await mkdir(cache, { recursive: true });

async function cachedDownload() {
  const archive = path.join(cache, download.archive);
  let bytes;
  try {
    bytes = await readFile(archive);
  } catch {
    /* download a missing cache entry */
  }
  if (!bytes || digest(bytes) !== download.sha256) {
    const response = await fetch(
      `https://nodejs.org/dist/v${manifest.version}/${download.archive}`,
    );
    if (!response.ok)
      throw new Error(`Node download failed (${response.status})`);
    bytes = Buffer.from(await response.arrayBuffer());
    if (digest(bytes) !== download.sha256)
      throw new Error(
        "Downloaded Node SHA-256 differs from the pinned manifest",
      );
    const temporary = `${archive}.partial`;
    await writeFile(temporary, bytes, { mode: 0o600 });
    await rename(temporary, archive);
  }
  return archive;
}

async function prepareNode() {
  const stamp = path.join(nodeDirectory, "source.json");
  try {
    const existing = JSON.parse(await readFile(stamp, "utf8"));
    if (
      existing.version === manifest.version &&
      existing.sourceSha256 === download.sha256 &&
      existing.platform === target &&
      existing.executableSha256 === digest(await readFile(node))
    ) {
      const version = execFileSync(node, ["--version"], {
        encoding: "utf8",
      }).trim();
      if (version === `v${manifest.version}`) return;
    }
  } catch {
    /* replace only our generated runtime */
  }
  const archive = await cachedDownload();
  const extraction = path.join(cache, `extract-${target}`);
  await rm(extraction, { recursive: true, force: true });
  await mkdir(extraction, { recursive: true });
  execFileSync("tar", ["-xf", archive, "-C", extraction], { stdio: "inherit" });
  const prefix = `node-v${manifest.version}-${process.platform === "win32" ? "win" : process.platform}-${process.arch}`;
  await mkdir(nodeDirectory, { recursive: true });
  await copyFile(path.join(extraction, prefix, download.executable), node);
  await chmod(node, 0o755);
  await copyFile(
    path.join(extraction, prefix, "LICENSE"),
    path.join(nodeDirectory, "LICENSE"),
  );
  await writeFile(
    stamp,
    JSON.stringify(
      {
        version: manifest.version,
        platform: target,
        sourceSha256: download.sha256,
        executableSha256: digest(await readFile(node)),
      },
      null,
      2,
    ) + "\n",
  );
  await rm(extraction, { recursive: true, force: true });
  if (
    execFileSync(node, ["--version"], { encoding: "utf8" }).trim() !==
    `v${manifest.version}`
  )
    throw new Error("Bundled Node version differs");
}

async function npmCli() {
  const candidates = [];
  if (process.env.npm_execpath?.endsWith("npm-cli.js"))
    candidates.push(process.env.npm_execpath);
  for (const directory of (process.env.PATH ?? "").split(path.delimiter)) {
    candidates.push(
      path.join(directory, "node_modules", "npm", "bin", "npm-cli.js"),
    );
    candidates.push(
      path.resolve(
        directory,
        "..",
        "lib",
        "node_modules",
        "npm",
        "bin",
        "npm-cli.js",
      ),
    );
    try {
      candidates.push(await realpath(path.join(directory, "npm")));
    } catch {
      /* next PATH entry */
    }
  }
  for (const candidate of candidates) {
    if (!candidate.endsWith("npm-cli.js")) continue;
    try {
      await access(candidate);
      return candidate;
    } catch {
      /* next candidate */
    }
  }
  throw new Error(
    "Build environment must provide npm; end users do not need it",
  );
}

async function install(directory) {
  execFileSync(
    node,
    [
      await npmCli(),
      "ci",
      "--ignore-scripts",
      "--omit=dev",
      "--no-audit",
      "--no-fund",
    ],
    {
      cwd: directory,
      stdio: "inherit",
      env: {
        ...process.env,
        PATH: `${nodeDirectory}${path.delimiter}${process.env.PATH ?? ""}`,
      },
    },
  );
}

async function signMacFiles(directory) {
  const identity = process.env.APPLE_SIGNING_IDENTITY;
  if (process.platform !== "darwin" || !identity) return;
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const filename = path.join(directory, entry.name);
    if (entry.isDirectory()) await signMacFiles(filename);
    else if (entry.isFile()) {
      const bytes = await readFile(filename);
      const magic = bytes.subarray(0, 4).toString("hex");
      if (["cffaedfe", "feedfacf", "cafebabe", "bebafeca"].includes(magic)) {
        // Hardened ad-hoc processes have no Team ID and cannot dlopen even
        // another ad-hoc addon. Preview bundles are not notarized; production
        // signs every Mach-O with the same Developer ID and keeps validation.
        const args = ["--force", "--sign", identity];
        if (identity !== "-") args.push("--options", "runtime");
        if (filename === node)
          args.push(
            "--entitlements",
            path.join(runtime, "node-entitlements.plist"),
          );
        execFileSync("codesign", [...args, filename], { stdio: "inherit" });
      }
    }
  }
}

await prepareNode();
if (process.argv.includes("--node-only")) {
  process.stdout.write(`${node}\n`);
} else if (process.argv.includes("--install-only")) {
  await install(runtime);
} else {
  const production = path.join(resources, "runtime");
  await rm(production, { recursive: true, force: true });
  await mkdir(production, { recursive: true });
  for (const filename of ["package.json", "package-lock.json"])
    await copyFile(
      path.join(runtime, filename),
      path.join(production, filename),
    );
  await cp(path.join(runtime, "src"), path.join(production, "src"), {
    recursive: true,
  });
  await install(production);
  await signMacFiles(resources);
  const stamp = path.join(nodeDirectory, "source.json");
  const source = JSON.parse(await readFile(stamp, "utf8"));
  await writeFile(
    stamp,
    JSON.stringify(
      { ...source, executableSha256: digest(await readFile(node)) },
      null,
      2,
    ) + "\n",
  );
  await writeFile(
    path.join(resources, "manifest.json"),
    JSON.stringify(
      {
        protocolVersion: 1,
        sdkVersion: "1.4.0",
        nodeVersion: manifest.version,
        platform: target,
        node: `node/${path.basename(node)}`,
        entry: "runtime/src/main.mjs",
        minimumMacOS: "13.5",
      },
      null,
      2,
    ) + "\n",
  );
  process.stdout.write(
    `Prepared assistant runtime (${target}, Node ${manifest.version}, Pi SDK 1.4.0)\n`,
  );
}
