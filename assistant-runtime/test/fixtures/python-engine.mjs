import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { access, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { buffer } from "node:stream/consumers";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import yauzl from "yauzl";

// The runner is driven the way the host will drive it: the staged Node, a
// code file, and the staged Python engine assets. Where the engine is offered
// (macOS) a missing asset fails the test; elsewhere the test cannot run.
export const root = fileURLToPath(new URL("../../", import.meta.url));
export const staged = path.join(root, "resources/node/node");
export const assets = path.join(root, "resources/engines/python");
export const officecli = path.join(
  root,
  "resources/engines/officecli/officecli",
);
export const runner = path.join(root, "src/python/runner.mjs");
export const skip =
  process.platform === "darwin" ? false : "the Python engine is macOS-only";
// A cold Pyodide start (numpy, matplotlib, lxml) takes several seconds.
export const options = { skip, timeout: 180_000 };

export const exists = (file) =>
  access(file).then(
    () => true,
    () => false,
  );

export async function requireEngine(...extra) {
  for (const file of [staged, path.join(assets, "assets.json"), ...extra])
    if (!(await exists(file)))
      assert.fail(
        `${path.relative(root, file)} is missing: run pnpm assistant:prepare first`,
      );
}

export async function scratch(t) {
  const directory = await mkdtemp(path.join(tmpdir(), "ofox-python-runner-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

/** Runs `code` in the runner and returns its exit status, stderr and files. */
export async function run(t, code, { assetsDir = assets } = {}) {
  await requireEngine();
  const directory = await scratch(t);
  const files = {
    code: path.join(directory, "code.py"),
    out: path.join(directory, "out.pptx"),
    lint: path.join(directory, "lint.json"),
  };
  await writeFile(files.code, code);
  const child = spawn(
    staged,
    [
      runner,
      "--assets",
      assetsDir,
      "--code",
      files.code,
      "--out",
      files.out,
      "--lint",
      files.lint,
    ],
    {
      env: { PATH: process.env.PATH },
      stdio: ["ignore", "pipe", "pipe"],
      signal: AbortSignal.timeout(150_000),
    },
  );
  const stderr = buffer(child.stderr).then((bytes) => bytes.toString("utf8"));
  buffer(child.stdout).catch(() => {});
  const status = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  return { status, stderr: await stderr, ...files };
}

/** Every entry of a zip file by name. */
export async function zipEntries(file) {
  const zip = await promisify(yauzl.open)(file, { lazyEntries: true });
  const open = promisify(zip.openReadStream.bind(zip));
  const entries = new Map();
  await new Promise((resolve, reject) => {
    zip.on("error", reject);
    zip.on("end", resolve);
    zip.on("entry", async (entry) => {
      try {
        entries.set(entry.fileName, await buffer(await open(entry)));
        zip.readEntry();
      } catch (error) {
        reject(error);
      }
    });
    zip.readEntry();
  });
  return entries;
}
