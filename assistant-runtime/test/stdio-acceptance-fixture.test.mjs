import assert from "node:assert/strict";
import { mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { createStdioAcceptance } from "./fixtures/stdio-acceptance.mjs";

function selectNode(t, value) {
  const previous = process.env.OFOX_STDIO_TEST_NODE;
  process.env.OFOX_STDIO_TEST_NODE = value;
  t.after(() => {
    if (previous === undefined) delete process.env.OFOX_STDIO_TEST_NODE;
    else process.env.OFOX_STDIO_TEST_NODE = previous;
  });
}

test("stdio fixture rejects a relative environment Node before creating fixture state", async (t) => {
  selectNode(t, "relative-node");
  await assert.rejects(
    createStdioAcceptance(t, () => assert.fail("No gateway may start")),
    /OFOX_STDIO_TEST_NODE must be absolute/,
  );
});

test("stdio fixture refuses non-regular and missing environment Nodes", async (t) => {
  const root = await mkdtemp(path.join(tmpdir(), "ofox-stdio-node-selection-"));
  t.after(() => rm(root, { recursive: true, force: true }));
  const previous = process.env.OFOX_STDIO_TEST_NODE;
  t.after(() => {
    if (previous === undefined) delete process.env.OFOX_STDIO_TEST_NODE;
    else process.env.OFOX_STDIO_TEST_NODE = previous;
  });
  for (const candidate of [root, path.join(root, "missing-node")]) {
    process.env.OFOX_STDIO_TEST_NODE = candidate;
    await assert.rejects(
      createStdioAcceptance(t, () => assert.fail("No gateway may start")),
      /OFOX_STDIO_TEST_NODE must be a regular file/,
    );
  }
});

test(
  "stdio fixture refuses a symlink environment Node",
  {
    skip:
      process.platform === "win32"
        ? "file symlinks require Windows privileges"
        : false,
  },
  async (t) => {
    const root = await mkdtemp(path.join(tmpdir(), "ofox-stdio-node-symlink-"));
    t.after(() => rm(root, { recursive: true, force: true }));
    const regular = path.join(root, "regular-node");
    const link = path.join(root, "node-link");
    await writeFile(regular, "fixture only, never executed");
    await symlink(regular, link);
    selectNode(t, link);
    await assert.rejects(
      createStdioAcceptance(t, () => assert.fail("No gateway may start")),
      /OFOX_STDIO_TEST_NODE must be a regular file/,
    );
  },
);

test("an explicit stdio Node takes precedence over the environment override", async (t) => {
  selectNode(t, "invalid-relative-override");
  const fixture = await createStdioAcceptance(t, undefined, {
    node: process.execPath,
  });
  assert.equal(fixture.port, undefined);
  const missing = path.join(tmpdir(), `ofox-missing-explicit-${process.pid}`);
  await assert.rejects(
    createStdioAcceptance(t, undefined, { node: missing }),
    /Bundled Node is missing.*pnpm assistant:prepare/,
  );
});

test("an explicitly selected environment Node drives the actual keyless sidecar", async (t) => {
  selectNode(t, process.execPath);
  const fixture = await createStdioAcceptance(t);
  const wire = fixture.spawnSidecar();
  assert.deepEqual(await wire.start("health"), {
    protocolVersion: 1,
    sdkVersion: "1.5.0",
  });
  await wire.close();
  assert.equal((await wire.exit).code, 0);
  assert.equal(wire.forcedTermination, false);
  await fixture.assertClean([]);
});
