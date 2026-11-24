import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const outputName = "windows-child-probe.node";
// A client checkout may use different line endings from the Windows builder.
const readBuildText = async (file) =>
  (await readFile(file, "utf8")).replaceAll("\r\n", "\n");

async function readIfPresent(file) {
  try {
    return await readFile(file);
  } catch (error) {
    if (error.code === "ENOENT") return null;
    throw error;
  }
}

async function readSdkArchive(cache, version, name, sha256) {
  const cachedPath = path.join(cache, name);
  const cached = await readIfPresent(cachedPath);
  if (cached && digest(cached) === sha256) return cached;

  const url = `https://nodejs.org/dist/v${version}/${name}`;
  const response = await fetch(url);
  if (!response.ok)
    throw new Error(`Node SDK download failed (${response.status}) for ${url}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  if (digest(bytes) !== sha256)
    throw new Error(`Node SDK checksum mismatch for ${name}`);

  const temporary = `${cachedPath}.${randomUUID()}.partial`;
  try {
    await writeFile(temporary, bytes, { flag: "wx" });
    await rename(temporary, cachedPath);
  } finally {
    await rm(temporary, { force: true });
  }
  return bytes;
}

function msvcEnvironment() {
  const installer = process.env["ProgramFiles(x86)"];
  if (!installer || !path.isAbsolute(installer))
    throw new Error("Visual Studio Build Tools require ProgramFiles(x86)");
  const vswhere = path.join(
    installer,
    "Microsoft Visual Studio",
    "Installer",
    "vswhere.exe",
  );
  const installation = execFileSync(
    vswhere,
    [
      "-latest",
      "-products",
      "*",
      "-requires",
      "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
      "-property",
      "installationPath",
    ],
    { encoding: "utf8", windowsHide: true },
  ).trim();
  if (!path.isAbsolute(installation))
    throw new Error(
      "Windows child probe requires Visual Studio x64 C++ Build Tools",
    );

  const buildOverrides = new Set([
    "CL",
    "_CL_",
    "LINK",
    "_LINK_",
    "INCLUDE",
    "LIB",
    "LIBPATH",
    "VSINSTALLDIR",
    "VCINSTALLDIR",
    "VCTOOLSINSTALLDIR",
    "VSCMD_VER",
  ]);
  return {
    ...Object.fromEntries(
      Object.entries(process.env).filter(
        ([key]) =>
          !buildOverrides.has(key.toUpperCase()) &&
          !key.toUpperCase().startsWith("__VSCMD"),
      ),
    ),
    OFOX_CHILD_PROBE_VSDEVCMD: path.join(
      installation,
      "Common7",
      "Tools",
      "VsDevCmd.bat",
    ),
  };
}

export async function createWindowsProbeBuildStage({ cache, resources }) {
  const [cacheRoot, resourceRoot] = await Promise.all([
    realpath(cache),
    realpath(resources),
  ]);
  const relative = path.relative(resourceRoot, cacheRoot);
  if (
    relative === "" ||
    (!path.isAbsolute(relative) &&
      relative !== ".." &&
      !relative.startsWith(`..${path.sep}`))
  )
    throw new Error(
      "Windows child probe build cache must be outside bundled resources",
    );
  return mkdtemp(path.join(cacheRoot, "windows-child-probe-build-"));
}

export async function prepareWindowsChildProbe({
  root,
  resources,
  cache,
  node,
}) {
  if (process.platform !== "win32" || process.arch !== "x64") return;

  const sourceDir = path.join(
    root,
    "assistant-runtime",
    "native",
    "windows-child-probe",
  );
  const [source, cleanupTest, sdkBytes, buildRecipe] = await Promise.all([
    readBuildText(path.join(sourceDir, "probe.c")),
    readBuildText(path.join(sourceDir, "cleanup.test.c")),
    readBuildText(path.join(sourceDir, "sdk.json")),
    readBuildText(fileURLToPath(import.meta.url)),
  ]);
  // sdk.json pins the headers archive from Node's signed
  // https://nodejs.org/dist/v22.23.4/SHASUMS256.txt.
  const sdk = JSON.parse(sdkBytes);
  if (
    !/^\d+\.\d+\.\d+$/.test(sdk.version) ||
    !/^[a-f0-9]{64}$/.test(sdk.headersSha256)
  )
    throw new Error("Invalid Windows child probe SDK pins");
  const version = execFileSync(node, ["--version"], {
    encoding: "utf8",
    windowsHide: true,
  }).trim();
  if (version !== `v${sdk.version}`)
    throw new Error(
      "Windows child probe SDK version differs from bundled Node",
    );

  const identity = {
    schemaVersion: 1,
    target: "win32-x64",
    nodeVersion: sdk.version,
    sourceSha256: digest(source),
    cleanupTestSha256: digest(cleanupTest),
    sdkSha256: digest(sdkBytes),
    buildRecipeSha256: digest(buildRecipe),
  };
  const outputDir = path.join(resources, "probes");
  const output = path.join(outputDir, outputName);
  const stamp = path.join(outputDir, "windows-child-probe.source.json");
  const [installedBinary, stampBytes] = await Promise.all([
    readIfPresent(output),
    readIfPresent(stamp),
  ]);
  if (installedBinary && stampBytes) {
    let installed;
    try {
      installed = JSON.parse(stampBytes);
    } catch {
      // Replace malformed provenance along with the generated artifact.
    }
    if (
      installed?.binarySha256 === digest(installedBinary) &&
      Object.entries(identity).every(([key, value]) => installed[key] === value)
    )
      return;
  }

  const systemRoot = process.env.SystemRoot;
  if (!systemRoot || !path.isAbsolute(systemRoot))
    throw new Error("Windows child probe requires an absolute SystemRoot");
  const environment = msvcEnvironment();
  await mkdir(cache, { recursive: true });
  const headersName = `node-v${sdk.version}-headers.tar.gz`;
  const headers = await readSdkArchive(
    cache,
    sdk.version,
    headersName,
    sdk.headersSha256,
  );

  await mkdir(outputDir, { recursive: true });
  const stage = await createWindowsProbeBuildStage({ cache, resources });
  try {
    const compiler = '"%VCToolsInstallDir%bin\\Hostx64\\x64\\cl.exe"';
    const flags = `/nologo /W4 /WX /O2 /MT /std:c11 /Brepro /I node-v${sdk.version}\\include\\node`;
    const command = `@echo off
call "%OFOX_CHILD_PROBE_VSDEVCMD%" -no_logo -arch=x64 -host_arch=x64
if errorlevel 1 exit /b 1
if not defined VCToolsInstallDir exit /b 1
${compiler} ${flags} /Gy /Fe:cleanup-test.exe cleanup.test.c /link /INCREMENTAL:NO /OPT:REF
if errorlevel 1 exit /b %errorlevel%
.\\cleanup-test.exe
if errorlevel 1 exit /b %errorlevel%
${compiler} ${flags} /LD /Fe:${outputName} probe.c /link /INCREMENTAL:NO
exit /b %errorlevel%
`;
    await Promise.all([
      writeFile(path.join(stage, headersName), headers),
      writeFile(path.join(stage, "probe.c"), source),
      writeFile(path.join(stage, "cleanup.test.c"), cleanupTest),
      writeFile(
        path.join(stage, "build.cmd"),
        command.replaceAll("\n", "\r\n"),
      ),
    ]);
    execFileSync(
      path.join(systemRoot, "System32", "tar.exe"),
      ["-xf", headersName],
      {
        cwd: stage,
        windowsHide: true,
        stdio: "inherit",
      },
    );
    execFileSync(
      path.join(systemRoot, "System32", "cmd.exe"),
      ["/d", "/c", "build.cmd"],
      {
        cwd: stage,
        env: environment,
        windowsHide: true,
        stdio: "inherit",
      },
    );
    const built = path.join(stage, outputName);
    const record = { ...identity, binarySha256: digest(await readFile(built)) };
    const stagedStamp = path.join(stage, "source.json");
    await writeFile(stagedStamp, JSON.stringify(record, null, 2) + "\n");
    await rename(built, output);
    await rename(stagedStamp, stamp);
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
}
