import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import {
  createWindowsProbeBuildStage,
  prepareWindowsChildProbe,
} from "../../scripts/assistant-windows-probe.mjs";

test("interrupted probe builds leave their staging files outside bundled resources", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-windows-child-probe-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const cache = path.join(root, ".cache");
  const resources = path.join(root, "resources");
  const output = path.join(resources, "probes");
  await Promise.all([mkdir(cache), mkdir(output, { recursive: true })]);

  const published = [
    "windows-child-probe.node",
    "windows-child-probe.source.json",
  ];
  await Promise.all(
    published.map((name) =>
      writeFile(path.join(output, name), "published fixture"),
    ),
  );
  const abandoned = await createWindowsProbeBuildStage({ cache, resources });
  await writeFile(
    path.join(abandoned, "cleanup-test.exe"),
    "private test fixture",
  );
  await writeFile(
    path.join(abandoned, "headers.tar.gz"),
    "private SDK fixture",
  );

  assert.equal(path.dirname(await realpath(abandoned)), await realpath(cache));
  assert.deepEqual((await readdir(output)).sort(), published);

  const nestedCache = path.join(resources, ".cache");
  await mkdir(nestedCache);
  await assert.rejects(
    createWindowsProbeBuildStage({ cache: nestedCache, resources }),
    /outside bundled resources/,
  );
  assert.deepEqual(await readdir(nestedCache), []);
});

test("the same Windows probe artifact is reused for LF and CRLF sources without MSVC", async (t) => {
  if (process.platform !== "win32" || process.arch !== "x64") {
    t.skip("requires native Windows x64");
    return;
  }
  const lf = (text) => text.replaceAll("\r\n", "\n");
  const sdkText = lf(
    await readFile(
      new URL("../native/windows-child-probe/sdk.json", import.meta.url),
      "utf8",
    ),
  );
  const sdk = JSON.parse(sdkText);
  if (process.version !== `v${sdk.version}`) {
    t.skip(`requires bundled Node v${sdk.version}`);
    return;
  }

  const root = await mkdtemp(path.join(tmpdir(), "ofox-windows-probe-reuse-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const sourceDir = path.join(
    root,
    "assistant-runtime",
    "native",
    "windows-child-probe",
  );
  const cache = path.join(root, ".cache");
  const resources = path.join(root, "resources");
  const output = path.join(resources, "probes");
  await Promise.all(
    [sourceDir, output].map((directory) =>
      mkdir(directory, { recursive: true }),
    ),
  );

  const source = "/* fixed probe source */\n";
  const cleanupTest = "/* fixed cleanup test source */\n";
  const binary = Buffer.from("prebuilt fixture bytes");
  const addonPath = path.join(output, "windows-child-probe.node");
  const stampPath = path.join(output, "windows-child-probe.source.json");
  const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
  const stamp = JSON.stringify({
    schemaVersion: 1,
    target: "win32-x64",
    nodeVersion: sdk.version,
    sourceSha256: digest(source),
    cleanupTestSha256: digest(cleanupTest),
    sdkSha256: digest(sdkText),
    buildRecipeSha256: digest(
      lf(
        await readFile(
          new URL("../../scripts/assistant-windows-probe.mjs", import.meta.url),
          "utf8",
        ),
      ),
    ),
    binarySha256: digest(binary),
  });
  await Promise.all([
    writeFile(addonPath, binary),
    writeFile(stampPath, stamp),
  ]);
  const publishedFiles = [addonPath, stampPath];
  const publicationTimes = () =>
    Promise.all(publishedFiles.map(async (file) => (await stat(file)).mtimeMs));
  const times = await publicationTimes();
  const prepare = () =>
    prepareWindowsChildProbe({
      root,
      resources,
      cache,
      node: process.execPath,
    });
  const compilerDirectory = "ProgramFiles(x86)";
  const original = process.env[compilerDirectory];
  process.env[compilerDirectory] = "fixture-without-msvc";
  try {
    for (const lineEnding of ["\n", "\r\n"]) {
      for (const [name, text] of [
        ["probe.c", source],
        ["cleanup.test.c", cleanupTest],
        ["sdk.json", sdkText],
      ]) {
        await writeFile(
          path.join(sourceDir, name),
          text.replaceAll("\n", lineEnding),
        );
      }
      await assert.doesNotReject(
        prepare,
        `expected warm reuse for ${lineEnding === "\n" ? "LF" : "CRLF"} sources`,
      );
      assert.deepEqual(await publicationTimes(), times);
      assert.deepEqual(await readFile(addonPath), binary);
      assert.equal(await readFile(stampPath, "utf8"), stamp);
    }

    const buildRequired =
      /Visual Studio Build Tools require ProgramFiles\(x86\)/;
    const tampered = Buffer.from("changed prebuilt fixture bytes");
    await writeFile(addonPath, tampered);
    await assert.rejects(prepare(), buildRequired);
    assert.deepEqual(await readFile(addonPath), tampered);

    await writeFile(addonPath, binary);
    await writeFile(
      path.join(sourceDir, "probe.c"),
      source + "/* changed */\n",
    );
    await assert.rejects(prepare(), buildRequired);
    assert.deepEqual(await readFile(addonPath), binary);
    assert.equal(await readFile(stampPath, "utf8"), stamp);
  } finally {
    if (original === undefined) {
      delete process.env[compilerDirectory];
    } else {
      process.env[compilerDirectory] = original;
    }
  }
});
