import { createHash, randomUUID } from "node:crypto";
import { createReadStream, constants } from "node:fs";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import {
  copyFile,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rename,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const exec = promisify(execFile);
export const LEGACY_MAC_VERSION = "1.3.8";
const BASE = "https://desktop.ofox.ai";
const PAGE = "https://ofox.ai/download";
const platforms = {
  "darwin-aarch64": ["release/mac/arm", "dmg"],
  "windows-x86_64": ["release/windows/x64", "exe"],
  "linux-x86_64": ["release/linux/x64", "AppImage"],
};
function versionParts(value) {
  if (
    typeof value !== "string" ||
    !/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(value)
  )
    throw new Error("invalidUpdateVersion");
  const parts = value.split(".").map(Number);
  if (!parts.every(Number.isSafeInteger))
    throw new Error("invalidUpdateVersion");
  return parts;
}
function compare(a, b) {
  const aa = versionParts(a),
    bb = versionParts(b);
  for (let i = 0; i < 3; i++)
    if (aa[i] !== bb[i]) return aa[i] > bb[i] ? 1 : -1;
  return 0;
}
function macVersion(value) {
  if (typeof value !== "string" || !/^\d+\.\d+(?:\.\d+)?$/.test(value))
    throw new Error("invalidMinimumMacOS");
  const normalized = value.split(".");
  while (normalized.length < 3) normalized.push("0");
  if (compare(normalized.join("."), "13.5.0") < 0)
    throw new Error("invalidMinimumMacOS");
  return value;
}
function installer(platform, version) {
  const item = Object.hasOwn(platforms, platform) ? platforms[platform] : null;
  if (!item) throw new Error("unsupportedUpdatePlatform");
  return `${BASE}/${item[0]}/ofox_desktop_${version}.${item[1]}`;
}
function record(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function validateFeed(feed, modern) {
  if (!record(feed) || !record(feed.downloads))
    throw new Error("invalidPreviousUpdateManifest");
  versionParts(feed.version);
  if (!feed.downloads["darwin-aarch64"] || !feed.downloads["windows-x86_64"])
    throw new Error("previousPlatformEntryMissing");
  if (modern && (feed.schemaVersion !== 2 || !macVersion(feed.minimumMacOS)))
    throw new Error("invalidModernUpdateManifest");
  for (const [platform, url] of Object.entries(feed.downloads)) {
    if (!Object.hasOwn(platforms, platform) || typeof url !== "string")
      throw new Error("invalidPreviousDownload");
    const [prefix, ext] = platforms[platform];
    const start = `${BASE}/${prefix}/ofox_desktop_`;
    if (!url.startsWith(start) || !url.endsWith("." + ext))
      throw new Error("invalidPreviousDownload");
    const version = url.slice(start.length, -ext.length - 1);
    if (
      installer(platform, version) !== url ||
      compare(version, feed.version) > 0
    )
      throw new Error("invalidPreviousDownload");
  }
  if (feed.downloadPage !== undefined && feed.downloadPage !== PAGE)
    throw new Error("invalidPreviousDownloadPage");
  if (feed.notes !== undefined && typeof feed.notes !== "string")
    throw new Error("invalidPreviousUpdateManifest");
}
export function buildUpdateManifests({
  legacy,
  modern,
  version,
  platforms: selected,
  pubDate,
  notes,
  minimumMacOS,
}) {
  validateFeed(legacy, false);
  if (
    legacy.downloads["darwin-aarch64"] !==
    installer("darwin-aarch64", LEGACY_MAC_VERSION)
  )
    throw new Error("legacyMacProtectionMissing");
  if (compare(version, LEGACY_MAC_VERSION) <= 0)
    throw new Error("historicalReleaseIsImmutable");
  if (compare(version, legacy.version) < 0)
    throw new Error("updateManifestDowngrade");
  if (modern !== null) {
    validateFeed(modern, true);
    if (compare(version, modern.version) < 0)
      throw new Error("updateManifestDowngrade");
  }
  if (
    !Array.isArray(selected) ||
    selected.length === 0 ||
    new Set(selected).size !== selected.length
  )
    throw new Error("invalidUpdatePlatforms");
  if (typeof pubDate !== "string" || !Number.isFinite(Date.parse(pubDate)))
    throw new Error("invalidUpdateDate");
  if (notes !== undefined && typeof notes !== "string")
    throw new Error("invalidUpdateNotes");
  macVersion(minimumMacOS);
  const oldDownloads = { ...legacy.downloads };
  const modernDownloads = { ...(modern ?? legacy).downloads };
  for (const platform of Object.keys(platforms).filter(
    (key) => !key.startsWith("darwin-"),
  )) {
    const a = oldDownloads[platform],
      b = modernDownloads[platform];
    if (!a && !b) continue;
    const start = BASE + "/" + platforms[platform][0] + "/ofox_desktop_";
    const suffix = "." + platforms[platform][1];
    const v = (url) => url.slice(start.length, -suffix.length);
    const chosen = !a ? b : !b ? a : compare(v(a), v(b)) >= 0 ? a : b;
    oldDownloads[platform] = chosen;
    modernDownloads[platform] = chosen;
  }
  for (const platform of selected) {
    const url = installer(platform, version);
    modernDownloads[platform] = url;
    if (!platform.startsWith("darwin-")) oldDownloads[platform] = url;
  }
  const common = {
    version,
    pubDate,
    notes: notes ?? modern?.notes ?? legacy.notes ?? "",
    downloadPage: PAGE,
  };
  return {
    legacy: { ...common, downloads: oldDownloads },
    modern: {
      ...common,
      schemaVersion: 2,
      minimumMacOS,
      downloads: modernDownloads,
    },
  };
}
async function save(path, value) {
  await mkdir(dirname(path), { recursive: true });
  const temporary = path + "." + randomUUID() + ".tmp";
  try {
    await writeFile(temporary, JSON.stringify(value, null, 2) + "\n", {
      flag: "wx",
      mode: 0o600,
    });
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}
export const UPDATE_FEED_LOCK_KEY = "release-control/update-feed-lock.json";
function missing(error) {
  return /\(NoSuchKey\)/.test(error?.stderr ?? "");
}
function conflict(error) {
  return /\((?:PreconditionFailed|ConditionalRequestConflict)\)/.test(
    error?.stderr ?? "",
  );
}
function validEtag(value) {
  if (typeof value !== "string" || !/^"[a-fA-F0-9-]{1,128}"$/.test(value))
    throw new Error("updateObjectETagMissing");
  return value;
}
async function hashFile(path) {
  const info = await lstat(path);
  if (
    !info.isFile() ||
    info.isSymbolicLink() ||
    info.size === 0 ||
    info.size > 1024 ** 3
  )
    throw new Error("invalidUpdateArtifact");
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return { bytes: info.size, sha256: hash.digest("hex") };
}
async function checkPublic(expected) {
  for (let attempt = 0; attempt < 5; attempt++) {
    try {
      const read = async (key) => {
        const response = await fetch(BASE + "/" + key, {
          headers: {
            "User-Agent": "ofox-desktop",
            "Cache-Control": "no-cache",
          },
          signal: AbortSignal.timeout(15000),
          redirect: "error",
        });
        if (!response.ok) throw new Error("publicUpdateManifestUnavailable");
        if (!response.body) throw new Error("invalidPublicUpdateManifest");
        const reader = response.body.getReader(),
          chunks = [];
        let bytes = 0;
        try {
          for (;;) {
            const item = await reader.read();
            if (item.done) break;
            bytes += item.value.byteLength;
            if (bytes > 1024 * 1024)
              throw new Error("invalidPublicUpdateManifest");
            chunks.push(item.value);
          }
        } finally {
          await reader.cancel();
        }
        return JSON.parse(
          new TextDecoder("utf-8", { fatal: true }).decode(
            Buffer.concat(chunks),
          ),
        );
      };
      verifyPublishedManifests({
        expected,
        published: {
          modern: await read("latest-v2.json"),
          legacy: await read("latest.json"),
        },
      });
      return;
    } catch {
      if (attempt === 4)
        throw new Error("publicUpdateManifestVerificationFailed");
      await new Promise((resolve) => setTimeout(resolve, 10000));
    }
  }
}
export async function publishUpdateRelease({
  bucket,
  endpoint,
  version,
  artifacts,
  minimumMacOS,
  pubDate,
  notes,
  output,
  dryRun = false,
  run,
  verifyPublic = checkPublic,
  validateArtifact,
}) {
  // These checks run before any remote write, including lock acquisition.
  if (
    !/^[a-z0-9][a-z0-9-]{1,62}$/.test(bucket) ||
    !/^https:\/\/[a-zA-Z0-9-]+\.r2\.cloudflarestorage\.com$/.test(
      endpoint ?? "",
    )
  )
    throw new Error("invalidManifestEndpoint");
  if (compare(version, LEGACY_MAC_VERSION) <= 0)
    throw new Error("historicalReleaseIsImmutable");
  if (
    !Array.isArray(artifacts) ||
    artifacts.length === 0 ||
    new Set(artifacts.map((item) => item.platform)).size !== artifacts.length
  )
    throw new Error("invalidUpdatePlatforms");
  const root = await mkdtemp(join(tmpdir(), "ofox-update-publish-"));
  const execute =
    run ??
    (async (tool, args) =>
      (
        await exec(tool, args, {
          timeout: 60000,
          maxBuffer: 1024 * 1024,
          env: {
            ...process.env,
            AWS_MAX_ATTEMPTS: "1",
            AWS_REQUEST_CHECKSUM_CALCULATION: "WHEN_REQUIRED",
            AWS_RESPONSE_CHECKSUM_VALIDATION: "WHEN_REQUIRED",
          },
        })
      ).stdout);
  const common = [
    "--bucket",
    bucket,
    "--endpoint-url",
    endpoint,
    "--output",
    "json",
  ];
  const get = async (key, allowMissing = false, limit = 1024 * 1024) => {
    const path = join(root, randomUUID());
    let metadata;
    try {
      metadata = JSON.parse(
        await execute("aws", [
          "s3api",
          "get-object",
          ...common,
          "--key",
          key,
          "--range",
          "bytes=0-" + limit,
          path,
        ]),
      );
    } catch (error) {
      if (allowMissing && missing(error)) return null;
      throw new Error("updateObjectReadFailed:" + key);
    }
    return { path, etag: validEtag(metadata.ETag) };
  };
  const getJson = async (key, allowMissing = false) => {
    const object = await get(key, allowMissing);
    if (!object) return null;
    if ((await lstat(object.path)).size > 1024 * 1024)
      throw new Error("invalidStoredUpdateManifest");
    let value;
    try {
      value = JSON.parse(await readFile(object.path, "utf8"));
    } catch {
      throw new Error("invalidStoredUpdateManifest");
    }
    if (!record(value)) throw new Error("invalidStoredUpdateManifest");
    return { ...object, value };
  };
  const put = async (key, path, etag, contentType = "application/json") => {
    try {
      const metadata = JSON.parse(
        await execute("aws", [
          "s3api",
          "put-object",
          ...common,
          "--key",
          key,
          "--body",
          path,
          "--content-type",
          contentType,
          "--cache-control",
          "no-cache, max-age=0",
          ...(etag ? ["--if-match", etag] : ["--if-none-match", "*"]),
        ]),
      );
      return validEtag(metadata.ETag);
    } catch (error) {
      if (conflict(error))
        throw Object.assign(
          new Error("updateObjectConditionalConflict:" + key),
          { conditionalConflict: true },
        );
      throw new Error("updateObjectWriteOutcomeUnknown:" + key);
    }
  };
  let lease = null,
    failure = null,
    result;
  try {
    const staged = [];
    for (const item of artifacts) {
      const url = installer(item.platform, version);
      await hashFile(item.path);
      const path = join(root, randomUUID());
      await copyFile(item.path, path, constants.COPYFILE_EXCL);
      const digest = await hashFile(path);
      await validateArtifact?.({ platform: item.platform, path });
      if (
        item.sha256 &&
        (item.sha256 !== digest.sha256 || item.bytes !== digest.bytes)
      )
        throw new Error("updateArtifactProvenanceMismatch");
      staged.push({
        platform: item.platform,
        path,
        digest,
        key: url.slice(BASE.length + 1),
      });
    }
    if (!dryRun) {
      const previous = await getJson(UPDATE_FEED_LOCK_KEY, true);
      if (
        previous &&
        (previous.value?.schemaVersion !== 1 ||
          !["active", "idle"].includes(previous.value?.state))
      )
        throw new Error("invalidUpdateFeedLock");
      if (previous?.value.state === "active")
        throw new Error("updateFeedReleaseLocked");
      const owner = randomUUID(),
        path = join(root, "lock.json");
      await save(path, {
        schemaVersion: 1,
        state: "active",
        owner,
        version,
        createdAt: new Date().toISOString(),
      });
      let etag;
      try {
        etag = await put(UPDATE_FEED_LOCK_KEY, path, previous?.etag);
      } catch (error) {
        if (error.conditionalConflict)
          throw new Error("updateFeedReleaseLocked");
        throw error;
      }
      // No timeout-based stealing: an unknown acquisition leaves a recoverable
      // active object. Only a confirmed owner may release its exact ETag.
      lease = { owner, etag };
    }
    const legacy = await getJson("latest.json");
    const modern = await getJson("latest-v2.json", true);
    const expected = buildUpdateManifests({
      legacy: legacy.value,
      modern: modern?.value ?? null,
      version,
      platforms: staged.map((item) => item.platform),
      minimumMacOS,
      pubDate,
      notes,
    });
    await save(join(output, "latest-v2.json"), expected.modern);
    await save(join(output, "latest.json"), expected.legacy);
    if (dryRun) return expected;
    for (const item of staged) {
      const type = item.platform.startsWith("darwin-")
        ? "application/x-apple-diskimage"
        : item.platform.startsWith("windows-")
          ? "application/vnd.microsoft.portable-executable"
          : "application/x-executable";
      try {
        await put(item.key, item.path, null, type);
      } catch (error) {
        if (!error.conditionalConflict) throw error;
        const existing = await get(item.key, false, item.digest.bytes);
        const digest = await hashFile(existing.path);
        if (
          digest.bytes !== item.digest.bytes ||
          digest.sha256 !== item.digest.sha256
        )
          throw new Error("updateArtifactIsImmutable:" + item.key);
      }
    }
    await put("latest-v2.json", join(output, "latest-v2.json"), modern?.etag);
    await put("latest.json", join(output, "latest.json"), legacy.etag);
    verifyPublishedManifests({
      expected,
      published: {
        modern: (await getJson("latest-v2.json")).value,
        legacy: (await getJson("latest.json")).value,
      },
    });
    await verifyPublic(expected);
    result = expected;
  } catch (error) {
    failure = error;
  } finally {
    if (lease) {
      try {
        const path = join(root, "idle.json");
        await save(path, {
          schemaVersion: 1,
          state: "idle",
          owner: lease.owner,
          version,
        });
        await put(UPDATE_FEED_LOCK_KEY, path, lease.etag);
      } catch {
        failure = new Error(
          (failure?.message ? failure.message + ";" : "") +
            "updateFeedLeaseRecoveryRequired",
        );
      }
    }
    await rm(root, { recursive: true, force: true });
  }
  if (failure) throw failure;
  return result;
}

function canonical(value) {
  if (Array.isArray(value)) return value.map(canonical);
  if (record(value))
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonical(value[key])]),
    );
  return value;
}
export function verifyPublishedManifests({ expected, published }) {
  if (
    JSON.stringify(canonical(expected)) !== JSON.stringify(canonical(published))
  )
    throw new Error("publishedUpdateManifestsMismatch");
}
async function cli() {
  const [command, ...rest] = process.argv.slice(2);
  const flags = new Map();
  for (let i = 0; i < rest.length; i++) {
    const name = rest[i];
    if (!name.startsWith("--") || flags.has(name))
      throw new Error("invalidArguments");
    flags.set(name, name === "--dry-run" ? true : rest[++i]);
  }
  const get = (name) => {
    const v = flags.get("--" + name);
    if (typeof v !== "string" || !v) throw new Error("missingArgument:" + name);
    return v;
  };
  if (command === "publish") {
    const capabilities = JSON.parse(
      (
        await exec(
          "aws",
          ["s3api", "put-object", "--generate-cli-skeleton", "input"],
          { timeout: 15000, maxBuffer: 1024 * 1024 },
        )
      ).stdout,
    );
    if (!("IfMatch" in capabilities) || !("IfNoneMatch" in capabilities))
      throw new Error("awsCliConditionalWritesRequired");
    const version = get("version");
    const config = JSON.parse(
      await readFile(
        new URL("../src-tauri/tauri.conf.json", import.meta.url),
        "utf8",
      ),
    );
    let artifacts, validateArtifact;
    if (flags.has("--artifacts-root")) {
      const { checkRelease } = await import("./release-artifacts.mjs");
      const root = get("artifacts-root");
      const proof = await checkRelease({
        artifactsRoot: root,
        sourceSha: get("source-sha"),
        version,
      });
      artifacts = proof.platforms.map((item) => ({
        platform: item.platform,
        path: join(root, "build-" + item.platform, item.artifact.file),
        sha256: item.artifact.sha256,
        bytes: item.artifact.bytes,
      }));
    } else {
      const artifact = get("artifact"),
        separator = artifact.indexOf("=");
      if (separator < 0 || artifact.slice(0, separator) !== "darwin-aarch64")
        throw new Error("unsupportedEmergencyUpdatePlatform");
      artifacts = [
        { platform: "darwin-aarch64", path: artifact.slice(separator + 1) },
      ];
      const { checkMacDmgMinimum } = await import("./release-artifacts.mjs");
      validateArtifact = async (item) => {
        await checkMacDmgMinimum({ dmg: item.path });
      };
    }
    await publishUpdateRelease({
      bucket: get("bucket"),
      endpoint: get("endpoint"),
      version,
      artifacts,
      minimumMacOS: config.bundle.macOS.minimumSystemVersion,
      pubDate: get("pub-date"),
      notes: flags.get("--notes"),
      output: get("output"),
      dryRun: flags.has("--dry-run"),
      validateArtifact,
    });
    return;
  }
  throw new Error("unknownCommand");
}
if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(resolve(process.argv[1])).href
) {
  cli().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
