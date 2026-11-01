// @vitest-environment node
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  installIsCurrent,
  installStamp,
  writeInstallStamp,
} from "../../scripts/assistant-install-stamp.mjs";

let root: string;
let runtime: string;
let production: string;
let stampFile: string;
let prepareScript: string;
let pruneScript: string;

async function sources(lock: string) {
  await writeFile(path.join(runtime, "package.json"), '{"name":"runtime"}');
  await writeFile(path.join(runtime, "package-lock.json"), lock);
}

beforeEach(async () => {
  root = await mkdtemp(path.join(tmpdir(), "ofox-prepare-"));
  runtime = path.join(root, "assistant-runtime");
  production = path.join(runtime, "resources", "runtime");
  stampFile = path.join(runtime, "resources", "runtime-install.json");
  await mkdir(path.join(production, "node_modules"), { recursive: true });
  await sources('{"lockfileVersion":3}');
  await mkdir(path.join(root, "scripts"));
  prepareScript = path.join(root, "scripts", "assistant-prepare.mjs");
  pruneScript = path.join(root, "scripts", "assistant-prune.mjs");
  await writeFile(prepareScript, "await install();\n");
  await writeFile(pruneScript, 'export const UNUSED = ["examples"];\n');
});

afterEach(() => rm(root, { recursive: true, force: true }));

describe("assistant runtime preparation", () => {
  const stamp = (nodeVersion = "22.23.4") =>
    installStamp({
      runtime,
      scripts: [prepareScript, pruneScript],
      nodeVersion,
      platform: "darwin-arm64",
    });

  it("installs the production tree when nothing was installed yet", async () => {
    expect(await installIsCurrent(production, stampFile, await stamp())).toBe(
      false,
    );
  });

  it("skips the install while the lockfile and Node stay the same", async () => {
    await writeInstallStamp(stampFile, await stamp());
    expect(await installIsCurrent(production, stampFile, await stamp())).toBe(
      true,
    );
  });

  it("installs again when the lockfile changes", async () => {
    await writeInstallStamp(stampFile, await stamp());
    await sources('{"lockfileVersion":3,"packages":{"undici":{}}}');
    expect(await installIsCurrent(production, stampFile, await stamp())).toBe(
      false,
    );
  });

  it("installs again for another Node version", async () => {
    await writeInstallStamp(stampFile, await stamp());
    expect(
      await installIsCurrent(production, stampFile, await stamp("22.24.0")),
    ).toBe(false);
  });

  it("installs and prunes again when the prepare or prune script changes", async () => {
    for (const [script, changed] of [
      [prepareScript, "await install({ prune: true });\n"],
      [pruneScript, 'export const UNUSED = ["examples", "docs"];\n'],
    ]) {
      await writeInstallStamp(stampFile, await stamp());
      await writeFile(script, changed);
      expect(
        await installIsCurrent(production, stampFile, await stamp()),
        script,
      ).toBe(false);
    }
  });

  it("installs again when the installed tree is gone", async () => {
    await writeInstallStamp(stampFile, await stamp());
    await rm(path.join(production, "node_modules"), { recursive: true });
    expect(await installIsCurrent(production, stampFile, await stamp())).toBe(
      false,
    );
  });
});
