import {
  readFile,
  mkdir,
  writeFile,
  mkdtemp,
  rename,
  rm,
  lstat,
} from "node:fs/promises";
import { createHash, randomUUID } from "node:crypto";
import path from "node:path";
import { jpegSize } from "./presentation-resource-coverage.mjs";

const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
async function directory(base, relative) {
  let current = base;
  for (const part of relative.split("/")) {
    current = path.join(current, part);
    await mkdir(current).catch((error) => {
      if (error.code !== "EEXIST") throw error;
    });
    const stat = await lstat(current);
    if (!stat.isDirectory() || stat.isSymbolicLink())
      throw new Error("Preview output directory is not a plain directory");
  }
}

/**
 * Publish immutable content-addressed assets, then atomically switch one evidence
 * pointer. Interruptions never overwrite old evidenced bytes. Generators pass
 * a task-private stagingRoot outside packaged resources and run serially.
 */
export async function publishSourcePreviewBatch(
  base,
  records,
  files,
  hooks = {},
) {
  const filename = path.join(base, "source-preview-evidence.json");
  const temporaryEvidence = filename + "." + randomUUID() + ".tmp";
  const stage = await mkdtemp(
    path.join(
      hooks.stagingRoot || path.dirname(base),
      ".source-preview-stage-",
    ),
  );
  const normalized = structuredClone(records),
    paths = new Map(),
    created = [];
  let committed = false;
  try {
    for (const [relative, bytes] of files) {
      const match =
        /^source-previews\/([a-z0-9-]+)\/(page-\d+|rebuilt-original)\.(jpg|pptx)$/.exec(
          relative,
        );
      if (!match) throw new Error("Invalid original preview output path");
      const sha256 = hash(bytes),
        immutable = `source-previews/${match[1]}/${match[2]}-${sha256}.${match[3]}`;
      paths.set(relative, { path: immutable, sha256 });
      await writeFile(path.join(stage, immutable.replaceAll("/", "_")), bytes, {
        flag: "wx",
      });
    }
    const references = new Set();
    for (const record of normalized) {
      if (!/^[a-z0-9-]+$/.test(record.id))
        throw new Error("Invalid original style ID");
      for (const asset of [
        ...record.pages,
        ...(record.rebuiltSourceDeck ? [record.rebuiltSourceDeck] : []),
      ]) {
        const replacement = paths.get(asset.path);
        if (
          !asset.path.startsWith(`source-previews/${record.id}/`) ||
          !replacement ||
          asset.sha256 !== replacement.sha256 ||
          references.has(asset.path)
        )
          throw new Error(
            "Original preview publication bytes and evidence disagree",
          );
        if (asset.path.endsWith(".jpg")) {
          const bytes = files.get(asset.path);
          if (
            bytes[0] !== 0xff ||
            bytes[1] !== 0xd8 ||
            bytes[2] !== 0xff ||
            asset.width !== 1920 ||
            asset.height !== 1080 ||
            jpegSize(bytes).some(
              (size, at) => size !== [asset.width, asset.height][at],
            )
          )
            throw new Error("Invalid original preview JPEG before publication");
        }
        references.add(asset.path);
        Object.assign(asset, replacement);
      }
    }
    if (references.size !== files.size)
      throw new Error("Unreferenced original preview output");
    let index = 0;
    for (const replacement of paths.values()) {
      await hooks.beforePublishFile?.(index++);
      await directory(base, path.posix.dirname(replacement.path));
      const target = path.join(base, replacement.path);
      try {
        const stat = await lstat(target);
        if (!stat.isFile() || stat.isSymbolicLink())
          throw new Error("Immutable original preview is not a plain file");
        if (hash(await readFile(target)) !== replacement.sha256) {
          // A corrupt content-addressed output can be repaired using bytes
          // already verified against its filename/record. Valid old outputs
          // are never overwritten; a failed repair batch leaves any repaired
          // old reference valid rather than restoring corrupt bytes.
          await rename(
            path.join(stage, replacement.path.replaceAll("/", "_")),
            target,
          );
        }
      } catch (error) {
        if (error.code !== "ENOENT") throw error;
        await rename(
          path.join(stage, replacement.path.replaceAll("/", "_")),
          target,
        );
        created.push(target);
      }
    }
    await hooks.beforeCommit?.();
    let existing = { schemaVersion: 1, styles: [] };
    try {
      existing = JSON.parse(await readFile(filename, "utf8"));
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
    const ids = new Set(normalized.map((record) => record.id));
    existing.styles = [
      ...existing.styles.filter((record) => !ids.has(record.id)),
      ...normalized,
    ];
    await writeFile(
      temporaryEvidence,
      JSON.stringify(existing, null, 2) + "\n",
      { flag: "wx" },
    );
    await rename(temporaryEvidence, filename);
    committed = true;
    return normalized;
  } finally {
    if (!committed)
      for (const target of created)
        await rm(target, { force: true }).catch(() => {});
    await rm(temporaryEvidence, { force: true }).catch(() => {});
    await rm(stage, { recursive: true, force: true }).catch(() => {});
  }
}
