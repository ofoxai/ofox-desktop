import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import {
  access,
  chmod,
  copyFile,
  cp,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  stat,
  writeFile,
  realpath,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { entitlementsFor } from "./assistant-entitlements.mjs";
import {
  clearInstallStamp,
  installIsCurrent,
  installStamp,
  writeInstallStamp,
} from "./assistant-install-stamp.mjs";
import { pruneProduction } from "./assistant-prune.mjs";
import { pyodideWheelUrl } from "./python-assets.mjs";

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

async function cachedDownload({ url, archive: name, sha256 }) {
  const archive = path.join(cache, name);
  let bytes;
  try {
    bytes = await readFile(archive);
  } catch {
    /* download a missing cache entry */
  }
  if (!bytes || digest(bytes) !== sha256) {
    const response = await fetch(url);
    if (!response.ok)
      throw new Error(`Download of ${url} failed (${response.status})`);
    bytes = Buffer.from(await response.arrayBuffer());
    if (digest(bytes) !== sha256)
      throw new Error(
        `Downloaded ${name} SHA-256 differs from the pinned manifest`,
      );
    const temporary = `${archive}.partial`;
    await writeFile(temporary, bytes, { mode: 0o600 });
    await rename(temporary, archive);
  }
  return archive;
}

function extractArchive(archive, directory) {
  let tar = "tar";
  if (process.platform === "win32") {
    const systemRoot = process.env.SystemRoot ?? process.env.SYSTEMROOT;
    if (!systemRoot || !path.win32.isAbsolute(systemRoot))
      throw new Error(
        "An absolute Windows SystemRoot is required to extract bundled archives",
      );
    // Git Bash prepends GNU tar, which treats drive-letter paths as remote
    // hosts and cannot unpack ZIPs. Windows' bsdtar supports the Node and
    // engine archives.
    tar = path.win32.join(systemRoot, "System32", "tar.exe");
  }
  execFileSync(tar, ["-xf", archive, "-C", directory], { stdio: "inherit" });
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
  const archive = await cachedDownload({
    url: `https://nodejs.org/dist/v${manifest.version}/${download.archive}`,
    archive: download.archive,
    sha256: download.sha256,
  });
  const extraction = path.join(cache, `extract-${target}`);
  await rm(extraction, { recursive: true, force: true });
  await mkdir(extraction, { recursive: true });
  extractArchive(archive, extraction);
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

async function install(directory, { omitDev = true } = {}) {
  execFileSync(
    node,
    [
      await npmCli(),
      "ci",
      "--ignore-scripts",
      ...(omitDev ? ["--omit=dev"] : []),
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

// The bundled Node must run on every macOS the application accepts, so the
// manifest records the application's own minimum instead of a second copy.
async function applicationMinimumMacOS() {
  const config = JSON.parse(
    await readFile(path.join(root, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const minimum = config.bundle?.macOS?.minimumSystemVersion;
  if (typeof minimum !== "string" || !minimum)
    throw new Error(
      "src-tauri/tauri.conf.json must set bundle.macOS.minimumSystemVersion",
    );
  return minimum;
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
        // Notarization needs a secure timestamp on every signed Mach-O.
        if (identity !== "-") args.push("--options", "runtime", "--timestamp");
        // Throws for an engine executable with no entry (assistant-entitlements.mjs).
        const entitlements = entitlementsFor(
          filename,
          path.join(resources, "engines"),
        );
        if (entitlements)
          args.push("--entitlements", path.join(runtime, entitlements));
        execFileSync("codesign", [...args, filename], { stdio: "inherit" });
      }
    }
  }
}

// The web search MCP server ships as the single file its build bundles
// (search-mcp/README.md). It is built with its devDependencies (esbuild) on
// the bundled Node, and skipped while its lockfile, package.json, build
// script, sources and this script are unchanged.
async function prepareSearchServer() {
  const source = path.join(runtime, "search-mcp");
  const shipped = path.join(resources, "search-mcp");
  const stampFile = path.join(resources, "search-mcp-build.json");
  const sources = (
    await readdir(path.join(source, "src"), {
      recursive: true,
      withFileTypes: true,
    })
  )
    .filter((entry) => entry.isFile())
    .map((entry) => path.join(entry.parentPath, entry.name))
    .sort();
  const wanted = await installStamp({
    runtime: source,
    scripts: [
      fileURLToPath(import.meta.url),
      path.join(source, "build.mjs"),
      path.join(source, "build-packages.mjs"),
      ...sources,
    ],
    nodeVersion: manifest.version,
    platform: target,
  });
  if (
    await installIsCurrent(shipped, stampFile, wanted, "ofox-search-mcp.mjs")
  ) {
    process.stdout.write(
      "Assistant search server is current; skipping build\n",
    );
    return;
  }
  await clearInstallStamp(stampFile);
  await install(source, { omitDev: false });
  execFileSync(node, ["build.mjs"], { cwd: source, stdio: "inherit" });
  await rm(shipped, { recursive: true, force: true });
  await mkdir(shipped, { recursive: true });
  await copyFile(
    path.join(source, "dist", "ofox-search-mcp.mjs"),
    path.join(shipped, "ofox-search-mcp.mjs"),
  );
  await writeInstallStamp(stampFile, wanted);
}

// The document engines are builds pinned by engine-downloads.json. Both
// officecli and typst keep state under HOME and officecli may start a resident
// process or update itself, so they only ever run here and in the smoke test
// with a throwaway HOME and those behaviors turned off.
async function engineVersion(executable) {
  const home = await mkdtemp(path.join(tmpdir(), "ofox-engine-"));
  const env = {
    OFFICECLI_NO_AUTO_RESIDENT: "1",
    OFFICECLI_SKIP_UPDATE: "1",
    HOME: home,
    USERPROFILE: home,
    TMPDIR: home,
    TEMP: home,
    TMP: home,
  };
  for (const key of ["PATH", "SystemRoot"])
    if (process.env[key]) env[key] = process.env[key];
  try {
    // A hung engine fails the prepare instead of stalling it.
    return execFileSync(executable, ["--version"], {
      encoding: "utf8",
      env,
      timeout: 60_000,
      stdio: ["ignore", "pipe", "pipe"],
    });
  } finally {
    await rm(home, { recursive: true, force: true });
  }
}

async function engineIsCurrent(executable, version) {
  try {
    await access(path.join(path.dirname(executable), "LICENSE"));
    return (await engineVersion(executable)).includes(version);
  } catch {
    return false;
  }
}

async function installEngine(name, engine, pinned, directory, executable) {
  const downloaded = await cachedDownload({
    url: pinned.url,
    archive: `${name}-${engine.version}-${pinned.archive ?? path.basename(pinned.url)}`,
    sha256: pinned.sha256,
  });
  const extraction = path.join(cache, `extract-${name}-${target}`);
  await rm(extraction, { recursive: true, force: true });
  let source = downloaded;
  const licenses = [];
  if (pinned.archive) {
    // The release archive carries its own LICENSE and NOTICE next to the binary.
    await mkdir(extraction, { recursive: true });
    extractArchive(downloaded, extraction);
    source = path.join(extraction, pinned.executable);
    for (const file of ["LICENSE", "NOTICE"])
      licenses.push([file, path.join(path.dirname(source), file)]);
  } else {
    // A bare binary has none; they come from the repository at the release tag.
    for (const [file, license] of Object.entries(engine.licenses))
      licenses.push([
        file,
        await cachedDownload({
          url: license.url,
          archive: `${name}-${engine.version}-${file}`,
          sha256: license.sha256,
        }),
      ]);
  }
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  await copyFile(source, executable);
  await chmod(executable, 0o755);
  for (const [file, from] of licenses) {
    const license = path.join(directory, file);
    await copyFile(from, license);
    await chmod(license, 0o644);
  }
  await rm(extraction, { recursive: true, force: true });
}

// The Python engine is not a binary: Pyodide's wheels, pure-Python wheels and
// a CJK font pinned by python-downloads.json, staged under
// resources/engines/python/ where the runner reads them. Pyodide itself comes
// from the runtime's npm dependency. The engine is offered on the platforms
// the pins list (macOS), so no other installer carries these ~40 MB.
async function preparePython(installed) {
  const pinsFile = path.join(root, "scripts", "python-downloads.json");
  const pinsBytes = await readFile(pinsFile);
  const downloads = JSON.parse(pinsBytes);
  if (!downloads.platforms.includes(target)) {
    process.stdout.write(`Python engine is not offered on ${target}\n`);
    return null;
  }
  const directory = path.join(resources, "engines", "python");
  const assetsFile = path.join(directory, "assets.json");
  const stamp = { version: downloads.pyodide, sha256: digest(pinsBytes) };
  const { font } = downloads;
  const files = [
    ...downloads.binary.map((pin) => ({
      to: `wheels/${pin.file}`,
      pin,
      url: pyodideWheelUrl(pin),
    })),
    ...downloads.pure.map((pin) => ({
      to: `wheels/${pin.file}`,
      pin,
      url: pin.url,
    })),
    { to: `fonts/${font.file}`, pin: font, url: font.url },
    {
      to: `fonts/${font.license.file}`,
      pin: font.license,
      url: font.license.url,
    },
  ];
  // Current means the stamp matches and every staged file still has its pinned
  // digest; assets.json is written last, so an interrupted copy has none.
  const isCurrent = async () => {
    if (JSON.stringify(installed.python) !== JSON.stringify(stamp))
      return false;
    try {
      await access(assetsFile);
      for (const { to, pin } of files)
        if (digest(await readFile(path.join(directory, to))) !== pin.sha256)
          return false;
      return true;
    } catch {
      return false;
    }
  };
  if (!(await isCurrent())) {
    await rm(directory, { recursive: true, force: true });
    for (const { to, pin, url } of files) {
      const cached = await cachedDownload({
        url,
        archive: `python-${pin.file}`,
        sha256: pin.sha256,
      });
      await mkdir(path.dirname(path.join(directory, to)), { recursive: true });
      await copyFile(cached, path.join(directory, to));
      // The cache keeps its files private; the bundle's must be readable.
      await chmod(path.join(directory, to), 0o644);
    }
    await writeFile(
      assetsFile,
      JSON.stringify(
        {
          pyodide: downloads.pyodide,
          binary: downloads.binary.map((pin) => pin.name),
          pure: downloads.pure.map((pin) => pin.file),
          font: `fonts/${font.file}`,
        },
        null,
        2,
      ) + "\n",
    );
    process.stdout.write(
      `Staged the Python engine (Pyodide ${stamp.version})\n`,
    );
  }
  return {
    stamp,
    entry: {
      path: path.posix.join("engines", "python", "assets.json"),
      runner: "runtime/src/python/runner.mjs",
      version: downloads.pyodide,
    },
  };
}

// resources/engines/source.json records what is installed. While it matches
// the pins and the engine reports its version, the next prepare skips the
// engine. Returns the manifest entry of each engine with a build for this
// platform.
async function prepareEngines() {
  const downloads = JSON.parse(
    await readFile(path.join(runtime, "engine-downloads.json"), "utf8"),
  );
  const stampFile = path.join(resources, "engines", "source.json");
  let installed = {};
  try {
    installed = JSON.parse(await readFile(stampFile, "utf8"));
  } catch {
    /* replace only our generated engines */
  }
  const stamps = {};
  const entries = {};
  for (const name of ["officecli", "typst"]) {
    const engine = downloads[name];
    const pinned = engine[target];
    if (!pinned) {
      process.stdout.write(
        `${name} ${engine.version} has no ${target} build\n`,
      );
      continue;
    }
    const directory = path.join(resources, "engines", name);
    const executable = path.join(directory, path.basename(pinned.executable));
    const stamp = {
      version: engine.version,
      platform: target,
      sha256: pinned.sha256,
    };
    if (
      JSON.stringify(installed[name]) !== JSON.stringify(stamp) ||
      !(await engineIsCurrent(executable, engine.version))
    ) {
      await installEngine(name, engine, pinned, directory, executable);
      if (!(await engineIsCurrent(executable, engine.version)))
        throw new Error(`Bundled ${name} does not report ${engine.version}`);
    }
    stamps[name] = stamp;
    await writeFile(stampFile, JSON.stringify(stamps, null, 2) + "\n");
    entries[name] = {
      path: path.posix.join("engines", name, path.basename(pinned.executable)),
      version: engine.version,
    };
  }
  const python = await preparePython(installed);
  if (python) {
    stamps.python = python.stamp;
    await writeFile(stampFile, JSON.stringify(stamps, null, 2) + "\n");
    entries.python = python.entry;
  }
  return entries;
}

await prepareNode();
if (process.argv.includes("--node-only")) {
  process.stdout.write(`${node}\n`);
} else if (process.argv.includes("--install-only")) {
  // Test dependencies: the runtime's, and the search server's with its
  // devDependencies (esbuild), which assistant:test builds and tests.
  await install(runtime);
  await install(path.join(runtime, "search-mcp"), { omitDev: false });
} else {
  const production = path.join(resources, "runtime");
  const stampFile = path.join(resources, "runtime-install.json");
  const wanted = await installStamp({
    runtime,
    // A change to how the tree is installed or pruned redoes both.
    scripts: [
      fileURLToPath(import.meta.url),
      path.join(root, "scripts", "assistant-prune.mjs"),
    ],
    nodeVersion: manifest.version,
    platform: target,
  });
  if (await installIsCurrent(production, stampFile, wanted)) {
    process.stdout.write(
      "Assistant runtime dependencies are current; skipping npm ci\n",
    );
  } else {
    await clearInstallStamp(stampFile);
    await rm(production, { recursive: true, force: true });
    await mkdir(production, { recursive: true });
    for (const filename of ["package.json", "package-lock.json"])
      await copyFile(
        path.join(runtime, filename),
        path.join(production, filename),
      );
    await install(production);
    await pruneProduction(production);
    await writeInstallStamp(stampFile, wanted);
  }
  // The runtime's own source is copied every time: it changes without the lockfile.
  await rm(path.join(production, "src"), { recursive: true, force: true });
  await cp(path.join(runtime, "src"), path.join(production, "src"), {
    recursive: true,
  });
  await rm(path.join(production, "skills"), { recursive: true, force: true });
  await cp(path.join(runtime, "skills"), path.join(production, "skills"), {
    recursive: true,
  });
  // Notices for third-party code the npm packages ship without them, such as
  // the libraries compiled into the native canvas addon. Copied every time,
  // like the source, because they change without the lockfile.
  const notices = path.join(resources, "notices");
  await rm(notices, { recursive: true, force: true });
  await cp(path.join(runtime, "notices"), notices, { recursive: true });
  await prepareSearchServer();
  // Before signing, so the engines' Mach-O files are signed with the rest.
  const engines = await prepareEngines();
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
  const { version: sdkVersion } = JSON.parse(
    await readFile(
      path.join(
        production,
        "node_modules/@earendil-works/pi-coding-agent/package.json",
      ),
      "utf8",
    ),
  );
  await writeFile(
    path.join(resources, "manifest.json"),
    JSON.stringify(
      {
        protocolVersion: 1,
        sdkVersion,
        nodeVersion: manifest.version,
        platform: target,
        node: `node/${path.basename(node)}`,
        entry: "runtime/src/main.mjs",
        searchServer: "search-mcp/ofox-search-mcp.mjs",
        engines,
        minimumMacOS: await applicationMinimumMacOS(),
      },
      null,
      2,
    ) + "\n",
  );
  process.stdout.write(
    `Prepared assistant runtime (${target}, Node ${manifest.version}, Pi SDK ${sdkVersion})\n`,
  );
}
