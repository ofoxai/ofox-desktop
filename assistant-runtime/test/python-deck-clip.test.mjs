import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { options, run, zipEntries } from "./fixtures/python-engine.mjs";

test(
  "actual CLIP failures preserve relation semantics, nested sum scopes and editable native math",
  options,
  async (t) => {
    const code = await readFile(
      new URL("./fixtures/clip-loss-contract.py", import.meta.url),
      "utf8",
    );
    const result = await run(t, code);
    assert.equal(result.status, 0, result.stderr);
    const entries = await zipEntries(result.out);
    assert.equal(
      [...entries.keys()].filter((name) =>
        /^ppt\/slides\/slide\d+\.xml$/.test(name),
      ).length,
      8,
    );
    const xml = entries.get("ppt/slides/slide5.xml").toString();
    assert.equal((xml.match(/<m:nary>/g) ?? []).length, 3);
    assert.match(xml, /<m:scr m:val="script"/);
    assert.match(xml, /<m:t[^>]*>log<\/m:t>/);
    assert.match(xml, /<m:t[^>]*>exp<\/m:t>/);
    assert.match(xml, /<mc:Choice Requires="a14">/);
    assert.match(
      xml,
      /Equation fallback \(rendered image, not editable math\)/,
    );
  },
);
