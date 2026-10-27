import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const exec = promisify(execFile);
const platforms = { "darwin-aarch64": "dmg", "windows-x86_64": "exe" };
function requireInput(sourceSha, version) {
  if (
    !/^[a-f0-9]{40}$/.test(sourceSha ?? "") ||
    !/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(version ?? "")
  )
    throw new Error("invalidReleaseIdentity");
}
async function digest(path) {
  const stat = await lstat(path);
  if (!stat.isFile() || stat.isSymbolicLink())
    throw new Error("unexpectedArtifact");
  return {
    sha256: createHash("sha256")
      .update(await readFile(path))
      .digest("hex"),
    bytes: stat.size,
  };
}
async function json(path) {
  return JSON.parse(await readFile(path, "utf8"));
}
function normalizedMacVersion(value) {
  if (typeof value !== "string" || !/^\d+\.\d+(?:\.\d+)?$/.test(value))
    return null;
  const parts = value.split(".").map(Number);
  if (!parts.every(Number.isSafeInteger)) return null;
  while (parts.length < 3) parts.push(0);
  return parts.join(".");
}
async function configuredMacMinimum() {
  const config = await json(
    new URL("../src-tauri/tauri.conf.json", import.meta.url),
  );
  const value = config.bundle?.macOS?.minimumSystemVersion;
  if (!normalizedMacVersion(value))
    throw new Error("invalidMinimumMacOSConfiguration");
  return value;
}
async function saveJson(path, value) {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${process.pid}.tmp`;
  await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, {
    mode: 0o600,
    flag: "wx",
  });
  try {
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}
async function installer(root, platform, version) {
  if (!(platform in platforms)) throw new Error("unsupportedPlatform");
  const stat = await lstat(root);
  if (!stat.isDirectory() || stat.isSymbolicLink())
    throw new Error("unexpectedArtifact");
  const file = `ofox_desktop_${version}.${platforms[platform]}`;
  const entries = await readdir(root);
  if (entries.length !== 1 || entries[0] !== file)
    throw new Error("unexpectedArtifact");
  return { file, ...(await digest(join(root, file))) };
}
export async function recordBuild({
  root,
  output,
  platform,
  version,
  sourceSha,
}) {
  requireInput(sourceSha, version);
  const result = {
    schemaVersion: 1,
    sourceSha,
    version,
    platform,
    artifact: await installer(root, platform, version),
  };
  await saveJson(output, result);
  return result;
}

// Do not expose credential arguments, subprocess stderr or raw notary errors in logs.
async function runCommand(command, args) {
  try {
    const result = await exec(command, args, {
      encoding: "utf8",
      timeout: 31 * 60_000,
      maxBuffer: 2 * 1024 * 1024,
    });
    return `${result.stdout}\n${result.stderr}`.trim();
  } catch {
    throw new Error(`releaseVerificationCommandFailed:${basename(command)}`);
  }
}
function appleAuth(auth) {
  if (!auth?.appleId || !auth.password || !auth.teamId)
    throw new Error("notarizationCredentialsUnavailable");
  return [
    "--apple-id",
    auth.appleId,
    "--password",
    auth.password,
    "--team-id",
    auth.teamId,
  ];
}
async function verifyAppMinimum(app, run) {
  const info = join(app, "Contents", "Info.plist");
  const minimumSystemVersion = await configuredMacMinimum();
  const actualMinimum = (
    await run("plutil", [
      "-extract",
      "LSMinimumSystemVersion",
      "raw",
      "-o",
      "-",
      info,
    ])
  ).trim();
  if (
    normalizedMacVersion(actualMinimum) !==
    normalizedMacVersion(minimumSystemVersion)
  )
    throw new Error("minimumMacOSMismatch");
  return minimumSystemVersion;
}

// The emergency uploader must check the selected DMG itself, including --skip-build.
export async function checkMacDmgMinimum({ dmg, run = runCommand }) {
  if (!dmg) throw new Error("unexpectedArtifact");
  const stat = await lstat(dmg);
  if (!stat.isFile() || stat.isSymbolicLink())
    throw new Error("unexpectedArtifact");
  const mount = await mkdtemp(join(tmpdir(), "ofox-minimum-mount-"));
  let attached = false;
  try {
    await run("hdiutil", [
      "attach",
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mount,
      "-plist",
      dmg,
    ]);
    attached = true;
    const apps = (await readdir(mount)).filter((entry) =>
      entry.endsWith(".app"),
    );
    if (apps.length !== 1) throw new Error("unexpectedAppBundle");
    const app = join(mount, apps[0]);
    const appStat = await lstat(app);
    if (!appStat.isDirectory() || appStat.isSymbolicLink())
      throw new Error("unexpectedAppBundle");
    return await verifyAppMinimum(app, run);
  } finally {
    if (attached) await run("hdiutil", ["detach", mount]);
    await rm(mount, { recursive: true, force: true });
  }
}

async function verifyMac({
  dmg,
  submissionId,
  expectedTeamId,
  version,
  auth,
  run,
}) {
  if (
    !/^[\w-]+$/.test(expectedTeamId ?? "") ||
    !/^[a-f0-9-]{36}$/i.test(submissionId)
  )
    throw new Error("invalidSigningIdentity");
  const credentials = appleAuth(auth);
  let accepted = false;
  // Bounded polling preserves the already submitted job. A retry never submits again.
  for (let attempt = 0; attempt < 10; attempt += 1) {
    let status;
    try {
      status = JSON.parse(
        await run("xcrun", [
          "notarytool",
          "info",
          submissionId,
          ...credentials,
          "--output-format",
          "json",
        ]),
      ).status;
    } catch {
      throw new Error("notarizationResponseInvalid");
    }
    if (status === "Accepted") {
      accepted = true;
      break;
    }
    if (["Invalid", "Rejected"].includes(status))
      throw new Error("notarizationRejected");
    if (status !== "In Progress") throw new Error("notarizationStatusUnknown");
    try {
      await run("xcrun", [
        "notarytool",
        "wait",
        submissionId,
        ...credentials,
        "--timeout",
        "30m",
      ]);
    } catch {
      /* Re-read the submitted job; do not infer acceptance from a wait exit code. */
    }
  }
  if (!accepted) throw new Error("notarizationPending");
  await run("xcrun", ["stapler", "staple", dmg]);
  await run("xcrun", ["stapler", "validate", dmg]);
  const stapledArtifact = await digest(dmg);
  await run("spctl", [
    "--assess",
    "--type",
    "open",
    "--context",
    "context:primary-signature",
    "--verbose=2",
    dmg,
  ]);
  const mount = await mkdtemp(join(tmpdir(), "ofox-release-mount-"));
  let attached = false;
  try {
    await run("hdiutil", [
      "attach",
      "-readonly",
      "-nobrowse",
      "-mountpoint",
      mount,
      "-plist",
      dmg,
    ]);
    attached = true;
    const apps = (await readdir(mount)).filter((entry) =>
      entry.endsWith(".app"),
    );
    if (apps.length !== 1) throw new Error("unexpectedAppBundle");
    const app = join(mount, apps[0]);
    if (
      !(await lstat(app)).isDirectory() ||
      (await lstat(app)).isSymbolicLink()
    )
      throw new Error("unexpectedAppBundle");
    await run("codesign", [
      "--verify",
      "--deep",
      "--strict",
      "--verbose=2",
      app,
    ]);
    const signature = await run("codesign", ["-dv", "--verbose=4", app]);
    if (
      !signature.includes("Authority=Developer ID Application:") ||
      signature.match(/^TeamIdentifier=(.+)$/m)?.[1]?.trim() !== expectedTeamId
    )
      throw new Error("signingIdentityMismatch");
    const info = join(app, "Contents", "Info.plist");
    if (
      (
        await run("plutil", [
          "-extract",
          "CFBundleIdentifier",
          "raw",
          "-o",
          "-",
          info,
        ])
      ).trim() !== "ai.ofox.desktop" ||
      (
        await run("plutil", [
          "-extract",
          "CFBundleShortVersionString",
          "raw",
          "-o",
          "-",
          info,
        ])
      ).trim() !== version
    )
      throw new Error("appIdentityMismatch");
    const minimumSystemVersion = await verifyAppMinimum(app, run);
    await run("spctl", ["--assess", "--type", "execute", "--verbose=2", app]);
    return {
      artifact: stapledArtifact,
      notarization: {
        status: "Accepted",
        submissionId,
        teamId: expectedTeamId,
        appBundleId: "ai.ofox.desktop",
        minimumSystemVersion,
        appCodeSignatureVerified: true,
        dmgStapleVerified: true,
        gatekeeperVerified: true,
      },
    };
  } finally {
    if (attached) await run("hdiutil", ["detach", mount]);
    // A failed detach must never recursively remove a still-mounted image.
    await rm(mount, { recursive: true, force: true });
  }
}

export async function verifyRelease({
  artifactsRoot,
  provenanceRoot,
  notaryRoot,
  sourceSha,
  version,
  expectedTeamId,
  auth,
  run = runCommand,
}) {
  requireInput(sourceSha, version);
  const expectedDirectories = Object.keys(platforms)
    .map((platform) => `build-${platform}`)
    .sort();
  if (
    JSON.stringify((await readdir(artifactsRoot)).sort()) !==
    JSON.stringify(expectedDirectories)
  )
    throw new Error("unexpectedArtifact");
  const records = [];
  // Validate every platform before any notarization/stapling side effect.
  for (const platform of Object.keys(platforms)) {
    const build = await json(
      join(provenanceRoot, `provenance-${platform}`, "build-provenance.json"),
    );
    if (
      build.schemaVersion !== 1 ||
      build.sourceSha !== sourceSha ||
      build.version !== version ||
      build.platform !== platform
    )
      throw new Error("buildProvenanceMismatch");
    const artifact = await installer(
      join(artifactsRoot, `build-${platform}`),
      platform,
      version,
    );
    if (
      build.artifact?.file !== artifact.file ||
      build.artifact?.sha256 !== artifact.sha256 ||
      build.artifact?.bytes !== artifact.bytes
    )
      throw new Error("artifactChanged");
    records.push({ platform, artifact });
  }
  const submissionId = (
    await readFile(
      join(notaryRoot, "notary-darwin-aarch64", "submission-id"),
      "utf8",
    )
  ).trim();
  const mac = records.find((record) => record.platform === "darwin-aarch64");
  const verifiedMac = await verifyMac({
    dmg: resolve(artifactsRoot, "build-darwin-aarch64", mac.artifact.file),
    submissionId,
    expectedTeamId,
    version,
    auth,
    run,
  });
  mac.notarization = verifiedMac.notarization;
  mac.artifact = { file: mac.artifact.file, ...verifiedMac.artifact };
  // Stapling changes the DMG. Only these final bytes may be consumed by publish.
  for (const record of records) {
    const finalArtifact = await installer(
      join(artifactsRoot, `build-${record.platform}`),
      record.platform,
      version,
    );
    if (
      record.artifact.sha256 !== finalArtifact.sha256 ||
      record.artifact.bytes !== finalArtifact.bytes
    )
      throw new Error("artifactChanged");
  }
  const manifest = {
    schemaVersion: 1,
    sourceSha,
    version,
    verifiedAt: new Date().toISOString(),
    platforms: records,
  };
  await saveJson(join(artifactsRoot, "verified-release.json"), manifest);
  return manifest;
}

export async function checkRelease({ artifactsRoot, sourceSha, version }) {
  requireInput(sourceSha, version);
  const proof = await json(join(artifactsRoot, "verified-release.json"));
  if (
    proof.schemaVersion !== 1 ||
    proof.sourceSha !== sourceSha ||
    proof.version !== version ||
    !Array.isArray(proof.platforms) ||
    proof.platforms.length !== 2
  )
    throw new Error("verificationProvenanceMismatch");
  const expectedDirectories = [
    "verified-release.json",
    ...Object.keys(platforms).map((platform) => `build-${platform}`),
  ].sort();
  if (
    JSON.stringify((await readdir(artifactsRoot)).sort()) !==
    JSON.stringify(expectedDirectories)
  )
    throw new Error("unexpectedArtifact");
  const seen = new Set();
  for (const record of proof.platforms) {
    if (!(record.platform in platforms) || seen.has(record.platform))
      throw new Error("verificationProvenanceMismatch");
    seen.add(record.platform);
    const artifact = await installer(
      join(artifactsRoot, `build-${record.platform}`),
      record.platform,
      version,
    );
    if (
      record.artifact?.file !== artifact.file ||
      record.artifact?.sha256 !== artifact.sha256 ||
      record.artifact?.bytes !== artifact.bytes
    )
      throw new Error("artifactChanged");
    if (
      record.platform === "darwin-aarch64" &&
      (record.notarization?.status !== "Accepted" ||
        record.notarization?.minimumSystemVersion !==
          (await configuredMacMinimum()) ||
        !record.notarization?.appCodeSignatureVerified ||
        !record.notarization?.dmgStapleVerified ||
        !record.notarization?.gatekeeperVerified)
    )
      throw new Error("verificationProvenanceMismatch");
  }
  return proof;
}

async function main() {
  const [mode] = process.argv.slice(2);
  if (mode === "check-macos-minimum") {
    const minimum = await checkMacDmgMinimum({ dmg: process.argv[3] });
    console.log(`App minimum macOS ${minimum} verified`);
    return;
  }
  const shared = {
    sourceSha: process.env.EXPECTED_SHA,
    version: process.env.VERSION,
  };
  if (mode === "record-build")
    await recordBuild({
      ...shared,
      root: "dist-release",
      output: "build-provenance.json",
      platform: process.env.RELEASE_PLATFORM,
    });
  else if (mode === "verify")
    await verifyRelease({
      ...shared,
      artifactsRoot: "artifacts",
      provenanceRoot: "provenance",
      notaryRoot: "notary",
      expectedTeamId: process.env.APPLE_TEAM_ID,
      auth: {
        appleId: process.env.APPLE_ID,
        password: process.env.APPLE_PASSWORD,
        teamId: process.env.APPLE_TEAM_ID,
      },
    });
  else if (mode === "check")
    await checkRelease({ ...shared, artifactsRoot: "artifacts" });
  else
    throw new Error("usage: release-artifacts.mjs record-build|verify|check");
  console.log(`Release artifacts ${mode} passed for ${shared.sourceSha}`);
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
