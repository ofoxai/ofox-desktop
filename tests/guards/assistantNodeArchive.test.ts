// @vitest-environment node
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  realpath,
  rm,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { afterEach, describe, expect, it } from "vitest";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});

async function prepareFixture(platform: string, systemRoot: string | null) {
  const root = await realpath(
    await mkdtemp(path.join(tmpdir(), "ofox-node-archive-")),
  );
  roots.push(root);
  const scripts = path.join(root, "scripts");
  const runtime = path.join(root, "assistant-runtime");
  const cache = path.join(runtime, ".cache");
  await mkdir(scripts);
  await mkdir(cache, { recursive: true });
  for (const name of [
    "assistant-prepare.mjs",
    "assistant-entitlements.mjs",
    "assistant-install-stamp.mjs",
    "assistant-prune.mjs",
    "python-assets.mjs",
    "python-downloads.json",
  ])
    await copyFile(path.resolve("scripts", name), path.join(scripts, name));
  const target = `${platform}-x64`;
  const executable = platform === "win32" ? "node.exe" : "node";
  const archive = path.join(
    cache,
    platform === "win32" ? "node.zip" : "node.tar.gz",
  );
  const archiveBytes = Buffer.from("checksum-verified fixture archive");
  await writeFile(archive, archiveBytes);
  await writeFile(
    path.join(runtime, "node-downloads.json"),
    JSON.stringify({
      version: "22.23.4",
      [target]: {
        archive: path.basename(archive),
        executable,
        sha256: createHash("sha256").update(archiveBytes).digest("hex"),
      },
    }),
  );
  const extraction = path.join(cache, `extract-${target}`);
  const prefix = `node-v22.23.4-${platform === "win32" ? "win" : platform}-x64`;
  const node = path.join(runtime, "resources", "node", executable);
  const expectedTar =
    platform === "win32" && systemRoot
      ? path.win32.join(systemRoot, "System32", "tar.exe")
      : "tar";
  const preload = path.join(root, "preload.mjs");
  await writeFile(
    preload,
    `
    import assert from "node:assert/strict";
    import childProcess from "node:child_process";
    import { mkdirSync, writeFileSync } from "node:fs";
    import { syncBuiltinESMExports } from "node:module";
    import path from "node:path";
    Object.defineProperty(process, "platform", { value: ${JSON.stringify(platform)} });
    Object.defineProperty(process, "arch", { value: "x64" });
    delete process.env.SystemRoot;
    delete process.env.SYSTEMROOT;
    ${systemRoot === null ? "" : `process.env.SystemRoot = ${JSON.stringify(systemRoot)};`}
    process.env.PATH = ${JSON.stringify("C:\\Program Files\\Git\\usr\\bin")};
    globalThis.fetch = () => { throw new Error("unexpected network request"); };
    childProcess.execFileSync = (command, args) => {
      if (args[0] === "--version") {
        assert.equal(command, ${JSON.stringify(node)});
        return "v22.23.4\\n";
      }
      assert.equal(command, ${JSON.stringify(expectedTar)}, "must not select Git Bash GNU tar for a Windows ZIP");
      assert.deepEqual(args, ["-xf", ${JSON.stringify(archive)}, "-C", ${JSON.stringify(extraction)}]);
      const extracted = path.join(${JSON.stringify(extraction)}, ${JSON.stringify(prefix)});
      mkdirSync(extracted, { recursive: true });
      writeFileSync(path.join(extracted, ${JSON.stringify(executable)}), "fixture node");
      writeFileSync(path.join(extracted, "LICENSE"), "fixture license");
      return "";
    };
    syncBuiltinESMExports();
  `,
  );
  return {
    root,
    runtime,
    node,
    run: () =>
      execFileSync(
        process.execPath,
        [
          "--import",
          pathToFileURL(preload).href,
          path.join(scripts, "assistant-prepare.mjs"),
          "--node-only",
        ],
        {
          cwd: root,
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
        },
      ),
  };
}

describe("cold bundled Node archive preparation", () => {
  it.each([
    ["win32", "D:\\Windows"],
    ["win32", "C:\\Windows Root"],
    ["darwin", null],
    ["linux", null],
  ])(
    "extracts on %s with SystemRoot=%s without resolving Windows tar through PATH",
    async (platform, systemRoot) => {
      const fixture = await prepareFixture(platform!, systemRoot);
      expect(fixture.run()).toContain(fixture.node);
      const stamp = JSON.parse(
        await readFile(
          path.join(fixture.runtime, "resources", "node", "source.json"),
          "utf8",
        ),
      );
      expect(stamp.platform).toBe(`${platform}-x64`);
      expect(stamp.executableSha256).toBe(
        createHash("sha256").update("fixture node").digest("hex"),
      );
    },
  );

  it.each([null, "relative-root"])(
    "refuses an invalid Windows SystemRoot (%s)",
    async (systemRoot) => {
      const fixture = await prepareFixture("win32", systemRoot);
      expect(fixture.run).toThrow(/absolute Windows SystemRoot/);
    },
  );
});
