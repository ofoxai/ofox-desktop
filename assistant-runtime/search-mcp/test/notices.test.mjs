// Every third-party package compiled into the bundle ships with a notice:
// the bundle carries no license comments of its own.
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("the runtime's search notice covers every package in the bundle", async () => {
  const bundled = JSON.parse(
    await readFile(
      new URL("../dist/bundled-packages.json", import.meta.url),
      "utf8",
    ),
  );
  const names = bundled.map((item) => item.name);
  for (const expected of ["@modelcontextprotocol/sdk", "zod", "undici"])
    assert.ok(names.includes(expected), expected);
  const notice = await readFile(
    new URL("../../notices/ofox-search-mcp.txt", import.meta.url),
    "utf8",
  );
  for (const { name, version, license } of bundled) {
    assert.ok(
      notice.includes(`\n${name} ${version} (${license})\n`),
      `${name} ${version} (${license}) needs a notice`,
    );
  }
});
