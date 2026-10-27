import assert from "node:assert/strict";
import test from "node:test";
import { bundledPackages } from "../build-packages.mjs";

test("each bundled file counts for its innermost package, nested copies included", async () => {
  const manifests = {
    "node_modules/zod/package.json": { version: "4.6.7", license: "MIT" },
    "node_modules/@scope/kit/package.json": {
      version: "1.0.0",
      license: "ISC",
    },
    "node_modules/@scope/kit/node_modules/zod/package.json": {
      version: "3.25.0",
      license: "MIT",
    },
    "node_modules/a/node_modules/@inner/b/package.json": {
      version: "2.0.0",
      license: { type: "BSD-3-Clause" },
    },
  };
  const read = async (file) => {
    assert.ok(manifests[file], `unexpected ${file}`);
    return manifests[file];
  };
  assert.deepEqual(
    await bundledPackages(
      [
        "src/server.mjs",
        "node_modules/zod/v4/core/core.js",
        "node_modules/zod/v4/index.js",
        "node_modules/@scope/kit/dist/index.js",
        "node_modules/@scope/kit/node_modules/zod/lib/index.js",
        "node_modules/a/node_modules/@inner/b/node_modules_helper/x.js",
      ],
      read,
    ),
    [
      { name: "@inner/b", version: "2.0.0", license: "BSD-3-Clause" },
      { name: "@scope/kit", version: "1.0.0", license: "ISC" },
      { name: "zod", version: "3.25.0", license: "MIT" },
      { name: "zod", version: "4.6.7", license: "MIT" },
    ],
  );
});
